//! Periodic fan control loop: read temps → evaluate curves → set duties.

use crate::curve::{CurveEvalState, apply_response_time, evaluate_curve};
use crate::models::Profile;
use crate::temp_source::resolve_curve_temp_sensor;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// Duty applied when a control's curve has no temperature to read: running blind
/// at the last duty could under-cool, so fail loud instead.
pub const FAILSAFE_DUTY: u8 = 100;

/// Consecutive steps without a temperature before [`FAILSAFE_DUTY`] kicks in, so
/// one transient sensor read error keeps the last duty instead of a fan burst.
pub const FAILSAFE_AFTER_MISSING_STEPS: u32 = 3;

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
/// falls back to first available temp) and evaluates the curve. A control whose
/// curve has no temperature source keeps its last duty, then gets [`FAILSAFE_DUTY`]
/// after [`FAILSAFE_AFTER_MISSING_STEPS`] consecutive misses (error entry each step).
///
/// Extra sensors (`profile.extra_sensors`) raise the input to the hottest of them
/// when present; only the bound CPU sensor is required.
pub fn evaluate_profile_step(
    profile: &Profile,
    temps: &HashMap<String, f64>,
    states: &mut HashMap<String, CurveEvalState>,
) -> ControlStepResult {
    evaluate_profile_step_at(profile, temps, states, Instant::now())
}

