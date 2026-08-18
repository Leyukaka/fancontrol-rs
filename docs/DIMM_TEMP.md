# DIMM (RAM) temperature

**Status: shipped.** Owner-validated on AMD (SmbusPIIX4) + NCT668x (`0xD5`) +
4× DDR5 SPD hubs (`host.dimm0..3.temp`, idle ~35–38 °C). Other chipsets/DIMMs
are unproven until someone sends logs. Sensor-only, read-only — never writes
to SPD.

## What it does

DDR5 modules ship an on-die "SPD hub" (JEDEC SPD5118-class) that exposes an
optional thermal sensor over SMBus, independent of the motherboard's Super
I/O / EC. fancontrol-rs opens a PawnIO SMBus module, probes the standard SPD
address range (`0x50`-`0x57`), and reads the sensor directly if the DIMM
reports itself as DDR5 and has thermal sensing enabled.

Exposed as sensors named `host.dimm{N}.temp` (°C), alongside a
`mock.dimm0.temp` sensor in the mock provider for UI/dev work without
hardware.

## Backend order: PIIX4 then I801

Two PawnIO modules can talk to the chipset's SMBus controller:

| Module | Chipset | Notes |
|--------|---------|-------|
| `SmbusPIIX4` | **AMD** (SB/FCH-style SMBus controller) | Tried first — owner's target platform is AMD. |
| `SmbusI801` | **Intel** (ICH/PCH SMBus controller) | Fallback when PIIX4 fails to open (wrong chipset / no AMD SMBus present). |

Whichever opens first is used for the whole session; there is no per-DIMM
mixing of backends.

## Detection

For each candidate address `0x50..=0x57`:

1. Read `DEVICE_TYPE_MOST` (offset `0x00`) — expect `0x51`.
2. Read `DEVICE_TYPE_LEAST` (offset `0x01`) — expect `0x18`.
3. If both match, the address is treated as a DDR5 SPD hub.

Detection also prefers hubs whose `DEVICE_CAPABILITY` (offset `0x05`) has bit 1
set (on-die thermal sensor present).

At sample time, `THERMAL_SENSOR_ENABLED` / MR26 (offset `0x1A`) is checked with
**JEDEC polarity: `0` = enabled, non-zero = disabled**. A disabled sensor is
skipped for that read (sensor shows as n/a).

Temperature comes from a word read at `TEMPERATURE_ADDRESS` (offset `0x31`;
MR49/MR50), converted as a 13-bit two's-complement field at 0.0625 °C/LSB
(bit `0x1000` is the sign bit; bits 15:13 are alarm flags and are masked off
before conversion). If `WORD_DATA` fails, two `BYTE_DATA` reads are assembled
little-endian (same packing as PawnIO PIIX4 `SMBHSTDAT0`/`DAT1`).

Register offsets are a **map only**, cross-referenced against the public
JEDEC SPD5118 layout and the offsets used by
[RAMSPDToolkit](https://github.com/GordonMcGregor/RAMSPDToolkit) (MPL-2.0).
No source code from that project is included here.

### DDR4

DDR4 modules commonly carry a separate TSOD (thermal sensor on DIMM) at
`0x18`-`0x1F`, distinct from the SPD EEPROM. That path is **not implemented**
— the register conventions in circulation are less consistently documented
across vendors than the DDR5 SPD5118 spec, and getting it wrong risks
misreporting a temperature with confidence it doesn't deserve. Left as
future work; happy to wire it up given a citable JEDEC/LHM register
reference plus hardware to validate against.

## Elevation & the SMBus mutex

Like every other PawnIO path in fancontrol-rs, opening the SMBus module
requires an elevated (Administrator) process. Before probing, the backend
best-effort-acquires the well-known global mutex
`Global\Access_SMBUS.HTP.Method` — the same one HWiNFO / LibreHardwareMonitor
use to avoid colliding with each other on the SMBus. Failure to acquire it
(not present, held elsewhere, timeout) is **non-fatal**: sampling proceeds
without it, same as the existing ISA-bus mutex handling for Super I/O.

## Caching

SMBus transactions are comparatively slow (per-byte, bus-arbitrated) and the
value barely moves, so reads are cached for ~3 seconds per provider instance
— consistent with the CPU-power sampling cache in the same crate.

## Known residual risks

- **Only validated read-only on one AMD + NCT668x + DDR5 machine.** Other
  chipsets, DIMM vendors, and BIOS SMBus routing are untested.
- **No paging support.** Some SPD5 hubs may require a page-register write to
  reach certain offsets; this implementation assumes the probed offsets are
  reachable without paging, which may not hold on every hub revision.
- **Silent skip on failure.** A DIMM that fails detection or a disabled
  thermal sensor simply doesn't appear as a sensor — there is no user-facing
  distinction between "not DDR5", "sensor disabled", and "bus error".
- **Possible SMBus contention** with other monitoring tools (HWiNFO, etc.)
  despite the best-effort mutex, if that tool doesn't honor the same name or
  holds it for the full sample duration.
