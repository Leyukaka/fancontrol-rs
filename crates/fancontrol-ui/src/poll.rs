//! Background hardware polling → UI snapshot.

use fancontrol_core::{ChannelMap, SensorKind};
use fancontrol_pawnio::PawnioProvider;
use fancontrol_plugins::ProviderRegistry;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, Default)]
pub struct Snapshot {
    pub temps: Vec<(String, String, f64)>,
    pub fans: Vec<(String, String, f64)>,
    pub controls: Vec<ControlSnap>,
    pub cpu_temp: Option<f64>,
    /// Sensor id behind `cpu_temp`, so the UI can seed a sensor picker with it.
    pub cpu_temp_id: Option<String>,
    /// Sensor id of the first host GPU sensor found (`host.gpu{index}`), so the UI
    /// can seed a sensor picker with it. `None` on machines without a supported
    /// discrete GPU (no `nvidia-smi`, AMD/Intel not probed).
    pub gpu_temp_id: Option<String>,
    pub error: Option<String>,
    pub tick: u64,
}

#[derive(Debug, Clone)]
pub struct ControlSnap {
    pub id: String,
    pub label: String,
    pub duty: Option<u8>,
    pub rpm: Option<f64>,
    pub writable: bool,
}

pub type SharedSnapshot = Arc<Mutex<Snapshot>>;
pub type SharedMap = Arc<Mutex<ChannelMap>>;

pub fn spawn_poller(
    reg: Arc<ProviderRegistry>,
    pawnio: Option<Arc<PawnioProvider>>,
    map: SharedMap,
    interval: Duration,
) -> SharedSnapshot {
    let shared = Arc::new(Mutex::new(Snapshot::default()));
    let out = Arc::clone(&shared);
    thread::Builder::new()
        .name("fancontrol-poll".into())
        .spawn(move || {
            let mut tick = 0u64;
            // Re-list each tick so host provider enable/disable is live.
            loop {
                let start = Instant::now();
                let sensors = reg.all_sensors();
                let controls = reg.all_controls();
                let map_snap = map.lock().map(|g| g.clone()).unwrap_or_default();
                let snap =
                    take_snapshot(&reg, pawnio.as_ref(), &sensors, &controls, &map_snap, tick);
                if let Ok(mut g) = shared.lock() {
                    *g = snap;
                }
                tick = tick.wrapping_add(1);
                let elapsed = start.elapsed();
                if elapsed < interval {
                    thread::sleep(interval - elapsed);
                }
            }
        })
        .expect("spawn poller");
    out
}

