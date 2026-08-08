//! CPU package (and optional DRAM) power via PawnIO MSR modules.
//!
//! - **AMD** Zen 17h–1Ah: `AMDFamily17` — `MSR_PWR_UNIT` / `MSR_PKG_ENERGY_STAT`
//! - **Intel** RAPL: `IntelMSR` — unit / PKG energy / PKG power info / DRAM energy
//!
//! Power is Δ energy / Δt. Read-only; no PL1/PL2 writes.

use crate::session::PawnSession;
use std::sync::Mutex;
use std::time::Instant;

/// Below this interval a delta is too noisy — reuse previous watts.
const MIN_SAMPLE_INTERVAL_SECS: f64 = 0.05;

// AMD
const AMD_MSR_PWR_UNIT: u32 = 0xC001_0299;
const AMD_MSR_PKG_ENERGY_STAT: u32 = 0xC001_029B;

// Intel RAPL
const INTEL_MSR_RAPL_POWER_UNIT: u32 = 0x0000_0606;
const INTEL_MSR_PKG_ENERGY_STATUS: u32 = 0x0000_0611;
const INTEL_MSR_PKG_POWER_INFO: u32 = 0x0000_0614;
const INTEL_MSR_DRAM_ENERGY_STATUS: u32 = 0x0000_0619;

struct EnergyState {
    last_counter: u32,
    last_time: Instant,
    last_watts: f64,
}

/// Live package / optional DRAM readings.
#[derive(Debug, Clone, Default)]
pub struct CpuPowerSample {
    pub package_w: f64,
    /// Thermal-spec / package power info when available (Intel).
    pub limit_w: Option<f64>,
    pub dram_w: Option<f64>,
}

/// Open MSR backend for the CPU vendor present on this machine.
pub enum CpuPower {
    Amd(AmdCpuPower),
    Intel(IntelCpuPower),
}

impl CpuPower {
    /// Prefer AMD module, then Intel. Either can fail with admin / unsupported CPU.
    pub fn try_open() -> Result<(Self, &'static str), String> {
        match AmdCpuPower::try_open() {
            Ok(a) => Ok((Self::Amd(a), "AMD MSR (AMDFamily17)")),
            Err(amd_e) => match IntelCpuPower::try_open() {
                Ok(i) => Ok((Self::Intel(i), "Intel RAPL (IntelMSR)")),
                Err(intel_e) => Err(format!("AMD: {amd_e}; Intel: {intel_e}")),
            },
        }
    }

    pub fn sample(&self) -> Result<CpuPowerSample, String> {
        match self {
            Self::Amd(a) => a.sample(),
            Self::Intel(i) => i.sample(),
        }
    }
}

/// AMD Zen package power only (no limit/DRAM MSR in AMDFamily17).
pub struct AmdCpuPower {
    session: PawnSession,
    energy_unit_joules: f64,
    state: Mutex<Option<EnergyState>>,
}

impl AmdCpuPower {
    pub fn try_open() -> Result<Self, String> {
        let session = PawnSession::open_embedded("AMDFamily17")?;
        let pwr_unit = read_msr(&session, AMD_MSR_PWR_UNIT)?;
        Ok(Self {
            session,
            energy_unit_joules: amd_energy_unit_joules(pwr_unit),
            state: Mutex::new(None),
        })
    }

    pub fn sample(&self) -> Result<CpuPowerSample, String> {
        let raw = read_msr(&self.session, AMD_MSR_PKG_ENERGY_STAT)?;
        let w = energy_delta_update(
            &self.state,
            raw as u32,
            self.energy_unit_joules,
            Instant::now(),
        );
        Ok(CpuPowerSample {
            package_w: w,
            limit_w: None,
            dram_w: None,
        })
    }
}

/// Intel RAPL package + optional thermal-spec limit + DRAM domain.
pub struct IntelCpuPower {
    session: PawnSession,
    energy_unit_joules: f64,
    power_unit_watts: f64,
    limit_w: Option<f64>,
    pkg: Mutex<Option<EnergyState>>,
    dram: Mutex<Option<EnergyState>>,
    /// False after DRAM MSR read fails once (unsupported / filtered).
    dram_ok: Mutex<bool>,
}

impl IntelCpuPower {
    pub fn try_open() -> Result<Self, String> {
        let session = PawnSession::open_embedded("IntelMSR")?;
        let units = read_msr(&session, INTEL_MSR_RAPL_POWER_UNIT)?;
        let energy_unit_joules = intel_energy_unit_joules(units);
        let power_unit_watts = intel_power_unit_watts(units);
        let limit_w = match read_msr(&session, INTEL_MSR_PKG_POWER_INFO) {
            Ok(info) => {
                let raw = info & 0x7FFF;
                let w = raw as f64 * power_unit_watts;
                if w.is_finite() && w > 1.0 {
                    Some(w)
                } else {
                    None
                }
            }
            Err(_) => None,
        };
        Ok(Self {
            session,
            energy_unit_joules,
            power_unit_watts,
            limit_w,
            pkg: Mutex::new(None),
            dram: Mutex::new(None),
            dram_ok: Mutex::new(true),
        })
    }

