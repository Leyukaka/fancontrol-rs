//! CPU detail panel: package power vs. limit, temperature, load % (read-only host metrics).

use crate::graph::TempHistory;
use crate::poll::CpuSnap;
use eframe::egui::{self, Color32, RichText};
use egui_plot::{Line, Plot};

/// Draw the CPU detail card into `ui`. `power_history` is an optional recent
/// package-power trace (independent of the Sensors graph selection) for a
/// small sparkline; omitted entirely when there is nothing to plot yet.
pub fn show_cpu_panel(ui: &mut egui::Ui, cpu: &CpuSnap, power_history: Option<&TempHistory>) {
    ui.horizontal(|ui| {
        ui.heading(t!("cpu.heading").to_string());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.small(t!("cpu.read_only_note").to_string());
        });
    });

    if cpu.temp_c.is_none() && cpu.power_w.is_none() {
        ui.colored_label(Color32::GRAY, t!("cpu.none").to_string());
        ui.small(t!("cpu.none_hint").to_string());
        return;
    }

    ui.add_space(4.0);
    ui.horizontal(|ui| {
        temp_chip(ui, t!("cpu.temp").to_string(), cpu.temp_c);
        load_chip(ui, t!("cpu.load").to_string(), cpu.load_pct);
    });

    ui.add_space(6.0);

    if cpu.power_w.is_some() || cpu.power_limit_w.is_some() {
        let draw = cpu.power_w.unwrap_or(0.0);
        let limit = cpu.power_limit_w.unwrap_or(0.0).max(1.0);
        let frac = if cpu.power_limit_w.is_some() {
            (draw / limit).clamp(0.0, 1.5) as f32
        } else {
            0.0
        };
        let text = match (cpu.power_w, cpu.power_limit_w) {
            (Some(d), Some(l)) => format!("{d:.0} / {l:.0} W"),
            (Some(d), None) => format!("{d:.0} W"),
            (None, Some(l)) => format!("— / {l:.0} W"),
            (None, None) => "—".into(),
        };
        ui.horizontal(|ui| {
            ui.label(t!("cpu.power").to_string());
            ui.label(
                RichText::new(text)
                    .monospace()
                    .strong()
                    .color(power_color(frac)),
            );
        });
        if cpu.power_w.is_some() && cpu.power_limit_w.is_some() {
            metric_bar(ui, frac.min(1.0), power_color(frac));
        } else if cpu.power_w.is_some() {
            ui.small(t!("cpu.power_limit_unavailable").to_string());
        }
    }

    if let Some(history) = power_history
        && !history.is_empty()
    {
        ui.add_space(6.0);
        ui.small(t!("cpu.power_history").to_string());
        let points = history.plot_points();
        let color = power_color(0.3);
        let line = Line::new("cpu_power", points).color(color).width(2.0);
        Plot::new("cpu_power_sparkline")
            .height(60.0)
            .allow_drag(false)
            .allow_zoom(false)
            .allow_scroll(false)
            .allow_boxed_zoom(false)
            .show_axes([false, true])
            .show_grid([false, true])
            .include_y(0.0)
            .show(ui, |plot_ui| {
                plot_ui.line(line);
            });
    }
}

fn temp_chip(ui: &mut egui::Ui, label: String, value: Option<f64>) {
    ui.group(|ui| {
        ui.set_min_width(72.0);
        ui.vertical(|ui| {
            ui.small(label);
            match value {
                Some(t) => {
                    ui.label(
                        RichText::new(format!("{t:.0}°C"))
                            .monospace()
                            .strong()
                            .size(18.0)
                            .color(temp_color(t as f32)),
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

fn load_chip(ui: &mut egui::Ui, label: String, value: Option<f64>) {
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

fn metric_bar(ui: &mut egui::Ui, frac: f32, color: Color32) {
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

fn temp_color(c: f32) -> Color32 {
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

fn load_color(frac: f32) -> Color32 {
    if frac < 0.5 {
        Color32::from_rgb(80, 200, 120)
    } else if frac < 0.8 {
        Color32::from_rgb(220, 180, 60)
    } else {
        Color32::from_rgb(230, 90, 70)
    }
}

fn power_color(frac: f32) -> Color32 {
    if frac < 0.4 {
        Color32::from_rgb(100, 180, 255)
    } else if frac < 0.75 {
        Color32::from_rgb(220, 180, 60)
    } else {
        Color32::from_rgb(230, 100, 80)
    }
}
