//! LpcIO module wrapper (port I/O + Super I/O config space).
//!
//! **Important:** `ioctl_select_slot` resets the module state (including allowed
//! BARs). After `find_bars`, do **not** call `select_slot` again or HWM port
//! I/O will return ACCESS_DENIED until bars are rediscovered.

use crate::session::PawnSession;

pub struct LpcIo {
    session: PawnSession,
}

impl LpcIo {
    pub fn open() -> Result<Self, String> {
        Ok(Self {
            session: PawnSession::open_embedded("LpcIO")?,
        })
    }

    /// Slot 0 = ports 0x2E/0x2F, slot 1 = 0x4E/0x4F.
    ///
    /// Resets BAR allow-list — must call `find_bars` again before HWM PIO.
    pub fn select_slot(&self, slot: u32) -> Result<(), String> {
        self.session
            .execute("ioctl_select_slot", &[slot as u64], 0)?;
        Ok(())
    }

    pub fn find_bars(&self) -> Result<(), String> {
        self.session.execute("ioctl_find_bars", &[], 0)?;
        Ok(())
    }

    /// Enter Winbond/Nuvoton config, discover BARs (must include HWM), exit config.
    /// Leaves the session ready for PIO to the hardware-monitor region.
    pub fn setup_nuvoton_hwm_bars(&self, slot: u8, register_port: u16) -> Result<(), String> {
        self.select_slot(slot as u32)?;
        // Enter config mode
        self.write_port(register_port, 0x87)?;
        self.write_port(register_port, 0x87)?;
        self.find_bars()
            .map_err(|e| format!("find_bars failed (HWM ports will be denied): {e}"))?;
        // Select HWM logical device + clear IO lock (best effort)
        let _ = self.select_ldn(0x0B);
        if let Ok(options) = self.superio_inb(0x28) {
            if options & 0x10 != 0 {
                let _ = self.superio_outb(0x28, options & !0x10);
            }
        }
        // Exit config — BARs stay in module memory
        let _ = self.write_port(register_port, 0xAA);
        Ok(())
    }

    pub fn read_port(&self, port: u16) -> Result<u8, String> {
        let out = self.session.execute("ioctl_pio_inb", &[port as u64], 1)?;
        Ok(out.first().copied().unwrap_or(0) as u8)
    }

    pub fn write_port(&self, port: u16, value: u8) -> Result<(), String> {
        self.session
            .execute("ioctl_pio_outb", &[port as u64, value as u64], 0)?;
        Ok(())
    }

    pub fn superio_inb(&self, reg: u8) -> Result<u8, String> {
        let out = self
            .session
            .execute("ioctl_superio_inb", &[reg as u64], 1)?;
        Ok(out.first().copied().unwrap_or(0) as u8)
    }

    pub fn superio_inw(&self, reg: u8) -> Result<u16, String> {
        let out = self
            .session
            .execute("ioctl_superio_inw", &[reg as u64], 1)?;
        Ok(out.first().copied().unwrap_or(0) as u16)
    }

    pub fn superio_outb(&self, reg: u8, value: u8) -> Result<(), String> {
        self.session
            .execute("ioctl_superio_outb", &[reg as u64, value as u64], 0)?;
        Ok(())
    }

    pub fn select_ldn(&self, ldn: u8) -> Result<(), String> {
        self.superio_outb(0x07, ldn)
    }
}
