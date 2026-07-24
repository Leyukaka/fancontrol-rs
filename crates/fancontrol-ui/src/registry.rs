//! Build provider registry for the UI (mirrors CLI wiring).

use fancontrol_core::{ControlDescriptor, ControlId, SensorDescriptor, SensorId};
use fancontrol_plugins::{
    ControlProvider, MockProvider, ProviderRegistry, Result, SensorProvider,
};
use fancontrol_pawnio::PawnioProvider;
use std::sync::Arc;

pub fn build_registry(include_mock: bool, include_hw: bool, allow_hw_write: bool) -> ProviderRegistry {
    let mut reg = ProviderRegistry::new();
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
        reg.register_control_provider(Box::new(ArcControl(arc)));
    }
    reg
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

pub fn backend_status_line(include_hw: bool) -> String {
    if !include_hw {
        return "Hardware probe disabled (--no-hw)".into();
    }
    if fancontrol_pawnio::is_available() {
        format!(
            "PawnIO OK · {}",
            fancontrol_pawnio::status_message()
                .lines()
                .find(|l| l.contains("Super I/O") || l.contains("NCT") || l.contains("slot"))
                .unwrap_or("executor open")
        )
    } else if fancontrol_pawnio::is_installed() {
        "PawnIO installed but executor closed (run as Administrator?)".into()
    } else {
        "PawnIO not installed".into()
    }
}

