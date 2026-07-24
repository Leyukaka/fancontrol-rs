//! Serialize Super I/O / HWM access within this process, and optionally
//! coordinate with other tools via the global ISA bus mutex.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{GetLastError, FALSE, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

const ISA_MUTEX_NAME: &str = "Global\\Access_ISABUS.HTP.Method";
const WAIT_ABANDONED: u32 = 0x0000_0080;

fn process_sio_mutex() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

/// Newtype so a process-lifetime HANDLE can live in a static.
struct IsaHandle(HANDLE);
// SAFETY: Windows mutex handles are usable from any thread in the process.
unsafe impl Send for IsaHandle {}
unsafe impl Sync for IsaHandle {}

fn isa_mutex_handle() -> Option<HANDLE> {
    static H: OnceLock<Option<IsaHandle>> = OnceLock::new();
    H.get_or_init(|| {
        let wide: Vec<u16> = ISA_MUTEX_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: null security attributes, valid name.
        let handle = unsafe { CreateMutexW(std::ptr::null(), FALSE, wide.as_ptr()) };
        if handle.is_null() {
            let err = unsafe { GetLastError() };
            tracing::debug!(err, "CreateMutex ISA bus failed");
            None
        } else {
            Some(IsaHandle(handle))
        }
    })
    .as_ref()
    .map(|h| h.0)
}

pub struct IsaBusGuard {
    _process: MutexGuard<'static, ()>,
    held_isa: bool,
}

impl IsaBusGuard {
    pub fn acquire(isa_timeout: Duration) -> Self {
        let process = process_sio_mutex()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let held_isa = try_wait_isa(isa_timeout);
        Self {
            _process: process,
            held_isa,
        }
    }
}

impl Drop for IsaBusGuard {
    fn drop(&mut self) {
        if self.held_isa {
            if let Some(h) = isa_mutex_handle() {
                unsafe {
                    let _ = ReleaseMutex(h);
                }
            }
        }
    }
}

fn try_wait_isa(timeout: Duration) -> bool {
    let Some(handle) = isa_mutex_handle() else {
        return false;
    };
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let ms = ms.max(1);
    // SAFETY: process-lifetime handle from CreateMutexW
    let r = unsafe { WaitForSingleObject(handle, ms) };
    if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
        true
    } else {
        tracing::trace!(r, "ISA bus mutex wait timed out (process lock still held)");
        false
    }
}
