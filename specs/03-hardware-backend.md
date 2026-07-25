# Hardware Backend Specification

## Primary backend: PawnIO

We use **[PawnIO](https://pawnio.eu/)** as the sole privileged hardware access layer for Super I/O / EC HWM.

### Why PawnIO?

- Modern replacement for the vulnerable WinRing0 driver
- Scriptable (Pawn language) modules; we vendor signed LpcIO/Echo blobs under `crates/fancontrol-pawnio/modules/`
- Avoids shipping and signing our own kernel driver
- **Prerequisite**: user installs PawnIO; elevation (Administrator) is typically required for `pawnio_open`

### Responsibilities of `fancontrol-pawnio`

| Responsibility | Status |
|----------------|--------|
| Detect install / open executor | **Done** (`is_installed`, `is_available`, status messaging) |
| Load LpcIO module | **Done** |
| Super I/O detect + BAR / NCT668x EC path | **Done** (owner-validated NCT6687D-class) |
| Batch HWM sample (`sample_all`) | **Done** |
| PWM write (duty %) | **Done** for validated channels; write-gated at product layer |
| Broader chip productization (IT87, banked NCT) | **Experimental / detect-only** — see `docs/SUPPORTED_HARDWARE.md` |

### Validated path (owner / reference)

| Item | Value |
|------|--------|
| Family | Nuvoton **NCT668x EC** (NCT6687D-class) |
| Chip id / rev | `0xD5` / `0x92` |
| HWM base | `0x0A20` |
| Super I/O | slot @ `0x4E` (board-dependent) |
| PWM | **ctrl0–3** reliable; higher ctrls may use DR / BIOS reclaim |

Full user-facing matrix: **[docs/SUPPORTED_HARDWARE.md](../docs/SUPPORTED_HARDWARE.md)**.

### Fallback behavior

If PawnIO is missing or not openable:

- UI shows a **startup dialog** (install vs admin) with link to pawnio.eu
- App still starts: mock / config / host sensors as enabled
- No silent hardware writes

### Secondary sources (host / plugins)

| Source | Method | Notes |
|--------|--------|-------|
| NVIDIA GPU temp | `nvidia-smi` at **fixed install paths** (no PATH walk) | Read-only |
| Storage temp | `DeviceIoControl` + `StorageDeviceTemperatureProperty` on `\\.\PhysicalDriveN` | No PowerShell; often needs elevation |
| Mock provider | In-process | Dev / CI / `--no-hw` |

### Non-negotiable rules

1. Never embed or ship WinRing0 or known-vulnerable ring-0 drivers.
2. Never require disabling Secure Boot or loading arbitrary unsigned drivers for core fan HWM.
3. PWM writes are **on by default** (CLI/UI); pass `--read-only` to disable for diagnostics. `--allow-hw-write` remains accepted for older scripts (no-op when already default-on).
4. Do not auto-install PawnIO or change Defender settings without explicit user approval.
