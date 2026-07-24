//! NCT668x EC-style hardware monitor (NCT6683/6686/6687).
//!
//! Register map adapted from LibreHardwareMonitor `Nct677X` (MPL-2.0).
//! Your board reports id=0xD5 rev=0x92 → NCT6687D family.

use crate::lpcio::LpcIo;
use crate::mutex_isa::IsaBusGuard;
use crate::superio::{DetectedChip, SuperIoChip, TempSource};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

const WINBOND_NUVOTON_HWM_LDN: u8 = 0x0B;
const NUVOTON_IO_SPACE_LOCK: u8 = 0x28;

const EC_PAGE_OFF: u16 = 0x04;
const EC_INDEX_OFF: u16 = 0x05;
const EC_DATA_OFF: u16 = 0x06;
const EC_PAGE_SELECT: u8 = 0xFF;

const INIT_REG: u16 = 0x180;

/// Temperatures (LHM NCT6687D labels are board-dependent; we use generic names).
const TEMP_REGS: &[(&str, u16)] = &[
    ("CPU", 0x100),
    ("System", 0x102),
    ("MOS", 0x104),
    ("PCH", 0x106),
    ("CPU_Socket", 0x108),
    ("PCIE_1", 0x10A),
    ("M2_1", 0x10C),
    ("PCIE_2", 0x10E),
    ("T_Sensor_8", 0x110),
    ("T_Sensor_9", 0x112),
    ("T_Sensor_10", 0x114),
];

/// Fan RPM registers (16-bit big-endian style high then low).
const FAN_RPM_REGS: &[u16] = &[
    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x154, 0x156, 0x158,
    0x15A, 0x15C, 0x15E,
];

/// PWM output / command for first 8 channels (classic NCT6687D path).
const FAN_PWM_OUT: [u16; 8] = [0x160, 0x161, 0x162, 0x163, 0x164, 0x165, 0x166, 0x167];
const FAN_PWM_CMD: [u16; 8] = [0xA28, 0xA29, 0xA2A, 0xA2B, 0xA2C, 0xA2D, 0xA2E, 0xA2F];
/// All share the same mode/request register on classic 6687D.
const FAN_CONTROL_MODE: u16 = 0xA00;
const FAN_PWM_REQUEST: u16 = 0xA01;

pub struct Nct668Device {
    lpc: LpcIo,
    hwm: u16,
    slot: u8,
    register_port: u16,
    #[allow(dead_code)]
    chip: SuperIoChip,
    duties: Mutex<Vec<u8>>,
}

impl Nct668Device {
    pub fn try_open(detected: &DetectedChip) -> Result<Self, String> {
        match detected.chip {
            SuperIoChip::Nct668x { .. } => {}
            _ => return Err("not an NCT668x chip".into()),
        }
        let hwm = detected
            .hwm_address
            .ok_or_else(|| "no HWM address for NCT668x".to_string())?;

        let lpc = LpcIo::open()?;
        {
            let _g = IsaBusGuard::acquire(Duration::from_millis(200));
            // select_slot + find_bars once; do NOT call select_slot again later
            // (it resets the BAR allow-list → ACCESS_DENIED on HWM ports).
            lpc.setup_nuvoton_hwm_bars(detected.slot, detected.register_port)?;
        }

        let dev = Self {
            lpc,
            hwm,
            slot: detected.slot,
            register_port: detected.register_port,
            chip: detected.chip,
            duties: Mutex::new(vec![0u8; FAN_PWM_OUT.len()]),
        };

        // Prove HWM PIO is allowed (EC page register).
        {
            let _g = IsaBusGuard::acquire(Duration::from_millis(100));
            dev.lpc
                .read_port(hwm + EC_PAGE_OFF)
                .map_err(|e| {
                    format!(
                        "HWM port 0x{:04X} not readable after find_bars: {e}. \
                         BAR allow-list may not include this region.",
                        hwm + EC_PAGE_OFF
                    )
                })?;
        }

        // Init EC monitor bit + enable SIO voltage channels (LHM does this).
        {
            let _g = IsaBusGuard::acquire(Duration::from_millis(100));
            if let Ok(data) = dev.read_byte(INIT_REG) {
                if data & 0x80 == 0 {
                    let _ = dev.write_byte(INIT_REG, data | 0x80);
                }
            }
            let _ = dev.write_byte(0x1BB, 0x61);
            let _ = dev.write_byte(0x1BC, 0x62);
            let _ = dev.write_byte(0x1BD, 0x63);
            let _ = dev.write_byte(0x1BE, 0x64);
            let _ = dev.write_byte(0x1BF, 0x65);
        }

        let _ = (dev.register_port, dev.slot, WINBOND_NUVOTON_HWM_LDN, NUVOTON_IO_SPACE_LOCK);
        Ok(dev)
    }

    pub fn hwm_address(&self) -> u16 {
        self.hwm
    }

    pub fn fan_count(&self) -> usize {
        FAN_RPM_REGS.len()
    }