/// [`evaluate_profile_step`] at an explicit time (curve response time).
pub fn evaluate_profile_step_at(
    profile: &Profile,
    temps: &HashMap<String, f64>,
    states: &mut HashMap<String, CurveEvalState>,
    now: Instant,
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

        // Curves use CPU-like temps only. Stale NCT668x `…temp.CPU` bindings on
        // banked boards resolve to CPUTIN/PECI; non-CPU bindings (SYSTIN/VRM/GPU)
        // are ignored in favour of the best CPU-like reading.
        let sensor_id = resolve_curve_temp_sensor(
            profile.sensor_bindings.get(control_id).map(|s| s.as_str()),
            temps,
        );
        let state = states.entry(control_id.clone()).or_default();
        // A non-finite reading counts as missing: NaN fails every comparison in the
        // interpolation and would land on the curve's last (usually 100 %) point.
        let Some(temp) = sensor_id
            .as_ref()
            .and_then(|id| temps.get(id))
            .copied()
            .filter(|t| t.is_finite())
        else {
            state.missing_steps = state.missing_steps.saturating_add(1);
            if state.missing_steps >= FAILSAFE_AFTER_MISSING_STEPS {
                result.errors.push(format!(
                    "control {control_id}: no temperature source, failsafe {FAILSAFE_DUTY}%"
                ));
                result.duties.insert(control_id.clone(), FAILSAFE_DUTY);
                state.applied_duty = Some(FAILSAFE_DUTY);
                state.last_change = Some(now);
            } else {
                result
                    .errors
                    .push(format!("control {control_id}: no temperature source"));
            }
            continue;
        };
        state.missing_steps = 0;
        let temp = profile
            .extra_sensors
            .get(control_id)
            .into_iter()
            .flatten()
            .filter_map(|id| temps.get(id).copied())
            .filter(|t| t.is_finite())
            .fold(temp, f64::max);
        let duty = evaluate_curve(curve, temp, Some(state));
        let duty = apply_response_time(curve, state, duty, now);
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
        p.assignments.insert("fan1".into(), "c".into());
        p.sensor_bindings.insert("fan1".into(), "cpu".into());

        let temps = HashMap::from([("cpu".into(), 50.0)]);
        let mut states = HashMap::new();
        let step = evaluate_profile_step(&p, &temps, &mut states);
        assert_eq!(step.duties.get("fan1"), Some(&60));
        assert!(step.errors.is_empty());
    }

    #[test]
    fn stale_cpu_binding_uses_cputin() {
        let mut p = Profile::new("t", "t");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("c"),
            name: "c".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 100)],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        });
        p.assignments.insert("fan1".into(), "c".into());
        p.sensor_bindings
            .insert("fan1".into(), "pawnio.0.temp.CPU".into());

        let temps = HashMap::from([("pawnio.0.temp.CPUTIN".into(), 50.0)]);
        let mut states = HashMap::new();
        let step = evaluate_profile_step(&p, &temps, &mut states);
        assert_eq!(step.duties.get("fan1"), Some(&60));
        assert!(step.errors.is_empty());
    }

    #[test]
    fn systin_binding_ignored_follows_cputin() {
        let mut p = Profile::new("t", "t");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("c"),
            name: "c".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 100)],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        });
        p.assignments.insert("fan1".into(), "c".into());
        p.sensor_bindings
            .insert("fan1".into(), "pawnio.0.temp.SYSTIN".into());

        // SYSTIN cooler than CPUTIN: duty must track CPUTIN (50°C → 60%), not SYSTIN.
        let temps = HashMap::from([
            ("pawnio.0.temp.SYSTIN".into(), 30.0),
            ("pawnio.0.temp.CPUTIN".into(), 50.0),
        ]);
        let mut states = HashMap::new();
        let step = evaluate_profile_step(&p, &temps, &mut states);
        assert_eq!(step.duties.get("fan1"), Some(&60));
        assert!(step.errors.is_empty());
    }

    fn curve_profile(response_time_s: f64) -> Profile {
        let mut p = Profile::new("t", "t");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("c"),
            name: "c".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 100)],
            hysteresis_c: 0.0,
            response_time_s,
        });
        p.assignments.insert("fan1".into(), "c".into());
        p.sensor_bindings
            .insert("fan1".into(), "pawnio.0.temp.CPUTIN".into());
        p
    }

    #[test]
    fn extra_sensors_drive_the_hottest_input() {
        let mut p = curve_profile(0.0);
        p.extra_sensors.insert(
            "fan1".into(),
            vec!["host.gpu.0.temp".into(), "host.ssd.0.temp".into()],
        );
        let temps = HashMap::from([
            ("pawnio.0.temp.CPUTIN".into(), 30.0),
            ("host.gpu.0.temp".into(), 70.0),
        ]);
        let mut states = HashMap::new();
        let step = evaluate_profile_step(&p, &temps, &mut states);
        // GPU at 70 °C wins over CPU at 30 °C; the absent SSD is ignored.
        assert_eq!(step.duties.get("fan1"), Some(&100));

        // Extras alone never replace a missing CPU sensor (failsafe path instead).
        let gpu_only = HashMap::from([("host.gpu.0.temp".into(), 70.0)]);
        let mut states = HashMap::new();
        let step = evaluate_profile_step(&p, &gpu_only, &mut states);
        assert!(step.duties.is_empty());
    }

    #[test]
    fn response_time_delays_decreases_only() {
        let p = curve_profile(5.0);
        let t0 = Instant::now();
        let at = |s: u64| t0 + Duration::from_secs(s);
        let temps = |c: f64| HashMap::from([("pawnio.0.temp.CPUTIN".to_string(), c)]);
        let mut states = HashMap::new();

        let mut step = |c: f64, s: u64| {
            let r = evaluate_profile_step_at(&p, &temps(c), &mut states, at(s));
            r.duties.get("fan1").copied()
        };
        assert_eq!(step(30.0, 0), Some(20));
        // Rising: applied at once.
        assert_eq!(step(70.0, 1), Some(100));
        // Falling 2 s after the last change: held.
        assert_eq!(step(30.0, 3), Some(100));
        // 5 s after the last change: allowed down.
        assert_eq!(step(30.0, 6), Some(20));
    }

    #[test]
    fn non_finite_temp_counts_as_missing() {
        let mut p = Profile::new("t", "t");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("c"),
            name: "c".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 100)],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        });
        p.assignments.insert("fan1".into(), "c".into());
        p.sensor_bindings
            .insert("fan1".into(), "pawnio.0.temp.CPUTIN".into());

        for bad in [f64::NAN, f64::INFINITY] {
            let temps = HashMap::from([("pawnio.0.temp.CPUTIN".into(), bad)]);
            let mut states = HashMap::new();
            let step = evaluate_profile_step(&p, &temps, &mut states);
            assert!(step.duties.is_empty(), "{bad} must not drive the curve");
            assert_eq!(states["fan1"].missing_steps, 1);
        }
    }

    #[test]
    fn missing_cpu_temp_applies_failsafe() {
        let mut p = Profile::new("t", "t");
        p.curves.push(FanCurve {
            id: crate::models::CurveId::new("c"),
            name: "c".into(),
            points: vec![CurvePoint::new(30.0, 20), CurvePoint::new(70.0, 60)],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        });
        p.assignments.insert("fan1".into(), "c".into());

        // Only a non-CPU reading: the curve cannot run. A transient miss keeps the
        // last duty; a persistent one must not, so failsafe after the grace steps.
        let temps = HashMap::from([("pawnio.0.temp.SYSTIN".into(), 30.0)]);
        let mut states = HashMap::new();
        for _ in 1..FAILSAFE_AFTER_MISSING_STEPS {
            let step = evaluate_profile_step(&p, &temps, &mut states);
            assert!(step.duties.is_empty());
            assert_eq!(step.errors.len(), 1);
        }
        let step = evaluate_profile_step(&p, &temps, &mut states);
        assert_eq!(step.duties.get("fan1"), Some(&FAILSAFE_DUTY));

        // A reading comes back: curve resumes and the miss counter resets.
        let ok = HashMap::from([("pawnio.0.temp.CPUTIN".into(), 30.0)]);
        let step = evaluate_profile_step(&p, &ok, &mut states);
        assert_eq!(step.duties.get("fan1"), Some(&20));
        assert_eq!(states["fan1"].missing_steps, 0);
    }
}
