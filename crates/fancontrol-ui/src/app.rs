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
use crate::theme::{self, ThemeChoice};
use crate::tray::{AppTray, TrayCommand, TrayState};
use crate::update_check::{UpdateChecker, UpdateStatus};
use crate::write_queue::WriteQueue;
use eframe::egui;
use fancontrol_core::{
    ChannelMap, CoreError, CurveEvalState, FanCurve, MetricSample, Profile, SensorKind,
    evaluate_profile_step, is_cpu_temp_candidate, list_profiles, load_profile,
    resolve_curve_temp_sensor, save_profile,
};
use fancontrol_metrics::{
    MetricSink, OtlpSink, SqliteMetricsStore, SqliteStoreConfig, default_metrics_db_path,
};
use fancontrol_plugins::ProviderRegistry;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

mod curves;
mod dashboard;
mod dialogs;
mod graph_area;
mod title_bar;
mod top_bar;

const GRAPH_WINDOWS: [u16; 4] = [10, 20, 30, 60];
const GRAPH_SAMPLES: [u16; 4] = [1, 2, 5, 10];
const PAWNIO_URL: &str = "https://pawnio.eu";

/// Width reserved for the right-hand value in the Temperatures / Fans lists.
const LIST_VALUE_W: f32 = 76.0;
/// Vertical padding around one list row, so labels never touch the row above.
const LIST_ROW_MARGIN: egui::Margin = egui::Margin {
    left: 2,
    right: 2,
    top: 3,
    bottom: 3,
};

/// One `label … value` row of the Temperatures / Fans lists.
///
/// The value owns a fixed column on the right and the label truncates into what
/// is left, so a long sensor name can no longer collide with its reading.
/// Returns `true` when the label was clicked (rename).
fn list_row(ui: &mut egui::Ui, label: &str, id: &str, value: impl FnOnce(&mut egui::Ui)) -> bool {
    let mut clicked = false;
    egui::Frame::NONE
        .inner_margin(LIST_ROW_MARGIN)
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let label_w = (ui.available_width() - LIST_VALUE_W - 8.0).max(48.0);
                ui.allocate_ui_with_layout(
                    egui::vec2(label_w, ui.spacing().interact_size.y),
                    egui::Layout::left_to_right(egui::Align::Center),
                    |ui| {
                        ui.set_min_width(label_w);
                        clicked = ui
                            .add(
                                egui::Label::new(label)
                                    .truncate()
                                    .sense(egui::Sense::click()),
                            )
                            .on_hover_text(format!("{label}\n{}", t!("dashboard.click_to_rename")))
                            .clicked();
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), value);
            });
            ui.small(id);
        });
    clicked
}

/// Hand fans back to BIOS SmartFan if the UI or a hardware worker thread panics
/// (the GUI build has no console, so a panic is otherwise a silent exit with fans
/// pinned at the last manual duty). Panics in unrelated helper threads (update
/// check, …) keep fan control.
///
/// The hook runs before unwinding, so a worker that panicked mid-write still
/// holds the (non-reentrant) bus lock: restoring on this thread would deadlock.
/// Restore on a fresh thread instead and wait a bounded time; if it is blocked on
/// that lock it finishes once this thread unwinds and releases it.
fn install_restore_on_panic(reg: Arc<ProviderRegistry>) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let thread = std::thread::current();
        if matches!(
            thread.name(),
            Some("main" | "fancontrol-poll" | "fancontrol-write")
        ) {
            let reg = Arc::clone(&reg);
            let (done_tx, done_rx) = std::sync::mpsc::channel();
            let spawned = std::thread::Builder::new()
                .name("fancontrol-restore".into())
                .spawn(move || {
                    reg.restore_all();
                    let _ = done_tx.send(());
                });
            if spawned.is_ok() {
                let _ = done_rx.recv_timeout(Duration::from_secs(3));
            }
        }
        previous(info);
    }));
}

