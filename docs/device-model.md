# OpenHardwareOS — Device Model

The device model is the contract every other part of the system speaks. It lives
in `crates/ohm-device-model` (types) and `crates/ohm-runtime` (the views derived
from it). Nothing in it mentions a vendor, a model number or a driver — that is
the property the whole architecture leans on.

```text
Adapter  ──discover──▶  Device { capabilities }        crates/ohm-device-model/src/device.rs
Adapter  ──poll──────▶  DeviceState { readings }       crates/ohm-device-model/src/state.rs
Runtime  ──write─────▶  (device, capability, Value)    crates/ohm-runtime/src/runtime.rs
```

---

## 1. The types and where they live

| Type | File | Role |
|---|---|---|
| `Device` | `crates/ohm-device-model/src/device.rs` | a *description*: what the hardware can do |
| `DeviceType` | same | closed enum of 15 hardware classes (`Cpu`, `Gpu`, `Fan`, `Pump`, …) |
| `Transport` | same | how the runtime reaches it (`System`, `Nvidia`, `UsbHid`, `Web`, `Bridge`, `Mock`, …) |
| `Capability` | `crates/ohm-device-model/src/capability.rs` | one thing a device can report or be told |
| `CapabilityKind` | same | `Sensor`, `Actuator`, `Event`, `Info` |
| `Unit` | `crates/ohm-device-model/src/unit.rs` | physical unit of a capability (18 variants) |
| `Value` | `crates/ohm-device-model/src/value.rs` | `Integer`/`Number`/`Bool`/`Text`, serialised untagged |
| `DeviceState` | `crates/ohm-device-model/src/state.rs` | a snapshot of every capability at one instant |
| `Reading` | same | one capability + its `ReadingStatus` |
| `ReadingStatus` | same | `Ok { value }` or `Unavailable { reason, detail }` |
| `UnavailableReason` | same | why a capability has no value (10 reasons) |
| `DeviceView` | `crates/ohm-runtime/src/snapshot.rs` | `Device` + `enabled` + `DeviceStatus` + latest `DeviceState`; what the UI renders |
| `DeviceStatus` | same | `Online` / `Offline` / `Disabled` / `Degraded` |
| `DeviceRecord` | `crates/ohm-runtime/src/device_table.rs` | `DeviceView`'s runtime bookkeeping counterpart in the table |
| `CapabilityIndex` | `crates/ohm-runtime/src/registry.rs` | `{ sources, targets }` for the automation editor |
| `CapabilityRef` | same | a capability of a concrete device, flattened for the UI; `qualified_id()` is `device/capability` |

Note the split: the *model* crate is pure data and has no runtime dependency; the
*views* (`DeviceView`, `CapabilityIndex`, `CapabilityRef`, `DeviceStatus`) live in
`ohm-runtime` because they combine model data with runtime state (enabled flags,
adapter health, latest readings).

## 2. `Device`

```rust
pub struct Device {
    pub id: DeviceId,
    pub name: String,
    #[serde(rename = "type")]          // the wire field is "type", not "device_type"
    pub device_type: DeviceType,
    #[serde(default)] pub vendor: String,
    #[serde(skip_serializing_if = "Option::is_none", default)] pub model: Option<String>,
    pub transport: Transport,
    pub adapter: AdapterId,            // which adapter owns it
    pub capabilities: Vec<Capability>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty", default)] pub metadata: BTreeMap<String, String>,
    #[serde(skip_serializing_if = "Vec::is_empty", default)] pub tags: Vec<String>,
}
```

Builder: `Device::new(id, name, device_type, transport, adapter)` plus
`with_vendor`, `with_model`, `with_capability`, `with_capabilities`,
`with_metadata`, `with_tag`. Lookups: `capability(&CapabilityId)`,
`capability_str(&str)`, `supports(&str)`, `sensors()`, `actuators()`,
`writable_capabilities()`, `is_controllable()`.

`Device::validate()` is the guard against an adapter registering nonsense, and it
is called by `DiscoveryManager` (`validate_devices`) **and** by the adapters
themselves before returning:

* at least one capability,
* no duplicate capability ids,
* for writable capabilities: `min` and `max` are either both set or both unset.

