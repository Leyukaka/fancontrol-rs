//! Desktop UI for fancontrol-rs (egui + eframe).
//!
//! **Status: stub.** The UI technology is locked to **egui + eframe**
//! (see `specs/04-ui.md`). Window, tray, and screens land in Phase 2.

use std::fmt;

/// Whether a real GUI can be launched (false until Phase 2 implements it).
pub fn is_implemented() -> bool {
    false
}

/// Placeholder entry point for the future egui app.
///
/// Returns an error until the UI is implemented so the binary can fall back
/// to CLI mode cleanly.
pub fn run() -> Result<(), UiError> {
    Err(UiError::NotImplemented)
}

#[derive(Debug)]
pub enum UiError {
    NotImplemented,
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::NotImplemented => write!(
                f,
                "UI not implemented yet (Phase 2). Use CLI subcommands for now."
            ),
        }
    }
}

impl std::error::Error for UiError {}
