//! Desktop UI for fancontrol-rs (egui + eframe).
//!
//! Live temperatures, fan RPM, and duty sliders. Hardware PWM writes require
//! `UiOptions::allow_hw_write` (same policy as the CLI).

mod app;
mod poll;
mod registry;

pub use app::UiOptions;

use std::fmt;

/// Whether a real GUI is available.
pub fn is_implemented() -> bool {
    true
}

/// Launch the egui application (blocking until the window closes).
pub fn run(options: UiOptions) -> Result<(), UiError> {
    app::run_native(options)
}

#[derive(Debug)]
pub enum UiError {
    Eframe(String),
    Backend(String),
}

impl fmt::Display for UiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UiError::Eframe(s) | UiError::Backend(s) => write!(f, "{s}"),
        }
    }
}

impl std::error::Error for UiError {}
