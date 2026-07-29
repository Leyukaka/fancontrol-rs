//! Activity deck: CPU load sparkline + top processes (CPU / RAM) with filter.

use crate::graph::TempHistory;
use eframe::egui::{self, Color32, RichText, Sense};
use egui_plot::{Line, Plot};
use fancontrol_plugins::ProcessRow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ActivityMode {
    #[default]
    Both,
    LoadOnly,
    ProcessesOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSort {
    #[default]
    Cpu,
    Ram,
}

pub fn load_color(pct: f32) -> Color32 {
    if pct >= 80.0 {
        Color32::from_rgb(230, 80, 80)
    } else if pct >= 50.0 {
        Color32::from_rgb(230, 170, 60)
    } else {
        Color32::from_rgb(80, 190, 120)
    }
}

pub fn format_bytes(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.1} GB", b / GB)
    } else if b >= MB {
        format!("{:.0} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{bytes} B")
    }
}

/// Arguments for [`show_activity_deck`] (keeps the call site readable).
pub struct ActivityDeckView<'a> {
    pub load_history: &'a TempHistory,
    pub processes: &'a [ProcessRow],
    pub load_pct: Option<f64>,
    pub mode: ActivityMode,
    pub sort: &'a mut ProcessSort,
    pub filter: &'a mut String,
    pub top_n: usize,
    /// Load chart X window (minutes), same convention as the thermal graph.
    pub window_minutes: u16,
}

/// Draw the Activity deck inside the available `ui` rect.
pub fn show_activity_deck(ui: &mut egui::Ui, view: ActivityDeckView<'_>) {
    let ActivityDeckView {
        load_history,
        processes,
        load_pct,
        mode,
        sort,
        filter,
        top_n,
        window_minutes,
    } = view;
    ui.horizontal(|ui| {
        ui.heading(t!("activity.heading").to_string());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(p) = load_pct {
                let c = load_color(p as f32);
                ui.label(
                    RichText::new(format!("{p:.0}%"))
                        .monospace()
                        .strong()
                        .size(22.0)
                        .color(c),
                );
                ui.label(t!("activity.cpu_label").to_string());
            } else {
                ui.small(t!("activity.collecting").to_string());
            }
        });
    });

    let show_load = matches!(mode, ActivityMode::Both | ActivityMode::LoadOnly);
    let show_procs = matches!(mode, ActivityMode::Both | ActivityMode::ProcessesOnly);

    if show_load && show_procs {
        ui.columns(2, |cols| {
            cols[0].push_id("activity_load_col", |ui| {
                show_load_plot(ui, load_history, load_pct, window_minutes);
            });
            cols[1].push_id("activity_proc_col", |ui| {
                show_process_table(ui, processes, sort, filter, top_n);
            });
        });
    } else if show_load {
        show_load_plot(ui, load_history, load_pct, window_minutes);
    } else if show_procs {
        show_process_table(ui, processes, sort, filter, top_n);
    }
}

fn show_load_plot(
    ui: &mut egui::Ui,
    history: &TempHistory,
    load_pct: Option<f64>,
    window_minutes: u16,
) {
    ui.small(t!("activity.load_chart").to_string());
    let height = ui.available_height().clamp(80.0, 200.0);
    if history.is_empty() {
        ui.allocate_ui(egui::vec2(ui.available_width(), height), |ui| {
            ui.centered_and_justified(|ui| {
                ui.colored_label(Color32::GRAY, t!("activity.collecting").to_string());
            });
        });
        return;
    }
    // X anchored to last sample (stable grid between samples).
    let points = history.plot_points();
    let color = load_color(load_pct.unwrap_or(0.0) as f32);
    let line = Line::new("cpu_load", points).color(color).width(2.0);
    let window_mins = f64::from(window_minutes.max(1));

    Plot::new("activity_load_plot")
        .height(height)
        .allow_zoom(false)
        .allow_drag(false)
        .allow_scroll(false)
        .include_x(-window_mins)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(100.0)
        .show_axes([true, true])
        .y_axis_label("%")
        .show(ui, |plot_ui| {
            plot_ui.line(line);
        });
}

