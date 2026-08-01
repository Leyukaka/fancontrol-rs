//! CPU temperature sparkline with glow fill and configurable time window.

use eframe::egui::{self, Color32};
use egui_plot::{Line, Plot};
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

    /// Timestamp of the newest sample, if any (shared multi-series X epoch).
    pub fn last_ts(&self) -> Option<Instant> {
        self.samples.back().map(|(t, _)| *t)
    }

    /// `[minutes-ago, value]` pairs for `egui_plot`, oldest first.
    ///
    /// X is anchored to `epoch` (usually the newest sample across series), **not**
    /// paint-frame `Instant::now()`. That keeps the grid frozen between samples;
    /// only a new sample scrolls the trace (Task Manager–style).
    pub fn plot_points_since(&self, epoch: Instant) -> Vec<[f64; 2]> {
        self.samples
            .iter()
            .map(|(ts, temp)| {
                let age_mins = epoch.saturating_duration_since(*ts).as_secs_f64() / 60.0;
                [-age_mins, f64::from(*temp)]
            })
            .collect()
    }

    /// Convenience: epoch = this history's newest sample.
    pub fn plot_points(&self) -> Vec<[f64; 2]> {
        match self.last_ts() {
            Some(t0) => self.plot_points_since(t0),
            None => Vec::new(),
        }
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

/// One plotted line on the multi-sensor graph.
pub struct GraphSeries<'a> {
    pub label: &'a str,
    /// Position in the user's ordered sensor selection — used both as the
    /// stable (restart-proof) color-palette index and to mark the "primary"
    /// series (index 0) that gets the full glow/fill/head-pulse treatment.
    pub palette_index: usize,
    pub history: &'a TempHistory,
    /// Unit for dual-axis grouping (`°C`, `W`, `%`, …). Defaults to °C when None.
    pub unit: Option<&'a str>,
}

impl GraphSeries<'_> {
    fn unit_key(&self) -> &str {
        self.unit.unwrap_or("°C")
    }
}

/// Categorical palette for identifying series by color once ≥2 sensors are
/// plotted (validated for pairwise contrast on this dark UI and colorblind
/// safety up to 8 concurrent series; cycles and relies on the legend text
/// beyond that — a deliberate tradeoff, not an oversight, since a 9th
/// generated/hashed hue would be less distinguishable, not more).
const SERIES_PALETTE: [Color32; 8] = [
    Color32::from_rgb(0x39, 0x87, 0xe5), // blue
    Color32::from_rgb(0xd9, 0x59, 0x26), // orange
    Color32::from_rgb(0x19, 0x9e, 0x70), // teal
    Color32::from_rgb(0xc9, 0x85, 0x00), // gold
    Color32::from_rgb(0xd5, 0x51, 0x81), // magenta
    Color32::from_rgb(0x00, 0x83, 0x00), // green
    Color32::from_rgb(0x90, 0x85, 0xe9), // violet
    Color32::from_rgb(0xe6, 0x67, 0x67), // red
];

pub fn series_color(palette_index: usize) -> Color32 {
    SERIES_PALETTE[palette_index % SERIES_PALETTE.len()]
}

/// Y-axis target max for power series (watts).
///
/// Prefers the GPU package **power limit** when known so the scale matches the
/// card (e.g. ~360 W on RTX 5080) instead of a huge empty range.
pub fn power_y_max(data_max: f32, power_limit_ceiling: Option<f32>) -> f32 {
    let data_driven = (data_max * 1.05).max(10.0);
    match power_limit_ceiling.filter(|c| c.is_finite() && *c > 1.0) {
        Some(limit) => {
            // Keep limit as the soft ceiling; if samples overshoot slightly, grow a little.
            let overshoot = if data_max > limit {
                data_max * 1.02
            } else {
                0.0
            };
            limit.max(overshoot).max(data_driven.min(limit * 1.05))
        }
        None => data_driven,
    }
}

