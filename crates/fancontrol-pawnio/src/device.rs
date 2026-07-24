//! Unified Super I/O device handle (banked NCT + EC NCT668x).

use crate::nct668::Nct668Device;
use crate::superio::{DetectedChip, NctBankedDevice, TempSource};

pub enum SuperIoDevice {
    Banked(NctBankedDevice),
    Nct668(Nct668Device),
}

impl SuperIoDevice {
    pub fn try_open(detected: &DetectedChip) -> Result<Self, String> {
        match detected.chip {
            crate::superio::SuperIoChip::NctBanked { .. } => {
                NctBankedDevice::try_open(detected).map(Self::Banked)
            }
            crate::superio::SuperIoChip::Nct668x { .. } => {
                Nct668Device::try_open(detected).map(Self::Nct668)
            }
            _ => Err(format!(
                "no HWM driver for chip {}",
                detected.chip.name()
            )),
        }
    }

    pub fn hwm_address(&self) -> u16 {
        match self {
            Self::Banked(d) => d.hwm_address(),
            Self::Nct668(d) => d.hwm_address(),
        }
    }

    pub fn kind_label(&self) -> &'static str {
        match self {
            Self::Banked(_) => "banked",
            Self::Nct668(_) => "nct668-ec",
        }
    }

    pub fn fan_count(&self) -> usize {
        match self {
            Self::Banked(d) => d.fan_count(),
            Self::Nct668(d) => d.fan_count(),
        }
    }

    pub fn control_count(&self) -> usize {
        match self {
            Self::Banked(d) => d.control_count(),
            Self::Nct668(d) => d.control_count(),
        }
    }

    pub fn temp_sources(&self) -> Vec<TempSource> {
        match self {
            Self::Banked(d) => d.temp_sources(),
            Self::Nct668(d) => d.temp_sources(),
        }
    }

    pub fn read_temp_named(&self, name: &str) -> Result<Option<f64>, String> {
        match self {
            Self::Banked(d) => {
                for ts in d.temp_sources() {
                    if ts.name == name {
                        return d.read_temp_c(ts.reg, ts.half);
                    }
                }
                Err(format!("unknown temp {name}"))
            }
            Self::Nct668(d) => {
                for ts in d.temp_sources() {
                    if ts.name == name {
                        return d.read_temp_c(ts.reg);
                    }
                }
                Err(format!("unknown temp {name}"))
            }
        }
    }

    pub fn read_fan_rpm(&self, index: usize) -> Result<Option<f64>, String> {
        match self {
            Self::Banked(d) => d.read_fan_rpm(index),
            Self::Nct668(d) => d.read_fan_rpm(index),
        }
    }

    pub fn read_duty_percent(&self, index: usize) -> Result<u8, String> {
        match self {
            Self::Banked(d) => d.read_duty_percent(index),
            Self::Nct668(d) => d.read_duty_percent(index),
        }
    }

    pub fn set_duty_percent(&self, index: usize, percent: u8) -> Result<(), String> {
        match self {
            Self::Banked(d) => d.set_duty_percent(index, percent),
            Self::Nct668(d) => d.set_duty_percent(index, percent),
        }
    }
}
