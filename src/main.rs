//! fancontrol-rs — modern fan control for Windows.
//!
//! CLI harness: mock and/or real PawnIO Super I/O sensors.
//! Full GUI arrives in Phase 2.

// GUI subsystem: Windows never auto-allocates a console for this binary (no flash on
// double-click/Explorer launch). CLI output is restored by re-attaching to a parent
// console at startup when one exists — see `attach_parent_console_for_cli`.
#![windows_subsystem = "windows"]

use clap::{Parser, Subcommand};
use fancontrol_core::{
    evaluate_curve, evaluate_profile_step, load_profile, save_profile, ChannelMap, ControlId,
    CurveEvalState, FanCurve, Profile, SensorId, SensorKind,
};
use fancontrol_plugins::{HostSensorProvider, MockProvider, ProviderRegistry};
use std::collections::HashMap;
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "fancontrol-rs",
    about = "Modern fan control for Windows (Rust + PawnIO)",
    version
)]
struct Cli {
    /// Include mock sensors/controls (**opt-in**; product default is hardware/host only)
    #[arg(long, global = true)]
    mock: bool,

    /// Hardware/host only (no mock). Default product mode; kept for scripts. Same as omitting `--mock`.
    #[arg(long, global = true)]
    hw_only: bool,

    /// Skip probing PawnIO Super I/O
    #[arg(long, global = true)]
    no_hw: bool,

    /// Allow real hardware PWM writes (accepted for scripts; writes are **on by default**).
    #[arg(long, global = true)]
    allow_hw_write: bool,

    /// Disable hardware PWM writes (read-only sensors / CLI / UI).
    #[arg(long, global = true)]
    read_only: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List discovered sensors
    ListSensors,
    /// List controllable fans
    ListControls,
    /// Read a sensor value by id
    Read {
        /// Sensor id (e.g. mock.cpu_temp or pawnio.0.temp.CPUTIN)
        id: String,
    },
    /// Set fan duty cycle (0-100).
    /// Hardware (`pawnio.*`) writes are on by default; use `--read-only` to block. Mock always allowed.
    SetDuty {
        /// Control id
        id: String,
        /// Duty percent 0..=100
        percent: u8,
    },
    /// Show PawnIO backend + Super I/O detection
    BackendStatus,
    /// Probe Super I/O chips only
    Detect,
    /// Diagnose SSD/HDD temperature sources (DeviceIoControl paths, no PowerShell).
    /// Prefer an elevated terminal so `\\.\PhysicalDriveN` opens.
    SampleStorage {
        /// Samples spaced by interval-ms (to check if temps change)
        #[arg(long, default_value_t = 3)]
        times: u32,
        #[arg(long, default_value_t = 5000)]
        interval_ms: u64,
    },
    /// Read-only snapshot: temps + fan RPMs + current duties (no writes)
    Sample {
        /// How many times to sample
        #[arg(long, default_value_t = 1)]
        times: u32,
        /// Delay between samples in ms
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        /// Show empty / n/a channels too
        #[arg(long)]
        all: bool,
    },
    /// Live read-only monitor (Ctrl+C to stop)
    Watch {
        /// Interval between samples in ms
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        /// Show empty channels
        #[arg(long)]
        all: bool,
    },
    /// Demo curve on mock (does not touch hardware fans)
    Demo {
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        #[arg(long, default_value_t = 40.0)]
        temp: f64,
    },
    /// Apply a saved profile in a loop.
    /// Default is **dry-run** (compute duties only). Real set_duty needs `--apply`
    /// (and must not use `--read-only`).
    Run {
        /// Profile id (default: default)
        #[arg(long, default_value = "default")]
        profile: String,
        /// Seconds to run (finite)
        #[arg(long, default_value_t = 30)]
        seconds: u64,
        /// Interval between steps in ms
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
        /// Actually call set_duty (writes are on by default; do not pass --read-only)
        #[arg(long)]
        apply: bool,
    },
    /// Safe single-control write probe: set duty briefly, sample, **restore**.
    /// Prefer a case fan. Use without `--read-only` (writes are on by default).
    TestDuty {
        /// Control id (e.g. pawnio.0.ctrl0)
        #[arg(long)]
        control: String,
        /// Temporary duty percent to apply
        #[arg(long)]
        percent: u8,
        /// How long to hold the test duty before restore (ms)
        #[arg(long, default_value_t = 3000)]
        hold_ms: u64,
    },
    /// Save a profile (mock by default, or hardware bindings with --hw)
    InitProfile {
        /// Bind to first live NCT668x CPU temp + ctrl0/ctrl1 style ids
        #[arg(long)]
        hw: bool,
        /// Profile id
        #[arg(long, default_value = "default")]
        id: String,
    },
    /// List saved profile ids
    ListProfiles,
    /// Write / update channel-map.json display names (owner NCT668x seed)
    MapInit {
        /// Overwrite existing map with seed
        #[arg(long)]
        force: bool,
    },
    /// Launch desktop UI (egui). Default when no subcommand. Flags: --mock, --hw-only, --read-only, --no-hw
    Ui,
}

