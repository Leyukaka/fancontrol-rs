# Plugin System Specification

## Goals

- Allow third parties (and us) to add sensors and controls without forking core logic
- Keep the core small and stable

## Core traits (implemented)

Located in `fancontrol-plugins` (IDs use core `SensorId` / `ControlId` types):

```rust
pub trait SensorProvider {
    fn name(&self) -> &str;
    fn sensors(&self) -> Vec<SensorDescriptor>;
    fn read(&self, id: &SensorId) -> Result<f64>;
}

pub trait ControlProvider {
    fn name(&self) -> &str;
    fn controls(&self) -> Vec<ControlDescriptor>;
    fn set_duty(&self, id: &ControlId, percent: u8) -> Result<()>;
    fn get_duty(&self, id: &ControlId) -> Result<u8>;
}
```

`ProviderRegistry` aggregates multiple providers for CLI and UI.

## Loading strategy

| Phase | Strategy |
|-------|----------|
| **v0.1.x (now)** | Official providers **compiled into** the binary |
| **Later** | Optional dynamic libraries + manifest under a `plugins/` directory |

### Built-in providers today

| Provider | Crate / module | Role |
|----------|----------------|------|
| Mock | `fancontrol-plugins::MockProvider` | Dev / `--no-hw` |
| Host | `fancontrol-plugins::HostSensorProvider` | GPU + storage temps (read-only) |
| PawnIO | `fancontrol-pawnio::PawnioProvider` | Super I/O / NCT668x HWM |

## Official plugins (future)

- HWInfo shared memory
- AMD GPU helpers
- OEM-specific APIs (Dell, HP, Lenovo, ASUS, …) when justified

## Compatibility note

The original FanControl has a .NET plugin API. We do **not** aim for binary compatibility with those plugins. We design a clean Rust-first API.

## Safety

- Dynamic plugins must not weaken `--allow-hw-write` or introduce WinRing0.
- Contribution of a new chip/provider requires machine validation notes (see CONTRIBUTING + SUPPORTED_HARDWARE).
