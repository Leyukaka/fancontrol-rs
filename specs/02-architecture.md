# Architecture

## High-level design

```
┌─────────────────────────────────────────────────────┐
│                    fancontrol-rs                    │
│                   (main binary)                     │
└───────────────────────┬─────────────────────────────┘
                        │
        ┌───────────────┼───────────────┐
        │               │               │
        ▼               ▼               ▼
┌───────────────┐ ┌──────────────┐ ┌────────────────┐
│ fancontrol-ui │ │fancontrol-   │ │ fancontrol-    │
│   (egui/iced) │ │   core       │ │   plugins      │
└───────────────┘ └──────┬───────┘ └───────┬────────┘
                         │                 │
                         ▼                 ▼
                ┌─────────────────┐  ┌─────────────┐
                │fancontrol-pawnio│  │ 3rd-party   │
                │  (backend)      │  │ plugins     │
                └─────────────────┘  └─────────────┘
```

## Crates (planned workspace)

| Crate | Responsibility |
|-------|----------------|
| `fancontrol-core` | Domain models (Sensor, Control, Curve, Profile), business logic, curve evaluation |
| `fancontrol-pawnio` | Low-level hardware access via PawnIO |
| `fancontrol-plugins` | Plugin loading, trait definitions, discovery |
| `fancontrol-ui` | Desktop UI |
| `fancontrol-rs` | Binary that wires everything together |

## Key design decisions

### 1. Backend: PawnIO (not custom driver)
- We deliberately refuse to ship a custom WinRing0-style driver.
- PawnIO is the current best secure alternative used by modern LibreHardwareMonitor builds.
- All Super I/O / EC / MSR access goes through PawnIO modules.

### 2. Plugin system
- Plugins can provide additional sensors and controls.
- Core defines traits: `SensorProvider`, `ControlProvider`.
- Plugins are loaded dynamically (or compiled in for official ones).

### 3. UI choice (locked)
- **egui + eframe** for v1 (see `specs/04-ui.md`).

### 4. Data flow
```
Hardware (PawnIO / Plugins)
        ↓
   Sensor / Control values
        ↓
   Core (curves evaluation)
        ↓
   UI (display + user input)
        ↓
   Core (apply new duties)
```

## Configuration

- Profiles and curves stored as JSON (or TOML) in user config directory.
- Location: `%APPDATA%/fancontrol-rs/` on Windows.
