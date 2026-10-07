//! Dashboard lists (Temperatures / Fans / Controls) and the per-control "Also follow" menu.

use super::*;

impl FanApp {
    pub(super) fn ui_temps_column(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
        ui.heading(t!("dashboard.temperatures").to_string());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("temps")
            .show(ui, |ui| {
                if snap.temps.is_empty() {
                    ui.label(t!("dashboard.none").to_string());
                }
                for (id, label, v) in &snap.temps {
                    let clicked = list_row(ui, label, id, |ui| {
                        ui.monospace(format!("{v:5.1} °C"));
                    });
                    if clicked {
                        self.begin_rename(id, label, false);
                    }
                }
            });
    }

    pub(super) fn ui_fans_column(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
        ui.heading(t!("dashboard.fans").to_string());
        ui.separator();
        egui::ScrollArea::vertical().id_salt("fans").show(ui, |ui| {
            let fans: Vec<_> = snap
                .fans
                .iter()
                .filter(|(_, _, v)| !self.settings.hide_zero_rpm || *v >= 1.0)
                .collect();
            if fans.is_empty() {
                ui.label(t!("dashboard.none").to_string());
            }
            for (id, label, v) in fans {
                let clicked = list_row(ui, label, id, |ui| {
                    if *v < 1.0 {
                        ui.weak("0");
                    } else {
                        ui.monospace(format!("{v:6.0}"));
                    }
                });
                if clicked {
                    self.begin_rename(id, label, false);
                }
            }
        });
    }

    pub(super) fn ui_controls_column(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
        ui.heading(t!("dashboard.controls").to_string());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("ctrls")
            .show(ui, |ui| {
                // hide_zero_rpm only affects the Fans list; this is a separate opt-in
                // filter based on duty. `duty: None` stays visible.
                let controls: Vec<_> = snap
                    .controls
                    .iter()
                    .filter(|c| !self.settings.hide_zero_duty_controls || c.duty.unwrap_or(1) >= 1)
                    .collect();
                if controls.is_empty() {
                    ui.label(t!("dashboard.none").to_string());
                }
                for c in controls {
                    egui::Frame::group(ui.style())
                        .inner_margin(egui::Margin::symmetric(10, 8))
                        .show(ui, |ui| {
                            if ui
                                .add(
                                    egui::Label::new(c.label.as_str())
                                        .truncate()
                                        .sense(egui::Sense::click()),
                                )
                                .on_hover_text(t!("dashboard.click_to_rename").to_string())
                                .clicked()
                            {
                                self.begin_rename(&c.id, &c.label, true);
                            }
                            ui.small(&c.id);
                            let slot =
                                c.id.rsplit("ctrl")
                                    .next()
                                    .and_then(|s| s.parse::<u32>().ok())
                                    .unwrap_or(0);
                            if slot >= 9 {
                                ui.small(t!("dashboard.ec_bios_warning").to_string());
                            }
                            ui.add_space(4.0);
                            if let Some(rpm) = c.rpm {
                                ui.monospace(format!("RPM {rpm:.0}"));
                            } else {
                                ui.weak(format!("RPM {}", t!("common.na")));
                            }
                            ui.add_space(4.0);

                            let cur = self
                                .profile
                                .assignments
                                .get(&c.id)
                                .map(|aid| {
                                    self.profile
                                        .curves
                                        .iter()
                                        .find(|cv| cv.id.as_str() == aid)
                                        .map(curve_combo_label)
                                        .unwrap_or(aid.as_str())
                                        .to_string()
                                })
                                .unwrap_or_else(|| t!("dashboard.none").to_string());
                            egui::ComboBox::from_id_salt(format!("asg-{}", c.id))
                                .selected_text(cur)
                                .show_ui(ui, |ui| {
                                    if ui
                                        .selectable_label(
                                            !self.profile.assignments.contains_key(&c.id),
                                            t!("dashboard.none").to_string(),
                                        )
                                        .clicked()
                                    {
                                        self.profile.assignments.remove(&c.id);
                                        self.profile.sensor_bindings.remove(&c.id);
                                    }
                                    let curve_opts: Vec<(String, String)> = self
                                        .profile
                                        .curves
                                        .iter()
                                        .map(|cv| {
                                            (
                                                cv.id.as_str().to_string(),
                                                curve_combo_label(cv).to_string(),
                                            )
                                        })
                                        .collect();
                                    for (cid, label) in curve_opts {
                                        let selected = self
                                            .profile
                                            .assignments
                                            .get(&c.id)
                                            .map(|x| x == &cid)
                                            .unwrap_or(false);
                                        if ui.selectable_label(selected, label).clicked() {
                                            self.profile.assignments.insert(c.id.clone(), cid);
                                            self.profile
                                                .sensor_bindings
                                                .entry(c.id.clone())
                                                .or_insert_with(|| default_cpu_curve_sensor(snap));
                                        }
                                    }
                                });

                            if self.profile.assignments.contains_key(&c.id) {
                                // Curves regulate on CPU-like temps only (not SYSTIN/VRM/GPU).
                                let cpu_temps: Vec<_> = snap
                                    .temps
                                    .iter()
                                    .filter(|(id, _, _)| is_cpu_temp_candidate(id))
                                    .collect();
                                // Keep the user's binding even when that sensor is missing from
                                // this poll: silently retargeting it made the fan follow another
                                // temperature (the control loop's failsafe covers a real outage).
                                let bound_id = match self.profile.sensor_bindings.get(&c.id) {
                                    Some(id) if is_cpu_temp_candidate(id) => id.clone(),
                                    _ => {
                                        let id = default_cpu_curve_sensor(snap);
                                        self.profile
                                            .sensor_bindings
                                            .insert(c.id.clone(), id.clone());
                                        id
                                    }
                                };
                                let bound_label = cpu_temps
                                    .iter()
                                    .find(|(id, _, _)| *id == bound_id)
                                    .map(|(_, label, _)| (*label).clone())
                                    .unwrap_or_else(|| format!("{bound_id} ({})", t!("common.na")));
                                let bind_resp =
                                    egui::ComboBox::from_id_salt(format!("bind-{}", c.id))
                                        .selected_text(bound_label)
                                        .show_ui(ui, |ui| {
                                            for (id, label, _) in &cpu_temps {
                                                let selected = *id == bound_id;
                                                if ui
                                                    .selectable_label(selected, label.as_str())
                                                    .clicked()
                                                    && !selected
                                                {
                                                    self.profile
                                                        .sensor_bindings
                                                        .insert(c.id.clone(), (*id).clone());
                                                }
                                            }
                                        });
                                bind_resp
                                    .response
                                    .on_hover_text(t!("dashboard.curve_sensor_hover").to_string());
                                self.ui_extra_sensors(ui, &c.id, snap, &bound_id);
                            }

                            let locked = self.is_user_locked(&c.id);
                            let hw_duty = c.duty.unwrap_or(0);
                            let holding = self
                                .echo_hold_until
                                .get(&c.id)
                                .is_some_and(|t| Instant::now() < *t);
                            if let Some(d) = c.duty.filter(|_| !locked && !holding) {
                                self.slider_state.insert(c.id.clone(), f32::from(d));
                            }
                            let mut value =
                                *self.slider_state.get(&c.id).unwrap_or(&f32::from(hw_duty));

                            let enabled = c.writable
                                && !self.show_writes_consent
                                && (self.options.allow_hw_write || c.id.starts_with("mock."));

                            if c.duty.is_none() {
                                ui.weak(format!("duty {}", t!("common.na")));
                            }
                            ui.add_space(2.0);

                            let mut changed = false;
                            ui.add_enabled_ui(enabled, |ui| {
                                let resp = ui.add(
                                    egui::Slider::new(&mut value, 0.0..=100.0)
                                        .suffix("%")
                                        .integer()
                                        .clamping(egui::SliderClamping::Always),
                                );
                                changed = resp.changed();
                                if resp.dragged() || resp.has_focus() {
                                    self.lock_user(&c.id, Duration::from_millis(2000));
                                }
                                // Write on release, or keyboard/click step without drag.
                                if resp.drag_stopped() || (changed && !resp.dragged()) {
                                    self.lock_user(&c.id, Duration::from_millis(1500));
                                    self.queue_write(&c.id, value);
                                }
                            });

                            self.slider_state.insert(c.id.clone(), value);
                            if !enabled {
                                ui.small(t!("dashboard.locked").to_string());
                            }
                        });
                    ui.add_space(6.0);
                }
            });
    }

