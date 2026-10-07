//! Modal-style windows: PWM consent, start with Windows, PawnIO help, rename.

use super::*;

impl FanApp {
    pub(super) fn show_writes_consent_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_writes_consent {
            return;
        }
        egui::Window::new(t!("writes_consent.title").to_string())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(t!("writes_consent.body").to_string());
                ui.add_space(8.0);
                ui.small(t!("writes_consent.hint").to_string());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("writes_consent.accept").to_string()).clicked() {
                        self.settings.writes_risk_acknowledged = true;
                        self.settings.save();
                        self.show_writes_consent = false;
                    }
                    if ui
                        .button(t!("writes_consent.read_only_session").to_string())
                        .clicked()
                    {
                        // Session-only: do not persist read-only; re-prompt next launch.
                        // Curve control is left alone: it is a saved setting, and every
                        // curve apply already refuses to write in a read-only session.
                        self.options.allow_hw_write = false;
                        self.show_writes_consent = false;
                    }
                });
            });
    }

    pub(super) fn show_startup_prompt_dialog(&mut self, ctx: &egui::Context) {
        if !self.show_startup_prompt {
            return;
        }
        egui::Window::new(t!("startup_prompt.title").to_string())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.set_max_width(460.0);
                ui.label(t!("startup_prompt.body").to_string());
                ui.add_space(8.0);
                ui.small(t!("startup_prompt.hint").to_string());
                ui.add_space(12.0);
                ui.horizontal(|ui| {
                    if ui.button(t!("startup_prompt.yes").to_string()).clicked() {
                        match crate::autostart::set_enabled(true) {
                            Ok(()) => {
                                self.settings.launch_on_startup = true;
                            }
                            Err(e) => {
                                self.profile_status =
                                    Some(format!("{}: {e}", t!("options.launch_on_startup_err")));
                            }
                        }
                        self.settings.startup_prompt_shown = true;
                        self.settings.save();
                        self.show_startup_prompt = false;
                    }
                    if ui.button(t!("startup_prompt.no").to_string()).clicked() {
                        let _ = crate::autostart::set_enabled(false);
                        self.settings.launch_on_startup = false;
                        self.settings.startup_prompt_shown = true;
                        self.settings.save();
                        self.show_startup_prompt = false;
                    }
                    if ui.button(t!("startup_prompt.later").to_string()).clicked() {
                        // Ask again next launch (do not set startup_prompt_shown).
                        self.show_startup_prompt = false;
                    }
                });
            });
    }

    pub(super) fn show_pawnio_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.pawnio_dialog else {
            return;
        };
        let mut open = true;
        let title = match kind {
            PawnioDialogKind::NotInstalled => t!("pawnio.title_not_installed").to_string(),
            PawnioDialogKind::NeedsAdmin => t!("pawnio.title_needs_admin").to_string(),
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(t!("pawnio.intro").to_string());
                ui.add_space(6.0);
                match kind {
                    PawnioDialogKind::NotInstalled => {
                        ui.label(t!("pawnio.body_not_installed").to_string());
                    }
                    PawnioDialogKind::NeedsAdmin => {
                        ui.label(t!("pawnio.body_needs_admin_1").to_string());
                        ui.label(t!("pawnio.body_needs_admin_2").to_string());
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label(t!("pawnio.site_label").to_string());
                    ui.hyperlink_to("pawnio.eu", PAWNIO_URL);
                });
                if let Some(msg) = &self.elevate_status {
                    ui.add_space(6.0);
                    ui.colored_label(theme::error(ui.visuals()), msg);
                }
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if matches!(kind, PawnioDialogKind::NeedsAdmin)
                        && !crate::elevation::is_elevated()
                        && ui
                            .button(t!("pawnio.restart_as_admin").to_string())
                            .clicked()
                    {
                        self.try_relaunch_elevated();
                    }
                    if ui.button(t!("pawnio.open_button").to_string()).clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(PAWNIO_URL));
                    }
                    if ui
                        .button(t!("pawnio.continue_without_hw").to_string())
                        .clicked()
                    {
                        self.pawnio_dialog = None;
                    }
                    if ui.button(t!("common.close").to_string()).clicked() {
                        self.pawnio_dialog = None;
                    }
                });
                ui.small(t!("pawnio.footer_note").to_string());
            });
        if !open {
            self.pawnio_dialog = None;
        }
    }

    /// Ask Windows for an elevated relaunch (UAC). On success, exit this process.
    pub(super) fn try_relaunch_elevated(&mut self) {
        match crate::elevation::relaunch_elevated() {
            Ok(()) => {
                // Elevated child is running - leave the non-elevated process
                // (`process::exit` skips `on_exit`, so hand fans back first).
                self.reg.restore_all();
                std::process::exit(0);
            }
            Err(crate::elevation::ElevateError::Cancelled) => {
                self.elevate_status = Some(t!("pawnio.elevate_cancelled").to_string());
            }
            Err(crate::elevation::ElevateError::AlreadyElevated) => {
                self.elevate_status = None;
            }
            Err(e) => {
                self.elevate_status =
                    Some(t!("pawnio.elevate_failed", error = e.to_string()).to_string());
            }
        }
    }

    pub(super) fn begin_rename(&mut self, id: &str, current: &str, is_control: bool) {
        self.rename_id = Some(id.to_string());
        self.rename_buf = current.to_string();
        self.rename_is_control = is_control;
    }

    pub(super) fn show_rename_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.rename_id.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new(t!("rename.title").to_string())
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(&id);
                ui.text_edit_singleline(&mut self.rename_buf);
                ui.horizontal(|ui| {
                    if ui.button(t!("common.save").to_string()).clicked() {
                        let name = self.rename_buf.trim().to_string();
                        if !name.is_empty()
                            && let Ok(mut map) = self.map.lock()
                        {
                            if self.rename_is_control {
                                map.set_control_name(&id, &name);
                            } else {
                                map.set_sensor_name(&id, &name);
                            }
                            if let Err(e) = map.save() {
                                // The new name is live in memory but would be lost on exit.
                                tracing::warn!(error = %e, "channel map save failed");
                                self.profile_status = Some(e.to_string());
                            }
                        }
                        self.rename_id = None;
                    }
                    if ui.button(t!("common.cancel").to_string()).clicked() {
                        self.rename_id = None;
                    }
                });
            });
        if !open {
            self.rename_id = None;
        }
    }
}
