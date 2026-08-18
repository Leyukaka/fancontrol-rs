//! In-memory mock provider for development without hardware.

use crate::traits::{ControlProvider, PluginError, Result, SensorProvider};
use fancontrol_core::{ControlDescriptor, ControlId, SensorDescriptor, SensorId, SensorKind};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Simulated hardware: CPU/GPU temps + two fans with writable duty.
///
/// `Clone` shares the same live state (`Arc`), so demos can mutate sensors
/// while the registry holds another handle.
#[derive(Debug, Clone)]
pub struct MockProvider {
    name: String,
    /// Live temperature / rpm values keyed by sensor id string.
    values: Arc<Mutex<HashMap<String, f64>>>,
    /// Live duty cycles keyed by control id string.
    duties: Arc<Mutex<HashMap<String, u8>>>,
}

impl Default for MockProvider {
    fn default() -> Self {
        Self::new()
    }
}

impl MockProvider {
    pub fn new() -> Self {
        let mut values = HashMap::new();
        values.insert("mock.cpu_temp".into(), 42.0);
        values.insert("mock.gpu_temp".into(), 38.0);
        values.insert("mock.gpu_power".into(), 55.0);
        values.insert("mock.gpu_power_limit".into(), 320.0);
        values.insert("mock.cpu_power".into(), 65.0);
        values.insert("mock.cpu_power_limit".into(), 142.0);
        values.insert("mock.gpu_load".into(), 12.0);
        values.insert("mock.gpu_load_mem".into(), 8.0);
        values.insert("mock.gpu_clock".into(), 1200.0);
        values.insert("mock.gpu_clock_mem".into(), 7000.0);
        values.insert("mock.gpu_fan".into(), 25.0);
        values.insert("mock.gpu_mem_used".into(), 2048.0);
        values.insert("mock.gpu_mem_total".into(), 12288.0);
        values.insert("mock.cpu_fan_rpm".into(), 900.0);
        values.insert("mock.case_fan_rpm".into(), 700.0);
        values.insert("mock.dimm0.temp".into(), 41.0);

        let mut duties = HashMap::new();
        duties.insert("mock.cpu_fan".into(), 30);
        duties.insert("mock.case_fan".into(), 25);

        Self {
            name: "mock".into(),
            values: Arc::new(Mutex::new(values)),
            duties: Arc::new(Mutex::new(duties)),
        }
    }

    /// Override a sensor value (tests / demos).
    pub fn set_sensor_value(&self, id: &str, value: f64) {
        self.values
            .lock()
            .expect("mock values")
            .insert(id.to_string(), value);
    }

    /// Approximate RPM from duty for demo realism.
    fn sync_rpm_from_duty(values: &mut HashMap<String, f64>, control: &str, duty: u8) {
        let rpm = match control {
            "mock.cpu_fan" => 400.0 + f64::from(duty) * 18.0,
            "mock.case_fan" => 300.0 + f64::from(duty) * 12.0,
            _ => return,
        };
        let sensor = match control {
            "mock.cpu_fan" => "mock.cpu_fan_rpm",
            "mock.case_fan" => "mock.case_fan_rpm",
            _ => return,
        };
        values.insert(sensor.into(), rpm);
    }
}