fn build_registry(include_mock: bool, include_hw: bool, allow_hw_write: bool) -> ProviderRegistry {
    let mut reg = ProviderRegistry::new();
    if include_mock {
        reg.register_both(MockProvider::new());
    }
    if include_hw {
        let p = fancontrol_pawnio::try_provider_with_writes(allow_hw_write);
        tracing::info!(
            devices = p.device_count(),
            write_enabled = p.write_enabled(),
            "PawnIO provider probed\n{}",
            p.detection_report()
        );
        let report = p.detection_report();
        let count = p.device_count();
        if count == 0 {
            tracing::warn!("no supported Super I/O HWM opened\n{report}");
        }
        let arc = std::sync::Arc::new(p);
        reg.register_sensor_provider(Box::new(ArcSensor(arc.clone())));
        reg.register_control_provider(Box::new(ArcControl(arc)));
    }
    // Best-effort GPU/SSD (nvidia-smi / DeviceIoControl) — host path
    reg.register_sensor_provider(Box::new(HostSensorProvider::new()));
    reg
}

/// Thin Arc adapters so one PawnioProvider instance serves both traits.
struct ArcSensor(std::sync::Arc<fancontrol_pawnio::PawnioProvider>);
struct ArcControl(std::sync::Arc<fancontrol_pawnio::PawnioProvider>);

impl fancontrol_plugins::SensorProvider for ArcSensor {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn sensors(&self) -> Vec<fancontrol_core::SensorDescriptor> {
        self.0.sensors()
    }
    fn read(&self, id: &SensorId) -> fancontrol_plugins::Result<f64> {
        self.0.read(id)
    }
}

impl fancontrol_plugins::ControlProvider for ArcControl {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn controls(&self) -> Vec<fancontrol_core::ControlDescriptor> {
        self.0.controls()
    }
    fn set_duty(&self, id: &ControlId, percent: u8) -> fancontrol_plugins::Result<()> {
        self.0.set_duty(id, percent)
    }
    fn get_duty(&self, id: &ControlId) -> fancontrol_plugins::Result<u8> {
        self.0.get_duty(id)
    }
}

