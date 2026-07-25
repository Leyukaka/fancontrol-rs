//! Desktop UI for fancontrol-rs (egui + eframe).

mod app;
mod curve_editor;
mod graph;
mod poll;
mod registry;
mod settings;
mod tray;
mod update_check;
mod write_queue;

pub use app::UiOptions;

use std::fmt;

pub fn is_implemented() -> bool {
    true
}

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