impl SensorProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn sensors(&self) -> Vec<SensorDescriptor> {
        vec![
            SensorDescriptor {
                id: SensorId::new("mock.cpu_temp"),
                name: "CPU Package".into(),
                kind: SensorKind::Temperature,
                provider: self.name.clone(),
                unit: Some("°C".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_temp"),
                name: "GPU Core".into(),
                kind: SensorKind::Temperature,
                provider: self.name.clone(),
                unit: Some("°C".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_power"),
                name: "GPU Power".into(),
                kind: SensorKind::Power,
                provider: self.name.clone(),
                unit: Some("W".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_power_limit"),
                name: "GPU Power limit".into(),
                kind: SensorKind::Power,
                provider: self.name.clone(),
                unit: Some("W".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.cpu_power"),
                name: "CPU Package Power".into(),
                kind: SensorKind::Power,
                provider: self.name.clone(),
                unit: Some("W".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.cpu_power_limit"),
                name: "CPU Power limit".into(),
                kind: SensorKind::Power,
                provider: self.name.clone(),
                unit: Some("W".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_load"),
                name: "GPU Utilization".into(),
                kind: SensorKind::Load,
                provider: self.name.clone(),
                unit: Some("%".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_load_mem"),
                name: "GPU Mem controller".into(),
                kind: SensorKind::Load,
                provider: self.name.clone(),
                unit: Some("%".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_clock"),
                name: "GPU Core clock".into(),
                kind: SensorKind::Other,
                provider: self.name.clone(),
                unit: Some("MHz".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_clock_mem"),
                name: "GPU Mem clock".into(),
                kind: SensorKind::Other,
                provider: self.name.clone(),
                unit: Some("MHz".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_fan"),
                name: "GPU Fan".into(),
                kind: SensorKind::Load,
                provider: self.name.clone(),
                unit: Some("%".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_mem_used"),
                name: "GPU VRAM used".into(),
                kind: SensorKind::Other,
                provider: self.name.clone(),
                unit: Some("MiB".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.gpu_mem_total"),
                name: "GPU VRAM total".into(),
                kind: SensorKind::Other,
                provider: self.name.clone(),
                unit: Some("MiB".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.cpu_fan_rpm"),
                name: "CPU Fan".into(),
                kind: SensorKind::FanRpm,
                provider: self.name.clone(),
                unit: Some("RPM".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.case_fan_rpm"),
                name: "Case Fan".into(),
                kind: SensorKind::FanRpm,
                provider: self.name.clone(),
                unit: Some("RPM".into()),
            },
            SensorDescriptor {
                id: SensorId::new("mock.dimm0.temp"),
                name: "DIMM 0 Temp".into(),
                kind: SensorKind::Temperature,
                provider: self.name.clone(),
                unit: Some("°C".into()),
            },
        ]
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        self.values
            .lock()
            .expect("mock values")
            .get(id.as_str())
            .copied()
            .ok_or_else(|| PluginError::SensorNotFound(id.to_string()))
    }
}

impl ControlProvider for MockProvider {
    fn name(&self) -> &str {
        &self.name
    }

    fn controls(&self) -> Vec<ControlDescriptor> {
        vec![
            ControlDescriptor {
                id: ControlId::new("mock.cpu_fan"),
                name: "CPU Fan".into(),
                provider: self.name.clone(),
                writable: true,
                rpm_sensor: Some(SensorId::new("mock.cpu_fan_rpm")),
            },
            ControlDescriptor {
                id: ControlId::new("mock.case_fan"),
                name: "Case Fan".into(),
                provider: self.name.clone(),
                writable: true,
                rpm_sensor: Some(SensorId::new("mock.case_fan_rpm")),
            },
        ]
    }

    fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()> {
        let percent = percent.min(100);
        let mut duties = self.duties.lock().expect("mock duties");
        if !duties.contains_key(id.as_str()) {
            return Err(PluginError::ControlNotFound(id.to_string()));
        }
        duties.insert(id.as_str().to_string(), percent);
        let mut values = self.values.lock().expect("mock values");
        Self::sync_rpm_from_duty(&mut values, id.as_str(), percent);
        tracing::debug!(control = %id, duty = percent, "mock set_duty");
        Ok(())
    }

    fn get_duty(&self, id: &ControlId) -> Result<u8> {
        self.duties
            .lock()
            .expect("mock duties")
            .get(id.as_str())
            .copied()
            .ok_or_else(|| PluginError::ControlNotFound(id.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mock_lists_and_reads() {
        let m = MockProvider::new();
        // 2 temps (CPU/GPU) + 9 GPU metrics + 2 CPU power + 2 fan RPM + 1 DIMM temp
        assert_eq!(m.sensors().len(), 16);
        assert_eq!(m.controls().len(), 2);
        let t = m.read(&SensorId::new("mock.cpu_temp")).unwrap();
        assert!((t - 42.0).abs() < f64::EPSILON);
        let p = m.read(&SensorId::new("mock.gpu_power")).unwrap();
        assert!((p - 55.0).abs() < f64::EPSILON);
        let cp = m.read(&SensorId::new("mock.cpu_power")).unwrap();
        assert!((cp - 65.0).abs() < f64::EPSILON);
        let cpl = m.read(&SensorId::new("mock.cpu_power_limit")).unwrap();
        assert!((cpl - 142.0).abs() < f64::EPSILON);
    }

    #[test]
    fn mock_set_duty_updates_rpm() {
        let m = MockProvider::new();
        m.set_duty(&ControlId::new("mock.cpu_fan"), 50).unwrap();
        assert_eq!(m.get_duty(&ControlId::new("mock.cpu_fan")).unwrap(), 50);
        let rpm = m.read(&SensorId::new("mock.cpu_fan_rpm")).unwrap();
        assert!((rpm - (400.0 + 50.0 * 18.0)).abs() < 0.1);
    }
}
