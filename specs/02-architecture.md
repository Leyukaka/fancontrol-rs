# Architecture

## High-level design

```
┌─────────────────────────────────────────────────────┐
│                    fancontrol-rs                    │
│              (CLI + ui subcommand binary)           │
└───────────────────────┬─────────────────────────────┘
                        │
        ┌───────────────┼───────────────┐
        │               │               │
        ▼               ▼               ▼
┌───────────────┐ ┌──────────────┐ ┌────────────────┐
│ fancontrol-ui │ │fancontrol-   │ │ fancontrol-    │
│  (egui 0.35)  │ │   core       │ │   plugins      │
└───────────────┘ └──────┬───────┘ └───────┬────────┘
                         │                 │
                         ▼                 ▼
                ┌─────────────────┐  ┌─────────────┐
                │fancontrol-pawnio│  │ Host / mock │
                │  (PawnIO HWM)   │  │  providers  │
                └─────────────────┘  └─────────────┘
```

## Crates (workspace — current)

| Crate | Responsibility |
|-------|----------------|
| `fancontrol-core` | Domain models (Sensor, Control, Curve, Profile), curve eval, channel map, config paths |
| `fancontrol-plugins` | Traits (`SensorProvider` / `ControlProvider`), registry, mock, host sensors |
| `fancontrol-pawnio` | PawnIOLib FFI, LpcIO modules, Super I/O detect, NCT668x EC HWM |
| `fancontrol-ui` | Desktop UI (egui + eframe **0.35**): live view, sliders, curves, graph, options |
| `fancontrol-rs` | Binary: CLI harness + `ui` subcommand |

Workspace version: **0.1.5** (see root `Cargo.toml`).

## Key design decisions

### 1. Backend: PawnIO (not custom driver)

- Refuse to ship WinRing0-style vulnerable drivers.
- PawnIO is a system prerequisite (install from https://pawnio.eu/) — **not** embedded in the exe.
- Super I/O / EC access via vendored LpcIO module blobs + Rust control plane.

### 2. Plugin / provider system

- Traits live in `fancontrol-plugins`.
- **v1 shipping model**: official providers **compiled in** (mock, host, pawnio). Dynamic `.dll` plugins later.
- Host sensors avoid PowerShell: GPU via fixed-path `nvidia-smi`; storage via `DeviceIoControl` temperature property.

### 3. UI choice (locked for v1)

- **egui + eframe 0.35** (see `specs/04-ui.md`).
- Hardware PWM writes **on by default** (product UI/CLI); UI reflects read-only vs write-enabled. Use `--read-only` to disable.
- Localization: 8 languages via `rust-i18n` (`crates/fancontrol-ui/locales/`), OS-locale default, live switch.
- First custom wgpu rendering: the optional fractal-fun panel (`crates/fancontrol-ui/src/fractal.rs`) uses `egui_wgpu::CallbackTrait` to run its own render pipeline inside an egui panel, rather than egui's own 2D vector painter used everywhere else. This is a precedent for any future GPU-accelerated widget — reuse the same `FractalResources`-in-`callback_resources` pattern (one-time pipeline setup via `cc.wgpu_render_state`, per-frame `prepare`/`paint`) rather than inventing a new one.

### 4. Concurrency

- UI poll thread for HWM `sample_all` batch reads.
- Write queue off the UI thread for PWM.
- Process-level ISA / SIO locking in pawnio path (see AGENTS / code).

### 5. Data flow

```
Hardware (PawnIO) + Host (nvidia-smi / storage IOCTL) + Mock
        ↓
   Sensor / Control values
        ↓
   Core (curve evaluation, profiles)
        ↓
   UI (display + user input) / CLI
        ↓
   Write queue → ControlProvider::set_duty  (only if allow_hw_write)
```

## Configuration (Windows)

Under the `directories` crate project dir for `fancontrol-rs` (typically under `%APPDATA%`):

| File | Purpose |
|------|---------|
| `channel-map.json` | Display names for sensors/controls |
| `ui-settings.json` | Graph window/sample rate, hide 0 RPM, curve auto-apply, etc. |
| `profiles/*.json` | Fan profiles |

## Distribution

- GitHub Actions CI (fmt, clippy, test, release build) + CodeQL + cargo-audit + Dependabot.
- Tag `v*.*.*` → Release workflow (environment **`release`**: owner approval) publishes `fancontrol-rs.exe` + `.sha256`.
- Unsigned binaries today; signing roadmap in `docs/SIGNING_AND_DISTRIBUTION.md`.
