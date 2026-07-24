//! Fan curve evaluation (linear interpolation + optional hysteresis).

use crate::models::{CurvePoint, FanCurve};

/// Runtime state for hysteresis-aware evaluation.
///
/// Keep one instance per control that is driven by a curve.
#[derive(Debug, Clone, Default)]
pub struct CurveEvalState {
    /// Last temperature that was allowed to lower the duty.
    pub last_temp_for_down: Option<f64>,
    /// Last computed duty (0..=100).
    pub last_duty: Option<u8>,
}

/// Evaluate a fan curve at the given temperature (°C).
///
/// - Points are sorted by temperature before interpolation.
/// - Below the first point → first duty; above the last → last duty.
/// - Between points → linear interpolation, duty rounded to nearest integer.
/// - If `hysteresis_c > 0` and temperature is falling, duty only decreases when
///   temperature drops by at least `hysteresis_c` below the last peak used for
///   a downward step (simple anti-oscillation).
///
/// When `state` is `None`, hysteresis is ignored (pure interpolation).
pub fn evaluate_curve(
    curve: &FanCurve,
    temperature: f64,
    state: Option<&mut CurveEvalState>,
) -> u8 {
    let raw = interpolate_duty(&curve.points, temperature);

    let Some(state) = state else {
        return raw;
    };

    if curve.hysteresis_c <= 0.0 {
        state.last_duty = Some(raw);
        state.last_temp_for_down = Some(temperature);
        return raw;
    }

    let last_duty = state.last_duty.unwrap_or(raw);

    // Temperature rising or equal: allow duty to follow curve freely upward.
    if state
        .last_temp_for_down
        .map(|t| temperature >= t)
        .unwrap_or(true)
        || raw >= last_duty
    {
        state.last_duty = Some(raw);
        // Track the high-water temperature while rising / holding.
        let prev = state.last_temp_for_down.unwrap_or(temperature);
        state.last_temp_for_down = Some(prev.max(temperature));
        return raw;
    }

    // Temperature falling and curve wants lower duty: require hysteresis gap.
    let peak = state.last_temp_for_down.unwrap_or(temperature);
    if peak - temperature >= curve.hysteresis_c {
        state.last_duty = Some(raw);
        state.last_temp_for_down = Some(temperature);
        raw
    } else {
        // Hold previous duty
        last_duty
    }
}

/// Pure linear interpolation without hysteresis.
pub fn interpolate_duty(points: &[CurvePoint], temperature: f64) -> u8 {
    if points.is_empty() {
        return 0;
    }

    let mut sorted: Vec<CurvePoint> = points.to_vec();
    sorted.sort_by(|a, b| {
        a.temperature
            .partial_cmp(&b.temperature)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    if sorted.len() == 1 || temperature <= sorted[0].temperature {
        return sorted[0].duty.min(100);
    }

    if temperature >= sorted[sorted.len() - 1].temperature {
        return sorted[sorted.len() - 1].duty.min(100);
    }

    for window in sorted.windows(2) {
        let a = window[0];
        let b = window[1];
        if temperature >= a.temperature && temperature <= b.temperature {
            let span = b.temperature - a.temperature;
            if span.abs() < f64::EPSILON {
                return a.duty.min(100);
            }
            let t = (temperature - a.temperature) / span;
            let duty = f64::from(a.duty) + t * (f64::from(b.duty) - f64::from(a.duty));
            return duty.round().clamp(0.0, 100.0) as u8;
        }
    }

    sorted[sorted.len() - 1].duty.min(100)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{CurveId, CurvePoint, FanCurve};

    fn sample_curve() -> FanCurve {
        FanCurve {
            id: CurveId::new("quiet"),
            name: "Quiet".into(),
            points: vec![
                CurvePoint::new(30.0, 20),
                CurvePoint::new(50.0, 50),
                CurvePoint::new(70.0, 100),
            ],
            hysteresis_c: 0.0,
            response_time_s: 0.0,
        }
    }

    #[test]
    fn interpolate_below_min() {
        let c = sample_curve();
        assert_eq!(evaluate_curve(&c, 10.0, None), 20);
    }

    #[test]
    fn interpolate_above_max() {
        let c = sample_curve();
        assert_eq!(evaluate_curve(&c, 90.0, None), 100);
    }

    #[test]
    fn interpolate_midpoint() {
        let c = sample_curve();
        // Midway 30→50 temp maps 20→50 duty → 40 at 40°C
        assert_eq!(evaluate_curve(&c, 40.0, None), 35);
    }

    #[test]
    fn interpolate_exact_point() {
        let c = sample_curve();
        assert_eq!(evaluate_curve(&c, 50.0, None), 50);
    }

    #[test]
    fn empty_points_returns_zero() {
        assert_eq!(interpolate_duty(&[], 40.0), 0);
    }

    #[test]
    fn single_point_constant() {
        let points = [CurvePoint::new(40.0, 42)];
        assert_eq!(interpolate_duty(&points, 10.0), 42);
        assert_eq!(interpolate_duty(&points, 100.0), 42);
    }

    #[test]
    fn hysteresis_holds_on_small_drop() {
        let mut c = sample_curve();
        c.hysteresis_c = 5.0;
        let mut state = CurveEvalState::default();

        // Rise to 60°C → duty between 50 and 100
        let d1 = evaluate_curve(&c, 60.0, Some(&mut state));
        assert!(d1 > 50);

        // Small drop 2°C — should hold previous duty
        let d2 = evaluate_curve(&c, 58.0, Some(&mut state));
        assert_eq!(d2, d1);

        // Large drop beyond hysteresis — allow decrease
        let d3 = evaluate_curve(&c, 50.0, Some(&mut state));
        assert!(d3 <= d1);
        assert_eq!(d3, 50);
    }

    #[test]
    fn linear_helper() {
        let c = FanCurve::linear("lin", "Linear", 30.0, 70.0, 0, 100);
        assert_eq!(evaluate_curve(&c, 50.0, None), 50);
    }
}
