//! egui application: live sensors, sliders, graph, rename, options.

use crate::activity::{show_activity_deck, ActivityDeckView, ActivityMode};
use crate::curve_editor::show_curve_editor;
use crate::graph::{show_temp_graph, GraphSeries, TempHistory, ThermalSignal};
use crate::i18n::{display_name_for, resolve_startup_locale, SUPPORTED};
use crate::poll::{spawn_poller, SharedMap, SharedSnapshot};
use crate::registry::{backend_status, build_registry, BackendStatus};
use crate::settings::{UiSettings, SHADER_FPS_ALLOWED};
use crate::shaders::{show_shader_panel, GraphStyle, ShaderGallery};
use crate::tray::{AppTray, TrayCommand, TrayState};
use crate::update_check::{UpdateChecker, UpdateStatus};
use crate::write_queue::WriteQueue;
use crate::UiError;
use eframe::egui;
use fancontrol_core::{
    evaluate_profile_step, list_profiles, load_profile, save_profile, ChannelMap, CurveEvalState,
    FanCurve, Profile,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
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
    let mut settings = UiSettings::load();
    let locale = resolve_startup_locale(&settings);
    rust_i18n::set_locale(&locale);
    if settings.language.is_none() {
        settings.language = Some(locale);
        settings.save();
    }
    let built = build_registry(
        options.include_mock,
        options.include_hw,
        options.allow_hw_write,
        settings.show_host_sensors,
    );
    let host_enabled = built.host_enabled;
    let reg = Arc::new(built.reg);
    let map = Arc::new(Mutex::new(ChannelMap::load_or_seed().unwrap_or_default()));
    let status = backend_status(options.include_hw);
    let snapshot = spawn_poller(
        Arc::clone(&reg),
        built.pawnio,
        Arc::clone(&map),
        Duration::from_millis(750),
    );
    let writes = WriteQueue::start(Arc::clone(&reg));
    let profile = load_or_create_default_profile(settings.last_profile_id.as_deref());
    if settings.last_profile_id.as_deref() != Some(profile.id.as_str()) {
        settings.last_profile_id = Some(profile.id.as_str().to_string());
        settings.save();
    }
    let pawnio_dialog = detect_pawnio_dialog(options.include_hw);
    // First-run writes consent only when the process actually allows PWM.
    let show_writes_consent = options.allow_hw_write && !settings.writes_risk_acknowledged;

    // Activity deck: sample only while the panel is enabled (default on).
    fancontrol_plugins::cpu_activity::set_enabled(settings.show_activity_deck);
    fancontrol_plugins::cpu_activity::set_sample_processes(
        settings.show_activity_deck && !matches!(settings.activity_mode, ActivityMode::LoadOnly),
    );

    let mut load_history = TempHistory::default();
    load_history.configure(settings.activity_window_minutes, 1);

    let app = FanApp {
        options,
        map,
        snapshot,
        writes,
        status,
        settings,
        host_enabled,
        slider_state: HashMap::new(),
        user_lock_until: HashMap::new(),
        write_error: None,
        rename_id: None,
        rename_buf: String::new(),
        rename_is_control: false,
        histories: HashMap::new(),
        graph_axis_max: None,
        load_history,
        activity_filter: String::new(),
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
        show_writes_consent,
        tray: None,
        really_exit: false,
        updates: UpdateChecker::new(),
        shader_clock: Instant::now(),
        shader_backend_available: false,
        window_visible: true,
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
        Box::new(move |cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());

            // CJK glyph fallback (egui's default fonts have no Chinese/Japanese coverage).
            // Pushed after the default fonts so Latin-script languages keep using those,
            // and loaded unconditionally since the language can be switched live at runtime.
            let mut fonts = egui::FontDefinitions::default();
            fonts.font_data.insert(
                "noto_sans_cjk".to_owned(),
                std::sync::Arc::new(egui::FontData::from_static(include_bytes!(
                    "../assets/fonts/NotoSansCJK-Regular.ttc"
                ))),
            );
            fonts
                .families
                .entry(egui::FontFamily::Proportional)
                .or_default()
                .push("noto_sans_cjk".to_owned());
            fonts
                .families
                .entry(egui::FontFamily::Monospace)
                .or_default()
                .push("noto_sans_cjk".to_owned());
            cc.egui_ctx.set_fonts(fonts);

            // One-time setup for the shader graph gallery's wgpu pipelines
            // (see crates/fancontrol-ui/src/shaders/mod.rs). Skipped gracefully
            // if the wgpu backend isn't active — Classic graph still works.
            let shader_backend_available = if let Some(render_state) = &cc.wgpu_render_state {
                let gallery = ShaderGallery::new(&render_state.device, render_state.target_format);
                render_state
                    .renderer
                    .write()
                    .callback_resources
                    .insert(gallery);
                true
            } else {
                tracing::warn!("wgpu render state unavailable: shader graph styles disabled");
                false
            };

            let mut app = app;
            app.shader_backend_available = shader_backend_available;
            // Must build after the event loop has started (tray-icon requirement).
            match AppTray::new() {
                Ok(tray) => app.tray = Some(tray),
                Err(e) => tracing::warn!("system tray unavailable: {e}"),
            }
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
    status: BackendStatus,
    settings: UiSettings,
    /// Live gate for host GPU/SSD (Options toggle).
    host_enabled: Arc<AtomicBool>,
    slider_state: HashMap<String, f32>,
    user_lock_until: HashMap<String, Instant>,
    write_error: Option<String>,
    rename_id: Option<String>,
    rename_buf: String,
    rename_is_control: bool,
    /// One history per selected sensor id (`settings.graph_sensor_ids`), lazily
    /// created/dropped as the selection changes.
    histories: HashMap<String, TempHistory>,
    /// Shared Y-axis smoothing state for the graph (eases toward a new max
    /// instead of jumping in one frame when the rolling window prunes a hot
    /// sample). Lives on `FanApp`, not `TempHistory`, since the axis is
    /// shared across every plotted series.
    graph_axis_max: Option<f32>,
    /// CPU load % history for the Activity deck.
    load_history: TempHistory,
    /// Process name filter (Activity deck).
    activity_filter: String,
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
    /// First-run modal: user must acknowledge PWM control risk.
    show_writes_consent: bool,
    tray: Option<AppTray>,
    /// Set when the tray "Exit" item fires, so the close-request handler lets it through
    /// instead of minimizing to tray.
    really_exit: bool,
    updates: UpdateChecker,
    shader_clock: Instant,
    /// Whether the wgpu backend (and thus any shader graph style) is available.
    shader_backend_available: bool,
    /// Tracks minimize-to-tray so a shader style's fast repaint doesn't run while hidden.
    window_visible: bool,
}

fn load_or_create_default_profile(preferred: Option<&str>) -> Profile {
    if let Some(id) = preferred {
        if let Ok(p) = load_profile(id) {
            return p;
        }
    }
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
        // Smooth shader animation needs a much faster repaint cadence than the
        // rest of the UI — only pay for it while a shader style is actually
        // active, the backend supports it, and the window isn't minimized to tray.
        let repaint_interval = if self.settings.graph_style.is_shader()
            && self.shader_backend_available
            && self.window_visible
        {
            Duration::from_secs_f32(1.0 / f32::from(self.settings.shader_fps))
        } else {
            Duration::from_millis(200)
        };
        ctx.request_repaint_after(repaint_interval);
        self.handle_tray(ctx);

        if self.tray.is_some() && !self.really_exit && ctx.input(|i| i.viewport().close_requested())
        {
            // Minimize to tray instead of exiting, unless "Exit" was chosen from the tray menu.
            // Without a tray icon there'd be no way to bring the window back, so skip this
            // entirely when the tray failed to initialize (rare — e.g. shell explorer.exe issues).
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(false));
            self.window_visible = false;
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        // Drain write-queue outcomes before applying more curve steps.
        for (id, duty) in self.writes.take_successes() {
            self.last_applied_duty.insert(id, duty);
            self.write_error = None;
        }
        for id in self.writes.take_failures() {
            self.last_applied_duty.remove(&id);
        }
        if let Some(e) = self.writes.take_error() {
            self.write_error = Some(e);
        }

        let snap = self.snapshot.lock().map(|g| g.clone()).unwrap_or_default();

        // One-shot seed: CPU/GPU (when known) as the initial multi-sensor graph selection.
        // `graph_sensor_ids_seeded` must start false for new installs (see settings Default).
        if !self.settings.graph_sensor_ids_seeded && (snap.tick > 0 || !snap.temps.is_empty()) {
            let mut seed = Vec::new();
            if let Some(id) = &snap.cpu_temp_id {
                seed.push(id.clone());
            }
            if let Some(id) = &snap.gpu_temp_id {
                seed.push(id.clone());
            }
            // Fallback: first available temp if CPU id not yet labeled
            if seed.is_empty() {
                if let Some((id, _, _)) = snap.temps.first() {
                    seed.push(id.clone());
                }
            }
            self.settings.graph_sensor_ids = seed;
            self.settings.graph_sensor_ids_seeded = true;
            self.settings.save();
        }

        let live_temps: HashMap<&str, f64> = snap
            .temps
            .iter()
            .map(|(id, _, v)| (id.as_str(), *v))
            .collect();
        let (win, samp) = (
            self.settings.graph_window_minutes,
            self.settings.graph_sample_secs,
        );
        for id in &self.settings.graph_sensor_ids {
            if let Some(&v) = live_temps.get(id.as_str()) {
                self.histories
                    .entry(id.clone())
                    .or_insert_with(|| {
                        let mut h = TempHistory::default();
                        h.configure(win, samp);
                        h
                    })
                    .push_if_due(v as f32, Instant::now());
            }
        }
        // Activity: one snapshot per frame; history configure only when window settings change
        // (done in Options / graph controls). Here we only push samples.
        let activity_snap = if self.settings.show_activity_deck {
            Some(fancontrol_plugins::cpu_activity::snapshot())
        } else {
            None
        };
        if let Some(act) = &activity_snap {
            if let Some(load) = act.load_pct {
                self.load_history.push_if_due(load as f32, Instant::now());
            }
        }
        self.histories
            .retain(|id, _| self.settings.graph_sensor_ids.contains(id));

        // Auto-apply curves ~1 Hz when enabled + write allowed + consent accepted
        if self.settings.auto_apply_curves
            && self.options.allow_hw_write
            && !self.show_writes_consent
            && self.last_curve_apply.elapsed() >= Duration::from_millis(1000)
        {
            self.apply_curves_from_snapshot(&snap);
            self.last_curve_apply = Instant::now();
        }

        egui::Panel::top("top").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.heading("fancontrol-rs");
                ui.separator();
                let write_capable =
                    self.options.allow_hw_write && matches!(self.status, BackendStatus::Ok(_));
                if write_capable {
                    ui.colored_label(
                        egui::Color32::LIGHT_GREEN,
                        t!("top_bar.write_enabled").to_string(),
                    );
                } else {
                    let hint = if !self.options.allow_hw_write {
                        Some(t!("top_bar.write_disabled_flag_hint").to_string())
                    } else {
                        match &self.status {
                            BackendStatus::NeedsAdmin => {
                                Some(t!("top_bar.write_disabled_admin_hint").to_string())
                            }
                            BackendStatus::NotInstalled => {
                                Some(t!("top_bar.write_disabled_pawnio_hint").to_string())
                            }
                            BackendStatus::Disabled => {
                                Some(t!("top_bar.write_disabled_probe_hint").to_string())
                            }
                            BackendStatus::Ok(_) => None,
                        }
                    };
                    let resp = ui
                        .colored_label(egui::Color32::YELLOW, t!("top_bar.read_only").to_string());
                    if let Some(hint) = hint {
                        resp.on_hover_text(hint);
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("⚙ {}", t!("top_bar.options_button")))
                        .clicked()
                    {
                        self.show_settings = !self.show_settings;
                    }
                    if ui
                        .selectable_label(self.show_curves, t!("top_bar.curves_toggle").to_string())
                        .on_hover_text(t!("top_bar.curves_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.show_curves = !self.show_curves;
                    }
                    // Prominent Curve control toggle (auto-apply to hardware)
                    let curve_on = self.settings.auto_apply_curves;
                    let (label, fill, text_color) = if curve_on {
                        (
                            t!("top_bar.curve_control_on").to_string(),
                            egui::Color32::from_rgb(30, 90, 50),
                            egui::Color32::from_rgb(140, 255, 170),
                        )
                    } else {
                        (
                            t!("top_bar.curve_control_off").to_string(),
                            egui::Color32::from_rgb(70, 40, 40),
                            egui::Color32::from_rgb(220, 160, 160),
                        )
                    };
                    let btn =
                        egui::Button::new(egui::RichText::new(label).color(text_color).strong())
                            .fill(fill);
                    if ui
                        .add(btn)
                        .on_hover_text(t!("top_bar.curve_control_tooltip").to_string())
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
                    t!("top_bar.curve_readonly_warning").to_string(),
                );
            }
            let status_text = match &self.status {
                BackendStatus::Disabled => t!("registry.hw_probe_disabled").to_string(),
                BackendStatus::Ok(detail) => t!("registry.pawnio_ok", detail = detail).to_string(),
                BackendStatus::NeedsAdmin => t!("registry.needs_admin").to_string(),
                BackendStatus::NotInstalled => t!("registry.not_installed").to_string(),
            };
            ui.label(status_text);
            if let Some(err) = &snap.error {
                ui.colored_label(
                    egui::Color32::YELLOW,
                    t!("top_bar.poll_error", error = err).to_string(),
                );
            }
            if let Some(err) = &self.write_error {
                ui.colored_label(
                    egui::Color32::RED,
                    t!("top_bar.write_error", error = err).to_string(),
                );
            }
            if !self.options.allow_hw_write {
                ui.small(t!("top_bar.sliders_locked").to_string());
            }
            ui.small(
                t!(
                    "top_bar.debug_counts",
                    temps = snap.temps.len(),
                    fans = snap.fans.len(),
                    controls = snap.controls.len(),
                    tick = snap.tick
                )
                .to_string(),
            );
        });

        if self.show_settings {
            egui::Panel::right("settings")
                .resizable(true)
                .default_size(260.0)
                .show(ui, |ui| {
                    ui.heading(t!("options.heading").to_string());
                    ui.separator();
                    let mut dirty = false;
                    dirty |= ui
                        .checkbox(
                            &mut self.settings.hide_zero_rpm,
                            t!("options.hide_zero_rpm").to_string(),
                        )
                        .changed();
                    dirty |= ui
                        .checkbox(
                            &mut self.settings.hide_zero_duty_controls,
                            t!("options.hide_zero_duty_controls").to_string(),
                        )
                        .changed();
                    dirty |= ui
                        .checkbox(
                            &mut self.settings.show_graph_panel,
                            t!("options.show_cpu_graph").to_string(),
                        )
                        .changed();
                    if ui
                        .checkbox(
                            &mut self.settings.show_activity_deck,
                            t!("options.show_activity_deck").to_string(),
                        )
                        .changed()
                    {
                        fancontrol_plugins::cpu_activity::set_enabled(
                            self.settings.show_activity_deck,
                        );
                        fancontrol_plugins::cpu_activity::set_sample_processes(
                            self.settings.show_activity_deck
                                && !matches!(self.settings.activity_mode, ActivityMode::LoadOnly),
                        );
                        dirty = true;
                    }
                    if self.settings.show_activity_deck {
                        ui.indent("activity_opts", |ui| {
                            ui.horizontal(|ui| {
                                ui.label(t!("options.activity_mode").to_string());
                                for (mode, key) in [
                                    (ActivityMode::Both, "options.activity_mode_both"),
                                    (ActivityMode::LoadOnly, "options.activity_mode_load"),
                                    (ActivityMode::ProcessesOnly, "options.activity_mode_procs"),
                                ] {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.activity_mode,
                                            mode,
                                            t!(key).to_string(),
                                        )
                                        .changed()
                                    {
                                        fancontrol_plugins::cpu_activity::set_sample_processes(
                                            !matches!(mode, ActivityMode::LoadOnly),
                                        );
                                        dirty = true;
                                    }
                                }
                            });
                            ui.horizontal(|ui| {
                                ui.label(t!("options.activity_top_n").to_string());
                                for n in [5_u8, 8, 10, 12, 16, 20] {
                                    if ui
                                        .selectable_value(
                                            &mut self.settings.activity_top_n,
                                            n,
                                            n.to_string(),
                                        )
                                        .changed()
                                    {
                                        dirty = true;
                                    }
                                }
                            });
                        });
                    }
                    if ui
                        .checkbox(
                            &mut self.settings.show_host_sensors,
                            t!("options.show_host_sensors").to_string(),
                        )
                        .changed()
                    {
                        self.host_enabled
                            .store(self.settings.show_host_sensors, Ordering::Relaxed);
                        dirty = true;
                    }
                    dirty |= ui
                        .checkbox(
                            &mut self.settings.auto_apply_curves,
                            t!("options.auto_apply_curves").to_string(),
                        )
                        .changed();
                    if self.settings.auto_apply_curves && !self.options.allow_hw_write {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            t!("options.auto_apply_needs_write").to_string(),
                        );
                    }
                    ui.small(t!("options.host_sensor_note").to_string());
                    ui.separator();
                    ui.label(t!("options.rgb_heading").to_string());
                    ui.small(t!("options.rgb_note").to_string());
                    ui.separator();
                    ui.label(t!("options.names_heading").to_string());
                    ui.small(t!("options.names_note").to_string());
                    ui.separator();
                    ui.label(t!("options.updates_heading").to_string());
                    if ui
                        .button(t!("options.check_updates_button").to_string())
                        .clicked()
                    {
                        self.updates.check_now();
                    }
                    match self.updates.status() {
                        Some(UpdateStatus::Checking) => {
                            ui.small(t!("options.checking").to_string());
                        }
                        Some(UpdateStatus::UpToDate) => {
                            ui.small(
                                t!("options.up_to_date", version = env!("CARGO_PKG_VERSION"))
                                    .to_string(),
                            );
                        }
                        Some(UpdateStatus::Available { version, url }) => {
                            ui.colored_label(
                                egui::Color32::LIGHT_GREEN,
                                t!("options.new_version_available", version = version).to_string(),
                            );
                            ui.hyperlink_to(t!("options.open_release_page").to_string(), url);
                        }
                        Some(UpdateStatus::Error(e)) => {
                            ui.colored_label(
                                egui::Color32::YELLOW,
                                t!("options.check_failed", error = e).to_string(),
                            );
                        }
                        None => {}
                    }
                    ui.separator();
                    ui.label(t!("options.language_heading").to_string());
                    let current_lang = self
                        .settings
                        .language
                        .clone()
                        .unwrap_or_else(|| "en".to_string());
                    egui::ComboBox::from_id_salt("language_pick")
                        .selected_text(display_name_for(&current_lang))
                        .show_ui(ui, |ui| {
                            for code in SUPPORTED {
                                let selected = current_lang == code;
                                if ui
                                    .selectable_label(selected, display_name_for(code))
                                    .clicked()
                                    && !selected
                                {
                                    self.settings.language = Some(code.to_string());
                                    rust_i18n::set_locale(code);
                                    if let Some(tray) = &self.tray {
                                        tray.retranslate();
                                    }
                                    self.settings.save();
                                }
                            }
                        });
                    ui.separator();
                    ui.label(t!("options.graph_style_heading").to_string());
                    let current_style = self.settings.graph_style;
                    egui::ComboBox::from_id_salt("graph_style_pick")
                        .selected_text(t!(current_style.display_key()).to_string())
                        .show_ui(ui, |ui| {
                            for style in GraphStyle::ALL {
                                let enabled =
                                    style == GraphStyle::Classic || self.shader_backend_available;
                                let selected = current_style == style;
                                ui.add_enabled_ui(enabled, |ui| {
                                    if ui
                                        .selectable_label(
                                            selected,
                                            t!(style.display_key()).to_string(),
                                        )
                                        .on_disabled_hover_text(
                                            t!("options.shader_unavailable").to_string(),
                                        )
                                        .clicked()
                                        && !selected
                                    {
                                        self.settings.graph_style = style;
                                        self.settings.save();
                                    }
                                });
                            }
                        });
                    if self.settings.graph_style.is_shader() {
                        ui.colored_label(
                            egui::Color32::YELLOW,
                            t!("options.shader_gpu_warning").to_string(),
                        );
                        dirty |= ui
                            .add(
                                egui::Slider::new(&mut self.settings.shader_speed, 0.0..=3.0)
                                    .text(t!("options.shader_speed").to_string()),
                            )
                            .changed();
                        ui.horizontal(|ui| {
                            ui.label(t!("options.fps_label").to_string());
                            for fps in SHADER_FPS_ALLOWED {
                                let selected = self.settings.shader_fps == fps;
                                let label = if fps >= 90 {
                                    format!("{fps} ⚠")
                                } else {
                                    format!("{fps}")
                                };
                                let resp = ui.selectable_label(selected, label);
                                let resp = if fps >= 90 {
                                    resp.on_hover_text(t!("options.fps_high_usage").to_string())
                                } else {
                                    resp
                                };
                                if resp.clicked() && !selected {
                                    self.settings.shader_fps = fps;
                                    dirty = true;
                                }
                            }
                        });
                        ui.horizontal(|ui| {
                            ui.label(t!("options.fractal_color_a").to_string());
                            dirty |= ui
                                .color_edit_button_rgb(&mut self.settings.shader_color_a)
                                .changed();
                            ui.label(t!("options.fractal_color_b").to_string());
                            dirty |= ui
                                .color_edit_button_rgb(&mut self.settings.shader_color_b)
                                .changed();
                        });
                    }
                    ui.separator();
                    ui.label(t!("options.graph_sensors_heading").to_string());
                    ui.small(t!("options.graph_sensors_note").to_string());
                    if snap.temps.is_empty() {
                        ui.small(t!("dashboard.none").to_string());
                    } else {
                        for (id, label, _) in &snap.temps {
                            let mut checked =
                                self.settings.graph_sensor_ids.iter().any(|s| s == id);
                            if ui.checkbox(&mut checked, label.as_str()).changed() {
                                if checked {
                                    self.settings.graph_sensor_ids.push(id.clone());
                                } else {
                                    self.settings.graph_sensor_ids.retain(|s| s != id);
                                }
                                self.settings.save();
                            }
                        }
                    }
                    if self.settings.graph_sensor_ids.len() > 6 {
                        ui.small(t!("options.graph_sensors_many_note").to_string());
                    }
                    if dirty {
                        self.settings.clamp_graph_options();
                        self.settings.save();
                    }
                    if ui.button(t!("common.close").to_string()).clicked() {
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

        let show_thermal = self.settings.show_graph_panel;
        let show_activity = self.settings.show_activity_deck;
        if show_thermal || show_activity {
            let labels: HashMap<&str, &str> = snap
                .temps
                .iter()
                .map(|(id, label, _)| (id.as_str(), label.as_str()))
                .collect();

            // Prefer one solid top strip: thermal alone ~240, activity alone ~220,
            // both stacked ~420 so neither plot collapses to 0 height.
            let default_h = match (show_thermal, show_activity) {
                (true, true) => 420.0,
                (true, false) => 240.0,
                (false, true) => 220.0,
                (false, false) => 0.0,
            };
            let min_h = match (show_thermal, show_activity) {
                (true, true) => 320.0,
                (true, false) => 180.0,
                (false, true) => 160.0,
                (false, false) => 0.0,
            };

            egui::Panel::top("graph_area")
                .resizable(true)
                .default_size(default_h)
                .min_size(min_h)
                .max_size(800.0)
                .show(ui, |ui| {
                    if show_thermal {
                        self.ui_graph_controls(ui);
                        // Build series for every selected id (create empty history if needed
                        // so the plot frame always shows instead of a blank panel).
                        let (win, samp) = (
                            self.settings.graph_window_minutes,
                            self.settings.graph_sample_secs,
                        );
                        for id in &self.settings.graph_sensor_ids {
                            self.histories.entry(id.clone()).or_insert_with(|| {
                                let mut h = TempHistory::default();
                                h.configure(win, samp);
                                h
                            });
                        }
                        let series: Vec<GraphSeries> = self
                            .settings
                            .graph_sensor_ids
                            .iter()
                            .enumerate()
                            .filter_map(|(i, id)| {
                                self.histories.get(id).map(|h| GraphSeries {
                                    label: labels.get(id.as_str()).copied().unwrap_or(id.as_str()),
                                    palette_index: i,
                                    history: h,
                                })
                            })
                            .collect();
                        let style = self.settings.graph_style;
                        // Explicit plot height — do NOT use allocate_ui(available_height)
                        // which can leave egui_plot with an unbounded/zero rect.
                        let plot_h = if show_activity {
                            (ui.available_height() * 0.5).clamp(140.0, 280.0)
                        } else {
                            ui.available_height().clamp(140.0, 480.0)
                        };
                        if style == GraphStyle::Classic || !self.shader_backend_available {
                            show_temp_graph(
                                ui,
                                &series,
                                self.settings.graph_window_minutes,
                                &mut self.graph_axis_max,
                                plot_h,
                            );
                            if style.is_shader() {
                                ui.small(t!("graph.shader_fallback_note").to_string());
                            }
                        } else {
                            let t = self.shader_clock.elapsed().as_secs_f32()
                                * self.settings.shader_speed;
                            let readings: Vec<(String, f32)> = series
                                .iter()
                                .filter_map(|s| s.history.last().map(|v| (s.label.to_string(), v)))
                                .collect();
                            let signal = ThermalSignal::from_readings(readings);
                            // Shader panel needs a sized rect too.
                            ui.allocate_ui(egui::vec2(ui.available_width(), plot_h), |ui| {
                                show_shader_panel(
                                    ui,
                                    style,
                                    t,
                                    signal,
                                    self.settings.shader_color_a,
                                    self.settings.shader_color_b,
                                );
                            });
                        }
                    }

                    if show_activity {
                        if show_thermal {
                            ui.separator();
                        }
                        // Reuse snapshot taken at start of frame (no second clone).
                        let act = activity_snap.as_ref().cloned().unwrap_or_default();
                        let sort_before = self.settings.activity_sort;
                        let act_h = ui.available_height().clamp(140.0, 320.0);
                        ui.allocate_ui(egui::vec2(ui.available_width(), act_h), |ui| {
                            show_activity_deck(
                                ui,
                                ActivityDeckView {
                                    load_history: &self.load_history,
                                    processes: &act.processes,
                                    load_pct: act.load_pct,
                                    mode: self.settings.activity_mode,
                                    sort: &mut self.settings.activity_sort,
                                    filter: &mut self.activity_filter,
                                    top_n: self.settings.activity_top_n as usize,
                                    window_minutes: self.settings.activity_window_minutes,
                                },
                            );
                        });
                        if self.settings.activity_sort != sort_before {
                            self.settings.save();
                        }
                    }
                });
        }

        egui::CentralPanel::default().show(ui, |ui| {
            ui.columns(3, |cols| {
                // Temps
                cols[0].heading(t!("dashboard.temperatures").to_string());
                cols[0].separator();
                egui::ScrollArea::vertical()
                    .id_salt("temps")
                    .show(&mut cols[0], |ui| {
                        if snap.temps.is_empty() {
                            ui.label(t!("dashboard.none").to_string());
                        }
                        for (id, label, v) in &snap.temps {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(label.as_str())
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(t!("dashboard.click_to_rename").to_string())
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
                cols[1].heading(t!("dashboard.fans").to_string());
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
                            ui.label(t!("dashboard.none").to_string());
                        }
                        for (id, label, v) in fans {
                            ui.horizontal(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(label.as_str())
                                            .sense(egui::Sense::click()),
                                    )
                                    .on_hover_text(t!("dashboard.click_to_rename").to_string())
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
                cols[2].heading(t!("dashboard.controls").to_string());
                cols[2].separator();
                egui::ScrollArea::vertical()
                    .id_salt("ctrls")
                    .show(&mut cols[2], |ui| {
                        // hide_zero_rpm only affects the Fans list above; this is a
                        // separate, opt-in filter based on duty (not rpm, since many
                        // controls legitimately have no rpm sensor at all). `duty: None`
                        // (unknown/unsupported readback) is treated as visible: hiding an
                        // interactive control on missing data is worse than showing a stale one.
                        let controls: Vec<_> = snap
                            .controls
                            .iter()
                            .filter(|c| {
                                !self.settings.hide_zero_duty_controls || c.duty.unwrap_or(1) >= 1
                            })
                            .collect();
                        if controls.is_empty() {
                            ui.label(t!("dashboard.none").to_string());
                        }
                        for c in controls {
                            ui.group(|ui| {
                                if ui
                                    .add(
                                        egui::Label::new(c.label.as_str())
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
                                                // Keep existing sensor binding; only default when missing.
                                                self.profile
                                                    .sensor_bindings
                                                    .entry(c.id.clone())
                                                    .or_insert_with(|| {
                                                        snap.cpu_temp_id.clone().unwrap_or_else(
                                                            || "pawnio.0.temp.CPU".into(),
                                                        )
                                                    });
                                            }
                                        }
                                    });

                                if self.profile.assignments.contains_key(&c.id) {
                                    let bound_id = self
                                        .profile
                                        .sensor_bindings
                                        .get(&c.id)
                                        .cloned()
                                        .unwrap_or_else(|| "pawnio.0.temp.CPU".to_string());
                                    let bound_label = snap
                                        .temps
                                        .iter()
                                        .find(|(id, _, _)| id == &bound_id)
                                        .map(|(_, label, _)| label.clone())
                                        .unwrap_or_else(|| bound_id.clone());
                                    let bind_resp =
                                        egui::ComboBox::from_id_salt(format!("bind-{}", c.id))
                                            .selected_text(bound_label)
                                            .show_ui(ui, |ui| {
                                                for (id, label, _) in &snap.temps {
                                                    let selected = id == &bound_id;
                                                    if ui
                                                        .selectable_label(selected, label.as_str())
                                                        .clicked()
                                                        && !selected
                                                    {
                                                        self.profile
                                                            .sensor_bindings
                                                            .insert(c.id.clone(), id.clone());
                                                    }
                                                }
                                            });
                                    bind_resp.response.on_hover_text(
                                        t!("dashboard.curve_sensor_hover").to_string(),
                                    );
                                }

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
                                    && !self.show_writes_consent
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
                                    ui.small(t!("dashboard.locked").to_string());
                                }
                            });
                            ui.add_space(6.0);
                        }
                    });
            });
        });

        self.show_rename_modal(&ctx);
        // Writes consent first (blocks PWM until answered); then PawnIO help if needed.
        self.show_writes_consent_dialog(&ctx);
        if !self.show_writes_consent {
            self.show_pawnio_dialog(&ctx);
        }
    }
}