fn main() -> anyhow::Result<()> {
    attach_parent_console_for_cli();

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    // Default product launch: hardware/host ON, mock OFF, PWM writes ON, subcommand UI.
    // --mock for dev sensors; --read-only / --no-hw for diagnostics.
    let include_mock = cli.mock && !cli.hw_only;
    let include_hw = !cli.no_hw;
    // Writes **enabled by default**. Only `--read-only` disables PWM.
    // `--allow-hw-write` remains valid for old scripts (no-op when already default-on).
    let allow_hw_write = !cli.read_only;
    let _ = cli.allow_hw_write;

    if allow_hw_write {
        tracing::warn!("hardware PWM writes enabled (default; use --read-only to disable)");
    }
    if include_mock {
        tracing::info!("mock provider enabled (--mock)");
    }

    // Double-click / bare `fancontrol-rs.exe` → desktop UI
    match cli.command.unwrap_or(Commands::Ui) {
        Commands::ListSensors => {
            let reg = build_registry(include_mock, include_hw, allow_hw_write);
            let map = ChannelMap::load_or_seed().unwrap_or_default();
            println!("Sensors:");
            for s in reg.all_sensors() {
                let value = reg.read_sensor(&s.id).ok();
                let unit = s.unit.as_deref().unwrap_or("");
                let val_str = value
                    .map(|v| format!("{v:.1}{unit}"))
                    .unwrap_or_else(|| "n/a".into());
                let label = map.sensor_name(s.id.as_str(), &s.name);
                println!(
                    "  [{kind:?}] {id} — {label} = {val}  (provider: {prov})",
                    kind = s.kind,
                    id = s.id,
                    val = val_str,
                    prov = s.provider,
                );
            }
            if reg.all_sensors().is_empty() {
                println!("  (none — try without --hw-only, or check backend-status)");
            }
        }
        Commands::ListControls => {
            let reg = build_registry(include_mock, include_hw, allow_hw_write);
            let map = ChannelMap::load_or_seed().unwrap_or_default();
            println!("Controls (duty is read-only current PWM %):");
            for c in reg.all_controls() {
                let duty = reg.get_duty(&c.id).ok();
                let duty_str = duty
                    .map(|d| format!("{d}%"))
                    .unwrap_or_else(|| "n/a".into());
                let label = map.control_name(c.id.as_str(), &c.name);
                println!(
                    "  {id} — {label}  duty={duty}  writable={writable}  (provider: {prov})",
                    id = c.id,
                    duty = duty_str,
                    writable = c.writable,
                    prov = c.provider,
                );
            }
        }
        Commands::Read { id } => {
            let reg = build_registry(include_mock, include_hw, false);
            let sid = SensorId::new(id);
            let v = reg.read_sensor(&sid)?;
            println!("{sid} = {v}");
        }
        Commands::SetDuty { id, percent } => {
            if id.starts_with("pawnio.") && !allow_hw_write {
                anyhow::bail!(
                    "refusing hardware write on {id}: --read-only is set.\n\
                     Drop --read-only to allow PWM, e.g.: fancontrol-rs set-duty {id} {percent}"
                );
            }
            if id.starts_with("pawnio.") {
                eprintln!(
                    "WARNING: writing real hardware duty on {id} → {percent}%. Ctrl+C now to abort…"
                );
                thread::sleep(Duration::from_millis(1500));
            }
            let reg = build_registry(include_mock, include_hw, allow_hw_write);
            let cid = ControlId::new(id);
            reg.set_duty(&cid, percent)?;
            let now = reg.get_duty(&cid)?;
            println!("Set {cid} → {now}%");
        }
        Commands::BackendStatus => {
            println!("{}", fancontrol_pawnio::status_message());
            println!("available={}", fancontrol_pawnio::is_available());
            let p = fancontrol_pawnio::try_provider();
            println!("\nProvider probe:\n{}", p.detection_report());
            println!("opened_devices={}", p.device_count());
            println!("hw_write_enabled={}", p.write_enabled());
        }
        Commands::SampleStorage { times, interval_ms } => {
            #[cfg(windows)]
            {
                use fancontrol_plugins::storage_win::diagnose_storage_temps;
                let n = times.max(1);
                for i in 0..n {
                    println!("=== sample-storage {}/{} ===", i + 1, n);
                    let rows = diagnose_storage_temps();
                    if rows.is_empty() {
                        println!("(no PhysicalDrive opened — run as Administrator, or no disks)");
                    }
                    for d in &rows {
                        println!(
                            "  drive{}  {}  chosen={:?} ({})  nvme={:?}  device_prop={:?}  adapter_prop={:?}",
                            d.index,
                            d.name,
                            d.chosen_c,
                            d.chosen_source.as_deref().unwrap_or("-"),
                            d.nvme_c,
                            d.device_prop_c,
                            d.adapter_prop_c,
                        );
                    }
                    if i + 1 < n {
                        thread::sleep(Duration::from_millis(interval_ms.max(500)));
                    }
                }
            }
            #[cfg(not(windows))]
            {
                anyhow::bail!("sample-storage is Windows-only");
            }
        }
        Commands::Detect => match fancontrol_pawnio::detect_chips() {
            Ok(chips) if chips.is_empty() => println!("No Super I/O chips detected."),
            Ok(chips) => {
                for c in chips {
                    println!(
                        "slot{} port=0x{:02X} chip={} hwm={:?}",
                        c.slot,
                        c.register_port,
                        c.chip.name(),
                        c.hwm_address.map(|a| format!("0x{a:04X}"))
                    );
                }
            }
            Err(e) => anyhow::bail!("detect failed: {e}"),
        },
        Commands::Sample {
            times,
            interval_ms,
            all,
        } => {
            let reg = build_registry(include_mock, include_hw, false);
            let map = ChannelMap::load_or_seed().unwrap_or_default();
            let times = times.max(1);
            for n in 0..times {
                if times > 1 {
                    println!("--- sample {}/{} ---", n + 1, times);
                }
                print_sample(&reg, &map, all);
                if n + 1 < times {
                    thread::sleep(Duration::from_millis(interval_ms));
                }
            }
        }
        Commands::Watch { interval_ms, all } => {
            let reg = build_registry(include_mock, include_hw, false);
            let map = ChannelMap::load_or_seed().unwrap_or_default();
            println!("Watching (read-only). Ctrl+C to stop.\n");
            loop {
                let ts = chrono_like_now();
                println!("======== {ts} ========");
                print_sample(&reg, &map, all);
                println!();
                thread::sleep(Duration::from_millis(interval_ms.max(200)));
            }
        }
        Commands::Demo { seconds, temp } => {
            run_demo(seconds, temp)?;
        }
        Commands::TestDuty {
            control,
            percent,
            hold_ms,
        } => {
            run_test_duty(
                &control,
                percent,
                hold_ms,
                include_mock,
                include_hw,
                allow_hw_write,
            )?;
        }
        Commands::Run {
            profile,
            seconds,
            interval_ms,
            apply,
        } => {
            let do_apply = apply;
            if do_apply && include_hw && !allow_hw_write {
                anyhow::bail!(
                    "refusing profile apply with hardware: drop --read-only and pass --apply. \
                     Default is dry-run / read-only."
                );
            }
            if do_apply && include_hw {
                eprintln!("WARNING: profile apply will write hardware duties. 1.5s to abort…");
                thread::sleep(Duration::from_millis(1500));
            }
            let reg = build_registry(include_mock, include_hw, allow_hw_write && do_apply);
            let profile = load_profile(&profile)?;
            let mut states: HashMap<String, CurveEvalState> = HashMap::new();
            let steps = seconds.max(1);
            let mode = if do_apply { "APPLY" } else { "DRY-RUN" };
            println!(
                "Running profile '{}' for {seconds}s (interval={interval_ms}ms mode={mode})",
                profile.name
            );
            for i in 0..steps {
                let mut temps = HashMap::new();
                for s in reg.all_sensors() {
                    if s.kind == SensorKind::Temperature {
                        if let Ok(v) = reg.read_sensor(&s.id) {
                            temps.insert(s.id.as_str().to_string(), v);
                        }
                    }
                }
                let step = evaluate_profile_step(&profile, &temps, &mut states);
                for err in &step.errors {
                    eprintln!("  warn: {err}");
                }
                for (ctrl, duty) in &step.duties {
                    if !do_apply {
                        println!("  t={i:03} {ctrl} → {duty}% (dry-run)");
                    } else {
                        match reg.set_duty(&ControlId::new(ctrl.clone()), *duty) {
                            Ok(()) => println!("  t={i:03} {ctrl} → {duty}%"),
                            Err(e) => eprintln!("  t={i:03} {ctrl} set failed: {e}"),
                        }
                    }
                }
                if step.duties.is_empty() {
                    println!("  t={i:03} no assignments applied (temps={temps:?})");
                }
                thread::sleep(Duration::from_millis(interval_ms));
            }
        }
        Commands::InitProfile { hw, id } => {
            let mut profile = Profile::new(&id, if hw { "Hardware default" } else { "Default" });
            profile.curves.push(FanCurve::linear(
                "quiet",
                "Quiet linear",
                30.0,
                75.0,
                25,
                100,
            ));
            if hw {
                // Matches owner NCT668x layout from validated sample.
                profile
                    .assignments
                    .insert("pawnio.0.ctrl0".into(), "quiet".into());
                profile
                    .sensor_bindings
                    .insert("pawnio.0.ctrl0".into(), "pawnio.0.temp.CPU".into());
                // Optional second channel if present
                profile
                    .assignments
                    .insert("pawnio.0.ctrl1".into(), "quiet".into());
                profile
                    .sensor_bindings
                    .insert("pawnio.0.ctrl1".into(), "pawnio.0.temp.CPU".into());
            } else {
                profile
                    .assignments
                    .insert("mock.cpu_fan".into(), "quiet".into());
                profile
                    .sensor_bindings
                    .insert("mock.cpu_fan".into(), "mock.cpu_temp".into());
            }
            let path = save_profile(&profile)?;
            println!("Wrote profile to {}", path.display());
            if hw {
                println!("Tip: dry-run with: cargo run -- --hw-only run --profile {id}");
            }
        }
        Commands::ListProfiles => match fancontrol_core::list_profiles() {
            Ok(ids) if ids.is_empty() => println!("No profiles saved yet. Try: init-profile"),
            Ok(ids) => {
                println!("Profiles:");
                for id in ids {
                    match load_profile(&id) {
                        Ok(p) => println!("  {} — {} ({} curves)", p.id, p.name, p.curves.len()),
                        Err(e) => println!("  {id} (error: {e})"),
                    }
                }
            }
            Err(e) => println!("Could not list profiles: {e}"),
        },
        Commands::MapInit { force } => {
            let path = if force {
                ChannelMap::write_seed()?
            } else {
                let (p, created) = ChannelMap::init_seed_if_missing()?;
                if !created {
                    println!(
                        "Map already exists at {} (use --force to overwrite)",
                        p.display()
                    );
                    return Ok(());
                }
                p
            };
            println!("Wrote channel map to {}", path.display());
            println!("Edit labels there; sample/ui will pick them up.");
        }
        Commands::Ui => {
            let opts = fancontrol_ui::UiOptions {
                include_mock,
                include_hw,
                allow_hw_write,
            };
            // Ensure seed map exists for labels
            let _ = ChannelMap::init_seed_if_missing();
            fancontrol_ui::run(opts).map_err(|e| anyhow::anyhow!(e))?;
        }
    }

    Ok(())
}

