//! GPU detail panel: power, temps, load, clocks, VRAM (read-only host metrics).

use crate::poll::GpuSnap;
use eframe::egui::{self, Color32, RichText};

/// Draw one or more GPU cards into `ui`.
pub fn show_gpu_panel(ui: &mut egui::Ui, gpus: &[GpuSnap]) {
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
                show_gpu_card(ui, gpu);
            }
        });
}

fn show_gpu_card(ui: &mut egui::Ui, gpu: &GpuSnap) {
    ui.label(RichText::new(&gpu.name).strong().size(16.0));

    // Temperature row: Core | Hot Spot | Memory
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        temp_chip(ui, t!("gpu.core").to_string(), gpu.temp_core, true);
        temp_chip(ui, t!("gpu.hotspot").to_string(), gpu.temp_hotspot, false);
        temp_chip(ui, t!("gpu.memory_temp").to_string(), gpu.temp_memory, true);
    });
    if gpu.temp_hotspot.is_none() {
        ui.small(t!("gpu.hotspot_unavailable").to_string())
            .on_hover_text(t!("gpu.hotspot_tooltip").to_string());
    }

    ui.add_space(6.0);

    // Power
    if gpu.power_w.is_some() || gpu.power_limit_w.is_some() {
        let draw = gpu.power_w.unwrap_or(0.0);
        let limit = gpu.power_limit_w.unwrap_or(0.0).max(1.0);
        let frac = if gpu.power_limit_w.is_some() {
            (draw / limit).clamp(0.0, 1.5) as f32
        } else {
            0.0
        };
        let text = match (gpu.power_w, gpu.power_limit_w) {
            (Some(d), Some(l)) => format!("{d:.0} / {l:.0} W"),
            (Some(d), None) => format!("{d:.0} W"),
            (None, Some(l)) => format!("— / {l:.0} W"),
            (None, None) => "—".into(),
        };
        ui.horizontal(|ui| {
            ui.label(t!("gpu.power").to_string());
            ui.label(
                RichText::new(text)
                    .monospace()
                    .strong()
                    .color(power_color(frac)),
            );
        });
        if gpu.power_w.is_some() && gpu.power_limit_w.is_some() {
            metric_bar(ui, frac.min(1.0), power_color(frac));
        }
    }

    // Utilization
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

    // Clocks
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

    // VRAM
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

    // Fan %
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

fn temp_chip(ui: &mut egui::Ui, label: String, value: Option<f64>, colorize: bool) {
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
