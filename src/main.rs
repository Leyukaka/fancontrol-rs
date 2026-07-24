//! fancontrol-rs — modern fan control for Windows.
//!
//! CLI harness: mock and/or real PawnIO Super I/O sensors.
//! Full GUI arrives in Phase 2.

use clap::{Parser, Subcommand};
use fancontrol_core::{
    evaluate_curve, evaluate_profile_step, load_profile, save_profile, ControlId, CurveEvalState,
    FanCurve, Profile, SensorId, SensorKind,
};
use fancontrol_plugins::{MockProvider, ProviderRegistry};
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
    /// Include mock sensors/controls (default: on unless --hw-only)
    #[arg(long, global = true)]
    mock: bool,

    /// Do not register mock provider
    #[arg(long, global = true)]
    hw_only: bool,

    /// Skip probing PawnIO Super I/O
    #[arg(long, global = true)]
    no_hw: bool,

    /// Allow real hardware PWM writes (default: **off**, read-only).
    /// Required for `set-duty` / `run` against pawnio.* controls.
    #[arg(long, global = true)]
    allow_hw_write: bool,

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
    /// Hardware (`pawnio.*`) requires global `--allow-hw-write`. Mock always allowed.
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
    /// Read-only snapshot: temps + fan RPMs + current duties (no writes)
    Sample {
        /// How many times to sample
        #[arg(long, default_value_t = 1)]
        times: u32,
        /// Delay between samples in ms
        #[arg(long, default_value_t = 1000)]
        interval_ms: u64,
    },
    /// Demo curve on mock (does not touch hardware fans)
    Demo {
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        #[arg(long, default_value_t = 40.0)]
        temp: f64,
    },
    /// Apply a saved profile in a loop.
    /// Default is **dry-run** (compute duties only). Real set_duty needs
    /// `--allow-hw-write --apply` together.
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
        /// Actually call set_duty (requires --allow-hw-write for pawnio)
        #[arg(long)]
        apply: bool,
    },
    /// Save a sample default profile (mock bindings)
    InitProfile,
    /// List saved profile ids
    ListProfiles,
    /// Launch the GUI (Phase 2 — not implemented yet)
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
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();
    // Default: mock ON (safe) + hardware probe ON unless --no-hw / --hw-only.
    // Hardware PWM writes OFF unless --allow-hw-write.
    let include_mock = !cli.hw_only;
    let include_hw = !cli.no_hw;
    let allow_hw_write = cli.allow_hw_write;
    let _ = cli.mock; // reserved: force-mock flag for future

    if allow_hw_write {
        tracing::warn!("--allow-hw-write is set: real fan PWM writes are permitted");
    }

    match cli.command.unwrap_or(Commands::ListSensors) {
        Commands::ListSensors => {
            let reg = build_registry(include_mock, include_hw, allow_hw_write);
            println!("Sensors:");
            for s in reg.all_sensors() {
                let value = reg.read_sensor(&s.id).ok();
                let unit = s.unit.as_deref().unwrap_or("");
                let val_str = value
                    .map(|v| format!("{v:.1}{unit}"))
                    .unwrap_or_else(|| "n/a".into());
                println!(
                    "  [{kind:?}] {id} — {name} = {val}  (provider: {prov})",
                    kind = s.kind,
                    id = s.id,
                    name = s.name,
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
            println!("Controls (duty is read-only current PWM %):");
            for c in reg.all_controls() {
                let duty = reg.get_duty(&c.id).ok();
                let duty_str = duty
                    .map(|d| format!("{d}%"))
                    .unwrap_or_else(|| "n/a".into());
                println!(
                    "  {id} — {name}  duty={duty}  writable={writable}  (provider: {prov})",
                    id = c.id,
                    name = c.name,
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
                    "refusing hardware write on {id}: read-only by default.\n\
                     When ready, re-run with: cargo run -- --allow-hw-write set-duty {id} {percent}"
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
        Commands::Detect => {
            match fancontrol_pawnio::detect_chips() {
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
            }
        }
        Commands::Sample { times, interval_ms } => {
            let reg = build_registry(include_mock, include_hw, false);
            let times = times.max(1);
            for n in 0..times {
                if times > 1 {
                    println!("--- sample {}/{} ---", n + 1, times);
                }
                println!("Temperatures:");
                for s in reg.all_sensors() {
                    if s.kind == SensorKind::Temperature {
                        match reg.read_sensor(&s.id) {
                            Ok(v) => println!("  {:>28}  {v:6.1} °C  ({})", s.id, s.name),
                            Err(e) => println!("  {:>28}  n/a ({e})", s.id),
                        }
                    }
                }
                println!("Fan RPM:");
                for s in reg.all_sensors() {
                    if s.kind == SensorKind::FanRpm {
                        match reg.read_sensor(&s.id) {
                            Ok(v) => println!("  {:>28}  {v:7.0} RPM  ({})", s.id, s.name),
                            Err(e) => println!("  {:>28}  n/a ({e})", s.id),
                        }
                    }
                }
                println!("Control duty (read):");
                for c in reg.all_controls() {
                    match reg.get_duty(&c.id) {
                        Ok(d) => println!("  {:>28}  {d:3}%  writable={}", c.id, c.writable),
                        Err(e) => println!("  {:>28}  n/a ({e})", c.id),
                    }
                }
                if n + 1 < times {
                    thread::sleep(Duration::from_millis(interval_ms));
                }
            }
        }
        Commands::Demo { seconds, temp } => {
            run_demo(seconds, temp)?;
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
                    "refusing profile apply with hardware: pass --allow-hw-write together with --apply. \
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
        Commands::InitProfile => {
            let mut profile = Profile::new("default", "Default");
            profile.curves.push(FanCurve::linear(
                "quiet",
                "Quiet linear",
                30.0,
                75.0,
                25,
                100,
            ));
            profile
                .assignments
                .insert("mock.cpu_fan".into(), "quiet".into());
            profile
                .sensor_bindings
                .insert("mock.cpu_fan".into(), "mock.cpu_temp".into());
            let path = save_profile(&profile)?;
            println!("Wrote profile to {}", path.display());
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
        Commands::Ui => {
            if fancontrol_ui::is_implemented() {
                fancontrol_ui::run().map_err(|e| anyhow::anyhow!(e))?;
            } else {
                anyhow::bail!(
                    "UI not implemented yet (Phase 2). Use list-sensors / demo / run for now."
                );
            }
        }
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