Classification helpers on `DeviceType`: `is_thermal_source()` (Cpu, Gpu, Storage,
TemperatureSensor, Motherboard), `is_actuatable()` (Fan, Pump, Rgb, Display) and
`requires_safe_floor()` (Fan, Pump) — the last one is what tells the safety layer
that a device is a critical output.

## 3. JSON: a GPU

Exact shape produced for the device built in `crates/ohm-device-model/src/device.rs`
(test `json_uses_type_key_and_snake_case`, which asserts `type`, `transport`,
`adapter` and `capabilities[0].id`, and that `metadata` is absent when empty).
Field order follows the struct declaration; optional fields are omitted when unset.

```json
{
  "id": "gpu.nvidia.0",
  "name": "NVIDIA GeForce RTX 5090",
  "type": "gpu",
  "vendor": "NVIDIA",
  "transport": "nvidia",
  "adapter": "nvidia",
  "capabilities": [
    {
      "id": "temperature.core",
      "name": "Core Temperature",
      "kind": "sensor",
      "unit": "celsius",
      "readable": true,
      "writable": false
    },
    {
      "id": "power.total",
      "name": "Total Power",
      "kind": "sensor",
      "unit": "watt",
      "readable": true,
      "writable": false
    },
    {
      "id": "fan.speed_percent",
      "name": "GPU Fan Speed",
      "kind": "actuator",
      "unit": "percent",
      "readable": true,
      "writable": true,
      "min": 0.0,
      "max": 100.0
    }
  ]
}
```

Two wire details that bite people:

* the device type is serialised as **`"type"`** (`#[serde(rename = "type")]` on
  `Device::device_type`), while the protocol's `DeviceDescriptor` uses
  `"device_type"` — see `docs/protocol.md` §6;
* `"vendor"` is **always** present (it is a `String` with `#[serde(default)]`, not
  an `Option`), so a device with no vendor serialises as `"vendor": ""`.

## 4. JSON: a fan device

From the crate-level test in `crates/ohm-device-model/src/lib.rs`
(`documented_device_model_shape`), which asserts `capabilities[1].writable`,
`min` and `max`:

```json
{
  "id": "fan.system.0",
  "name": "Chassis Fan 1",
  "type": "fan",
  "vendor": "",
  "transport": "system",
  "adapter": "system",
  "capabilities": [
    {
      "id": "fan.rpm",
      "name": "Fan RPM",
      "kind": "sensor",
      "unit": "rpm",
      "readable": true,
      "writable": false
    },
    {
      "id": "fan.speed_percent",
      "name": "Fan Speed",
      "kind": "actuator",
      "unit": "percent",
      "readable": true,
      "writable": true,
      "min": 0.0,
      "max": 100.0
    }
  ]
}
```

## 5. JSON: state and readings

`Reading` flattens its `ReadingStatus` (`#[serde(tag = "status")]`) next to the
capability id, so a reading is one flat object. From
`crates/ohm-device-model/src/state.rs` (test `json_shape_is_ui_friendly`):

```json
{
  "device": "gpu.mock.0",
  "timestamp_ms": 1700000000000,
  "readings": [
    { "capability": "temperature.core", "status": "ok", "value": 68.5 },
    {
      "capability": "temperature.hotspot",
      "status": "unavailable",
      "reason": "unsupported",
      "detail": "mock has no hotspot sensor"
    }
  ],
  "online": true
}
```

* `online: false` (set by `DeviceState::offline(reason, detail)`) means the whole
  poll failed — e.g. the GPU powered down — and rewrites *every* reading to
  `unavailable` with that reason.
* `message` (adapter note such as "requires Administrator") and `detail` are
  omitted when absent.
* `DeviceState::set(reading)` replaces in place, so a poll can report a subset
  without dropping the rest.

## 6. `Capability`

| Field | Meaning for business logic |
|---|---|
| `id: CapabilityId` | stable, vendor-neutral (`temperature.core`) |
| `name: String` | UI label only — never match on it |
| `kind: CapabilityKind` | `sensor` = read-only measurement, `actuator` = writable set point, `event` = device-pushed, `info` = static/slow data |
| `unit: Unit` | drives rendering, epsilon selection and "is this a temperature/duty?" decisions |
| `readable`, `writable` | the flags the runtime actually enforces |
| `min`, `max`, `step` | declared range and quantisation for actuators |
| `values: Vec<String>` | when non-empty, the capability is an *enumeration*: `validate` matches text instead of numbers |
| `description`, `poll_interval_ms` | documentation and a per-capability cadence hint (`poll_interval_ms` is currently not read by the runtime) |
| `safety_critical` | marks outputs that must never be driven to an unsafe value (mock and LHM pumps set it) |

