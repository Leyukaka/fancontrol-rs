//! NCT668x EC-style hardware monitor (NCT6683/6686/6687 / 6687DR-style).
//!
//! Register map / fan-config protocol adapted from LibreHardwareMonitor `Nct677X` (MPL-2.0).
//! Owner board: id=0xD5 rev=0x92 → NCT6687DR-class EC.
//!
//! **Note:** System-fan headers (ctrl9–15) are often driven by the motherboard
//! SmartFan engine. Without the DR fan-config phase (request → manual bit →
//! command → done), writes look like they "snap back" to a BIOS curve (e.g. 60%).

use crate::lpcio::LpcIo;
use crate::mutex_isa::IsaBusGuard;
use crate::superio::{DetectedChip, SuperIoChip, TempSource};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

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

const FAN_RPM_CLASSIC: &[u16] = &[
    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x154, 0x156, 0x158,
    0x15A, 0x15C, 0x15E, 0x852,
];

/// LHM NCT6687DR tach order.
const FAN_RPM_DR: &[u16] = &[
    0x140, 0x142, 0x144, 0x146, 0x148, 0x14A, 0x14C, 0x14E, 0x150, 0x152, 0x15E, 0x15C, 0x15A,
    0x158, 0x156, 0x154,
];

/// Per-channel DR control descriptors (LHM).
/// `out` = duty cycle sensor, `cmd` = software command, `mode` + `mode_bit` = manual enable.
#[derive(Clone, Copy)]
struct DrChannel {
    out: u16,
    cmd: u16,
    mode: u16,
    mode_bit: i8, // -1 = invalid
    rpm_idx: Option<usize>,
}

/// Indices 0–15; slot 8 unused (0xFFF).
const DR_CHANNELS: [DrChannel; 16] = [
    // 0 CPU
    DrChannel {
        out: 0x160,
        cmd: 0xA28,
        mode: 0xA00,
        mode_bit: 0,
        rpm_idx: Some(0),
    },
    // 1 Pump
    DrChannel {
        out: 0x161,
        cmd: 0xA29,
        mode: 0xA00,
        mode_bit: 1,
        rpm_idx: Some(1),
    },
    // 2 Chipset
    DrChannel {
        out: 0x162,
        cmd: 0xA2A,
        mode: 0xA00,
        mode_bit: 2,
        rpm_idx: Some(2),
    },
    // 3 EZ-Connect
    DrChannel {
        out: 0x163,
        cmd: 0xA2B,
        mode: 0xA00,
        mode_bit: 3,
        rpm_idx: Some(3),
    },
    DrChannel {
        out: 0x164,
        cmd: 0xFFFF,
        mode: 0xA00,
        mode_bit: -1,
        rpm_idx: Some(4),
    },
    DrChannel {
        out: 0x165,
        cmd: 0xFFFF,
        mode: 0xA00,
        mode_bit: -1,
        rpm_idx: Some(5),
    },
    DrChannel {
        out: 0x166,
        cmd: 0xFFFF,
        mode: 0xA00,
        mode_bit: -1,
        rpm_idx: Some(6),
    },
    DrChannel {
        out: 0x167,
        cmd: 0xFFFF,
        mode: 0xA00,
        mode_bit: -1,
        rpm_idx: Some(7),
    },
    // 8 unused
    DrChannel {
        out: 0xFFFF,
        cmd: 0xFFFF,
        mode: 0xFFFF,
        mode_bit: -1,
        rpm_idx: None,
    },
    // 9 SYSFAN7
    DrChannel {
        out: 0xC93,
        cmd: 0x8E9,
        mode: 0x80F,
        mode_bit: 1,
        rpm_idx: Some(9),
    },
    // 10–15 SYSFAN1–6 (LHM order)
    DrChannel {
        out: 0xE05,
        cmd: 0x265,
        mode: 0x80F,
        mode_bit: 7,
        rpm_idx: Some(10),
    },
    DrChannel {
        out: 0xE04,
        cmd: 0x264,
        mode: 0x80F,
        mode_bit: 6,
        rpm_idx: Some(11),
    },
    DrChannel {
        out: 0xE03,
        cmd: 0x263,
        mode: 0x80F,
        mode_bit: 5,
        rpm_idx: Some(12),
    },
    DrChannel {
        out: 0xE02,
        cmd: 0x262,
        mode: 0x80F,
        mode_bit: 4,
        rpm_idx: Some(13),
    },
    DrChannel {
        out: 0xE01,
        cmd: 0x261,
        mode: 0x80F,
        mode_bit: 3,
        rpm_idx: Some(14),
    },
    DrChannel {
        out: 0xE00,
        cmd: 0x260,
        mode: 0x80F,
        mode_bit: 2,
        rpm_idx: Some(15),
    },
];

