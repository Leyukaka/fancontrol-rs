//! Error types for fancontrol-core.

use thiserror::Error;

pub type Result<T> = std::result::Result<T, CoreError>;

#[derive(Debug, Error)]
pub enum CoreError {
    #[error("invalid fan curve: {0}")]
    InvalidCurve(String),

    #[error("profile not found: {0}")]
    ProfileNotFound(String),

    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    #[error("serialization error: {0}")]
    Serde(#[from] serde_json::Error),

    #[error("config path unavailable: {0}")]
    ConfigPath(String),
}
