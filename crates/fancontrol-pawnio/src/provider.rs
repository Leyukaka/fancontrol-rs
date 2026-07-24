//! Plugin provider bridging Super I/O hardware into fancontrol traits.

use crate::device::SuperIoDevice;
use crate::superio::detect_chips;
use fancontrol_core::{
    ControlDescriptor, ControlId, SensorDescriptor, SensorId, SensorKind,
};
use fancontrol_plugins::{ControlProvider, PluginError, Result, SensorProvider};
use std::sync::Mutex;

pub struct PawnioProvider {
    devices: Vec<SuperIoDevice>,
    detect_notes: Mutex<Vec<String>>,
    init_error: Option<String>,
}

impl PawnioProvider {
    /// Probe hardware and open supported HWM devices.
    pub fn probe() -> Self {
        match detect_chips() {
            Ok(chips) => {
                let mut notes = Vec::new();
                let mut devices = Vec::new();
                for c in &chips {
                    notes.push(format!(
                        "slot{} @0x{:02X}: {} hwm={:?}",
                        c.slot,
                        c.register_port,
                        c.chip.name(),
                        c.hwm_address.map(|a| format!("0x{a:04X}"))
                    ));
                    match SuperIoDevice::try_open(c) {
                        Ok(dev) => {
                            notes.push(format!(
                                "  opened {} HWM at 0x{:04X} (temps={} fans={} ctrls={})",
                                dev.kind_label(),
                                dev.hwm_address(),
                                dev.temp_sources().len(),
                                dev.fan_count(),
                                dev.control_count()
                            ));
                            devices.push(dev);
                        }
                        Err(e) => notes.push(format!("  open skipped/failed: {e}")),
                    }
                }
                if chips.is_empty() {
                    notes.push("no Super I/O chip detected".into());
                }
                Self {
                    devices,
                    detect_notes: Mutex::new(notes),
                    init_error: None,
                }
            }
            Err(e) => Self {
                devices: Vec::new(),
                detect_notes: Mutex::new(vec![format!("detect failed: {e}")]),
                init_error: Some(e),
            },
        }
    }

    pub fn detection_report(&self) -> String {
        let mut lines = Vec::new();
        if let Some(e) = &self.init_error {
            lines.push(format!("error: {e}"));
        }
        if let Ok(n) = self.detect_notes.lock() {
            lines.extend(n.iter().cloned());
        }
        lines.join("\n")
    }

    pub fn device_count(&self) -> usize {
        self.devices.len()
    }
}

impl Default for PawnioProvider {
    fn default() -> Self {
        Self::probe()
    }
}

impl SensorProvider for PawnioProvider {
    fn name(&self) -> &str {
        "pawnio"
    }

    fn sensors(&self) -> Vec<SensorDescriptor> {
        let mut out = Vec::new();
        for (di, dev) in self.devices.iter().enumerate() {
            for ts in dev.temp_sources() {
                out.push(SensorDescriptor {
                    id: SensorId::new(format!("pawnio.{di}.temp.{}", ts.name)),
                    name: format!("SIO{di} {}", ts.name),
                    kind: SensorKind::Temperature,
                    provider: "pawnio".into(),
                    unit: Some("°C".into()),
                });
            }
            for fi in 0..dev.fan_count() {
                out.push(SensorDescriptor {
                    id: SensorId::new(format!("pawnio.{di}.fan{fi}")),
                    name: format!("SIO{di} Fan {fi}"),
                    kind: SensorKind::FanRpm,
                    provider: "pawnio".into(),
                    unit: Some("RPM".into()),
                });
            }
        }
        out
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        let s = id.as_str();
        let rest = s
            .strip_prefix("pawnio.")
            .ok_or_else(|| PluginError::SensorNotFound(s.into()))?;
        let mut parts = rest.splitn(2, '.');
        let di: usize = parts
            .next()
            .and_then(|p| p.parse().ok())
            .ok_or_else(|| PluginError::SensorNotFound(s.into()))?;
        let tail = parts
            .next()
            .ok_or_else(|| PluginError::SensorNotFound(s.into()))?;
        let dev = self
            .devices
            .get(di)
            .ok_or_else(|| PluginError::SensorNotFound(s.into()))?;

        if let Some(name) = tail.strip_prefix("temp.") {
            return dev
                .read_temp_named(name)
                .map_err(PluginError::Io)?
                .ok_or_else(|| PluginError::Other("temp out of range / missing".into()));
        }
        if let Some(idx) = tail.strip_prefix("fan") {
            let fi: usize = idx
                .parse()
                .map_err(|_| PluginError::SensorNotFound(s.into()))?;
            return dev
                .read_fan_rpm(fi)
                .map_err(PluginError::Io)?
                .ok_or_else(|| PluginError::Other("fan not present".into()));
        }
        Err(PluginError::SensorNotFound(s.into()))
    }
}

impl ControlProvider for PawnioProvider {
    fn name(&self) -> &str {
        "pawnio"
    }

    fn controls(&self) -> Vec<ControlDescriptor> {
        let mut out = Vec::new();
        for (di, dev) in self.devices.iter().enumerate() {
            for ci in 0..dev.control_count() {
                out.push(ControlDescriptor {
                    id: ControlId::new(format!("pawnio.{di}.ctrl{ci}")),
                    name: format!("SIO{di} Control {ci}"),
                    provider: "pawnio".into(),
                    writable: true,
                    rpm_sensor: Some(SensorId::new(format!("pawnio.{di}.fan{ci}"))),
                });
            }
        }
        out
    }

    fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()> {
        let (di, ci) = parse_ctrl(id.as_str())?;
        let dev = self
            .devices
            .get(di)
            .ok_or_else(|| PluginError::ControlNotFound(id.to_string()))?;
        dev.set_duty_percent(ci, percent).map_err(PluginError::Io)
    }

    fn get_duty(&self, id: &ControlId) -> Result<u8> {
        let (di, ci) = parse_ctrl(id.as_str())?;
        let dev = self
            .devices
            .get(di)
            .ok_or_else(|| PluginError::ControlNotFound(id.to_string()))?;
        dev.read_duty_percent(ci).map_err(PluginError::Io)
    }
}

fn parse_ctrl(s: &str) -> Result<(usize, usize)> {
    let rest = s
        .strip_prefix("pawnio.")
        .ok_or_else(|| PluginError::ControlNotFound(s.into()))?;
    let mut parts = rest.splitn(2, '.');
    let di: usize = parts
        .next()
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| PluginError::ControlNotFound(s.into()))?;
    let tail = parts
        .next()
        .ok_or_else(|| PluginError::ControlNotFound(s.into()))?;
    let ci: usize = tail
        .strip_prefix("ctrl")
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| PluginError::ControlNotFound(s.into()))?;
    Ok((di, ci))
}
