//! One loaded PawnIO module session (open + load blob + execute).

use crate::ffi::{self, format_hr, HANDLE};

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
}
