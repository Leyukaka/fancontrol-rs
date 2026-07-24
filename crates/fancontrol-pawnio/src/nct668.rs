//! NCT668x EC-style hardware monitor (NCT6683/6686/6687 / 6687DR-style).
//!
//! Register map adapted from LibreHardwareMonitor `Nct677X` (MPL-2.0).
//! Owner board: id=0xD5 rev=0x92 → NCT6687D / 6687DR family.

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

/// Classic sequential RPM banks (works for many 6687D boards).
const FAN_RPM_CLASSIC: &[u16] = &[
    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x154, 0x156, 0x158,
    0x15A, 0x15C, 0x15E, 0x852, // some boards: SYS Fan 7 / extra tach
];

/// LHM NCT6687DR-style reordering (MSI AM5 / similar).
const FAN_RPM_DR: &[u16] = &[
    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x15E, 0x15C, 0x15A,
    0x158, 0x156, 0x154,
];

/// Primary PWM outs (always present on 6687).
const PWM_PRIMARY: &[u16] = &[0x160, 0x161, 0x162, 0x163, 0x164, 0x165, 0x166, 0x167];

/// Extended PWM outs used by NCT6687DR for system fans (LHM).
/// Index pairs with FAN_RPM_DR for channels 9–15.
const PWM_EXTENDED: &[(usize, u16)] = &[
    (9, 0xC93),  // SYSFAN7-style
    (10, 0xE05), // SYSFAN1
    (11, 0xE04),
    (12, 0xE03),
    (13, 0xE02),
    (14, 0xE01),
    (15, 0xE00),
];

const FAN_PWM_CMD_PRIMARY: [u16; 8] = [0xA28, 0xA29, 0xA2A, 0xA2B, 0xA2C, 0xA2D, 0xA2E, 0xA2F];
const FAN_CONTROL_MODE: u16 = 0xA00;
const FAN_PWM_REQUEST: u16 = 0xA01;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Classic,
    Dr,
}

