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
| Banked NCT (Nuvoton NCT679x-class) | **Validated** on ASUS ROG STRIX B550-A (`0xD4`) reads+PWM; other boards uncertified - see `docs/SUPPORTED_HARDWARE.md` |
| Broader chip productization (ITE IT87, more NCT ids) | **Experimental / detect-only** until board-certified |

### Validated paths (owner / reference)

| Item | NCT668x EC | Banked NCT (B550-A) |
|------|------------|---------------------|
| Family | Nuvoton **NCT668x EC** (NCT6687D-class) | Nuvoton **banked NCT** (NCT6798-class) |
| Chip id / rev | `0xD5` / `0x92` | `0xD4` / `0x2B` |
| HWM base | `0x0A20` | `0x0290` |
| Super I/O | slot @ `0x4E` (board-dependent) | slot @ `0x2E` |
| PWM | **ctrl0–3** reliable; higher may use DR / BIOS reclaim | Owner-confirmed PWM writes |

Full user-facing matrix: **[docs/SUPPORTED_HARDWARE.md](../docs/SUPPORTED_HARDWARE.md)**.

### Curve temperature source

Fan curves use **CPU-like** Super I/O temps only (`CPU`, `PECI` / `PECI_0`, `CPUTIN`, package-style names). Runtime resolves stale `…temp.CPU` bindings on banked chips to a live CPU-like reading. **Not** used for regulation: SYSTIN / VRM / AUX / GPU (display-only in the UI graph).

### Fallback behavior

If PawnIO is missing or not openable:

- UI shows a **startup dialog** (install vs admin) with link to pawnio.eu
- Needs-admin dialog / top bar: **Restart as Administrator** button → UAC via `ShellExecute` `runas` (no silent elevation)
- App still starts: mock / config / host sensors as enabled
- No silent hardware writes

### Secondary sources (host / plugins)

| Source | Method | Notes |
|--------|--------|-------|
| NVIDIA GPU multi-metric | `nvidia-smi` at **fixed install paths** (no PATH walk): temp, power, util, clocks, fan %, VRAM | Read-only |
| NVIDIA Hot Spot | Not via smi/NVML public; would need NvAPI (not shipped) | — |
| Storage temp | `DeviceIoControl` + `StorageDeviceTemperatureProperty` on `\\.\PhysicalDriveN` | No PowerShell; often needs elevation |
| Mock provider | In-process | Dev / CI / `--no-hw` |

### Non-negotiable rules

1. Never embed or ship WinRing0 or known-vulnerable ring-0 drivers.
2. Never require disabling Secure Boot or loading arbitrary unsigned drivers for core fan HWM.
3. PWM writes are **on by default** (CLI/UI); pass `--read-only` to disable for diagnostics. `--allow-hw-write` remains accepted for older scripts (no-op when already default-on).
4. Do not auto-install PawnIO or change Defender settings without explicit user approval.
