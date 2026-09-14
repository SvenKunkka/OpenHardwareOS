# 0004 — Vendor SDKs and AMD

## Status

Proposed — awaiting maintainer sign-off

The AMD ADL/ADLX restriction is a legal reading of those SDK licences (sources in
`docs/research.md` §7). It blocks a vendor integration path, so it needs the owner's
decision even though the code already avoids every SDK it names.

## Date

2026-09-11

## Context

GPU telemetry and GPU fan control are the features users ask for first, and the
vendor situation on Windows is asymmetric:

| Vendor | Telemetry | Fan control | Licence |
|---|---|---|---|
| NVIDIA | NVML (temperature, power, utilisation, clocks, fan RPM/%) | `nvmlDeviceSetFanSpeed_v2`, `nvmlDeviceSetDefaultFanSpeed_v2`; Maxwell or newer, **requires elevation** | proprietary, but loadable from the installed driver |
| AMD | ADL2 Overdrive telemetry (`ADL2_OverdriveN_Temperature_Get`, `ADL2_New_QueryPMLogData_Get`) or ADLX `IPerformanceMonitoring` | `ADL2_Overdrive*_FanSpeed_Set`, `ADL2_CustomFan_Set`, ADLX tuning interfaces | proprietary AMD EULA — hostile to this project |

AMD's licences are the blocker, and the wording is explicit:

- **ADL SDK EULA §5(e)** forbids modifying or distributing SDK source "so that any
  part of it becomes subject to an **Excluded license**", defined there as any
  licence requiring source disclosure or granting the right to modify — i.e.
  Apache-2.0. **§3(d)** allows object-code distribution only; **§4(a)** requires a
  restrictive end-user EULA; **§5(f)** forbids publishing the SDK or its
  documentation "for others to use or copy"; **Schedule A** gives tools and
  documentation "no redistribution of any kind"
  ([verbatim text](https://chromium.googlesource.com/chromiumos/overlays/portage-stable/+/b1ad75f083a189fdc9b70fc2e1daf08ecd45eada/licenses/AMD-ADL)).
- **ADLX SDK licence §4(f)** forbids using the licensed materials "in way that
  requires that the Licensed Materials or any portion thereof be licensed under a
  Free Software License", and §1.3 includes licences requiring the result to be
  "**redistributable at no charge**". §2(c) is object-code-only, §3 mandates an
  end-user agreement naming **AMD as a third party beneficiary**, and the grant is
  revocable ([ADLX](https://github.com/GPUOpen-LibrariesAndSDKs/ADLX)).

Apache-2.0 grants precisely the rights those clauses forbid.

## Decision

**No ADL or ADLX material enters this repository — no headers, no generated
bindings, no hand-written `-sys` crate whose declarations are transcribed from ADL
headers, no samples, no wrapper libraries.** `ADLXWrapper` additionally ships **no
LICENSE file at any standard path** (all rights reserved), so it is unusable even
ignoring ADLX. Instead:

1. **AMD GPU telemetry and fan control come from the LibreHardwareMonitor host**
   (`adapters/libre-hardware-monitor`). LHM's AMD backend creates `SensorType.Fan`
   and `SensorType.Control` sensors and drives Overdrive through `atiadlxx.dll`;
   that interop is LHM's compliance problem under MPL-2.0, and we consume its
   output over IPC (`0003-libre-hardware-monitor-integration.md`).
2. **NVIDIA telemetry comes from NVML via `nvml-wrapper` 0.13.0** (MIT OR
   Apache-2.0) in `adapters/nvidia`. The crate loads `nvml.dll` through
   `libloading`, resolving to the copy installed with the display driver in
   `System32`, so **we redistribute no NVIDIA binary**. Redistribution is not
   merely unnecessary, it is prohibited: NVIDIA Driver License Agreement §2.7
   ("you may not sell, rent, sublicense, distribute or transfer the SOFTWARE") and
   §2.9 ("You may not use the SOFTWARE in any manner that would cause it to become
   subject to an open source software license"); NVML is not on the CUDA EULA's
   Attachment A redistributable list, and CUDA EULA §1.2 forbids copying any part
   of the SDK.
3. **No hand-rolled NVAPI FFI either.** NVIDIA's public GPU Cooler Interface page
   documents only `NvAPI_GPU_GetTachReading`; `NvAPI_GPU_SetCoolerLevels` is filed
   as undocumented ([nvapi-sys](https://docs.rs/nvapi-sys/latest/nvapi_sys/gpu/cooler/index.html)).
   Declarations derived from a reverse-engineered interface are a maintenance and
   legal risk; where a real curve/policy write is needed it goes through LHM.
4. **Enforcement.** The allow-list in `0002-license.md` plus `cargo-deny` (see
   [research.md §7.3](../research.md); not yet wired into CI) is the mechanical check,
   review is the real one: a PR adding `atiadlxx`, `ADL2_`, `ADLX*`, `nvapi64`,
   `NvAPI_` or a vendor `.dll`/`.lib`/`.h` is rejected on sight.

## Consequences

**Positive**

- The repository is auditable: `grep -ri 'adl\|adlx' crates adapters apps` returns
  our own prose, not vendor-derived code.
- AMD support exists (through LHM) without a licence negotiation we would lose, and
  NVIDIA support exists without shipping a driver component.
- The Linux AMD future is clean: `amdgpu` sysfs exposes `temp1_input`,
  `fan1_input`, `pwm1` (0–255), `pwm1_enable` (1 = manual, set first) and a real
  `fan_curve` interface on modern RDNA — no vendor SDK
  ([kernel docs](https://docs.kernel.org/gpu/amdgpu/thermal.html)).

**Negative**

- **Our AMD support is exactly as good as LHM's.** No LHM running, no AMD GPU data
  beyond the OS-level name and driver version — surfaced as an unavailable provider
  rather than a missing feature.
- Even inside LHM, AMD control is a capability-probing state machine: AMD's OD8
  sample says to check `ADL2_Overdrive_Caps` first and notes that when the driver's
  fan-curve feature is available "the legacy Fan controls will disabled". Per-SKU and
  elevation behaviour is **unverified** (AMD documents neither), so an AMD `Control`
  sensor never implies a working write.
- NVML exposes **no fan curve API** (duty percentage plus a policy enum) and no
  hotspot temperature, and refused writes are normal on both vendors
  (`NVML_ERROR_NO_PERMISSION` without elevation, `ControlRefused` when the board
  exposes no channel) — reported as such, never retried in a loop.

## Alternatives considered

- **Link ADL and review the licence later.** Rejected: §5(e) is unambiguous, and an
  `-sys` crate transcribed from ADL headers is itself a derivative work.
- **Load `atiadlxx.dll` at runtime and declare the functions by hand.** Rejected:
  the declarations would still be derived from ADL's headers, and the documentation
  we would develop against is covered by §5(f). Runtime loading solves
  redistribution, not derivation.
- **ADLX instead of ADL.** Rejected for the same reason plus a revocable grant,
  object-code-only terms, a mandatory EULA naming AMD a third party beneficiary, and
  a clause aimed squarely at free-of-charge redistribution.
- **OpenRGB (GPL-2.0) as an implementation.** Rejected: GPL-2.0 cannot be linked or
  copied into an Apache-2.0 product; its SMBus/PawnIO documentation is cited as
  reference reading only.
- **ROCm SMI / `amdsmi` on Windows.** Rejected: AMD's component table lists System
  Management for Linux only and the AMD SMI docs say it "supports Linux bare metal
  and Linux virtual machine guest environments". It is the right answer on Linux,
  where it is MIT.
- **Vendor WMI/ACPI gadgets for OEM laptops.** Rejected: the EC is firmware-owned, no
  library abstracts it, and laptop fan control is not claimed anywhere.
