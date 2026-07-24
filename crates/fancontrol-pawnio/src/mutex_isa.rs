//! Serialize Super I/O / HWM access within this process, and optionally
//! coordinate with other tools via the global ISA bus mutex.
//!
//! UI poller + slider writes used to race / fail on ISA `OpenMutex` timeouts.
//! We always take a **process-local** mutex (cannot fail open), and treat the
//! global ISA mutex as best-effort only.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, FALSE, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

const ISA_MUTEX_NAME: &str = "Global\\Access_ISABUS.HTP.Method";

fn process_sio_mutex() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

/// Guard held for the duration of one HWM transaction.
pub struct IsaBusGuard {
    _process: MutexGuard<'static, ()>,
    isa_handle: Option<HANDLE>,
}

impl IsaBusGuard {
    /// Acquire exclusive HWM access (blocking on process lock).
    /// `isa_timeout` only applies to the optional global ISA mutex.
    pub fn acquire(isa_timeout: Duration) -> Self {
        let process = process_sio_mutex()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let isa_handle = try_acquire_isa(isa_timeout);
        Self {
            _process: process,
            isa_handle,
        }
    }
}

impl Drop for IsaBusGuard {
    fn drop(&mut self) {
        if let Some(h) = self.isa_handle.take() {
            unsafe {
                let _ = ReleaseMutex(h);
                let _ = CloseHandle(h);
            }
        }
    }
}

fn try_acquire_isa(timeout: Duration) -> Option<HANDLE> {
    let wide: Vec<u16> = ISA_MUTEX_NAME
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    // CreateMutex opens existing or creates — more reliable than OpenMutex alone.
    // SAFETY: null security attributes, valid name.
    let handle = unsafe { CreateMutexW(std::ptr::null(), FALSE, wide.as_ptr()) };
    if handle.is_null() {
        let err = unsafe { GetLastError() };
        tracing::debug!(err, "CreateMutex ISA bus failed");
        return None;
    }
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let ms = ms.max(1);
    // SAFETY: handle from CreateMutexW
    let r = unsafe { WaitForSingleObject(handle, ms) };
    if r == WAIT_OBJECT_0 {
        Some(handle)
    } else {
        tracing::trace!("ISA bus mutex wait timed out (process lock still held)");
        unsafe {
            let _ = CloseHandle(handle);
        }
        None
    }
}
