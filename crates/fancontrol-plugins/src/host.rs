//! Best-effort host sensors (GPU / storage) without privileged drivers.
//!
//! - NVIDIA GPU: `nvidia-smi` if present on PATH (no PowerShell)
//! - Storage (Windows): `DeviceIoControl` temperature property — **no PowerShell**

use crate::traits::{PluginError, Result, SensorProvider};
use fancontrol_core::{SensorDescriptor, SensorId, SensorKind};
use std::process::Command;
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
    cache: Arc<Mutex<Option<Cached>>>,
    gpu_ttl: Duration,
    storage_ttl: Duration,
    /// Last time we attempted a storage probe (independent of GPU).
    last_storage: Mutex<Instant>,
    storage_cache: Mutex<Vec<(String, String, f64)>>,
    started: Mutex<bool>,
}

impl Default for HostSensorProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl HostSensorProvider {
    pub fn new() -> Self {
        let p = Self {
            cache: Arc::new(Mutex::new(None)),
            gpu_ttl: Duration::from_secs(2),
            storage_ttl: Duration::from_secs(15),
            last_storage: Mutex::new(Instant::now() - Duration::from_secs(60)),
            storage_cache: Mutex::new(Vec::new()),
            started: Mutex::new(false),
        };
        p.ensure_bg_refresh();
        p
    }

    fn ensure_bg_refresh(&self) {
        let mut started = self.started.lock().unwrap_or_else(|e| e.into_inner());
        if *started {
            return;
        }
        *started = true;
        let cache = Arc::clone(&self.cache);
        let gpu_ttl = self.gpu_ttl;
        // Background loop: nvidia-smi frequently; storage refreshed on demand (merge_storage_if_due)
        thread::Builder::new()
            .name("host-sensors".into())
            .spawn(move || loop {
                let gpu = probe_nvidia();
                {
                    let mut g = cache.lock().unwrap_or_else(|e| e.into_inner());
                    let storage = g
                        .as_ref()
                        .map(|c| {
                            c.values
                                .iter()
                                .filter(|(id, _, _)| id.starts_with("host.ssd"))
                                .cloned()
                                .collect::<Vec<_>>()
                        })
                        .unwrap_or_default();
                    let mut values = gpu;
                    values.extend(storage);
                    *g = Some(Cached {
                        values: Arc::new(values),
                    });
                }
                thread::sleep(gpu_ttl);
            })
            .ok();
    }

    fn merge_storage_if_due(&self) {
        let mut last = self.last_storage.lock().unwrap_or_else(|e| e.into_inner());
        if last.elapsed() < self.storage_ttl {
            return;
        }
        *last = Instant::now();
        let storage = probe_storage_temps();
        *self.storage_cache.lock().unwrap_or_else(|e| e.into_inner()) = storage.clone();
        let mut g = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let gpu: Vec<_> = g
            .as_ref()
            .map(|c| {
                c.values
                    .iter()
                    .filter(|(id, _, _)| id.starts_with("host.gpu"))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default();
        let mut values = gpu;
        values.extend(storage);
        *g = Some(Cached {
            values: Arc::new(values),
        });
    }

    fn snapshot(&self) -> Arc<Vec<(String, String, f64)>> {
        self.ensure_bg_refresh();
        self.merge_storage_if_due();
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

/// Prefer absolute NVIDIA install paths so we do **not** walk a polluted `PATH`
/// (VirusTotal sandboxes often list every `…\javapath\nvidia-smi.exe` probe as a
/// "file opened"). Bare `nvidia-smi` on PATH is last resort only.
fn resolve_nvidia_smi() -> Option<std::path::PathBuf> {
    use std::path::PathBuf;

    let mut candidates: Vec<PathBuf> = Vec::new();

    #[cfg(windows)]
    {
        const REL: &str = r"NVIDIA Corporation\NVSMI\nvidia-smi.exe";
        for env_key in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
            if let Ok(root) = std::env::var(env_key) {
                candidates.push(PathBuf::from(root).join(REL));
            }
        }
        // Common defaults if env is odd in a sandbox
        candidates.push(PathBuf::from(r"C:\Program Files\NVIDIA Corporation\NVSMI\nvidia-smi.exe"));
        candidates.push(PathBuf::from(
            r"C:\Program Files (x86)\NVIDIA Corporation\NVSMI\nvidia-smi.exe",
        ));
        // Some driver layouts also place a copy under System32
        if let Ok(sys) = std::env::var("SystemRoot") {
            candidates.push(PathBuf::from(sys).join(r"System32\nvidia-smi.exe"));
        }
    }

    for p in &candidates {
        if p.is_file() {
            return Some(p.clone());
        }
    }

    // No bare PATH lookup on Windows: CreateProcess("nvidia-smi") and manual PATH
    // walks both produce VirusTotal "files opened" noise under every odd PATH entry
    // (Oracle javapath, Unrar, …). Non-Windows: allow PATH for dev convenience.
    #[cfg(not(windows))]
    {
        which_on_path("nvidia-smi")
    }
    #[cfg(windows)]
    {
        None
    }
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
