# fancontrol-rs

**A modern, full-featured fan control application for Windows written in Rust.**

Spiritual successor to [FanControl](https://github.com/Rem0o/FanControl.Releases) by Rem0o.

> This project is developed using **Spec-Driven Design**.  
> All important decisions and requirements live in the [`specs/`](./specs) folder.  
> Read those documents before writing code.

## Specs

| Document | Description |
|----------|-------------|
| [00-overview](./specs/00-overview.md) | Vision & principles |
| [01-product](./specs/01-product.md) | Product requirements |
| [02-architecture](./specs/02-architecture.md) | Architecture & crates |
| [03-hardware-backend](./specs/03-hardware-backend.md) | PawnIO & hardware access |
| [04-ui](./specs/04-ui.md) | UI requirements |
| [05-plugins](./specs/05-plugins.md) | Plugin system |
| [06-roadmap](./specs/06-roadmap.md) | Development phases |

## Status

🚧 **Early stage** — Specs complete, Phase 0 foundation in progress.

| Crate | Status |
|-------|--------|
| `fancontrol-core` | Models, curve engine, profile JSON |
| `fancontrol-plugins` | Traits + mock provider |
| `fancontrol-pawnio` | Stub (availability probe only) |
| `fancontrol-ui` | Stub (egui locked, not implemented) |
| `fancontrol-rs` (CLI) | `list-sensors`, `set-duty`, `demo`, … |

## Building

```bash
cargo build --release
cargo test
```

## CLI (current harness)

```bash
cargo run -- list-sensors
cargo run -- list-controls
cargo run -- set-duty mock.cpu_fan 45
cargo run -- demo --seconds 5
cargo run -- backend-status
cargo run -- init-profile
```

## License

MIT OR Apache-2.0

## Credits

- Inspired by the excellent work of Rémi Mercier ([@Rem0o](https://github.com/Rem0o)) and the LibreHardwareMonitor team.
- PawnIO by [namazso](https://github.com/namazso).
