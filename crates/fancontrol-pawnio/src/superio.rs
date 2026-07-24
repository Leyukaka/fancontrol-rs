//! Super I/O chip detection and Nuvoton banked hardware-monitor access.
//!
//! Detection sequences adapted from LibreHardwareMonitor (MPL-2.0).
//! Sensor register maps for the common NCT679x family (not NCT668x EC).

use crate::lpcio::LpcIo;
use crate::mutex_isa::IsaBusGuard;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const CHIP_ID_REGISTER: u8 = 0x20;
const CHIP_REVISION_REGISTER: u8 = 0x21;
const BASE_ADDRESS_REGISTER: u8 = 0x60;
const WINBOND_NUVOTON_HWM_LDN: u8 = 0x0B;
const NUVOTON_IO_SPACE_LOCK: u8 = 0x28;
const ADDR_OFF: u16 = 0x05;
const DATA_OFF: u16 = 0x06;
const BANK_SELECT: u8 = 0x4E;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuperIoChip {
    Unknown {
        id: u8,
        revision: u8,
    },
    /// Generic banked Nuvoton NCT6779/NCT679x-class (not 668x).
    NctBanked {
        id: u8,
        revision: u8,
    },
    Nct668x {
        id: u8,
        revision: u8,
    },
    It87 {
        chip_id: u16,
    },
}

impl SuperIoChip {
    pub fn name(&self) -> String {
        match self {
            SuperIoChip::Unknown { id, revision } => {
                format!("Unknown SuperIO id=0x{id:02X} rev=0x{revision:02X}")
            }
            SuperIoChip::NctBanked { id, revision } => {
                format!("Nuvoton NCT(banked) id=0x{id:02X} rev=0x{revision:02X}")
            }
            SuperIoChip::Nct668x { id, revision } => {
                format!("Nuvoton NCT668x id=0x{id:02X} rev=0x{revision:02X}")
            }
            SuperIoChip::It87 { chip_id } => format!("ITE IT87xx chip=0x{chip_id:04X}"),
        }
    }

    pub fn supports_banked_hwm(&self) -> bool {
        matches!(self, SuperIoChip::NctBanked { .. })
    }

    pub fn supports_nct668_ec(&self) -> bool {
        matches!(self, SuperIoChip::Nct668x { .. })
    }

    pub fn has_hwm_driver(&self) -> bool {
        self.supports_banked_hwm() || self.supports_nct668_ec()
    }
}

fn classify_winbond(id: u8, revision: u8) -> SuperIoChip {
    // NCT668x family uses EC-style page/index/data access.
    // 0xD5/0x92 = NCT6687D (or MSI NCT6687DR variant — same EC bus, mode bits differ on write).
    if matches!(id, 0xC7) || matches!((id, revision), (0xD4, 0x40 | 0x41) | (0xD5, 0x92)) {
        return SuperIoChip::Nct668x { id, revision };
    }
    // Known banked NCT / Winbond monitor chips (subset of LHM map).
    let banked = matches!(
        id,
        0xB4 | 0xC3 | 0xC4 | 0xC5 | 0xC8 | 0xC9 | 0xD1 | 0xD3 | 0xD4 | 0xD8
    ) || matches!(
        (id, revision & 0xF0),
        (0xB4, 0x70) | (0xC3, 0x30) | (0xC5, 0x60) | (0xC4, 0x50)
    );
    if banked
        || matches!(
            id,
            0xC8 | 0xC9 | 0xD1 | 0xD3 | 0xD4 | 0xD8 | 0xB4 | 0xC3 | 0xC5
        )
    {
        SuperIoChip::NctBanked { id, revision }
    } else {
        SuperIoChip::Unknown { id, revision }
    }
}

#[derive(Debug, Clone)]
pub struct DetectedChip {
    pub slot: u8,
    pub register_port: u16,
    pub chip: SuperIoChip,
    pub hwm_address: Option<u16>,
}

