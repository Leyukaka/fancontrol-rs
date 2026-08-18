//! Plugin provider bridging Super I/O hardware into fancontrol traits.

use crate::cpu_power::{CpuPower, CpuPowerSample};
use crate::device::SuperIoDevice;
use crate::dimm_temp::DimmTemp;
use crate::nct668::HwmSample;
use crate::superio::detect_chips;
use fancontrol_core::{ControlDescriptor, ControlId, SensorDescriptor, SensorId, SensorKind};
use fancontrol_plugins::{ControlProvider, PluginError, Result, SensorProvider};
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// Sensor ids for CPU/DRAM power (host-facing, PawnIO MSR backend).
const CPU_POWER_PACKAGE_ID: &str = "host.cpu.power.package";
const CPU_POWER_LIMIT_ID: &str = "host.cpu.power.limit";
const RAM_POWER_ID: &str = "host.ram.power";

/// Reuse one MSR sample for all power ids within this window (avoids triple ΔE).
const CPU_POWER_SAMPLE_TTL: Duration = Duration::from_millis(200);

/// DIMM SMBus reads are slow (bus arbitration, per-byte transactions) and the
/// value barely moves - cache aggressively.
const DIMM_TEMP_SAMPLE_TTL: Duration = Duration::from_secs(3);

/// (index, temperature °C) pairs from the last DIMM SMBus sample.
type DimmTempSample = Vec<(usize, f64)>;

pub struct PawnioProvider {
    devices: Vec<SuperIoDevice>,
    detect_notes: Mutex<Vec<String>>,
    init_error: Option<String>,
    write_enabled: bool,
    /// AMD or Intel package power (PawnIO MSR). `None` when unavailable.
    cpu_power: Option<CpuPower>,
    /// Whether limit / DRAM sensors should be advertised (Intel RAPL).
    cpu_power_has_limit: bool,
    cpu_power_has_dram: bool,
    cpu_power_cache: Mutex<Option<(Instant, CpuPowerSample)>>,
    /// Experimental DDR5 DIMM temperature (SMBus SPD hub). `None` when no
    /// SMBus module opened or no DDR5 DIMM answered the probe.
    dimm_temp: Option<DimmTemp>,
    dimm_temp_cache: Mutex<Option<(Instant, DimmTempSample)>>,
}

impl PawnioProvider {
    pub fn probe() -> Self {
        Self::probe_with_writes(false)
    }