/// `f32::clamp` panics when `lo > hi`. Layout heights are dynamic; always order bounds.
fn clamp_ui_height(v: f32, lo: f32, hi: f32) -> f32 {
    let min_b = lo.min(hi);
    let max_b = lo.max(hi);
    v.clamp(min_b, max_b)
}

/// ComboBox label: stored curve name, fallback to id if the name is empty.
fn curve_combo_label(curve: &FanCurve) -> &str {
    let n = curve.name.trim();
    if n.is_empty() { curve.id.as_str() } else { n }
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
    install_restore_on_panic(Arc::clone(&reg));
    let profile = load_or_create_default_profile(settings.last_profile_id.as_deref());
    if settings.last_profile_id.as_deref() != Some(profile.id.as_str()) {
        settings.last_profile_id = Some(profile.id.as_str().to_string());
        settings.save();
    }
    let pawnio_dialog = detect_pawnio_dialog(options.include_hw);
    // First-run writes consent only when the process actually allows PWM.
    let show_writes_consent = options.allow_hw_write && !settings.writes_risk_acknowledged;
    let (theme_choice, system_font) = (settings.theme, settings.system_font);
    let language = settings.language.clone().unwrap_or_default();

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
        reg,
        options,
        map,
        snapshot,
        writes,
        status,
        settings,
        host_enabled,
        slider_state: HashMap::new(),
        user_lock_until: HashMap::new(),
        echo_hold_until: HashMap::new(),
        write_error: None,
        rename_id: None,
        rename_buf: String::new(),
        rename_is_control: false,
        histories: HashMap::new(),
        graph_axis_max: None,
        graph_axis_max_secondary: None,
        metrics_sink,
        pending_export: None,
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
        title_icon: None,
        top_toggles_w: 0.0,
        last_ui_pass: Instant::now(),
        last_stall_log: Instant::now(),
    };

    let icon = eframe::icon_data::from_png_bytes(include_bytes!("../../../assets/icon.png"))
        .map_err(|e| UiError::Eframe(format!("app icon: {e}")))?;

    let defaults = eframe::NativeOptions::default();
    let mut wgpu_options = defaults.wgpu_options;
    wgpu_options.on_surface_status = Arc::new(surface_status_action);
    if let eframe::egui_wgpu::WgpuSetup::CreateNew(setup) = &mut wgpu_options.wgpu_setup
        && std::env::var_os("WGPU_BACKEND").is_none()
    {
        setup.native_adapter_selector = Some(Arc::new(prefer_dx12_adapter));
    }
    let native = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1200.0, 860.0])
            .with_title("Fancontrol-RS")
            // Neon draws its own title bar (theme::apply keeps this in sync later).
            .with_decorations(!theme_choice.custom_frame())
            .with_icon(icon),
        wgpu_options,
        ..defaults
    };

    eframe::run_native(
        "Fancontrol-RS",
        native,
        Box::new(move |cc| {
            theme::apply(&cc.egui_ctx, theme_choice);
            // Windows UI fonts + CJK fallback (loaded unconditionally: the language
            // can be switched live at runtime).
            theme::install_fonts(&cc.egui_ctx, system_font, &language);

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

            // Wake the event loop at least twice a second even if a scheduled
            // repaint gets lost (seen after the Windows resize loop): keeps the UI,
            // tray commands and curve apply alive whatever the window backend does.
            let ctx = cc.egui_ctx.clone();
            std::thread::Builder::new()
                .name("repaint-watchdog".into())
                .spawn(move || {
                    loop {
                        std::thread::sleep(Duration::from_millis(500));
                        ctx.request_repaint();
                    }
                })
                .ok();

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
    /// Kept to hand controls back to BIOS SmartFan on exit.
    reg: Arc<ProviderRegistry>,
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
    /// Per control: keep the requested duty on the slider until this instant, so the
    /// previous hardware reading (the EC echo lags a poll) does not snap it back.
    echo_hold_until: HashMap<String, Instant>,
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
    /// CSV export running on the metrics worker: target path + reply channel,
    /// polled in `background_tick` so the UI thread never waits on it.
    pending_export: Option<PendingExport>,
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
    /// App icon texture for the Neon title bar (see `app/title_bar.rs`).
    title_icon: Option<egui::TextureHandle>,
    /// Width of the top-bar toggles last frame (see `ui_top_toggles`).
    top_toggles_w: f32,
    /// Start of the last `ui()` pass, and of the last "UI stalled" log (see `log_ui_stall`).
    last_ui_pass: Instant,
    last_stall_log: Instant,
}

fn load_or_create_default_profile(preferred: Option<&str>) -> Profile {
    if let Some(id) = preferred
        && let Ok(p) = load_profile(id)
    {
        return p;
    }
    let default_missing = match load_profile("default") {
        Ok(p) => return p,
        Err(CoreError::ProfileNotFound(_)) => true,
        Err(e) => {
            // Keep the user's file: run on the built-in default without saving over it.
            tracing::warn!(error = %e, "default profile unreadable, using built-in curves");
            false
        }
    };
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
    if default_missing {
        let _ = save_profile(&p);
    }
    p
}

/// A CSV export in flight: target path and the metrics worker's reply channel.
type PendingExport = (
    std::path::PathBuf,
    std::sync::mpsc::Receiver<Result<usize, String>>,
);

/// Lay out `add_contents` in the available width without letting over-wide
/// content (narrow window) widen the parent. egui sizes a panel, and the
/// separator line it draws, from its content rect, so an overflowing row used to
/// draw the line straight across the Options panel. The overflow itself is
/// clipped by the panel.
fn clamp_width<R>(ui: &mut egui::Ui, add_contents: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let max_rect = ui.available_rect_before_wrap();
    let mut child = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(max_rect)
            .layout(*ui.layout()),
    );
    let inner = add_contents(&mut child);
    let used = egui::vec2(max_rect.width(), child.min_rect().height());
    ui.allocate_rect(
        egui::Rect::from_min_size(max_rect.min, used),
        egui::Sense::hover(),
    );
    inner
}

/// Pick the GPU adapter: DirectX 12 first. On NVIDIA (RTX 5080, driver 617)
/// the Vulkan path stopped presenting after a window resize: the image stayed at
/// the old size and the event loop no longer woke up on its own. DX12 does not
/// show it. Falls back to any other usable adapter; `WGPU_BACKEND` (e.g.
/// `vulkan`) bypasses this selector entirely.
fn prefer_dx12_adapter(
    adapters: &[eframe::egui_wgpu::wgpu::Adapter],
    surface: Option<&eframe::egui_wgpu::wgpu::Surface<'_>>,
) -> Result<eframe::egui_wgpu::wgpu::Adapter, String> {
    use eframe::egui_wgpu::wgpu::{Backend, DeviceType};
    let usable: Vec<_> = adapters
        .iter()
        .filter(|a| surface.is_none_or(|s| a.is_surface_supported(s)))
        .filter(|a| a.get_info().device_type != DeviceType::Cpu)
        .collect();
    let dx12 = usable.iter().find(|a| a.get_info().backend == Backend::Dx12);
    dx12.or(usable.first())
        .map(|a| (*a).clone())
        .or_else(|| adapters.first().cloned())
        .ok_or_else(|| "no usable wgpu adapter".to_owned())
}

/// How to recover when wgpu can't hand us a frame to draw into.
///
/// egui-wgpu's default silently skips the frame on `Occluded` and `Timeout`. On
/// Windows that state can stick after a minimize / restore or a resize: the app
/// keeps running (tray, curves) but nothing is repainted, leaving the old frame
/// at the old size with a black band around it. Reconfiguring the swapchain
/// gets a fresh frame next time; when the window really is hidden eframe does
/// not paint at all, so this costs nothing then.
fn surface_status_action(
    status: &eframe::egui_wgpu::wgpu::CurrentSurfaceTexture,
) -> eframe::egui_wgpu::SurfaceErrorAction {
    use eframe::egui_wgpu::SurfaceErrorAction;
    use eframe::egui_wgpu::wgpu::CurrentSurfaceTexture;
    use std::sync::atomic::AtomicU64;

    // Log the first occurrence and then every 100th, enough to confirm the path
    // from a user's log without flooding it at 5 Hz.
    static DROPPED: AtomicU64 = AtomicU64::new(0);
    let n = DROPPED.fetch_add(1, Ordering::Relaxed);
    if n.is_multiple_of(100) {
        tracing::info!(status = ?status, dropped = n + 1, "wgpu frame not acquired");
    }

    match status {
        CurrentSurfaceTexture::Lost => SurfaceErrorAction::RecreateSurface,
        CurrentSurfaceTexture::Outdated
        | CurrentSurfaceTexture::Occluded
        | CurrentSurfaceTexture::Timeout => SurfaceErrorAction::Reconfigure,
        _ => SurfaceErrorAction::SkipFrame,
    }
}

impl eframe::App for FanApp {
    fn on_exit(&mut self) {
        self.reg.restore_all();
    }

    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Smooth shader animation needs a much faster repaint cadence than the
        // rest of the UI - only pay for it while a shader style is actually
        // active, the backend supports it, and the window isn't minimized to tray.
        // Not while the graph is hidden or the window is minimized to the taskbar
        // (only the tray hide clears `window_visible`).
        let minimized = ctx.input(|i| i.viewport().minimized == Some(true));
        let repaint_interval = if self.settings.graph_style.is_shader()
            && self.shader_backend_available
            && self.settings.show_graph_panel
            && self.window_visible
            && !minimized
        {
            Duration::from_secs_f32(1.0 / f32::from(self.settings.shader_fps))
        } else if self.settings.theme.is_animated() && self.window_visible && !minimized {
            // Neon border animation (20 fps is enough for a slow hue drift).
            Duration::from_millis(50)
        } else {
            Duration::from_millis(200)
        };
        ctx.request_repaint_after(repaint_interval);
        self.handle_tray(ctx);
        self.background_tick();
        self.log_ui_stall(ctx);

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
        let gap = self.last_ui_pass.elapsed();
        if self.window_visible && gap > Duration::from_secs(2) {
            tracing::info!(
                gap_ms = gap.as_millis() as u64,
                "ui pass resumed after a gap"
            );
        }
        self.last_ui_pass = Instant::now();
        let ctx = ui.ctx().clone();
        let snap = self.snapshot.lock().map(|g| g.clone()).unwrap_or_default();

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

        if self.settings.theme.custom_frame() {
            self.ui_title_bar(ui);
        }
        egui::Panel::top("top").show(ui, |ui| {
            let toggles_w = self.top_toggles_w;
            let mut wrap_toggles = false;
            ui.horizontal(|ui| {
                self.ui_graph_controls(ui);
                ui.separator();
                let write_capable =
                    self.options.allow_hw_write && matches!(self.status, BackendStatus::Ok(_));
                if write_capable {
                    ui.colored_label(
                        theme::ok(ui.visuals()),
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
                    let warn = theme::warn(ui.visuals());
                    let resp = ui.colored_label(warn, t!("top_bar.read_only").to_string());
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
                    ui.colored_label(theme::error(ui.visuals()), msg);
                }
                if ui.available_width() >= toggles_w {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        self.ui_top_toggles(ui);
                    });
                } else {
                    wrap_toggles = true;
                }
            });
            // Narrow window: the toggles get their own row instead of overlapping the
            // controls on the left.
            if wrap_toggles {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.ui_top_toggles(ui);
                });
            }
            if self.settings.auto_apply_curves && !self.options.allow_hw_write {
                ui.colored_label(
                    theme::warn(ui.visuals()),
                    t!("top_bar.curve_readonly_warning").to_string(),
                );
            }
            // Backend detail + channel counts live in Options (less noise on the main strip).
            // Keep live errors here so failures stay visible.
            if let Some(err) = &snap.error {
                ui.colored_label(
                    theme::warn(ui.visuals()),
                    t!("top_bar.poll_error", error = err).to_string(),
                );
            }
            if let Some(err) = &self.write_error {
                ui.colored_label(
                    theme::error(ui.visuals()),
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
                                                theme::ok(ui.visuals()),
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
                                                theme::warn(ui.visuals()),
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

                            egui::CollapsingHeader::new(t!("options.section_theme").to_string())
                                .default_open(false)
                                .show(ui, |ui| {
                                    egui::ComboBox::from_id_salt("theme_pick")
                                        .selected_text(self.settings.theme.label())
                                        .show_ui(ui, |ui| {
                                            for choice in ThemeChoice::ALL {
                                                let selected = self.settings.theme == choice;
                                                if ui
                                                    .selectable_label(selected, choice.label())
                                                    .clicked()
                                                    && !selected
                                                {
                                                    self.settings.theme = choice;
                                                    theme::apply(ui.ctx(), choice);
                                                    self.settings.save();
                                                }
                                            }
                                        });
                                    if ui
                                        .checkbox(
                                            &mut self.settings.system_font,
                                            t!("options.system_font").to_string(),
                                        )
                                        .changed()
                                    {
                                        let lang =
                                            self.settings.language.as_deref().unwrap_or("en");
                                        theme::install_fonts(
                                            ui.ctx(),
                                            self.settings.system_font,
                                            lang,
                                        );
                                        self.settings.save();
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
                                                    // CJK face follows the language.
                                                    theme::install_fonts(
                                                        ui.ctx(),
                                                        self.settings.system_font,
                                                        code,
                                                    );
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
                                        theme::warn(ui.visuals()),
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
                                            theme::warn(ui.visuals()),
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
                                        self.apply_activity_deck_gate();
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
                                                    store.set_retention_and_purge(u32::from(d));
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
                                    let exporting = self.pending_export.is_some();
                                    if ui
                                        .add_enabled(
                                            !exporting,
                                            egui::Button::new(
                                                t!("options.metrics_export_csv").to_string(),
                                            ),
                                        )
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
                                        match store.start_export_csv(&path) {
                                            Ok(rx) => self.pending_export = Some((path, rx)),
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
                                        // Apply on Enter / focus loss, not per keystroke: each
                                        // change started a new exporter and rewrote settings.
                                        if ui
                                            .text_edit_singleline(&mut self.settings.otel_endpoint)
                                            .lost_focus()
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
                    clamp_width(ui, |ui| self.ui_curves_panel(ui, &snap));
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

            let graph_body = |ui: &mut egui::Ui| {
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
            };
            graph_panel.show(ui, |ui| clamp_width(ui, graph_body));
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
            // After critical hardware dialogs, offer start-with-Windows once. Not while
            // the PawnIO window is up: both are centered and would stack.
            if self.pawnio_dialog.is_none() {
                self.show_startup_prompt_dialog(&ctx);
            }
        }
        if self.settings.theme.custom_frame() {
            self.handle_frame_resize(&ctx);
        }
        if self.settings.theme == ThemeChoice::Neon {
            theme::paint_neon_border(&ctx, self.shader_clock.elapsed().as_secs_f64());
        }
    }
}

impl FanApp {
    /// Gate the CPU-activity worker from `show_activity_deck` + mode.
    fn apply_activity_deck_gate(&self) {
        let on = self.settings.show_activity_deck;
        fancontrol_plugins::cpu_activity::set_enabled(on);
        fancontrol_plugins::cpu_activity::set_sample_processes(
            on && !matches!(self.settings.activity_mode, ActivityMode::LoadOnly),
        );
    }

    /// Diagnostic for the "window stops repainting" bug: `logic()` keeps running
    /// (tray, curves) while the window is meant to be visible, but `ui()` has not
    /// run for a while. Logs what eframe believes about the window at that point.
    fn log_ui_stall(&mut self, ctx: &egui::Context) {
        let stalled = self.last_ui_pass.elapsed();
        if !self.window_visible
            || stalled < Duration::from_secs(2)
            || self.last_stall_log.elapsed() < Duration::from_secs(5)
        {
            return;
        }
        self.last_stall_log = Instant::now();
        let info = ctx.input(|i| i.viewport().clone());
        let stalled_ms = stalled.as_millis() as u64;
        if info.minimized == Some(true) {
            // Expected while minimized to the taskbar; debug only so it doesn't flood.
            tracing::debug!(stalled_ms, "ui pass paused: window minimized");
            return;
        }
        tracing::warn!(
            stalled_ms,
            occluded = ?info.occluded,
            focused = ?info.focused,
            inner_rect = ?info.inner_rect,
            "ui pass not running while the window should be visible"
        );
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
                    tracing::info!("tray Exit: closing the app");
                    self.really_exit = true;
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            }
        }
    }

    /// Per-tick work that must keep running while the window is hidden to tray:
    /// eframe skips `ui()` for a hidden root viewport and only calls `logic()`.
    /// Drains write outcomes, records graph/metrics history and applies curves.
    fn background_tick(&mut self) {
        self.poll_csv_export();
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
        self.histories
            .retain(|id, _| self.settings.graph_sensor_ids.contains(id));

        // Auto-apply curves ~1 Hz when enabled + write allowed + consent accepted.
        // Wait for the first real poll (tick 0 = empty default snapshot), else the
        // missing-temperature failsafe would briefly blast every fan at startup.
        if self.settings.auto_apply_curves
            && snap.tick > 0
            && self.options.allow_hw_write
            && !self.show_writes_consent
            && self.last_curve_apply.elapsed() >= Duration::from_millis(1000)
        {
            self.apply_curves_from_snapshot(&snap);
            self.last_curve_apply = Instant::now();
        }
    }

    fn apply_curves_from_snapshot(&mut self, snap: &crate::poll::Snapshot) {
        // Shared by auto-apply, "Apply now" and the tray entry: honour the consent
        // dialog and a read-only session here so no caller can bypass them.
        if self.show_writes_consent || snap.tick == 0 {
            return;
        }
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
            if !self.options.allow_hw_write && !ctrl.starts_with("mock.") {
                continue;
            }
            if self.last_applied_duty.get(&ctrl) == Some(&duty) {
                continue;
            }
            // Do not mark applied until WriteQueue reports success (see take_successes).
            self.writes.enqueue(&ctrl, duty);
            self.hold_echo(&ctrl);
            self.slider_state.insert(ctrl, f32::from(duty));
        }
        for e in step.errors {
            tracing::debug!(error = %e, "curve apply");
        }
    }

    fn poll_csv_export(&mut self) {
        let Some((path, rx)) = &self.pending_export else {
            return;
        };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                Err("metrics store worker stopped".to_string())
            }
        };
        self.profile_status = Some(match result {
            Ok(n) => format!(
                "{} ({n} rows) → {}",
                t!("options.metrics_export_ok"),
                path.display()
            ),
            Err(e) => format!("{}: {e}", t!("options.metrics_export_err")),
        });
        self.pending_export = None;
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
        self.hold_echo(id);
    }

    fn hold_echo(&mut self, id: &str) {
        self.echo_hold_until
            .insert(id.to_string(), Instant::now() + Duration::from_millis(2000));
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
