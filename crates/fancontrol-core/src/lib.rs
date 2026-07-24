//! Domain models and business logic for fancontrol-rs.
//!
//! Pure logic only: sensors/controls descriptors, fan curves, profiles,
//! and config paths. No hardware access lives here.

pub mod config;
pub mod curve;
pub mod error;
pub mod models;
pub mod profile;

pub use config::{config_dir, profiles_dir, ensure_config_dirs};
pub use curve::{evaluate_curve, CurveEvalState};
pub use error::{CoreError, Result};
pub use models::{
    ControlDescriptor, ControlId, CurveId, CurvePoint, FanCurve, Profile, ProfileId, SensorDescriptor,
    SensorId, SensorKind,
};
pub use profile::{load_profile, save_profile, list_profiles, delete_profile};
