//! CPU temperature sparkline with glow fill and configurable time window.

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, StrokeKind, Vec2};
use std::collections::VecDeque;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct TempHistory {
    /// `(timestamp, temp_c)` samples, oldest first.
    samples: VecDeque<(Instant, f32)>,
    window: Duration,
    sample_interval: Duration,
    max_len: usize,
    last_push: Option<Instant>,
}

impl Default for TempHistory {
    fn default() -> Self {
        let mut h = Self {
            samples: VecDeque::new(),
            window: Duration::from_secs(10 * 60),
            sample_interval: Duration::from_secs(2),
            max_len: 0,
            last_push: None,
        };
        h.recompute_max_len();
        h
    }
}

impl TempHistory {
    pub fn configure(&mut self, window_minutes: u16, sample_secs: u16) {
        let window_minutes = window_minutes.max(1) as u64;
        let sample_secs = sample_secs.max(1) as u64;
        self.window = Duration::from_secs(window_minutes * 60);
        self.sample_interval = Duration::from_secs(sample_secs);
        self.recompute_max_len();
        self.prune(Instant::now());
    }

    fn recompute_max_len(&mut self) {
        let window_secs = self.window.as_secs().max(1);
        let sample_secs = self.sample_interval.as_secs().max(1);
        self.max_len = (window_secs / sample_secs) as usize + 8;
    }

    /// Push only if `sample_secs` elapsed since last push; then prune by window.
    pub fn push_if_due(&mut self, temp: f32, now: Instant) {
        if let Some(last) = self.last_push {
            if now.duration_since(last) < self.sample_interval {
                return;
            }
        }
        self.samples.push_back((now, temp));
        self.last_push = Some(now);
        self.prune(now);
    }

    fn prune(&mut self, now: Instant) {
        // Prefer relative to newest sample when present; fall back to `now`.
        let anchor = self.samples.back().map(|(t, _)| *t).unwrap_or(now);
        while let Some(&(t, _)) = self.samples.front() {
            if anchor.saturating_duration_since(t) > self.window {
                self.samples.pop_front();
            } else {
                break;
            }
        }
        while self.samples.len() > self.max_len {
            self.samples.pop_front();
        }
    }

