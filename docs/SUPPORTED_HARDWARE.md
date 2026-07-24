# Supported hardware

fancontrol-rs controls PC fans on **Windows** through **[PawnIO](https://pawnio.eu/)** (LpcIO module → Super I/O / EC HWM registers). It is **not** a multi-OS LibreHardwareMonitor clone and does **not** ship a kernel driver of its own.

> **Status language**
>
> | Label | Meaning |
> |-------|---------|
> | **Validated** | Read and/or write exercised on a real machine owned by maintainers or trusted reporters; results documented. |
> | **Experimental** | Code path exists; limited or no production validation. May mis-identify chips or skip banks. |
> | **Read-only** | Sensors only (no PWM write), or write not offered for that path. |
> | **Not supported** | Out of scope for now — do not expect control. |

---

## Prerequisites

| Requirement | Notes |
|-------------|--------|
| **Windows** | Primary and only target OS for hardware access. |
| **[PawnIO](https://pawnio.eu/)** | **Prerequisite — not bundled** with fancontrol-rs. Install the official PawnIO package first. If missing, the app still starts (mock / config / host sensors); the UI surfaces a clear message / popup when hardware backend is unavailable. |
| **Administrator** | `pawnio_open` needs elevation. Run an elevated terminal or launch the UI as admin for live Super I/O. |
| **Defender awareness** | Unsigned binaries that talk to PawnIO may false-positive. Prefer a folder exclusion for `target\` (see README) — do not disable AV entirely. |

Vendored PawnIO modules (e.g. `LpcIO.bin`) ship under `crates/fancontrol-pawnio/modules/` for the user-mode loader; the **PawnIO service/driver** itself must already be installed on the machine.

---

## Support matrix

| Source | Role | Status | Notes |
|--------|------|--------|-------|
| **Nuvoton NCT668x EC HWM** (class id `0xD5`, e.g. NCT6687D-class) | Temps, fans, PWM | **Validated** (reads + writes) | Owner board: id `0xD5` rev `0x92`, HWM base `0x0A20`, Super I/O slot @ `0x4E`. Via PawnIO **LpcIO**. |
| **ctrl0–ctrl3** (NCT668x) | Fan duty | **Validated** | Reliable control path on validated board. |
| **Higher control indices** (NCT668x) | Fan duty | **Experimental / DR path** | Direct-register style path; less trusted than ctrl0–3. Validate carefully before relying on it. |
| **Banked Nuvoton NCT / ITE IT87** (classic Super I/O HWM) | Detect / sensors | **Experimental** | Detection helpers exist; not the primary validated path. |
| **Host: GPU** (`nvidia-smi`) | Temperature | **Read-only** | Process spawn; no fan curve write through nvidia-smi. |
| **Host: storage** (`DeviceIoControl` temperature property) | Temperature | **Read-only** | Win10+ storage stack via `\\.\PhysicalDriveN` (no PowerShell). Often needs elevation. |
| **Mock provider** | Dev / UI without hardware | Always available | `--no-hw` |

---

## NCT668x (validated details)

Validated configuration (2026-07-24):

| Field | Value |
|-------|--------|
| Chip class | Nuvoton **NCT6687D-class** EC HWM |
| Chip id / rev | `0xD5` / `0x92` |
| Super I/O | slot1 @ `0x4E` |
| HWM base | `0x0A20` |
| Backend | PawnIO + **LpcIO** |

**Reads (sample):** CPU ~56 °C, System ~36 °C; fans 0 / 1 / 12 / 13 / 14 live; ctrl0 ~38 %, ctrl1 ~54 %.

**Writes:** `test-duty pawnio.0.ctrl1` 54 % → 40 % → 54 % with RPM ~2150 → ~1826 → ~1954. EC write path OK for validated channels.

Hardware remains **read-only by default**. PWM writes require explicit:

```bash
cargo run -- --allow-hw-write …
# or
cargo run -- --hw-only --allow-hw-write ui
```

Prefer `sample`, `list-sensors`, `list-controls`, and `test-duty` for validation before leaving curves auto-apply on.

---

## What is **not** controlled

| Area | Status |
|------|--------|
| **RGB / ARGB** | Not Super I/O fan HWM — out of scope for v1 |
| **Linux / macOS** | Out of scope |
| **WinRing0 / unsigned random drivers** | **Forbidden** — never embedded |
| **Cloud / remote control** | Out of scope |
| **All motherboards / all EC firmwares** | Only paths listed above; most boards untested |
| **AIO pumps, GPU native fan curves (write)** | Not via current Super I/O path |
| **Shipping PawnIO inside the installer** | Not done; user installs PawnIO separately |

---

## Contributing a chip / board report

We need **logs from a real machine**, not guesses.

1. Install PawnIO; run an **elevated** shell.
2. Capture:

```bash
cargo run -- backend-status
cargo run -- detect
cargo run -- list-sensors
cargo run -- list-controls
cargo run -- sample
```

3. If you attempt writes (only if you accept risk):

```bash
cargo run -- --allow-hw-write test-duty <control-id> --percent 40
# restore previous duty afterward
```

4. Open an issue with:
   - Motherboard model / Super I/O if known (HWiNFO, board manual)
   - Chip id, rev, LDN, HWM base from `detect`
   - Sensor/control IDs and whether values look sane vs HWiNFO / FanControl
   - Write results **only if** you actually ran them (RPM before/after)

See [CONTRIBUTING.md](../CONTRIBUTING.md). Do not claim validation you did not perform.

---

## Related docs

- [specs/03-hardware-backend.md](../specs/03-hardware-backend.md) — PawnIO architecture rules  
- [README.md](../README.md) — build, CLI, Defender  
- [docs/SIGNING_AND_DISTRIBUTION.md](./SIGNING_AND_DISTRIBUTION.md) — release packaging  
