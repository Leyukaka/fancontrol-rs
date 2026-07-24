//! egui application: live sensors, sliders, graph, rename, options.

use crate::curve_editor::show_curve_editor;
use crate::graph::{show_cpu_graph, TempHistory};
use crate::poll::{spawn_poller, SharedMap, SharedSnapshot};
use crate::registry::{backend_status_line, build_registry};
use crate::settings::UiSettings;
use crate::write_queue::WriteQueue;
use crate::UiError;
use eframe::egui;
use fancontrol_core::{
    evaluate_profile_step, list_profiles, load_profile, save_profile, ChannelMap, CurveEvalState,
    FanCurve, Profile,
};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const GRAPH_WINDOWS: [u16; 4] = [10, 20, 30, 60];
const GRAPH_SAMPLES: [u16; 4] = [1, 2, 5, 10];
const PAWNIO_URL: &str = "https://pawnio.eu";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PawnioDialogKind {
    NotInstalled,
    NeedsAdmin,
}

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
            allow_hw_write: true,
        }
    }
}

fn detect_pawnio_dialog(include_hw: bool) -> Option<PawnioDialogKind> {
    if !include_hw {
        return None;
    }
    if !fancontrol_pawnio::is_installed() {
        Some(PawnioDialogKind::NotInstalled)
    } else if !fancontrol_pawnio::is_available() {
        Some(PawnioDialogKind::NeedsAdmin)
    } else {
        None
    }
}

pub fn run_native(options: UiOptions) -> Result<(), UiError> {
    let settings = UiSettings::load();
    let built = build_registry(
        options.include_mock,
        options.include_hw,
        options.allow_hw_write,
        settings.show_host_sensors,
    );
    let reg = Arc::new(built.reg);
    let map = Arc::new(Mutex::new(ChannelMap::load_or_seed().unwrap_or_default()));
    let status = backend_status_line(options.include_hw);
    let snapshot = spawn_poller(
        Arc::clone(&reg),
        built.pawnio,
        Arc::clone(&map),
        Duration::from_millis(750),
    );
    let writes = WriteQueue::start(Arc::clone(&reg));
    let profile = load_or_create_default_profile();
    let pawnio_dialog = detect_pawnio_dialog(options.include_hw);

    let mut cpu_history = TempHistory::default();
    cpu_history.configure(settings.graph_window_minutes, settings.graph_sample_secs);

    let app = FanApp {
        options,
        map,
        snapshot,
        writes,
        status,
        settings,
        slider_state: HashMap::new(),
        user_lock_until: HashMap::new(),
        write_error: None,
        rename_id: None,
        rename_buf: String::new(),
        rename_is_control: false,
        cpu_history,
        show_settings: false,
        show_curves: true,
        profile,
        profile_list: list_profiles().unwrap_or_default(),
        selected_curve: 0,
        curve_states: HashMap::new(),
        last_curve_apply: Instant::now() - Duration::from_secs(10),
        last_applied_duty: HashMap::new(),
        profile_status: None,
        new_profile_name: "default".into(),
        pawnio_dialog,
    };

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .map_err(|e| UiError::Eframe(format!("app icon: {e}")))?;

    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 860.0])
            .with_title("fancontrol-rs")
            .with_icon(icon),
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
    map: SharedMap,
    snapshot: SharedSnapshot,
    writes: WriteQueue,
    status: String,
    settings: UiSettings,
    slider_state: HashMap<String, f32>,
    user_lock_until: HashMap<String, Instant>,
    write_error: Option<String>,
    rename_id: Option<String>,
    rename_buf: String,
    rename_is_control: bool,
    cpu_history: TempHistory,
    show_settings: bool,
    show_curves: bool,
    profile: Profile,
    profile_list: Vec<String>,
    selected_curve: usize,
    curve_states: HashMap<String, CurveEvalState>,
    last_curve_apply: Instant,
    last_applied_duty: HashMap<String, u8>,
    profile_status: Option<String>,
    new_profile_name: String,
    pawnio_dialog: Option<PawnioDialogKind>,
}

fn load_or_create_default_profile() -> Profile {
    if let Ok(p) = load_profile("default") {
        return p;
    }
    let mut p = Profile::new("default", "Default");
    p.curves
        .push(FanCurve::linear("quiet", "Quiet", 30.0, 75.0, 25, 100));
    p.assignments
        .insert("pawnio.0.ctrl0".into(), "quiet".into());
    p.sensor_bindings
        .insert("pawnio.0.ctrl0".into(), "pawnio.0.temp.CPU".into());
    p.assignments
        .insert("pawnio.0.ctrl1".into(), "quiet".into());
    p.sensor_bindings
        .insert("pawnio.0.ctrl1".into(), "pawnio.0.temp.CPU".into());
    let _ = save_profile(&p);
    p
}