/// Detect Super I/O chips on both LPC slots (read-only config probes).
pub fn detect_chips() -> Result<Vec<DetectedChip>, String> {
    let _guard = IsaBusGuard::acquire(Duration::from_millis(200));
    let lpc = LpcIo::open()?;
    let mut found = Vec::new();

    for slot in 0u8..2 {
        lpc.select_slot(slot as u32)?;
        let reg_port: u16 = if slot == 0 { 0x2E } else { 0x4E };

        // --- Winbond / Nuvoton / Fintek enter ---
        lpc.write_port(reg_port, 0x87)?;
        lpc.write_port(reg_port, 0x87)?;
        let id = lpc.superio_inb(CHIP_ID_REGISTER)?;
        let revision = lpc.superio_inb(CHIP_REVISION_REGISTER)?;

        if id != 0 && id != 0xFF {
            let chip = classify_winbond(id, revision);
            let mut hwm = None;
            if chip.has_hwm_driver() {
                let _ = lpc.find_bars();
                let _ = lpc.select_ldn(WINBOND_NUVOTON_HWM_LDN);
                // Disable IO space lock on NCT679x
                if let Ok(options) = lpc.superio_inb(NUVOTON_IO_SPACE_LOCK) {
                    if options & 0x10 != 0 {
                        let _ = lpc.superio_outb(NUVOTON_IO_SPACE_LOCK, options & !0x10);
                    }
                }
                let addr = lpc.superio_inw(BASE_ADDRESS_REGISTER)?;
                thread::sleep(Duration::from_millis(1));
                let verify = lpc.superio_inw(BASE_ADDRESS_REGISTER)?;
                if addr == verify && addr >= 0x100 && (addr & 0xF007) == 0 {
                    hwm = Some(addr);
                }
            }
            // Exit config
            let _ = lpc.write_port(reg_port, 0xAA);

            found.push(DetectedChip {
                slot,
                register_port: reg_port,
                chip,
                hwm_address: hwm,
            });
            continue;
        }
        let _ = lpc.write_port(reg_port, 0xAA);

        // --- ITE IT87 enter ---
        lpc.write_port(reg_port, 0x87)?;
        lpc.write_port(reg_port, 0x01)?;
        lpc.write_port(reg_port, 0x55)?;
        lpc.write_port(reg_port, if reg_port == 0x4E { 0xAA } else { 0x55 })?;
        let chip_id = lpc.superio_inw(CHIP_ID_REGISTER)?;
        if chip_id != 0 && chip_id != 0xFFFF {
            let _ = lpc.find_bars();
            let _ = lpc.select_ldn(0x04);
            let addr = lpc.superio_inw(BASE_ADDRESS_REGISTER).ok();
            // Exit (primary port only)
            if reg_port != 0x4E {
                let _ = lpc.write_port(reg_port, 0x02);
                let _ = lpc.write_port(reg_port + 1, 0x02);
            }
            found.push(DetectedChip {
                slot,
                register_port: reg_port,
                chip: SuperIoChip::It87 { chip_id },
                hwm_address: addr.filter(|&a| a >= 0x100 && (a & 0xF007) == 0),
            });
        }
    }

    Ok(found)
}

/// Live banked Nuvoton HWM access for one chip.
pub struct NctBankedDevice {
    lpc: LpcIo,
    register_port: u16,
    hwm: u16,
    slot: u8,
    #[allow(dead_code)]
    chip: SuperIoChip,
    /// Last known duties 0..=100 (software cache; HW may differ until written).
    duties: Mutex<Vec<u8>>,
    fan_count: usize,
    control_count: usize,
}

impl NctBankedDevice {
    pub fn try_open(detected: &DetectedChip) -> Result<Self, String> {
        if !detected.chip.supports_banked_hwm() {
            return Err("chip is not a banked NCT HWM".into());
        }
        let hwm = detected
            .hwm_address
            .ok_or_else(|| "no HWM address for chip".to_string())?;

        let lpc = LpcIo::open()?;
        {
            let _g = IsaBusGuard::acquire(Duration::from_millis(200));
            // Once only — select_slot again would clear BARs.
            lpc.setup_nuvoton_hwm_bars(detected.slot, detected.register_port)?;
            // Smoke-test banked index/data ports
            lpc.read_port(hwm + ADDR_OFF).map_err(|e| {
                format!(
                    "HWM port 0x{:04X} not readable after find_bars: {e}",
                    hwm + ADDR_OFF
                )
            })?;
        }

        // Classic NCT679x: up to 7 fans/controls
        let fan_count = 7;
        let control_count = 7;
        Ok(Self {
            lpc,
            register_port: detected.register_port,
            hwm,
            slot: detected.slot,
            chip: detected.chip,
            duties: Mutex::new(vec![0u8; control_count]),
            fan_count,
            control_count,
        })
    }

    pub fn hwm_address(&self) -> u16 {
        self.hwm
    }

    pub fn fan_count(&self) -> usize {
        self.fan_count
    }

    pub fn control_count(&self) -> usize {
        self.control_count
    }

    fn read_byte(&self, address: u16) -> Result<u8, String> {
        let bank = (address >> 8) as u8;
        let register = (address & 0xFF) as u8;
        self.lpc.write_port(self.hwm + ADDR_OFF, BANK_SELECT)?;
        self.lpc.write_port(self.hwm + DATA_OFF, bank)?;
        self.lpc.write_port(self.hwm + ADDR_OFF, register)?;
        self.lpc.read_port(self.hwm + DATA_OFF)
    }

