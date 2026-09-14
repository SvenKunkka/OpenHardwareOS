# OpenHardwareOS — Architecture

Rust workspace (edition 2024, MSRV 1.95) plus a Tauri v2 + React 19 desktop shell.
This document describes how the pieces fit, why the boundaries are where they are,
and what does not work yet. Every claim cites a file and a type or function.

Scope note: numbers in this document are the defaults in code. `docs/research.md`
holds the vendor/library research (citing upstream sources) that motivated the
adapter strategy; this file describes the code that exists today.

---

## 1. Layered model

```text
┌────────────────────────────── front ends ───────────────────────────────┐
│ apps/desktop/src           React 19 UI (screens, hooks, lib/ipc.ts)     │
│ apps/desktop/src-tauri     commands.rs · events.rs · state.rs · tray.rs │
│ apps/cli                   ohm-cli: doctor, status, watch, demo, rules, │
│                            audit, paths, protocol                       │
└───────────┬─────────────────────────────────────────────┬──────────────┘
            │ invoke(...)  /  "snapshot", "runtime-event"  │ direct calls
            ▼                                             ▼
┌─────────────────────── crates/ohm-runtime ────────────────────────────┐
│ Runtime            write gate, poll + discovery loops, supervisor      │
│ DeviceTable        what is attached, enabled, online (hotplug)         │
│ StateStore         latest DeviceState per device + bounded history     │
│ EventBus           tokio::sync::broadcast<RuntimeEvent>                │
│ SafetyPolicy       floors, emergency ceiling, fail-safe duty           │
│ AuditLog           append-only JSONL of every write                    │
│ DiscoveryManager   owns the adapters, their probe() health             │
│ ConfigPaths/Settings  settings.json, rules/, logs/, audit.jsonl        │
└───────┬───────────────────────────────────────────▲───────────────────┘
        │ Arc<dyn HardwareAdapter>                  │ RuntimeEvent
        ▼                                           │ Runtime::write_value
┌── crates/ohm-adapter-api ──┐            ┌── crates/ohm-automation ──────┐
│ trait HardwareAdapter      │            │ AutomationEngine (tick loop)  │
│ AdapterInfo/Status         │            │ Rule → Curve → evaluator      │
│ WriteOutcome, AdapterState │            │ RuleStore  (rules/*.yaml)     │
└───────┬────────────────────┘            └───────────────────────────────┘
        │ implemented by
        ▼
┌──────────────────────────── adapters ─────────────────────────────────┐
│ crates/adapters        build_adapters(), AdapterOptions (which run)    │
│ adapters/system        OS sensors (sysinfo, Windows storage counters)  │
│ adapters/nvidia        NVML telemetry + nvmlDeviceSetFanSpeed_v2       │
│ adapters/libre-hardware-monitor  LHM HTTP JSON + fan control channels  │
│ adapters/open-protocol OpdAdapter — ODP devices                        │
│ adapters/mock          deterministic simulated PC, faults, profiles    │
└───────────────────────────────┬───────────────────────────────────────┘
                                │ crates/ohm-protocol (ODP v0.1)
                                ▼
                       hardware / firmware / USB device
```

Supporting crates that sit under everything and depend on nothing vendor specific:

| Crate | Contains | Depends on |
|---|---|---|
| `crates/ohm-core` | `DeviceId`/`CapabilityId`/`AdapterId`/`RuleId`, `OhmError`, `ConfigPaths`, time helpers, logging | — |
| `crates/ohm-device-model` | `Device`, `DeviceType`, `Transport`, `Capability`, `Value`, `DeviceState`, `Reading`, `Unit` | `ohm-core` |
| `crates/ohm-adapter-api` | the `HardwareAdapter` trait and its result types | `ohm-core`, `ohm-device-model` |
| `crates/ohm-protocol` | Open Device Protocol: framing, messages, descriptors, transports, mock device | `ohm-core`, `ohm-device-model` |
| `tests/` (`ohm-integration-tests`) | `Session` harness: a real runtime with simulated hardware | runtime, automation, adapters |

Dependency direction is one-way: `ohm-runtime` depends on `ohm-adapter-api` but
**not** on any adapter crate; `crates/adapters` depends on `ohm-runtime` (for
`Settings`) and on the concrete adapters. The runtime therefore never learns a
vendor name — it is handed `Vec<Arc<dyn HardwareAdapter>>`
(`crates/ohm-runtime/src/runtime.rs` — `Runtime::new`).

