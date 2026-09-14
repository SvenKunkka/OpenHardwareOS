# 0005 — NVML precedence and device identity

## Status

Accepted

## Date

2026-09-11

## Context

Two providers can see the same physical NVIDIA GPU: `adapters/nvidia` through NVML
(`Nvml::init()` → `device_by_index(i)`), device id `gpu.nvidia.0`
(`gpu_device_id`, `adapters/nvidia/src/lib.rs`); and
`adapters/libre-hardware-monitor` through LHM's NVIDIA backend (NVAPI for the cooler
and hotspot surfaces), device id `gpu.lhm.0`
(`DeviceId::compose(device_type, NAMESPACE, index)`, `NAMESPACE = "lhm"`).

Nothing collapses those two ids: `crates/ohm-runtime/src/discovery.rs` deduplicates
*adapters* by `AdapterId` (`deduplicate_adapters`) and device ids are
adapter-namespaced, so with both providers enabled a machine shows **two** entries
for one card, each with partial readings, and the GPU fan has two writers.

The providers are not equivalent:

| Capability | NVML (`adapters/nvidia`) | LHM (NVAPI/NVML mix) |
|---|---|---|
| Core temperature, power, utilisation, clocks, VRAM | yes | yes |
| Hotspot / junction temperature | **no** — NVML's sensor enum has no hotspot member | yes, via NVAPI |
| Fan write | `nvmlDeviceSetFanSpeed_v2`, **requires elevation** | yes, through LHM control channels |

## Decision

**NVML is registered only when LibreHardwareMonitor is not enabled**, expressed in
`AdapterOptions::from_settings` (`crates/adapters/src/lib.rs`):

```rust
nvidia: settings.adapter_enabled(ohm_adapter_nvidia::ADAPTER_ID)
    && (nvidia_forced || !lhm_enabled),
```

with `nvidia_forced` read from `adapter_settings.nvidia.always`. The module docs in
that file give the reason: both adapters would report the same physical GPU twice
with half the readings each, and LHM reports more (hotspot via NVAPI), so LHM wins.

**The escape hatch is `adapter_settings.nvidia.always = true`**, for users who
deliberately want NVML's fan path back — a machine where LHM serves only the
motherboard SuperIO, or a card where `nvmlDeviceSetFanSpeed_v2` works and LHM's
NVAPI path does not. It is covered by the `settings_can_force_nvml_and_mock` test.

**What NVML can and cannot do is not hidden either.** `adapters/nvidia/src/lib.rs`
declares the hotspot capability with the description "Not exposed by NVML;
LibreHardwareMonitor reads it via NVAPI" and reports it unsupported rather than
omitting it. Fan writes are classified (`permission_denied` without elevation,
`vendor_limitation` when the SKU or driver refuses third-party control) and never
reported as success. There is no fan-curve API: a curve on NVML is our own
poll-and-set loop in `crates/ohm-automation`, which the engine already implements.

## Consequences

**Positive**

- One GPU entry, with the richer sensor set, by default: no duplicated device and no
  two writers fighting over one fan.
- LHM's presence *adds* capability rather than competing — hotspot, motherboard fans
  and AMD support all arrive with it.
- The escape hatch is explicit, persisted and tested, so precedence is a
  configuration fact rather than hidden heuristics.
- Machines without an NVIDIA driver (or with AMD/Intel only) degrade to "NVML
  unavailable": `Nvml::init()` failing is a normal state, not an error.

**Negative**

- **The rule is a heuristic, not identity.** It keys off "is LHM enabled", not "are
  these two entries the same card": disabling LHM's motherboard part also drops
  NVML, and forcing `nvidia.always = true` brings the duplicate entries back with
  nothing beyond the Devices page to explain it.
- NVML-only machines lose hotspot; LHM-only machines lose
  `nvmlDeviceGetFanSpeedRPM` precision and NVML's `num_fans()`/`min_max_fan_speed()`
  probe.
- Toggling either provider changes device ids, so rules written against
  `gpu.nvidia.0/...` stop resolving when LHM takes over (and vice versa) — the
  concrete user-visible cost of the missing stable identity below.

## Planned fix (not implemented)

Give every `Device` a **stable `physical_id` in `Device::metadata`** and make adapter
priority explicit, so any two adapters reporting one physical device collapse into a
single entry instead of relying on "is LHM enabled". Sketch:

- Each adapter fills `metadata["physical_id"]` from the strongest identity it can
  get — the NVIDIA UUID (already read into `metadata["uuid"]`), a PCI
  bus/device/function address, or a vendor serial.
- The device table merges records sharing a `physical_id`, preferring the
  higher-priority adapter **per capability** rather than per device, so NVML can
  supply fan RPM while LHM supplies hotspot on one entry.
- Adapter priority becomes a documented, ordered property of registration
  (`lhm` > `nvidia` > `system` > `opd` > `mock` — the order `enabled_ids` and
  `build_adapters` already use).

**This is a design note, not shipped behaviour.** It is recorded so the current
precedence rule is understood as interim, not as identity.

## Alternatives considered

- **Register both and let the user sort it out.** Rejected: duplicate GPU entries
  with half the readings each look like a bug, and two writers on one fan can
  oscillate — a safety problem, not just a cosmetic one.
- **Always prefer NVML and drop LHM's GPU support.** Rejected: NVML cannot report
  hotspot, has no curve API, needs elevation for writes, and is blind on AMD
  machines.
- **Merge the two adapters by device *name*.** Rejected: names collide across
  identical cards and change with driver marketing strings; a name is not identity.
- **Per-adapter capability filtering with no precedence (show both, hide
  overlaps).** Rejected: the duplicate device still exists in the device table, so
  rules and the UI still see two targets for one fan.
- **Force NVML off entirely and rely on LHM for all NVIDIA data.** Rejected: LHM may
  be absent or disabled, and NVML is the documented, unprivileged telemetry path and
  the only documented fan write needing no reverse-engineered interface (`0004`).