    pub fn last(&self) -> Option<f32> {
        self.samples.back().map(|(_, t)| *t)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Frame-rate-independent exponential ease toward `target`, using a half-life
/// in seconds so the speed of approach doesn't depend on frame rate.
pub fn ease_toward(current: f32, target: f32, dt_secs: f32, half_life_secs: f32) -> f32 {
    let alpha = 1.0 - 0.5_f32.powf((dt_secs / half_life_secs.max(0.001)).max(0.0));
    current + (target - current) * alpha
}

/// How long the Y axis takes to settle onto a new max after the rolling
/// window prunes away a hot sample, instead of snapping instantly.
const AXIS_MAX_HALF_LIFE_SECS: f32 = 1.5;

/// Draw a glowing temperature graph in `ui`.
///
/// `window_minutes` is used for the X-axis labels (`-Nm` … `now`). `axis_max`
/// is smoothing state owned by the caller (shared across frames, and across
/// series once the graph becomes multi-series) so the Y axis eases toward a
/// new max instead of jumping in a single frame.
pub fn show_cpu_graph(
    ui: &mut egui::Ui,
    history: &TempHistory,
    title: &str,
    window_minutes: u16,
    axis_max: &mut Option<f32>,
) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(title);
            ui.weak(format!("({window_minutes}m)"));
            if let Some(t) = history.last() {
                let color = temp_color(t);
                ui.colored_label(color, format!("{t:.1} °C"));
            }
        });

        let height = ui.available_height().max(60.0);
        let (rect, _resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        if history.is_empty() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                t!("graph.loading").to_string(),
                egui::FontId::proportional(14.0),
                Color32::GRAY,
            );
            return;
        }

        let min_t = 20.0_f32;
        let target_max = history
            .samples
            .iter()
            .map(|(_, t)| *t)
            .fold(80.0_f32, f32::max)
            .max(50.0)
            + 5.0;
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.5);
        let max_t = ease_toward(
            *axis_max.get_or_insert(target_max),
            target_max,
            dt,
            AXIS_MAX_HALF_LIFE_SECS,
        );
        *axis_max = Some(max_t);

        let painter = ui.painter_at(rect);
        // Background
        painter.rect_filled(rect, 6.0, Color32::from_rgb(12, 14, 22));
        painter.rect_stroke(
            rect,
            6.0,
            Stroke::new(1.0_f32, Color32::from_rgb(40, 48, 70)),
            StrokeKind::Inside,
        );

        // Grid
        for i in 0..4 {
            let y = rect.top() + rect.height() * (i as f32) / 3.0;
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(60, 70, 100, 40)),
            );
        }

        let window = Duration::from_secs(u64::from(window_minutes.max(1)) * 60);
        let newest = history
            .samples
            .back()
            .map(|(t, _)| *t)
            .unwrap_or_else(Instant::now);

        let map_y = |t: f32| {
            let u = ((t - min_t) / (max_t - min_t)).clamp(0.0, 1.0);
            rect.bottom() - u * rect.height()
        };
        let map_x = |ts: Instant| {
            let age = newest.saturating_duration_since(ts).as_secs_f32();
            let span = window.as_secs_f32().max(1.0);
            // leftmost = oldest (full window age), rightmost = newest (age 0)
            let u = (1.0 - age / span).clamp(0.0, 1.0);
            rect.left() + u * rect.width()
        };

        let points: Vec<Pos2> = history
            .samples
            .iter()
            .map(|(ts, t)| Pos2::new(map_x(*ts), map_y(*t)))
            .collect();

        // Fill under curve
        if points.len() >= 2 {
            let mut fill = points.clone();
            fill.push(Pos2::new(points.last().unwrap().x, rect.bottom()));
            fill.push(Pos2::new(points[0].x, rect.bottom()));
            painter.add(egui::Shape::convex_polygon(
                fill,
                Color32::from_rgba_unmultiplied(0, 200, 255, 28),
                Stroke::new(0.0_f32, Color32::TRANSPARENT),
            ));
        }

        // Glow layers
        let last_t = history.last().unwrap_or(40.0);
        let base = temp_color(last_t);
        for (w, a) in [(10.0_f32, 20_u8), (6.0, 40), (3.0, 90)] {
            let stroke = Stroke::new(
                w,
                Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), a),
            );
            painter.add(egui::Shape::line(points.clone(), stroke));
        }
        // Core line
        painter.add(egui::Shape::line(
            points.clone(),
            Stroke::new(2.0_f32, Color32::from_rgb(220, 245, 255)),
        ));

        // Head pulse
        if let Some(p) = points.last() {
            // Wrap in f64 before casting to f32 (avoids losing phase precision after
            // long uptimes) and use a smooth sine wave instead of `.sin().abs()`
            // (which folds the wave and creates a direction-reversal cusp at every
            // zero-crossing).
            let time = ui.input(|i| i.time);
            let phase = (time * 4.0).rem_euclid(std::f64::consts::TAU) as f32;
            let r = 4.0 + (phase.sin() * 0.5 + 0.5) * 2.0;
            painter.circle_filled(
                *p,
                r + 4.0,
                Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 50),
            );
            painter.circle_filled(*p, r, base);
        }

        // Min/max temp labels
        painter.text(
            Pos2::new(rect.left() + 6.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            format!("{max_t:.0}°"),
            egui::FontId::monospace(10.0),
            Color32::GRAY,
        );
        painter.text(
            Pos2::new(rect.left() + 6.0, rect.bottom() - 28.0),
            egui::Align2::LEFT_TOP,
            format!("{min_t:.0}°"),
            egui::FontId::monospace(10.0),
            Color32::GRAY,
        );

        // Time axis labels: -Nm … now
        painter.text(
            Pos2::new(rect.left() + 6.0, rect.bottom() - 2.0),
            egui::Align2::LEFT_BOTTOM,
            format!("-{window_minutes}m"),
            egui::FontId::monospace(10.0),
            Color32::from_rgb(100, 110, 140),
        );
        painter.text(
            Pos2::new(rect.right() - 6.0, rect.bottom() - 2.0),
            egui::Align2::RIGHT_BOTTOM,
            t!("graph.now").to_string(),
            egui::FontId::monospace(10.0),
            Color32::from_rgb(100, 110, 140),
        );
    });
}

