//! Periodic fan control loop: read temps → evaluate curves → set duties.

use crate::curve::{evaluate_curve, CurveEvalState};
use crate::models::Profile;
use std::collections::HashMap;
use std::time::Duration;

/// One step of control: maps control id → computed duty.
#[derive(Debug, Clone, Default)]
pub struct ControlStepResult {
    pub duties: HashMap<String, u8>,
    pub temps: HashMap<String, f64>,
    pub errors: Vec<String>,
}

/// Stateless one-shot evaluation against a profile and current readings.
///
/// `temps`: sensor_id → °C  
/// For each assignment control→curve, looks up `sensor_bindings[control]` (or
/// falls back to first available temp) and evaluates the curve.
pub fn evaluate_profile_step(
    profile: &Profile,
    temps: &HashMap<String, f64>,
    states: &mut HashMap<String, CurveEvalState>,
) -> ControlStepResult {
    let mut result = ControlStepResult {
        temps: temps.clone(),
        ..Default::default()
    };

    for (control_id, curve_id) in &profile.assignments {
        let Some(curve) = profile.find_curve(curve_id) else {
            result
                .errors
                .push(format!("control {control_id}: missing curve {curve_id}"));
            continue;
        };

        let sensor_id = profile
            .sensor_bindings
            .get(control_id)
            .cloned()
            .or_else(|| temps.keys().next().cloned());

        let Some(sensor_id) = sensor_id else {
            result
                .errors
                .push(format!("control {control_id}: no temperature source"));
            continue;
        };

        let Some(&temp) = temps.get(&sensor_id) else {
            result.errors.push(format!(
                "control {control_id}: sensor {sensor_id} not in readings"
            ));
            continue;
        };

        let state = states.entry(control_id.clone()).or_default();
        let duty = evaluate_curve(curve, temp, Some(state));
        result.duties.insert(control_id.clone(), duty);
    }

    result
}

/// Suggested default poll interval for the control loop.
pub fn default_interval() -> Duration {
    Duration::from_millis(1000)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CurvePoint, FanCurve, Profile};

    #[test]
    fn evaluates_assignment() {
        let mut p = Profile::new("t", "t");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("c"),
            name: "c".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 100)],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        });
        p.assignments
            .insert("fan1".into(), "c".into());
        p.sensor_bindings
            .insert("fan1".into(), "cpu".into());

        let temps = HashMap::from([("cpu".into(), 50.0)]);
        let mut states = HashMap::new();
        let step = evaluate_profile_step(&p, &temps, &mut states);
        assert_eq!(step.duties.get("fan1"), Some(&60));
        assert!(step.errors.is_empty());
    }
}