    pub fn control_count(&self) -> usize {
        FAN_PWM_OUT.len()
    }

    pub fn temp_sources(&self) -> Vec<TempSource> {
        TEMP_REGS
            .iter()
            .copied()
            .map(|(name, reg)| TempSource {
                name,
                reg,
                half: None, // half-bit is reg+1 bit7 for EC path
            })
            .collect()
    }

    fn wait_ec_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let access = self.lpc.read_port(self.hwm + EC_PAGE_OFF)?;
            if access == EC_PAGE_SELECT || Instant::now() >= deadline {
                if access != EC_PAGE_SELECT {
                    // Force free access
                    let _ = self.lpc.write_port(self.hwm + EC_PAGE_OFF, EC_PAGE_SELECT);
                }
                return Ok(());
            }
            thread::sleep(Duration::from_millis(1));
        }
    }

    fn read_byte(&self, address: u16) -> Result<u8, String> {
        let page = (address >> 8) as u8;
        let index = (address & 0xFF) as u8;
        self.wait_ec_ready()?;
        self.lpc.write_port(self.hwm + EC_PAGE_OFF, page)?;
        self.lpc.write_port(self.hwm + EC_INDEX_OFF, index)?;
        let result = self.lpc.read_port(self.hwm + EC_DATA_OFF)?;
        let _ = self.lpc.write_port(self.hwm + EC_PAGE_OFF, EC_PAGE_SELECT);
        Ok(result)
    }

    fn write_byte(&self, address: u16, value: u8) -> Result<(), String> {
        let page = (address >> 8) as u8;
        let index = (address & 0xFF) as u8;
        self.wait_ec_ready()?;
        self.lpc.write_port(self.hwm + EC_PAGE_OFF, page)?;
        self.lpc.write_port(self.hwm + EC_INDEX_OFF, index)?;
        self.lpc.write_port(self.hwm + EC_DATA_OFF, value)?;
        let _ = self.lpc.write_port(self.hwm + EC_PAGE_OFF, EC_PAGE_SELECT);
        Ok(())
    }

    pub fn read_temp_c(&self, reg: u16) -> Result<Option<f64>, String> {
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        // Do not call select_slot here — it clears BARs.
        let value = self.read_byte(reg)? as i8;
        let half = (self.read_byte(reg.wrapping_add(1))? >> 7) & 1;
        let t = f64::from(value) + 0.5 * f64::from(half);
        // 0 °C on this family is almost always an unused/unwired channel.
        if !(-55.0..=125.0).contains(&t) || t == 0.0 {
            Ok(None)
        } else {
            Ok(Some(t))
        }
    }

    pub fn read_fan_rpm(&self, index: usize) -> Result<Option<f64>, String> {
        if index >= FAN_RPM_REGS.len() {
            return Ok(None);
        }
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        let reg = FAN_RPM_REGS[index];
        let high = self.read_byte(reg)?;
        let low = self.read_byte(reg + 1)?;
        // NCT6687D: 0xFFF8 often means no fan
        if high == 0xFF && low == 0xF8 {
            return Ok(Some(0.0));
        }
        let rpm = u16::from(high) << 8 | u16::from(low);
        if rpm == 0 || rpm == 0xFFFF {
            Ok(None)
        } else {
            Ok(Some(f64::from(rpm)))
        }
    }

    pub fn read_duty_percent(&self, index: usize) -> Result<u8, String> {
        if index >= FAN_PWM_OUT.len() {
            return Err("control index out of range".into());
        }
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        let value = self.read_byte(FAN_PWM_OUT[index])?;
        Ok(((f64::from(value) / 2.55).round() as u8).min(100))
    }

    /// Manual PWM write (classic NCT6687D request/mode/command sequence).
    pub fn set_duty_percent(&self, index: usize, percent: u8) -> Result<(), String> {
        if index >= FAN_PWM_OUT.len() {
            return Err("control index out of range".into());
        }
        let percent = percent.min(100);
        let pwm = ((f64::from(percent) * 2.55).round() as u16).min(255) as u8;

        let _g = IsaBusGuard::acquire(Duration::from_millis(200))
            .ok_or_else(|| "could not acquire ISA bus mutex".to_string());
        // Do not select_slot (would wipe BARs).

        // Request config
        self.write_byte(FAN_PWM_REQUEST, 0x80)?;
        thread::sleep(Duration::from_millis(50));

        // Set manual mode bit for this channel
        let mode = self.read_byte(FAN_CONTROL_MODE)?;
        let bit_mask = 1u8 << index;
        self.write_byte(FAN_CONTROL_MODE, mode | bit_mask)?;

        // PWM command
        self.write_byte(FAN_PWM_CMD[index], pwm)?;

        // Done
        self.write_byte(FAN_PWM_REQUEST, 0x40)?;
        thread::sleep(Duration::from_millis(50));

        if let Ok(mut d) = self.duties.lock() {
            d[index] = percent;
        }
        Ok(())
    }
}