### Why adapters are behind a trait, not a match statement

Every alternative design leaks vendor knowledge into business logic: an `enum
Provider` in the runtime forces a recompile of the core for each new device, and
a `match device.vendor` in the automation engine makes rules model specific.
`HardwareAdapter` (`crates/ohm-adapter-api/src/lib.rs`) exists so that the same
rule text drives `gpu.mock.0` in CI and `gpu.nvidia.0` on a real machine. The
rejected tradeoff is an extra layer of indirection and `async_trait` boxing on
every poll — accepted because polling is 1 Hz, not 1 kHz.

---

## 2. The hard rule: one write path

**The UI never talks to an adapter.** Every actuator write goes through
`Runtime::write_value` (`crates/ohm-runtime/src/runtime.rs`), and therefore
through, in this order: device resolution → `Capability::validate` → range clamp
→ `SafetyPolicy::check_duty` → dry-run check → the adapter → audit log → event
bus.

Structurally enforced at three points:

1. **The Tauri command layer only holds `Runtime` and `AutomationEngine`.**
   `apps/desktop/src-tauri/src/state.rs` — `AppState { runtime, engine, paths, .. }`.
   `apps/desktop/src-tauri/src/commands.rs` — the only write command,
   `write_capability`, is a three-line wrapper around
   `state.runtime.write_value(&device, &capability, value, WriteOrigin::Manual)`.
2. **The frontend has exactly one IPC entry point.**
   `apps/desktop/src/lib/ipc.ts` wraps `invoke` in a single `call<T>()`; every
   screen imports `api.*`. There are no other `invoke(` call sites in
   `apps/desktop/src`.
3. **Adapters are only reachable through the runtime.** In the whole workspace
   there is exactly one non-test call to `HardwareAdapter::write`:
   `crates/ohm-runtime/src/runtime.rs` (inside the write gate). Adapters
   themselves are non-`pub` implementation details of `crates/adapters`.

The rule is a reviewable property of the command layer, not a type-system
guarantee: `Runtime::adapter(id)` is `pub` (used by the mock panel, see the
limitation in §10). Anyone adding a new command must call
`Runtime::write_value`; there is no other sanctioned door.

### The one deliberate exception

The simulator panel (`apps/desktop/src-tauri/src/commands.rs` — `mock_set_load`,
`mock_apply_profile`, `mock_set_ambient`, `mock_force_gpu_temperature`,
`mock_set_faults`) downcasts `Runtime::adapter("mock")` to the concrete
`MockAdapter` via `as_any()` and drives the *load generator* (`set_gpu_load`,
`set_load_profile`, `tick`). These calls change what the simulated hardware
would report; they never write an actuator, and they cannot exist for a real
adapter. `HardwareAdapter::as_any` exists for exactly this, and the mock is the
only implementor that is downcast in production code.

---

## 3. Concurrency model

### What runs where

| Task | Spawned by | Cadence | Work |
|---|---|---|---|
| Runtime poll loop | `Runtime::spawn_loops` | `Settings::polling_interval_ms`, default 1000 ms (100–60000) | `Runtime::poll_inner` then `Runtime::supervise_safety` |
| Runtime discovery loop | `Runtime::spawn_loops` | `Settings::discovery_interval_ms`, default 5000 ms (500–600000, forced ≥ poll interval) | `Runtime::refresh_inner` (probe + discover + reconcile) |
| Automation tick | `AutomationEngine::start` | `TICK_INTERVAL_MS = 100` ms | evaluate due rules; per rule `Rule::update_interval_ms`, default 1000 ms |
| Tauri event bridge | `events::spawn` | reactive | forward `RuntimeEvent`, push `RuntimeSnapshot` at most every `SNAPSHOT_INTERVAL = 250` ms (4 Hz), plus a 1 s `HEARTBEAT` |
| Tray actions | `src-tauri/src/tray.rs` | on click | Rescan → `refresh_devices` + `poll_once` + snapshot; Quit → `engine.stop()` + `runtime.shutdown()` |

The desktop app starts the runtime and the engine from Tauri's async runtime
(`apps/desktop/src-tauri/src/lib.rs`, `.setup(...)`): `runtime.start().await`,
then `engine.ensure_example()`, then `engine.start().await`. The CLI does the
same three calls per subcommand, which is why CLI and GUI behaviours cannot
diverge.