pub fn temp_color(t: f32) -> Color32 {
    if t < 50.0 {
        Color32::from_rgb(80, 220, 255)
    } else if t < 70.0 {
        Color32::from_rgb(120, 255, 160)
    } else if t < 85.0 {
        Color32::from_rgb(255, 200, 60)
    } else {
        Color32::from_rgb(255, 80, 80)
    }
}

/// Normalize a temperature to a 0..1 "how hot" scale, using the same bands as
/// `temp_color` (cool below 50C, hot at/above 85C).
pub fn temp_heat01(t: f32) -> f32 {
    ((t - 30.0) / (85.0 - 30.0)).clamp(0.0, 1.0)
}

/// Neutral GPU baseline used when no GPU temperature has ever been sampled
/// (no nvidia-smi, AMD/Intel not probed) so shader styles blending CPU/GPU
/// heat never look broken on machines without GPU temp support.
const GPU_FALLBACK_C: f32 = 40.0;

/// Per-frame "how hot is it" signal shared by the classic graph's coloring
/// and any active shader style's uniforms.
#[derive(Debug, Clone, Copy)]
pub struct ThermalSignal {
    pub cpu_c: f32,
    pub gpu_c: f32,
    pub gpu_present: bool,
    pub cpu01: f32,
    pub gpu01: f32,
    /// max(cpu01, gpu01) — "how worried should this look".
    pub heat01: f32,
}

impl ThermalSignal {
    pub fn from_histories(cpu: &TempHistory, gpu: &TempHistory) -> Self {
        let cpu_c = cpu.last().unwrap_or(40.0);
        let gpu_present = !gpu.is_empty();
        let gpu_c = gpu.last().unwrap_or(GPU_FALLBACK_C);
        let cpu01 = temp_heat01(cpu_c);
        let gpu01 = if gpu_present { temp_heat01(gpu_c) } else { 0.0 };
        Self {
            cpu_c,
            gpu_c,
            gpu_present,
            cpu01,
            gpu01,
            heat01: cpu01.max(gpu01),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ease_toward_zero_dt_is_a_no_op() {
        assert_eq!(ease_toward(50.0, 90.0, 0.0, 1.5), 50.0);
    }

    #[test]
    fn ease_toward_converges_to_target() {
        let mut v = 50.0_f32;
        for _ in 0..2000 {
            v = ease_toward(v, 90.0, 0.05, 1.5);
        }
        assert!((v - 90.0).abs() < 0.01, "expected convergence, got {v}");
    }

    #[test]
    fn ease_toward_is_monotonic_toward_target_from_above_and_below() {
        let up = ease_toward(50.0, 90.0, 0.1, 1.5);
        assert!(up > 50.0 && up < 90.0);
        let down = ease_toward(90.0, 50.0, 0.1, 1.5);
        assert!(down < 90.0 && down > 50.0);
    }

    #[test]
    fn phase_wrap_never_negative_or_nan_for_large_time() {
        for t in [0.0_f64, 1.0, 1000.0, 1_000_000.0, 86_400.0 * 30.0] {
            let phase = (t * 4.0).rem_euclid(std::f64::consts::TAU);
            assert!(phase.is_finite());
            assert!((0.0..std::f64::consts::TAU).contains(&phase));
        }
    }
}
