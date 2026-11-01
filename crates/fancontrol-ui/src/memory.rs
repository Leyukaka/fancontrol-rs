//! Process memory housekeeping.

/// Ask Windows to page out as much of the process working set as it can.
///
/// Called once after the window is hidden to the tray: Windows trims a minimized
/// window's process on its own, but not a hidden one, so the GPU / font / UI pages
/// would otherwise stay resident while only the background loop runs. Pages come
/// back on demand when the window is shown again; committed memory is unchanged.
pub fn trim_working_set() {
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::Threading::{GetCurrentProcess, SetProcessWorkingSetSize};
        // (usize::MAX, usize::MAX) is the documented "remove as many pages as
        // possible" request; GetCurrentProcess returns a pseudo-handle, no close needed.
        let ok = unsafe { SetProcessWorkingSetSize(GetCurrentProcess(), usize::MAX, usize::MAX) };
        if ok == 0 {
            tracing::debug!(
                error = %std::io::Error::last_os_error(),
                "working set trim failed"
            );
        }
    }
}