/// Re-attach to the launching terminal's console, if one exists, since this binary
/// is compiled as a GUI-subsystem app (so Windows never auto-allocates a console —
/// no flash on double-click). Double-click / Explorer launches have no parent
/// console, so `AttachConsole` simply fails and we proceed console-less into the UI.
#[cfg(windows)]
fn attach_parent_console_for_cli() {
    use windows_sys::Win32::System::Console::{
        AttachConsole, ATTACH_PARENT_PROCESS, STD_ERROR_HANDLE, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE,
    };

    // SAFETY: documented Win32 API; failure (no parent console) is a normal, expected
    // outcome for a double-click launch and simply means we proceed console-less.
    let attached = unsafe { AttachConsole(ATTACH_PARENT_PROCESS) } != 0;
    if !attached {
        return;
    }
    reopen_std_handle(STD_OUTPUT_HANDLE, "CONOUT$");
    reopen_std_handle(STD_ERROR_HANDLE, "CONOUT$");
    reopen_std_handle(STD_INPUT_HANDLE, "CONIN$");
}

/// Point a standard handle at the just-attached console, but only if it isn't
/// already valid — i.e. only when the shell didn't already redirect that stream to
/// a file or pipe (`> out.txt`, `| something`), which must be left untouched.
#[cfg(windows)]
fn reopen_std_handle(which: u32, file_name: &str) {
    use windows_sys::Win32::Foundation::{GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle};

    // SAFETY: `which` is one of the STD_*_HANDLE constants; this is a plain query.
    let current = unsafe { GetStdHandle(which) };
    if !current.is_null() && current != INVALID_HANDLE_VALUE {
        return;
    }
    let name: Vec<u16> = file_name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `name` is a valid null-terminated wide string naming the console stream
    // we just attached to; other args are the standard "open an existing handle" set.
    let handle = unsafe {
        CreateFileW(
            name.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        )
    };
    if handle != INVALID_HANDLE_VALUE {
        // SAFETY: `handle` was just successfully opened above.
        unsafe { SetStdHandle(which, handle) };
    }
}

