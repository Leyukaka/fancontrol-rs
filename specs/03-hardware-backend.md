# Hardware Backend Specification

## Primary Backend: PawnIO

We use **PawnIO** (https://pawnio.eu/) as the sole privileged hardware access layer.

### Why PawnIO?
- Modern replacement for the vulnerable WinRing0 driver
- Scriptable (Pawn language) → safer and more maintainable
- Already adopted by recent LibreHardwareMonitor / FanControl versions
- Avoids the need to sign and maintain our own kernel driver

### Responsibilities of `fancontrol-pawnio`

- Detect if PawnIO is installed and running
- Load required PawnIO modules (SuperIO, EC, etc.)
- Expose a clean Rust API:
  - `list_sensors() -> Vec<Sensor>`
  - `list_controls() -> Vec<Control>`
  - `read_sensor(id) -> f64`
  - `set_control_duty(id, percent: u8)`
- Handle errors cleanly when PawnIO is missing or modules fail

### Fallback behavior

If PawnIO is not available:
- Show a clear message in the UI with a link to install it
- Still allow the application to start (for configuration / plugin testing)
- Sensors/controls from other plugins remain available

## Secondary sources (via plugins)

- Manufacturer-specific APIs (Dell, HP, Lenovo, ASUS, etc.)
- GPU vendor libraries when useful
- External tools via plugins (HWInfo shared memory, etc.)

## Non-negotiable rules

1. Never embed or ship WinRing0 or any known-vulnerable driver.
2. Never require the user to disable Secure Boot or load unsigned drivers for core functionality.
3. Prefer read-only access when write is not needed.
