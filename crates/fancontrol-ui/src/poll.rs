//! Background hardware polling → UI snapshot.

use fancontrol_core::{cpu_temp_seed_priority, ChannelMap, SensorKind};
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
    /// Aggregated host GPU metrics for the GPU detail panel (nvidia-smi multi-metric).
    pub gpus: Vec<GpuSnap>,
    /// All series eligible for the multi-sensor graph (temps + GPU power/util/…).
    pub plottable: Vec<PlottableSensor>,
    pub cpu_temp: Option<f64>,
    /// Sensor id behind `cpu_temp`, so the UI can seed a sensor picker with it.
    pub cpu_temp_id: Option<String>,
    /// Sensor id of the first host GPU core temp (`host.gpu{index}` alias), so the UI
    /// can seed a sensor picker with it. `None` on machines without a supported
    /// discrete GPU (no `nvidia-smi`, AMD/Intel not probed).
    pub gpu_temp_id: Option<String>,
    /// Sensor id of CPU package power (`host.cpu.power.package` / `mock.cpu_power`),
    /// so the UI can seed the graph with it. `None` when unavailable (non-AMD CPU,
    /// unsupported family, PawnIO missing, or mock disabled).
    pub cpu_power_id: Option<String>,
    pub error: Option<String>,
    pub tick: u64,
}

/// One live value that can be selected on the graph (any unit).
#[derive(Debug, Clone)]
pub struct PlottableSensor {
    pub id: String,
    pub label: String,
    pub value: f64,
    pub kind: SensorKind,
    pub unit: Option<String>,
}

/// One discrete GPU as assembled from host sensor ids (`host.gpu{N}.*`).
#[derive(Debug, Clone, Default)]
pub struct GpuSnap {
    /// Adapter index from nvidia-smi (for multi-GPU labels / future UI).
    #[allow(dead_code)]
    pub index: u32,
    /// Friendly name from the alias sensor (`GPU 0 (RTX …)`).
    pub name: String,
    pub temp_core: Option<f64>,
    pub temp_memory: Option<f64>,
    /// Always `None` with the current nvidia-smi path (hotspot needs NvAPI).
    pub temp_hotspot: Option<f64>,
    pub power_w: Option<f64>,
    pub power_limit_w: Option<f64>,
    pub util_gpu: Option<f64>,
    pub util_mem: Option<f64>,
    pub clock_graphics_mhz: Option<f64>,
    pub clock_memory_mhz: Option<f64>,
    pub fan_percent: Option<f64>,
    pub mem_used_mib: Option<f64>,
    pub mem_total_mib: Option<f64>,
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
    let mut cpu_prio = u8::MAX;
    let mut gpu_temp_id = None;
    let mut cpu_power_id = None;
    // id → value for host GPU metrics (also mock.gpu* for UI demos).
    let mut host_gpu_vals: HashMap<String, f64> = HashMap::new();
    let mut host_gpu_names: HashMap<u32, String> = HashMap::new();
    let mut plottable: Vec<PlottableSensor> = Vec::new();

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
                            // Prefer CPU / PECI_0 / CPUTIN over "first temp" (banked NCT).
                            let prio = cpu_temp_seed_priority(name)
                                .min(cpu_temp_seed_priority(label.as_str()));
                            if prio < cpu_prio {
                                cpu_prio = prio;
                                cpu_temp = Some(t);
                                cpu_temp_id = Some(id.to_string());
                            }
                            temps.push((id.to_string(), label.clone(), t));
                            plottable.push(PlottableSensor {
                                id: id.to_string(),
                                label,
                                value: t,
                                kind: SensorKind::Temperature,
                                unit: Some("°C".into()),
                            });
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
            Ok(v) => {
                if id.starts_with("host.gpu") || id.starts_with("mock.gpu") {
                    host_gpu_vals.insert(id.to_string(), v);
                }
                match s.kind {
                    SensorKind::Temperature if v != 0.0 || id.starts_with("host.gpu") => {
                        let label = map.sensor_name(id, &s.name).to_string();
                        // Prefer the short alias `host.gpu{N}` for graph seed.
                        let is_alias = id.strip_prefix("host.gpu").is_some_and(|rest| {
                            !rest.is_empty() && rest.chars().all(|c| c.is_ascii_digit())
                        }) || id == "mock.gpu_temp";
                        if gpu_temp_id.is_none() && is_alias {
                            gpu_temp_id = Some(id.to_string());
                            if let Some(rest) = id.strip_prefix("host.gpu") {
                                if let Ok(idx) = rest.parse::<u32>() {
                                    host_gpu_names.insert(idx, label.clone());
                                }
                            }
                        }
                        // Skip `.temp.core` duplicate of the short alias in Temps/graph picker.
                        if !id.ends_with(".temp.core") {
                            temps.push((id.to_string(), label.clone(), v));
                            plottable.push(PlottableSensor {
                                id: id.to_string(),
                                label,
                                value: v,
                                kind: SensorKind::Temperature,
                                unit: s.unit.clone().or_else(|| Some("°C".into())),
                            });
                        }
                    }
                    SensorKind::FanRpm if v >= 0.0 && !v.is_nan() => {
                        let label = map.sensor_name(id, &s.name).to_string();
                        fans.push((id.to_string(), label, v));
                    }
                    // Power / Load / Other: plottable for graph + GPU panel assembly.
                    SensorKind::Power
                    | SensorKind::Load
                    | SensorKind::Voltage
                    | SensorKind::Other
                        if v.is_finite()
                            && (id.starts_with("host.gpu")
                                || id.starts_with("mock.gpu")
                                || s.kind == SensorKind::Power) =>
                    {
                        if cpu_power_id.is_none()
                            && (id == "host.cpu.power.package" || id == "mock.cpu_power")
                        {
                            cpu_power_id = Some(id.to_string());
                        }
                        let label = map.sensor_name(id, &s.name).to_string();
                        plottable.push(PlottableSensor {
                            id: id.to_string(),
                            label,
                            value: v,
                            kind: s.kind,
                            unit: s.unit.clone(),
                        });
                    }
                    _ => {}
                }
            }
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

