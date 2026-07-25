# GPU vendor API research (AMD / Intel)

Research notes only — **no code implements this yet**. GPU temperature today is
NVIDIA-only via `nvidia-smi` (see `crates/fancontrol-plugins/src/host.rs`). This
machine only has an NVIDIA GPU available, so neither path below has been validated
against real hardware. Per `CONTRIBUTING.md`'s ban on hallucinated APIs and fake
hardware-validation claims, nothing here should be implemented as a confident FFI
binding until either real hardware is available to test against, or the actual SDK
headers have been read end to end (not paraphrased from secondary docs).

## AMD — ADL (AMD Display Library)

- DLL: `atiadlxx.dll` (64-bit), `atiadlxy.dll` (32-bit-on-64-bit). Loaded dynamically
  (`libloading`, no link-time dependency), same philosophy as
  `crates/fancontrol-pawnio/src/ffi.rs`.
- Lifecycle: `ADL2_Main_Control_Create(callback, iEnumConnectedAdapters, &context)` →
  context handle threaded through every subsequent call →
  `ADL2_Main_Control_Destroy(context)`. Adapter enumeration via
  `ADL2_Adapter_NumberOfAdapters_Get` / `ADL2_Adapter_Active_Get`.
- **Materially different FFI shape than PawnIO's simple open/execute/close model**:
  `ADL2_Main_Control_Create` requires the caller to supply an
  `ADL_MAIN_MALLOC_CALLBACK` — a Rust `extern "C" fn` allocator the DLL calls back
  into for buffer allocation. This is new scaffolding, not a copy of the existing
  PawnIO pattern.
- **No single settled temperature call**: legacy `ADL2_Overdrive6_Temperature_Get`
  only covers older GCN-era cards. The commonly-cited
  `ADL2_New_QueryPMLogData_Get` is itself documented by AMD as **deprecated**, in
  favor of a shared-memory read API
  (`ADL2_Overdrive8_PMLog_ShareMemory_Read`, gated behind
  `ADL2_Overdrive8_PMLog_ShareMemory_Support`) — meaningfully more scaffolding than a
  plain call.
- Struct layouts (`ADLTemperature`, `ADLPMLogDataOutput`, etc.) must come from the
  actual ADL SDK headers (`adl.h`, `adl_structures.h`, `overdrive8.h`), not
  transcribed from memory — a mismatched field order/padding would silently produce
  a garbage temperature reading rather than a clean failure, which is worse than
  "unsupported."
- **Before implementing**: either get access to real AMD GPU hardware to validate
  against, or read the SDK headers closely enough to commit to one temperature-query
  code path (likely needs a capability check + two branches: legacy Overdrive6 vs.
  PMLog/shared-memory for newer cards).

## Intel — IGCL (Intel Graphics Control Library)

- Real, actively maintained open-source project:
  `intel/drivers.gpu.control-library`. DLL: `ControlLib.dll`, ships inside the
  Intel Graphics driver package (no separate SDK install needed — same "prerequisite
  already on the system" spirit as `nvidia-smi`).
- Lifecycle: `ctlInit()` → `ctlEnumerateDevices()` → `ctlEnumTemperatureSensors()` →
  `ctlTemperatureGetProperties()` / `ctlTemperatureGetState()`. Closer to PawnIO's
  simpler open→query shape than ADL's callback-based model — less new scaffolding
  expected.
- **Platform floor**: telemetry (including temperature) is documented as available
  from **Alder Lake-P and newer** platforms only, and the full telemetry API is
  **64-bit-process only** (Level Zero backend restriction). Older Intel iGPUs would
  get nothing — if implemented, this must degrade silently and be documented as
  partial support, not presented as full Intel GPU coverage.
- Exact struct layouts (`ctl_temp_properties_t`, the temperature-state struct) still
  need confirmation against the real `igcl_api.h` header before writing any
  `#[repr(C)]` binding.
- Exact DLL install path/casing on a real machine is unverified from documentation
  alone — needs confirming on an actual Intel-GPU Windows box.

## Ruled out: WMI

`Win32_VideoController` exposes display/driver metadata only, no standardized
thermal property. `MSAcpi_ThermalZoneTemperature` reports ACPI thermal zones
(motherboard/chipset-defined), not reliably wired to GPU die temperature. **Not a
viable fallback for GPU temperature on either vendor.**

## Recommended next step

Don't start writing FFI code from this document alone. Either:

1. Get real AMD and/or Intel GPU hardware (yours or a contributor's) to validate
   against, or
2. Read the actual SDK headers (`adl.h`/`adl_structures.h`/`overdrive8.h` for AMD,
   `igcl_api.h` for Intel) end to end to confirm exact struct layouts before
   committing to a signature.

Intel is the more tractable of the two (single documented lifecycle, no allocator
callback) — if only one gets picked up first, start there.
