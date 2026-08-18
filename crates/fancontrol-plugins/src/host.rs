//! Best-effort host sensors (GPU / storage) without privileged drivers.
//!
//! - NVIDIA GPU: fixed-path `nvidia-smi` multi-metric query (no PATH walk on Windows)
//! - Storage (Windows): `DeviceIoControl` temperature property - **no PowerShell**
//!
//! Hot Spot is **not** exposed by `nvidia-smi` / public NVML; LibreHardwareMonitor
//! gets it via reverse-engineered NvAPI. We intentionally do not fake it here.

use crate::traits::{PluginError, Result, SensorProvider};
use fancontrol_core::{SensorDescriptor, SensorId, SensorKind};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// CREATE_NO_WINDOW - avoid flashing a console when spawning nvidia-smi.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// One cached host sensor row (GPU metric or storage temp).
#[derive(Debug, Clone)]
struct SensorRow {
    id: String,
    name: String,
    value: f64,
    kind: SensorKind,
    unit: Option<&'static str>,
}

#[derive(Clone)]
struct Cached {
    values: Arc<Vec<SensorRow>>,
}

pub struct HostSensorProvider {
    /// When false, `sensors()`/`read()` are empty and the background loop idles.
    enabled: Arc<AtomicBool>,
    cache: Arc<Mutex<Option<Cached>>>,
    started: Mutex<bool>,
}

impl Default for HostSensorProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl HostSensorProvider {
    /// Host sensors enabled (CLI default).
    pub fn new() -> Self {
        Self::with_enabled(Arc::new(AtomicBool::new(true)))
    }

    /// Share an enable flag with the UI (live Options toggle).
    pub fn with_enabled(enabled: Arc<AtomicBool>) -> Self {
        let p = Self {
            enabled,
            cache: Arc::new(Mutex::new(None)),
            started: Mutex::new(false),
        };
        p.ensure_bg_refresh();
        p
    }

    pub fn enabled_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.enabled)
    }

    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::Relaxed);
    }

    fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::Relaxed)
    }

    fn ensure_bg_refresh(&self) {
        let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
        if *started {
            return;
        }
        *started = true;
        let cache = Arc::clone(&self.cache);
        let enabled = Arc::clone(&self.enabled);
        thread::Builder::new()
            .name("host-sensors".into())
            .spawn(move || {
                let mut empty_gpu_streak = 0u32;
                let mut last_storage = Instant::now() - Duration::from_secs(60);
                let storage_every = Duration::from_secs(5);
                loop {
                    if !enabled.load(Ordering::Relaxed) {
                        empty_gpu_streak = 0;
                        thread::sleep(Duration::from_secs(2));
                        continue;
                    }
                    // Storage on its own cadence (not tied to GPU backoff).
                    let refresh_storage = last_storage.elapsed() >= storage_every;
                    if refresh_storage {
                        last_storage = Instant::now();
                    }
                    let storage = if refresh_storage {
                        // Never let a storage IOCTL bug tear down the UI process.
                        std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe_storage_temps))
                            .unwrap_or_else(|_| {
                                tracing::error!("storage temp probe panicked; skipping this cycle");
                                Vec::new()
                            })
                    } else {
                        // Keep previous SSD rows from cache
                        cache
                            .lock()
                            .ok()
                            .and_then(|g| {
                                g.as_ref().map(|c| {
                                    c.values
                                        .iter()
                                        .filter(|r| r.id.starts_with("host.ssd"))
                                        .cloned()
                                        .collect::<Vec<_>>()
                                })
                            })
                            .unwrap_or_default()
                    };
                    let gpu = probe_nvidia();
                    if gpu.is_empty() {
                        empty_gpu_streak = empty_gpu_streak.saturating_add(1);
                    } else {
                        empty_gpu_streak = 0;
                    }
                    {
                        let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
                        let mut values = gpu;
                        values.extend(storage);
                        *g = Some(Cached {
                            values: Arc::new(values),
                        });
                    }
                    // GPU probe backoff when absent; storage still refreshed above.
                    let sleep = if empty_gpu_streak == 0 {
                        Duration::from_secs(3)
                    } else if empty_gpu_streak < 3 {
                        Duration::from_secs(5)
                    } else {
                        Duration::from_secs(15)
                    };
                    thread::sleep(sleep);
                }
            })
            .ok();
    }

    fn snapshot(&self) -> Arc<Vec<SensorRow>> {
        if !self.is_enabled() {
            return Arc::new(Vec::new());
        }
        // Storage + GPU probes run only on the bg thread (no second IOCTL path here).
        self.ensure_bg_refresh();
        let g = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        g.as_ref()
            .map(|c| Arc::clone(&c.values))
            .unwrap_or_else(|| Arc::new(Vec::new()))
    }
}

impl SensorProvider for HostSensorProvider {
    fn name(&self) -> &str {
        "host"
    }