### Locks and what they protect

| Lock | Type | Protects |
|---|---|---|
| `RuntimeInner.settings` | `parking_lot::RwLock<Settings>` | the live settings; loops re-read it every cycle, so an interval change needs no restart |
| `RuntimeInner.table` | `parking_lot::RwLock<DeviceTable>` | device records, enabled flags, online status |
| `RuntimeInner.write_gate` | `tokio::sync::Mutex<()>` | one adapter write at a time, so two rules cannot interleave on one actuator |
| `RuntimeInner.tasks` | `parking_lot::Mutex<Vec<JoinHandle<()>>>` | loop handles, for shutdown |
| `StateStore.latest`, `.history` | `parking_lot::RwLock<HashMap<..>>` | authoritative readings; a `DeviceState` is stored as `Arc` so readers never block writers for long |
| `DiscoveryManager.statuses` | `parking_lot::RwLock<BTreeMap<AdapterId, AdapterStatus>>` | adapter health |
| `AutomationEngine.rules`, `.states` | `parking_lot::RwLock<Vec<Rule>>`, `Mutex<HashMap<RuleId, RuleState>>` | rule set and per-rule hysteresis/deadband state |
| `AuditLog.lock` | `parking_lot::Mutex<()>` | serialises append-only JSONL writes |
| `OpdAdapter.transport` | `parking_lot::Mutex<LoopbackTransport<..>>` | one ODP link is a single resource: frames must not interleave |

Cross-task signalling uses `tokio::sync::watch` (`stop_tx`) for shutdown and
`tokio::sync::broadcast` (`EventBus`, capacity `EVENT_CHANNEL_CAPACITY = 2048`)
for events. Counters are plain atomics (`Stats`, `RuntimeInner.running`,
`emergency_active`).

### The rule: never hold a lock across an `.await`

It is followed deliberately, and the two places where it would be easiest to get
wrong show the pattern:

* `Runtime::write_with_origin` copies the device and capability out of the table
  inside a scoped block, *then* awaits the adapter while holding only the async
  `write_gate` (an async mutex exists precisely so holding it across `.await` is
  legal).
* `AutomationEngine::tick` clones the rule's `RuleState` out of the `states`
  `parking_lot::Mutex` in a scoped block and only then calls
  `run_rule(..).await`; `run_rule` uses `self.remember(...)` to write the state
  back afterwards.
* `DiscoveryManager::discover_all` awaits adapter I/O with no lock held and
  stores statuses afterwards, which is why `discovery.rs` documents discovery as
  "slow, I/O bound, never holds a lock".

The corollary: a blocking lock held across `.await` would deadlock a
single-threaded executor and stall a multi-threaded one; the codebase avoids
`tokio::sync::Mutex` everywhere except the write gate, where serialisation is the
point.

Blocking calls are confined to short windows: the OS adapter's
`sysinfo` refresh, the audit file append and settings saves are synchronous
`std::fs` work executed on the calling task. Write rates are bounded by
`Rule::update_interval_ms` (rejected below `MIN_UPDATE_INTERVAL_MS` = 100 ms by
`Rule::validate`, and warned about below 250 ms by
`AutomationEngine::check_rule`) and by the manual UI path, so this is acceptable
today — but it is blocking I/O on a tokio worker, not offloaded with
`spawn_blocking`.

---

## 4. Read path: discovery → reconcile → poll → diff → bus → UI

```text
discovery loop ─▶ DiscoveryManager::discover_all(&settings)
                     ├─ probe() per adapter (disabled ⇒ UnavailableReason::Disabled)
                     ├─ validate_devices(): drop devices an adapter described wrongly
                     └─ zero devices ⇒ AdapterStatus::Degraded(NotPresent), not an error
                  ─▶ DeviceTable::reconcile(adapter, devices, settings, now)
                     ├─ added  ⇒ RuntimeEvent::DeviceAdded
                     ├─ removed⇒ store.remove + RuntimeEvent::DeviceRemoved(NotPresent)
                     └─ updated⇒ capability set or metadata changed
poll loop ────▶ Runtime::poll_inner
                     ├─ table.poll_targets()  (enabled devices only)
                     ├─ adapter.read_all(&batch)     ← batch API, one refresh per cycle
                     ├─ StateStore::update(state, &table.units_map())
                     │     └─ ReadingChange per reading past the unit epsilon
                     ├─ DeviceTable::apply_state_status  (Online/Degraded/Offline)
                     └─ publish ReadingChanged + StateChanged per device
                  ─▶ Runtime::supervise_safety (independent of any rule)
                  ─▶ EventBus(broadcast) ─▶ events::spawn ─▶ "runtime-event" + "snapshot"
automation ───▶ AutomationEngine::tick ─▶ Rule evaluation ─▶ Runtime::write_value
```

