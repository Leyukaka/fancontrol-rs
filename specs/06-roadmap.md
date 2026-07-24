# Roadmap

## Phase 0 — Foundation (current)
- [x] Repository + Spec-Driven Design documents
- [x] Workspace structure (`crates/`)
- [x] Final UI technology decision (egui + eframe)
- [x] Basic `fancontrol-core` domain models + curve engine + profile JSON
- [x] Plugin traits + mock provider
- [x] CLI harness (`list-sensors`, `set-duty`, `demo`, …)
- [x] CI (GitHub Actions)

## Phase 1 — Hardware & Core
- [x] `fancontrol-pawnio` FFI to PawnIOLib + embedded LpcIO/Echo modules
- [x] Super I/O detect + Nuvoton banked HWM provider (temps/fans/PWM)
- [x] Core curve evaluation engine + control loop helper
- [x] Profile serialization (JSON)
- [x] CLI harness (`detect`, `backend-status`, `list-sensors`, `run`, …)
- [ ] Verify readings/writes on elevated process (pawnio_open needs admin)
- [ ] Broader chip coverage (ITE IT87, NCT668x EC path)

## Phase 2 — UI MVP
- [ ] Main window with sensor list + control list
- [ ] Manual duty control
- [ ] Basic curve editor
- [ ] Profile save/load
- [ ] System tray

## Phase 3 — Polish & Plugins
- [ ] Full curve editor experience
- [ ] Live graphs
- [ ] Plugin loading infrastructure
- [ ] Error handling & missing PawnIO UX
- [ ] Installer / packaging

## Phase 4 — Advanced
- [ ] Multi-sensor curves
- [ ] Additional official plugins
- [ ] Auto-update
- [ ] Internationalization

## Out of scope for v1
- Linux support
- RGB
- Cloud / remote features
