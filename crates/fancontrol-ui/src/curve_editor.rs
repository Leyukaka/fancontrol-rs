//! Interactive fan curve editor (temp °C → duty %).

use eframe::egui::{self, Color32, Pos2, Sense, Stroke, Vec2};
use fancontrol_core::{CurvePoint, FanCurve};

const TEMP_MIN: f32 = 20.0;
const TEMP_MAX: f32 = 100.0;

/// Draw / edit curve points. Returns true if curve was modified.
pub fn show_curve_editor(ui: &mut egui::Ui, curve: &mut FanCurve, live_temp: Option<f64>) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.heading(&curve.name);
        ui.small(format!("id={}", curve.id.as_str()));
        if ui.button("+ point").clicked() {
            curve.points.push(CurvePoint::new(55.0, 50));
            curve.sort_points();
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.label("Hysteresis °C");
        let mut h = curve.hysteresis_c as f32;
        if ui
            .add(egui::DragValue::new(&mut h).range(0.0..=15.0).speed(0.1))
            .changed()
        {
            curve.hysteresis_c = f64::from(h);
            changed = true;
        }
    });

    let height = 200.0;
    let (rect, resp) = ui.allocate_exact_size(
        Vec2::new(ui.available_width().max(200.0), height),
        Sense::click_and_drag(),
    );
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 6.0, Color32::from_rgb(14, 16, 26));
    painter.rect_stroke(
        rect,
        6.0,
        Stroke::new(1.0_f32, Color32::from_rgb(50, 60, 90)),
    );

    // Grid
    for i in 0..=4 {
        let x = rect.left() + rect.width() * (i as f32) / 4.0;
        let y = rect.top() + rect.height() * (i as f32) / 4.0;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(70, 80, 110, 50)),
        );
        painter.line_segment(
            [Pos2::new(rect.left(), y), Pos2::new(rect.right(), y)],
            Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(70, 80, 110, 50)),
        );
    }

    let to_pos = |t: f32, d: f32| {
        let u = ((t - TEMP_MIN) / (TEMP_MAX - TEMP_MIN)).clamp(0.0, 1.0);
        let v = (d / 100.0).clamp(0.0, 1.0);
        Pos2::new(
            rect.left() + u * rect.width(),
            rect.bottom() - v * rect.height(),
        )
    };
    let from_pos = |p: Pos2| {
        let u = ((p.x - rect.left()) / rect.width()).clamp(0.0, 1.0);
        let v = ((rect.bottom() - p.y) / rect.height()).clamp(0.0, 1.0);
        (
            TEMP_MIN + u * (TEMP_MAX - TEMP_MIN),
            (v * 100.0).round().clamp(0.0, 100.0) as u8,
        )
    };

    // Curve polyline
    if curve.points.len() >= 2 {
        let pts: Vec<Pos2> = curve
            .points
            .iter()
            .map(|p| to_pos(p.temperature as f32, f32::from(p.duty)))
            .collect();
        // glow
        painter.add(egui::Shape::line(
            pts.clone(),
            Stroke::new(6.0_f32, Color32::from_rgba_unmultiplied(80, 200, 255, 40)),
        ));
        painter.add(egui::Shape::line(
            pts,
            Stroke::new(2.0_f32, Color32::from_rgb(120, 220, 255)),
        ));
    }

    // Live temp marker
    if let Some(t) = live_temp {
        let x = to_pos(t as f32, 0.0).x;
        painter.line_segment(
            [Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom())],
            Stroke::new(1.5_f32, Color32::from_rgb(255, 180, 60)),
        );
        painter.text(
            Pos2::new(x + 4.0, rect.top() + 4.0),
            egui::Align2::LEFT_TOP,
            format!("{t:.0}°C"),
            egui::FontId::monospace(11.0),
            Color32::from_rgb(255, 200, 100),
        );
    }

    // Handles
    let pointer = resp.interact_pointer_pos();
    let mut drag_idx: Option<usize> = None;
    // Find which point is under pointer / being dragged via id memory
    let id = ui.id().with("curve_drag");
    let mut state = ui
        .ctx()
        .data(|d| d.get_temp::<Option<usize>>(id))
        .unwrap_or(None);

    if resp.drag_started() {
        if let Some(pos) = pointer {
            let mut best = None;
            let mut best_d = 18.0_f32;
            for (i, p) in curve.points.iter().enumerate() {
                let hp = to_pos(p.temperature as f32, f32::from(p.duty));
                let d = hp.distance(pos);
                if d < best_d {
                    best_d = d;
                    best = Some(i);
                }
            }
            state = best;
        }
    }
    if resp.dragged() {
        drag_idx = state;
    }
    if resp.drag_stopped() {
        state = None;
        curve.sort_points();
        changed = true;
    }
    ui.ctx().data_mut(|d| d.insert_temp(id, state));

    if let (Some(i), Some(pos)) = (drag_idx, pointer) {
        if let Some(p) = curve.points.get_mut(i) {
            let (t, d) = from_pos(pos);
            p.temperature = f64::from(t);
            p.duty = d;
            changed = true;
        }
    }

    for (i, p) in curve.points.iter().enumerate() {
        let hp = to_pos(p.temperature as f32, f32::from(p.duty));
        let active = state == Some(i);
        painter.circle_filled(
            hp,
            if active { 7.0 } else { 5.0 },
            if active {
                Color32::from_rgb(255, 220, 80)
            } else {
                Color32::from_rgb(100, 210, 255)
            },
        );
        painter.circle_stroke(
            hp,
            if active { 7.0 } else { 5.0 },
            Stroke::new(1.0_f32, Color32::WHITE),
        );
    }

    // Axis labels
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.bottom() - 14.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{TEMP_MIN:.0}°C"),
        egui::FontId::monospace(10.0),
        Color32::GRAY,
    );
    painter.text(
        Pos2::new(rect.right() - 4.0, rect.bottom() - 14.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("{TEMP_MAX:.0}°C"),
        egui::FontId::monospace(10.0),
        Color32::GRAY,
    );
    painter.text(
        Pos2::new(rect.left() + 4.0, rect.top() + 4.0),
        egui::Align2::LEFT_TOP,
        "100%",
        egui::FontId::monospace(10.0),
        Color32::GRAY,
    );

    // Point list
    ui.separator();
    let mut remove: Option<usize> = None;
    let n_pts = curve.points.len();
    for i in 0..n_pts {
        let mut t = curve.points[i].temperature as f32;
        let mut d = f32::from(curve.points[i].duty);
        ui.horizontal(|ui| {
            ui.label(format!("#{i}"));
            if ui
                .add(
                    egui::DragValue::new(&mut t)
                        .prefix("T ")
                        .suffix("°C")
                        .range(0.0..=120.0),
                )
                .changed()
            {
                curve.points[i].temperature = f64::from(t);
                changed = true;
            }
            if ui
                .add(
                    egui::DragValue::new(&mut d)
                        .prefix("D ")
                        .suffix("%")
                        .range(0.0..=100.0),
                )
                .changed()
            {
                curve.points[i].duty = d.round() as u8;
                changed = true;
            }
            if n_pts > 1 && ui.small_button("x").clicked() {
                remove = Some(i);
            }
        });
    }
    if let Some(i) = remove {
        curve.points.remove(i);
        changed = true;
    }
    if changed {
        curve.sort_points();
    }
    changed
}
