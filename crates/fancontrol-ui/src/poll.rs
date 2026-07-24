//! Background hardware polling → UI snapshot.

use fancontrol_core::{ChannelMap, SensorKind};
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
    pub error: Option<String>,
    pub tick: u64,
}

#[derive(Debug, Clone)]
pub struct ControlSnap {
    pub id: String,
    pub label: String,
    pub duty: u8,
    pub rpm: Option<f64>,
    pub writable: bool,
}

pub type SharedSnapshot = Arc<Mutex<Snapshot>>;
pub type SharedMap = Arc<Mutex<ChannelMap>>;

pub fn spawn_poller(
    reg: Arc<ProviderRegistry>,
    map: SharedMap,
    interval: Duration,
) -> SharedSnapshot {
    let shared = Arc::new(Mutex::new(Snapshot::default()));
    let out = Arc::clone(&shared);
    thread::Builder::new()
        .name("fancontrol-poll".into())
        .spawn(move || {
            let mut tick = 0u64;
            loop {
                let start = Instant::now();
                let map_snap = map.lock().map(|g| g.clone()).unwrap_or_default();
                let snap = take_snapshot(&reg, &map_snap, tick);
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

fn take_snapshot(reg: &ProviderRegistry, map: &ChannelMap, tick: u64) -> Snapshot {
    let mut temps = Vec::new();
    let mut fans = Vec::new();
    let mut controls = Vec::new();
    let mut error = None;
    let mut cpu_temp = None;

    for s in reg.all_sensors() {
        match reg.read_sensor(&s.id) {
            Ok(v) => match s.kind {
                SensorKind::Temperature if v != 0.0 => {
                    let label = map.sensor_name(s.id.as_str(), &s.name).to_string();
                    let id = s.id.as_str().to_string();
                    // Prefer Super I/O CPU package for graph
                    if cpu_temp.is_none()
                        && (id.contains("temp.CPU")
                            || id.ends_with("cpu_temp")
                            || label.eq_ignore_ascii_case("CPU"))
                    {
                        cpu_temp = Some(v);
                    }
                    temps.push((id, label, v));
                }
                SensorKind::FanRpm if v >= 0.0 && !v.is_nan() => {
                    let label = map.sensor_name(s.id.as_str(), &s.name).to_string();
                    fans.push((s.id.as_str().to_string(), label, v));
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

    let rpm_by_id: HashMap<String, f64> = fans
        .iter()
        .map(|(id, _, rpm)| (id.clone(), *rpm))
        .collect();

    for c in reg.all_controls() {
        let duty = reg.get_duty(&c.id).unwrap_or(0);
        let rpm = c
            .rpm_sensor
            .as_ref()
            .and_then(|sid| rpm_by_id.get(sid.as_str()).copied())
            .or_else(|| {
                c.rpm_sensor
                    .as_ref()
                    .and_then(|sid| reg.read_sensor(sid).ok())
            });
        let label = map.control_name(c.id.as_str(), &c.name).to_string();
        controls.push(ControlSnap {
            id: c.id.as_str().to_string(),
            label,
            duty,
            rpm,
            writable: c.writable,
        });
    }

    fans.sort_by(|a, b| a.0.cmp(&b.0));
    controls.sort_by(|a, b| a.id.cmp(&b.id));
    temps.sort_by(|a, b| a.1.cmp(&b.1));

    Snapshot {
        temps,
        fans,
        controls,
        cpu_temp,
        error,
        tick,
    }
}