// Fan engine status (Linux nct6687d / LHM)
const REG_FAN_ENGINE_STS: u16 = 0xCF8;
const FAN_CFG_LOCK: u8 = 1 << 6;
const FAN_CFG_PHASE: u8 = 1 << 3;
const FAN_CFG_INVALID: u8 = 1 << 4;
const FAN_CFG_CHECK_DONE: u8 = 1 << 5;
const FAN_CFG_REQ: u8 = 0x80;
const FAN_CFG_DONE: u8 = 0x40;
const FAN_PWM_REQUEST: u16 = 0xA01;
const FAN_CONTROL_MODE_PRIMARY: u16 = 0xA00;

const FAN_PWM_CMD_PRIMARY: [u16; 8] = [0xA28, 0xA29, 0xA2A, 0xA2B, 0xA2C, 0xA2D, 0xA2E, 0xA2F];
const PWM_PRIMARY: [u16; 8] = [0x160, 0x161, 0x162, 0x163, 0x164, 0x165, 0x166, 0x167];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Layout {
    Classic,
    Dr,
}

pub struct Nct668Device {
    lpc: LpcIo,
    hwm: u16,
    #[allow(dead_code)]
    slot: u8,
    #[allow(dead_code)]
    register_port: u16,
    #[allow(dead_code)]
    chip: SuperIoChip,
    layout: Layout,
    /// Hardware control slot → out register (0xFFFF = hole)
    control_out: Vec<u16>,
    control_cmd: Vec<u16>,
    control_mode: Vec<u16>,
    control_mode_bit: Vec<i8>,
    control_rpm_idx: Vec<Option<usize>>,
    fan_rpm_regs: Vec<u16>,
    /// Last software-commanded duty (UI can prefer this while locked)
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
            control_out: PWM_PRIMARY.to_vec(),
            control_cmd: FAN_PWM_CMD_PRIMARY.to_vec(),
            control_mode: vec![FAN_CONTROL_MODE_PRIMARY; 8],
            control_mode_bit: (0..8).map(|i| i as i8).collect(),
            control_rpm_idx: (0..8).map(Some).collect(),
            fan_rpm_regs: FAN_RPM_CLASSIC.to_vec(),
            duties: Mutex::new(vec![0u8; 8]),
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
            for (a, v) in [
                (0x1BB, 0x61),
                (0x1BC, 0x62),
                (0x1BD, 0x63),
                (0x1BE, 0x64),
                (0x1BF, 0x65),
            ] {
                let _ = dev.write_byte(a, v);
            }
        }

        // 0xD5/0x92 and boards with live extended outs → DR protocol
        let force_dr = matches!(
            detected.chip,
            SuperIoChip::Nct668x {
                id: 0xD5,
                revision: 0x92
            }
        );
        if force_dr || dev.probe_extended_outs() {
            dev.apply_dr_layout();
            tracing::info!("NCT668x: DR layout + fan-config write protocol");
        } else {
            tracing::info!("NCT668x: classic 8-channel layout");
        }

        let n = dev.control_out.len();
        *dev.duties.lock().expect("duties") = vec![0u8; n];
        Ok(dev)
    }

    fn probe_extended_outs(&self) -> bool {
        let _g = IsaBusGuard::acquire(Duration::from_millis(80));
        for ch in DR_CHANNELS.iter().skip(9) {
            if ch.out == 0xFFFF {
                continue;
            }
            if let Ok(v) = self.read_byte(ch.out) {
                if (5..=250).contains(&v) {
                    return true;
                }
            }
        }
        false
    }

    fn apply_dr_layout(&mut self) {
        self.layout = Layout::Dr;
        self.fan_rpm_regs = FAN_RPM_DR.to_vec();
        self.control_out = DR_CHANNELS.iter().map(|c| c.out).collect();
        self.control_cmd = DR_CHANNELS.iter().map(|c| c.cmd).collect();
        self.control_mode = DR_CHANNELS.iter().map(|c| c.mode).collect();
        self.control_mode_bit = DR_CHANNELS.iter().map(|c| c.mode_bit).collect();
        self.control_rpm_idx = DR_CHANNELS.iter().map(|c| c.rpm_idx).collect();
    }

    pub fn hwm_address(&self) -> u16 {
        self.hwm
    }

    pub fn layout_label(&self) -> &'static str {
        match self.layout {
            Layout::Classic => "nct668-classic",
            Layout::Dr => "nct668-dr",
        }
    }

    pub fn fan_count(&self) -> usize {
        self.fan_rpm_regs.len()
    }

    pub fn control_slots(&self) -> Vec<(usize, u16)> {
        self.control_out
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

    fn update_byte(&self, address: u16, and_mask: u8, or_mask: u8) -> Result<(), String> {
        let v = self.read_byte(address)?;
        self.write_byte(address, (v & and_mask) | or_mask)
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

        if high == 0xFF && (low == 0xF8 || low == 0xFF || low == 0x00) {
            return Ok(None);
        }

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
        Ok(Some(f64::from(rpm)))
    }

    pub fn read_duty_percent(&self, control_slot: usize) -> Result<u8, String> {
        let reg = *self
            .control_out
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
        let out = *self
            .control_out
            .get(control_slot)
            .ok_or_else(|| "control index out of range".to_string())?;
        if out == 0xFFFF {
            return Err("control not available on this layout".into());
        }
        let percent = percent.min(100);
        let pwm = ((f64::from(percent) * 2.55).round() as u16).min(255) as u8;

        let _g = IsaBusGuard::acquire(Duration::from_millis(1500));

        match self.layout {
            Layout::Classic => self.set_duty_classic(control_slot, pwm)?,
            Layout::Dr => self.set_duty_dr(control_slot, pwm)?,
        }

        if let Ok(mut d) = self.duties.lock() {
            if control_slot < d.len() {
                d[control_slot] = percent;
            }
        }

        // Verify EC accepted (read OUT). BIOS may still reclaim later.
        thread::sleep(Duration::from_millis(80));
        if let Ok(actual) = self.read_byte(out) {
            let actual_pct = ((f64::from(actual) / 2.55).round() as i16 - percent as i16).abs();
            if actual_pct > 8 {
                tracing::warn!(
                    control_slot,
                    percent,
                    actual,
                    "duty readback differs — EC/BIOS may still drive this header (SmartFan)"
                );
            }
        }
        Ok(())
    }

    fn set_duty_classic(&self, control_slot: usize, pwm: u8) -> Result<(), String> {
        if control_slot >= 8 {
            return Err("classic layout only has controls 0..7".into());
        }
        self.write_byte(FAN_PWM_REQUEST, FAN_CFG_REQ)?;
        thread::sleep(Duration::from_millis(50));
        let mode = self.read_byte(FAN_CONTROL_MODE_PRIMARY)?;
        let bit_mask = 1u8 << control_slot;
        self.write_byte(FAN_CONTROL_MODE_PRIMARY, mode | bit_mask)?;
        self.write_byte(FAN_PWM_CMD_PRIMARY[control_slot], pwm)?;
        self.write_byte(FAN_PWM_REQUEST, FAN_CFG_DONE)?;
        thread::sleep(Duration::from_millis(50));
        Ok(())
    }

    /// LHM / Linux-style fan configuration phase for NCT6687DR.
    fn set_duty_dr(&self, control_slot: usize, pwm: u8) -> Result<(), String> {
        let cmd = *self
            .control_cmd
            .get(control_slot)
            .ok_or_else(|| "no cmd reg".to_string())?;
        let mode = *self
            .control_mode
            .get(control_slot)
            .ok_or_else(|| "no mode reg".to_string())?;
        let bit = self
            .control_mode_bit
            .get(control_slot)
            .copied()
            .unwrap_or(-1);
        if cmd == 0xFFFF || mode == 0xFFFF || bit < 0 {
            return Err(format!(
                "control {control_slot} is not software-mappable on this board (BIOS-only / unused)"
            ));
        }
        let bit_mask = 1u8 << (bit as u8);

        // Retry up to 3 times if EC rejects config (INVALID bit)
        let mut confirmed = false;
        for attempt in 0..3 {
            if !self.start_fan_cfg_update()? {
                tracing::debug!(attempt, "fan cfg phase start timeout");
            }
            // Set manual-mode bit, write PWM command (not the OUT sensor reg)
            self.update_byte(mode, !bit_mask, bit_mask)?;
            self.write_byte(cmd, pwm)?;
            if self.complete_fan_cfg_update()? {
                confirmed = true;
                break;
            }
            tracing::debug!(attempt, "fan cfg rejected/invalid, retry");
            thread::sleep(Duration::from_millis(20));
        }

        if !confirmed {
            // Last attempt: leave command programmed anyway
            let _ = self.write_byte(cmd, pwm);
            tracing::warn!(
                control_slot,
                "EC did not confirm fan-config for ctrl{control_slot}; \
                 BIOS SmartFan may reclaim duty (often ~60%). \
                 Prefer ctrl0–ctrl3 if possible, or set this header to 'manual/full speed' in BIOS."
            );
        }
        Ok(())
    }

    fn start_fan_cfg_update(&self) -> Result<bool, String> {
        let engine = self.read_byte(REG_FAN_ENGINE_STS)?;
        if engine & FAN_CFG_LOCK == 0 && engine & FAN_CFG_PHASE != 0 {
            return Ok(true);
        }

        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            let engine = self.read_byte(REG_FAN_ENGINE_STS)?;
            if engine & FAN_CFG_PHASE == 0 {
                let req = self.read_byte(FAN_PWM_REQUEST)?;
                if req & FAN_CFG_REQ == 0 {
                    break;
                }
            }
            thread::sleep(Duration::from_millis(1));
        }

        // RMW set CFG_REQ
        self.update_byte(FAN_PWM_REQUEST, !FAN_CFG_REQ, FAN_CFG_REQ)?;
        thread::sleep(Duration::from_millis(10));

        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            let engine = self.read_byte(REG_FAN_ENGINE_STS)?;
            if engine & FAN_CFG_LOCK == 0 && engine & FAN_CFG_PHASE != 0 {
                return Ok(true);
            }
            thread::sleep(Duration::from_millis(1));
        }
        Ok(false)
    }

    fn complete_fan_cfg_update(&self) -> Result<bool, String> {
        let engine = self.read_byte(REG_FAN_ENGINE_STS)?;
        if engine & FAN_CFG_LOCK != 0 || engine & FAN_CFG_PHASE == 0 {
            return Ok(false);
        }
        // RMW set CFG_DONE
        self.update_byte(FAN_PWM_REQUEST, !FAN_CFG_DONE, FAN_CFG_DONE)?;
        thread::sleep(Duration::from_millis(10));

        let deadline = Instant::now() + Duration::from_millis(800);
        while Instant::now() < deadline {
            let engine = self.read_byte(REG_FAN_ENGINE_STS)?;
            if engine & FAN_CFG_CHECK_DONE != 0 {
                break;
            }
            thread::sleep(Duration::from_millis(1));
        }
        let engine = self.read_byte(REG_FAN_ENGINE_STS)?;
        Ok(engine & FAN_CFG_INVALID == 0)
    }
}
