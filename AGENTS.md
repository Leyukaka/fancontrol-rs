# AGENTS.md — fancontrol-rs

Instructions for **coding agents** and maintainers working in this repository.
Read this file, [`CONTRIBUTING.md`](./CONTRIBUTING.md), and `specs/` before non-trivial changes.

## What this project is

Modern Windows fan control app in Rust — spiritual successor to [FanControl (Rem0o)](https://github.com/Rem0o/FanControl.Releases).
Hardware access via **PawnIO only** (never WinRing0 / unsigned vulnerable drivers).
Spec-Driven Design: product and architecture decisions live in `specs/`.

## Prefer durable project files

- Prefer `specs/`, `docs/`, code, and commits over chat-only context.
- Leave the tree **buildable** and documented after significant work.
- After meaningful status changes, update this file’s **Status** section and/or `specs/06-roadmap.md`.

## Environment (critical)

Hardware sensors and PawnIO need a **real Windows host** (not a disposable Linux container as the only target).

- **Do not** install software, drivers, packages (`winget` / `choco` / MSI), or system-wide tools without **explicit user approval**.
- Usually fine without asking: `cargo build`, `cargo test`, `cargo clippy`, `cargo run` (app under test), local git inspect.
- **Ask first** when in doubt: installs, admin elevation, service changes, `git push`, force/destructive git, deleting unrelated files, remote state changes.
- Before privileged actions: *Do I have permission on this machine?* If unsure → ask.

Optional maintainer-only preferences may live in an untracked `AGENTS.local.md` (gitignored). Do not commit personal machine paths or secrets there.

## Architecture (workspace)

| Crate | Role |
|-------|------|
| `fancontrol-core` | Domain: Sensor/Control/Curve/Profile, curve eval, profile JSON |
| `fancontrol-plugins` | `SensorProvider` / `ControlProvider` traits, mock + host sensors |
| `fancontrol-pawnio` | PawnIO backend (FFI + LpcIO + NCT668x EC) |
| `fancontrol-ui` | Desktop UI **egui + eframe** (live, sliders, curve editor, graph) |
| `fancontrol-rs` | Binary: CLI + `ui` |

Config dir: `%APPDATA%` via `directories` → project `fancontrol-rs` (`profiles/*.json`, `channel-map.json`, `ui-settings.json`).

## Status (keep updated)

- Product line **v0.1.4** (public repo). Specs under `specs/` aligned with this status.
- Phase 0 foundation: **done**.
- Phase 1: PawnIOLib FFI + LpcIO + **NCT668x EC HWM** (validated class id `0xD5` rev `0x92` @ `0x0A20`) + banked NCT path (experimental) + control loop + CLI.
- Elevation required for `pawnio_open` (Administrator). **PawnIO is a prerequisite** (not bundled) — UI shows a startup dialog if missing / not openable.
- Validated path: **Nuvoton NCT6687D-class**; **ctrl0–3** reliable PWM; higher controls = DR/experimental. See `docs/SUPPORTED_HARDWARE.md`.
- Host sensors: fixed-path `nvidia-smi` (NVIDIA-only; AMD/Intel research documented in `docs/GPU_VENDOR_APIS.md`, not implemented) + storage via `DeviceIoControl` with an NVMe health-log fallback (read-only, no PowerShell, no PATH walk for GPU).
- Vendored modules: `crates/fancontrol-pawnio/modules/` (PawnIO.Modules).
- UI: **egui/eframe 0.35** — live sensors, sliders, curve editor, curve auto-apply, CPU graph windows, rename map, options, system tray (minimize-to-tray, state icon, quick menu), profile switch/save persisted as last-used and auto-loaded on startup, manual "Check for updates" (GitHub latest-release compare + link, no auto-download).
- Binary is GUI-subsystem (no console flash on launch); CLI usage from an existing terminal re-attaches to it automatically.
- Packaging / sec: release workflow + owner `release` environment approval; CodeQL + cargo-audit + Dependabot; unsigned exe + SHA256. Signing later — `docs/SIGNING_AND_DISTRIBUTION.md`.

## Next priorities (order)

1. Broader chip validation (IT87 / banked NCT still experimental).
2. Code signing (SmartScreen) — see `docs/SIGNING_AND_DISTRIBUTION.md`.
3. Auto-update: download + SHA256 verify + install (manual check already done) — see `docs/SECURITY.md`.
4. AMD/Intel GPU temp — blocked on hardware to validate against, see `docs/GPU_VENDOR_APIS.md`.
5. RGB (future — not Super I/O).

## Safety product rules

1. Never embed or ship WinRing0 or known-vulnerable ring-0 drivers.
2. Graceful degrade if PawnIO missing (clear UI message; app still starts for mock/config).
3. **PWM writes are on by default** for the product UI. Use `--read-only` for diagnostics. Prefer elevated process for real HWM.
4. Prefer `sample` / `list-sensors` / `list-controls` for validation; never write casually.
5. **Windows Defender** may false-positive unsigned builds that call PawnIO. Do **not** disable AV; document folder exclusion for `target\` (see README). Do not change Defender settings without explicit user approval.

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -- list-sensors
cargo run -- backend-status
cargo run -- --no-hw ui
```

## Specs map

| Doc | Content |
|-----|---------|
| `specs/00-overview.md` | Vision |
| `specs/01-product.md` | Requirements |
| `specs/02-architecture.md` | Crates / data flow |
| `specs/03-hardware-backend.md` | PawnIO |
| `specs/04-ui.md` | UI (egui) |
| `specs/05-plugins.md` | Plugin traits |
| `specs/06-roadmap.md` | Phases |

Also read: `CONTRIBUTING.md`, `docs/SUPPORTED_HARDWARE.md`.
