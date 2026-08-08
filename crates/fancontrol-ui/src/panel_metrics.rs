//! Shared metric widgets (bar, temp/load chip, color ramps, power sparkline)
//! for the GPU and CPU detail panels, so both stay visually consistent.

use crate::graph::{TempHistory, power_y_max};
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot};

/// Thin horizontal fill bar (used under a metric row: power, load, VRAM, fan…).
pub fn metric_bar(ui: &mut egui::Ui, frac: f32, color: Color32) {
    let frac = frac.clamp(0.0, 1.0);
    let desired = egui::vec2(ui.available_width().clamp(80.0, 280.0), 8.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, Color32::from_gray(40));
    if frac > 0.0 {
        let mut fill = rect;
        fill.set_width(rect.width() * frac);
        painter.rect_filled(fill, 2.0, color);
    }
}

/// Small boxed temperature readout (`"—"` when unavailable).
pub fn temp_chip(ui: &mut egui::Ui, label: String, value: Option<f64>, colorize: bool) {
    ui.group(|ui| {
        ui.set_min_width(72.0);
        ui.vertical(|ui| {
            ui.small(label);
            match value {
                Some(t) => {
                    let c = if colorize {
                        temp_color(t as f32)
                    } else {
                        Color32::LIGHT_GRAY
                    };
                    ui.label(
                        RichText::new(format!("{t:.0}°C"))
                            .monospace()
                            .strong()
                            .size(18.0)
                            .color(c),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("—")
                            .monospace()
                            .strong()
                            .size(18.0)
                            .color(Color32::DARK_GRAY),
                    );
                }
            }
        });
    });
}

/// Small boxed percentage readout (`"—"` when unavailable), colorized like a load bar.
pub fn load_chip(ui: &mut egui::Ui, label: String, value: Option<f64>) {
    ui.group(|ui| {
        ui.set_min_width(72.0);
        ui.vertical(|ui| {
            ui.small(label);
            match value {
                Some(p) => {
                    ui.label(
                        RichText::new(format!("{p:.0}%"))
                            .monospace()
                            .strong()
                            .size(18.0)
                            .color(load_color(p as f32 / 100.0)),
                    );
                }
                None => {
                    ui.label(
                        RichText::new("—")
                            .monospace()
                            .strong()
                            .size(18.0)
                            .color(Color32::DARK_GRAY),
                    );
                }
            }
        });
    });
}

pub fn temp_color(c: f32) -> Color32 {
    if c < 50.0 {
        Color32::from_rgb(80, 200, 120)
    } else if c < 70.0 {
        Color32::from_rgb(220, 200, 80)
    } else if c < 85.0 {
        Color32::from_rgb(230, 140, 60)
    } else {
        Color32::from_rgb(230, 80, 80)
    }
}

pub fn load_color(frac: f32) -> Color32 {
    if frac < 0.5 {
        Color32::from_rgb(80, 200, 120)
    } else if frac < 0.8 {
        Color32::from_rgb(220, 180, 60)
    } else {
        Color32::from_rgb(230, 90, 70)
    }
}

pub fn power_color(frac: f32) -> Color32 {
    if frac < 0.4 {
        Color32::from_rgb(100, 180, 255)
    } else if frac < 0.75 {
        Color32::from_rgb(220, 180, 60)
    } else {
        Color32::from_rgb(230, 100, 80)
    }
}

/// Recent power-draw sparkline shared by the CPU and GPU panels: real axes
/// (not hidden), a grid, watts on Y, minutes-ago on X, a zero floor, and a
/// soft ceiling from the reported power limit when known.
pub fn power_sparkline(
    ui: &mut egui::Ui,
    id_salt: &str,
    history: &TempHistory,
    limit_w: Option<f32>,
) {
    let points = history.plot_points();
    let data_max = points.iter().map(|p| p[1] as f32).fold(0.0_f32, f32::max);
    let max_y = power_y_max(data_max, limit_w);
    let window_mins = history.window_minutes();
    let color = power_color(0.3);
    let line = Line::new(id_salt.to_string(), points)
        .color(color)
        .width(2.0);
    Plot::new(id_salt)
        .height(60.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .allow_boxed_zoom(false)
        .show_axes([true, true])
        .show_grid([true, true])
        .include_x(-window_mins)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(f64::from(max_y))
        .x_axis_formatter(|mark, _range| format!("{:.0}m", mark.value))
        .y_axis_formatter(|mark, _range| format!("{:.0}W", mark.value))
        .show(ui, |plot_ui| {
            plot_ui.line(line);
        });
}
