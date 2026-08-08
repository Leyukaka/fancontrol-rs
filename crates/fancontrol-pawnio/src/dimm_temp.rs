//! **Experimental** DDR5 DIMM temperature via SMBus SPD hub (JEDEC SPD5118-class).
//!
//! Owner target: AMD platform, NCT668x EC, DDR5. Opens the PawnIO `SmbusPIIX4`
//! module first (AMD chipset SMBus controller), falling back to `SmbusI801`
//! (Intel ICH/PCH). Probes the standard SPD address range `0x50..=0x57` for a
//! DDR5 SPD hub signature, then reads the on-die thermal sensor if enabled.
//!
//! Register offsets are a **map only** (not code) derived from the public
//! JEDEC SPD5118 layout, cross-referenced against the offsets used by
//! [RAMSPDToolkit](https://github.com/GordonMcGregor/RAMSPDToolkit) (MPL-2.0).
//! No source from that project is copied here.
//!
//! Read-only: no SPD writes are ever issued. Every bus transaction goes
//! through a short timeout so a wedged / absent device cannot hang sampling.

use crate::mutex_smbus::SmbusGuard;
use crate::session::PawnSession;
use std::time::Duration;

/// SMBus protocol codes for `ioctl_smbus_xfer` (SMBus spec protocol field).
const PROTOCOL_BYTE_DATA: u64 = 2;
const PROTOCOL_WORD_DATA: u64 = 3;

const SMBUS_READ: u64 = 1;
const SMBUS_WRITE: u64 = 0;

/// Standard SPD/SPD5 hub address range on the SMBus.
const SPD_ADDR_FIRST: u8 = 0x50;
const SPD_ADDR_LAST: u8 = 0x57;

// JEDEC SPD5118 (DDR5 SPD hub) register map (offsets only).
// Map cross-checked against public JEDEC layout / RAMSPDToolkit constants (MPL map only).
const REG_DEVICE_TYPE_MOST: u64 = 0x00;
const REG_DEVICE_TYPE_LEAST: u64 = 0x01;
const REG_DEVICE_CAPABILITY: u64 = 0x05;
/// MR26: **0 = thermal sensor enabled**, non-zero = disabled (JEDEC / RAMSPDToolkit).
const REG_THERMAL_SENSOR_ENABLED: u64 = 0x1A;
const REG_TEMPERATURE: u64 = 0x31;

const DEVICE_TYPE_MOST_DDR5: u8 = 0x51;
const DEVICE_TYPE_LEAST_DDR5: u8 = 0x18;
/// Bit 1 of DEVICE_CAPABILITY: hub reports an on-die temperature sensor.
const CAP_TEMP_SENSOR_BIT: u8 = 1 << 1;

/// Sign bit of the 13-bit two's-complement temperature field.
const TEMP_SIGN_BIT: u16 = 0x1000;
/// Reserved/alarm bits (15:13) above the 13-bit temperature field.
const TEMP_VALUE_MASK: u16 = 0x1FFF;
const TEMP_LSB_C: f64 = 0.0625;

const XFER_TIMEOUT: Duration = Duration::from_millis(100);

/// One detected DDR5 SPD hub with a live thermal sensor.
struct DimmSlot {
    /// SMBus 7-bit address (0x50..=0x57).
    addr: u8,
}

/// Read-only DDR5 DIMM temperature backend.
pub struct DimmTemp {
    session: PawnSession,
    module_label: &'static str,
    slots: Vec<DimmSlot>,
}

impl DimmTemp {
    /// Try `SmbusPIIX4` (AMD) then `SmbusI801` (Intel), probe for DDR5 SPD
    /// hubs with an active thermal sensor. `Ok` with an empty slot list is a
    /// normal outcome (module opened but no DDR5 DIMM answered) — only
    /// `Err` when neither SMBus module could be opened at all (missing
    /// elevation / unsupported chipset).
    pub fn try_open() -> Result<Self, String> {
        let (session, module_label) = match PawnSession::open_embedded("SmbusPIIX4") {
            Ok(s) => (s, "SmbusPIIX4 (AMD)"),
            Err(piix4_err) => match PawnSession::open_embedded("SmbusI801") {
                Ok(s) => (s, "SmbusI801 (Intel)"),
                Err(i801_err) => {
                    return Err(format!("PIIX4: {piix4_err}; I801: {i801_err}"));
                }
            },
        };

        let slots = probe_ddr5_slots(&session);
        Ok(Self {
            session,
            module_label,
            slots,
        })
    }

    pub fn module_label(&self) -> &'static str {
        self.module_label
    }

    pub fn dimm_count(&self) -> usize {
        self.slots.len()
    }

    /// Read all detected DIMM temperatures (°C), skipping any that fail.
    pub fn sample_all(&self) -> Vec<(usize, f64)> {
        let _guard = SmbusGuard::acquire(XFER_TIMEOUT);
        let mut out = Vec::with_capacity(self.slots.len());
        for (i, slot) in self.slots.iter().enumerate() {
            match read_temperature_c(&self.session, slot.addr) {
                Ok(t) => out.push((i, t)),
                Err(e) => {
                    tracing::debug!(addr = format!("0x{:02X}", slot.addr), error = %e, "DIMM temp read failed")
                }
            }
        }
        out
    }
}

