# Roadmap

Last aligned with product reality: **v0.1.5** (2026-07) — note `main` has since moved past this: i18n shipped under tag `v0.1.6-i18n-beta.1`, and the fractal-fun panel has landed on top of it (unreleased/untagged so far).

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

## Phase 2 — UI MVP

- [x] Main window: sensors + controls (egui 0.35)
- [x] Manual duty sliders (write-gated)
- [x] Channel map + rename
- [x] Curve editor MVP + curve auto-apply (“Curve control”)
- [x] CPU graph (10/20/30/60 min + sample interval)
- [x] PawnIO install/admin dialog
- [x] Profile UX polish (switch / default / startup)
- [x] System tray

## Phase 3 — Polish, packaging, security

- [x] Public GitHub repo + CONTRIBUTING + hardware/security docs
- [x] Release workflow (`v*.*.*` → exe + SHA256; environment approval for owner)
- [x] CodeQL, cargo-audit, Dependabot
- [x] Branch protection + tag ruleset + release environment
- [ ] Code signing (SignPath / Azure / cert — **not** enabled)
- [ ] Installer / end-user packaging polish
- [ ] Full curve editor polish (multi-curve UX, bindings)
- [ ] Richer multi-sensor graphs
- [ ] Dynamic plugin loading infrastructure

## Phase 4 — Advanced

- [ ] Multi-sensor curves
- [ ] Additional official plugins
- [x] In-app auto-update: manual check (GitHub latest-release compare + link)
- [ ] In-app auto-update: download + SHA256 verify + install
- [x] Internationalization (GUI: en/fr/de/es/it/zh/ja/lb via `rust-i18n`, Options-panel picker, OS-locale default)
- [ ] AMD/Intel GPU temperature sensors — research documented in `docs/GPU_VENDOR_APIS.md`; blocked on hardware to validate against (see doc for why this isn't a quick FFI add)

## Next priorities (order)

1. Broader chip validation (community logs + experimental paths)  
2. Code signing when ready for wider audience  
3. Auto-update (manual button first)  
4. RGB remains **future / out of Super I/O fan HWM scope**

## Out of scope for v1

- Linux support  
- RGB as part of Super I/O fan control  
- Cloud / remote control  
- Bundling PawnIO inside the app binary  
- Claiming signed binaries before signing is wired  
