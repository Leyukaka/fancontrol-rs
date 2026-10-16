# 08 - Lighting (RGB / ARGB)

Status: **in progress**, experimental CLI only (`rgb-probe`, `rgb-set`).

## Scope

- Fan and AIO lighting: motherboard ARGB headers and USB HID controllers.
- Not now: keyboards, mice, headsets, DRAM RGB (SMBus), GPU RGB (I2C).
- Native HID backends. An OpenRGB SDK client may come later for wider coverage.

Lighting controllers for fans and AIOs are USB HID devices. No kernel driver, no PawnIO,
usually no elevation. Access goes through `windows-sys` (CfgMgr32 + `hid.dll`).
AIOs without USB (3-pin 5 V ARGB lead) are driven through the motherboard header they plug into.

## Crate

`fancontrol-lighting`: HID enumeration and feature reports (`hid`), known controller table
(`known`), per-vendor frame builders (`msi_mystic`).

## MSI Mystic Light, USB `0DB0:0076`

Shared by MSI X870 and B850 boards. Protocol from
[nmelo/msi-mystic-light-x870e](https://github.com/nmelo/msi-mystic-light-x870e) (MIT),
tested there on one X870E board.

- HID feature report `0x51`, 761 bytes.
- `[0]=0x51`, `[1]=0x09`, `[2]=0x04`, `[3]=` header (0 = JARGB1), `[6]=` LED count, `[7..]` RGB triplets.
- Direct per-LED colors only.

Rules:

- Only this direct color report is sent. No mode, effect or save-to-flash command: older
  Mystic Light code in OpenRGB bricked RGB on some boards by sending modes.
- `rgb-set` refuses to write if the device's feature report length is not 761.
- MSI Center running at the same time will overwrite our colors.

Open question: OpenRGB issue [#5429](https://gitlab.com/CalcProgrammer1/OpenRGB/-/work_items/5429)
reports a B850 board with the same ids that the X870 code path did not drive.

## Phases

| Phase | Content |
|-------|---------|
| L1 | `rgb-probe`: list HID devices, dump feature reports `0x50`-`0x52` of known controllers (read-only) |
| L2 | `rgb-set`: solid color per header. Go / no-go: if nothing lights up, capture MSI Center USB traffic before going further |
| L3 | Domain model (device, zone, LED count), `LightingProvider` trait, effect thread (static, breathing, rainbow, color from a sensor), resume after sleep |
| L4 | UI panel, i18n |
| L5 | `docs/SUPPORTED_HARDWARE.md`, roadmap |

## Licensing

The project is MIT OR Apache-2.0. OpenRGB and liquidctl are GPL: do not port their code.
Protocol facts from MIT sources (credited), own USB captures or public docs are fine.