    pub fn probe_with_writes(write_enabled: bool) -> Self {
        let (cpu_power, cpu_power_note, has_limit, has_dram) = match CpuPower::try_open() {
            Ok((p, label)) => {
                // Probe once so we know which optional sensors to list.
                let (has_limit, has_dram) = match p.sample() {
                    Ok(s) => (s.limit_w.is_some(), s.dram_w.is_some()),
                    Err(_) => (false, false),
                };
                (
                    Some(p),
                    format!("{CPU_POWER_PACKAGE_ID}: {label} available"),
                    has_limit,
                    has_dram,
                )
            }
            Err(e) => {
                tracing::debug!(error = %e, "CPU package power unavailable");
                (
                    None,
                    format!("{CPU_POWER_PACKAGE_ID}: unavailable ({e})"),
                    false,
                    false,
                )
            }
        };

        let dimm_temp = match DimmTemp::try_open() {
            Ok(d) => Some(d),
            Err(e) => {
                tracing::debug!(error = %e, "DIMM temp (SMBus) unavailable");
                None
            }
        };
        let dimm_temp_note = match &dimm_temp {
            Some(d) if d.dimm_count() > 0 => format!(
                "host.dimm.temp: {} DDR5 DIMM(s) via {}",
                d.dimm_count(),
                d.module_label()
            ),
            Some(d) => format!(
                "host.dimm.temp: {} opened, no DDR5 SPD hub answered",
                d.module_label()
            ),
            None => "host.dimm.temp: unavailable (no SMBus module opened)".into(),
        };

        match detect_chips() {
            Ok(chips) => {
                let mut notes = Vec::new();
                let mut devices = Vec::new();
                notes.push(if write_enabled {
                    "mode=READ+WRITE (hardware writes enabled)".into()
                } else {
                    "mode=READ-ONLY (hardware writes blocked; drop --read-only / don't pass --read-only)"
                        .into()
                });
                notes.push(cpu_power_note);
                notes.push(dimm_temp_note);
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
                                dev.control_slots().len()
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
                    write_enabled,
                    cpu_power,
                    cpu_power_has_limit: has_limit,
                    cpu_power_has_dram: has_dram,
                    cpu_power_cache: Mutex::new(None),
                    dimm_temp,
                    dimm_temp_cache: Mutex::new(None),
                }
            }
            Err(e) => Self {
                devices: Vec::new(),
                detect_notes: Mutex::new(vec![
                    format!("detect failed: {e}"),
                    cpu_power_note,
                    dimm_temp_note,
                ]),
                init_error: Some(e),
                write_enabled,
                cpu_power,
                cpu_power_has_limit: has_limit,
                cpu_power_has_dram: has_dram,
                cpu_power_cache: Mutex::new(None),
                dimm_temp,
                dimm_temp_cache: Mutex::new(None),
            },
        }
    }

    fn cpu_power_sample(&self) -> std::result::Result<CpuPowerSample, PluginError> {
        let now = Instant::now();
        if let Ok(g) = self.cpu_power_cache.lock()
            && let Some((t, s)) = g.as_ref()
            && now.duration_since(*t) < CPU_POWER_SAMPLE_TTL
        {
            return Ok(s.clone());
        }
        let backend = self
            .cpu_power
            .as_ref()
            .ok_or_else(|| PluginError::SensorNotFound(CPU_POWER_PACKAGE_ID.into()))?;
        let s = backend.sample().map_err(PluginError::Io)?;
        if let Ok(mut g) = self.cpu_power_cache.lock() {
            *g = Some((now, s.clone()));
        }
        Ok(s)
    }

    /// Cached DIMM temperature sample (slow SMBus reads, TTL'd like CPU power).
    fn dimm_temp_sample(&self) -> DimmTempSample {
        let now = Instant::now();
        if let Ok(g) = self.dimm_temp_cache.lock()
            && let Some((t, s)) = g.as_ref()
            && now.duration_since(*t) < DIMM_TEMP_SAMPLE_TTL
        {
            return s.clone();
        }
        let Some(dimm_temp) = self.dimm_temp.as_ref() else {
            return Vec::new();
        };
        let s = dimm_temp.sample_all();
        if let Ok(mut g) = self.dimm_temp_cache.lock() {
            *g = Some((now, s.clone()));
        }
        s
    }

    pub fn write_enabled(&self) -> bool {
        self.write_enabled
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

    /// Batch-read all Super I/O channels (one bus lock per device).
    pub fn sample_all_devices(&self) -> Vec<(usize, HwmSample)> {
        let mut out = Vec::new();
        for (di, dev) in self.devices.iter().enumerate() {
            match dev.sample_all() {
                Ok(s) => out.push((di, s)),
                Err(e) => tracing::warn!(di, error = %e, "HWM sample_all failed"),
            }
        }
        out
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
        if self.cpu_power.is_some() {
            out.push(SensorDescriptor {
                id: SensorId::new(CPU_POWER_PACKAGE_ID),
                name: "CPU Package Power".into(),
                kind: SensorKind::Power,
                provider: "host".into(),
                unit: Some("W".into()),
            });
            if self.cpu_power_has_limit {
                out.push(SensorDescriptor {
                    id: SensorId::new(CPU_POWER_LIMIT_ID),
                    name: "CPU Power limit (TDP-ish)".into(),
                    kind: SensorKind::Power,
                    provider: "host".into(),
                    unit: Some("W".into()),
                });
            }
            if self.cpu_power_has_dram {
                out.push(SensorDescriptor {
                    id: SensorId::new(RAM_POWER_ID),
                    name: "DRAM Power".into(),
                    kind: SensorKind::Power,
                    provider: "host".into(),
                    unit: Some("W".into()),
                });
            }
        }
        if let Some(dimm_temp) = &self.dimm_temp {
            for i in 0..dimm_temp.dimm_count() {
                out.push(SensorDescriptor {
                    id: SensorId::new(format!("host.dimm{i}.temp")),
                    name: format!("DIMM {i} Temp"),
                    kind: SensorKind::Temperature,
                    provider: "host".into(),
                    unit: Some("°C".into()),
                });
            }
        }
        out
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        let s = id.as_str();
        if s == CPU_POWER_PACKAGE_ID {
            return Ok(self.cpu_power_sample()?.package_w);
        }
        if s == CPU_POWER_LIMIT_ID {
            return self
                .cpu_power_sample()?
                .limit_w
                .ok_or_else(|| PluginError::SensorNotFound(s.into()));
        }
        if s == RAM_POWER_ID {
            return self
                .cpu_power_sample()?
                .dram_w
                .ok_or_else(|| PluginError::SensorNotFound(s.into()));
        }
        if let Some(tail) = s.strip_prefix("host.dimm")
            && let Some(idx_str) = tail.strip_suffix(".temp")
        {
            let idx: usize = idx_str
                .parse()
                .map_err(|_| PluginError::SensorNotFound(s.into()))?;
            return self
                .dimm_temp_sample()
                .into_iter()
                .find(|(i, _)| *i == idx)
                .map(|(_, t)| t)
                .ok_or_else(|| PluginError::Other("DIMM temp out of range / missing".into()));
        }
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
            // Present but stopped → 0.0; disconnected header → error (filtered in UI unless --all)
            return match dev.read_fan_rpm(fi).map_err(PluginError::Io)? {
                Some(v) => Ok(v),
                None => Err(PluginError::Other("fan not present".into())),
            };
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
            for (slot, rpm_idx) in dev.control_slots() {
                let rpm_sensor = rpm_idx.map(|fi| SensorId::new(format!("pawnio.{di}.fan{fi}")));
                out.push(ControlDescriptor {
                    id: ControlId::new(format!("pawnio.{di}.ctrl{slot}")),
                    name: format!("SIO{di} Control {slot}"),
                    provider: "pawnio".into(),
                    writable: self.write_enabled,
                    rpm_sensor,
                });
            }
        }
        out
    }

    fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()> {
        if !self.write_enabled {
            return Err(PluginError::NotWritable(format!(
                "{id}: hardware writes disabled (read-only mode). Re-run without --read-only when ready."
            )));
        }
        let (di, slot) = parse_ctrl(id.as_str())?;
        let dev = self
            .devices
            .get(di)
            .ok_or_else(|| PluginError::ControlNotFound(id.to_string()))?;
        dev.set_duty_percent(slot, percent).map_err(PluginError::Io)
    }

    fn get_duty(&self, id: &ControlId) -> Result<u8> {
        let (di, slot) = parse_ctrl(id.as_str())?;
        let dev = self
            .devices
            .get(di)
            .ok_or_else(|| PluginError::ControlNotFound(id.to_string()))?;
        dev.read_duty_percent(slot).map_err(PluginError::Io)
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
    let slot: usize = tail
        .strip_prefix("ctrl")
        .and_then(|p| p.parse().ok())
        .ok_or_else(|| PluginError::ControlNotFound(s.into()))?;
    Ok((di, slot))
}
