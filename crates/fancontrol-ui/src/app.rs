//! egui application: live sensors, sliders, graph, rename, options.

use crate::UiError;
use crate::activity::{ActivityDeckView, ActivityMode, show_activity_deck};
use crate::cpu_panel::show_cpu_panel;
use crate::curve_editor::show_curve_editor;
use crate::gpu_panel::show_gpu_panel;
use crate::graph::{GraphSeries, TempHistory, ThermalSignal, show_metric_graph};
use crate::i18n::{SUPPORTED, display_name_for, resolve_startup_locale};
use crate::poll::{SharedMap, SharedSnapshot, spawn_poller};
use crate::registry::{BackendStatus, backend_status, build_registry};
use crate::settings::{SHADER_FPS_ALLOWED, UiSettings};
use crate::shaders::{GraphStyle, ShaderGallery, show_shader_panel};
use crate::tray::{AppTray, TrayCommand, TrayState};
use crate::update_check::{UpdateChecker, UpdateStatus};
use crate::write_queue::WriteQueue;
use eframe::egui;
use fancontrol_core::{
    ChannelMap, CurveEvalState, FanCurve, MetricSample, Profile, SensorKind, evaluate_profile_step,
    is_cpu_temp_candidate, list_profiles, load_profile, save_profile,
};
use fancontrol_metrics::{
    MetricSink, OtlpSink, SqliteMetricsStore, SqliteStoreConfig, default_metrics_db_path,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const GRAPH_WINDOWS: [u16; 4] = [10, 20, 30, 60];
const GRAPH_SAMPLES: [u16; 4] = [1, 2, 5, 10];
const PAWNIO_URL: &str = "https://pawnio.eu";

/// `f32::clamp` panics when `lo > hi`. Layout heights are dynamic; always order bounds.
fn clamp_ui_height(v: f32, lo: f32, hi: f32) -> f32 {
    let min_b = lo.min(hi);
    let max_b = lo.max(hi);
    v.clamp(min_b, max_b)
}

/// ComboBox label: stored curve name, fallback to id if the name is empty.
fn curve_combo_label(curve: &FanCurve) -> &str {
    let n = curve.name.trim();
    if n.is_empty() {
        curve.id.as_str()
    } else {
        n
    }
}

/// Default curve sensor: live CPU seed, else first CPU-like temp, else NCT668x-style id.
fn default_cpu_curve_sensor(snap: &crate::poll::Snapshot) -> String {
    if let Some(id) = &snap.cpu_temp_id {
        return id.clone();
    }
    snap.temps
        .iter()
        .find(|(id, _, _)| is_cpu_temp_candidate(id))
        .map(|(id, _, _)| id.clone())
        .unwrap_or_else(|| "pawnio.0.temp.CPU".into())
}

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
    let mut cpu_power_history = TempHistory::default();
    cpu_power_history.configure(settings.graph_window_minutes, settings.graph_sample_secs);
    let mut gpu_power_history = TempHistory::default();
    gpu_power_history.configure(settings.graph_window_minutes, settings.graph_sample_secs);

    let metrics_sink = if settings.metrics_store_enabled {
        SqliteMetricsStore::spawn(SqliteStoreConfig {
            path: default_metrics_db_path()
                .unwrap_or_else(|| std::path::PathBuf::from("metrics.sqlite")),
            retention_days: u32::from(settings.metrics_retention_days.max(1)),
            flush_ms: 500,
        })
    } else {
        None
    };
    let otel_sink = if settings.otel_enabled {
        OtlpSink::spawn(settings.otel_endpoint.clone())
    } else {
        None
    };

    // Keep HKCU Run path in sync if the user already opted in.
    if settings.launch_on_startup {
        crate::autostart::refresh_if_enabled();
        if !crate::autostart::is_enabled() {
            // Setting says on but registry missing (e.g. cleaned by OS) - re-apply.
            if let Err(e) = crate::autostart::set_enabled(true) {
                tracing::warn!(error = %e, "failed to restore autostart registry entry");
            }
        }
    } else if crate::autostart::is_enabled() {
        // Registry has us but settings say off - reflect OS state into settings once.
        settings.launch_on_startup = true;
        settings.save();
    }
    let show_startup_prompt = !settings.startup_prompt_shown;

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
        graph_axis_max_secondary: None,
        metrics_sink,
        otel_sink,
        last_metrics_record: Instant::now() - Duration::from_secs(60),
        load_history,
        cpu_power_history,
        gpu_power_history,
        activity_filter: String::new(),
        show_settings: false,
        show_curves: true,
        show_temps: true,
        show_fans: true,
        show_controls: true,
        profile,
        profile_list: list_profiles().unwrap_or_default(),
        selected_curve: 0,
        curve_states: HashMap::new(),
        last_curve_apply: Instant::now() - Duration::from_secs(10),
        last_applied_duty: HashMap::new(),
        profile_status: None,
        new_profile_name: "default".into(),
        pawnio_dialog,
        elevate_status: None,
        show_writes_consent,
        show_startup_prompt,
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
            .with_title("Fancontrol-RS")
            .with_icon(icon),
        ..Default::default()
    };

    eframe::run_native(
        "Fancontrol-RS",
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
            // if the wgpu backend isn't active - Classic graph still works.
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
    /// Secondary Y-axis (second unit group: W, %, …).
    graph_axis_max_secondary: Option<f32>,
    /// Optional local SQLite metrics store (background writer).
    metrics_sink: Option<SqliteMetricsStore>,
    /// Optional OTLP/HTTP export (background sender).
    otel_sink: Option<OtlpSink>,
    last_metrics_record: Instant,
    /// CPU load % history for the Activity deck.
    load_history: TempHistory,
    /// CPU package power history for the CPU panel sparkline. Independent of
    /// `graph_sensor_ids` so it keeps tracking even when the Sensors graph
    /// filters power series out (see `ui_thermal_graph_block`).
    cpu_power_history: TempHistory,
    /// First-GPU power history for the GPU panel sparkline (see `cpu_power_history`).
    gpu_power_history: TempHistory,
    /// Process name filter (Activity deck).
    activity_filter: String,
    show_settings: bool,
    show_curves: bool,
    /// Session toggles for the three central columns (like `show_curves`).
    show_temps: bool,
    show_fans: bool,
    show_controls: bool,
    profile: Profile,
    profile_list: Vec<String>,
    selected_curve: usize,
    curve_states: HashMap<String, CurveEvalState>,
    last_curve_apply: Instant,
    last_applied_duty: HashMap<String, u8>,
    profile_status: Option<String>,
    new_profile_name: String,
    pawnio_dialog: Option<PawnioDialogKind>,
    /// Last elevation relaunch error (UAC cancel / ShellExecute failure).
    elevate_status: Option<String>,
    /// First-run modal: user must acknowledge PWM control risk.
    show_writes_consent: bool,
    /// First-run (or first after upgrade) "Start with Windows?" prompt.
    show_startup_prompt: bool,
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
    if let Some(id) = preferred
        && let Ok(p) = load_profile(id)
    {
        return p;
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
        // rest of the UI - only pay for it while a shader style is actually
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
            // entirely when the tray failed to initialize (rare - e.g. shell explorer.exe issues).
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
            // Package power belongs to the CPU panel, not the Sensors (temperature)
            // graph - do not seed `cpu_power_id` here (see `ui_thermal_graph_block`).
            // Fallback: first available temp if CPU id not yet labeled
            if seed.is_empty()
                && let Some((id, _, _)) = snap.temps.first()
            {
                seed.push(id.clone());
            }
            self.settings.graph_sensor_ids = seed;
            self.settings.graph_sensor_ids_seeded = true;
            self.settings.save();
        }

        let live_plot: HashMap<&str, f64> = snap
            .plottable
            .iter()
            .map(|p| (p.id.as_str(), p.value))
            .collect();
        let (win, samp) = (
            self.settings.graph_window_minutes,
            self.settings.graph_sample_secs,
        );
        for id in &self.settings.graph_sensor_ids {
            if let Some(&v) = live_plot.get(id.as_str()) {
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
        if let Some(w) = snap.cpu.power_w {
            self.cpu_power_history.push_if_due(w as f32, Instant::now());
        }
        if let Some(w) = snap.gpus.first().and_then(|g| g.power_w) {
            self.gpu_power_history.push_if_due(w as f32, Instant::now());
        }

        // Metrics store / OTEL (best-effort, separate cadence).
        if self.settings.metrics_store_enabled || self.settings.otel_enabled {
            let every = Duration::from_secs(u64::from(self.settings.metrics_sample_secs.max(1)));
            if self.last_metrics_record.elapsed() >= every {
                let ts_ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or(0);
                let batch: Vec<MetricSample> = snap
                    .plottable
                    .iter()
                    .map(|p| {
                        MetricSample::new(
                            p.id.clone(),
                            p.label.clone(),
                            p.kind,
                            p.unit.clone(),
                            p.value,
                            ts_ms,
                        )
                    })
                    .collect();
                if !batch.is_empty() {
                    if let Some(store) = self.metrics_sink.as_mut() {
                        store.record(&batch);
                    }
                    if let Some(otel) = self.otel_sink.as_mut() {
                        otel.record(&batch);
                    }
                }
                self.last_metrics_record = Instant::now();
            }
        }
        // Activity: one snapshot per frame; history configure only when window settings change
        // (done in Options / graph controls). Here we only push samples.
        let activity_snap = if self.settings.show_activity_deck {
            Some(fancontrol_plugins::cpu_activity::snapshot())
        } else {
            None
        };
        if let Some(act) = &activity_snap
            && let Some(load) = act.load_pct
        {
            self.load_history.push_if_due(load as f32, Instant::now());
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
                self.ui_graph_controls(ui);
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
                    // One-click UAC relaunch when PawnIO is installed but not openable.
                    if matches!(self.status, BackendStatus::NeedsAdmin)
                        && !crate::elevation::is_elevated()
                        && ui
                            .button(t!("pawnio.restart_as_admin").to_string())
                            .on_hover_text(t!("top_bar.write_disabled_admin_hint").to_string())
                            .clicked()
                    {
                        self.try_relaunch_elevated();
                    }
                }
                if let Some(msg) = &self.elevate_status {
                    ui.colored_label(egui::Color32::LIGHT_RED, msg);
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .button(format!("⚙ {}", t!("top_bar.options_button")))
                        .clicked()
                    {
                        self.show_settings = !self.show_settings;
                    }
                    // Updates: Options only (no top-bar button - clutter / unclear action).
                    // right-to-left: add Controls, Fans, Temps, then Curves
                    if ui
                        .selectable_label(
                            self.show_controls,
                            t!("top_bar.controls_toggle").to_string(),
                        )
                        .on_hover_text(t!("top_bar.controls_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.show_controls = !self.show_controls;
                    }
                    if ui
                        .selectable_label(self.show_fans, t!("top_bar.fans_toggle").to_string())
                        .on_hover_text(t!("top_bar.fans_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.show_fans = !self.show_fans;
                    }
                    if ui
                        .selectable_label(self.show_temps, t!("top_bar.temps_toggle").to_string())
                        .on_hover_text(t!("top_bar.temps_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.show_temps = !self.show_temps;
                    }
                    if ui
                        .selectable_label(
                            self.settings.show_cpu_panel,
                            t!("top_bar.cpu_toggle").to_string(),
                        )
                        .on_hover_text(t!("top_bar.cpu_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.settings.show_cpu_panel = !self.settings.show_cpu_panel;
                        self.settings.save();
                    }
                    if ui
                        .selectable_label(
                            self.settings.show_gpu_panel,
                            t!("top_bar.gpu_toggle").to_string(),
                        )
                        .on_hover_text(t!("top_bar.gpu_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.settings.show_gpu_panel = !self.settings.show_gpu_panel;
                        self.settings.save();
                    }
                    if ui
                        .selectable_label(
                            self.settings.show_graph_panel,
                            t!("top_bar.sensors_toggle").to_string(),
                        )
                        .on_hover_text(t!("top_bar.sensors_toggle_tooltip").to_string())
                        .clicked()
                    {
                        self.settings.show_graph_panel = !self.settings.show_graph_panel;
                        self.settings.save();
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
            // Backend detail + channel counts live in Options (less noise on the main strip).
            // Keep live errors here so failures stay visible.
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
        });

        if self.show_settings {
            egui::Panel::right("settings")
                .resizable(true)
                .default_size(300.0)
                .show(ui, |ui| {
                    ui.heading(t!("options.heading").to_string());
                    ui.separator();
                    ui.label(t!("options.backend_heading").to_string());
                    let status_text = match &self.status {
                        BackendStatus::Disabled => t!("registry.hw_probe_disabled").to_string(),
                        BackendStatus::Ok(detail) => {
                            t!("registry.pawnio_ok", detail = detail).to_string()
                        }
                        BackendStatus::NeedsAdmin => t!("registry.needs_admin").to_string(),
                        BackendStatus::NotInstalled => t!("registry.not_installed").to_string(),
                    };
                    ui.small(status_text);
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
                    ui.separator();

                    // Everything below can get long (graph sensors, metrics, updates, …);
                    // scroll it so the Close button below always stays reachable.
                    egui::ScrollArea::vertical()
                        .id_salt("options_scroll")
                        .auto_shrink([false, false])
                        .show(ui, |ui| {
                            egui::CollapsingHeader::new(t!("options.section_updates").to_string())
                                .default_open(true)
                                .show(ui, |ui| {
                                    ui.add_space(2.0);
                                    let big_button = egui::Button::new(
                                        egui::RichText::new(
                                            t!("options.check_updates_button").to_string(),
                                        )
                                        .size(16.0)
                                        .strong(),
                                    );
                                    if ui
                                        .add_sized([ui.available_width(), 36.0], big_button)
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
                                                t!(
                                                    "options.up_to_date",
                                                    version = env!("CARGO_PKG_VERSION")
                                                )
                                                .to_string(),
                                            );
                                        }
                                        Some(UpdateStatus::Available { version, url }) => {
                                            ui.colored_label(
                                                egui::Color32::LIGHT_GREEN,
                                                t!(
                                                    "options.new_version_available",
                                                    version = version
                                                )
                                                .to_string(),
                                            );
                                            ui.hyperlink_to(
                                                t!("options.open_release_page").to_string(),
                                                url,
                                            );
                                        }
                                        Some(UpdateStatus::Error(e)) => {
                                            ui.colored_label(
                                                egui::Color32::YELLOW,
                                                t!("options.check_failed", error = e).to_string(),
                                            );
                                        }
                                        None => {}
                                    }
                                    ui.add_space(6.0);
                                    ui.separator();
                                    let mut launch = self.settings.launch_on_startup;
                                    if ui
                                        .checkbox(
                                            &mut launch,
                                            t!("options.launch_on_startup").to_string(),
                                        )
                                        .on_hover_text(
                                            t!("options.launch_on_startup_tooltip").to_string(),
                                        )
                                        .changed()
                                    {
                                        match crate::autostart::set_enabled(launch) {
                                            Ok(()) => {
                                                self.settings.launch_on_startup = launch;
                                                self.settings.startup_prompt_shown = true;
                                                dirty = true;
                                            }
                                            Err(e) => {
                                                self.profile_status = Some(format!(
                                                    "{}: {e}",
                                                    t!("options.launch_on_startup_err")
                                                ));
                                            }
                                        }
                                    }
                                });

                            egui::CollapsingHeader::new(t!("options.section_language").to_string())
                                .default_open(false)
                                .show(ui, |ui| {
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
                                                    .selectable_label(
                                                        selected,
                                                        display_name_for(code),
                                                    )
                                                    .clicked()
                                                    && !selected
                                                {
                                                    self.settings.language =
                                                        Some(code.to_string());
                                                    rust_i18n::set_locale(code);
                                                    if let Some(tray) = &self.tray {
                                                        tray.retranslate();
                                                    }
                                                    self.settings.save();
                                                }
                                            }
                                        });
                                });

                            egui::CollapsingHeader::new(
                                t!("options.section_graph_sensors").to_string(),
                            )
                            .default_open(true)
                            .show(ui, |ui| {
                                dirty |= ui
                                    .checkbox(
                                        &mut self.settings.show_graph_panel,
                                        t!("options.show_sensors_graph").to_string(),
                                    )
                                    .changed();
                                ui.add_space(4.0);
                                ui.label(t!("options.graph_style_heading").to_string());
                                let current_style = self.settings.graph_style;
                                egui::ComboBox::from_id_salt("graph_style_pick")
                                    .selected_text(t!(current_style.display_key()).to_string())
                                    .show_ui(ui, |ui| {
                                        for style in GraphStyle::ALL {
                                            let enabled = style == GraphStyle::Classic
                                                || self.shader_backend_available;
                                            let selected = current_style == style;
                                            ui.add_enabled_ui(enabled, |ui| {
                                                if ui
                                                    .selectable_label(
                                                        selected,
                                                        t!(style.display_key()).to_string(),
                                                    )
                                                    .on_disabled_hover_text(
                                                        t!("options.shader_unavailable")
                                                            .to_string(),
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
                                            egui::Slider::new(
                                                &mut self.settings.shader_speed,
                                                0.0..=3.0,
                                            )
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
                                                resp.on_hover_text(
                                                    t!("options.fps_high_usage").to_string(),
                                                )
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
                                            .color_edit_button_rgb(
                                                &mut self.settings.shader_color_a,
                                            )
                                            .changed();
                                        ui.label(t!("options.fractal_color_b").to_string());
                                        dirty |= ui
                                            .color_edit_button_rgb(
                                                &mut self.settings.shader_color_b,
                                            )
                                            .changed();
                                    });
                                }
                                ui.add_space(4.0);
                                ui.label(t!("options.graph_sensors_heading").to_string());
                                ui.small(t!("options.graph_sensors_note").to_string());
                                // Sensors graph plots temperature only (GPU/CPU power and
                                // load live in their own panels) - only offer temp sensors
                                // here so the picker matches what the graph can show.
                                let temps: Vec<_> = snap
                                    .plottable
                                    .iter()
                                    .filter(|p| p.kind == SensorKind::Temperature)
                                    .collect();
                                if temps.is_empty() {
                                    ui.small(t!("dashboard.none").to_string());
                                } else {
                                    for p in &temps {
                                        let mut checked = self
                                            .settings
                                            .graph_sensor_ids
                                            .iter()
                                            .any(|s| s == &p.id);
                                        if ui.checkbox(&mut checked, p.label.as_str()).changed() {
                                            if checked {
                                                self.settings.graph_sensor_ids.push(p.id.clone());
                                            } else {
                                                self.settings
                                                    .graph_sensor_ids
                                                    .retain(|s| s != &p.id);
                                            }
                                            self.settings.save();
                                        }
                                    }
                                }
                                if self.settings.graph_sensor_ids.len() > 6 {
                                    ui.small(t!("options.graph_sensors_many_note").to_string());
                                }
                            });

                            egui::CollapsingHeader::new(t!("options.section_gpu_cpu").to_string())
                                .default_open(true)
                                .show(ui, |ui| {
                                    if ui
                                        .checkbox(
                                            &mut self.settings.show_gpu_panel,
                                            t!("options.show_gpu_panel").to_string(),
                                        )
                                        .changed()
                                    {
                                        dirty = true;
                                    }
                                    if ui
                                        .checkbox(
                                            &mut self.settings.show_cpu_panel,
                                            t!("options.show_cpu_panel").to_string(),
                                        )
                                        .changed()
                                    {
                                        dirty = true;
                                    }
                                    if ui
                                        .checkbox(
                                            &mut self.settings.show_host_sensors,
                                            t!("options.show_host_sensors").to_string(),
                                        )
                                        .changed()
                                    {
                                        self.host_enabled.store(
                                            self.settings.show_host_sensors,
                                            Ordering::Relaxed,
                                        );
                                        dirty = true;
                                    }
                                    ui.small(t!("options.host_sensor_note").to_string());
                                    ui.add_space(4.0);
                                    dirty |= ui
                                        .checkbox(
                                            &mut self.settings.auto_apply_curves,
                                            t!("options.auto_apply_curves").to_string(),
                                        )
                                        .changed();
                                    if self.settings.auto_apply_curves
                                        && !self.options.allow_hw_write
                                    {
                                        ui.colored_label(
                                            egui::Color32::YELLOW,
                                            t!("options.auto_apply_needs_write").to_string(),
                                        );
                                    }
                                    ui.add_space(4.0);
                                    ui.label(t!("options.rgb_heading").to_string());
                                    ui.small(t!("options.rgb_note").to_string());
                                    ui.add_space(4.0);
                                    ui.label(t!("options.names_heading").to_string());
                                    ui.small(t!("options.names_note").to_string());
                                });

                            egui::CollapsingHeader::new(t!("options.section_activity").to_string())
                                .default_open(false)
                                .show(ui, |ui| {
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
                                                && !matches!(
                                                    self.settings.activity_mode,
                                                    ActivityMode::LoadOnly
                                                ),
                                        );
                                        dirty = true;
                                    }
                                    if self.settings.show_activity_deck {
                                        ui.indent("activity_opts", |ui| {
                                            ui.horizontal(|ui| {
                                                ui.label(t!("options.activity_mode").to_string());
                                                for (mode, key) in [
                                                    (ActivityMode::Both, "options.activity_mode_both"),
                                                    (
                                                        ActivityMode::LoadOnly,
                                                        "options.activity_mode_load",
                                                    ),
                                                    (
                                                        ActivityMode::ProcessesOnly,
                                                        "options.activity_mode_procs",
                                                    ),
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
                                });

                            egui::CollapsingHeader::new(
                                t!("options.section_host_metrics").to_string(),
                            )
                            .default_open(false)
                            .show(ui, |ui| {
                                ui.label(t!("options.metrics_heading").to_string());
                                ui.small(t!("options.metrics_note").to_string());
                                if ui
                                    .checkbox(
                                        &mut self.settings.metrics_store_enabled,
                                        t!("options.metrics_store_enabled").to_string(),
                                    )
                                    .changed()
                                {
                                    dirty = true;
                                    if self.settings.metrics_store_enabled {
                                        self.metrics_sink =
                                            SqliteMetricsStore::spawn(SqliteStoreConfig {
                                                path: default_metrics_db_path().unwrap_or_else(
                                                    || std::path::PathBuf::from("metrics.sqlite"),
                                                ),
                                                retention_days: u32::from(
                                                    self.settings.metrics_retention_days.max(1),
                                                ),
                                                flush_ms: 500,
                                            });
                                    } else {
                                        self.metrics_sink = None;
                                    }
                                }
                                if self.settings.metrics_store_enabled {
                                    ui.horizontal(|ui| {
                                        ui.label(t!("options.metrics_sample_secs").to_string());
                                        for s in [2_u16, 5, 10, 30] {
                                            if ui
                                                .selectable_value(
                                                    &mut self.settings.metrics_sample_secs,
                                                    s,
                                                    format!("{s}s"),
                                                )
                                                .changed()
                                            {
                                                dirty = true;
                                            }
                                        }
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label(t!("options.metrics_retention_days").to_string());
                                        for d in [1_u16, 7, 30, 90] {
                                            if ui
                                                .selectable_value(
                                                    &mut self.settings.metrics_retention_days,
                                                    d,
                                                    format!("{d}d"),
                                                )
                                                .changed()
                                            {
                                                dirty = true;
                                                if let Some(store) = &self.metrics_sink {
                                                    store.request_purge();
                                                }
                                            }
                                        }
                                    });
                                    if let Some(path) = default_metrics_db_path() {
                                        ui.small(format!(
                                            "{} {}",
                                            t!("options.metrics_path"),
                                            path.display()
                                        ));
                                    }
                                    if ui
                                        .button(t!("options.metrics_export_csv").to_string())
                                        .clicked()
                                        && let Some(store) = &self.metrics_sink
                                        && let Ok(dir) = fancontrol_core::config_dir()
                                    {
                                        let exports = dir.join("exports");
                                        let _ = std::fs::create_dir_all(&exports);
                                        let name = format!(
                                            "metrics-{}.csv",
                                            std::time::SystemTime::now()
                                                .duration_since(std::time::UNIX_EPOCH)
                                                .map(|d| d.as_secs())
                                                .unwrap_or(0)
                                        );
                                        let path = exports.join(name);
                                        match store.request_export_csv(&path) {
                                            Ok(n) => {
                                                self.profile_status = Some(format!(
                                                    "{} ({n} rows) → {}",
                                                    t!("options.metrics_export_ok"),
                                                    path.display()
                                                ));
                                            }
                                            Err(e) => {
                                                self.profile_status = Some(format!(
                                                    "{}: {e}",
                                                    t!("options.metrics_export_err")
                                                ));
                                            }
                                        }
                                    }
                                }
                                ui.add_space(4.0);
                                if ui
                                    .checkbox(
                                        &mut self.settings.otel_enabled,
                                        t!("options.otel_enabled").to_string(),
                                    )
                                    .changed()
                                {
                                    dirty = true;
                                    self.otel_sink = if self.settings.otel_enabled {
                                        OtlpSink::spawn(self.settings.otel_endpoint.clone())
                                    } else {
                                        None
                                    };
                                }
                                if self.settings.otel_enabled {
                                    ui.horizontal(|ui| {
                                        ui.label(t!("options.otel_endpoint").to_string());
                                        if ui
                                            .text_edit_singleline(&mut self.settings.otel_endpoint)
                                            .changed()
                                        {
                                            dirty = true;
                                            self.otel_sink =
                                                OtlpSink::spawn(self.settings.otel_endpoint.clone());
                                        }
                                    });
                                    ui.small(t!("options.otel_deferred_note").to_string());
                                }
                            });
                        });

                    if dirty {
                        self.settings.clamp_graph_options();
                        self.settings.save();
                    }
                    ui.separator();
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
        let show_gpu = self.settings.show_gpu_panel;
        let show_cpu = self.settings.show_cpu_panel;
        // Activity deck load % lands on the CPU panel's load chip when both are live;
        // the poll thread can't fill this in itself (separate sampler/cadence).
        let mut cpu_view = snap.cpu.clone();
        if let Some(act) = &activity_snap {
            cpu_view.load_pct = act.load_pct;
        }
        // When Temps/Fans/Controls are all closed, grow graphs into that space.
        let dashboard_open = self.show_temps || self.show_fans || self.show_controls;
        if show_thermal || show_activity || show_gpu || show_cpu {
            let labels: HashMap<&str, &str> = snap
                .plottable
                .iter()
                .map(|p| (p.id.as_str(), p.label.as_str()))
                .collect();
            let units: HashMap<&str, Option<&str>> = snap
                .plottable
                .iter()
                .map(|p| (p.id.as_str(), p.unit.as_deref()))
                .collect();
            let kinds: HashMap<&str, SensorKind> = snap
                .plottable
                .iter()
                .map(|p| (p.id.as_str(), p.kind))
                .collect();

            // Compact defaults when the dashboard lists are visible; when they are
            // all closed, fill almost all remaining height so plots are not stuck
            // at ~half the window.
            let top_viz = show_thermal || show_gpu || show_cpu;
            let default_h = match (top_viz, show_activity) {
                (true, true) => 420.0,
                (true, false) => 260.0,
                (false, true) => 220.0,
                (false, false) => 0.0,
            };
            let min_h = match (top_viz, show_activity) {
                (true, true) => 320.0,
                (true, false) => 180.0,
                (false, true) => 160.0,
                (false, false) => 0.0,
            };

            let avail = ui.available_height();
            let mut graph_panel = egui::Panel::top("graph_area").resizable(true);
            if dashboard_open {
                // Leave room for the central columns; user can still drag larger.
                graph_panel = graph_panel
                    .default_size(default_h)
                    .min_size(min_h)
                    .max_size((avail * 0.75).max(min_h + 40.0));
            } else {
                // Lists hidden: claim nearly all remaining height (leave a thin strip).
                // Order bounds so a short window never panics `f32::clamp` (lo > hi).
                let fill = clamp_ui_height(avail - 8.0, min_h.max(200.0), avail.max(200.0));
                graph_panel = graph_panel.exact_size(fill);
            }

            graph_panel.show(ui, |ui| {
                // Top row: thermal graph and/or GPU detail (side-by-side when both).
                if top_viz {
                    let room = ui.available_height().max(80.0);
                    let row_h = if show_activity {
                        clamp_ui_height(room * 0.55, 140.0, (room - 100.0).max(40.0))
                    } else {
                        clamp_ui_height(room, 140.0_f32.min(room), room)
                    };

                    // Ceiling from GPU power.limit and CPU package power limit
                    // (host.cpu.power.limit / mock.cpu_power_limit), whichever is higher.
                    let power_ceiling = snap
                        .gpus
                        .iter()
                        .filter_map(|g| g.power_limit_w)
                        .chain(snap.plottable.iter().filter_map(|p| {
                            (p.id == "host.cpu.power.limit" || p.id == "mock.cpu_power_limit")
                                .then_some(p.value)
                        }))
                        .filter(|w| w.is_finite() && *w > 1.0)
                        .fold(None, |acc: Option<f64>, w| {
                            Some(acc.map(|a| a.max(w)).unwrap_or(w))
                        })
                        .map(|w| w as f32);

                    let active_cols =
                        usize::from(show_thermal) + usize::from(show_gpu) + usize::from(show_cpu);
                    // Below this per-column width, `ui.columns` no longer clips its content
                    // to the column rect, so a wide GPU/CPU row visually bleeds into the
                    // neighboring column (e.g. GPU overlaying Sensors). Stack vertically
                    // instead of squeezing columns thinner than a metric row can shrink to.
                    const MIN_DOMAIN_COL_WIDTH: f32 = 180.0;
                    let too_narrow_for_columns = active_cols > 1
                        && ui.available_width() / (active_cols as f32) < MIN_DOMAIN_COL_WIDTH;

                    if active_cols > 1 && !too_narrow_for_columns {
                        ui.columns(active_cols, |cols| {
                            let mut i = 0;
                            if show_thermal {
                                let col_rect = cols[i].max_rect();
                                cols[i].push_id("thermal_graph_col", |ui| {
                                    ui.set_clip_rect(col_rect);
                                    // Equal vertical slot as GPU/CPU columns.
                                    ui.allocate_ui(egui::vec2(ui.available_width(), row_h), |ui| {
                                        ui.set_min_height(row_h);
                                        ui.set_max_height(row_h);
                                        self.ui_thermal_graph_block(
                                            ui,
                                            &labels,
                                            &units,
                                            &kinds,
                                            row_h,
                                            power_ceiling,
                                        );
                                    });
                                });
                                i += 1;
                            }
                            if show_gpu {
                                let col_rect = cols[i].max_rect();
                                cols[i].push_id("gpu_detail_col", |ui| {
                                    ui.set_clip_rect(col_rect);
                                    Self::domain_column_slot(ui, row_h, |ui| {
                                        show_gpu_panel(
                                            ui,
                                            &snap.gpus,
                                            Some(&self.gpu_power_history),
                                        );
                                    });
                                });
                                i += 1;
                            }
                            if show_cpu {
                                let col_rect = cols[i].max_rect();
                                cols[i].push_id("cpu_detail_col", |ui| {
                                    ui.set_clip_rect(col_rect);
                                    Self::domain_column_slot(ui, row_h, |ui| {
                                        show_cpu_panel(
                                            ui,
                                            &cpu_view,
                                            Some(&self.cpu_power_history),
                                        );
                                    });
                                });
                            }
                        });
                    } else if active_cols > 1 {
                        // Too narrow for side-by-side columns: stack the domain panels
                        // vertically in a scroll area instead of overlaying each other.
                        egui::ScrollArea::vertical()
                            .id_salt("domain_stack_scroll")
                            .max_height(room)
                            .auto_shrink([false, false])
                            .show(ui, |ui| {
                                if show_thermal {
                                    ui.push_id("thermal_graph_col", |ui| {
                                        ui.allocate_ui(
                                            egui::vec2(ui.available_width(), row_h),
                                            |ui| {
                                                ui.set_min_height(row_h);
                                                self.ui_thermal_graph_block(
                                                    ui,
                                                    &labels,
                                                    &units,
                                                    &kinds,
                                                    row_h,
                                                    power_ceiling,
                                                );
                                            },
                                        );
                                    });
                                    ui.add_space(6.0);
                                    ui.separator();
                                }
                                if show_gpu {
                                    ui.push_id("gpu_detail_col", |ui| {
                                        Self::domain_column_slot(ui, row_h, |ui| {
                                            show_gpu_panel(
                                                ui,
                                                &snap.gpus,
                                                Some(&self.gpu_power_history),
                                            );
                                        });
                                    });
                                    ui.add_space(6.0);
                                    ui.separator();
                                }
                                if show_cpu {
                                    ui.push_id("cpu_detail_col", |ui| {
                                        Self::domain_column_slot(ui, row_h, |ui| {
                                            show_cpu_panel(
                                                ui,
                                                &cpu_view,
                                                Some(&self.cpu_power_history),
                                            );
                                        });
                                    });
                                }
                            });
                    } else if show_thermal {
                        ui.allocate_ui(egui::vec2(ui.available_width(), row_h), |ui| {
                            ui.set_min_height(row_h);
                            self.ui_thermal_graph_block(
                                ui,
                                &labels,
                                &units,
                                &kinds,
                                row_h,
                                power_ceiling,
                            );
                        });
                    } else if show_gpu {
                        Self::domain_column_slot(ui, row_h, |ui| {
                            show_gpu_panel(ui, &snap.gpus, Some(&self.gpu_power_history));
                        });
                    } else if show_cpu {
                        Self::domain_column_slot(ui, row_h, |ui| {
                            show_cpu_panel(ui, &cpu_view, Some(&self.cpu_power_history));
                        });
                    }
                }

                if show_activity {
                    if top_viz {
                        ui.separator();
                    }
                    let act = activity_snap.as_ref().cloned().unwrap_or_default();
                    let sort_before = self.settings.activity_sort;
                    let act_h = ui
                        .available_height()
                        .clamp(120.0, ui.available_height().max(120.0));
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
            let n = usize::from(self.show_temps)
                + usize::from(self.show_fans)
                + usize::from(self.show_controls);
            if n == 0 {
                ui.weak(t!("dashboard.all_hidden").to_string());
                return;
            }
            ui.columns(n, |cols| {
                let mut i = 0;
                if self.show_temps {
                    self.ui_temps_column(&mut cols[i], &snap);
                    i += 1;
                }
                if self.show_fans {
                    self.ui_fans_column(&mut cols[i], &snap);
                    i += 1;
                }
                if self.show_controls {
                    self.ui_controls_column(&mut cols[i], &snap);
                }
            });
        });

        self.show_rename_modal(&ctx);
        // Writes consent first (blocks PWM until answered); then PawnIO help if needed.
        self.show_writes_consent_dialog(&ctx);
        if !self.show_writes_consent {
            self.show_pawnio_dialog(&ctx);
            // After critical hardware dialogs, offer start-with-Windows once.
            self.show_startup_prompt_dialog(&ctx);
        }
    }
}

impl FanApp {
    fn ui_temps_column(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
        ui.heading(t!("dashboard.temperatures").to_string());
        ui.separator();
        egui::ScrollArea::vertical()
            .id_salt("temps")
            .show(ui, |ui| {
                if snap.temps.is_empty() {
                    ui.label(t!("dashboard.none").to_string());
                }
                for (id, label, v) in &snap.temps {
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Label::new(label.as_str()).sense(egui::Sense::click()))
                            .on_hover_text(t!("dashboard.click_to_rename").to_string())
                            .clicked()
                        {
                            self.begin_rename(id, label, false);
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            ui.monospace(format!("{v:5.1} °C"));
                        });
                    });
                    ui.small(id);
                }
            });
    }

    fn ui_fans_column(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
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
                ui.horizontal(|ui| {
                    if ui
                        .add(egui::Label::new(label.as_str()).sense(egui::Sense::click()))
                        .on_hover_text(t!("dashboard.click_to_rename").to_string())
                        .clicked()
                    {
                        self.begin_rename(id, label, false);
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if *v < 1.0 {
                            ui.weak("0");
                        } else {
                            ui.monospace(format!("{v:6.0}"));
                        }
                    });
                });
                ui.small(id);
            }
        });
    }

    fn ui_controls_column(&mut self, ui: &mut egui::Ui, snap: &crate::poll::Snapshot) {
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
                    ui.group(|ui| {
                        if ui
                            .add(egui::Label::new(c.label.as_str()).sense(egui::Sense::click()))
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
                            ui.weak("RPM -");
                        }

                        let cur = self
                            .profile
                            .assignments
                            .get(&c.id)
                            .map(|aid| {
                                self.profile
                                    .curves
                                    .iter()
                                    .find(|cv| cv.id == *aid)
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
                                    .map(|cv| (cv.id.clone(), curve_combo_label(cv).to_string()))
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
                            let stored = self.profile.sensor_bindings.get(&c.id).cloned();
                            let bound_id = stored
                                .filter(|id| is_cpu_temp_candidate(id))
                                .filter(|id| cpu_temps.iter().any(|(sid, _, _)| sid == id))
                                .unwrap_or_else(|| default_cpu_curve_sensor(snap));
                            if self.profile.sensor_bindings.get(&c.id) != Some(&bound_id) {
                                self.profile
                                    .sensor_bindings
                                    .insert(c.id.clone(), bound_id.clone());
                            }
                            let bound_label = cpu_temps
                                .iter()
                                .find(|(id, _, _)| *id == bound_id)
                                .map(|(_, label, _)| (*label).clone())
                                .unwrap_or_else(|| bound_id.clone());
                            let bind_resp = egui::ComboBox::from_id_salt(format!("bind-{}", c.id))
                                .selected_text(bound_label)
                                .show_ui(ui, |ui| {
                                    for (id, label, _) in &cpu_temps {
                                        let selected = *id == bound_id;
                                        if ui.selectable_label(selected, label.as_str()).clicked()
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
                        }

                        let locked = self.is_user_locked(&c.id);
                        let hw_duty = c.duty.unwrap_or(0);
                        if !locked && let Some(d) = c.duty {
                            self.slider_state.insert(c.id.clone(), f32::from(d));
                        }
                        let mut value =
                            *self.slider_state.get(&c.id).unwrap_or(&f32::from(hw_duty));

                        let enabled = c.writable
                            && !self.show_writes_consent
                            && (self.options.allow_hw_write || c.id.starts_with("mock."));

                        if c.duty.is_none() {
                            ui.weak("duty -");
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

    /// Fixed-height slot shared by Sensors / GPU / CPU columns so bottoms align.
    fn domain_column_slot(ui: &mut egui::Ui, row_h: f32, add_contents: impl FnOnce(&mut egui::Ui)) {
        ui.allocate_ui(egui::vec2(ui.available_width(), row_h), |ui| {
            ui.set_min_height(row_h);
            ui.set_max_height(row_h);
            egui::ScrollArea::vertical()
                .id_salt(ui.id().with("domain_slot_scroll"))
                .max_height(row_h)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.set_min_height(row_h);
                    add_contents(ui);
                });
        });
    }

    /// Thermal / multi-metric graph (or shader style) for the top visualization row.
    /// `slot_h` is the **total** column height (equal to GPU/CPU slots); the plot uses
    /// remaining space after the legend so the three domain cards share one surface.
    fn ui_thermal_graph_block(
        &mut self,
        ui: &mut egui::Ui,
        labels: &HashMap<&str, &str>,
        units: &HashMap<&str, Option<&str>>,
        kinds: &HashMap<&str, SensorKind>,
        slot_h: f32,
        power_axis_ceiling: Option<f32>,
    ) {
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
        // Sensors graph is temperature-only (see spec goal 2/8): GPU/CPU power ids some
        // users still have saved in `graph_sensor_ids` from before the CPU/GPU panels
        // existed are filtered out here (by live `SensorKind`, not stripped from
        // settings) rather than stripped from settings, so nothing is lost if a future
        // graph adds other units back. An id currently absent from the live snapshot
        // (e.g. a power sensor with PawnIO not elevated) is excluded too - its kind is
        // unknown, and showing an empty, uncategorized ghost series helps no one.
        let series: Vec<GraphSeries> = self
            .settings
            .graph_sensor_ids
            .iter()
            .enumerate()
            .filter(|(_, id)| kinds.get(id.as_str()) == Some(&SensorKind::Temperature))
            .filter_map(|(i, id)| {
                self.histories.get(id).map(|h| GraphSeries {
                    label: labels.get(id.as_str()).copied().unwrap_or(id.as_str()),
                    palette_index: i,
                    history: h,
                    unit: units.get(id.as_str()).copied().flatten(),
                })
            })
            .collect();
        let style = self.settings.graph_style;
        let only_temps = series
            .iter()
            .all(|s| s.unit.is_none() || s.unit == Some("°C") || s.unit == Some("C"));

        // Match GPU/CPU domain_card outer size: fill the slot, plot uses rest of height.
        let fill = egui::vec2(ui.available_width(), slot_h.max(40.0));
        ui.allocate_ui(fill, |ui| {
            ui.set_min_height(slot_h);
            ui.set_max_height(slot_h);
            // Reserve plot height from remaining space after group header (~legend).
            // header_budget: multi-sensor legend can wrap; keep plot usable.
            let header_budget = if series.len() > 1 { 56.0 } else { 36.0 };
            let plot_h = (ui.available_height() - header_budget).clamp(70.0, slot_h);

            if style == GraphStyle::Classic || !self.shader_backend_available || !only_temps {
                show_metric_graph(
                    ui,
                    &series,
                    self.settings.graph_window_minutes,
                    &mut self.graph_axis_max,
                    &mut self.graph_axis_max_secondary,
                    plot_h,
                    power_axis_ceiling,
                );
                if style.is_shader() && !only_temps {
                    ui.small(t!("graph.shader_temps_only_note").to_string());
                } else if style.is_shader() && !self.shader_backend_available {
                    ui.small(t!("graph.shader_fallback_note").to_string());
                }
            } else {
                let t = self.shader_clock.elapsed().as_secs_f32() * self.settings.shader_speed;
                let readings: Vec<(String, f32)> = series
                    .iter()
                    .filter_map(|s| s.history.last().map(|v| (s.label.to_string(), v)))
                    .collect();
                let signal = ThermalSignal::from_readings(readings);
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
            let leftover = ui.available_height();
            if leftover > 1.0 {
                ui.allocate_exact_size(
                    egui::vec2(ui.available_width(), leftover),
                    egui::Sense::hover(),
                );
            }
        });
    }

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
            self.cpu_power_history.configure(
                self.settings.graph_window_minutes,
                self.settings.graph_sample_secs,
            );
            self.gpu_power_history.configure(
                self.settings.graph_window_minutes,
                self.settings.graph_sample_secs,
            );
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

    fn show_startup_prompt_dialog(&mut self, ctx: &egui::Context) {
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
                if let Some(msg) = &self.elevate_status {
                    ui.add_space(6.0);
                    ui.colored_label(egui::Color32::LIGHT_RED, msg);
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
    fn try_relaunch_elevated(&mut self) {
        match crate::elevation::relaunch_elevated() {
            Ok(()) => {
                // Elevated child is running - leave the non-elevated process.
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
        // Ensure the live CPU candidate is present under its real id (CPUTIN/PECI/CPU).
        if let (Some(id), Some(t)) = (&snap.cpu_temp_id, snap.cpu_temp) {
            temps.entry(id.clone()).or_insert(t);
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
                        if !name.is_empty()
                            && let Ok(mut map) = self.map.lock()
                        {
                            if self.rename_is_control {
                                map.set_control_name(&id, &name);
                            } else {
                                map.set_sensor_name(&id, &name);
                            }
                            let _ = map.save();
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

#[cfg(test)]
mod tests {
    use super::curve_combo_label;
    use fancontrol_core::FanCurve;

    #[test]
    fn combo_label_uses_name_not_id() {
        let cv = FanCurve::linear("curve2", "Full Speed", 30.0, 80.0, 20, 100);
        assert_eq!(curve_combo_label(&cv), "Full Speed");
    }

    #[test]
    fn combo_label_falls_back_to_id_when_name_blank() {
        let cv = FanCurve::linear("curve3", "   ", 30.0, 80.0, 20, 100);
        assert_eq!(curve_combo_label(&cv), "curve3");
    }
}