impl FanApp {
    fn ui_graph_controls(&mut self, ui: &mut egui::Ui) {
        let mut dirty = false;
        ui.horizontal(|ui| {
            ui.label(t!("graph_controls.window_label").to_string());
            for m in GRAPH_WINDOWS {
                let selected = self.settings.graph_window_minutes == m;
                if ui.selectable_label(selected, format!("{m}m")).clicked() && !selected {
                    self.settings.graph_window_minutes = m;
                    dirty = true;
                }
            }
            ui.separator();
            ui.label(t!("graph_controls.sample_label").to_string());
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
            for h in self.histories.values_mut() {
                h.configure(
                    self.settings.graph_window_minutes,
                    self.settings.graph_sample_secs,
                );
            }
            self.load_history.configure(
                self.settings.activity_window_minutes,
                1, // activity worker ~1 Hz
            );
        }
    }

    fn show_writes_consent_dialog(&mut self, ctx: &egui::Context) {
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
                        self.options.allow_hw_write = false;
                        self.settings.auto_apply_curves = false;
                        self.show_writes_consent = false;
                    }
                });
            });
    }

    fn show_pawnio_dialog(&mut self, ctx: &egui::Context) {
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
                ui.add_space(10.0);
                ui.horizontal(|ui| {
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

    fn ui_curves_panel(&mut self, ui: &mut egui::Ui, live_temp: Option<f64>) {
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
                        {
                            if let Ok(p) = load_profile(&id) {
                                self.profile = p;
                                self.selected_curve = 0;
                                self.curve_states.clear();
                                self.profile_status =
                                    Some(t!("curves_panel.loaded_status", id = id).to_string());
                                self.settings.last_profile_id = Some(id.clone());
                                self.settings.save();
                            }
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
                    let id = format!("curve{}", self.profile.curves.len() + 1);
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
            });
            ui.separator();
            ui.vertical(|ui| {
                if let Some(curve) = self.profile.curves.get_mut(self.selected_curve) {
                    let mut name = curve.name.clone();
                    if ui.text_edit_singleline(&mut name).changed() {
                        curve.name = name;
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

    fn handle_tray(&mut self, ctx: &egui::Context) {
        let Some(tray) = &self.tray else { return };
        let commands = tray.poll_commands();

        let state = if self.pawnio_dialog.is_some() {
            TrayState::Error
        } else {
            let snap_err = self
                .snapshot
                .lock()
                .map(|g| g.error.is_some())
                .unwrap_or(false);
            if snap_err || self.write_error.is_some() {
                TrayState::Warning
            } else {
                TrayState::Normal
            }
        };
        if let Some(tray) = &mut self.tray {
            tray.set_state(state);
        }

        for cmd in commands {
            match cmd {
                TrayCommand::Open => {
                    ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
                    ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                    self.window_visible = true;
                }
                TrayCommand::ApplyDefaultProfile => {
                    let snap = self.snapshot.lock().map(|g| g.clone()).unwrap_or_default();
                    self.apply_curves_from_snapshot(&snap);
                }
                TrayCommand::Exit => {
                    self.really_exit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
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
            // Do not mark applied until WriteQueue reports success (see take_successes).
            self.writes.enqueue(&ctrl, duty);
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
                    if ui.button(t!("common.cancel").to_string()).clicked() {
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
        if self.show_writes_consent {
            return;
        }
        if !self.options.allow_hw_write && !id.starts_with("mock.") {
            return;
        }
        let percent = duty.round().clamp(0.0, 100.0) as u8;
        // Optimistic UI skip only after queue success drain; clear on failure.
        self.last_applied_duty.remove(id);
        self.writes.enqueue(id, percent);
    }
}