#[cfg(not(windows))]
fn attach_parent_console_for_cli() {}

fn chrono_like_now() -> String {
    // Avoid extra dep: local time via system clock formatting.
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix={secs}")
}

fn print_sample(reg: &ProviderRegistry, map: &ChannelMap, show_all: bool) {
    println!("Temperatures:");
    let mut any_t = false;
    for s in reg.all_sensors() {
        if s.kind != SensorKind::Temperature {
            continue;
        }
        match reg.read_sensor(&s.id) {
            Ok(v) => {
                any_t = true;
                let label = map.sensor_name(s.id.as_str(), &s.name);
                println!("  {:>28}  {v:6.1} °C  ({label})", s.id);
            }
            Err(e) => {
                if show_all {
                    println!("  {:>28}  n/a ({e})", s.id);
                }
            }
        }
    }
    if !any_t {
        println!("  (none with valid readings)");
    }

    println!("Fan RPM:");
    let mut any_f = false;
    for s in reg.all_sensors() {
        if s.kind != SensorKind::FanRpm {
            continue;
        }
        match reg.read_sensor(&s.id) {
            Ok(v) => {
                if !show_all && v < 1.0 {
                    continue;
                }
                any_f = true;
                let label = map.sensor_name(s.id.as_str(), &s.name);
                println!("  {:>28}  {v:7.0} RPM  ({label})", s.id);
            }
            Err(e) => {
                if show_all {
                    println!("  {:>28}  n/a ({e})", s.id);
                }
            }
        }
    }
    if !any_f {
        println!("  (none with valid readings)");
    }

    println!("Control duty (read):");
    let mut any_c = false;
    for c in reg.all_controls() {
        match reg.get_duty(&c.id) {
            Ok(d) => {
                if !show_all && d == 0 {
                    continue;
                }
                any_c = true;
                let label = map.control_name(c.id.as_str(), &c.name);
                let rpm = c.rpm_sensor.as_ref().map(|s| s.as_str()).unwrap_or("-");
                println!(
                    "  {:>28}  {d:3}%  {label}  writable={}  rpm={rpm}",
                    c.id, c.writable
                );
            }
            Err(e) => {
                if show_all {
                    println!("  {:>28}  n/a ({e})", c.id);
                }
            }
        }
    }
    if !any_c {
        println!("  (none with non-zero duty — use --all to list zeros)");
    }
}

