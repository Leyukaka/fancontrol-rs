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
        // While Instant is in the future, do not overwrite slider from HW.
        user_lock_until: HashMap::new(),
        pending_write: HashMap::new(),
        last_hw_write: HashMap::new(),
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
    slider_state: HashMap<String, f32>,
    user_lock_until: HashMap<String, Instant>,
    /// Desired duty queued while dragging (applied throttled / on release).
    pending_write: HashMap<String, u8>,
    last_hw_write: HashMap<String, Instant>,
    write_error: Option<String>,
}

impl eframe::App for FanApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(200));
        self.flush_pending_writes();

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
                "map: {} · fans {} · controls {} · tick {}",
                self.map.sensors.len(),
                snap.fans.len(),
                snap.controls.len(),
                snap.tick
            ));
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.columns(3, |cols| {
                cols[0].heading("Temperatures");
                cols[0].separator();
                egui::ScrollArea::vertical()
                    .id_salt("temps")
                    .show(&mut cols[0], |ui| {
                        if snap.temps.is_empty() {
                            ui.label("(none)");
                        }
                        for (_id, label, v) in &snap.temps {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.monospace(format!("{v:5.1} °C"));
                                    },
                                );
                            });
                        }
                    });

                cols[1].heading("Fans (RPM)");
                cols[1].separator();
                egui::ScrollArea::vertical()
                    .id_salt("fans")
                    .show(&mut cols[1], |ui| {
                        if snap.fans.is_empty() {
                            ui.label("(none)");
                        }
                        for (id, label, v) in &snap.fans {
                            ui.horizontal(|ui| {
                                ui.label(label);
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        if *v < 1.0 {
                                            ui.weak("0");
                                        } else {
                                            ui.monospace(format!("{v:6.0}"));
                                        }
                                    },
                                );
                            });
                            ui.small(id);
                        }
                    });

                cols[2].heading("Controls");
                cols[2].separator();
                egui::ScrollArea::vertical()
                    .id_salt("ctrls")
                    .show(&mut cols[2], |ui| {
                        if snap.controls.is_empty() {
                            ui.label("(none)");
                        }
                        for c in &snap.controls {
                            ui.group(|ui| {
                                ui.label(&c.label);
                                ui.small(&c.id);
                                let slot = c
                                    .id
                                    .rsplit("ctrl")
                                    .next()
                                    .and_then(|s| s.parse::<u32>().ok())
                                    .unwrap_or(0);
                                if slot >= 9 {
                                    ui.small("EC/BIOS may reclaim (SmartFan)");
                                }
                                if let Some(rpm) = c.rpm {
                                    ui.monospace(format!("RPM {rpm:.0}"));
                                } else {
                                    ui.weak("RPM —");
                                }

                                let locked = self.is_user_locked(&c.id);
                                if !locked {
                                    // Only sync from HW when user is not interacting.
                                    self.slider_state
                                        .insert(c.id.clone(), f32::from(c.duty));
                                }
                                let mut value = *self
                                    .slider_state
                                    .get(&c.id)
                                    .unwrap_or(&f32::from(c.duty));

                                let enabled = c.writable
                                    && (self.options.allow_hw_write
                                        || c.id.starts_with("mock."));

                                let mut changed = false;
                                let mut dragging = false;
                                ui.add_enabled_ui(enabled, |ui| {
                                    let resp = ui.add(
                                        egui::Slider::new(&mut value, 0.0..=100.0)
                                            .suffix("%")
                                            .integer()
                                            .clamping(egui::SliderClamping::Always),
                                    );
                                    changed = resp.changed();
                                    dragging = resp.dragged() || resp.has_focus();
                                    if resp.drag_stopped() {
                                        // Final commit on release
                                        self.lock_user(&c.id, Duration::from_millis(1500));
                                        self.queue_write(&c.id, value, true);
                                    }
                                });

                                self.slider_state.insert(c.id.clone(), value);
                                if dragging || changed {
                                    self.lock_user(&c.id, Duration::from_millis(2000));
                                }
                                if changed {
                                    // Throttled live apply while dragging
                                    self.queue_write(&c.id, value, false);
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
    fn is_user_locked(&self, id: &str) -> bool {
        self.user_lock_until
            .get(id)
            .map(|t| Instant::now() < *t)
            .unwrap_or(false)
    }

    fn lock_user(&mut self, id: &str, for_dur: Duration) {
        self.user_lock_until
            .insert(id.to_string(), Instant::now() + for_dur);
    }

    fn queue_write(&mut self, id: &str, duty: f32, force: bool) {
        let percent = duty.round().clamp(0.0, 100.0) as u8;
        self.pending_write.insert(id.to_string(), percent);
        if force {
            self.last_hw_write.remove(id);
        }
    }

    fn flush_pending_writes(&mut self) {
        let now = Instant::now();
        let ids: Vec<String> = self.pending_write.keys().cloned().collect();
        for id in ids {
            if let Some(t) = self.last_hw_write.get(&id) {
                if now.duration_since(*t) < Duration::from_millis(200) {
                    continue;
                }
            }
            let Some(percent) = self.pending_write.remove(&id) else {
                continue;
            };
            // One retry: poller may hold the process SIO lock briefly.
            let mut result = self.reg.set_duty(&ControlId::new(id.clone()), percent);
            if result.is_err() {
                std::thread::sleep(Duration::from_millis(50));
                result = self.reg.set_duty(&ControlId::new(id.clone()), percent);
            }
            match result {
                Ok(()) => {
                    self.last_hw_write.insert(id, now);
                    self.write_error = None;
                }
                Err(e) => {
                    self.write_error = Some(format!("{id}: {e}"));
                }
            }
        }
    }
}
