//! CPU temperature sparkline with glow fill.

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use std::collections::VecDeque;

const HISTORY: usize = 180; // ~ samples at poll rate

#[derive(Debug, Clone)]
pub struct TempHistory {
    samples: VecDeque<f32>,
    max_len: usize,
}

impl Default for TempHistory {
    fn default() -> Self {
        Self {
            samples: VecDeque::with_capacity(HISTORY),
            max_len: HISTORY,
        }
    }
}

impl TempHistory {
    pub fn push(&mut self, t: f32) {
        if self.samples.len() >= self.max_len {
            self.samples.pop_front();
        }
        self.samples.push_back(t);
    }

    pub fn last(&self) -> Option<f32> {
        self.samples.back().copied()
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

/// Draw a glowing temperature graph in `ui`.
pub fn show_cpu_graph(ui: &mut egui::Ui, history: &TempHistory, title: &str) {
    ui.group(|ui| {
        ui.horizontal(|ui| {
            ui.heading(title);
            if let Some(t) = history.last() {
                let color = temp_color(t);
                ui.colored_label(color, format!("{t:.1} °C"));
            }
        });

        let height = 140.0;
        let (rect, _resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), height), Sense::hover());
        if history.is_empty() {
            ui.painter()
                .text(rect.center(), egui::Align2::CENTER_CENTER, "…", egui::FontId::proportional(14.0), Color32::GRAY);
            return;
        }

        let min_t = 20.0_f32;
        let max_t = history
            .samples
            .iter()
            .copied()
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

        let n = history.samples.len().max(2) as f32;
        let map_y = |t: f32| {
            let u = ((t - min_t) / (max_t - min_t)).clamp(0.0, 1.0);
            rect.bottom() - u * rect.height()
        };
        let map_x = |i: usize| rect.left() + (i as f32) / (n - 1.0) * rect.width();

        let points: Vec<Pos2> = history
            .samples
            .iter()
            .enumerate()
            .map(|(i, t)| Pos2::new(map_x(i), map_y(*t)))
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
            painter.circle_filled(*p, r + 4.0, Color32::from_rgba_unmultiplied(base.r(), base.g(), base.b(), 50));
            painter.circle_filled(*p, r, base);
        }

        // Min/max labels
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
