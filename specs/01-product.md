# Product Specification

## Goal

Replace FanControl with a modern, secure, open-source alternative written in Rust while keeping a high level of power and usability.

## Must-have features (v1)

### Sensors & Controls

| Capability | Status (v0.1.x) |
|------------|-----------------|
| Discover / list temps, fans, controls | **Done** (PawnIO NCT668x + mock + host) |
| Motherboard Super I/O / EC via PawnIO | **Done** for NCT668x EC + banked NCT on ROG B550-A (validated); other chips/boards experimental until certified |
| Real-time temps + RPM | **Done** |
| Manual duty % control | **Done** (CLI + UI sliders; write-gated) |
| GPU / storage host sensors (read-only) | **Done** (`nvidia-smi` multi-metric + GPU panel; SSD via `DeviceIoControl`, no PowerShell) |
| GPU detail panel (power / util / clocks / VRAM) | **Done** (v0.3.5; Hot Spot shown unavailable without NvAPI) |

### Fan Curves

| Capability | Status |
|------------|--------|
| Create / edit / delete curves | **Done** (core + UI editor MVP) |
| Multi-point temp → duty, linear interp | **Done** |
| Assign curve to controls | **Done** (profile assignments; curve input = **CPU-like** temp only) |
| Hysteresis | **Done** (core + UI field) |
| Auto-apply curves to hardware | **Done** (UI “Curve control”; writes on by default, `--read-only` disables) |

### Profiles

| Capability | Status |
|------------|--------|
| Save / load JSON profiles | **Done** (core + UI list/save) |
| Quick switch | **Done** (UI profile dropdown) |
| Auto-apply last profile on startup | **Done** (last switched-to/saved profile persisted in `ui-settings.json`, auto-loaded next launch) |

### UI

| Capability | Status |
|------------|--------|
| Overview sensors + controls | **Done** |
| Visual curve editor | **Done** (MVP; polish ongoing) |
| Live temp graph | **Done** (CPU; windows 10/20/30/60 min + sample rate) |
| Dark theme default | **Done** |
| System tray | **Done** |
| Rename channels | **Done** (`channel-map.json`) |
| Missing-PawnIO popup | **Done** |
| Localization (8 languages: en/fr/de/es/it/zh/ja/lb) | **Done** (`rust-i18n`, picker in Options panel, OS-locale default) |
| Manual update check (GitHub latest-release compare + link) | **Done** |

### Reliability

| Capability | Status |
|------------|--------|
| Graceful degrade without PawnIO | **Done** (mock/config/host; UI dialog) |
| Clear unsupported / admin messaging | **Done** / polish ongoing |
| No silent PWM writes | **Done** (writes on by default with clear UI banner + startup log warning; `--read-only` opts out) |

## Nice-to-have (v1.x / v2)

- Multi-sensor curves (e.g. max of CPU + GPU)
- External sensor sources (HWInfo shared memory, more plugins)
- Scheduling / time-based profiles
- **In-app auto-update**: download + SHA256 verify + install (manual "check for updates" already shipped — see UI table above)
- Remote monitoring (optional)
- Linux support
- Authenticode **code signing** (docs ready; not wired)

## Non-goals (for now)

- RGB control (**planned later** — separate subsystem, not Super I/O)
- Overclocking
- Cross-platform parity in v1
- Bundling PawnIO inside the app binary
- Shipping WinRing0 or disabling AV as a product requirement