    fn write_byte(&self, address: u16, value: u8) -> Result<(), String> {
        let bank = (address >> 8) as u8;
        let register = (address & 0xFF) as u8;
        self.lpc.write_port(self.hwm + ADDR_OFF, BANK_SELECT)?;
        self.lpc.write_port(self.hwm + DATA_OFF, bank)?;
        self.lpc.write_port(self.hwm + ADDR_OFF, register)?;
        self.lpc.write_port(self.hwm + DATA_OFF, value)
    }

    /// Read a temperature °C from a primary register (optional half-bit).
    pub fn read_temp_c(
        &self,
        reg: u16,
        half_reg: Option<(u16, u8)>,
    ) -> Result<Option<f64>, String> {
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        let raw = self.read_byte(reg)? as i8;
        let mut value = (raw as i16) << 1;
        if let Some((hr, bit)) = half_reg {
            value |= ((self.read_byte(hr)? >> bit) & 1) as i16;
        }
        let t = 0.5 * f64::from(value);
        if !(-55.0..=125.0).contains(&t) {
            Ok(None)
        } else {
            Ok(Some(t))
        }
    }

    /// 13-bit fan count → RPM (LHM formula).
    pub fn read_fan_rpm(&self, index: usize) -> Result<Option<f64>, String> {
        if index >= self.fan_count {
            return Ok(None);
        }
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        // fan count registers 0x4B0, 0x4B2, ...
        let base = 0x4B0u16 + (index as u16) * 2;
        let high = self.read_byte(base)?;
        let low = self.read_byte(base + 1)?;
        let count = (u16::from(high) << 5) | (u16::from(low) & 0x1F);
        const MAX: u16 = 0x1FFF;
        const MIN: u16 = 0x15;
        if count < MAX {
            if count >= MIN {
                Ok(Some(1.35e6 / f64::from(count)))
            } else {
                Ok(None)
            }
        } else {
            Ok(Some(0.0))
        }
    }

    /// Current PWM output as percent 0..=100.
    pub fn read_duty_percent(&self, index: usize) -> Result<u8, String> {
        if index >= self.control_count {
            return Err("control index out of range".into());
        }
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        // PWM out regs for classic NCT679x
        let regs: [u16; 7] = [0x001, 0x003, 0x011, 0x013, 0x015, 0x017, 0x029];
        let value = self.read_byte(regs[index])?;
        Ok(((f64::from(value) / 2.55).round() as u8).min(100))
    }

    /// Set manual PWM duty 0..=100. **Writes hardware.**
    pub fn set_duty_percent(&self, index: usize, percent: u8) -> Result<(), String> {
        if index >= self.control_count {
            return Err("control index out of range".into());
        }
        let percent = percent.min(100);
        let pwm = ((f64::from(percent) * 2.55).round() as u16).min(255) as u8;
        let mode_regs: [u16; 7] = [0x102, 0x202, 0x302, 0x802, 0x902, 0xA02, 0xB02];
        let cmd_regs: [u16; 7] = [0x109, 0x209, 0x309, 0x809, 0x909, 0xA09, 0xB09];

        let _g = IsaBusGuard::acquire(Duration::from_millis(1000));
        // Do not select_slot (clears BARs).
        self.write_byte(mode_regs[index], 0)?;
        self.write_byte(cmd_regs[index], pwm)?;
        if let Ok(mut d) = self.duties.lock() {
            d[index] = percent;
        }
        let _ = (
            self.register_port,
            self.slot,
            WINBOND_NUVOTON_HWM_LDN,
            NUVOTON_IO_SPACE_LOCK,
        );
        Ok(())
    }

    /// Named temperature sources (common NCT679x).
    pub fn temp_sources(&self) -> Vec<TempSource> {
        vec![
            TempSource {
                name: "PECI_0",
                reg: 0x073,
                half: Some((0x074, 7)),
            },
            TempSource {
                name: "CPUTIN",
                reg: 0x075,
                half: Some((0x076, 7)),
            },
            TempSource {
                name: "SYSTIN",
                reg: 0x077,
                half: Some((0x078, 7)),
            },
            TempSource {
                name: "AUXTIN0",
                reg: 0x079,
                half: Some((0x07A, 7)),
            },
        ]
    }
}

/// One temperature channel description for banked NCT HWM.
#[derive(Debug, Clone, Copy)]
pub struct TempSource {
    pub name: &'static str,
    pub reg: u16,
    pub half: Option<(u16, u8)>,
}