    let gpus = assemble_gpu_snaps(&host_gpu_vals, &host_gpu_names);

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
    plottable.sort_by(|a, b| a.label.cmp(&b.label));

    Snapshot {
        temps,
        fans,
        controls: ctrl_snaps,
        gpus,
        plottable,
        cpu_temp,
        cpu_temp_id,
        gpu_temp_id,
        cpu_power_id,
        error,
        tick,
    }
}

/// Build [`GpuSnap`] entries from flat host/mock sensor values.
fn assemble_gpu_snaps(vals: &HashMap<String, f64>, names: &HashMap<u32, String>) -> Vec<GpuSnap> {
    // Discover GPU indices from any host.gpu{N}… id.
    let mut indices: Vec<u32> = vals
        .keys()
        .filter_map(|id| {
            let rest = id.strip_prefix("host.gpu")?;
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            digits.parse().ok()
        })
        .collect();
    indices.sort_unstable();
    indices.dedup();

    let mut out: Vec<GpuSnap> = indices
        .into_iter()
        .map(|index| {
            let prefix = format!("host.gpu{index}");
            let get = |suffix: &str| vals.get(&format!("{prefix}.{suffix}")).copied();
            let name = names
                .get(&index)
                .cloned()
                .or_else(|| vals.get(&prefix).map(|_| format!("GPU {index}")))
                .unwrap_or_else(|| format!("GPU {index}"));
            GpuSnap {
                index,
                name,
                temp_core: vals.get(&prefix).copied().or_else(|| get("temp.core")),
                temp_memory: get("temp.memory"),
                temp_hotspot: None, // nvidia-smi does not expose hotspot
                power_w: get("power.draw"),
                power_limit_w: get("power.limit"),
                util_gpu: get("load.gpu"),
                util_mem: get("load.mem"),
                clock_graphics_mhz: get("clock.graphics"),
                clock_memory_mhz: get("clock.memory"),
                fan_percent: get("fan"),
                mem_used_mib: get("mem.used"),
                mem_total_mib: get("mem.total"),
            }
        })
        .collect();

    // Mock path: single synthetic GPU when no host NVIDIA metrics.
    if out.is_empty() {
        if let Some(&core) = vals.get("mock.gpu_temp") {
            out.push(GpuSnap {
                index: 0,
                name: "GPU (mock)".into(),
                temp_core: Some(core),
                temp_memory: vals.get("mock.gpu_temp_memory").copied(),
                temp_hotspot: None,
                power_w: vals.get("mock.gpu_power").copied(),
                power_limit_w: vals.get("mock.gpu_power_limit").copied(),
                util_gpu: vals.get("mock.gpu_load").copied(),
                util_mem: vals.get("mock.gpu_load_mem").copied(),
                clock_graphics_mhz: vals.get("mock.gpu_clock").copied(),
                clock_memory_mhz: vals.get("mock.gpu_clock_mem").copied(),
                fan_percent: vals.get("mock.gpu_fan").copied(),
                mem_used_mib: vals.get("mock.gpu_mem_used").copied(),
                mem_total_mib: vals.get("mock.gpu_mem_total").copied(),
            });
        }
    }

    out
}
