//! CPU temperature sparkline with glow fill and configurable time window.

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
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
        let anchor = self
            .samples
            .back()
            .map(|(t, _)| *t)
            .unwrap_or(now);
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

/// Draw a glowing temperature graph in `ui`.
///
/// `window_minutes` is used for the X-axis labels (`-Nm` … `now`).
pub fn show_cpu_graph(ui: &mut egui::Ui, history: &TempHistory, title: &str, window_minutes: u16) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(title);
            ui.weak(format!("({window_minutes}m)"));
            if let Some(t) = history.last() {
                let color = temp_color(t);
                ui.colored_label(color, format!("{t:.1} °C"));
            }
        });

        let height = 140.0;
        let (rect, _resp) =
            ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        if history.is_empty() {
            ui.painter().text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                "…",
                egui::FontId::proportional(14.0),
                Color32::GRAY,
            );
            return;
        }

        let min_t = 20.0_f32;
        let max_t = history
            .samples
            .iter()
            .map(|(_, t)| *t)
            .fold(80.0_f32, f32::max)
            .max(50.0)
            + 5.0;

        let painter = ui.painter_at(rect);
        // Background
        painter.rect_filled(rect, 6.0, Color32::from_rgb(12, 14, 22));
        painter.rect_stroke(rect, 6.0, Stroke::new(1.0, Color32::from_rgb(40, 48, 70)));

        // Grid
        for i in 0..4 {
            let y = rect.top() + rect.height() * (i as f32) / 3.0;
            painter.line_segment(
                [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
                Stroke::new(1.0, Color32::from_rgba_unmultiplied(60, 70, 100, 40)),
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
                Stroke::new(0.0, Color32::TRANSPARENT),
            ));
        }

        // Glow layers
        let last_t = history.last().unwrap_or(40.0);
        let base = temp_color(last_t);
        for (w, a) in [(10.0, 20), (6.0, 40), (3.0, 90)] {
            let stroke = Stroke::new(
                w,
                Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), a),
            );
            painter.add(egui::Shape::line(points.clone(), stroke));
        }
        // Core line
        painter.add(egui::Shape::line(
            points.clone(),
            Stroke::new(2.0, Color32::from_rgb(220, 245, 255)),
        ));

        // Head pulse
        if let Some(p) = points.last() {
            let r = 4.0 + (ui.input(|i| i.time) as f32 * 4.0).sin().abs() * 2.0;
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
            Pos2::new(rect.left() + 6.0, rect.bottom() - 14.0),
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
            "now",
            egui::FontId::monospace(10.0),
            Color32::from_rgb(100, 110, 140),
        );
    });
}

fn temp_color(t: f32) -> Color32 {
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
