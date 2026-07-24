//! Plugin traits and built-in providers for fancontrol-rs.
//!
//! Official providers may be compiled into the binary. Dynamic plugin loading
//! is planned for a later phase.

pub mod mock;
pub mod traits;

pub use mock::MockProvider;
pub use traits::{ControlProvider, PluginError, ProviderRegistry, Result, SensorProvider};
