//! CPU detail panel: package power vs. limit, temperature, load % (read-only host metrics).

use crate::graph::TempHistory;
use crate::panel_metrics::{
    load_chip, load_color, metric_bar, power_color, power_sparkline, temp_chip,
};
use crate::poll::CpuSnap;
use eframe::egui::{self, Color32, RichText};

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
        temp_chip(ui, t!("cpu.temp").to_string(), cpu.temp_c, true);
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

    // GPU-style load row + bar, denser parity with the GPU panel's utilization row
    // (the load chip above is a compact readout; this makes the fraction visible).
    if let Some(l) = cpu.load_pct {
        let frac = (l / 100.0) as f32;
        ui.horizontal(|ui| {
            ui.label(t!("cpu.load").to_string());
            ui.label(
                RichText::new(format!("{l:.0}%"))
                    .monospace()
                    .color(load_color(frac)),
            );
        });
        metric_bar(ui, frac, load_color(frac));
    }

    if let Some(history) = power_history
        && !history.is_empty()
    {
        ui.add_space(6.0);
        ui.small(t!("cpu.power_history").to_string());
        power_sparkline(
            ui,
            "cpu_power_sparkline",
            history,
            cpu.power_limit_w.map(|w| w as f32),
        );
    }
}
