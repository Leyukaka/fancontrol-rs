//! Provider traits for sensors and fan controls.

use fancontrol_core::{ControlDescriptor, ControlId, SensorDescriptor, SensorId};
use thiserror::Error;

pub type Result<T> = std::result::Result<T, PluginError>;

#[derive(Debug, Error)]
pub enum PluginError {
    #[error("sensor not found: {0}")]
    SensorNotFound(String),

    #[error("control not found: {0}")]
    ControlNotFound(String),

    #[error("control is not writable: {0}")]
    NotWritable(String),

    #[error("backend unavailable: {0}")]
    BackendUnavailable(String),

    #[error("I/O error: {0}")]
    Io(String),

    #[error("{0}")]
    Other(String),
}

/// Provides temperature / RPM / other sensor readings.
pub trait SensorProvider: Send + Sync {
    /// Short stable name (e.g. `"mock"`, `"pawnio"`).
    fn name(&self) -> &str;

    /// List all sensors currently available from this provider.
    fn sensors(&self) -> Vec<SensorDescriptor>;

    /// Read the current value for a sensor.
    ///
    /// Convention:
    /// - Temperature → °C
    /// - FanRpm → revolutions per minute
    /// - Voltage → volts
    /// - Power → watts
    fn read(&self, id: &SensorId) -> Result<f64>;
}

/// Provides controllable fan / PWM outputs.
pub trait ControlProvider: Send + Sync {
    fn name(&self) -> &str;

    fn controls(&self) -> Vec<ControlDescriptor>;

    /// Set duty cycle 0..=100 (%).
    fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()>;

    /// Read last-set or reported duty cycle 0..=100 (%).
    fn get_duty(&self, id: &ControlId) -> Result<u8>;
}

/// Aggregates multiple providers for discovery and access.
#[derive(Default)]
pub struct ProviderRegistry {
    sensors: Vec<Box<dyn SensorProvider>>,
    controls: Vec<Box<dyn ControlProvider>>,
}

impl ProviderRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register_sensor_provider(&mut self, provider: Box<dyn SensorProvider>) {
        tracing::info!(provider = provider.name(), "registered sensor provider");
        self.sensors.push(provider);
    }

    pub fn register_control_provider(&mut self, provider: Box<dyn ControlProvider>) {
        tracing::info!(provider = provider.name(), "registered control provider");
        self.controls.push(provider);
    }

    /// Register a type that implements both traits (e.g. MockProvider).
    pub fn register_both<P>(&mut self, provider: P)
    where
        P: SensorProvider + ControlProvider + Clone + 'static,
    {
        self.register_sensor_provider(Box::new(provider.clone()));
        self.register_control_provider(Box::new(provider));
    }

    pub fn all_sensors(&self) -> Vec<SensorDescriptor> {
        self.sensors.iter().flat_map(|p| p.sensors()).collect()
    }

    pub fn all_controls(&self) -> Vec<ControlDescriptor> {
        self.controls.iter().flat_map(|p| p.controls()).collect()
    }

    pub fn read_sensor(&self, id: &SensorId) -> Result<f64> {
        let mut last_err = PluginError::SensorNotFound(id.to_string());
        for p in &self.sensors {
            match p.read(id) {
                Ok(v) => return Ok(v),
                Err(PluginError::SensorNotFound(_)) => continue,
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()> {
        let percent = percent.min(100);
        let mut last_err = PluginError::ControlNotFound(id.to_string());
        for p in &self.controls {
            match p.set_duty(id, percent) {
                Ok(()) => return Ok(()),
                Err(PluginError::ControlNotFound(_)) => continue,
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub fn get_duty(&self, id: &ControlId) -> Result<u8> {
        let mut last_err = PluginError::ControlNotFound(id.to_string());
        for p in &self.controls {
            match p.get_duty(id) {
                Ok(v) => return Ok(v),
                Err(PluginError::ControlNotFound(_)) => continue,
                Err(e) => last_err = e,
            }
        }
        Err(last_err)
    }

    pub fn sensor_provider_names(&self) -> Vec<&str> {
        self.sensors.iter().map(|p| p.name()).collect()
    }

    pub fn control_provider_names(&self) -> Vec<&str> {
        self.controls.iter().map(|p| p.name()).collect()
    }
}
