# AGENTS.md — fancontrol-rs

Instructions for **any** coding agent (Grok Build, Claude Code, Codex, Cursor, etc.).
Read this file and `specs/` before non-trivial changes.

## What this project is

Modern Windows fan control app in Rust — spiritual successor to [FanControl (Rem0o)](https://github.com/Rem0o/FanControl.Releases).
Hardware access via **PawnIO only** (never WinRing0 / unsigned vulnerable drivers).
Spec-Driven Design: product/architecture decisions live in `specs/`.

## Multi-agent / multi-tool

- Owner works with **Grok Build (code)** and **Claude Code** on the same repo.
- Prefer durable project files (`AGENTS.md`, `specs/`, code, commits) over chat-only context.
- Do not assume exclusive ownership of the working tree; leave it buildable and documented.
- After significant work: update this file’s **Status** section if reality changed.

## Environment (critical)

- Agents run on the **real Windows host** (not WSL, not a disposable container), because hardware sensors / PawnIO need the real machine.
- **Never install software, drivers, packages (winget/choco/msi), or system-wide tools without explicit user approval.**
- Allowed without asking: `cargo build`, `cargo test`, `cargo clippy`, `cargo run` (app under test), `git status/diff/log/add/commit` when asked or when finishing a clear unit of work the user requested.
- **Ask first** when in doubt: installs, admin elevation, service install, PawnIO module side-effects you’re unsure about, `git push`, force/destructive git, deleting files outside intentional cleanup, network calls that change remote state.
- Before any action: *Do I have the right to do this on a real machine?* If unsure → ask.

## Working style (owner preferences)

- Communicate in **French** with the owner.
- **Code freely** for project edits once a direction is clear — don’t interrupt for every small file change.
- Switch to **agent/implementation mode** as soon as the task is clear; don’t stay in endless planning.
- **Questions**: only on real blockers, risky permissions (install/admin/destructive/remote), or lasting product decisions. **Propose** them briefly — no cosmetic “shall I…?” when the next step is already implied. Don’t invent product scope silently.
- **Never claim “terminé / done”** unless verified (build/tests/smoke ran, or uncertainty is stated clearly).
- After context compaction, re-read this file + `specs/06-roadmap.md` + latest commits.

## Architecture (workspace)

| Crate | Role |
|-------|------|
| `fancontrol-core` | Domain: Sensor/Control/Curve/Profile, curve eval, profile JSON |
| `fancontrol-plugins` | `SensorProvider` / `ControlProvider` traits, `MockProvider`, registry |
| `fancontrol-pawnio` | PawnIO backend (FFI + LpcIO + NCT668x EC) |
| `fancontrol-ui` | Desktop UI **egui + eframe** (live, sliders, curve editor, graph) |
| `fancontrol-rs` | Binary: CLI + `ui` |

Config dir: `%APPDATA%` via `directories` → project `fancontrol-rs` (`profiles/*.json`).

## Status (keep updated)

- Phase 0 foundation: **done**.
- Phase 1: PawnIOLib FFI + LpcIO + **NCT668x EC HWM** (owner chip id=0xD5 rev=0x92 @ 0x0A20) + banked NCT path (experimental) + control loop + CLI.
- Elevation required for `pawnio_open` (admin terminal). **PawnIO is a prerequisite** (not bundled).
- Owner board: **Nuvoton NCT6687D-class** slot1 @0x4E hwm=0x0A20.
- **Reads validated (2026-07-24):** CPU~56°C, System~36, fans 0/1/12/13/14 live, ctrl0~38% ctrl1~54%.
- **Writes validated (2026-07-24):** `test-duty pawnio.0.ctrl1` 54%→40%→54% with RPM 2150→~1826→~1954. NCT6687D EC write path OK. **ctrl0–3 reliable**; higher ctrls = DR/experimental path.
- Host sensors: nvidia-smi + PowerShell storage (read-only).
- Vendored modules: `crates/fancontrol-pawnio/modules/` (PawnIO.Modules 0.2.9).
- CLI: `list-sensors`, `list-controls`, `sample`, `watch`, `test-duty`, `map-init`, `ui`, …
- UI: live + sliders + curve editor MVP + CPU graph; `cargo run -- ui` / `--hw-only ui` / `--allow-hw-write ui`
- Channel map: `map-init` → `channel-map.json`
- Docs/packaging: `CONTRIBUTING.md`, `docs/SUPPORTED_HARDWARE.md`, `docs/SIGNING_AND_DISTRIBUTION.md`, `.github/workflows/release.yml` (unsigned tag builds; no signing yet).

## Next priorities (order)

1. System tray.
2. Broader chip validation (IT87 / banked NCT still experimental).
3. Profile UX polish in UI.
4. Code signing later (SmartScreen) — see `docs/SIGNING_AND_DISTRIBUTION.md`.
5. RGB (future — not Super I/O).

## Safety product rules

1. Never embed or ship WinRing0 or known-vulnerable ring-0 drivers.
2. Graceful degrade if PawnIO missing (clear message, app still starts for config/mock).
3. **Hardware is read-only by default.** PWM writes require explicit `--allow-hw-write`.
4. Prefer `sample` / `list-sensors` / `list-controls` for validation; never write casually.
5. **Windows Defender** may false-positive unsigned builds that call PawnIO. Do **not** disable AV;
   document folder exclusion for `target\` (see README). Agents must not disable Defender or add
   exclusions without explicit owner approval (system change).

## Commands

```bash
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo run -- list-sensors
cargo run -- demo --seconds 5
cargo run -- backend-status
```

## Specs map

| Doc | Content |
|-----|---------|
| `specs/00-overview.md` | Vision |
| `specs/01-product.md` | Requirements |
| `specs/02-architecture.md` | Crates / data flow |
| `specs/03-hardware-backend.md` | PawnIO |
| `specs/04-ui.md` | UI (egui locked) |
| `specs/05-plugins.md` | Plugin traits |
| `specs/06-roadmap.md` | Phases |
