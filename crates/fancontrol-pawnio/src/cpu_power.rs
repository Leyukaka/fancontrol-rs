//! AMD CPU package power via the PawnIO `AMDFamily17` module (Zen family 17h-1Ah).
//!
//! Read-only MSR access: `MSR_PWR_UNIT` for the energy scale, `MSR_PKG_ENERGY_STAT`
//! for the running package energy counter. Package watts is derived from the energy
//! delta between two reads divided by the elapsed time, the same approach as
//! LibreHardwareMonitor's `Amd17Cpu` (MPL-2.0, register map only, no code copied).
//!
//! Intel RAPL (`IntelMSR`) and the DRAM energy domain are a later task — the
//! `AMDFamily17` module does not expose a package power-limit MSR, so
//! `host.cpu.power.limit` / `host.ram.power` stay unavailable on AMD for now
//! rather than being estimated.

use crate::session::PawnSession;
use std::sync::Mutex;
use std::time::Instant;

const MSR_PWR_UNIT: u32 = 0xC001_0299;
const MSR_PKG_ENERGY_STAT: u32 = 0xC001_029B;

/// Below this interval a delta is too noisy to trust (e.g. two polls landing
/// a couple of milliseconds apart) — the previous wattage is reused instead.
const MIN_SAMPLE_INTERVAL_SECS: f64 = 0.05;

struct EnergyState {
    last_counter: u32,
    last_time: Instant,
    last_watts: f64,
}

/// One open `AMDFamily17` session plus running energy-counter state.
pub struct AmdCpuPower {
    session: PawnSession,
    energy_unit_joules: f64,
    state: Mutex<Option<EnergyState>>,
}

impl AmdCpuPower {
    /// Open the module and read the energy unit scale once.
    ///
    /// The module's own `main()` returns `STATUS_NOT_SUPPORTED` on non-AMD CPUs
    /// or AMD families outside 17h-1Ah, which surfaces here as `Err` — callers
    /// should treat any error as "sensor not available", not a hard failure.
    pub fn try_open() -> Result<Self, String> {
        let session = PawnSession::open_embedded("AMDFamily17")?;
        let pwr_unit = read_msr(&session, MSR_PWR_UNIT)?;
        Ok(Self {
            session,
            energy_unit_joules: energy_unit_joules_from_pwr_unit(pwr_unit),
            state: Mutex::new(None),
        })
    }

    /// Package power in watts, derived from the energy counter delta since the
    /// previous call. Returns `0.0` for the first call (no prior sample yet) and
    /// while warming up (see [`MIN_SAMPLE_INTERVAL_SECS`]) rather than an error,
    /// so the UI does not flash a transient error on startup.
    pub fn sample_watts(&self) -> Result<f64, String> {
        let raw = read_msr(&self.session, MSR_PKG_ENERGY_STAT)?;
        let counter = raw as u32;
        let now = Instant::now();

        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let watts = match state.as_ref() {
            None => 0.0,
            Some(prev) => {
                let dt = now.duration_since(prev.last_time).as_secs_f64();
                if dt < MIN_SAMPLE_INTERVAL_SECS {
                    prev.last_watts
                } else {
                    energy_delta_to_watts(counter, prev.last_counter, self.energy_unit_joules, dt)
                }
            }
        };
        *state = Some(EnergyState {
            last_counter: counter,
            last_time: now,
            last_watts: watts,
        });
        Ok(watts)
    }
}

fn read_msr(session: &PawnSession, msr: u32) -> Result<u64, String> {
    let out = session.execute("ioctl_read_msr", &[u64::from(msr)], 1)?;
    Ok(out.first().copied().unwrap_or(0))
}

/// `MSR_PWR_UNIT` energy-status-unit field (bits 12:8) -> joules per counter increment.
/// AMD's documented default is ESU=16 (15.3 microjoules/increment).
fn energy_unit_joules_from_pwr_unit(pwr_unit_msr: u64) -> f64 {
    let esu = ((pwr_unit_msr >> 8) & 0x1F) as i32;
    0.5f64.powi(esu)
}

/// 32-bit energy counter delta (wrapping) -> average watts over `delta_secs`.
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
    fn default_energy_unit_matches_amd_docs() {
        // ESU=16 (bits 12:8 = 0x10) is AMD's documented default: 15.3 uJ/increment.
        let pwr_unit = 0x10_u64 << 8;
        let unit = energy_unit_joules_from_pwr_unit(pwr_unit);
        assert!((unit - 1.0 / 65536.0).abs() < 1e-12);
        assert!((unit * 1e6 - 15.259).abs() < 0.01);
    }

    #[test]
    fn watts_from_simple_delta() {
        let unit = energy_unit_joules_from_pwr_unit(0x10_u64 << 8);
        // 65536 counts over exactly 1s at this unit = 1 J/s = 1 W.
        let w = energy_delta_to_watts(65536, 0, unit, 1.0);
        assert!((w - 1.0).abs() < 1e-9);
    }

    #[test]
    fn watts_handle_counter_wrap() {
        let unit = energy_unit_joules_from_pwr_unit(0x10_u64 << 8);
        let previous = u32::MAX - 100;
        let current = 200u32;
        let w = energy_delta_to_watts(current, previous, unit, 1.0);
        // 100 counts to reach MAX, +1 to wrap to 0, +200 more = 301.
        let expected = unit * 301.0;
        assert!((w - expected).abs() < 1e-9);
    }

    #[test]
    fn zero_or_negative_interval_is_zero_watts() {
        let unit = energy_unit_joules_from_pwr_unit(0x10_u64 << 8);
        assert_eq!(energy_delta_to_watts(100, 50, unit, 0.0), 0.0);
    }

    #[test]
    fn higher_esu_gives_smaller_unit() {
        let unit_16 = energy_unit_joules_from_pwr_unit(0x10_u64 << 8);
        let unit_17 = energy_unit_joules_from_pwr_unit(0x11_u64 << 8);
        assert!(unit_17 < unit_16);
        assert!((unit_17 - unit_16 / 2.0).abs() < 1e-15);
    }
}