    fn sensors(&self) -> Vec<SensorDescriptor> {
        if !self.is_enabled() {
            return Vec::new();
        }
        self.snapshot()
            .iter()
            .map(|r| SensorDescriptor {
                id: SensorId::new(r.id.clone()),
                name: r.name.clone(),
                kind: r.kind,
                provider: "host".into(),
                unit: r.unit.map(|u| u.to_string()),
            })
            .collect()
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        if !self.is_enabled() {
            return Err(PluginError::SensorNotFound(id.to_string()));
        }
        self.snapshot()
            .iter()
            .find(|r| r.id == id.as_str())
            .map(|r| r.value)
            .ok_or_else(|| PluginError::SensorNotFound(id.to_string()))
    }
}

fn probe_nvidia() -> Vec<SensorRow> {
    let Some(smi) = resolve_nvidia_smi() else {
        return Vec::new();
    };
    let mut cmd = Command::new(&smi);
    // Multi-metric query inspired by LibreHardwareMonitor's NVIDIA surface area,
    // but limited to fields exposed by nvidia-smi (documented / stable).
    cmd.args([
        "--query-gpu=index,name,temperature.gpu,temperature.memory,power.draw,power.limit,utilization.gpu,utilization.memory,clocks.current.graphics,clocks.current.memory,fan.speed,memory.used,memory.total",
        "--format=csv,noheader,nounits",
    ]);
    #[cfg(windows)]
    cmd.creation_flags(CREATE_NO_WINDOW);
    let output = cmd.output();
    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    parse_nvidia_smi_csv(&text)
}

/// Parse nvidia-smi multi-metric CSV (`csv,noheader,nounits`).
///
/// Column order matches the query in [`probe_nvidia`].
fn parse_nvidia_smi_csv(text: &str) -> Vec<SensorRow> {
    let mut rows = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        // GPU names can contain commas rarely; nvidia-smi usually quotes nothing.
        // Split on commas and trim; require at least index+name+temp.core.
        let parts: Vec<_> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let idx = parts[0];
        if idx.parse::<u32>().is_err() {
            continue;
        }
        let name = parts[1];
        let label = format!("GPU {idx} ({name})");
        let prefix = format!("host.gpu{idx}");

        let temp_core = parts.get(2).copied().and_then(parse_smi_f64);
        if let Some(t) = temp_core {
            // Compat alias used by graph seed / older channel maps.
            rows.push(SensorRow {
                id: prefix.clone(),
                name: label.clone(),
                value: t,
                kind: SensorKind::Temperature,
                unit: Some("°C"),
            });
            rows.push(SensorRow {
                id: format!("{prefix}.temp.core"),
                name: format!("{label} · Core"),
                value: t,
                kind: SensorKind::Temperature,
                unit: Some("°C"),
            });
        }

        // Optional multi-metric fields (skip N/A).
        let mut push = |suffix: &str,
                        display: &str,
                        raw: Option<&str>,
                        kind: SensorKind,
                        unit: Option<&'static str>| {
            let Some(raw) = raw else {
                return;
            };
            let Some(v) = parse_smi_f64(raw) else {
                return;
            };
            rows.push(SensorRow {
                id: format!("{prefix}.{suffix}"),
                name: format!("{label} · {display}"),
                value: v,
                kind,
                unit,
            });
        };

        push(
            "temp.memory",
            "Memory",
            parts.get(3).copied(),
            SensorKind::Temperature,
            Some("°C"),
        );
        push(
            "power.draw",
            "Power",
            parts.get(4).copied(),
            SensorKind::Power,
            Some("W"),
        );
        push(
            "power.limit",
            "Power limit",
            parts.get(5).copied(),
            SensorKind::Power,
            Some("W"),
        );
        push(
            "load.gpu",
            "Utilization",
            parts.get(6).copied(),
            SensorKind::Load,
            Some("%"),
        );
        push(
            "load.mem",
            "Mem controller",
            parts.get(7).copied(),
            SensorKind::Load,
            Some("%"),
        );
        push(
            "clock.graphics",
            "Core clock",
            parts.get(8).copied(),
            SensorKind::Other,
            Some("MHz"),
        );
        push(
            "clock.memory",
            "Mem clock",
            parts.get(9).copied(),
            SensorKind::Other,
            Some("MHz"),
        );
        push(
            "fan",
            "Fan",
            parts.get(10).copied(),
            SensorKind::Load,
            Some("%"),
        );
        push(
            "mem.used",
            "VRAM used",
            parts.get(11).copied(),
            SensorKind::Other,
            Some("MiB"),
        );
        push(
            "mem.total",
            "VRAM total",
            parts.get(12).copied(),
            SensorKind::Other,
            Some("MiB"),
        );
    }
    rows
}