Key properties, each with its reason:

* **Reconciliation is a table operation, not an adapter one.** `discover()` returns
  what exists *now*; anything not returned is gone. That is how hotplug works
  without a per-adapter subscription API, at the cost of a full re-enumeration
  every `discovery_interval_ms`. `AdapterInfo::supports_hotplug` exists to declare
  that an adapter could do better, but no reader in `crates/ohm-runtime` branches
  on it yet: the discovery cadence is a single global setting.
* **Change detection is unit aware.** `store::reading_epsilon(Unit)` (0.1 °C,
  1 rpm, 0.5 %, …) decides whether a reading is published as a change. Without
  it, sensor jitter would flood the 2048-slot bus and make the UI redraw for
  noise. Transitions in and out of `unavailable` are always published, even with
  an unchanged value.
* **Events are notifications, never truth.** `StateStore` is authoritative; a
  subscriber that lags is dropped to `RecvError::Lagged` and the Tauri bridge
  resynchronises by emitting a full snapshot (`events.rs`).
* **Batch reads.** `HardwareAdapter::read_all` has a serial default; the system,
  NVML and LHM adapters override it so one cycle does one OS/NVML/HTTP refresh
  rather than one per device.
* **Online ≠ everything readable.** `DeviceTable::apply_state_status` marks a
  device `Degraded` when any declared capability is missing, and the per-reading
  `UnavailableReason` says which.

---

## 5. Write path: UI → command → validate → clamp → safety → adapter → audit → events

```text
React control
  └─ api.writeCapability(device, capability, value)        apps/desktop/src/lib/ipc.ts
      └─ invoke("write_capability", ..)                     commands.rs
          └─ to_value(json) → Value::{Integer,Number,Bool,Text}
              └─ Runtime::write_value(.., WriteOrigin::Manual)     runtime.rs
                  1. resolve device + capability from the DeviceTable (no lock across await)
                  2. refuse: device disabled (device_disabled)
                             capability not writable (capability_read_only)
                  3. Capability::validate(value)                  (invalid_value / value_out_of_range,
                                                                    NaN rejected, enumerations checked)
                  4. Capability::clamp(numeric)                   range + `step` quantisation
                  5. SafetyPolicy::check_duty(device_type, value, current, hottest)
                       Allow{..} | Emergency{..} | Block{..}  → safety_blocked
                  6. Settings::dry_run ⇒ audited WriteReport{status: Simulated}, hardware untouched
                  7. adapter lookup (adapter_unavailable when not registered)
                  8. write_gate.lock().await → adapter.write(device, capability, value).await
                  9. WriteReport{requested, applied, clamped, status, origin, simulated}
                 10. AuditLog::record + EventBus::WritePerformed
                 11. on error: WriteOrigin::Safety fail-safe duty to a duty control, once
                     (allow_fail_safe = false for that retry, so it cannot recurse)
```

Consequences worth stating explicitly:

* **A refusal is an event too.** Every early return goes through
  `Runtime::reject`, which writes an audit entry with
  `status: Rejected` and `error_code`, and publishes `WriteRejected`. "Nothing
  happened" is never silent.
* **Only duty controls go through the safety gate.** `check_duty` is invoked when
  `capability.is_duty_control()` (kind `actuator` **and** unit `percent`/`pwm`).
  A future non-duty actuator (LED brightness is a duty; a display mode is not)
  gets range validation and the audit log but not a duty floor.
* **Clamping is reported, not hidden.** `WriteReport.clamped` is set when either
  the declared range or the safety layer changed the value, and
  `WriteReport::summary()` renders it for the activity feed.
* **The write gate is async and narrow.** It covers only the adapter call, so a
  slow ODP device cannot block poll cycles.
* **Dry run is a settings flag** (`Settings::dry_run`), read on every write.

`WriteOrigin` (`crates/ohm-runtime/src/audit.rs`) records who asked:
`Manual`, `Automation{rule_id}`, `Safety{reason}`, `Startup`, `Shutdown`, `Api`.
`Api` and `Startup` are declared but never constructed today (see §11).

