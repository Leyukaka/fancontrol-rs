//! One loaded PawnIO module session (open + load blob + execute).

use crate::ffi::{self, HANDLE, format_hr};

pub struct PawnSession {
    handle: HANDLE,
}

impl PawnSession {
    /// Open executor and load a module blob.
    pub fn open_with_blob(blob: &[u8]) -> Result<Self, String> {
        let api = ffi::api()?;
        let handle = api
            .open()
            .map_err(|hr| format!("pawnio_open: {}", format_hr(hr)))?;
        if let Err(hr) = api.load_blob(handle, blob) {
            api.close(handle);
            return Err(format!("pawnio_load: {}", format_hr(hr)));
        }
        Ok(Self { handle })
    }

    /// Load embedded module by name (`"LpcIO"`, `"Echo"`).
    pub fn open_embedded(name: &str) -> Result<Self, String> {
        let blob = embedded_module(name)?;
        Self::open_with_blob(blob)
    }

    pub fn execute(&self, name: &str, input: &[u64], out_len: usize) -> Result<Vec<u64>, String> {
        let api = ffi::api()?;
        api.execute(self.handle, name, input, out_len)
            .map_err(|hr| format!("pawnio_execute({name}): {}", format_hr(hr)))
    }
}

impl Drop for PawnSession {
    fn drop(&mut self) {
        if let Ok(api) = ffi::api() {
            api.close(self.handle);
        }
        self.handle = std::ptr::null_mut();
    }
}

// SAFETY: sessions are used under higher-level locking (ISA mutex + single-threaded
// CLI). The HANDLE is not shared across threads without external synchronization.
unsafe impl Send for PawnSession {}
unsafe impl Sync for PawnSession {}

fn embedded_module(name: &str) -> Result<&'static [u8], String> {
    match name {
        "LpcIO" | "lpcio" => Ok(include_bytes!("../modules/LpcIO.bin")),
        "Echo" | "echo" => Ok(include_bytes!("../modules/Echo.bin")),
        "AMDFamily17" | "amdfamily17" => Ok(include_bytes!("../modules/AMDFamily17.bin")),
        "IntelMSR" | "intelmsr" => Ok(include_bytes!("../modules/IntelMSR.bin")),
        "SmbusPIIX4" | "smbuspiix4" => Ok(include_bytes!("../modules/SmbusPIIX4.bin")),
        "SmbusI801" | "smbusi801" => Ok(include_bytes!("../modules/SmbusI801.bin")),
        other => Err(format!("unknown embedded module: {other}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_lpcio_non_empty() {
        let b = embedded_module("LpcIO").unwrap();
        assert!(b.len() > 100);
    }

    #[test]
    fn embedded_msr_modules_non_empty() {
        assert!(embedded_module("AMDFamily17").unwrap().len() > 100);
        assert!(embedded_module("IntelMSR").unwrap().len() > 100);
    }

    #[test]
    fn embedded_smbus_modules_non_empty() {
        assert!(embedded_module("SmbusPIIX4").unwrap().len() > 100);
        assert!(embedded_module("SmbusI801").unwrap().len() > 100);
    }
}
