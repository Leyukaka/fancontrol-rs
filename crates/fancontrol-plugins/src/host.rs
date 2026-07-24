//! Best-effort host sensors (GPU / storage) without privileged drivers.
//!
//! - NVIDIA GPU: `nvidia-smi` if present on PATH
//! - Storage / thermal: optional PowerShell CIM (slow, cached)
//!
//! Failures are silent (empty sensor list) so the Super I/O path stays primary.

use crate::traits::{PluginError, Result, SensorProvider};
use fancontrol_core::{SensorDescriptor, SensorId, SensorKind};
use std::process::Command;
use std::sync::Mutex;
use std::time::{Duration, Instant};

#[derive(Clone)]
struct Cached {
    at: Instant,
    values: Vec<(String, String, f64)>, // id, name, °C
}

pub struct HostSensorProvider {
    cache: Mutex<Option<Cached>>,
    ttl: Duration,
}

impl Default for HostSensorProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl HostSensorProvider {
    pub fn new() -> Self {
        Self {
            cache: Mutex::new(None),
            ttl: Duration::from_secs(2),
        }
    }

    fn refresh(&self) -> Vec<(String, String, f64)> {
        let mut out = Vec::new();
        out.extend(probe_nvidia());
        out.extend(probe_storage_temps());
        out
    }

    fn values(&self) -> Vec<(String, String, f64)> {
        let mut guard = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        let stale = guard
            .as_ref()
            .map(|c| c.at.elapsed() > self.ttl)
            .unwrap_or(true);
        if stale {
            let values = self.refresh();
            *guard = Some(Cached {
                at: Instant::now(),
                values: values.clone(),
            });
            values
        } else {
            guard.as_ref().map(|c| c.values.clone()).unwrap_or_default()
        }
    }
}

impl SensorProvider for HostSensorProvider {
    fn name(&self) -> &str {
        "host"
    }

    fn sensors(&self) -> Vec<SensorDescriptor> {
        self.values()
            .into_iter()
            .map(|(id, name, _)| SensorDescriptor {
                id: SensorId::new(id),
                name,
                kind: SensorKind::Temperature,
                provider: "host".into(),
                unit: Some("°C".into()),
            })
            .collect()
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        self.values()
            .into_iter()
            .find(|(i, _, _)| i == id.as_str())
            .map(|(_, _, v)| v)
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

/// Best-effort storage temperature via PowerShell StorageReliabilityCounter.
/// Slow — results cached by HostSensorProvider TTL.
fn probe_storage_temps() -> Vec<(String, String, f64)> {
    // Keep the script short; hide errors.
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
        // StorageReliabilityCounter is often already °C on modern Windows
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
