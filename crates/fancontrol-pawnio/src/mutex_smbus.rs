//! Coordinate SMBus access with other tools (HWiNFO, LibreHardwareMonitor, …)
//! via the well-known global mutex. Best-effort: absence is non-fatal, since
//! the SMBus module itself still serializes access within this process.

use std::sync::{Mutex, MutexGuard, OnceLock};
use std::time::Duration;
use windows_sys::Win32::Foundation::{FALSE, GetLastError, HANDLE, WAIT_OBJECT_0};
use windows_sys::Win32::System::Threading::{CreateMutexW, ReleaseMutex, WaitForSingleObject};

const SMBUS_MUTEX_NAME: &str = "Global\\Access_SMBUS.HTP.Method";
const WAIT_ABANDONED: u32 = 0x0000_0080;

fn process_smbus_mutex() -> &'static Mutex<()> {
    static M: OnceLock<Mutex<()>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(()))
}

/// Newtype so a process-lifetime HANDLE can live in a static.
struct SmbusHandle(HANDLE);
// SAFETY: Windows mutex handles are usable from any thread in the process.
unsafe impl Send for SmbusHandle {}
unsafe impl Sync for SmbusHandle {}

fn smbus_mutex_handle() -> Option<HANDLE> {
    static H: OnceLock<Option<SmbusHandle>> = OnceLock::new();
    H.get_or_init(|| {
        let wide: Vec<u16> = SMBUS_MUTEX_NAME
            .encode_utf16()
            .chain(std::iter::once(0))
            .collect();
        // SAFETY: null security attributes, valid name. Creates the object if
        // no other tool has it yet, or opens the existing one — either way
        // it becomes a shared cross-process handle.
        let handle = unsafe { CreateMutexW(std::ptr::null(), FALSE, wide.as_ptr()) };
        if handle.is_null() {
            let err = unsafe { GetLastError() };
            tracing::debug!(err, "CreateMutex SMBus failed (non-fatal)");
            None
        } else {
            Some(SmbusHandle(handle))
        }
    })
    .as_ref()
    .map(|h| h.0)
}

/// Best-effort SMBus mutex guard. Not holding it is never treated as an error
/// by callers — it only reduces the chance of colliding with another tool.
pub struct SmbusGuard {
    _process: MutexGuard<'static, ()>,
    held: bool,
}

impl SmbusGuard {
    pub fn acquire(timeout: Duration) -> Self {
        let process = process_smbus_mutex()
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let held = try_wait_smbus(timeout);
        Self {
            _process: process,
            held,
        }
    }
}

impl Drop for SmbusGuard {
    fn drop(&mut self) {
        if self.held
            && let Some(h) = smbus_mutex_handle()
        {
            unsafe {
                let _ = ReleaseMutex(h);
            }
        }
    }
}

fn try_wait_smbus(timeout: Duration) -> bool {
    let Some(handle) = smbus_mutex_handle() else {
        return false;
    };
    let ms = timeout.as_millis().min(u32::MAX as u128) as u32;
    let ms = ms.max(1);
    // SAFETY: process-lifetime handle from CreateMutexW
    let r = unsafe { WaitForSingleObject(handle, ms) };
    if r == WAIT_OBJECT_0 || r == WAIT_ABANDONED {
        true
    } else {
        tracing::trace!(r, "SMBus mutex wait timed out (proceeding without it)");
        false
    }
}