impl eframe::App for FanApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(200));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        if let Some(e) = self.writes.take_error() {
            self.write_error = Some(e);
        }

        let snap = self.snapshot.lock().map(|g| g.clone()).unwrap_or_default();

        if let Some(t) = snap.cpu_temp {
            self.cpu_history.push_if_due(t as f32, Instant::now());
        }

        // Auto-apply curves ~1 Hz when enabled + write allowed
        if self.settings.auto_apply_curves
            && self.options.allow_hw_write
            && self.last_curve_apply.elapsed() >= Duration::from_millis(1000)
        {
            self.apply_curves_from_snapshot(&snap);
            self.last_curve_apply = Instant::now();
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("fancontrol-rs");
                ui.separator();
                if self.options.allow_hw_write {
                    ui.colored_label(egui::Color32::LIGHT_RED, "WRITE ENABLED");
                } else {
                    ui.colored_label(egui::Color32::LIGHT_GREEN, "READ-ONLY");
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("⚙ Options").clicked() {
                        self.show_settings = !self.show_settings;
                    }
                    if ui
                        .selectable_label(self.show_curves, "Curves")
                        .on_hover_text("Afficher/masquer l'éditeur de courbes")
                        .clicked()
                    {
                        self.show_curves = !self.show_curves;
                    }
                    // Prominent Curve control toggle (auto-apply to hardware)
                    let curve_on = self.settings.auto_apply_curves;
                    let (label, fill, text_color) = if curve_on {
                        (
                            "Curve control: ON",
                            egui::Color32::from_rgb(30, 90, 50),
                            egui::Color32::from_rgb(140, 255, 170),
                        )
                    } else {
                        (
                            "Curve control: OFF",
                            egui::Color32::from_rgb(70, 40, 40),
                            egui::Color32::from_rgb(220, 160, 160),
                        )
                    };
                    let btn =
                        egui::Button::new(egui::RichText::new(label).color(text_color).strong())
                            .fill(fill);
                    if ui
                        .add(btn)
                        .on_hover_text(
                            "Activer l'application automatique des courbes aux contrôles PWM",
                        )
                        .clicked()
                    {
                        self.settings.auto_apply_curves = !self.settings.auto_apply_curves;
                        self.settings.save();
                    }
                });
            });
            if self.settings.auto_apply_curves && !self.options.allow_hw_write {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    "Curve control ON but hardware is read-only — drop --read-only to write PWM",
                );
            }
            ui.label(&self.status);
            if let Some(err) = &snap.error {
                ui.colored_label(egui::Color32::YELLOW, format!("poll: {err}"));
            }
            if let Some(err) = &self.write_error {
                ui.colored_label(egui::Color32::RED, format!("write: {err}"));
            }
            if !self.options.allow_hw_write {
                ui.small("Hardware sliders locked · launched with --read-only");
            }
            ui.small(format!(
                "temps {} · fans {} · controls {} · tick {}",
                snap.temps.len(),
                snap.fans.len(),
                snap.controls.len(),
                snap.tick
            ));
        });

        if self.show_settings {
            egui::Panel::right("settings")
                .resizable(true)
                .default_size(260.0)
                .show(ui, |ui| {
                    ui.heading("Options");
                    ui.separator();
                    let mut dirty = false;
                    dirty |= ui
                        .checkbox(&mut self.settings.hide_zero_rpm, "Cacher ventilos à 0 RPM")
                        .changed();
                    dirty |= ui
                        .checkbox(&mut self.settings.show_cpu_graph, "Graphe température CPU")
                        .changed();
                    dirty |= ui
                        .checkbox(
                            &mut self.settings.show_host_sensors,
                            "GPU / SSD (host, si dispo)",
                        )
                        .changed();
                    dirty |= ui
                        .checkbox(
                            &mut self.settings.auto_apply_curves,
                            "Appliquer les courbes (auto, 1 Hz)",
                        )
                        .changed();
                    if self.settings.auto_apply_curves && !self.options.allow_hw_write {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            "Auto-apply needs writes (drop --read-only)",
                        );
                    }
                    ui.small("GPU: nvidia-smi · SSD: DeviceIoControl (no PowerShell)");
                    ui.separator();
                    ui.label("RGB");
                    ui.small(
                        "Contrôle RGB prévu dans une version ultérieure (hors scope v1 hardware).",
                    );
                    ui.separator();
                    ui.label("Noms");
                    ui.small("Clic sur un nom de fan/control pour renommer (sauvé dans channel-map.json).");
                    if dirty {
                        self.settings.save();
                    }
                    if ui.button("Fermer").clicked() {
                        self.show_settings = false;
                    }
                });
        }

        if self.show_curves {
            egui::Panel::bottom("curves")
                .resizable(true)
                .default_size(280.0)
                .show(ui, |ui| {
                    self.ui_curves_panel(ui, snap.cpu_temp);
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if self.settings.show_cpu_graph {
                self.ui_graph_controls(ui);
                show_cpu_graph(
                    ui,
                    &self.cpu_history,
                    "CPU temperature",
                    self.settings.graph_window_minutes,
                );
                ui.add_space(8.0);
            }

            ui.columns(3, |cols| {
                // Temps
                cols[0].heading("Temperatures");
                cols[0].separator();
                egui::ScrollArea::vertical()
                    .id_salt("temps")
                    .show(&mut cols[0], |ui| {
                        if snap.temps.is_empty() {
                            ui.label("(none)");
                        }
                        for (id, label, v) in &snap.temps {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(label.as_str())
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Cliquer pour renommer")
                                    .clicked()
                                {
                                    self.begin_rename(id, label, false);
                                }
                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        ui.monospace(format!("{v:5.1} °C"));
                                    },
                                );
                            });
                            ui.small(id);
                        }
                    });

                // Fans
                cols[1].heading("Fans (RPM)");
                cols[1].separator();
                egui::ScrollArea::vertical()
                    .id_salt("fans")
                    .show(&mut cols[1], |ui| {
                        let fans: Vec<_> = snap
                            .fans
                            .iter()
                            .filter(|(_, _, v)| !self.settings.hide_zero_rpm || *v >= 1.0)
                            .collect();
                        if fans.is_empty() {
                            ui.label("(none)");
                        }
                        for (id, label, v) in fans {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(label.as_str())
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Cliquer pour renommer")
                                    .clicked()
                                {
                                    self.begin_rename(id, label, false);
                                }
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

                // Controls
                cols[2].heading("Controls");
                cols[2].separator();
                egui::ScrollArea::vertical()
                    .id_salt("ctrls")
                    .show(&mut cols[2], |ui| {
                        if snap.controls.is_empty() {
                            ui.label("(none)");
                        }
                        for c in &snap.controls {
                            // Never hide controls when hide_zero_rpm (that option is for fan list only)
                            ui.group(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(c.label.as_str())
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text("Cliquer pour renommer")
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
                                    ui.small("EC/BIOS may reclaim (SmartFan)");
                                }
                                if let Some(rpm) = c.rpm {
                                    ui.monospace(format!("RPM {rpm:.0}"));
                                } else {
                                    ui.weak("RPM —");
                                }

                                // Curve assignment for this control
                                let cur = self
                                    .profile
                                    .assignments
                                    .get(&c.id)
                                    .cloned()
                                    .unwrap_or_else(|| "(none)".into());
                                egui::ComboBox::from_id_salt(format!("asg-{}", c.id))
                                    .selected_text(cur)
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                !self.profile.assignments.contains_key(&c.id),
                                                "(none)",
                                            )
                                            .clicked()
                                        {
                                            self.profile.assignments.remove(&c.id);
                                            self.profile.sensor_bindings.remove(&c.id);
                                        }
                                        let curve_ids: Vec<String> = self
                                            .profile
                                            .curves
                                            .iter()
                                            .map(|cv| cv.id.as_str().to_string())
                                            .collect();
                                        for cid in curve_ids {
                                            let selected = self
                                                .profile
                                                .assignments
                                                .get(&c.id)
                                                .map(|x| x == &cid)
                                                .unwrap_or(false);
                                            if ui.selectable_label(selected, &cid).clicked() {
                                                self.profile.assignments.insert(c.id.clone(), cid);
                                                self.profile.sensor_bindings.insert(
                                                    c.id.clone(),
                                                    "pawnio.0.temp.CPU".into(),
                                                );
                                            }
                                        }
                                    });

                                let locked = self.is_user_locked(&c.id);
                                let hw_duty = c.duty.unwrap_or(0);
                                if !locked {
                                    if let Some(d) = c.duty {
                                        self.slider_state.insert(c.id.clone(), f32::from(d));
                                    }
                                }
                                let mut value =
                                    *self.slider_state.get(&c.id).unwrap_or(&f32::from(hw_duty));

                                let enabled = c.writable
                                    && (self.options.allow_hw_write || c.id.starts_with("mock."));

                                if c.duty.is_none() {
                                    ui.weak("duty —");
                                }

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
                                    // Write only on release — avoids EC spam + UI freezes
                                    if resp.drag_stopped() {
                                        self.lock_user(&c.id, Duration::from_millis(1500));
                                        self.queue_write(&c.id, value);
                                    } else if changed && !resp.dragged() {
                                        // Keyboard / click step
                                        self.lock_user(&c.id, Duration::from_millis(1500));
                                        self.queue_write(&c.id, value);
                                    }
                                });

                                self.slider_state.insert(c.id.clone(), value);
                                if !enabled {
                                    ui.small("locked");
                                }
                            });
                            ui.add_space(6.0);
                        }
                    });
            });
        });

        self.show_rename_modal(&ctx);
        self.show_pawnio_dialog(&ctx);
    }
}