fn run_test_duty(
    control: &str,
    percent: u8,
    hold_ms: u64,
    include_mock: bool,
    include_hw: bool,
    allow_hw_write: bool,
) -> anyhow::Result<()> {
    if control.starts_with("pawnio.") && !allow_hw_write {
        anyhow::bail!(
            "refusing hardware test-duty: --read-only is set (or writes disabled).\n\
             Example:\n  fancontrol-rs --hw-only test-duty --control {control} --percent {percent}"
        );
    }
    let percent = percent.min(100);
    let reg = build_registry(include_mock, include_hw, allow_hw_write);
    let cid = ControlId::new(control);

    let ctrl_meta = reg
        .all_controls()
        .into_iter()
        .find(|c| c.id.as_str() == control);
    let rpm_id = ctrl_meta.and_then(|c| c.rpm_sensor.clone());

    let baseline_duty = reg.get_duty(&cid)?;
    let baseline_rpm = rpm_id.as_ref().and_then(|id| reg.read_sensor(id).ok());

    println!("test-duty on {control}");
    println!("  baseline duty={baseline_duty}%  rpm={baseline_rpm:?}");
    println!("  will set {percent}% for {hold_ms}ms then restore {baseline_duty}%");
    eprintln!("WARNING: real PWM write in 2s — Ctrl+C to abort…");
    thread::sleep(Duration::from_secs(2));

    reg.set_duty(&cid, percent)?;
    // NCT668x PWM-out register can lag one EC poll after a command write.
    thread::sleep(Duration::from_millis(100));
    let after = reg.get_duty(&cid)?;
    println!("  applied duty={after}% (target {percent}%)");

    let steps = (hold_ms / 500).max(1);
    for i in 0..steps {
        thread::sleep(Duration::from_millis(500));
        let d = reg.get_duty(&cid).unwrap_or(0);
        let rpm = rpm_id.as_ref().and_then(|id| reg.read_sensor(id).ok());
        println!("  hold {i}: duty={d}% rpm={rpm:?}");
    }

    println!("  restoring baseline {baseline_duty}%…");
    reg.set_duty(&cid, baseline_duty)?;
    thread::sleep(Duration::from_millis(500));
    let restored = reg.get_duty(&cid)?;
    let rpm_restored = rpm_id.as_ref().and_then(|id| reg.read_sensor(id).ok());
    println!("  restored duty={restored}% rpm={rpm_restored:?}");
    if restored != baseline_duty {
        eprintln!("warn: restored duty {restored}% != baseline {baseline_duty}%");
    }
    Ok(())
}

