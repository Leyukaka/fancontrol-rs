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
- **Propose questions** when blocked or when a decision has lasting product impact (don’t silently invent product scope).
- **Never claim “terminé / done”** unless verified (build/tests/smoke ran, or uncertainty is stated clearly).
- After context compaction, re-read this file + `specs/06-roadmap.md` + latest commits.

## Architecture (workspace)

| Crate | Role |
|-------|------|
| `fancontrol-core` | Domain: Sensor/Control/Curve/Profile, curve eval, profile JSON |
| `fancontrol-plugins` | `SensorProvider` / `ControlProvider` traits, `MockProvider`, registry |
| `fancontrol-pawnio` | PawnIO backend (**stub** until real bindings) |
| `fancontrol-ui` | Desktop UI (**stub**; locked to **egui + eframe**) |
| `fancontrol-rs` | Binary: CLI harness now; wires UI later |

Config dir: `%APPDATA%` via `directories` → project `fancontrol-rs` (`profiles/*.json`).

## Status (keep updated)

- Phase 0 foundation: **done** (workspace, core, mock, CLI, CI, UI decision locked).
- Phase 1 next: real PawnIO integration + control loop; UI Phase 2 after or in parallel against mock.
- CLI: `list-sensors`, `list-controls`, `read`, `set-duty`, `demo`, `backend-status`, `init-profile`, `list-profiles`, `ui` (stub).
- On owner machine: `backend-status` reported **PawnIO appears installed** (path probe only; bindings not implemented).

## Next priorities (order)

1. Research/implement real PawnIO bindings (FFI/IPC) — ask before installing PawnIO modules or elevating.
2. Wire PawnioProvider into CLI; keep mock for CI/dev without hardware.
3. Continuous control loop: read temp → `evaluate_curve` → `set_duty`.
4. UI MVP (egui): sensor/control lists, duty slider, basic curve editor, tray later.
5. Polish / plugins dynamic loading / packaging — later.

## Safety product rules

1. Never embed or ship WinRing0 or known-vulnerable ring-0 drivers.
2. Graceful degrade if PawnIO missing (clear message, app still starts for config/mock).
3. Prefer read-only hardware access until write path is validated on real fans.

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