Constructors: `Capability::sensor`, `Capability::actuator` (requires min/max),
`Capability::info` (kind `Info`, unit `Text`), plus `with_step`, `with_values`,
`with_description`, `with_poll_interval_ms`, `safety_critical`.

Validation and clamping are separate on purpose:

* `Capability::validate(&Value)` **rejects** (returns `Err`) non-writable
  capabilities, non-numeric values for numeric units, non-finite numbers, values
  outside `min..=max`, and text outside an enumeration. The runtime calls it
  before writing, so a refused value never reaches an adapter.
* `Capability::clamp(f64)` **repairs**: clamps into range, then rounds to the
  nearest `step` multiple and clamps again (so `step = 5` turns 42.4 into 40 and
  43.0 into 45). `Capability::range_label()` renders `0-100 %` for the UI.

`is_duty_control()` is `kind.is_actuator() && unit.is_duty()` (unit `percent` or
`pwm`). It is the predicate the runtime, the safety policy, the emergency
supervisor and `Runtime::release_control` all use to find "cooling outputs".

## 7. Naming rules and well-known capability ids

`DeviceId`, `CapabilityId`, `AdapterId` and `RuleId` are validated newtypes
(`crates/ohm-core/src/ids.rs`): non-empty, ≤ 96 characters, lowercase ASCII
letters/digits plus `.`, `_`, `-`, `:` only, serialised transparently (a
`DeviceId` is just a JSON string). `DeviceId::compose(namespace, qualifier, index)`
lowercases and replaces anything else with `_`, which is how
`("gpu", "NVIDIA GeForce", 0)` becomes `gpu.nvidia_geforce.0`.

Convention: `<type>.<source>.<instance>`, e.g. `cpu.system.0`, `gpu.nvidia.0`,
`fan.openhardwareos_of4_0001.0`, `fan.lhm.lpc_nct6687d_0_1`. The instance is an
adapter's own addressing, not a position in a list: `compose` numbers devices that
have nothing better, while an adapter that can name a device after the hardware
does so. The LibreHardwareMonitor adapter, for example, composes its ids from LHM's
own path (`/cpu/0`, `/lpc/nct6687d/0`) and the channel number LHM reports, because
a rule that targeted `fan.lhm.3` by position would follow our enumeration order to
whichever fan happened to be third after a re-enumeration. Ids are stable while the
hardware is present, not across restarts (nothing persists them — see
`docs/architecture.md` §10).

Well-known capability ids — `ohm_core::ids::capability`, re-exported as
`ohm_device_model::caps`:

| Constant | Id | Unit |
|---|---|---|
| `TEMPERATURE_CORE` | `temperature.core` | celsius |
| `TEMPERATURE_HOTSPOT` | `temperature.hotspot` | celsius |
| `TEMPERATURE_SYSTEM` | `temperature.system` | celsius |
| `LOAD` | `load.total` | percent |
| `CPU_LOAD` | `load.cpu` | percent |
| `GPU_LOAD` | `load.gpu` | percent |
| `POWER_TOTAL` | `power.total` | watt |
| `POWER_GPU` | `power.gpu` | watt |
| `FAN_RPM` | `fan.rpm` | rpm |
| `FAN_SPEED_PERCENT` | `fan.speed_percent` | percent (actuator) |
| `FAN_PWM` | `fan.pwm` | pwm (actuator, 0–255) |
| `PUMP_SPEED_PERCENT` | `pump.speed_percent` | percent (actuator) |
| `PUMP_RPM` | `pump.rpm` | rpm |
| `CLOCK_MHZ` | `clock.mhz` | megahertz |
| `STATUS_MESSAGE` | `status.message` | text |
| `MEMORY_USED` | `memory.used` | byte |
| `DISK_FREE` | `storage.free` | byte |

### Why business logic keys off id + kind + unit, never vendor/model

