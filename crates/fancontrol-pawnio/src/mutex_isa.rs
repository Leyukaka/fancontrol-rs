//! ISA bus mutex shared with other hardware tools (LHM, FanControl, …).

use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0, WAIT_TIMEOUT};
use windows_sys::Win32::System::Threading::{OpenMutexW, ReleaseMutex, WaitForSingleObject};

const ISA_MUTEX_NAME: &str = "Global\\Access_ISABUS.HTP.Method";
// SYNCHRONIZE | MUTEX_MODIFY_STATE
const MUTEX_RIGHTS: u32 = 0x0010_0000 | 0x0001;

pub struct IsaBusGuard {
    handle: HANDLE,
}

impl IsaBusGuard {
    /// Try to acquire the global ISA bus mutex within `timeout`.
    pub fn acquire(timeout: Duration) -> Option<Self> {
        let wide: Vec<u16> = ISA_MUTEX_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: valid null-terminated UTF-16 name.
        let handle = unsafe { OpenMutexW(MUTEX_RIGHTS, 0, wide.as_ptr()) };
        if handle.is_null() {
            tracing::debug!("ISA bus mutex not available (OpenMutex failed)");
            return None;
        }

        let deadline = Instant::now() + timeout;
        let remaining = deadline.saturating_duration_since(Instant::now());
        let ms = remaining.as_millis().min(u32::MAX as u128) as u32;
        // SAFETY: handle from OpenMutexW
        let r = unsafe { WaitForSingleObject(handle, ms) };
        if r == WAIT_OBJECT_0 {
            return Some(Self { handle });
        }
        if r == WAIT_TIMEOUT {
            tracing::debug!("ISA bus mutex wait timed out");
        }
        unsafe {
            let _ = CloseHandle(handle);
        }
        None
    }
}

impl Drop for IsaBusGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = ReleaseMutex(self.handle);
            let _ = CloseHandle(self.handle);
        }
    }
}
