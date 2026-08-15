//! GPU detail panel: power, temps, load, clocks, VRAM (aligned card layout with CPU).

use crate::graph::TempHistory;
use crate::panel_metrics::{
    domain_card, load_color, metric_bar, power_history_block, power_metric_row, temp_chip,
};
use crate::poll::GpuSnap;
use eframe::egui::{self, Color32, RichText};

/// Draw one or more GPU cards. First card may show a shared power history sparkline.
pub fn show_gpu_panel(ui: &mut egui::Ui, gpus: &[GpuSnap], power_history: Option<&TempHistory>) {
    domain_card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.heading(t!("gpu.heading").to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.small(t!("gpu.read_only_note").to_string());
            });
        });

        if gpus.is_empty() {
            ui.colored_label(Color32::GRAY, t!("gpu.none").to_string());
            ui.small(t!("gpu.none_hint").to_string());
            return;
        }

        egui::ScrollArea::vertical()
            .id_salt("gpu_panel_scroll")
            .auto_shrink([false, false])
            .show(ui, |ui| {
                for (i, gpu) in gpus.iter().enumerate() {
                    if i > 0 {
                        ui.add_space(8.0);
                        ui.separator();
                        ui.add_space(4.0);
                    }
                    let history = if i == 0 { power_history } else { None };
                    show_gpu_card(ui, gpu, history);
                }
            });
    });
}

fn show_gpu_card(ui: &mut egui::Ui, gpu: &GpuSnap, power_history: Option<&TempHistory>) {
    // Subtitle = device name (matches CPU package subtitle slot).
    ui.label(RichText::new(&gpu.name).strong().size(16.0));

    // Chip row: Core | Hot Spot | Memory (always 3 slots). N/A notes sit
    // to the right so they do not steal vertical space vs the CPU card.
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        temp_chip(ui, t!("gpu.core").to_string(), gpu.temp_core, true);
        let hotspot = temp_chip(ui, t!("gpu.hotspot").to_string(), gpu.temp_hotspot, false);
        if gpu.temp_hotspot.is_none() {
            hotspot.on_hover_text(t!("gpu.hotspot_tooltip").to_string());
        }
        let memory = temp_chip(ui, t!("gpu.memory_temp").to_string(), gpu.temp_memory, true);
        if gpu.temp_memory.is_none() {
            memory.on_hover_text(t!("gpu.memory_tooltip").to_string());
        }
        if gpu.temp_hotspot.is_none() || gpu.temp_memory.is_none() {
            ui.add_space(6.0);
            ui.vertical(|ui| {
                ui.add_space(4.0);
                if gpu.temp_hotspot.is_none() {
                    ui.small(t!("gpu.hotspot_unavailable").to_string())
                        .on_hover_text(t!("gpu.hotspot_tooltip").to_string());
                }
                if gpu.temp_memory.is_none() {
                    ui.small(t!("gpu.memory_unavailable").to_string())
                        .on_hover_text(t!("gpu.memory_tooltip").to_string());
                }
            });
        }
    });

    ui.add_space(6.0);

    // Power row + bar (same block as CPU).
    power_metric_row(ui, &t!("gpu.power"), gpu.power_w, gpu.power_limit_w, None);

    // Power history (fixed height, same as CPU).
    power_history_block(
        ui,
        &t!("gpu.power_history"),
        "gpu_power_sparkline",
        power_history,
        gpu.power_limit_w.map(|w| w as f32),
    );

    // Secondary metrics below the shared “above the fold” (GPU-only extras).
    ui.add_space(6.0);

    if let Some(u) = gpu.util_gpu {
        ui.horizontal(|ui| {
            ui.label(t!("gpu.util_gpu").to_string());
            ui.label(
                RichText::new(format!("{u:.0}%"))
                    .monospace()
                    .color(load_color(u as f32 / 100.0)),
            );
            if let Some(m) = gpu.util_mem {
                ui.separator();
                ui.label(t!("gpu.util_mem").to_string());
                ui.label(RichText::new(format!("{m:.0}%")).monospace());
            }
        });
        metric_bar(ui, (u / 100.0) as f32, load_color(u as f32 / 100.0));
    }

    if gpu.clock_graphics_mhz.is_some() || gpu.clock_memory_mhz.is_some() {
        ui.horizontal(|ui| {
            ui.label(t!("gpu.clocks").to_string());
            if let Some(c) = gpu.clock_graphics_mhz {
                ui.label(RichText::new(format!("{c:.0} MHz")).monospace());
                ui.small(t!("gpu.clock_core").to_string());
            }
            if let Some(c) = gpu.clock_memory_mhz {
                ui.separator();
                ui.label(RichText::new(format!("{c:.0} MHz")).monospace());
                ui.small(t!("gpu.clock_mem").to_string());
            }
        });
    }

    if let (Some(used), Some(total)) = (gpu.mem_used_mib, gpu.mem_total_mib)
        && total > 0.0
    {
        let frac = (used / total).clamp(0.0, 1.0) as f32;
        ui.horizontal(|ui| {
            ui.label(t!("gpu.vram").to_string());
            ui.label(
                RichText::new(format!("{used:.0} / {total:.0} MiB"))
                    .monospace()
                    .color(load_color(frac)),
            );
        });
        metric_bar(ui, frac, load_color(frac));
    }

    if let Some(f) = gpu.fan_percent {
        ui.horizontal(|ui| {
            ui.label(t!("gpu.fan").to_string());
            ui.label(RichText::new(format!("{f:.0}%")).monospace());
        });
        metric_bar(
            ui,
            (f / 100.0).clamp(0.0, 1.0) as f32,
            Color32::from_rgb(100, 180, 220),
        );
    }
}
