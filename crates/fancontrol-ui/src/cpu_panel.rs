//! CPU detail panel: package power vs. limit, temperature, load % (aligned with GPU card).

use crate::graph::TempHistory;
use crate::panel_metrics::{
    domain_card, empty_chip, load_chip, power_history_block, power_metric_row, temp_chip,
};
use crate::poll::CpuSnap;
use eframe::egui::{self, Color32, RichText};

/// Draw the CPU detail card. Layout mirrors [`crate::gpu_panel`]: chips → power → history.
pub fn show_cpu_panel(ui: &mut egui::Ui, cpu: &CpuSnap, power_history: Option<&TempHistory>) {
    domain_card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(t!("cpu.heading").to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(t!("cpu.read_only_note").to_string());
            });
        });

        if cpu.temp_c.is_none() && cpu.power_w.is_none() && cpu.load_pct.is_none() {
            ui.colored_label(Color32::GRAY, t!("cpu.none").to_string());
            ui.small(t!("cpu.none_hint").to_string());
            return;
        }

        // Subtitle row (GPU has device name here).
        ui.label(
            RichText::new(t!("cpu.package_subtitle").to_string())
                .strong()
                .size(16.0),
        );

        // Chip row: Temp | Load | empty (3 slots like GPU Core|Hot Spot|Memory).
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            temp_chip(ui, t!("cpu.temp").to_string(), cpu.temp_c, true);
            load_chip(ui, t!("cpu.load").to_string(), cpu.load_pct);
            empty_chip(ui, " ".to_string());
        });

        ui.add_space(6.0);

        // Power row + bar (always reserves height).
        power_metric_row(
            ui,
            &t!("cpu.power"),
            cpu.power_w,
            cpu.power_limit_w,
            Some(&t!("cpu.power_limit_unavailable")),
        );

        // Power history (fixed height, same as GPU).
        power_history_block(
            ui,
            &t!("cpu.power_history"),
            "cpu_power_sparkline",
            power_history,
            cpu.power_limit_w.map(|w| w as f32),
        );
    });
}
