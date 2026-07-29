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

## Workflow (current phase: solo project)

No PR process for now: commit and push straight to `main` (a local feature branch is still fine for a large/risky change, but merge it yourself, no `gh pr create`). Still run `cargo test`/`clippy`/`fmt` before pushing, CI runs on `main` pushes too. `CONTRIBUTING.md`'s PR template, AI-disclosure section, and review rules are written for when this project opens up to outside contributors; they don't apply to day-to-day solo work right now.

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

- Product line: **v0.3.0** (Activity deck: CPU load + top processes CPU/RAM opt-in; SSD temps IOCTL; mock opt-in; writes consent; host toggle live).
- Phase 0 foundation: **done**.
- Phase 1: PawnIOLib FFI + LpcIO + **NCT668x EC HWM** (validated class id `0xD5` rev `0x92` @ `0x0A20`) + banked NCT path (experimental) + control loop + CLI.
- Elevation required for `pawnio_open` (Administrator). **PawnIO is a prerequisite** (not bundled) — UI shows a startup dialog if missing / not openable.
- Validated path: **Nuvoton NCT6687D-class**; **ctrl0–3** reliable PWM; higher controls = DR/experimental. See `docs/SUPPORTED_HARDWARE.md`.
- Host sensors: fixed-path `nvidia-smi` (NVIDIA-only; AMD/Intel research documented in `docs/GPU_VENDOR_APIS.md`, not implemented) + storage via `DeviceIoControl` with an NVMe health-log fallback (read-only, no PowerShell, no PATH walk for GPU).
- Vendored modules: `crates/fancontrol-pawnio/modules/` (PawnIO.Modules).
- i18n: 8 languages (en/fr/de/es/it/zh/ja/lb) via `rust-i18n`, picker in Options panel, OS-locale default on first run, live switch (no restart), Noto Sans CJK bundled for zh/ja glyph coverage. `crates/fancontrol-ui/locales/`.
- Fun extra: optional fractal pyramid panel (raymarched, GLSL→WGSL port) rendered via a custom wgpu pipeline through `egui_wgpu::CallbackTrait` — first custom wgpu callback in this codebase (`crates/fancontrol-ui/src/fractal.rs` + `fractal_shader.wgsl`). Toggle + speed + 2 colors in Options panel; off by default.
- UI: **egui/eframe 0.35** — live sensors, sliders, curve editor, curve auto-apply, graph windows, rename map, options, system tray (minimize-to-tray, state icon, quick menu), profile switch/save persisted as last-used and auto-loaded on startup, manual "Check for updates" (GitHub latest-release compare + link, no auto-download).
- Graph: multi-sensor (pick any combination of live sensors in Options, ordered `graph_sensor_ids`, categorical color legend once >1 is plotted), per-control curve sensor binding next to the curve-assignment combo (defaults unchanged, so untouched controls behave exactly as before), "hide controls at 0% duty" option. Rendered via **`egui_plot`** (0.36, the release that pairs with `egui` 0.35, not 0.35.0 which pairs with `egui` 0.34) instead of a hand-rolled painter — the old custom fill polygon (`egui::Shape::convex_polygon`) fanned triangles from the oldest sample, which is only correct for a convex area and produced spike artifacts on any real (concave) trace. `crates/fancontrol-ui/src/graph.rs`.
- **Activity deck** (v0.3.0, Options toggle, **default on**): CPU load sparkline (0–100 %, X anchored to last sample) + top processes with CPU % and RAM, sort CPU/RAM, name filter. Load-only mode skips process scan. Windows APIs only — no PowerShell, no WMI, no process kill.
- Binary is GUI-subsystem (no console flash on launch); CLI usage from an existing terminal re-attaches to it automatically.
- Packaging / sec: release workflow + owner `release` environment approval; CodeQL + cargo-audit + Dependabot; unsigned exe + SHA256. Signing later — `docs/SIGNING_AND_DISTRIBUTION.md`.

## Next priorities (order)

1. Broader chip validation (IT87 / banked NCT still experimental).
2. Code signing (SmartScreen) — see `docs/SIGNING_AND_DISTRIBUTION.md`.
3. Auto-update: download + SHA256 verify + install (manual check already done) — see `docs/SECURITY.md`.
4. AMD/Intel GPU temp — blocked on hardware to validate against, see `docs/GPU_VENDOR_APIS.md`.
5. RGB (future — not Super I/O).
6. SSD/NVMe temps: validated under load on owner hardware (device_prop live); EVO SATA may report no temp.
7. Mock is **opt-in** (`--mock`); product default is hardware/host only.

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
