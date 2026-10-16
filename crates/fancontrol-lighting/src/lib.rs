//! RGB / ARGB lighting for fancontrol-rs. See `specs/08-lighting.md`.
//!
//! Lighting controllers for fans and AIOs are USB HID devices: no kernel driver,
//! no PawnIO. Phase L1/L2: HID enumeration + MSI Mystic Light direct color frames.

pub mod color;
#[cfg(windows)]
pub mod hid;
pub mod known;
pub mod msi_mystic;

pub use color::Rgb;
pub use known::{KnownController, lookup_known};

#[derive(Debug, thiserror::Error)]
pub enum LightingError {
    #[error("invalid color {0:?} (expected RRGGBB hex)")]
    InvalidColor(String),
    #[error("header {header} out of range (0..{max})")]
    HeaderOutOfRange { header: u8, max: u8 },
    #[error("too many LEDs: {count} (max {max})")]
    TooManyLeds { count: usize, max: usize },
    #[error("HID: {0}")]
    Hid(String),
}

pub type Result<T> = std::result::Result<T, LightingError>;
