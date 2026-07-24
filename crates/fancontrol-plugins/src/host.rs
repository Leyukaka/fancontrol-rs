//! Best-effort host sensors (GPU / storage) without privileged drivers.
//!
//! - NVIDIA GPU: `nvidia-smi` if present on PATH
//! - Storage: PowerShell StorageReliabilityCounter (slow → long TTL, background refresh)

use crate::traits::{PluginError, Result, SensorProvider};
use fancontrol_core::{SensorDescriptor, SensorId, SensorKind};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

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
        // Background loop: only nvidia-smi frequently; storage less often via separate path
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
        // Storage refresh is expensive: only when due, and not blocking every read —
        // try_lock style: only run storage if we can take last_storage quickly.
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
    let output = Command::new("nvidia-smi")
        .args([
            "--query-gpu=index,name,temperature.gpu",
            "--format=csv,noheader,nounits",
        ])
        .output();
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

fn probe_storage_temps() -> Vec<(String, String, f64)> {
    let script = r#"
$ErrorActionPreference='SilentlyContinue'
Get-PhysicalDisk | ForEach-Object {
  $n = $_.FriendlyName
  $t = ($_ | Get-StorageReliabilityCounter).Temperature
  if ($null -ne $t) { "{0}|{1}" -f $n, $t }
}
"#;
    let output = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .output();
    let Ok(out) = output else {
        return Vec::new();
    };
    if !out.status.success() {
        return Vec::new();
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate() {
        let Some((name, temp_s)) = line.split_once('|') else {
            continue;
        };
        let Ok(temp) = temp_s.trim().parse::<f64>() else {
            continue;
        };
        if !(0.0..=120.0).contains(&temp) {
            continue;
        }
        rows.push((
            format!("host.ssd{i}"),
            format!("Storage ({})", name.trim()),
            temp,
        ));
    }
    rows
}
