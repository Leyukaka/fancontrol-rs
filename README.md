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

## Docs

| Document | Description |
|----------|-------------|
| [CONTRIBUTING.md](./CONTRIBUTING.md) | Bugs, PRs, AI contribution policy |
| [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md) | Chip matrix, prerequisites, validation |
| [docs/SIGNING_AND_DISTRIBUTION.md](./docs/SIGNING_AND_DISTRIBUTION.md) | Signing options, release checklist, CI notes |

## Status

🚧 **Early stage** — Phase 1 hardware (PawnIO / NCT668x) + Phase 2 UI (live sensors, sliders, curve editor MVP, temp graph).

| Crate | Status |
|-------|--------|
| `fancontrol-core` | Models, curves, profiles, channel map, control loop |
| `fancontrol-plugins` | Traits + mock + host sensors |
| `fancontrol-pawnio` | PawnIO FFI + LpcIO + NCT668x EC HWM |
| `fancontrol-ui` | egui live + sliders + curve editor + graph |
| `fancontrol-rs` (CLI) | sample, watch, test-duty, ui, map-init, … |

Validated hardware summary: **Nuvoton NCT6687D-class** EC (id `0xD5` rev `0x92` @ `0x0A20`) — reads and PWM writes via PawnIO. Details: [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md).

## Download

- **GitHub Releases** — when a version tag `v*.*.*` is pushed, [`.github/workflows/release.yml`](./.github/workflows/release.yml) builds `fancontrol-rs.exe` (and a SHA256 file) on `windows-latest`. See the [Releases](https://github.com/Leyukaka/fancontrol-rs/releases) page once tags exist.
- **Build from source** (always available) — see below.

> **Binaries are not code-signed yet.** Expect SmartScreen “unknown publisher” and possible Defender false positives. We do **not** claim signed releases. Roadmap for signing: [docs/SIGNING_AND_DISTRIBUTION.md](./docs/SIGNING_AND_DISTRIBUTION.md).

Release workflows are plain **YAML** under `.github/workflows/` (required by GitHub Actions). They are kept **minimal** on purpose — not a cargo-dist-style generator.

## Prerequisites (hardware)

1. **[PawnIO](https://pawnio.eu/)** — **required** for Super I/O / EC access. It is a **prerequisite**, **not bundled** with fancontrol-rs. Install the official package first. If PawnIO is missing, the app still starts (mock / config / host sensors); the UI shows a clear message / popup when the hardware backend is unavailable.
2. **Administrator** — `pawnio_open` needs an elevated process for live hardware.
3. Compatible board path — today, **NCT668x EC** is the validated path; see [docs/SUPPORTED_HARDWARE.md](./docs/SUPPORTED_HARDWARE.md).

## Building

```bash
cargo build --release
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
```

Release binary: `target/release/fancontrol-rs.exe`.

## Windows Defender / SmartScreen

This app talks to **PawnIO** (kernel I/O). Like FanControl / LibreHardwareMonitor, Windows Defender may flag the **unsigned** debug/release binary as a false positive — especially with `--allow-hw-write` (real PWM control).

**Do not disable Defender entirely.** Prefer a folder exclusion for local development:

1. Windows Security → Virus & threat protection → Manage settings  
2. **Exclusions** → Add exclusion → **Folder**  
3. Add at least:
   - `C:\projet\fancontrol-rs\target`  
   - optionally the whole repo `C:\projet\fancontrol-rs`  
4. If it already quarantined the exe: Protection history → restore `fancontrol-rs.exe`

Also run an **elevated** (Admin) terminal when using hardware, or PawnIO open fails with access denied.

Signing + installer (later) should reduce SmartScreen friction for end users — not configured yet.

## Safety

- **Never** WinRing0 or known-vulnerable ring-0 drivers.
- Hardware is **read-only by default**. PWM writes require explicit `--allow-hw-write`.
- Prefer `sample` / `list-sensors` / `list-controls` for validation; do not write casually.

## CLI (current harness)

```bash
cargo run -- list-sensors
cargo run -- list-controls
cargo run -- sample
cargo run -- map-init
cargo run -- demo --seconds 5
cargo run -- backend-status
cargo run -- init-profile --hw
```

## UI (egui)

```bash
# Mock only (no admin, rarely flagged)
cargo run -- --no-hw ui

# Live hardware (Administrator recommended)
cargo run -- --hw-only ui

# Live + PWM sliders enabled (most likely to trip Defender)
cargo run -- --hw-only --allow-hw-write ui
```

Rename fans/sensors: edit `channel-map.json` under the app config dir (`map-init` creates it).

## Contributing

See **[CONTRIBUTING.md](./CONTRIBUTING.md)** — issue-first for non-trivial work, quality gates (fmt/clippy/tests), hardware safety, and a **strict AI disclosure** policy.

## License

MIT OR Apache-2.0

## Credits

- Inspired by the excellent work of Rémi Mercier ([@Rem0o](https://github.com/Rem0o)) and the LibreHardwareMonitor team.
- PawnIO by [namazso](https://github.com/namazso).
