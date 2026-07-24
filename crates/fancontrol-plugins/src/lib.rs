//! Plugin traits and built-in providers for fancontrol-rs.
//!
//! Official providers may be compiled into the binary. Dynamic plugin loading
//! is planned for a later phase.

pub mod host;
pub mod mock;
#[cfg(windows)]
mod storage_win;
pub mod traits;

pub use host::HostSensorProvider;
pub use mock::MockProvider;
pub use traits::{ControlProvider, PluginError, ProviderRegistry, Result, SensorProvider};