The automation editor populates itself from `CapabilityRegistry::sources` (any
enabled, readable, non-text sensor) and `::targets` (any enabled, writable
actuator); a cooling curve validates its source with
`BindingRole::TemperatureSource` (`unit.is_temperature()`) and its target with
`BindingRole::CoolingActuator` (`unit.is_duty()`). The same rule text therefore
works against `gpu.mock.0 → fan.mock.0` in CI and `gpu.nvidia.0 →
fan.lhm.lpc_nct6687d_0_1` on a real machine. Matching on `device.name` or `vendor` would make
every rule a per-model special case, and would break the moment a user renames a
device or a vendor ships a new SKU.

The escape hatch is that capability ids are **free-form**: `CapabilityId::new`
only enforces the character set, so an adapter that finds something new can invent
`temperature.vrm` or `load.memory` and the UI still renders it. Only the
well-known ids get special treatment (safety floors by `DeviceType`, default
epsilon by `Unit`, curve sources by `unit.is_temperature()`).

## 8. Lifecycle

| Step | Code | Guarantee |
|---|---|---|
| 1. discover | `HardwareAdapter::discover` → `DiscoveryManager::discover_all` | adapters report only what exists now; `validate_devices` drops invalid or duplicate descriptions with a warning |
| 2. validate | `Device::validate` | ≥ 1 capability, no duplicates, writable bounds paired |
| 3. register | `DeviceTable::reconcile` / `upsert` | adds, updates (capability set changed) or leaves unchanged; `enabled` comes from `Settings::disabled_devices`; `first_seen_ms` is preserved on update; `RuntimeEvent::DeviceAdded` published |
| 4. poll | `Runtime::poll_inner` → `HardwareAdapter::read_all` (batch where implemented) | only `enabled` devices; one cycle, one refresh per adapter |
| 5. read | `DeviceState` → `StateStore::update` → `DeviceTable::apply_state_status` | readings land in the store, changes past the unit epsilon are published; `Online` only when *every* declared capability read successfully, otherwise `Degraded` |
| 6. write | `Runtime::write_value` | resolve → validate → clamp → safety → adapter → audit; the device is never written when disabled |

A device that disappears is removed by `reconcile` with
`UnavailableReason::NotPresent`, its state and history are dropped
(`StateStore::remove`) and `RuntimeEvent::DeviceRemoved` is published.
`DeviceTable::mark_adapter_offline` marks an adapter's devices offline without
removing them, which is what a crashed provider looks like.

## 9. Unit semantics and the change epsilon

`Unit` carries a rendering suffix (`Unit::suffix()`: `" °C"`, `" rpm"`, `" %"`, …)
and three classifiers used by logic: `is_temperature()` (celsius, fahrenheit),
`is_rotational_speed()` (rpm, hertz), `is_duty()` (percent, pwm).
`Unit::to_celsius(f64)` converts for the emergency supervisor, which compares
every temperature sensor on the machine in one scale.

The state store only publishes a `ReadingChanged` when the value moves by at
least the unit's epsilon (`crates/ohm-runtime/src/store.rs` —
`reading_epsilon`, used by `value_changed`):

| Unit | Epsilon |
|---|---|
| celsius, fahrenheit | 0.1 |
| percent, pwm | 0.5 |
| rpm | 1.0 |
| watt, milliwatt | 0.5 |
| volt, ampere | 0.01 |
| hertz, megahertz | 1.0 |
| byte | 1.0 |
| second, millisecond | 0.001 |
| everything else (`count`, `boolean`, `text`, `none`) | 0.0 → exact comparison |

The unit lookup is `DeviceTable::units_map()` (capability id → unit); a capability
missing from the map falls back to an exact comparison. Status transitions
(ok ⇄ unavailable) are published regardless of the epsilon, otherwise a sensor
dropout at an unchanged value would be invisible.

## 10. Which adapter produces which capabilities today

Registration order is priority order (`crates/adapters/src/lib.rs`). All ids below
are the ones the code actually builds.