    pub fn sample(&self) -> Result<CpuPowerSample, String> {
        let now = Instant::now();
        let pkg_raw = read_msr(&self.session, INTEL_MSR_PKG_ENERGY_STATUS)?;
        let package_w =
            energy_delta_update(&self.pkg, pkg_raw as u32, self.energy_unit_joules, now);

        let dram_w = {
            let ok = *self.dram_ok.lock().unwrap_or_else(|e| e.into_inner());
            if !ok {
                None
            } else {
                match read_msr(&self.session, INTEL_MSR_DRAM_ENERGY_STATUS) {
                    Ok(raw) => Some(energy_delta_update(
                        &self.dram,
                        raw as u32,
                        self.energy_unit_joules,
                        now,
                    )),
                    Err(_) => {
                        *self.dram_ok.lock().unwrap_or_else(|e| e.into_inner()) = false;
                        None
                    }
                }
            }
        };

        let _ = self.power_unit_watts; // used at open for limit
        Ok(CpuPowerSample {
            package_w,
            limit_w: self.limit_w,
            dram_w,
        })
    }
}

fn read_msr(session: &PawnSession, msr: u32) -> Result<u64, String> {
    let out = session.execute("ioctl_read_msr", &[u64::from(msr)], 1)?;
    Ok(out.first().copied().unwrap_or(0))
}

/// AMD `MSR_PWR_UNIT` energy-status unit (bits 12:8) → joules / increment.
fn amd_energy_unit_joules(pwr_unit_msr: u64) -> f64 {
    let esu = ((pwr_unit_msr >> 8) & 0x1F) as i32;
    0.5f64.powi(esu)
}

/// Intel RAPL energy status unit (bits 12:8).
fn intel_energy_unit_joules(rapl_unit: u64) -> f64 {
    let esu = ((rapl_unit >> 8) & 0x1F) as i32;
    0.5f64.powi(esu)
}

/// Intel RAPL power unit (bits 3:0) → watts scale for power-info fields.
fn intel_power_unit_watts(rapl_unit: u64) -> f64 {
    let pu = (rapl_unit & 0xF) as i32;
    0.5f64.powi(pu)
}

fn energy_delta_update(
    state: &Mutex<Option<EnergyState>>,
    counter: u32,
    energy_unit_joules: f64,
    now: Instant,
) -> f64 {
    let mut g = state.lock().unwrap_or_else(|e| e.into_inner());
    let watts = match g.as_ref() {
        None => 0.0,
        Some(prev) => {
            let dt = now.duration_since(prev.last_time).as_secs_f64();
            if dt < MIN_SAMPLE_INTERVAL_SECS {
                prev.last_watts
            } else {
                energy_delta_to_watts(counter, prev.last_counter, energy_unit_joules, dt)
            }
        }
    };
    *g = Some(EnergyState {
        last_counter: counter,
        last_time: now,
        last_watts: watts,
    });
    watts
}

fn energy_delta_to_watts(
    current: u32,
    previous: u32,
    energy_unit_joules: f64,
    delta_secs: f64,
) -> f64 {
    if delta_secs <= 0.0 {
        return 0.0;
    }
    let delta = current.wrapping_sub(previous);
    energy_unit_joules * f64::from(delta) / delta_secs
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn amd_default_energy_unit() {
        let pwr_unit = 0x10_u64 << 8;
        let unit = amd_energy_unit_joules(pwr_unit);
        assert!((unit - 1.0 / 65536.0).abs() < 1e-12);
    }

    #[test]
    fn intel_units_match_common_defaults() {
        // Common: PU=3 → 1/8 W, ESU=16 → 1/65536 J
        let u = (0x10_u64 << 8) | 0x3;
        assert!((intel_power_unit_watts(u) - 0.125).abs() < 1e-12);
        assert!((intel_energy_unit_joules(u) - 1.0 / 65536.0).abs() < 1e-12);
    }

    #[test]
    fn watts_from_simple_delta() {
        let unit = 1.0 / 65536.0;
        let w = energy_delta_to_watts(65536, 0, unit, 1.0);
        assert!((w - 1.0).abs() < 1e-9);
    }

    #[test]
    fn watts_handle_counter_wrap() {
        let unit = 1.0 / 65536.0;
        let previous = u32::MAX - 100;
        let current = 200u32;
        let w = energy_delta_to_watts(current, previous, unit, 1.0);
        let expected = unit * 301.0;
        assert!((w - expected).abs() < 1e-9);
    }

    #[test]
    fn zero_interval_is_zero() {
        assert_eq!(energy_delta_to_watts(100, 50, 1e-5, 0.0), 0.0);
    }
}
