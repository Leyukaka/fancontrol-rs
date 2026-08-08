//! Windows "start with Windows" via HKCU Run key (no admin, no PowerShell).

/// Registry value name under the current user's Run key.
pub const RUN_VALUE_NAME: &str = "fancontrol-rs";

/// Whether the app is registered to launch at user logon.
pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        win::is_enabled()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Enable or disable launch at user logon. Updates the Run key to the current exe path.
pub fn set_enabled(on: bool) -> Result<(), String> {
    #[cfg(windows)]
    {
        if on { win::enable() } else { win::disable() }
    }
    #[cfg(not(windows))]
    {
        let _ = on;
        Err("start with Windows is only supported on Windows".into())
    }
}

/// Re-write the Run key if enabled so a moved/updated exe path stays valid.
pub fn refresh_if_enabled() {
    #[cfg(windows)]
    {
        if win::is_enabled() {
            let _ = win::enable();
        }
    }
}

#[cfg(windows)]
mod win {
    use super::RUN_VALUE_NAME;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use std::path::PathBuf;
    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_QUERY_VALUE, KEY_SET_VALUE, REG_SZ, RegCloseKey,
        RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    };

    const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";

    fn wide(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    fn open_run_key(access: u32) -> Result<HKEY, String> {
        let sub = wide(RUN_SUBKEY);
        let mut hkey: HKEY = std::ptr::null_mut();
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, sub.as_ptr(), 0, access, &mut hkey) };
        if status != ERROR_SUCCESS {
            return Err(format!("RegOpenKeyExW failed: {status}"));
        }
        Ok(hkey)
    }

    fn current_exe_quoted() -> Result<String, String> {
        let path: PathBuf = std::env::current_exe().map_err(|e| e.to_string())?;
        let s = path.to_string_lossy();
        // Quote path for spaces; optional future: append " --minimized".
        Ok(format!("\"{s}\""))
    }

    pub fn is_enabled() -> bool {
        let Ok(hkey) = open_run_key(KEY_QUERY_VALUE) else {
            return false;
        };
        let name = wide(RUN_VALUE_NAME);
        let mut typ: u32 = 0;
        let mut size: u32 = 0;
        let status = unsafe {
            RegQueryValueExW(
                hkey,
                name.as_ptr(),
                std::ptr::null_mut(),
                &mut typ,
                std::ptr::null_mut(),
                &mut size,
            )
        };
        unsafe {
            RegCloseKey(hkey);
        }
        status == ERROR_SUCCESS && size > 2
    }

    pub fn enable() -> Result<(), String> {
        let cmd = current_exe_quoted()?;
        let hkey = open_run_key(KEY_SET_VALUE | KEY_QUERY_VALUE)?;
        let name = wide(RUN_VALUE_NAME);
        let data = wide(&cmd);
        // REG_SZ data size is bytes including null.
        let bytes = (data.len() * 2) as u32;
        let status = unsafe {
            RegSetValueExW(
                hkey,
                name.as_ptr(),
                0,
                REG_SZ,
                data.as_ptr() as *const u8,
                bytes,
            )
        };
        unsafe {
            RegCloseKey(hkey);
        }
        if status != ERROR_SUCCESS {
            return Err(format!("RegSetValueExW failed: {status}"));
        }
        tracing::info!(%cmd, "autostart enabled (HKCU Run)");
        Ok(())
    }

    pub fn disable() -> Result<(), String> {
        let hkey = open_run_key(KEY_SET_VALUE)?;
        let name = wide(RUN_VALUE_NAME);
        let status = unsafe { RegDeleteValueW(hkey, name.as_ptr()) };
        unsafe {
            RegCloseKey(hkey);
        }
        // Already absent is success.
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return Err(format!("RegDeleteValueW failed: {status}"));
        }
        tracing::info!("autostart disabled (HKCU Run)");
        Ok(())
    }
}
