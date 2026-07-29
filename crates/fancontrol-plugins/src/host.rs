//! Best-effort host sensors (GPU / storage) without privileged drivers.
//!
//! - NVIDIA GPU: fixed-path `nvidia-smi` (no PATH walk on Windows)
//! - Storage (Windows): `DeviceIoControl` temperature property — **no PowerShell**

use crate::traits::{PluginError, Result, SensorProvider};
use fancontrol_core::{SensorDescriptor, SensorId, SensorKind};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[cfg(windows)]
use std::os::windows::process::CommandExt;

/// CREATE_NO_WINDOW — avoid flashing a console when spawning nvidia-smi.
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone)]
struct Cached {
    values: Arc<Vec<(String, String, f64)>>,
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
                                        .filter(|(id, _, _)| id.starts_with("host.ssd"))
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

    fn snapshot(&self) -> Arc<Vec<(String, String, f64)>> {
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
            .map(|(id, name, _)| SensorDescriptor {
                id: SensorId::new(id.clone()),
                name: name.clone(),
                kind: SensorKind::Temperature,
                provider: "host".into(),
                unit: Some("°C".into()),
            })
            .collect()
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        if !self.is_enabled() {
            return Err(PluginError::SensorNotFound(id.to_string()));
        }
        self.snapshot()
            .iter()
            .find(|(i, _, _)| i == id.as_str())
            .map(|(_, _, v)| *v)
            .ok_or_else(|| PluginError::SensorNotFound(id.to_string()))
    }
}

fn probe_nvidia() -> Vec<(String, String, f64)> {
    let Some(smi) = resolve_nvidia_smi() else {
        return Vec::new();
    };
    let mut cmd = Command::new(&smi);
    cmd.args([
        "--query-gpu=index,name,temperature.gpu",
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
    let mut rows = Vec::new();
    for line in text.lines() {
        let parts: Vec<_> = line.split(',').map(|s| s.trim()).collect();
        if parts.len() < 3 {
            continue;
        }
        let idx = parts[0];
        let name = parts[1];
        let Ok(temp) = parts[2].parse::<f64>() else {
            continue;
        };
        rows.push((
            format!("host.gpu{idx}"),
            format!("GPU {idx} ({name})"),
            temp,
        ));
    }
    rows
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

fn probe_storage_temps() -> Vec<(String, String, f64)> {
    #[cfg(windows)]
    {
        crate::storage_win::probe_storage_temps()
    }
    #[cfg(not(windows))]
    {
        Vec::new()
    }
}