fn take_snapshot(
    reg: &ProviderRegistry,
    pawnio: Option<&Arc<PawnioProvider>>,
    sensors: &[fancontrol_core::SensorDescriptor],
    controls: &[fancontrol_core::ControlDescriptor],
    map: &ChannelMap,
    tick: u64,
) -> Snapshot {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    let mut ctrl_snaps = Vec::new();
    let mut error = None;
    let mut cpu_temp = None;
    let mut cpu_temp_id = None;
    let mut gpu_temp_id = None;

    // Fast path: one HWM bus transaction for all pawnio channels.
    // Fan/duty maps keyed by (device_index, slot) so multi-chip boards don't clobber.
    let mut pawnio_temp: HashMap<String, f64> = HashMap::new();
    let mut pawnio_fan: HashMap<(usize, usize), f64> = HashMap::new();
    let mut pawnio_duty: HashMap<(usize, usize), u8> = HashMap::new();

    if let Some(p) = pawnio {
        for (di, sample) in p.sample_all_devices() {
            for (name, v) in sample.temps {
                if let Some(t) = v {
                    let id = format!("pawnio.{di}.temp.{name}");
                    pawnio_temp.insert(id, t);
                }
            }
            for (fi, v) in sample.fans {
                if let Some(rpm) = v {
                    pawnio_fan.insert((di, fi), rpm);
                }
            }
            for (slot, v) in sample.duties {
                if let Some(d) = v {
                    pawnio_duty.insert((di, slot), d);
                }
            }
        }
    }

    for s in sensors {
        let id = s.id.as_str();
        // Prefer batch values for pawnio
        if let Some(rest) = id.strip_prefix("pawnio.") {
            if let Some((di_s, tail)) = rest.split_once('.') {
                if di_s.parse::<usize>().is_ok() {
                    if let Some(name) = tail.strip_prefix("temp.") {
                        let key = format!("pawnio.{di_s}.temp.{name}");
                        if let Some(&t) = pawnio_temp.get(&key) {
                            let label = map.sensor_name(id, &s.name).to_string();
                            if cpu_temp.is_none()
                                && (name == "CPU" || label.eq_ignore_ascii_case("CPU"))
                            {
                                cpu_temp = Some(t);
                                cpu_temp_id = Some(id.to_string());
                            }
                            temps.push((id.to_string(), label, t));
                            continue;
                        }
                    }
                    if let Some(idx) = tail.strip_prefix("fan") {
                        if let (Ok(di), Ok(fi)) = (di_s.parse::<usize>(), idx.parse::<usize>()) {
                            if let Some(&rpm) = pawnio_fan.get(&(di, fi)) {
                                let label = map.sensor_name(id, &s.name).to_string();
                                fans.push((id.to_string(), label, rpm));
                                continue;
                            }
                            // absent fan — skip without error
                            continue;
                        }
                    }
                }
            }
        }

        // Host / mock / fallback
        match reg.read_sensor(&s.id) {
            Ok(v) => match s.kind {
                SensorKind::Temperature if v != 0.0 => {
                    let label = map.sensor_name(id, &s.name).to_string();
                    if gpu_temp_id.is_none() && id.starts_with("host.gpu") {
                        gpu_temp_id = Some(id.to_string());
                    }
                    temps.push((id.to_string(), label, v));
                }
                SensorKind::FanRpm if v >= 0.0 && !v.is_nan() => {
                    let label = map.sensor_name(id, &s.name).to_string();
                    fans.push((id.to_string(), label, v));
                }
                _ => {}
            },
            Err(e) => {
                let msg = e.to_string();
                let benign = msg.contains("fan not present")
                    || msg.contains("temp out of range")
                    || msg.contains("missing")
                    || msg.contains("not present")
                    || msg.contains("Sensor not found");
                if !benign && error.is_none() {
                    error = Some(msg);
                }
            }
        }
    }

    let rpm_by_id: HashMap<String, f64> =
        fans.iter().map(|(id, _, rpm)| (id.clone(), *rpm)).collect();

    for c in controls {
        let id = c.id.as_str();
        let duty = if let Some(rest) = id.strip_prefix("pawnio.") {
            rest.split_once('.')
                .and_then(|(di_s, tail)| {
                    let di = di_s.parse::<usize>().ok()?;
                    let slot = tail.strip_prefix("ctrl")?.parse::<usize>().ok()?;
                    pawnio_duty.get(&(di, slot)).copied()
                })
                .or_else(|| reg.get_duty(&c.id).ok())
        } else {
            reg.get_duty(&c.id).ok()
        };

        let rpm = c
            .rpm_sensor
            .as_ref()
            .and_then(|sid| rpm_by_id.get(sid.as_str()).copied());

        let label = map.control_name(id, &c.name).to_string();
        ctrl_snaps.push(ControlSnap {
            id: id.to_string(),
            label,
            duty,
            rpm,
            writable: c.writable,
        });
    }

    fans.sort_by(|a, b| a.0.cmp(&b.0));
    ctrl_snaps.sort_by(|a, b| a.id.cmp(&b.id));
    temps.sort_by(|a, b| a.1.cmp(&b.1));

    Snapshot {
        temps,
        fans,
        controls: ctrl_snaps,
        cpu_temp,
        cpu_temp_id,
        gpu_temp_id,
        error,
        tick,
    }
}
