//! Core domain models.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for a temperature (or other) sensor.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SensorId(pub String);

impl SensorId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for SensorId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a controllable fan / PWM channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ControlId(pub String);

impl ControlId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ControlId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Kind of sensor reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SensorKind {
    Temperature,
    FanRpm,
    Voltage,
    Power,
    Other,
}

/// Static description of a sensor (does not hold live values).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensorDescriptor {
    pub id: SensorId,
    pub name: String,
    pub kind: SensorKind,
    /// Provider that owns this sensor (e.g. "mock", "pawnio").
    pub provider: String,
    /// Optional unit label for display (e.g. "°C", "RPM").
    pub unit: Option<String>,
}

/// Static description of a controllable output.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlDescriptor {
    pub id: ControlId,
    pub name: String,
    /// Provider that owns this control.
    pub provider: String,
    /// Whether software can write a duty cycle.
    pub writable: bool,
    /// Optional paired RPM sensor id.
    pub rpm_sensor: Option<SensorId>,
}

/// A single point on a fan curve: temperature (°C) → duty (%).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    /// Temperature in degrees Celsius.
    pub temperature: f64,
    /// Fan duty cycle percentage, clamped to 0..=100 when applied.
    pub duty: u8,
}

impl CurvePoint {
    pub fn new(temperature: f64, duty: u8) -> Self {
        Self {
            temperature,
            duty: duty.min(100),
        }
    }
}

/// Identifier for a named fan curve.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CurveId(pub String);

impl CurveId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A named fan curve with optional hysteresis / response settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanCurve {
    pub id: CurveId,
    pub name: String,
    /// Points sorted by ascending temperature. Must contain at least two points
    /// for meaningful interpolation (one point is allowed and acts as constant duty).
    pub points: Vec<CurvePoint>,
    /// Hysteresis in °C: temperature must fall by this amount before the curve
    /// is allowed to reduce duty (reduces oscillation). `0.0` disables.
    #[serde(default)]
    pub hysteresis_c: f64,
    /// Minimum seconds between duty changes (soft rate limit). `0.0` disables.
    #[serde(default)]
    pub response_time_s: f64,
}

impl FanCurve {
    /// Create a simple two-point linear curve.
    pub fn linear(
        id: impl Into<String>,
        name: impl Into<String>,
        min_temp: f64,
        max_temp: f64,
        min_duty: u8,
        max_duty: u8,
    ) -> Self {
        Self {
            id: CurveId::new(id),
            name: name.into(),
            points: vec![
                CurvePoint::new(min_temp, min_duty),
                CurvePoint::new(max_temp, max_duty),
            ],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        }
    }

    /// Sort points by temperature ascending (stable for equal temps).
    pub fn sort_points(&mut self) {
        self.points.sort_by(|a, b| {
            a.temperature
                .partial_cmp(&b.temperature)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
    }

    /// Validate curve shape. Returns an error message if invalid.
    pub fn validate(&self) -> Result<(), String> {
        if self.points.is_empty() {
            return Err("curve must have at least one point".into());
        }
        for p in &self.points {
            if !(0.0..=150.0).contains(&p.temperature) {
                return Err(format!(
                    "temperature out of range (0..=150): {}",
                    p.temperature
                ));
            }
            if p.duty > 100 {
                return Err(format!("duty out of range (0..=100): {}", p.duty));
            }
        }
        // Ensure non-decreasing temperatures after sort check
        let mut sorted = self.points.clone();
        sorted.sort_by(|a, b| {
            a.temperature
                .partial_cmp(&b.temperature)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        for w in sorted.windows(2) {
            if w[1].temperature < w[0].temperature {
                return Err("points must be sortable by temperature".into());
            }
        }
        if self.hysteresis_c < 0.0 {
            return Err("hysteresis_c must be >= 0".into());
        }
        if self.response_time_s < 0.0 {
            return Err("response_time_s must be >= 0".into());
        }
        Ok(())
    }
}

/// Identifier for a saved profile.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProfileId(pub String);

impl ProfileId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A complete configuration: named curves + control→curve assignments.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub id: ProfileId,
    pub name: String,
    /// All curves available in this profile.
    pub curves: Vec<FanCurve>,
    /// Map control id → curve id.
    #[serde(default)]
    pub assignments: HashMap<String, String>,
    /// Optional sensor id used as the temperature source for a control
    /// (control id → sensor id). If missing, the UI/core may pick a default.
    #[serde(default)]
    pub sensor_bindings: HashMap<String, String>,
}

impl Profile {
    pub fn new(id: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            id: ProfileId::new(id),
            name: name.into(),
            curves: Vec::new(),
            assignments: HashMap::new(),
            sensor_bindings: HashMap::new(),
        }
    }

    pub fn find_curve(&self, curve_id: &str) -> Option<&FanCurve> {
        self.curves.iter().find(|c| c.id.as_str() == curve_id)
    }

    pub fn assignment_for(&self, control_id: &str) -> Option<&FanCurve> {
        self.assignments
            .get(control_id)
            .and_then(|cid| self.find_curve(cid))
    }

    pub fn validate(&self) -> Result<(), String> {
        for curve in &self.curves {
            curve.validate()?;
        }
        for (control, curve_id) in &self.assignments {
            if self.find_curve(curve_id).is_none() {
                return Err(format!(
                    "assignment for control '{control}' references unknown curve '{curve_id}'"
                ));
            }
        }
        Ok(())
    }
}
