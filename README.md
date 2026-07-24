# fancontrol-rs

**A modern, full-featured fan control application for Windows written in Rust.**

Spiritual successor to [FanControl](https://github.com/Rem0o/FanControl.Releases) by Rem0o.

## Goals

- Full-featured desktop application (fan curves, sensors, graphs, profiles, plugins...)
- Secure hardware access via **PawnIO** (no more vulnerable WinRing0)
- Clean, modular architecture with plugin support
- Pure Rust where possible
- Modern, responsive UI

## Status

🚧 **Very early stage** — scaffolding only.

## Architecture (planned)

```
fancontrol-rs/
├── crates/
│   ├── fancontrol-core/     # Core logic, sensors, controls, curves
│   ├── fancontrol-ui/       # Desktop UI (egui or iced)
│   ├── fancontrol-plugins/  # Plugin system
│   └── fancontrol-pawnio/   # PawnIO backend bindings
├── src/                     # Main binary
└── plugins/                 # Example / official plugins
```

### Hardware backend

We intentionally avoid shipping a custom ring-0 driver.
Primary backend will be [PawnIO](https://pawnio.eu/) (the modern replacement used by recent LibreHardwareMonitor / FanControl versions).

Fallback / complementary sources:
- Manufacturer APIs (Dell, HP, Lenovo...)
- Existing plugins model inspired by FanControl

## Building

```bash
cargo build --release
```

> Requires a recent Rust toolchain (edition 2024 / 1.85+ recommended).

## License

MIT OR Apache-2.0

## Credits

- Inspired by the excellent work of Rémi Mercier ([@Rem0o](https://github.com/Rem0o)) and the LibreHardwareMonitor team.
- PawnIO by [namazso](https://github.com/namazso).
