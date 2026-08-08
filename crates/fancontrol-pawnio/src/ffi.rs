//! Dynamic loading of `PawnIOLib.dll` (no link-time dependency).

#![allow(clippy::upper_case_acronyms)] // Win32 names: HRESULT, HANDLE

use libloading::{Library, Symbol};
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

pub type HRESULT = i32;
pub type HANDLE = *mut c_void;

pub const S_OK: HRESULT = 0;

type FnVersion = unsafe extern "system" fn(*mut u32) -> HRESULT;
type FnOpen = unsafe extern "system" fn(*mut HANDLE) -> HRESULT;
type FnLoad = unsafe extern "system" fn(HANDLE, *const u8, usize) -> HRESULT;
type FnExecute = unsafe extern "system" fn(
    HANDLE,
    *const u8,
    *const u64,
    usize,
    *mut u64,
    usize,
    *mut usize,
) -> HRESULT;
type FnClose = unsafe extern "system" fn(HANDLE) -> HRESULT;

pub struct PawnIoApi {
    _lib: Library,
    version: FnVersion,
    open: FnOpen,
    load: FnLoad,
    execute: FnExecute,
    close: FnClose,
}

// SAFETY: PawnIOLib is a C DLL with process-wide driver access; we serialize
// higher-level use with mutexes. Function pointers remain valid while `_lib` lives.
unsafe impl Send for PawnIoApi {}
unsafe impl Sync for PawnIoApi {}

impl PawnIoApi {
    pub fn load_from(path: &Path) -> Result<Self, String> {
        // SAFETY: path points to the official PawnIOLib.dll; we only call documented exports.
        let lib = unsafe { Library::new(path) }
            .map_err(|e| format!("LoadLibrary({}): {e}", path.display()))?;
        unsafe {
            let version: Symbol<FnVersion> = lib
                .get(b"pawnio_version\0")
                .map_err(|e| format!("pawnio_version: {e}"))?;
            let open: Symbol<FnOpen> = lib
                .get(b"pawnio_open\0")
                .map_err(|e| format!("pawnio_open: {e}"))?;
            let load: Symbol<FnLoad> = lib
                .get(b"pawnio_load\0")
                .map_err(|e| format!("pawnio_load: {e}"))?;
            let execute: Symbol<FnExecute> = lib
                .get(b"pawnio_execute\0")
                .map_err(|e| format!("pawnio_execute: {e}"))?;
            let close: Symbol<FnClose> = lib
                .get(b"pawnio_close\0")
                .map_err(|e| format!("pawnio_close: {e}"))?;
            Ok(Self {
                version: *version,
                open: *open,
                load: *load,
                execute: *execute,
                close: *close,
                _lib: lib,
            })
        }
    }

    pub fn version(&self) -> Result<u32, HRESULT> {
        let mut v = 0u32;
        let hr = unsafe { (self.version)(&mut v) };
        if hr == S_OK { Ok(v) } else { Err(hr) }
    }

    pub fn open(&self) -> Result<HANDLE, HRESULT> {
        let mut h: HANDLE = std::ptr::null_mut();
        let hr = unsafe { (self.open)(&mut h) };
        if hr == S_OK && !h.is_null() {
            Ok(h)
        } else {
            Err(hr)
        }
    }

    pub fn load_blob(&self, handle: HANDLE, blob: &[u8]) -> Result<(), HRESULT> {
        let hr = unsafe { (self.load)(handle, blob.as_ptr(), blob.len()) };
        if hr == S_OK { Ok(()) } else { Err(hr) }
    }

    pub fn execute(
        &self,
        handle: HANDLE,
        name: &str,
        input: &[u64],
        out_len: usize,
    ) -> Result<Vec<u64>, HRESULT> {
        let mut name_buf = name.as_bytes().to_vec();
        name_buf.push(0);
        let mut out = vec![0u64; out_len.max(1)];
        let mut ret = 0usize;
        let hr = unsafe {
            (self.execute)(
                handle,
                name_buf.as_ptr(),
                if input.is_empty() {
                    std::ptr::null()
                } else {
                    input.as_ptr()
                },
                input.len(),
                out.as_mut_ptr(),
                out_len,
                &mut ret,
            )
        };
        if hr == S_OK {
            out.truncate(ret.min(out_len));
            if out_len == 0 {
                out.clear();
            }
            Ok(out)
        } else {
            Err(hr)
        }
    }

    pub fn close(&self, handle: HANDLE) {
        if !handle.is_null() {
            let _ = unsafe { (self.close)(handle) };
        }
    }
}

static API: OnceLock<Result<PawnIoApi, String>> = OnceLock::new();

pub fn dll_candidates() -> Vec<PathBuf> {
    let mut v = Vec::new();
    v.push(PathBuf::from(r"C:\Program Files\PawnIO\PawnIOLib.dll"));
    v.push(PathBuf::from(
        r"C:\Program Files (x86)\PawnIO\PawnIOLib.dll",
    ));
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        v.push(dir.join("PawnIOLib.dll"));
    }
    v
}

pub fn api() -> Result<&'static PawnIoApi, String> {
    API.get_or_init(|| {
        let mut last = "PawnIOLib.dll not found".to_string();
        for p in dll_candidates() {
            if p.exists() {
                match PawnIoApi::load_from(&p) {
                    Ok(api) => {
                        tracing::info!(path = %p.display(), "loaded PawnIOLib");
                        return Ok(api);
                    }
                    Err(e) => last = e,
                }
            }
        }
        Err(last)
    })
    .as_ref()
    .map_err(|e| e.clone())
}

pub fn format_hr(hr: HRESULT) -> String {
    let name = match hr as u32 {
        0x80070005 => "E_ACCESSDENIED (run as Administrator)",
        0x80070002 => "ERROR_FILE_NOT_FOUND",
        0x80070057 => "E_INVALIDARG",
        0x80004005 => "E_FAIL",
        _ => "",
    };
    if name.is_empty() {
        format!("HRESULT 0x{hr:08X}")
    } else {
        format!("HRESULT 0x{hr:08X} {name}")
    }
}
