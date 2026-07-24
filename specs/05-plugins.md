# Plugin System Specification

## Goals

- Allow third parties (and us) to add new sensors and controls without modifying the core
- Keep the core small and stable

## Core Traits (planned)

```rust
pub trait SensorProvider {
    fn name(&self) -> &str;
    fn sensors(&self) -> Vec<SensorDescriptor>;
    fn read(&self, id: &str) -> Result<f64>;
}

pub trait ControlProvider {
    fn name(&self) -> &str;
    fn controls(&self) -> Vec<ControlDescriptor>;
    fn set_duty(&self, id: &str, percent: u8) -> Result<()>;
    fn get_duty(&self, id: &str) -> Result<u8>;
}
```

## Loading strategy (v1)

- Official providers compiled into the binary or loaded as dynamic libraries
- Simple discovery via a `plugins/` directory
- Manifest file (TOML/JSON) describing the plugin

## Official plugins (planned)

- PawnIO SuperIO / EC provider (core backend)
- Possible future: HWInfo shared memory, NVIDIA / AMD GPU helpers, etc.

## Compatibility note

The original FanControl has a .NET plugin API. We do **not** aim for binary compatibility with those plugins. We design our own clean Rust-first API.