---

## 6. Safety architecture

Safety is layered so that no single failure reaches the hardware. Ordered from the
outside in:

| Layer | Where | What it does |
|---|---|---|
| Device-declared range | `Capability::{min,max,step}` + `Capability::validate`/`clamp` | refuses/re-quantises anything outside what the device says it accepts |
| Rule limits | `Rule::{min_output,max_output,deadband,hysteresis}` | tighter clamp after the curve; suppresses writes smaller than `deadband` (0.5 %) and slows down only after `hysteresis` (2.0) source units of drop |
| Duty floor | `SafetyPolicy::duty_floor` | fans ≥ `min_duty_percent` (25 %) when `require_min_duty`; pumps ≥ `pump_min_duty_percent` (60 %) **unconditionally** — the floor survives both `enabled: false` and `require_min_duty: false` |
| Ramp limiting | `SafetyPolicy::max_write_delta_percent` | optional slew limit per write; default 0 (off) because some fans stall when ramped slowly |
| Emergency ceiling | `SafetyPolicy::emergency_override` + `check_duty` step 1 | at/above `emergency_temp_c` (90 °C) a write is raised to `emergency_duty_percent` (100 %) |
| Emergency supervisor | `Runtime::supervise_safety`, after every poll | independent of rules: forces every writable duty control on fan/pump devices to the emergency duty, publishes `SafetyTriggered{kind:"emergency"}`, audits it, and only acts once per episode |
| Fail-safe on write failure | `Runtime::write_with_origin` error branch | a failed duty write triggers one retry with `WriteOrigin::Safety` at `fail_safe_duty` (70 %, never below the floor) |
| Rule-level fallback | `Rule::fallback` (`on_sensor_missing`, `on_write_failure`) | a rule that loses its sensor drives the safe default instead of freezing a fan |
| Device-side fallback | `crates/ohm-protocol` — `MockOpenFan` | if the *host* stops talking for `DEFAULT_FALLBACK_AFTER_MS` (5000 ms) the device drives itself to `DEFAULT_FALLBACK_DUTY` (70 %). Official hardware is required to implement this (`adapters/open-protocol/src/lib.rs` — `OpdAdapter::shutdown` documents that the device owns its fallback) |
| Relinquish on exit | `Runtime::shutdown` → `release_control()` + `HardwareAdapter::shutdown` | with `relinquish_on_exit` (default true) every controlled output is first set to the fail-safe duty, then each adapter is asked to hand hardware back to firmware/BIOS (`WriteOrigin::Shutdown`, audited) |
| Audit | `AuditLog` | every write, applied or refused, plus lifecycle entries (`runtime_started`, `emergency_override`, `control_released`, `runtime_stopped`) |

`Settings::sanitise` → `SafetyPolicy::sanitise` repairs contradictory
configuration (a pump floor below the fan floor, a fail-safe below the floor, an
emergency duty under 50 %, an emergency temperature above 130 °C) rather than
refusing to start.

The supervisor can be switched off only as a whole
(`emergency_override_enabled`, default true); knowing it exists is not optional —
`safety.rs` is a mandatory gate on the write path, which is why it lives in the
runtime rather than in the automation engine.

---

## 7. Configuration and local-first storage

`ConfigPaths` (`crates/ohm-core/src/paths.rs`), root overridable with
`OHM_CONFIG_DIR` (used by the test-suite and by a portable build):

| Path | Contents |
|---|---|
| `<root>/settings.json` | `Settings` (polling/discovery intervals, safety policy, UI flags, `disabled_adapters`, `disabled_devices`, `adapter_settings` bags) |
| `<root>/rules/<id>.yaml` | one file per automation rule (`RuleStore::path_for`) |
| `<root>/logs/openhardwareos.log.<date>` | daily-rotating tracing log (`tracing_appender::rolling::daily`) |
| `<root>/audit.jsonl` | append-only JSON Lines: `{at_ms, at, kind: "write"|"lifecycle", report|action, detail}` |

* **Atomic saves.** `SettingsStore::save` writes `settings.json.tmp` and renames
  it over the target, so a crash cannot leave a truncated configuration
  (`RuleStore::save` does the same with a YAML header comment).