/// Parse a single nvidia-smi CSV cell (`nounits`). Returns `None` for N/A / empty / garbage.
fn parse_smi_f64(raw: &str) -> Option<f64> {
    let s = raw.trim();
    if s.is_empty() {
        return None;
    }
    let lower = s.to_ascii_lowercase();
    if lower == "n/a"
        || lower == "[n/a]"
        || lower == "na"
        || lower.contains("not support")
        || lower.contains("deprecated")
        || lower.contains("error")
    {
        return None;
    }
    // Tolerate leftover units if a future call drops `nounits`.
    let cleaned = s
        .trim_end_matches('%')
        .trim()
        .trim_end_matches("W")
        .trim()
        .trim_end_matches("MiB")
        .trim()
        .trim_end_matches("MHz")
        .trim()
        .trim_end_matches('C')
        .trim();
    cleaned.parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Prefer absolute NVIDIA install paths so we do **not** walk a polluted `PATH`.
fn resolve_nvidia_smi() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    use std::sync::OnceLock;

    static CACHED: OnceLock<Option<PathBuf>> = OnceLock::new();
    CACHED
        .get_or_init(|| {
            let mut candidates: Vec<PathBuf> = Vec::new();
            #[cfg(windows)]
            {
                const REL: &str = r"NVIDIA Corporation\NVSMI\nvidia-smi.exe";
                for env_key in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
                    if let Ok(root) = std::env::var(env_key) {
                        candidates.push(PathBuf::from(root).join(REL));
                    }
                }
                candidates.push(PathBuf::from(
                    r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
                ));
                candidates.push(PathBuf::from(
                    r"C:\Program Files (x86)\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
                ));
                if let Ok(sys) = std::env::var("SystemRoot") {
                    candidates.push(PathBuf::from(sys).join(r"System32\nvidia-smi.exe"));
                }
            }
            for p in &candidates {
                if p.is_file() {
                    return Some(p.clone());
                }
            }
            #[cfg(not(windows))]
            {
                return which_on_path("nvidia-smi");
            }
            #[cfg(windows)]
            {
                None
            }
        })
        .clone()
}

#[cfg(not(windows))]
fn which_on_path(name: &str) -> Option<std::path::PathBuf> {
    use std::path::PathBuf;
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let bare = dir.join(name);
        if bare.is_file() {
            return Some(bare);
        }
    }
    None
}

fn probe_storage_temps() -> Vec<SensorRow> {
    #[cfg(windows)]
    {
        crate::storage_win::probe_storage_temps()
            .into_iter()
            .map(|(id, name, value)| SensorRow {
                id,
                name,
                value,
                kind: SensorKind::Temperature,
                unit: Some("°C"),
            })
            .collect()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_smi_skips_na_and_emits_typed_rows() {
        let csv = "0, NVIDIA GeForce RTX 5080, 43, [N/A], 40.23, 360.00, 5, 6, 502, 7001, 0, 2174, 16303\n";
        let rows = parse_nvidia_smi_csv(csv);
        let by_id: std::collections::HashMap<_, _> =
            rows.iter().map(|r| (r.id.as_str(), r)).collect();

        assert!(by_id.contains_key("host.gpu0"));
        assert!((by_id["host.gpu0"].value - 43.0).abs() < 0.01);
        assert_eq!(by_id["host.gpu0"].kind, SensorKind::Temperature);

        assert!(by_id.contains_key("host.gpu0.temp.core"));
        assert!(!by_id.contains_key("host.gpu0.temp.memory")); // N/A

        assert!((by_id["host.gpu0.power.draw"].value - 40.23).abs() < 0.01);
        assert_eq!(by_id["host.gpu0.power.draw"].kind, SensorKind::Power);
        assert_eq!(by_id["host.gpu0.power.draw"].unit, Some("W"));

        assert!((by_id["host.gpu0.load.gpu"].value - 5.0).abs() < 0.01);
        assert_eq!(by_id["host.gpu0.load.gpu"].kind, SensorKind::Load);

        assert!((by_id["host.gpu0.clock.graphics"].value - 502.0).abs() < 0.01);
        assert!((by_id["host.gpu0.mem.used"].value - 2174.0).abs() < 0.01);
        assert!((by_id["host.gpu0.mem.total"].value - 16303.0).abs() < 0.01);
        assert!((by_id["host.gpu0.fan"].value - 0.0).abs() < 0.01);
    }

    #[test]
    fn parse_smi_multi_gpu() {
        let csv = "\
0, Card A, 40, 50, 100, 300, 10, 20, 1000, 8000, 30, 1000, 8000
1, Card B, 55, N/A, 80, 250, 40, 15, 1500, 9000, 45, 2000, 12000
";
        let rows = parse_nvidia_smi_csv(csv);
        assert!(rows.iter().any(|r| r.id == "host.gpu0.power.draw"));
        assert!(rows.iter().any(|r| r.id == "host.gpu1.temp.core"));
        assert!(!rows.iter().any(|r| r.id == "host.gpu1.temp.memory"));
        let g1 = rows.iter().find(|r| r.id == "host.gpu1").unwrap();
        assert!((g1.value - 55.0).abs() < 0.01);
    }

    #[test]
    fn parse_smi_empty_and_garbage() {
        assert!(parse_nvidia_smi_csv("").is_empty());
        assert!(parse_nvidia_smi_csv("not,a,gpu,line\n").is_empty());
        assert!(parse_smi_f64("N/A").is_none());
        assert!(parse_smi_f64("").is_none());
        assert!((parse_smi_f64("42.5").unwrap() - 42.5).abs() < 0.01);
        assert!((parse_smi_f64("42.5 %").unwrap() - 42.5).abs() < 0.01);
    }
}
