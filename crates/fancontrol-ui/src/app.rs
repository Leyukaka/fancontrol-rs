//! egui application: live sensors + duty sliders.

use crate::poll::{spawn_poller, SharedSnapshot};
use crate::registry::{backend_status_line, build_registry};
use crate::UiError;
use eframe::egui;
use fancontrol_core::{ChannelMap, ControlId};
use fancontrol_plugins::ProviderRegistry;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[derive(Debug, Clone)]
pub struct UiOptions {
    pub include_mock: bool,
    pub include_hw: bool,
    pub allow_hw_write: bool,
}

impl Default for UiOptions {
    fn default() -> Self {
        Self {
            include_mock: true,
            include_hw: true,
            allow_hw_write: false,
        }
    }
}

pub fn run_native(options: UiOptions) -> Result<(), UiError> {
    let reg = Arc::new(build_registry(
        options.include_mock,
        options.include_hw,
        options.allow_hw_write,
    ));
    let map = ChannelMap::load_or_seed().unwrap_or_default();
    let status = backend_status_line(options.include_hw);
    let snapshot = spawn_poller(Arc::clone(&reg), map.clone(), Duration::from_millis(750));

    let app = FanApp {
        options,
        reg,
        map,
        snapshot,
        status,
        slider_state: HashMap::new(),
        last_write: HashMap::new(),
        write_error: None,
    };

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1040.0, 720.0])
            .with_title("fancontrol-rs"),
        ..Default::default()
    };

    eframe::run_native(
        "fancontrol-rs",
        native,
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            Ok(Box::new(app))
        }),
    )
    .map_err(|e| UiError::Eframe(e.to_string()))
}

struct FanApp {
    options: UiOptions,
    reg: Arc<ProviderRegistry>,
    map: ChannelMap,
    snapshot: SharedSnapshot,
    status: String,
    /// Local slider values (id → duty f32)
    slider_state: HashMap<String, f32>,
    last_write: HashMap<String, Instant>,
    write_error: Option<String>,
}

impl eframe::App for FanApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Continuous repaint so live values refresh
        ctx.request_repaint_after(Duration::from_millis(200));

        let snap = self
            .snapshot
            .lock()
            .map(|g| g.clone())
            .unwrap_or_default();

        egui::TopBottomPanel::top("top").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading("fancontrol-rs");
                ui.separator();
                if self.options.allow_hw_write {
                    ui.colored_label(egui::Color32::LIGHT_RED, "WRITE ENABLED");
                } else {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, "READ-ONLY");
                }
            });
            ui.label(&self.status);
            if let Some(err) = &snap.error {
                ui.colored_label(egui::Color32::YELLOW, format!("poll: {err}"));
            }
            if let Some(err) = &self.write_error {
                ui.colored_label(egui::Color32::RED, format!("write: {err}"));
            }
            if !self.options.allow_hw_write {
                ui.small("Hardware sliders locked. CLI: cargo run -- --allow-hw-write ui");
            }
            ui.small(format!(
                "map: {} sensors · tick {}",
                self.map.sensors.len(),
                snap.tick
            ));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(3, |cols| {
                // Temperatures
                cols[0].heading("Temperatures");
                cols[0].separator();
                egui::ScrollArea::vertical().show(&mut cols[0], |ui| {
                    if snap.temps.is_empty() {
                        ui.label("(none)");
                    }
                    for (_id, label, v) in &snap.temps {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.monospace(format!("{v:5.1} °C"));
                            });
                        });
                    }
                });

                // Fans
                cols[1].heading("Fans (RPM)");
                cols[1].separator();
                egui::ScrollArea::vertical().show(&mut cols[1], |ui| {
                    if snap.fans.is_empty() {
                        ui.label("(none)");
                    }
                    for (_id, label, v) in &snap.fans {
                        ui.horizontal(|ui| {
                            ui.label(label);
                            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                ui.monospace(format!("{v:6.0}"));
                            });
                        });
                    }
                });

                // Controls + sliders
                cols[2].heading("Controls");
                cols[2].separator();
                egui::ScrollArea::vertical().show(&mut cols[2], |ui| {
                    if snap.controls.is_empty() {
                        ui.label("(none with activity — mock or live fans)");
                    }
                    for c in &snap.controls {
                        ui.group(|ui| {
                            ui.label(&c.label);
                            ui.small(&c.id);
                            if let Some(rpm) = c.rpm {
                                ui.monospace(format!("RPM {rpm:.0}"));
                            }
                            let pull = self
                                .last_write
                                .get(&c.id)
                                .map(|t| t.elapsed() > Duration::from_millis(800))
                                .unwrap_or(true);
                            if pull {
                                self.slider_state
                                    .insert(c.id.clone(), f32::from(c.duty));
                            } else {
                                self.slider_state
                                    .entry(c.id.clone())
                                    .or_insert(f32::from(c.duty));
                            }
                            let mut value = *self
                                .slider_state
                                .get(&c.id)
                                .unwrap_or(&f32::from(c.duty));

                            let enabled = c.writable
                                && (self.options.allow_hw_write
                                    || c.id.starts_with("mock."));
                            let mut changed = false;
                            ui.add_enabled_ui(enabled, |ui| {
                                let resp = ui.add(
                                    egui::Slider::new(&mut value, 0.0..=100.0)
                                        .suffix("%")
                                        .integer(),
                                );
                                changed = resp.changed();
                            });
                            self.slider_state.insert(c.id.clone(), value);
                            if changed {
                                self.apply_slider(&c.id, value);
                            }
                            if !enabled {
                                ui.small("locked");
                            }
                        });
                        ui.add_space(6.0);
                    }
                });
            });
        });
    }
}

impl FanApp {
    fn apply_slider(&mut self, id: &str, duty: f32) {
        let percent = duty.round().clamp(0.0, 100.0) as u8;
        // Throttle hardware writes
        if let Some(t) = self.last_write.get(id) {
            if t.elapsed() < Duration::from_millis(120) {
                return;
            }
        }
        match self.reg.set_duty(&ControlId::new(id), percent) {
            Ok(()) => {
                self.last_write.insert(id.to_string(), Instant::now());
                self.write_error = None;
            }
            Err(e) => {
                self.write_error = Some(format!("{id}: {e}"));
            }
        }
    }
}
