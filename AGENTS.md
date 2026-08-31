# AGENTS.md - fancontrol-rs

Instructions for **coding agents** and maintainers working in this repository.
Read this file, [`CONTRIBUTING.md`](./CONTRIBUTING.md), and `specs/` before non-trivial changes.

A maintainer-only private context checkout may exist as a sibling of this repo. It is **not required** to build or contribute. Do not invent maintainer process that is not written here or in `specs/`.

## What this project is

Modern Windows fan control app in Rust - spiritual successor to [FanControl (Rem0o)](https://github.com/Rem0o/FanControl.Releases).
Hardware access via **PawnIO only** (never WinRing0 / unsigned vulnerable drivers).
Spec-Driven Design: product and architecture decisions live in `specs/`.

## Prefer durable project files

- Prefer `specs/`, `docs/`, code, and commits over chat-only context.
- Leave the tree **buildable** and documented after significant work.
- After meaningful status changes, update this file’s **Status** section and/or `specs/06-roadmap.md`.

## Workflow (current phase: solo project)

No PR process for now: commit and push straight to `main` (a local feature branch is still fine for a large/risky change, but merge it yourself, no `gh pr create`). Still run `cargo test`/`clippy`/`fmt` before pushing, CI runs on `main` pushes too. `CONTRIBUTING.md`'s PR template, AI-disclosure section, and review rules are written for when this project opens up to outside contributors; they don't apply to day-to-day solo work right now.

### Commit messages (mandatory for agents)

- **Never** add AI trailers or co-author spam: no `Co-Authored-By:`, `Co-authored-by:`, `Generated-by:`, `Signed-off-by:` for models, or similar.
- Commits are authored as the human maintainer only. Do not advertise the tool in the commit body.

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
| `fancontrol-core` | Domain: Sensor/Control/Curve/Profile, curve eval, profile JSON, `MetricSample` |
| `fancontrol-plugins` | `SensorProvider` / `ControlProvider` traits, mock + host sensors |
| `fancontrol-pawnio` | PawnIO backend (FFI + LpcIO + NCT668x EC) |
| `fancontrol-metrics` | Metric sinks: MultiSink, SQLite, CSV export, OTEL (v0.4) |
| `fancontrol-ui` | Desktop UI **egui + eframe** (live, sliders, curve editor, graph) |
| `fancontrol-rs` | Binary: CLI + `ui` |

Config dir: `%APPDATA%` via `directories` → project `fancontrol-rs` (`profiles/*.json`, `channel-map.json`, `ui-settings.json`).

Rust edition 2024, stable toolchain pinned via `rust-toolchain.toml` (rustfmt + clippy components).

## Status (keep updated)

- Product line: **v0.5.3**. Roadmap and feature checklist: `specs/06-roadmap.md`.
- Hardware backend: PawnIO + NCT668x (validated `0xD5` / banked `0xD4`) + host GPU/storage + CPU package power + DDR5 DIMM temps. Details: `docs/SUPPORTED_HARDWARE.md`, `docs/DIMM_TEMP.md`, `specs/03-hardware-backend.md`.
- UI: egui 0.36, domain panels, multi-kind graph, activity deck, i18n (8 locales), manual update check only. `specs/04-ui.md`.
- Metrics store (SQLite/CSV) shipped; OTLP/HTTP export opt-in. `specs/07-metrics-telemetry.md`.
- PawnIO is a prerequisite (not bundled). Elevation via UAC **Restart as Administrator** only (no silent elevation). PWM writes on by default; `--read-only` for diagnostics.

## Next priorities (order)

1. Broader chip validation (ITE IT87 still experimental; more banked NCT boards).
2. SSD/NVMe temp validation on more drives.
3. Code signing (SmartScreen) - see `docs/SIGNING_AND_DISTRIBUTION.md`.
4. Optional later: download + SHA256 after **manual** check only (no silent auto-update) - see `docs/SECURITY.md`.
5. AMD/Intel GPU sensors - blocked on hardware; see `docs/GPU_VENDOR_APIS.md`. Optional later: experimental NvAPI Hot Spot (not shipped).
6. RGB (future - not Super I/O).
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
# Faster optimized local builds (no full LTO). Ship with --release only.
cargo run --profile release-fast -- ui
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
| `specs/07-metrics-telemetry.md` | Metrics store, graph multi-kind, OTEL |

Also read: `CONTRIBUTING.md`, `docs/SUPPORTED_HARDWARE.md`.
