//! fancontrol-rs — modern fan control for Windows.
//!
//! Phase 0/1 harness: CLI against mock (+ optional PawnIO stub status).
//! Full GUI arrives in Phase 2.

use clap::{Parser, Subcommand};
use fancontrol_core::{
    evaluate_curve, load_profile, save_profile, ControlId, CurveEvalState, FanCurve, Profile,
    SensorId, SensorKind,
};
use fancontrol_plugins::{MockProvider, ProviderRegistry};
use std::thread;
use std::time::Duration;

#[derive(Parser, Debug)]
#[command(
    name = "fancontrol-rs",
    about = "Modern fan control for Windows (Rust + PawnIO)",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// List discovered sensors (mock by default)
    ListSensors,
    /// List controllable fans
    ListControls,
    /// Read a sensor value by id
    Read {
        /// Sensor id (e.g. mock.cpu_temp)
        id: String,
    },
    /// Set fan duty cycle (0-100)
    SetDuty {
        /// Control id (e.g. mock.cpu_fan)
        id: String,
        /// Duty percent 0..=100
        percent: u8,
    },
    /// Show PawnIO backend status
    BackendStatus,
    /// Run a demo loop: evaluate a curve against mock CPU temp and apply duty
    Demo {
        /// Seconds to run (default 10)
        #[arg(long, default_value_t = 10)]
        seconds: u64,
        /// Starting mock CPU temperature
        #[arg(long, default_value_t = 40.0)]
        temp: f64,
    },
    /// Save a sample default profile to the user config directory
    InitProfile,
    /// List saved profile ids
    ListProfiles,
    /// Launch the GUI (Phase 2 — not implemented yet)
    Ui,
}

fn build_registry(include_mock: bool) -> ProviderRegistry {
    let mut reg = ProviderRegistry::new();
    if include_mock {
        reg.register_both(MockProvider::new());
    }
    // PawnIO stub is registered for status only; it exposes no sensors yet.
    // When real bindings land, register PawnioProvider the same way.
    reg
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let cli = Cli::parse();

    match cli.command.unwrap_or(Commands::ListSensors) {
        Commands::ListSensors => {
            let reg = build_registry(true);
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
                println!("  (none)");
            }
        }
        Commands::ListControls => {
            let reg = build_registry(true);
            println!("Controls:");
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
            let reg = build_registry(true);
            let sid = SensorId::new(id);
            let v = reg.read_sensor(&sid)?;
            println!("{sid} = {v}");
        }
        Commands::SetDuty { id, percent } => {
            let reg = build_registry(true);
            let cid = ControlId::new(id);
            reg.set_duty(&cid, percent)?;
            let now = reg.get_duty(&cid)?;
            println!("Set {cid} → {now}%");
        }
        Commands::BackendStatus => {
            println!("{}", fancontrol_pawnio::status_message());
            println!("available={}", fancontrol_pawnio::is_available());
        }
        Commands::Demo { seconds, temp } => {
            run_demo(seconds, temp)?;
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
        Commands::ListProfiles => {
            match fancontrol_core::list_profiles() {
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
            }
        }
        Commands::Ui => {
            if fancontrol_ui::is_implemented() {
                fancontrol_ui::run().map_err(|e| anyhow::anyhow!(e))?;
            } else {
                anyhow::bail!(
                    "UI not implemented yet (Phase 2). Use list-sensors / demo / set-duty for now."
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
        "Demo: applying curve '{}' for {seconds}s (start temp={start_temp}°C)",
        curve.name
    );
    println!("  curve points: {:?}", curve.points);

    let steps = seconds.max(1);
    for i in 0..steps {
        // Simulate temperature climbing then falling a bit
        let t = start_temp + (i as f64) * 2.5 - if i > steps / 2 { 8.0 } else { 0.0 };
        mock.set_sensor_value("mock.cpu_temp", t);
        let temp = reg.read_sensor(&sensor)?;
        let duty = evaluate_curve(&curve, temp, Some(&mut state));
        reg.set_duty(&control, duty)?;
        let rpm = reg
            .read_sensor(&SensorId::new("mock.cpu_fan_rpm"))
            .unwrap_or(0.0);
        println!(
            "  t={i:02}  temp={temp:5.1}°C  duty={duty:3}%  rpm={rpm:.0}"
        );
        thread::sleep(Duration::from_millis(400));
    }

    // Also print temperature sensors summary
    println!("\nFinal sensors:");
    for s in reg.all_sensors() {
        if s.kind == SensorKind::Temperature || s.kind == SensorKind::FanRpm {
            if let Ok(v) = reg.read_sensor(&s.id) {
                println!("  {} = {v:.1}", s.id);
            }
        }
    }
    Ok(())
}
