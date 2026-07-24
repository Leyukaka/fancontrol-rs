//! fancontrol-rs — modern fan control for Windows
//!
//! Early scaffolding. Nothing hardware-related is implemented yet.

fn main() -> anyhow::Result<()> {
    // Basic logging setup
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    tracing::info!("fancontrol-rs starting...");
    tracing::info!("This is currently just a skeleton. Hardware backend & UI coming soon.");

    // TODO:
    // 1. Initialize PawnIO backend
    // 2. Discover sensors & controls
    // 3. Launch UI (egui/eframe or iced)
    // 4. Load user profiles / fan curves

    Ok(())
}
