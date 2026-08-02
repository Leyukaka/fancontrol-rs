//! Windows self-elevation helpers (UAC via ShellExecute `runas`).
//!
//! No silent elevation: callers must invoke `relaunch_elevated` only after
//! explicit user action (dialog / top-bar button).

/// Whether this process is running with an elevated token (Administrator).
pub fn is_elevated() -> bool {
    #[cfg(windows)]
    {
        win::is_elevated()
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Relaunch the current executable with the same CLI args via UAC (`runas`).
///
/// On success the elevated process is started and this function returns `Ok(())`.
/// The caller should exit the non-elevated process. On UAC cancel or API failure,
/// returns `Err` with a short message (never panics).
pub fn relaunch_elevated() -> Result<(), ElevateError> {
    #[cfg(windows)]
    {
        win::relaunch_elevated()
    }
    #[cfg(not(windows))]
    {
        Err(ElevateError::Unsupported)
    }
}

/// Failure modes for elevation / relaunch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ElevateError {
    /// Process is already elevated; no relaunch needed.
    AlreadyElevated,
    /// User dismissed the UAC consent dialog.
    Cancelled,
    /// Platform does not support elevation (non-Windows).
    Unsupported,
    /// ShellExecute / token API failure with a detail string.
    Failed(String),
}

impl std::fmt::Display for ElevateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ElevateError::AlreadyElevated => write!(f, "already elevated"),
            ElevateError::Cancelled => write!(f, "UAC cancelled"),
            ElevateError::Unsupported => write!(f, "elevation not supported on this platform"),
            ElevateError::Failed(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for ElevateError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn elevate_error_display() {
        assert!(ElevateError::Cancelled.to_string().contains("UAC"));
        assert!(ElevateError::Failed("x".into()).to_string().contains('x'));
    }
}

#[cfg(windows)]
mod win {
    use super::ElevateError;
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, GetLastError, ERROR_CANCELLED, HANDLE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY,
    };
    use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
    use windows_sys::Win32::UI::Shell::{ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW};
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    fn wide(s: impl AsRef<OsStr>) -> Vec<u16> {
        s.as_ref()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect()
    }

    /// Quote a single argument the way CreateProcess / Shell expects (spaces).
    fn quote_arg(arg: &std::ffi::OsString) -> String {
        let s = arg.to_string_lossy();
        if s.is_empty() {
            return "\"\"".into();
        }
        let needs_quotes = s.chars().any(|c| c.is_whitespace() || c == '"');
        if !needs_quotes {
            return s.into_owned();
        }
        let mut out = String::from("\"");
        for c in s.chars() {
            if c == '"' {
                out.push('\\');
            }
            out.push(c);
        }
        out.push('"');
        out
    }

    pub fn is_elevated() -> bool {
        unsafe {
            let mut token: HANDLE = std::ptr::null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return false;
            }
            let mut elevation = TOKEN_ELEVATION {
                TokenIsElevated: 0,
            };
            let mut ret_len: u32 = 0;
            let ok = GetTokenInformation(
                token,
                TokenElevation,
                &mut elevation as *mut _ as *mut _,
                std::mem::size_of::<TOKEN_ELEVATION>() as u32,
                &mut ret_len,
            );
            CloseHandle(token);
            ok != 0 && elevation.TokenIsElevated != 0
        }
    }

    pub fn relaunch_elevated() -> Result<(), ElevateError> {
        if is_elevated() {
            return Err(ElevateError::AlreadyElevated);
        }

        let exe = std::env::current_exe().map_err(|e| ElevateError::Failed(e.to_string()))?;
        let exe_wide = wide(exe.as_os_str());

        let params: String = std::env::args_os()
            .skip(1)
            .map(|a| quote_arg(&a))
            .collect::<Vec<_>>()
            .join(" ");
        let params_wide = wide(params.as_str());
        let verb = wide("runas");

        // Zeroed SHELLEXECUTEINFOW; only fill fields we need.
        let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
        info.cbSize = std::mem::size_of::<SHELLEXECUTEINFOW>() as u32;
        info.fMask = SEE_MASK_NOCLOSEPROCESS;
        info.hwnd = std::ptr::null_mut();
        info.lpVerb = verb.as_ptr();
        info.lpFile = exe_wide.as_ptr();
        info.lpParameters = if params.is_empty() {
            std::ptr::null()
        } else {
            params_wide.as_ptr()
        };
        info.nShow = SW_SHOWNORMAL;

        let ok = unsafe { ShellExecuteExW(&mut info) };
        if ok == 0 {
            let err = unsafe { GetLastError() };
            if err == ERROR_CANCELLED {
                return Err(ElevateError::Cancelled);
            }
            return Err(ElevateError::Failed(format!(
                "ShellExecuteExW failed (error {err})"
            )));
        }

        // Close the process handle we asked for; we do not wait on the child.
        if !info.hProcess.is_null() {
            unsafe {
                CloseHandle(info.hProcess);
            }
        }

        tracing::info!(%params, "relaunched elevated via UAC runas");
        Ok(())
    }
}