fn probe_ddr5_slots(session: &PawnSession) -> Vec<DimmSlot> {
    let _guard = SmbusGuard::acquire(XFER_TIMEOUT);
    let mut slots = Vec::new();
    for addr in SPD_ADDR_FIRST..=SPD_ADDR_LAST {
        if is_ddr5_spd_hub(session, addr) {
            slots.push(DimmSlot { addr });
        }
    }
    slots
}

fn is_ddr5_spd_hub(session: &PawnSession, addr: u8) -> bool {
    let most = match smbus_read_byte(session, addr, REG_DEVICE_TYPE_MOST) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if most != DEVICE_TYPE_MOST_DDR5 {
        return false;
    }
    let least = match smbus_read_byte(session, addr, REG_DEVICE_TYPE_LEAST) {
        Ok(v) => v,
        Err(_) => return false,
    };
    if least != DEVICE_TYPE_LEAST_DDR5 {
        return false;
    }
    // Prefer hubs that advertise a temp sensor in capability (bit 1). If the
    // capability read fails, still accept the 0x5118 signature — owner boards
    // may answer device-type but flake on 0x05 under contention.
    match smbus_read_byte(session, addr, REG_DEVICE_CAPABILITY) {
        Ok(cap) if cap & CAP_TEMP_SENSOR_BIT == 0 => {
            tracing::debug!(
                addr = format!("0x{:02X}", addr),
                cap,
                "DDR5 hub without temp-sensor capability bit; skipping"
            );
            false
        }
        Ok(_) | Err(_) => true,
    }
}

fn read_temperature_c(session: &PawnSession, addr: u8) -> Result<f64, String> {
    // JEDEC / RAMSPDToolkit: 0 = thermal sensor enabled, non-zero = disabled.
    let enabled_reg = smbus_read_byte(session, addr, REG_THERMAL_SENSOR_ENABLED)?;
    if enabled_reg != 0 {
        return Err(format!(
            "thermal sensor disabled (MR26=0x{enabled_reg:02X})"
        ));
    }
    let raw = smbus_read_word(session, addr, REG_TEMPERATURE).or_else(|e| -> Result<u16, String> {
        // Some controllers flake on WORD_DATA; fall back to two BYTE_DATA reads
        // (MR49 low, MR50 high) and assemble little-endian like PIIX4 WORD_DATA.
        tracing::trace!(addr = format!("0x{:02X}", addr), error = %e, "DIMM word temp failed; trying byte pair");
        let lo = smbus_read_byte(session, addr, REG_TEMPERATURE)?;
        let hi = smbus_read_byte(session, addr, REG_TEMPERATURE + 1)?;
        Ok(u16::from(lo) | (u16::from(hi) << 8))
    })?;
    Ok(ddr5_temp_from_raw(raw))
}

/// JEDEC SPD5118-style conversion: 13-bit two's-complement, 0.0625 °C/LSB.
fn ddr5_temp_from_raw(raw: u16) -> f64 {
    let v = raw & TEMP_VALUE_MASK;
    if v & TEMP_SIGN_BIT != 0 {
        f64::from(v & !TEMP_SIGN_BIT) * TEMP_LSB_C - 256.0
    } else {
        f64::from(v) * TEMP_LSB_C
    }
}

fn smbus_read_byte(session: &PawnSession, addr: u8, reg: u64) -> Result<u8, String> {
    let input = [u64::from(addr), SMBUS_READ, reg, PROTOCOL_BYTE_DATA];
    let out = session.execute("ioctl_smbus_xfer", &input, 1)?;
    let v = out.first().copied().ok_or("empty SMBus response")?;
    Ok(v as u8)
}

fn smbus_read_word(session: &PawnSession, addr: u8, reg: u64) -> Result<u16, String> {
    let input = [u64::from(addr), SMBUS_READ, reg, PROTOCOL_WORD_DATA];
    let out = session.execute("ioctl_smbus_xfer", &input, 1)?;
    let v = out.first().copied().ok_or("empty SMBus response")?;
    Ok(v as u16)
}

// Suppress unused-const warnings for the write direction constant: it
// documents the ioctl shape even though this module never issues writes.
const _: u64 = SMBUS_WRITE;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_temperature_conversion() {
        // 40.0 °C = 640 * 0.0625, no sign bit.
        let raw = 640u16;
        assert!((ddr5_temp_from_raw(raw) - 40.0).abs() < 1e-9);
    }

    #[test]
    fn negative_temperature_conversion() {
        // -10 °C: two's complement 13-bit value = 8192 - 160 = 8032 (0x1F60).
        let raw = 0x1F60u16;
        assert!((ddr5_temp_from_raw(raw) - (-10.0)).abs() < 1e-9);
    }

    #[test]
    fn zero_is_zero() {
        assert!((ddr5_temp_from_raw(0) - 0.0).abs() < 1e-9);
    }

    #[test]
    fn reserved_alarm_bits_ignored() {
        // Bits 13-15 set (alarm flags) shouldn't perturb the magnitude.
        let raw = 640u16 | 0xE000;
        assert!((ddr5_temp_from_raw(raw) - 40.0).abs() < 1e-9);
    }

    #[test]
    fn bus_pirate_example_about_21c() {
        // Bus Pirate DDR5 demo: MR49=0x54, MR50=0x01 → LE word 0x0154 ≈ 21.25 °C
        // at 0.0625 °C/LSB (matches their ~21 °C at 9-bit/0.5 packing).
        let raw = u16::from(0x54u8) | (u16::from(0x01u8) << 8);
        assert!((ddr5_temp_from_raw(raw) - 21.25).abs() < 1e-9);
    }
}