| Adapter | Device ids / types | Capabilities | What it cannot do |
|---|---|---|---|
| `system` (always on, no elevation) | `cpu.system.0` (Cpu) | `temperature.core`, `load.cpu`, `clock.mhz` | live/boost clock (reports whatever `sysinfo` gives: rated clock on Windows/macOS); no fan RPM or PWM at all — the adapter's module doc states that no OS API exposes them |
| | `temperature.system.{N}` (TemperatureSensor) | `temperature.system` | only OS thermal zones; Apple Silicon exposes dozens of near-duplicates, so `devices::select_thermal_zones` picks the meaningful ones and the rest are summarised in CPU metadata |
| | `storage.system.{N}` (Storage) | `temperature.core`, `storage.free` | drive temperature only on Windows (WMI `MSFT_StorageReliabilityCounter`); elsewhere reported `unsupported`. Removable volumes skipped; duplicate (name, capacity) pairs collapsed |
| `nvidia` (NVML, write needs elevation) | `gpu.nvidia.{index}` (Gpu) | `temperature.core`, `load.gpu`, `power.gpu`, `memory.used`, `clock.mhz`, `temperature.hotspot` (always `unsupported`), and when `num_fans() > 0`: `fan.rpm` + `fan.speed_percent` (0–100, actuator) | hotspot/junction (not exposed by NVML); per-fan reads/writes (only fan index 0 is used); memory-junction temperature; fan **curve** — NVML has duty + policy only, so a curve is our own poll-and-set loop; fan write without Administrator (`NVML_ERROR_NO_PERMISSION` → `permission_denied`) |
| `lhm` (needs a running, elevated LibreHardwareMonitor with its web server on) | one device per top-level LHM hardware node, id `<type>.lhm.<LHM path>` (Cpu, Gpu, Motherboard, Storage, …) — e.g. `cpu.lhm.cpu_0`, `motherboard.lhm.lpc_nct6687d_0` | mapped from LHM sensors: `temperature.core` / `.hotspot` / `.system` (or `temperature.{slug}`), `load.cpu` / `load.gpu` (or `load.{slug}`), `power.total` / `power.gpu`, `clock.mhz`, `memory.used` | nothing without LHM running as Administrator; the out-of-process bridge transport is roadmap (HTTP JSON only today); ad-hoc ids (`temperature.vrm`) are LHM-specific and not portable to other adapters |
| | fan channel devices `fan.lhm.<LHM path>_<channel>` / `pump.lhm.<LHM path>_<channel>` — e.g. `fan.lhm.lpc_nct6687d_0_1`, `fan.lhm.gpu_0_0` when LHM's sensor name carries no number (Fan, Pump) | `fan.rpm` + `fan.speed_percent`, or `pump.rpm` + `pump.speed_percent` (pump control is `safety_critical`, 0–100) | only channels LHM exposes as `Control` sensors; board-dependent (many headers have none, some revert to firmware control); write requires LHM to be elevated |
| `opd` (Open Device Protocol) | `fan.<vendor>_<serial>.0` (Fan) | whatever the device's `DeviceDescriptor` advertises; the simulated OpenFan advertises `fan.speed_percent`, `fan.rpm`, `temperature.core`, `status.message` | no real USB enumeration yet — the only transport implemented is in-process loopback to `MockOpenFan`; the device must implement ODP itself |
| `mock` (opt-in; deterministic; never real hardware) | `cpu.mock.0`, `gpu.mock.0`, `ssd.mock.0`, `fan.mock.{N}`, `pump.mock.{N}`, `temperature.mock.{N}` | Cpu: `temperature.core`, `load.cpu`, `power.total`, `clock.mhz`; Gpu: `temperature.core`, `temperature.hotspot`, `load.gpu`, `power.gpu`, `fan.rpm`, `fan.speed_percent`; Storage: `temperature.core`, `storage.free`; Fan: `fan.rpm`, `fan.speed_percent`; Pump: `pump.rpm`, `pump.speed_percent` (60–100, `safety_critical`); TemperatureSensor: `temperature.system` | writes are reported as `WriteStatus::Simulated`, never `Applied`, so no caller can mistake simulation for hardware |

Details worth knowing when reading those tables:

* The **mock GPU fan** and the **NVML GPU fan** are both `fan.speed_percent` on a
  `Gpu` device, not a separate `Fan` device. Safety floors are chosen by
  `DeviceType`, and `DeviceType::Gpu` has no floor (`SafetyPolicy::duty_floor`
  returns 0.0), which matches the vendor reality that a GPU fan at 0 % in
  "zero-RPM" mode is normal.
* The **system adapter declares no fan capability at all** rather than showing a
  fan it cannot read. That is the model's honesty rule in practice: a missing
  capability is a limitation (`UnavailableReason::Unsupported`) and never an
  error or a fake zero.
