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
- [ ] `fancontrol-pawnio` crate (PawnIO bindings + basic sensor/control discovery)
- [ ] Core curve evaluation engine
- [ ] Profile serialization (JSON)
- [ ] CLI or minimal test harness to verify readings

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
