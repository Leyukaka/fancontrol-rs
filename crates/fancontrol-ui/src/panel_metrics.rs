//! Shared metric widgets for GPU and CPU detail panels (aligned layout).

use crate::graph::{TempHistory, power_y_max};
use eframe::egui::{self, Color32, Frame, Margin, RichText, Stroke};
use egui_plot::{Line, Plot};

/// Fixed sparkline height so GPU and CPU power graphs align.
pub const POWER_SPARKLINE_HEIGHT: f32 = 72.0;

/// Card frame around a domain panel (GPU / CPU) so headers and plots share padding.
pub fn domain_card(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui)) {
    Frame::group(ui.style())
        .inner_margin(Margin::same(8))
        .stroke(Stroke::new(
            1.0,
            ui.visuals().widgets.noninteractive.bg_stroke.color,
        ))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            add_contents(ui);
        });
}

/// Thin horizontal fill bar under a metric row.
pub fn metric_bar(ui: &mut egui::Ui, frac: f32, color: Color32) {
    let frac = frac.clamp(0.0, 1.0);
    let desired = egui::vec2(ui.available_width().max(40.0), 8.0);
    let (rect, _) = ui.allocate_exact_size(desired, egui::Sense::hover());
    let painter = ui.painter();
    painter.rect_filled(rect, 2.0, Color32::from_gray(40));
    if frac > 0.0 {
        let mut fill = rect;
        fill.set_width(rect.width() * frac);
        painter.rect_filled(fill, 2.0, color);
    }
}

/// Boxed temperature readout (`"—"` when unavailable).
pub fn temp_chip(ui: &mut egui::Ui, label: String, value: Option<f64>, colorize: bool) {
    ui.group(|ui| {
        ui.set_min_width(72.0);
        ui.set_min_height(48.0);
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

/// Boxed percentage readout.
pub fn load_chip(ui: &mut egui::Ui, label: String, value: Option<f64>) {
    ui.group(|ui| {
        ui.set_min_width(72.0);
        ui.set_min_height(48.0);
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

/// Empty chip slot to keep a 3-chip row aligned across panels.
pub fn empty_chip(ui: &mut egui::Ui, label: String) {
    temp_chip(ui, label, None, false);
}

/// Power row + bar. Always allocates bar space (grey if no value) so layout height is stable.
pub fn power_metric_row(
    ui: &mut egui::Ui,
    label: &str,
    power_w: Option<f64>,
    limit_w: Option<f64>,
    no_limit_hint: Option<&str>,
) {
    let draw = power_w.unwrap_or(0.0);
    let limit = limit_w.unwrap_or(0.0).max(1.0);
    let frac = if power_w.is_some() && limit_w.is_some() {
        (draw / limit).clamp(0.0, 1.5) as f32
    } else {
        0.0
    };
    let text = match (power_w, limit_w) {
        (Some(d), Some(l)) => format!("{d:.0} / {l:.0} W"),
        (Some(d), None) => format!("{d:.0} W"),
        (None, Some(l)) => format!("— / {l:.0} W"),
        (None, None) => "—".into(),
    };
    let color = if power_w.is_some() {
        power_color(frac)
    } else {
        Color32::DARK_GRAY
    };
    ui.horizontal(|ui| {
        ui.label(label.to_string());
        ui.label(RichText::new(text).monospace().strong().color(color));
    });
    if power_w.is_some() && limit_w.is_some() {
        metric_bar(ui, frac.min(1.0), power_color(frac));
    } else if power_w.is_some() {
        // Keep bar slot height even without limit.
        metric_bar(ui, 0.0, Color32::from_gray(40));
        if let Some(hint) = no_limit_hint {
            ui.small(hint.to_string());
        }
    } else {
        metric_bar(ui, 0.0, Color32::from_gray(40));
    }
}

/// Power history block: fixed height plot so GPU/CPU graphs align.
pub fn power_history_block(
    ui: &mut egui::Ui,
    title: &str,
    id_salt: &str,
    history: Option<&TempHistory>,
    limit_w: Option<f32>,
) {
    ui.add_space(6.0);
    ui.small(title.to_string());
    if let Some(history) = history
        && !history.is_empty()
    {
        power_sparkline(ui, id_salt, history, limit_w);
    } else {
        // Reserve the same vertical space as the sparkline.
        let (rect, _) = ui.allocate_exact_size(
            egui::vec2(ui.available_width(), POWER_SPARKLINE_HEIGHT),
            egui::Sense::hover(),
        );
        ui.painter().rect_filled(rect, 2.0, Color32::from_gray(25));
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "—",
            egui::FontId::proportional(12.0),
            Color32::DARK_GRAY,
        );
    }
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

/// Power sparkline: fixed height, axes, grid, W / minutes-ago.
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
        .height(POWER_SPARKLINE_HEIGHT)
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
