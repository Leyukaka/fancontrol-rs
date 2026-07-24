//! PawnIO hardware backend for fancontrol-rs.
//!
//! **Status: stub.** Real bindings to the PawnIO driver/service will be added
//! in Phase 1. For now this crate only exposes availability detection and a
//! provider that reports "backend unavailable".

use fancontrol_core::{ControlDescriptor, ControlId, SensorDescriptor, SensorId};
use fancontrol_plugins::{ControlProvider, PluginError, Result, SensorProvider};

/// Probe whether PawnIO appears to be installed/available on this system.
///
/// Current heuristic (Windows): check for a well-known install path or service.
/// Always returns `false` on non-Windows until a real probe exists.
pub fn is_available() -> bool {
    #[cfg(windows)]
    {
        // Placeholder paths — refine once PawnIO install layout is confirmed.
        let candidates = [
            r"C:\Program Files\PawnIO",
            r"C:\Program Files (x86)\PawnIO",
        ];
        candidates.iter().any(|p| std::path::Path::new(p).exists())
    }
    #[cfg(not(windows))]
    {
        false
    }
}

/// Human-readable status for UI / CLI.
pub fn status_message() -> String {
    if is_available() {
        "PawnIO appears installed (bindings not yet implemented)".into()
    } else {
        "PawnIO not detected. Install from https://pawnio.eu/ — hardware sensors will be unavailable until then.".into()
    }
}

/// Stub provider: discovers nothing, all reads/writes fail with BackendUnavailable.
#[derive(Debug, Default, Clone)]
pub struct PawnioProvider;

impl PawnioProvider {
    pub fn new() -> Self {
        Self
    }
}

impl SensorProvider for PawnioProvider {
    fn name(&self) -> &str {
        "pawnio"
    }

    fn sensors(&self) -> Vec<SensorDescriptor> {
        // Real discovery will query loaded PawnIO modules.
        Vec::new()
    }

    fn read(&self, id: &SensorId) -> Result<f64> {
        let _ = id;
        Err(PluginError::BackendUnavailable(status_message()))
    }
}

impl ControlProvider for PawnioProvider {
    fn name(&self) -> &str {
        "pawnio"
    }

    fn controls(&self) -> Vec<ControlDescriptor> {
        Vec::new()
    }

    fn set_duty(&self, id: &ControlId, _percent: u8) -> Result<()> {
        let _ = id;
        Err(PluginError::BackendUnavailable(status_message()))
    }

    fn get_duty(&self, id: &ControlId) -> Result<u8> {
        let _ = id;
        Err(PluginError::BackendUnavailable(status_message()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_is_non_empty() {
        assert!(!status_message().is_empty());
    }

    #[test]
    fn stub_has_no_sensors() {
        let p = PawnioProvider::new();
        assert!(p.sensors().is_empty());
        assert!(p.controls().is_empty());
    }
}