* **`temperature.hotspot` on NVIDIA** is declared and always reported
  `unsupported` with an explanatory detail, so the UI can tell the user *where*
  the reading would come from (LHM via NVAPI) instead of silently hiding it.

## 11. Adding a capability or a device type safely

**Adding a capability id** is the cheap, safe path:

1. Add a constant to `ohm_core::ids::capability` if it is reusable, otherwise
   invent the id inside the adapter.
2. Declare it on the device (`Capability::sensor/actuator/info`) with the correct
   `Unit` — the unit, not the id, decides whether the safety floor and the
   temperature comparisons apply.
3. Nothing else changes: rules reference `device/capability` strings, the
   registry indexes by `kind` + `readable`/`writable`, and the UI formats with
   `Unit::suffix()`.

**Compatibility rules the model relies on** (verified by grep: there is no
`deny_unknown_fields` anywhere in the workspace, and `Device`, `Capability`,
`DeviceState` and `Settings` all use `#[serde(default)]` at the container or field
level):

* **New optional fields are tolerated by old readers** — unknown keys are ignored.
* **Missing fields fall back to defaults** — a payload from an older adapter still
  deserialises (`Device::vendor` defaults to `""`, `DeviceState::online` defaults
  to `true`, `Settings::history_points` to 180, and so on).
* **Optional fields stay out of the payload when unset**
  (`skip_serializing_if`), keeping frames and snapshots small.
* **`Value` is untagged**, so JSON `76.0`, `1120`, `true`, `"auto"` map to
  `Number`, `Integer`, `Bool`, `Text` — with `Integer` first so `42` round-trips
  as an integer rather than `42.0`.

**Adding a device type** requires touching a closed enum, so do it deliberately:

1. Add the variant to `DeviceType` and to `DeviceType::ALL`.
2. Add the snake_case string to `as_str` and the accepted spellings to `FromStr`.
3. Decide the classifications: `is_thermal_source`, `is_actuatable`, and —
   most importantly — `requires_safe_floor`, which is what makes the safety layer
   treat the type as a critical output.
4. If the type carries cooling outputs, check that
   `DeviceTable::cooling_devices()` (currently `Fan | Pump`) and
   `Runtime::supervise_safety` / `release_control` include it; those are the two
   places where "a cooling device" is spelled out, and forgetting them means the
   emergency ceiling and control release skip the new type.

The one-way-compatibility caveat: adding a variant to `DeviceType`,
`CapabilityKind` or `Unit` is additive for a *newer* reader, but an *older*
reader that receives the new value fails to deserialise (serde has no
"unknown enum variant" fallback here). Within one workspace that is a non-issue;
`Unit::None` and `DeviceType::Unknown` exist as explicit fallbacks, and both are
reachable today (an LHM `Other` sensor with no unit yields `Unit::None`, and an
unrecognised LHM hardware type yields `DeviceType::Unknown`).

## 12. Not implemented (roadmap)

* **`CapabilityKind::Event` has no producer.** The variant exists so push-capable
  transports (ODP `SUBSCRIBE_EVENT`) have somewhere to land, but no adapter
  declares one and the runtime has no event-capability path: ODP subscriptions are
  acknowledged by the mock device and then ignored — the runtime polls instead
  (`crates/ohm-protocol/src/lib.rs` transport docs; `unsubscribe`/`subscribed`
  only increment a counter in `MockOpenFan`).
* **`Capability::info()` is unused** in production: the only `Info` capability in
  the tree is built as a struct literal inside `MockOpenFan` (`status.message`,
  carrying the firmware version).
* **Capability-level cadence hints** (`Capability::poll_interval_ms`,
  `AdapterCapabilities::poll_interval_ms` / `discovery_interval_ms`) are declared
  and serialised but never read by the runtime.
* **No device-model versioning or migration.** Compatibility rests on serde
  defaults, not on a schema version field; there is no `model_version` in any
  payload.
* **No persisted device identity.** Ids are composed from live enumeration, so a
  device can change id between runs (for example an ODP device that gains a
  serial), and rules referencing the old id then fail validation with
  `device_not_found` rather than being migrated.
* **No device-type-specific UI logic beyond grouping**
  (`ohm_runtime::group_by_type`) and no per-type capability requirements: nothing
  enforces that a `Fan` must expose a tachometer.