fn show_process_table(
    ui: &mut egui::Ui,
    processes: &[ProcessRow],
    sort: &mut ProcessSort,
    filter: &mut String,
    top_n: usize,
) {
    ui.horizontal(|ui| {
        ui.small(t!("activity.processes").to_string());
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add(
                egui::TextEdit::singleline(filter)
                    .desired_width(120.0)
                    .hint_text(t!("activity.filter_hint").to_string()),
            );
        });
    });

    ui.horizontal(|ui| {
        ui.small(t!("activity.sort").to_string());
        ui.selectable_value(sort, ProcessSort::Cpu, t!("activity.sort_cpu").to_string());
        ui.selectable_value(sort, ProcessSort::Ram, t!("activity.sort_ram").to_string());
    });

    // Filter once; ASCII-insensitive match avoids per-row `to_lowercase` allocs.
    let filter_lc = filter.to_lowercase();
    let mut rows: Vec<&ProcessRow> = if filter_lc.is_empty() {
        processes.iter().collect()
    } else {
        processes
            .iter()
            .filter(|p| {
                // exe names are typically ASCII; fallback contains for non-ASCII.
                if p.name.is_ascii() && filter_lc.is_ascii() {
                    // case-insensitive substring without allocating
                    let name = p.name.as_bytes();
                    let needle = filter_lc.as_bytes();
                    name.windows(needle.len()).any(|w| {
                        w.iter()
                            .zip(needle.iter())
                            .all(|(a, b)| a.to_ascii_lowercase() == *b)
                    })
                } else {
                    p.name.to_lowercase().contains(&filter_lc)
                }
            })
            .collect()
    };

    // Sampler already returns CPU-desc order; only re-sort for RAM or after filter.
    match *sort {
        ProcessSort::Cpu if filter_lc.is_empty() => {
            // already sorted by CPU in the worker
        }
        ProcessSort::Cpu => rows.sort_by(|a, b| {
            b.cpu_pct
                .partial_cmp(&a.cpu_pct)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        ProcessSort::Ram => rows.sort_by_key(|b| std::cmp::Reverse(b.ram_bytes)),
    }
    rows.truncate(top_n.max(1));

    if rows.is_empty() {
        ui.colored_label(Color32::GRAY, t!("activity.no_processes").to_string());
        return;
    }

    let max_cpu = rows
        .iter()
        .map(|r| r.cpu_pct)
        .fold(1.0_f64, f64::max)
        .max(1.0);
    let max_ram = rows.iter().map(|r| r.ram_bytes).max().unwrap_or(1).max(1) as f64;

    egui::ScrollArea::vertical()
        .auto_shrink([false, true])
        .show(ui, |ui| {
            egui::Grid::new("activity_proc_grid")
                .num_columns(4)
                .spacing([8.0, 4.0])
                .striped(true)
                .show(ui, |ui| {
                    ui.small(RichText::new(t!("activity.col_name").to_string()).strong());
                    ui.small(RichText::new(t!("activity.col_cpu").to_string()).strong());
                    ui.small(RichText::new(t!("activity.col_ram").to_string()).strong());
                    ui.small(RichText::new("PID").strong());
                    ui.end_row();

                    for r in rows {
                        let dim = r.cpu_pct < 0.5 && *sort == ProcessSort::Cpu;
                        let name_color = if dim {
                            Color32::GRAY
                        } else {
                            ui.visuals().text_color()
                        };
                        ui.colored_label(name_color, truncate_name(&r.name, 28));

                        // CPU bar + %
                        let cpu_frac = (r.cpu_pct / max_cpu).clamp(0.0, 1.0) as f32;
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(48.0, 10.0), Sense::hover());
                            let fill = load_color(r.cpu_pct as f32);
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width() * cpu_frac, rect.height()),
                                ),
                                2.0,
                                fill,
                            );
                            ui.painter().rect_stroke(
                                rect,
                                2.0,
                                egui::Stroke::new(1.0, Color32::from_gray(60)),
                                egui::StrokeKind::Outside,
                            );
                            ui.monospace(format!("{:.0}%", r.cpu_pct));
                        });

                        let ram_frac = (r.ram_bytes as f64 / max_ram).clamp(0.0, 1.0) as f32;
                        ui.horizontal(|ui| {
                            let (rect, _) =
                                ui.allocate_exact_size(egui::vec2(48.0, 10.0), Sense::hover());
                            ui.painter().rect_filled(
                                egui::Rect::from_min_size(
                                    rect.min,
                                    egui::vec2(rect.width() * ram_frac, rect.height()),
                                ),
                                2.0,
                                Color32::from_rgb(100, 140, 220),
                            );
                            ui.painter().rect_stroke(
                                rect,
                                2.0,
                                egui::Stroke::new(1.0, Color32::from_gray(60)),
                                egui::StrokeKind::Outside,
                            );
                            ui.monospace(format_bytes(r.ram_bytes));
                        });

                        ui.monospace(format!("{}", r.pid));
                        ui.end_row();
                    }
                });
        });
}

fn truncate_name(name: &str, max: usize) -> String {
    if name.chars().count() <= max {
        name.to_string()
    } else {
        let t: String = name.chars().take(max.saturating_sub(1)).collect();
        format!("{t}…")
    }
}