* **Corrupt-file quarantine.** An unparseable `settings.json` is renamed to
  `settings.json.corrupt` and defaults are used: the app always starts. Unknown
  keys are ignored and missing keys fall back to defaults (test:
  `partial_and_unknown_keys_are_tolerated`). A broken *rule* file is collected in
  `LoadReport::errors`, reported as a warning and skipped — never fatal.
* **No cloud, no account, no telemetry.** Everything the runtime writes is under
  the config root; nothing in the workspace performs a network call except the
  LHM adapter's `GET /data.json` against `127.0.0.1` (default
  `DEFAULT_BASE_URL`/`DEFAULT_PORT`).
* **History is in-memory.** `StateStore` keeps `Settings::history_points`
  (default 180) samples per capability; nothing is persisted across restarts.

---

## 8. Failure-handling philosophy

`UnavailableReason` (`crates/ohm-device-model/src/state.rs`) is the vocabulary
that keeps a missing sensor from looking like a broken program:

* **"Unsupported" is not an error.** `UnavailableReason::is_failure()` is true
  only for `ReadError`, `Timeout` and `Unknown`. `Unsupported`,
  `NotPresent`, `PermissionDenied`, `HardwareLimitation`, `VendorLimitation`,
  `DriverMissing` and `Disabled` are *limitations*: the UI shows them grey with
  the supplied detail text, and `DeviceState::has_failure()` returns false. A
  machine with an AMD GPU therefore does not look "broken" for having no NVML.
* **Adapters must explain themselves.** `HardwareAdapter::probe` returns
  `AdapterStatus` with `state` (`available`/`degraded`/`unavailable`/`error`) and
  a machine-readable `reason` plus a human `detail`. `DiscoveryManager::probe`
  substitutes `Disabled` when the user turned the provider off, and
  `discover_all` records `Degraded(NotPresent)` when an adapter starts but finds
  nothing — the difference between "not installed" and "installed, nothing
  attached" is visible in Settings → Providers.
* **One bad adapter cannot take the system down.** `discover_all` contains
  discovery errors per adapter; a failed poll marks only that device offline
  (`DeviceTable::mark_offline`) and publishes a warning `RuntimeEvent::Log`.
* **Errors carry a hint.** `OhmError::code()` / `hint()` /
  `is_unsupported()` are surfaced by the Tauri layer as
  `CommandError { code, message, hint, unsupported }`, so the UI can say "run as
  Administrator" instead of showing a stack trace.
* **The UI never invents a zero.** `apps/cli/src/main.rs` — `render()` prints
  `[unsupported]` rather than `0`.

---

## 9. Extension points

**Add an adapter**

1. New crate under `adapters/<name>` (`src/lib.rs` with a crate-level doc
   explaining what it can and cannot do), added to the workspace `members` and
   `workspace.dependencies` in the root `Cargo.toml`.
2. Implement `HardwareAdapter`: `info`, `probe`, `discover`, `read_state`,
   `write`, optional `read_all`, `shutdown`, `as_any`. Adapters must never panic
   on missing hardware and must return `WriteOutcome::rejected`/an error instead
   of pretending a write succeeded.
3. Declare capabilities with the well-known ids from
   `ohm_core::ids::capability` where one exists.
4. Register it in `crates/adapters`: add a field to `AdapterOptions`, honour it
   in `build_adapters` and in `AdapterOptions::from_settings`, and add it to
   `adapter_catalogue()` so Settings → Providers can list it while disabled.
   **Registration order is priority order**: adapters are probed in build order,
   `deduplicate_adapters` keeps the first adapter registered under a duplicate
   adapter id, and two adapters that report the same device id contend for one
   `DeviceTable` entry — which is exactly why NVML steps aside when
   LibreHardwareMonitor is present.

**Add a capability id** — add a constant to `ohm_core::ids::capability`, declare
it on the device with `Capability::sensor/actuator/info`, and it automatically
appears in `CapabilityRegistry::index` (sources vs targets) and therefore in the
automation editor. No runtime change is needed: capability ids are opaque strings
(`CapabilityId` only enforces lowercase ascii + `.`/`_`/`-`/`:` and ≤ 96 chars).

**Add a rule source aggregate** — `Aggregate` (`crates/ohm-automation/src/rule.rs`)
currently implements `Max`, `Min`, `Avg` via `Aggregate::reduce(&[f64])`, used by
`Source::Combined`. A new aggregate is one enum variant plus one arm in `reduce`,
plus its `as_str`/`FromStr` entries.

