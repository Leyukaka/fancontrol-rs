<p align="center">
  <img src="assets/logo.svg" alt="fancontrol-rs logo" width="160">
</p>

<h1 align="center">fancontrol-rs</h1>

<p align="center">
  <strong>Windows fan control and system activity (CPU / RAM) in Rust.</strong><br>
  Spiritual successor to <a href="https://github.com/Rem0o/FanControl.Releases">FanControl</a> by Rem0o.<br>
  PawnIO only (no WinRing0) · profiles & curves · live UI
</p>

<p align="center">
  <img src="https://img.shields.io/badge/rust-edition%202021-orange?logo=rust&logoColor=white" alt="Rust">
  <img src="https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue" alt="License">
  <img src="https://img.shields.io/badge/version-0.3.0-informational" alt="Version">
  <img src="https://img.shields.io/badge/backend-PawnIO-success" alt="PawnIO">
  <img src="https://img.shields.io/badge/platform-Windows%2010%2F11-blue" alt="Windows">
</p>

<p align="center">
  <img src="assets/screenshot.jpg" alt="fancontrol-rs UI: temps, fans, Activity deck, curves" width="920">
</p>

## Features

- **Fan control**: curves, profiles, live duty sliders, multi-sensor temperature graph
- **Activity deck**: CPU load history + top processes (CPU % and RAM), filter and sort (default on, can be turned off in Options)
- **Security**: [PawnIO](https://pawnio.eu/) only for Super I/O / EC. Never ships WinRing0 or other known-vulnerable ring-0 drivers
- **Host sensors**: NVIDIA GPU temp via fixed-path `nvidia-smi`, SSD/NVMe temps via DeviceIoControl (no PowerShell)
- **UI**: egui desktop app, system tray, first-run write consent, 8 languages (en/fr/de/es/it/zh/ja/lb)
- **CLI**: `sample`, `list-sensors`, `list-controls`, `test-duty`, `sample-storage`, …
- **Optional fun**: shader graph styles (wgpu) in Options
- Spec-driven design: decisions live under [`specs/`](./specs)

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

## Docs

| Document | Description |
|----------|-------------|
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Bugs, PRs, AI contribution policy |
| [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md) | Chip matrix, prerequisites, validation |
| [docs/SIGNING_AND_DISTRIBUTION.md](./docs/SIGNING_AND_DISTRIBUTION.md) | Signing options, release checklist, CI notes |
| [docs/SECURITY.md](./docs/SECURITY.md) | Reporting, CodeQL/audit, SHA256 verify, signing/auto-update status |

## Status

**v0.3.0**. NCT668x path + full UI (sensors, sliders, curves, thermal graph, Activity deck).  
Public source of truth: this repo ([Releases](https://github.com/Leyukaka/fancontrol-rs/releases), issues, PRs).

| Crate | Role |
|-------|------|
| `fancontrol-core` | Models, curves, profiles, channel map, control loop |
| `fancontrol-plugins` | Traits, mock, host sensors, CPU activity |
| `fancontrol-pawnio` | PawnIO FFI + LpcIO + NCT668x EC HWM |
| `fancontrol-ui` | egui app |
| `fancontrol-rs` | CLI + binary entry |

Validated hardware: **Nuvoton NCT6687D-class** EC (id `0xD5` rev `0x92` @ `0x0A20`). Reads and PWM writes via PawnIO.  
Details: [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md).

## Download

- **[GitHub Releases](https://github.com/Leyukaka/fancontrol-rs/releases)**: tags `v*.*.*` build `fancontrol-rs.exe` + SHA256 on `windows-latest` (see [release workflow](./.github/workflows/release.yml)).
- **Build from source**: see below.

Binaries are **not code-signed yet**. Expect SmartScreen and occasional Defender false positives. Signing plan: [docs/SIGNING_AND_DISTRIBUTION.md](./docs/SIGNING_AND_DISTRIBUTION.md).

## Prerequisites (hardware)

1. **[PawnIO](https://pawnio.eu/)** is required for Super I/O / EC access. Not bundled: install the official package first. Without it the app still starts (mock / config / host sensors); the UI explains if the backend cannot open.
2. **Administrator**: live HWM / `pawnio_open` needs elevation.
3. Compatible board: **NCT668x EC** is the validated path; see [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md).

## Building

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Release binary: `target/release/fancontrol-rs.exe`.

## Windows Defender / SmartScreen / VirusTotal

The app talks to **PawnIO** (kernel I/O), may spawn **nvidia-smi** for GPU temps, and reads SSD temps via native storage APIs (no PowerShell). Unsigned builds that touch hardware drivers often get heuristic flags (same class of tools as FanControl / LibreHardwareMonitor).

- Prefer official Releases and verify **SHA256**: [docs/SECURITY.md](./docs/SECURITY.md).
- PWM writes are on by default; `--read-only` is quieter for scanners and safer for diagnostics.

**Do not disable Defender.** For local builds, exclude the folder:

1. Windows Security → Virus & threat protection → Manage settings  
2. **Exclusions** → Add exclusion → **Folder**  
3. Add `C:\projet\fancontrol-rs\target` (or the whole repo if needed)  
4. If quarantined: Protection history → restore `fancontrol-rs.exe`

Use an **elevated** terminal for real hardware, or PawnIO open fails with access denied.

## Safety

- **Never** WinRing0 or known-vulnerable ring-0 drivers.
- PWM writes are **on by default** (UI + curve control). Use `--read-only` when you only want sensors.
- Prefer `sample` / `list-sensors` / `list-controls` before writing casually.

## CLI

```bash
cargo run -- list-sensors
cargo run -- list-controls
cargo run -- sample
cargo run -- sample-storage --times 3
cargo run -- map-init
cargo run -- backend-status
cargo run -- init-profile --hw
```

## UI

Bare exe (or no subcommand) opens the **UI** with PWM writes and curve control on (first-run confirmation). Mock channels need `--mock`.

```bash
cargo run --release
# same as: cargo run --release -- ui

cargo run -- --mock --no-hw ui
cargo run -- --hw-only ui
cargo run -- --read-only ui
```

Rename channels: `channel-map.json` in the app config dir (`map-init` creates it).

## Contributing

Bug reports and PRs welcome. Read **[CONTRIBUTING.md](./CONTRIBUTING.md)** first (issue-first for non-trivial work, quality gates, hardware safety, AI disclosure).

## License

MIT OR Apache-2.0

## Credits

- Inspired by Rémi Mercier ([@Rem0o](https://github.com/Rem0o)) and LibreHardwareMonitor.
- PawnIO by [namazso](https://github.com/namazso).

## Support

[![Buy Me A Coffee](https://img.shields.io/badge/Buy%20Me%20a%20Coffee-ffdd00?style=for-the-badge&logo=buy-me-a-coffee&logoColor=black)](https://buymeacoffee.com/leyukaka)
