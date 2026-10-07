//! Profiles & curves panel: profile pick/save, curve list, curve editor host.

use super::*;

impl FanApp {
    /// Temperature the selected curve is actually driven by: the input of the first
    /// control using it (same resolution as the control loop), else CPU temp.
    pub(super) fn selected_curve_temp(&self, snap: &crate::poll::Snapshot) -> Option<f64> {
        let curve_id = self.profile.curves.get(self.selected_curve)?.id.as_str();
        let temps: HashMap<String, f64> = snap
            .temps
            .iter()
            .map(|(id, _, v)| (id.clone(), *v))
            .collect();
        self.profile
            .assignments
            .iter()
            .filter(|(_, cid)| cid.as_str() == curve_id)
            .find_map(|(ctrl, _)| {
                let bound = self.profile.sensor_bindings.get(ctrl).map(String::as_str);
                let id = resolve_curve_temp_sensor(bound, &temps)?;
                let base = temps.get(&id).copied()?;
                // Same input as the control loop: the hottest of the extra sensors.
                let extras = self.profile.extra_sensors.get(ctrl).into_iter().flatten();
                let hottest = extras
                    .filter_map(|e| temps.get(e).copied())
                    .filter(|t| t.is_finite())
                    .fold(base, f64::max);
                Some(hottest)
            })
            .or(snap.cpu_temp)
    }

    /// First free `curveN` id: a loaded profile can already use `curve3` with only
    /// two curves, and duplicate ids are indistinguishable.
    pub(super) fn free_curve_id(&self) -> String {
        (self.profile.curves.len() + 1..)
            .map(|n| format!("curve{n}"))
            .find(|id| self.profile.find_curve(id).is_none())
            .unwrap_or_default()
    }

