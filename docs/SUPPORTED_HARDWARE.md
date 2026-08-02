# Supported hardware

fancontrol-rs controls PC fans on **Windows** through **[PawnIO](https://pawnio.eu/)** (LpcIO module → Super I/O / EC HWM registers). It is **not** a multi-OS LibreHardwareMonitor clone and does **not** ship a kernel driver of its own.

> **Status language**
>
> | Label | Meaning |
> |-------|---------|
> | **Validated** | Read and/or write exercised on a real machine owned by maintainers or trusted reporters; results documented. |
> | **Experimental** | Code path exists; limited or no production validation. May mis-identify chips or skip banks. |
> | **Read-only** | Sensors only (no PWM write), or write not offered for that path. |
> | **Not supported** | Out of scope for now - do not expect control. |

---

## Prerequisites

| Requirement | Notes |
|-------------|--------|
| **Windows** | Primary and only target OS for hardware access. |
| **[PawnIO](https://pawnio.eu/)** | **Prerequisite - not bundled** with fancontrol-rs. Install the official PawnIO package first. If missing, the app still starts (mock / config / host sensors); the UI surfaces a clear message / popup when hardware backend is unavailable. |
| **Administrator** | `pawnio_open` needs elevation. In the UI use **Restart as Administrator** (UAC), or right-click the exe / use an elevated terminal for CLI. |
| **Defender awareness** | Unsigned binaries that talk to PawnIO may false-positive. Prefer a folder exclusion for `target\` (see README) - do not disable AV entirely. |

Vendored PawnIO modules (e.g. `LpcIO.bin`) ship under `crates/fancontrol-pawnio/modules/` for the user-mode loader; the **PawnIO service/driver** itself must already be installed on the machine.

---

## Policy

**All desktop motherboards are in scope.** The stack targets common Super I/O / EC chips (Nuvoton NCT668x, banked NCT679x, ITE IT87, …) through PawnIO. If your board exposes fans via those paths, fancontrol-rs is intended to work.

**Certification is separate from “should work”.** We only mark a **board model** (or chip class) as **Validated / Certified** after real logs from that machine (reads; preferably a careful PWM write). Please send reports - even “sensors look good, I did not write duty” helps.

- GitHub issue template: [Hardware report](https://github.com/Leyukaka/fancontrol-rs/issues/new?template=hardware_report.yml)
- Front-page summary: [README - Motherboard support](../README.md#motherboard-support)

## Validated boards (certified)

| Board / setup | Chip (class) | Id / rev | HWM | What was proven | Date |
|---------------|--------------|----------|-----|-----------------|------|
| Maintainer board (NCT668x) | Nuvoton **NCT6687D-class** EC | `0xD5` / `0x92` | `0x0A20` @ `0x4E` | Temps, fans, **PWM write** (ctrl0-3) | 2026-07-24 |
| **ASUS ROG STRIX B550-A GAMING** | Banked NCT (**NCT6798D-class**) | `0xD4` / `0x2B` | `0x0290` @ `0x2E` | Temps, fans, **PWM write**; curve temp fallback | 2026-07-29 |

## Need reports (help certify)

These are **high value** for expanding the certified list. Code may already route them; we still need pasteable logs.

| Priority | Target | Why |
|----------|--------|-----|
| P1 | Other **ASUS** AM4/AM5/Intel (B550/X570/B650/Z790/…) | Same banked NCT family as B550-A |
| P1 | **MSI** boards with **NCT668x** (X670/B650/Z790/…) | EC path certified on one board only; MSI layouts can differ |
| P1 | Other **NCT679x** ids (`0xD1`-`0xD8`…) | Banked path exists; per-board map still unknown |
| P2 | **Gigabyte** / **ASRock** (**ITE IT87**: 8688/8689/8665/8792/…) | Detection present; write often BIOS-sensitive |
| P2 | Dual Super I/O boards (extra ITE chip) | Fans may live on the second chip |
| P3 | Older Fintek / Winbond Super I/O | Lower priority |

## Support matrix

| Source | Role | Status | Notes |
|--------|------|--------|-------|
| **Nuvoton NCT668x EC HWM** (class id `0xD5`, e.g. NCT6687D-class) | Temps, fans, PWM | **Validated** (reads + writes) | Owner board: id `0xD5` rev `0x92`, HWM base `0x0A20`, Super I/O slot @ `0x4E`. Via PawnIO **LpcIO**. |
| **ctrl0-ctrl3** (NCT668x) | Fan duty | **Validated** | Reliable control path on validated board. |
| **Higher control indices** (NCT668x) | Fan duty | **Experimental / DR path** | Direct-register style path; less trusted than ctrl0-3. Validate carefully before relying on it. |
| **Banked Nuvoton NCT** (classic Super I/O HWM) | Temps, fans, PWM | **Validated** (reads + writes on ROG B550-A) | ASUS **ROG STRIX B550-A GAMING**: id `0xD4` rev `0x2B`, HWM `0x0290`, slot `@0x2E`. Temps named `CPUTIN` / `PECI_0` (not `temp.CPU`). Curve binding falls back if profile still points at `pawnio.0.temp.CPU`. Some fan channels may be noise (e.g. fan6 absurd RPM). |
| **ITE IT87** / other banked layouts | Detect / sensors | **Experimental** (needs reports) | Detection helpers exist; not certified on maintainer hardware. |
| **Any other Super I/O board** | - | **Should work - not certified until reported** | Open a hardware report with logs. |
| **Host: GPU** (`nvidia-smi` multi-metric) | Temp core/memory, power W, util %, clocks, fan %, VRAM | **Read-only** | Fixed paths; UI GPU panel. **Hot Spot not available** via smi (would need NvAPI). No GPU fan curve writes. |
| **Host: storage** (`DeviceIoControl`, no PowerShell) | Temperature | **Read-only** | Prefer **NVMe health log** (composite °C), then `StorageDeviceTemperatureProperty` / adapter (all sensors scanned). Elevate for `\\.\PhysicalDriveN`. CLI: `sample-storage`. |
| **Host: Activity deck** (Options, default on) | CPU load %, top processes (CPU + RAM) | **Read-only** | Windows APIs only (`GetSystemTimes`, Toolhelp, `GetProcessTimes`, working set). **No PowerShell, no WMI.** ~1 s while enabled; Load-only skips process enum. |
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

PWM writes are **on by default** when launching the app. Use `--read-only` to stay read-only for diagnostics:

```bash
cargo run -- --read-only …
# or
cargo run -- --hw-only --read-only ui
```

Prefer `sample`, `list-sensors`, `list-controls`, and `test-duty` for validation before leaving curves auto-apply on.

---

## Banked NCT (ROG STRIX B550-A GAMING)

Validated configuration (2026-07-29):

| Field | Value |
|-------|--------|
| Board | ASUS **ROG STRIX B550-A GAMING** |
| Chip class | Nuvoton **NCT (banked)** |
| Chip id / rev | `0xD4` / `0x2B` |
| Super I/O | slot0 @ `0x2E` |
| HWM base | `0x0290` |
| Backend | PawnIO + LpcIO banked path |

**Reads:** fan0 / fan1 / fan3 live RPM; PECI_0 / CPUTIN / SYSTIN / AUXTIN0 present (may report similar values depending on wiring). Host GPU (nvidia-smi) + storage temps OK.

**Writes:** PWM control confirmed working by owner (not only read-only probe).

**Curves:** regulation uses **CPU-like temps only** (`CPU`, `PECI` / `PECI_0`, `CPUTIN`, package-style names). Not used for fan curves: SYSTIN / VRM / AUX / GPU (those stay display-only on the graph). Default profiles may still bind `pawnio.0.temp.CPU` (NCT668x name); runtime maps missing or non-CPU bindings to a live CPU-like reading via `fancontrol_core::resolve_curve_temp_sensor` (v0.3.3+).

---

## What is **not** controlled

| Area | Status |
|------|--------|
| **RGB / ARGB** | Not Super I/O fan HWM - out of scope for v1 |
| **Linux / macOS** | Out of scope |
| **WinRing0 / unsigned random drivers** | **Forbidden** - never embedded |
| **Cloud / remote control** | Out of scope |
| **All motherboards / all EC firmwares** | Only paths listed above; most boards untested |
| **AIO pumps, GPU native fan curves (write)** | Not via current Super I/O path |
| **Shipping PawnIO inside the installer** | Not done; user installs PawnIO separately |

---

## Contributing a chip / board report

We need **logs from a real machine**, not guesses. Prefer the GitHub form:  
**[Hardware report](https://github.com/Leyukaka/fancontrol-rs/issues/new?template=hardware_report.yml)**  
(or a Reddit/Discord paste if you prefer - same content).

### Prerequisites

1. Windows 10/11 desktop (not a VM for Super I/O).
2. Install **[PawnIO](https://pawnio.eu/)** (required; not bundled).
3. Download a [release](https://github.com/Leyukaka/fancontrol-rs/releases) **or** build from source.
4. Run the shell / app **as Administrator**.

### Commands (release binary)

If `fancontrol-rs.exe` is on your PATH or in the current directory:

```bat
fancontrol-rs backend-status
fancontrol-rs detect
fancontrol-rs list-sensors
fancontrol-rs list-controls
fancontrol-rs sample
```

From a source checkout:

```bash
cargo run -- backend-status
cargo run -- detect
cargo run -- list-sensors
cargo run -- list-controls
cargo run -- sample
```

### Optional PWM write (only if you accept risk)

Restores are your responsibility. Prefer a single control and return to the previous duty.

```bat
fancontrol-rs --hw-only test-duty --control <control-id> --percent 40
```

Read-only session (no writes):

```bat
fancontrol-rs --read-only ui
```

### What to include in the report

- Motherboard **exact model** (e.g. `ASUS ROG STRIX B550-A GAMING`)
- Super I/O name if known (HWiNFO / board manual)
- Full paste of `detect`, `list-sensors`, `list-controls` (and `sample` if short)
- Whether RPM / temps look sane vs HWiNFO or FanControl
- Write results **only if** you ran them (duty before/after + RPM)

See [CONTRIBUTING.md](../CONTRIBUTING.md). Do not claim validation you did not perform.

---

## Related docs

- [specs/03-hardware-backend.md](../specs/03-hardware-backend.md) - PawnIO architecture rules  
- [README.md](../README.md) - build, CLI, Defender  
- [docs/SIGNING_AND_DISTRIBUTION.md](./SIGNING_AND_DISTRIBUTION.md) - release packaging  