**Add a device type** — `DeviceType` (`crates/ohm-device-model/src/device.rs`) is
a closed enum with `ALL`, `as_str`, `FromStr` (accepting synonyms) and the
classification helpers `is_thermal_source`, `is_actuatable`,
`requires_safe_floor`. `requires_safe_floor` is what makes the safety layer treat
a new type as a pump-like critical output.

---

## 10. Known limitations

Specific and current, not aspirational:

1. **`Runtime::adapter(id)` is public.** The write-path rule is enforced by the
   command layer, not by types; new code could bypass `Runtime::write_value` by
   calling `adapter.write(...)` directly. Review, not the compiler, protects this.
2. **`--dry-run` is not persisted, by design.** `lib.rs` folds the flag into
   `Settings::dry_run` through `apply_startup_overrides` before the runtime is
   built, so it really does stop writes (they are audited as `simulated`), but it
   is per-invocation: the saved `settings.json` is untouched, and the UI checkbox
   is what makes it permanent.
3. **The adapter set is fixed at startup.** Enabling simulated hardware or the ODP
   device in Settings requires a restart; `commands::update_settings` logs that
   instead of pretending otherwise. *Devices* are hot-pluggable, adapters are not.
4. **Blocking I/O on async workers** (audit append, settings save, `sysinfo`
   refresh) is not offloaded with `spawn_blocking`.
5. **The safety gate covers duty controls only** (`Capability::is_duty_control`).
6. **The emergency supervisor is disabled together with automation**: it returns
   early when `automation_enabled` is false, so a user who turns automation off
   loses the rule-independent ceiling too.
7. **Fan control is best-effort by vendor.** NVML offers duty % plus a policy enum
   and needs elevation; LHM needs a running, elevated GUI with its web server
   enabled; laptops/ECs are out of scope. See `docs/research.md` §2 for the
   hardware reality and `adapters/*/src/lib.rs` for what each adapter admits.
8. **No persistence of device identity or history** across restarts: device ids
   are derived from live enumeration, and history is a bounded in-memory ring.
9. **Snapshot cost.** `events.rs` pushes a full `RuntimeSnapshot` (every device,
   capability and reading) at up to 4 Hz; there is no delta protocol to the
   webview.
10. **Rule priorities are evaluated, not arbitrated.** Two rules writing the same
    actuator are serialised by the write gate, but the last writer wins; there is
    no conflict detection at save time.
11. **Adapter self-description is informational.** `AdapterCapabilities`
    (`can_write`, `can_control_cooling`, `write_requires_admin`,
    `poll_interval_ms`, `discovery_interval_ms`) and
    `Capability::poll_interval_ms` are declared, serialised and shown in
    Settings → Providers, but no code in `crates/ohm-runtime` reads them: the poll
    and discovery cadences are global settings, and every bundled adapter passes
    `None` for the per-adapter intervals.
12. **The workspace parses rules with `serde_yaml_ng` 0.10**, which
    `docs/research.md` §8 does not recommend (frozen upstream, pulls
    `unsafe-libyaml`). The recommendation there is `serde-saphyr` 1.2.0 behind a
    single config module, which is where YAML is already isolated
    (`crates/ohm-automation/src/store.rs` is the only YAML reader/writer).

## 11. Not implemented (roadmap)

Each of these is referenced in code as future work, not present:

| Item | Status |
|---|---|
| Out-of-process LibreHardwareMonitor bridge (named pipe / stdio) | roadmap — `adapters/libre-hardware-monitor/src/lib.rs` transport table; today only the HTTP JSON path exists |
| Real USB HID / CDC enumeration for ODP devices | roadmap — `crates/ohm-protocol/src/lib.rs` status table ("roadmap (v0.4)"); only `LoopbackTransport` and the in-process mock device exist |
| A remote/API write client | not implemented — `WriteOrigin::Api` exists but is never constructed |
| Startup-time writes | not implemented — `WriteOrigin::Startup` exists but is never constructed |
| Cross-platform vendor GPU fan control (AMD ADL/ADLX) | deliberately excluded, licensing — `docs/research.md` §4; AMD telemetry arrives via LHM instead |
| Rule conditions, scenes/profiles, natural-language automation | not implemented — `crates/ohm-automation/src/lib.rs` describes the intended `WHEN/THEN` shape; only `sensor → curve → actuator` exists |
| Kernel driver / PawnIO integration of our own | out of scope — `docs/research.md` §1.5 and §2.3 |
