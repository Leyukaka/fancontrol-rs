//! PawnIO hardware backend for fancontrol-rs.
//!
//! Uses the system-installed `PawnIOLib.dll` + vendored signed modules
//! (`modules/LpcIO.bin`). Super I/O detection and Nuvoton banked HWM reads
//! are implemented; full chip coverage is incremental.

mod device;
mod ffi;
mod lpcio;
mod mutex_isa;
mod nct668;
mod provider;
mod session;
mod superio;

pub use nct668::HwmSample;
pub use provider::PawnioProvider;
pub use superio::{detect_chips, DetectedChip, SuperIoChip};

/// Installation present (DLL/sys on disk). Does **not** guarantee open rights.
pub fn is_installed() -> bool {
    ffi::dll_candidates().iter().any(|p| p.exists())
        || std::path::Path::new(r"C:\Program Files\PawnIO\PawnIO.sys").exists()
}

/// True only if we can open a PawnIO executor (typically requires elevation).
pub fn is_available() -> bool {
    #[cfg(windows)]
    {
        match session::PawnSession::open_embedded("Echo") {
            Ok(s) => {
                drop(s);
                true
            }
            Err(e) => {
                tracing::debug!(error = %e, "PawnIO open failed");
                false
            }
        }
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Human-readable status for CLI / UI.
pub fn status_message() -> String {
    let mut parts = Vec::new();
    match ffi::api() {
        Ok(api) => match api.version() {
            Ok(v) => parts.push(format!("PawnIOLib version raw={v} (0x{v:08X})")),
            Err(hr) => parts.push(format!("PawnIOLib loaded, version failed: {}", ffi::format_hr(hr))),
        },
        Err(e) => parts.push(format!("PawnIOLib: {e}")),
    }

    let sys = std::path::Path::new(r"C:\Program Files\PawnIO\PawnIO.sys");
    if sys.exists() {
        parts.push("PawnIO.sys present".into());
    } else {
        parts.push("PawnIO.sys not found under Program Files".into());
    }

    parts.push(format!("installed={}", is_installed()));
    parts.push(format!("executor_open={}", is_available()));

    match detect_chips() {
        Ok(chips) if chips.is_empty() => parts.push("Super I/O: none detected".into()),
        Ok(chips) => {
            for c in chips {
                parts.push(format!(
                    "Super I/O slot{} @0x{:02X}: {} hwm={:?}",
                    c.slot,
                    c.register_port,
                    c.chip.name(),
                    c.hwm_address.map(|a| format!("0x{a:04X}"))
                ));
            }
        }
        Err(e) => {
            parts.push(format!("Super I/O detect error: {e}"));
            if e.contains("ACCESSDENIED") || e.contains("0x80070005") {
                parts.push(
                    "Hint: open an elevated terminal (Run as administrator) and retry detect."
                        .into(),
                );
            }
        }
    }

    parts.join("\n")
}

/// Probe and build a **read-only** provider (may be empty if no supported chip).
pub fn try_provider() -> PawnioProvider {
    PawnioProvider::probe()
}

/// Probe with optional hardware write permission (`allow_write=false` by default policy).
pub fn try_provider_with_writes(allow_write: bool) -> PawnioProvider {
    PawnioProvider::probe_with_writes(allow_write)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_non_empty() {
        // Does not require hardware success — just that we produce text.
        let msg = status_message();
        assert!(!msg.is_empty());
    }

    #[test]
    fn embedded_modules_present() {
        let lpc = include_bytes!("../modules/LpcIO.bin");
        let echo = include_bytes!("../modules/Echo.bin");
        assert!(lpc.len() > 1000);
        assert!(echo.len() > 100);
    }
}
