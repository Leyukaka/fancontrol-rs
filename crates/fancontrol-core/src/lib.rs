//! Domain models and business logic for fancontrol-rs.
//!
//! Pure logic only: sensors/controls descriptors, fan curves, profiles,
//! and config paths. No hardware access lives here.

pub mod channel_map;
pub mod config;
pub mod control_loop;
pub mod curve;
pub mod error;
pub mod models;
pub mod profile;
pub mod temp_source;

pub use channel_map::ChannelMap;
pub use config::{config_dir, ensure_config_dirs, profiles_dir};
pub use control_loop::{default_interval, evaluate_profile_step, ControlStepResult};
pub use curve::{evaluate_curve, CurveEvalState};
pub use error::{CoreError, Result};
pub use models::{
    ControlDescriptor, ControlId, CurveId, CurvePoint, FanCurve, Profile, ProfileId,
    SensorDescriptor, SensorId, SensorKind,
};
pub use profile::{delete_profile, list_profiles, load_profile, save_profile};
pub use temp_source::{
    cpu_temp_seed_priority, is_cpu_temp_candidate, pick_cpu_temp_id, resolve_curve_temp_sensor,
    temp_sensor_short_name,
};