/// Draw the multi-sensor graph for 0..N selected sensors in `ui`.
///
/// Series are grouped by **unit**. If two units are present (e.g. °C and W),
/// two stacked plots share the X window (dual-axis style without a third axis).
/// More than two units: first two unit groups only + a small note.
///
/// `axis_max_primary` / `axis_max_secondary` ease Y bounds for the two unit groups.
/// `power_axis_ceiling` (watts) is typically the GPU `power.limit` when available.
pub fn show_metric_graph(
    ui: &mut egui::Ui,
    series: &[GraphSeries<'_>],
    window_minutes: u16,
    axis_max_primary: &mut Option<f32>,
    axis_max_secondary: &mut Option<f32>,
    plot_height: f32,
    power_axis_ceiling: Option<f32>,
) {
    ui.group(|ui| {
        if series.is_empty() {
            ui.horizontal(|ui| {
                ui.heading(t!("graph.multi_sensor_title").to_string());
                ui.weak(format!("({window_minutes}m)"));
            });
            ui.colored_label(Color32::GRAY, t!("graph.no_sensors_selected").to_string());
            return;
        }

        // Unit order: prefer °C first, then appearance order.
        let mut unit_order: Vec<&str> = Vec::new();
        for s in series {
            let u = s.unit_key();
            if !unit_order.contains(&u) {
                unit_order.push(u);
            }
        }
        unit_order.sort_by(|a, b| {
            let rank = |u: &str| {
                if u == "°C" || u.eq_ignore_ascii_case("c") {
                    0
                } else {
                    1
                }
            };
            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
        });
        let too_many_units = unit_order.len() > 2;
        if too_many_units {
            unit_order.truncate(2);
            ui.small(t!("graph.too_many_units").to_string());
        }

        if series.len() <= 1 {
            ui.horizontal(|ui| {
                ui.heading(series.first().map(|s| s.label).unwrap_or(""));
                ui.weak(format!("({window_minutes}m)"));
                if let Some(s) = series.first() {
                    if let Some(t) = s.history.last() {
                        let u = s.unit_key();
                        let color = if u == "°C" {
                            temp_color(t)
                        } else {
                            series_color(0)
                        };
                        ui.colored_label(color, format!("{t:.1} {u}"));
                    }
                }
            });
        } else {
            ui.horizontal(|ui| {
                ui.heading(t!("graph.multi_sensor_title").to_string());
                ui.weak(format!("({window_minutes}m)"));
            });
            ui.horizontal_wrapped(|ui| {
                for s in series {
                    if !unit_order.iter().any(|u| *u == s.unit_key()) {
                        continue;
                    }
                    let color = series_color(s.palette_index);
                    let u = s.unit_key();
                    let val = s
                        .history
                        .last()
                        .map(|t| format!("{t:.1} {u}"))
                        .unwrap_or_else(|| "—".to_string());
                    ui.label(egui::RichText::new("●").color(color));
                    ui.label(egui::RichText::new(format!("{} {val}", s.label)).weak());
                    ui.add_space(10.0);
                }
            });
            if series.len() > 6 {
                ui.small(t!("graph.many_sensors_note").to_string());
            }
        }

        let n_plots = unit_order.len().max(1);
        let header = 40.0;
        let height_each =
            ((plot_height - header) / n_plots as f32).clamp(70.0, plot_height.max(70.0));

        if series.iter().all(|s| s.history.is_empty()) {
            ui.allocate_ui(egui::vec2(ui.available_width(), height_each), |ui| {
                ui.centered_and_justified(|ui| {
                    ui.colored_label(Color32::GRAY, t!("graph.loading").to_string());
                });
            });
            return;
        }

        let epoch = series
            .iter()
            .filter_map(|s| s.history.last_ts())
            .max()
            .unwrap_or_else(Instant::now);
        let window_mins = f64::from(window_minutes.max(1));
        let dt = ui.input(|i| i.stable_dt).clamp(0.0, 0.5);

        for (plot_i, unit) in unit_order.iter().enumerate() {
            let group: Vec<&GraphSeries<'_>> =
                series.iter().filter(|s| s.unit_key() == *unit).collect();
            if group.is_empty() {
                continue;
            }
            let axis_state = if plot_i == 0 {
                &mut *axis_max_primary
            } else {
                &mut *axis_max_secondary
            };
            draw_unit_plot(UnitPlotArgs {
                ui,
                group: &group,
                unit,
                window_mins,
                height: height_each,
                epoch,
                dt,
                axis_max: axis_state,
                plot_index: plot_i,
                single_series_mode: series.len() <= 1,
                power_axis_ceiling,
            });
        }
    });
}

struct UnitPlotArgs<'a, 'b> {
    ui: &'a mut egui::Ui,
    group: &'a [&'b GraphSeries<'b>],
    unit: &'a str,
    window_mins: f64,
    height: f32,
    epoch: Instant,
    dt: f32,
    axis_max: &'a mut Option<f32>,
    plot_index: usize,
    single_series_mode: bool,
    power_axis_ceiling: Option<f32>,
}

fn draw_unit_plot(args: UnitPlotArgs<'_, '_>) {
    let UnitPlotArgs {
        ui,
        group,
        unit,
        window_mins,
        height,
        epoch,
        dt,
        axis_max,
        plot_index,
        single_series_mode,
        power_axis_ceiling,
    } = args;
    let is_temp = unit == "°C" || unit.eq_ignore_ascii_case("c");
    let is_power = unit == "W" || unit.eq_ignore_ascii_case("w");
    let (min_y, default_max) = if is_temp {
        (20.0_f32, 80.0_f32)
    } else if unit == "%" {
        (0.0, 100.0)
    } else if is_power {
        (0.0, 50.0)
    } else {
        (0.0, 10.0)
    };
    let data_max = group
        .iter()
        .flat_map(|s| s.history.samples.iter().map(|(_, t)| *t))
        .fold(default_max * 0.5, f32::max);
    let target_max = if is_temp {
        data_max.max(50.0) + 5.0
    } else if unit == "%" {
        100.0
    } else if is_power {
        power_y_max(data_max, power_axis_ceiling)
    } else {
        (data_max * 1.1).max(1.0)
    };
    let max_y = ease_toward(
        *axis_max.get_or_insert(target_max),
        target_max,
        dt,
        AXIS_MAX_HALF_LIFE_SECS,
    );
    *axis_max = Some(max_y);

    let unit_owned = unit.to_string();
    let plot_id = format!("metric_graph_{plot_index}_{unit}");
    Plot::new(plot_id)
        .height(height)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .include_x(-window_mins)
        .include_x(0.0)
        .include_y(f64::from(min_y))
        .include_y(f64::from(max_y))
        .x_axis_formatter(|mark, _range| format!("{:.0}m", mark.value))
        .y_axis_formatter({
            let u = unit_owned.clone();
            move |mark, _range| format!("{:.0}{u}", mark.value)
        })
        .label_formatter({
            let u = unit_owned.clone();
            move |hover| match hover {
                egui_plot::HoverPosition::NearDataPoint {
                    plot_name,
                    position,
                    ..
                } => Some(format!("{plot_name}\n{:.1} {u}", position.y)),
                egui_plot::HoverPosition::Elsewhere { .. } => None,
            }
        })
        .show(ui, |plot_ui| {
            for s in group {
                if s.history.is_empty() {
                    continue;
                }
                let color = if single_series_mode && is_temp {
                    temp_color(s.history.last().unwrap_or(40.0))
                } else {
                    series_color(s.palette_index)
                };
                let mut line = Line::new(s.label, s.history.plot_points_since(epoch))
                    .color(color)
                    .width(2.0_f32);
                if s.palette_index == 0 && is_temp {
                    line = line.fill(min_y);
                }
                plot_ui.line(line);
            }
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

/// Neutral baseline used when no sensor is selected/live, so a shader style
/// never looks broken (uninitialized-looking) with nothing to read from.
const NEUTRAL_FALLBACK_C: f32 = 40.0;

/// Per-frame "how hot is it" signal shared by any active shader style's
/// uniforms and its on-panel temperature readout, built from whichever
/// sensors are currently selected and live (not hardcoded to CPU/GPU).
#[derive(Debug, Clone)]
pub struct ThermalSignal {
    /// (label, celsius, heat01) per currently-selected, currently-live sensor.
    pub readings: Vec<(String, f32, f32)>,
    /// max(heat01) across readings — the single scalar that can drive a shader.
    pub heat01_max: f32,
}

impl ThermalSignal {
    pub fn from_readings(readings: Vec<(String, f32)>) -> Self {
        if readings.is_empty() {
            let heat01 = temp_heat01(NEUTRAL_FALLBACK_C);
            return Self {
                readings: vec![("—".to_string(), NEUTRAL_FALLBACK_C, heat01)],
                heat01_max: heat01,
            };
        }
        let readings: Vec<(String, f32, f32)> = readings
            .into_iter()
            .map(|(label, c)| (label, c, temp_heat01(c)))
            .collect();
        let heat01_max = readings.iter().map(|(_, _, h)| *h).fold(0.0_f32, f32::max);
        Self {
            readings,
            heat01_max,
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
    fn power_y_max_uses_gpu_limit_not_huge_empty_scale() {
        // Idle ~40 W on a 360 W card → axis near limit, not 1000.
        let y = power_y_max(40.0, Some(360.0));
        assert!((360.0..=400.0).contains(&y), "got {y}");
        // No limit: data-driven.
        let y2 = power_y_max(40.0, None);
        assert!((y2 - 42.0).abs() < 1.0, "got {y2}");
        // Limit ignored when garbage.
        let y3 = power_y_max(50.0, Some(0.0));
        assert!(y3 < 100.0, "got {y3}");
    }

    #[test]
    fn plot_points_places_a_single_sample_at_x_zero() {
        let now = Instant::now();
        let mut h = TempHistory::default();
        h.push_if_due(55.0, now);
        let pts = h.plot_points();
        assert_eq!(pts.len(), 1);
        assert_eq!(pts[0], [0.0, 55.0]);
    }

    #[test]
    fn plot_points_stay_stable_when_wall_clock_advances() {
        let t0 = Instant::now();
        let mut h = TempHistory::default();
        h.configure(10, 1);
        // Force two samples at known stamps via push_if_due with spaced times.
        h.push_if_due(40.0, t0);
        // Bypass interval by using a later Instant directly on a second history path:
        // push_if_due respects sample_interval — configure 1s then sleep is flaky in tests.
        // Instead stamp via two histories' public API: only last sample at x=0 matters.
        let pts_a = h.plot_points();
        let pts_b = h.plot_points_since(t0 + Duration::from_secs(30));
        // Same data anchored to different epochs → newest (only) sample moves relative to
        // a future epoch; when anchored to own last_ts, still [0, y].
        assert_eq!(pts_a[0], [0.0, 40.0]);
        // Relative to a later epoch, the single sample is in the past (negative x).
        assert!(
            pts_b[0][0] < -0.4,
            "expected left of 0, got {}",
            pts_b[0][0]
        );
        // Re-anchor to last_ts again: back to stable zero (no paint-clock crawl).
        assert_eq!(h.plot_points()[0], [0.0, 40.0]);
    }

    #[test]
    fn series_color_distinct_within_palette_and_wraps() {
        let colors: Vec<Color32> = (0..8).map(series_color).collect();
        for i in 0..colors.len() {
            for j in (i + 1)..colors.len() {
                assert_ne!(colors[i], colors[j], "colors {i} and {j} should differ");
            }
        }
        assert_eq!(series_color(8), series_color(0));
        assert_eq!(series_color(9), series_color(1));
    }

    #[test]
    fn thermal_signal_empty_readings_falls_back_to_neutral() {
        let signal = ThermalSignal::from_readings(Vec::new());
        assert_eq!(signal.readings.len(), 1);
        assert!(signal.heat01_max.is_finite());
    }

    #[test]
    fn thermal_signal_heat01_max_is_the_max_across_readings() {
        let signal = ThermalSignal::from_readings(vec![
            ("A".to_string(), 40.0),
            ("B".to_string(), 90.0),
            ("C".to_string(), 60.0),
        ]);
        assert_eq!(signal.readings.len(), 3);
        assert!((signal.heat01_max - temp_heat01(90.0)).abs() < f32::EPSILON);
    }
}
