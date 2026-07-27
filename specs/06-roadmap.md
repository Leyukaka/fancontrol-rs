# Roadmap

Last aligned with product reality: **v0.2.0** (2026-07).

## Phase 0 — Foundation

- [x] Repository + Spec-Driven Design documents (`specs/`)
- [x] Workspace structure (`crates/`)
- [x] UI technology decision (egui + eframe)
- [x] `fancontrol-core` domain models + curve engine + profile JSON
- [x] Plugin traits + mock provider
- [x] CLI harness (`list-sensors`, `set-duty`, `demo`, …)
- [x] CI (GitHub Actions)

## Phase 1 — Hardware & Core

- [x] `fancontrol-pawnio` FFI + embedded LpcIO/Echo modules
- [x] Super I/O detect + Nuvoton NCT668x EC HWM (temps/fans/PWM)
- [x] Curve evaluation + control loop helper
- [x] Profile serialization (JSON)
- [x] CLI: `detect`, `backend-status`, `sample`, `watch`, `test-duty`, `map-init`, `ui`, …
- [x] Elevated read/write validation (owner NCT6687D-class)
- [x] Host sensors: GPU (fixed-path `nvidia-smi`), storage (`DeviceIoControl`, no PowerShell)
- [ ] Broader chip productization (ITE IT87, banked NCT — experimental detect only)
- [ ] Confirm SSD/NVMe temps on real hardware (`storage_win.rs`)

## Phase 2 — UI MVP

- [x] Main window: sensors + controls (egui 0.35)
- [x] Manual duty sliders (write-gated / product default writes on)
- [x] Channel map + rename
- [x] Curve editor MVP + curve auto-apply (“Curve control”)
- [x] Multi-sensor graph (`egui_plot`) + sensor picker + per-control curve binding
- [x] Graph window 10/20/30/60 min + sample interval
- [x] Shader / gallery graph styles (`shaders/`)
- [x] PawnIO install/admin dialog
- [x] Profile UX (switch / last-used / startup)
- [x] System tray
- [x] i18n (8 languages)

## Phase 3 — Polish, packaging, security

- [x] Public GitHub repo + CONTRIBUTING + hardware/security docs
- [x] Release workflow (`v*.*.*` → exe + SHA256; environment approval for owner)
- [x] CodeQL, cargo-audit, Dependabot
- [x] Branch protection + tag ruleset + release environment
- [x] Manual update check (GitHub latest-release compare + link)
- [ ] Code signing (SignPath / Azure / cert — **not** enabled)
- [ ] Installer / end-user packaging polish
- [ ] Full curve editor polish (multi-curve UX)
- [ ] Dynamic plugin loading infrastructure
- [ ] Decide mock-in-default-UI vs `--hw-only` product default

## Phase 4 — Advanced

- [ ] Multi-sensor curves (e.g. max CPU+GPU as single input)
- [ ] Additional official plugins
- [ ] Auto-update: download + SHA256 verify + install
- [ ] AMD/Intel GPU temperature sensors — see `docs/GPU_VENDOR_APIS.md`

## Next priorities (order)

1. Broader chip validation (community logs + experimental paths)
2. SSD/NVMe temp validation on real hardware
3. Code signing when ready for wider audience
4. Auto-update download + SHA256
5. Mock opt-in for shipped product defaults (if desired)
6. RGB remains **future / out of Super I/O fan HWM scope**

## Out of scope for v1

- Linux support
- RGB as part of Super I/O fan control
- Cloud / remote control
- Bundling PawnIO inside the app binary
- Claiming signed binaries before signing is wired