fn run_demo(seconds: u64, start_temp: f64) -> anyhow::Result<()> {
    let mock = MockProvider::new();
    mock.set_sensor_value("mock.cpu_temp", start_temp);

    let mut reg = ProviderRegistry::new();
    reg.register_both(mock.clone());

    let curve = FanCurve::linear("demo", "Demo", 30.0, 80.0, 20, 100);
    let mut state = CurveEvalState::default();
    let control = ControlId::new("mock.cpu_fan");
    let sensor = SensorId::new("mock.cpu_temp");

    println!(
        "Demo: applying curve '{}' for {seconds}s (start temp={start_temp}°C) [mock only]",
        curve.name
    );

    let steps = seconds.max(1);
    for i in 0..steps {
        let t = start_temp + (i as f64) * 2.5 - if i > steps / 2 { 8.0 } else { 0.0 };
        mock.set_sensor_value("mock.cpu_temp", t);
        let temp = reg.read_sensor(&sensor)?;
        let duty = evaluate_curve(&curve, temp, Some(&mut state));
        reg.set_duty(&control, duty)?;
        let rpm = reg
            .read_sensor(&SensorId::new("mock.cpu_fan_rpm"))
            .unwrap_or(0.0);
        println!("  t={i:02}  temp={temp:5.1}°C  duty={duty:3}%  rpm={rpm:.0}");
        thread::sleep(Duration::from_millis(400));
    }
    Ok(())
}