    /// "Also follow" menu for a curve-driven control: extra temperature sensors
    /// (GPU, SSD, motherboard...) whose hottest reading can raise the curve input.
    pub(super) fn ui_extra_sensors(
        &mut self,
        ui: &mut egui::Ui,
        control_id: &str,
        snap: &crate::poll::Snapshot,
        bound_id: &str,
    ) {
        let extras = self
            .profile
            .extra_sensors
            .get(control_id)
            .cloned()
            .unwrap_or_default();
        let label = if extras.is_empty() {
            t!("dashboard.extra_sensors_none").to_string()
        } else {
            t!("dashboard.extra_sensors_some", count = extras.len()).to_string()
        };
        let menu = ui.menu_button(label, |ui| {
            let live = snap.temps.iter().filter(|(id, _, _)| id != bound_id);
            // Keep extras that are absent from this poll listed, so they can be removed.
            let missing = extras
                .iter()
                .filter(|e| !snap.temps.iter().any(|(id, _, _)| id == *e));
            let rows: Vec<(String, String)> = live
                .map(|(id, name, v)| (id.clone(), format!("{name} ({v:.0} °C)")))
                .chain(missing.map(|id| (id.clone(), format!("{id} ({})", t!("common.na")))))
                .collect();
            for (id, text) in rows {
                let mut on = extras.contains(&id);
                if ui.checkbox(&mut on, text).changed() {
                    let list = self
                        .profile
                        .extra_sensors
                        .entry(control_id.to_string())
                        .or_default();
                    if on {
                        list.push(id);
                    } else {
                        list.retain(|e| *e != id);
                    }
                    if list.is_empty() {
                        self.profile.extra_sensors.remove(control_id);
                    }
                    self.profile_status = Some(t!("curves_panel.curve_edited_status").to_string());
                }
            }
        });
        menu.response
            .on_hover_text(t!("dashboard.extra_sensors_hover").to_string());
    }
}