pub struct Nct668Device {
    lpc: LpcIo,
    hwm: u16,
    slot: u8,
    register_port: u16,
    #[allow(dead_code)]
    chip: SuperIoChip,
    layout: Layout,
    /// Parallel arrays: control index → (pwm_out_reg, optional rpm_reg_index in fan list)
    control_pwm: Vec<u16>,
    control_rpm_idx: Vec<Option<usize>>,
    fan_rpm_regs: Vec<u16>,
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
            lpc.setup_nuvoton_hwm_bars(detected.slot, detected.register_port)?;
        }

        let mut dev = Self {
            lpc,
            hwm,
            slot: detected.slot,
            register_port: detected.register_port,
            chip: detected.chip,
            layout: Layout::Classic,
            control_pwm: PWM_PRIMARY.to_vec(),
            control_rpm_idx: (0..PWM_PRIMARY.len()).map(Some).collect(),
            fan_rpm_regs: FAN_RPM_CLASSIC.to_vec(),
            duties: Mutex::new(vec![0u8; PWM_PRIMARY.len()]),
        };

        {
            let _g = IsaBusGuard::acquire(Duration::from_millis(100));
            dev.lpc.read_port(hwm + EC_PAGE_OFF).map_err(|e| {
                format!(
                    "HWM port 0x{:04X} not readable after find_bars: {e}.",
                    hwm + EC_PAGE_OFF
                )
            })?;
        }

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

        // Owner chip 0xD5/0x92 and any board with live extended PWM → DR-style map
        // (exposes system-fan PWM beyond the first 8 headers).
        let force_dr = matches!(detected.chip, SuperIoChip::Nct668x { id: 0xD5, revision: 0x92 });
        if force_dr || dev.probe_extended_pwm() {
            dev.apply_dr_layout();
            tracing::info!(force_dr, "NCT668x: using DR-style extended PWM/fan map");
        } else {
            tracing::info!("NCT668x: using classic 8-channel PWM + sequential fans");
        }

        let n = dev.control_pwm.len();
        *dev.duties.lock().expect("duties") = vec![0u8; n];

        let _ = (dev.register_port, dev.slot, WINBOND_NUVOTON_HWM_LDN, NUVOTON_IO_SPACE_LOCK);
        Ok(dev)
    }

    fn probe_extended_pwm(&self) -> bool {
        let _g = IsaBusGuard::acquire(Duration::from_millis(80));
        // If any extended PWM register is not stuck 0xFF/0x00-only-noise, enable DR map.
        let mut hits = 0;
        for &(_, reg) in PWM_EXTENDED {
            if let Ok(v) = self.read_byte(reg) {
                // Any mid-range duty suggests a real channel
                if (5..=250).contains(&v) {
                    hits += 1;
                }
            }
        }
        hits >= 1
    }

    fn apply_dr_layout(&mut self) {
        self.layout = Layout::Dr;
        self.fan_rpm_regs = FAN_RPM_DR.to_vec();
        // 16 controls: 0-7 primary, 8 unused, 9-15 extended
        let mut pwm = vec![0xFFFFu16; 16];
        for (i, &r) in PWM_PRIMARY.iter().enumerate() {
            pwm[i] = r;
        }
        for &(idx, reg) in PWM_EXTENDED {
            if idx < pwm.len() {
                pwm[idx] = reg;
            }
        }
        self.control_pwm = pwm;
        self.control_rpm_idx = (0..16).map(Some).collect();
        // Channel 8 invalid in LHM DR table
        self.control_pwm[8] = 0xFFFF;
        self.control_rpm_idx[8] = None;
    }

    pub fn hwm_address(&self) -> u16 {
        self.hwm
    }

    pub fn fan_count(&self) -> usize {
        self.fan_rpm_regs.len()
    }

    pub fn layout_label(&self) -> &'static str {
        match self.layout {
            Layout::Classic => "nct668-classic",
            Layout::Dr => "nct668-dr",
        }
    }

    /// Enumerate available control slots (skips 0xFFFF holes).
    pub fn control_slots(&self) -> Vec<(usize, u16)> {
        self.control_pwm
            .iter()
            .enumerate()
            .filter(|(_, &r)| r != 0xFFFF)
            .map(|(i, &r)| (i, r))
            .collect()
    }

    pub fn rpm_index_for_control(&self, control_slot: usize) -> Option<usize> {
        self.control_rpm_idx.get(control_slot).copied().flatten()
    }

    pub fn temp_sources(&self) -> Vec<TempSource> {
        TEMP_REGS
            .iter()
            .copied()
            .map(|(name, reg)| TempSource {
                name,
                reg,
                half: None,
            })
            .collect()
    }

    fn wait_ec_ready(&self) -> Result<(), String> {
        let deadline = Instant::now() + Duration::from_millis(500);
        loop {
            let access = self.lpc.read_port(self.hwm + EC_PAGE_OFF)?;
            if access == EC_PAGE_SELECT || Instant::now() >= deadline {
                if access != EC_PAGE_SELECT {
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
        let value = self.read_byte(reg)? as i8;
        let half = (self.read_byte(reg.wrapping_add(1))? >> 7) & 1;
        let t = f64::from(value) + 0.5 * f64::from(half);
        if !(-55.0..=125.0).contains(&t) || t == 0.0 {
            Ok(None)
        } else {
            Ok(Some(t))
        }
    }

    pub fn read_fan_rpm(&self, index: usize) -> Result<Option<f64>, String> {
        if index >= self.fan_rpm_regs.len() {
            return Ok(None);
        }
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        let reg = self.fan_rpm_regs[index];
        let high = self.read_byte(reg)?;
        let low = self.read_byte(reg + 1)?;

        // No fan / open header
        if high == 0xFF && (low == 0xF8 || low == 0xFF) {
            return Ok(None);
        }
        if high == 0xFF && low == 0x00 {
            return Ok(None);
        }

        // 13-bit style used on some extra tachs (high bits empty-ish)
        if reg == 0x852 {
            let count = (u16::from(high) << 5) | (u16::from(low) & 0x1F);
            if count >= 0x1FFF {
                return Ok(Some(0.0));
            }
            if count < 0x15 {
                return Ok(None);
            }
            return Ok(Some(1.35e6 / f64::from(count)));
        }

        let rpm = (u16::from(high) << 8) | u16::from(low);
        if rpm == 0xFFFF {
            return Ok(None);
        }
        // 0 RPM = fan present but stopped — still report it
        Ok(Some(f64::from(rpm)))
    }

    pub fn read_duty_percent(&self, control_slot: usize) -> Result<u8, String> {
        let reg = *self
            .control_pwm
            .get(control_slot)
            .ok_or_else(|| "control index out of range".to_string())?;
        if reg == 0xFFFF {
            return Err("control not available on this layout".into());
        }
        let _g = IsaBusGuard::acquire(Duration::from_millis(50));
        let value = self.read_byte(reg)?;
        Ok(((f64::from(value) / 2.55).round() as u8).min(100))
    }

    pub fn set_duty_percent(&self, control_slot: usize, percent: u8) -> Result<(), String> {
        let reg = *self
            .control_pwm
            .get(control_slot)
            .ok_or_else(|| "control index out of range".to_string())?;
        if reg == 0xFFFF {
            return Err("control not available on this layout".into());
        }
        let percent = percent.min(100);
        let pwm = ((f64::from(percent) * 2.55).round() as u16).min(255) as u8;

        let _g = IsaBusGuard::acquire(Duration::from_millis(200))
            .ok_or_else(|| "could not acquire ISA bus mutex".to_string())?;

        if control_slot < 8 {
            // Classic request / mode / command path
            self.write_byte(FAN_PWM_REQUEST, 0x80)?;
            thread::sleep(Duration::from_millis(50));
            let mode = self.read_byte(FAN_CONTROL_MODE)?;
            let bit_mask = 1u8 << control_slot;
            self.write_byte(FAN_CONTROL_MODE, mode | bit_mask)?;
            self.write_byte(FAN_PWM_CMD_PRIMARY[control_slot], pwm)?;
            self.write_byte(FAN_PWM_REQUEST, 0x40)?;
            thread::sleep(Duration::from_millis(50));
        } else {
            // Extended channels: write PWM out register directly (manual mode board-dependent).
            // Still use request strobe when available.
            let _ = self.write_byte(FAN_PWM_REQUEST, 0x80);
            thread::sleep(Duration::from_millis(30));
            self.write_byte(reg, pwm)?;
            let _ = self.write_byte(FAN_PWM_REQUEST, 0x40);
            thread::sleep(Duration::from_millis(30));
        }

        if let Ok(mut d) = self.duties.lock() {
            if control_slot < d.len() {
                d[control_slot] = percent;
            }
        }
        let _ = self.layout;
        Ok(())
    }
}
