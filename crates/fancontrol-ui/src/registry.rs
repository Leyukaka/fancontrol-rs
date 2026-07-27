//! Build provider registry for the UI (mirrors CLI wiring).

use fancontrol_core::{ControlDescriptor, ControlId, SensorDescriptor, SensorId};
use fancontrol_pawnio::PawnioProvider;
use fancontrol_plugins::{
    ControlProvider, HostSensorProvider, MockProvider, ProviderRegistry, Result, SensorProvider,
};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub struct BuiltProviders {
    pub reg: ProviderRegistry,
    pub pawnio: Option<Arc<PawnioProvider>>,
    /// Live gate for host GPU/SSD sensors (shared with Options toggle).
    pub host_enabled: Arc<AtomicBool>,
}

pub fn build_registry(
    include_mock: bool,
    include_hw: bool,
    allow_hw_write: bool,
    include_host: bool,
) -> BuiltProviders {
    let mut reg = ProviderRegistry::new();
    let mut pawnio = None;
    if include_mock {
        reg.register_both(MockProvider::new());
    }
    if include_hw {
        let p = fancontrol_pawnio::try_provider_with_writes(allow_hw_write);
        tracing::info!(
            devices = p.device_count(),
            write = p.write_enabled(),
            "UI PawnIO probe\n{}",
            p.detection_report()
        );
        let arc = Arc::new(p);
        reg.register_sensor_provider(Box::new(ArcSensor(arc.clone())));
        reg.register_control_provider(Box::new(ArcControl(arc.clone())));
        pawnio = Some(arc);
    }
    // Always register host provider; gate with AtomicBool for live Options toggle.
    let host_enabled = Arc::new(AtomicBool::new(include_host));
    reg.register_sensor_provider(Box::new(HostSensorProvider::with_enabled(Arc::clone(
        &host_enabled,
    ))));
    BuiltProviders {
        reg,
        pawnio,
        host_enabled,
    }
}

struct ArcSensor(Arc<PawnioProvider>);
struct ArcControl(Arc<PawnioProvider>);

impl SensorProvider for ArcSensor {
    fn name(&self) -> &str {
        SensorProvider::name(&*self.0)
    }
    fn sensors(&self) -> Vec<SensorDescriptor> {
        SensorProvider::sensors(&*self.0)
    }
    fn read(&self, id: &SensorId) -> Result<f64> {
        SensorProvider::read(&*self.0, id)
    }
}

impl ControlProvider for ArcControl {
    fn name(&self) -> &str {
        ControlProvider::name(&*self.0)
    }
    fn controls(&self) -> Vec<ControlDescriptor> {
        ControlProvider::controls(&*self.0)
    }
    fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()> {
        ControlProvider::set_duty(&*self.0, id, percent)
    }
    fn get_duty(&self, id: &ControlId) -> Result<u8> {
        ControlProvider::get_duty(&*self.0, id)
    }
}

/// Backend hardware-probe status, translated fresh every frame by the caller
/// (rather than pre-formatted once at startup) so a live language switch applies to it.
pub enum BackendStatus {
    Disabled,
    Ok(String),
    NeedsAdmin,
    NotInstalled,
}

pub fn backend_status(include_hw: bool) -> BackendStatus {
    if !include_hw {
        return BackendStatus::Disabled;
    }
    if fancontrol_pawnio::is_available() {
        let detail = fancontrol_pawnio::status_message()
            .lines()
            .find(|l| l.contains("Super I/O") || l.contains("NCT") || l.contains("slot"))
            .unwrap_or("executor open")
            .to_string();
        BackendStatus::Ok(detail)
    } else if fancontrol_pawnio::is_installed() {
        BackendStatus::NeedsAdmin
    } else {
        BackendStatus::NotInstalled
    }
}