impl FanApp {
    fn ui_graph_controls(&mut self, ui: &mut egui::Ui) {
        let mut dirty = false;
        ui.horizontal(|ui| {
            ui.label("Fenêtre");
            for m in GRAPH_WINDOWS {
                let selected = self.settings.graph_window_minutes == m;
                if ui.selectable_label(selected, format!("{m}m")).clicked() && !selected {
                    self.settings.graph_window_minutes = m;
                    dirty = true;
                }
            }
            ui.separator();
            ui.label("Échantillonnage");
            for s in GRAPH_SAMPLES {
                let selected = self.settings.graph_sample_secs == s;
                if ui.selectable_label(selected, format!("{s}s")).clicked() && !selected {
                    self.settings.graph_sample_secs = s;
                    dirty = true;
                }
            }
        });
        if dirty {
            self.settings.clamp_graph_options();
            self.settings.save();
            self.cpu_history.configure(
                self.settings.graph_window_minutes,
                self.settings.graph_sample_secs,
            );
        }
    }

    fn show_pawnio_dialog(&mut self, ctx: &egui::Context) {
        let Some(kind) = self.pawnio_dialog else {
            return;
        };
        let mut open = true;
        let title = match kind {
            PawnioDialogKind::NotInstalled => "PawnIO non installé",
            PawnioDialogKind::NeedsAdmin => "PawnIO — droits administrateur",
        };
        egui::Window::new(title)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.set_max_width(440.0);
                ui.label(
                    "PawnIO est un pilote kernel d'I/O pour Super I/O (capteurs / ventilateurs), \
                     plus sûr que WinRing0.",
                );
                ui.add_space(6.0);
                match kind {
                    PawnioDialogKind::NotInstalled => {
                        ui.label(
                            "PawnIO n'est pas installé sur cette machine. Installez-le depuis \
                             le site officiel, puis relancez l'application en Administrateur.",
                        );
                    }
                    PawnioDialogKind::NeedsAdmin => {
                        ui.label(
                            "PawnIO est installé, mais l'exécuteur n'est pas accessible \
                             (souvent : droits administrateur manquants, ou service fermé).",
                        );
                        ui.label("Relancez fancontrol-rs en tant qu'Administrateur.");
                    }
                }
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.label("Site :");
                    ui.hyperlink_to("pawnio.eu", PAWNIO_URL);
                });
                ui.add_space(10.0);
                ui.horizontal(|ui| {
                    if ui.button("Ouvrir pawnio.eu").clicked() {
                        ui.ctx().open_url(egui::OpenUrl::new_tab(PAWNIO_URL));
                    }
                    if ui.button("Continuer sans hardware").clicked() {
                        self.pawnio_dialog = None;
                    }
                    if ui.button("Fermer").clicked() {
                        self.pawnio_dialog = None;
                    }
                });
                ui.small(
                    "L'app reste utilisable (mock / config). Aucune installation automatique.",
                );
            });
        if !open {
            self.pawnio_dialog = None;
        }
    }

    fn ui_curves_panel(&mut self, ui: &mut egui::Ui, live_temp: Option<f64>) {
        ui.horizontal(|ui| {
            ui.heading("Profiles & curves");
            if let Some(s) = &self.profile_status {
                ui.small(s);
            }
        });
        ui.horizontal(|ui| {
            ui.label("Profile");
            egui::ComboBox::from_id_salt("profile_pick")
                .selected_text(self.profile.id.as_str())
                .show_ui(ui, |ui| {
                    for id in self.profile_list.clone() {
                        if ui
                            .selectable_label(self.profile.id.as_str() == id, &id)
                            .clicked()
                        {
                            if let Ok(p) = load_profile(&id) {
                                self.profile = p;
                                self.selected_curve = 0;
                                self.curve_states.clear();
                                self.profile_status = Some(format!("Loaded {id}"));
                            }
                        }
                    }
                });
            if ui.button("Reload list").clicked() {
                self.profile_list = list_profiles().unwrap_or_default();
            }
            if ui.button("Save").clicked() {
                match save_profile(&self.profile) {
                    Ok(path) => {
                        self.profile_status = Some(format!("Saved {}", path.display()));
                        self.profile_list = list_profiles().unwrap_or_default();
                    }
                    Err(e) => self.profile_status = Some(format!("Save error: {e}")),
                }
            }
            ui.text_edit_singleline(&mut self.new_profile_name);
            if ui.button("New / Save as").clicked() {
                let name = self.new_profile_name.trim();
                if !name.is_empty() {
                    self.profile.id = fancontrol_core::ProfileId::new(name);
                    self.profile.name = name.to_string();
                    match save_profile(&self.profile) {
                        Ok(_) => {
                            self.profile_list = list_profiles().unwrap_or_default();
                            self.profile_status = Some(format!("Saved as {name}"));
                        }
                        Err(e) => self.profile_status = Some(format!("Save error: {e}")),
                    }
                }
            }
            if ui.button("Apply now").clicked() {
                let s = self.snapshot.lock().map(|g| g.clone()).unwrap_or_default();
                self.apply_curves_from_snapshot(&s);
                self.profile_status = Some("Curves applied once".into());
            }
        });

        ui.horizontal(|ui| {
            ui.vertical(|ui| {
                ui.label("Curves");
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
                if ui.button("+ curve").clicked() {
                    let id = format!("curve{}", self.profile.curves.len() + 1);
                    self.profile.curves.push(FanCurve::linear(
                        id,
                        "New curve",
                        30.0,
                        80.0,
                        20,
                        100,
                    ));
                    self.selected_curve = self.profile.curves.len().saturating_sub(1);
                }
            });
            ui.separator();
            ui.vertical(|ui| {
                if let Some(curve) = self.profile.curves.get_mut(self.selected_curve) {
                    let mut name = curve.name.clone();
                    if ui.text_edit_singleline(&mut name).changed() {
                        curve.name = name;
                    }
                    if show_curve_editor(ui, curve, live_temp) {
                        self.profile_status = Some("Curve edited (Save to persist)".into());
                    }
                } else {
                    ui.label("No curve selected");
                }
            });
        });
    }

    fn apply_curves_from_snapshot(&mut self, snap: &crate::poll::Snapshot) {
        let mut temps: HashMap<String, f64> = snap
            .temps
            .iter()
            .map(|(id, _, v)| (id.clone(), *v))
            .collect();
        if let Some(t) = snap.cpu_temp {
            temps.entry("pawnio.0.temp.CPU".into()).or_insert(t);
        }
        let step = evaluate_profile_step(&self.profile, &temps, &mut self.curve_states);
        for (ctrl, duty) in step.duties {
            if self.is_user_locked(&ctrl) {
                continue;
            }
            if self.last_applied_duty.get(&ctrl) == Some(&duty) {
                continue;
            }
            self.writes.enqueue(&ctrl, duty);
            self.last_applied_duty.insert(ctrl.clone(), duty);
            self.slider_state.insert(ctrl, f32::from(duty));
        }
        for e in step.errors {
            tracing::debug!(error = %e, "curve apply");
        }
    }

    fn begin_rename(&mut self, id: &str, current: &str, is_control: bool) {
        self.rename_id = Some(id.to_string());
        self.rename_buf = current.to_string();
        self.rename_is_control = is_control;
    }

    fn show_rename_modal(&mut self, ctx: &egui::Context) {
        let Some(id) = self.rename_id.clone() else {
            return;
        };
        let mut open = true;
        egui::Window::new("Renommer")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .open(&mut open)
            .show(ctx, |ui| {
                ui.label(&id);
                ui.text_edit_singleline(&mut self.rename_buf);
                ui.horizontal(|ui| {
                    if ui.button("Enregistrer").clicked() {
                        let name = self.rename_buf.trim().to_string();
                        if !name.is_empty() {
                            if let Ok(mut map) = self.map.lock() {
                                if self.rename_is_control {
                                    map.set_control_name(&id, &name);
                                } else {
                                    map.set_sensor_name(&id, &name);
                                }
                                let _ = map.save();
                            }
                        }
                        self.rename_id = None;
                    }
                    if ui.button("Annuler").clicked() {
                        self.rename_id = None;
                    }
                });
            });
        if !open {
            self.rename_id = None;
        }
    }

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

    fn queue_write(&mut self, id: &str, duty: f32) {
        let percent = duty.round().clamp(0.0, 100.0) as u8;
        self.writes.enqueue(id, percent);
        self.last_applied_duty.insert(id.to_string(), percent);
    }
}
