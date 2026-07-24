# Roadmap

## Phase 0 — Foundation
- [x] Repository + Spec-Driven Design documents
- [x] Workspace structure (`crates/`)
- [x] Final UI technology decision (egui + eframe)
- [x] Basic `fancontrol-core` domain models + curve engine + profile JSON
- [x] Plugin traits + mock provider
- [x] CLI harness (`list-sensors`, `set-duty`, `demo`, …)
- [x] CI (GitHub Actions)

## Phase 1 — Hardware & Core
- [x] `fancontrol-pawnio` FFI to PawnIOLib + embedded LpcIO/Echo modules
- [x] Super I/O detect + Nuvoton NCT668x EC HWM provider (temps/fans/PWM)
- [x] Core curve evaluation engine + control loop helper
- [x] Profile serialization (JSON)
- [x] CLI harness (`detect`, `backend-status`, `list-sensors`, `run`, `sample`, `test-duty`, …)
- [x] Verify readings/writes on elevated process (owner NCT6687D-class; admin required)
- [ ] Broader chip coverage (ITE IT87, banked NCT — experimental detect only today)
- [x] Host sensors: GPU (`nvidia-smi`), storage (`DeviceIoControl`, no PowerShell) — read-only

## Phase 2 — UI MVP
- [x] Main window with sensor list + control list (egui)
- [x] Manual duty control (sliders, write-gated)
- [x] Channel display map (`channel-map.json`)
- [x] Basic curve editor (in-app; polish ongoing)
- [x] Live temperature graph (CPU sparkline)
- [ ] Profile save/load polish in UI
- [ ] System tray
- [x] Missing-PawnIO messaging path (UI probe / popup direction)

## Phase 3 — Polish & Plugins
- [ ] Full curve editor experience (multi-curve UX, bindings polish)
- [ ] Richer multi-sensor graphs
- [ ] Plugin loading infrastructure
- [ ] Error handling & missing PawnIO UX polish
- [x] Release packaging skeleton (`.github/workflows/release.yml` + docs)
- [ ] Installer / end-user packaging polish
- [ ] Code signing (see `docs/SIGNING_AND_DISTRIBUTION.md`) — **not** enabled yet

## Phase 4 — Advanced
- [ ] Multi-sensor curves
- [ ] Additional official plugins
- [ ] Auto-update
- [ ] Internationalization

## Next priorities (order)

1. System tray  
2. Broader chip validation (community logs + experimental paths)  
3. Profile UX polish in UI  
4. Signing / SmartScreen reduction when ready for wider audience  
5. RGB remains **future / out of Super I/O fan scope**

## Out of scope for v1
- Linux support
- RGB (not Super I/O fan HWM)
- Cloud / remote features
- Bundling PawnIO inside the app binary