    pub(super) fn ui_curves_panel(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
        let live_temp = self.selected_curve_temp(snap);
        ui.horizontal(|ui| {
            ui.heading(t!("curves_panel.heading").to_string());
            if let Some(s) = &self.profile_status {
                ui.small(s);
            }
        });
        ui.horizontal(|ui| {
            ui.label(t!("curves_panel.profile_label").to_string());
            egui::ComboBox::from_id_salt("profile_pick")
                .selected_text(self.profile.id.as_str())
                .show_ui(ui, |ui| {
                    for id in self.profile_list.clone() {
                        if ui
                            .selectable_label(self.profile.id.as_str() == id, &id)
                            .clicked()
                            && let Ok(p) = load_profile(&id)
                        {
                            self.profile = p;
                            self.selected_curve = 0;
                            self.curve_states.clear();
                            self.profile_status =
                                Some(t!("curves_panel.loaded_status", id = id).to_string());
                            self.settings.last_profile_id = Some(id.clone());
                            self.settings.save();
                        }
                    }
                });
            if ui
                .button(t!("curves_panel.reload_list").to_string())
                .clicked()
            {
                self.profile_list = list_profiles().unwrap_or_default();
            }
            if ui.button(t!("curves_panel.save").to_string()).clicked() {
                match save_profile(&self.profile) {
                    Ok(path) => {
                        self.profile_status = Some(
                            t!(
                                "curves_panel.saved_status",
                                path = path.display().to_string()
                            )
                            .to_string(),
                        );
                        self.profile_list = list_profiles().unwrap_or_default();
                        self.settings.last_profile_id = Some(self.profile.id.as_str().to_string());
                        self.settings.save();
                    }
                    Err(e) => {
                        self.profile_status =
                            Some(t!("curves_panel.save_error", error = e).to_string())
                    }
                }
            }
            ui.text_edit_singleline(&mut self.new_profile_name);
            if ui
                .button(t!("curves_panel.new_save_as").to_string())
                .clicked()
            {
                let name = self.new_profile_name.trim();
                if !name.is_empty() {
                    self.profile.id = fancontrol_core::ProfileId::new(name);
                    self.profile.name = name.to_string();
                    match save_profile(&self.profile) {
                        Ok(_) => {
                            self.profile_list = list_profiles().unwrap_or_default();
                            self.profile_status =
                                Some(t!("curves_panel.saved_as_status", name = name).to_string());
                            self.settings.last_profile_id = Some(name.to_string());
                            self.settings.save();
                        }
                        Err(e) => {
                            self.profile_status =
                                Some(t!("curves_panel.save_error", error = e).to_string())
                        }
                    }
                }
            }
            if ui
                .button(t!("curves_panel.apply_now").to_string())
                .clicked()
            {
                let s = self.snapshot.lock().map(|g| g.clone()).unwrap_or_default();
                self.apply_curves_from_snapshot(&s);
                self.profile_status = Some(t!("curves_panel.curves_applied_once").to_string());
            }
        });

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label(t!("curves_panel.curves_label").to_string());
                let n = self.profile.curves.len();
                for i in 0..n {
                    let name = self.profile.curves[i].name.clone();
                    if ui
                        .selectable_label(self.selected_curve == i, name)
                        .clicked()
                    {
                        self.selected_curve = i;
                    }
                }
                if ui
                    .button(t!("curves_panel.add_curve").to_string())
                    .clicked()
                {
                    let id = self.free_curve_id();
                    self.profile.curves.push(FanCurve::linear(
                        id,
                        t!("curves_panel.new_curve_name").to_string(),
                        30.0,
                        80.0,
                        20,
                        100,
                    ));
                    self.selected_curve = self.profile.curves.len().saturating_sub(1);
                }
                if let Some(selected) = self.profile.curves.get(self.selected_curve).cloned() {
                    if ui
                        .button(t!("curves_panel.duplicate_curve").to_string())
                        .clicked()
                    {
                        let mut copy = selected.clone();
                        copy.id = fancontrol_core::CurveId::new(self.free_curve_id());
                        copy.name = t!("curves_panel.copy_name", name = selected.name).to_string();
                        self.profile.curves.push(copy);
                        self.selected_curve = self.profile.curves.len() - 1;
                    }
                    // A curve still assigned to a fan cannot go: the fan would lose
                    // its curve (and fall back to failsafe) without the user noticing.
                    let in_use = self
                        .profile
                        .assignments
                        .values()
                        .any(|cid| cid == selected.id.as_str());
                    if ui
                        .add_enabled(
                            !in_use,
                            egui::Button::new(t!("curves_panel.delete_curve").to_string()),
                        )
                        .on_disabled_hover_text(t!("curves_panel.delete_in_use").to_string())
                        .clicked()
                    {
                        self.profile.curves.remove(self.selected_curve);
                        self.selected_curve = self.selected_curve.saturating_sub(1);
                        self.profile_status =
                            Some(t!("curves_panel.curve_edited_status").to_string());
                    }
                }
            });
            ui.separator();
            // Fans driven by the selected curve, by their display name.
            let users: Vec<String> = self
                .profile
                .curves
                .get(self.selected_curve)
                .map(|curve| {
                    self.profile
                        .assignments
                        .iter()
                        .filter(|(_, cid)| cid.as_str() == curve.id.as_str())
                        .map(|(ctrl, _)| {
                            snap.controls
                                .iter()
                                .find(|c| &c.id == ctrl)
                                .map_or_else(|| ctrl.clone(), |c| c.label.clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            ui.vertical(|ui| {
                if let Some(curve) = self.profile.curves.get_mut(self.selected_curve) {
                    let mut name = curve.name.clone();
                    if ui.text_edit_singleline(&mut name).changed() {
                        curve.name = name;
                    }
                    if users.is_empty() {
                        ui.small(t!("curves_panel.used_by_none").to_string());
                    } else {
                        ui.small(t!("curves_panel.used_by", list = users.join(", ")).to_string());
                    }
                    if show_curve_editor(ui, curve, live_temp) {
                        self.profile_status =
                            Some(t!("curves_panel.curve_edited_status").to_string());
                    }
                } else {
                    ui.label(t!("curves_panel.no_curve_selected").to_string());
                }
            });
        });
    }
}
