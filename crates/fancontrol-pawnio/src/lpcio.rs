//! LpcIO module wrapper (port I/O + Super I/O config space).

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
    pub fn select_slot(&self, slot: u32) -> Result<(), String> {
        self.session
            .execute("ioctl_select_slot", &[slot as u64], 0)?;
        Ok(())
    }

    pub fn find_bars(&self) -> Result<(), String> {
        self.session.execute("ioctl_find_bars", &[], 0)?;
        Ok(())
    }

    pub fn read_port(&self, port: u16) -> Result<u8, String> {
        let out = self
            .session
            .execute("ioctl_pio_inb", &[port as u64], 1)?;
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
