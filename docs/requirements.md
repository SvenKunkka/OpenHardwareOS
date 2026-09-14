# Requirements traceability

This document maps **every clause of the original OpenHardwareOS brief** to what
exists in this repository, and states honestly what does not. It exists because
two earlier statements in the docs contradicted each other — the roadmap called
v0.1 "shipped" while a review found requirements from the brief's *minimum* list
still missing. The rule from here on:

> A clause is **Implemented** only when the code path exists, is reachable from
> the app or the CLI, and is covered by a test that would fail if it broke.
> Anything else is **Partial**, **Missing**, or **Unverified on hardware**.

Last reviewed: 2026-09-14 (round 2), against commit `096e13b` (no release tag yet).
Test baseline at that revision: **413 Rust tests + 27 frontend behaviour tests, 0
failures**, with `cargo clippy --workspace --all-targets -- -D warnings` clean.
Commands, environment and per-command results are in `docs/verification-log.md`;
that file is the authority for any number quoted here.

## Status vocabulary

| Status | Meaning |
|---|---|
| **Implemented** | Code + reachable + tested in this repository. |
| **Partial** | Works, but with a stated limitation (platform, provider, or scope). |
| **Unverified on hardware** | Implemented and tested against simulated or in-process endpoints; never exercised on the real device class. |
| **Missing** | The brief asks for it; nothing implements it. |
| **Deferred by design** | The brief asks for it *later* (Phase 2 / roadmap); deliberately not in this MVP. |

---

## 一.1 Read PC hardware state

| Required | Status | Where | Notes |
|---|---|---|---|
| CPU name | Implemented | `adapters/system/src/lib.rs` (`SystemAdapter::build_devices`) | `sysinfo` brand string; tested on macOS and in CI |
| CPU temperature | Implemented | same, via `sysinfo::Components` | Real reading verified on this machine (Apple M5, ~69 °C). Where no package sensor exists, the reading is `unsupported`/`hardware_limitation`, never 0 |
| CPU load | Implemented | same | |
| CPU package power | **Partial** | `adapters/libre-hardware-monitor/src/mapping.rs` maps LHM's CPU power sensor to `power.total` | **Windows + LibreHardwareMonitor only.** There is no native collector: no RAPL, no MSR, no WMI power path in this build. Absence is reported explicitly by `ohm-cli doctor` ("Capability notes") rather than as `0 W` |
| GPU name / temp / load / power | Implemented | `adapters/nvidia`, `adapters/libre-hardware-monitor` | NVML for NVIDIA; LHM for any GPU |
| GPU hotspot | **Partial** | declared by `adapters/nvidia`, reported `unsupported` | NVML has no hotspot sensor; only LHM (NVAPI) provides it. The capability is declared so the UI can explain *why* it is missing |
| GPU fan RPM | Implemented | `adapters/nvidia` (`fan_speed_rpm`), `adapters/libre-hardware-monitor` | NVML implementation varies per driver; a refusal becomes `unsupported` |
| SSD name / temperature | Implemented | `adapters/system` (Windows WMI reliability counters), `adapters/libre-hardware-monitor` | **Unverified on hardware** on Windows; on platforms that expose nothing the reading says so |
| System fan RPM | Implemented (Windows) | `adapters/libre-hardware-monitor` (pairs `Fan #N` with `Fan Control #N`) | **Unverified on hardware** |
| System fan PWM read/write | Implemented (Windows) | same, via LHM `/Sensor?action=Set` | **Unverified on hardware**; requires LHM running as Administrator |
| Must not crash; must show `unsupported` / `unavailable` | Implemented | `ohm_device_model::UnavailableReason` + `Reading` | Ten reasons, surfaced per capability in the UI, CLI and protocol |

## 一.2 Unified device model

| Required | Status | Where |
|---|---|---|
| Device / capability abstraction, no vendor branches | Implemented | `crates/ohm-device-model` (28 unit tests) |
| Documented JSON shape (`id`, `name`, `type`, `vendor`, `transport`, `capabilities`) | Implemented | `crates/ohm-device-model/src/lib.rs` — `documented_device_model_shape` asserts it |
| Sensor vs actuator, units, ranges | Implemented | `Capability`, `Unit`, `Capability::validate`/`clamp` |
| Business logic depends only on type + capabilities | Implemented | automation targets capabilities, never models — `crates/ohm-automation/src/evaluator.rs` |

## 一.3 Hardware runtime

| Required module | Status | Where |
|---|---|---|
| Device Manager | Implemented | `crates/ohm-runtime/src/device_table.rs` |
| Discovery Manager | Implemented | `crates/ohm-runtime/src/discovery.rs` |
| Capability Registry | Implemented | `crates/ohm-runtime/src/registry.rs` |
| State Store | Implemented | `crates/ohm-runtime/src/store.rs` |
| Event Bus | Implemented | `crates/ohm-runtime/src/bus.rs` |
| Driver / Adapter Layer | Implemented | `crates/ohm-adapter-api` + `adapters/*` |
| Automation Engine | Implemented | `crates/ohm-automation` |
| Persistence | Implemented | `config.rs` (settings), `RuleStore` (rules), `audit.rs` (audit trail) |
| Logging | Implemented | `crates/ohm-core/src/logging.rs` |
| Discover / register / read / write / publish / subscribe / enable-disable / persist | Implemented | `Runtime` public API; "UI may not call a driver" is enforced by the Tauri command layer, which only exposes runtime calls |

## 一.4 Automation engine

| Required | Status | Where | Notes |
|---|---|---|---|
| Numeric condition | **Implemented** (round 1) | `crates/ohm-automation/src/rule.rs` (`Condition`, `Comparator`), `evaluator.rs` (`resolve_gate`, `evaluate_gated`) | `when {source, op, value, otherwise}`. Six operators; `eq`/`ne` use a documented tolerance and warn on analog readings. Non-finite thresholds are rejected at validation; non-finite readings never satisfy a condition |
| Curve mapping | Implemented | `curve.rs` | Linear interpolation, clamped at both ends |
| Min / Max | Implemented | `Rule::min_output` / `max_output` + the device range | Whichever is tighter |
| Deadband / hysteresis | Implemented | `evaluator.rs` | Both documented and tested at their boundaries |
| Update interval | Implemented | `update_interval_ms`, `MIN_UPDATE_INTERVAL_MS` | |
| Emergency fallback | Implemented | `Rule::fallback` + `SafetyPolicy` + the runtime supervisor | |
| Condition false must not hold a stale fan value | Implemented | `evaluate_gated` | Drives `otherwise` (fail-safe duty by default, or an explicit percent). Unbounded "hold" is deliberately not offered; see `OtherwiseAction` docs |
| `release` only when the adapter can really release | Implemented by refusal | `OtherwiseAction` has no `release` variant; `FallbackAction::Release` is rejected by `Rule::validate` and sanitised on load | No adapter in this build advertises a mid-run release channel, so offering one would be a promise we cannot keep — and a `release` fallback that silently did nothing was a real defect in this build (see the round-2 entry in `docs/verification-log.md`). `release` can no longer be saved, imported, enabled or loaded into the active set; a hand-written `release:` in an old rule file is left **untouched on disk**, replaced in memory by the fail-safe duty, and reported as a per-field compatibility note. LHM's `SetDefault` release happens on shutdown |
| A write that was accepted but **not** confirmed must not be reported as applied | **Implemented** (round 2) | `WriteStatus::Unconfirmed` + `WriteOutcome::is_confirmed` (`crates/ohm-adapter-api`); `Runtime::write_value`; `crates/ohm-automation/src/engine.rs` | Three outcomes are now distinguishable end to end: the request was accepted, the value was **read back and confirmed**, or the result is failed/unknown. An unconfirmed write carries **no** value, so it can never become the recorded `applied_output`, never suppresses the next attempt, and the engine retries it — after `MAX_CONSECUTIVE_UNCONFIRMED` (3) consecutive unconfirmed writes the rule's write-failure policy runs the fail-safe duty. The read-back for LHM is the *channel set point*, which says nothing about airflow, and the detail says so |
| Editing a rule must not inherit the previous target's state | **Implemented** (round 2) | `RuleChange` / `apply_rule_change` / `perform_pending_handovers` in `crates/ohm-automation/src/engine.rs`; the same reconciliation runs for `save_rule` and `load_rules` | Changing `target`, `source` or `when` (and `otherwise`, which drives the output when the gate is closed) discards the rule's applied/held state, so the new output is driven from the new target's own evidence on the first cycle instead of being skipped by "unchanged value" dedup. The abandoned output is handed to the fail-safe duty at the next cycle and the handover is audited with the reason. Metadata-only edits (name, description, interval) apply at the next cycle without interrupting control |
| Condition source missing/stale follows the sensor policy | Implemented | `resolve_gate` | Same `fallback.sensor_timeout_s` grace period, same anchor discipline as the main source |
| Emergency protection must not be blockable by `when` | Implemented | `crates/ohm-runtime/src/runtime.rs` (`supervise_safety`) | The supervisor is independent of rules; a gated rule's write is still clamped by `SafetyPolicy::check_duty` |
| Extensible to WHEN/IF/AND/OR/THEN, SCENE, PROFILE, EVENT | Deferred by design | `docs/roadmap.md` (v0.6) | No event system has been introduced, per the brief's "do not over-engineer" rule |

## 一.5 Desktop UI

| Required | Status | Where |
|---|---|---|
| Overview with live values | Implemented | `apps/desktop/src/screens/Overview.tsx` |
| Devices page with device cards and capabilities | Implemented | `Devices.tsx`, `DeviceDetail.tsx` |
| Automation page (form-based) | Implemented | `Automation.tsx` (now incl. the condition editor) |
| Settings: start with Windows, minimize to tray, polling interval, logging level, experimental features, developer mode | Implemented | `Settings.tsx`; each switch was checked end-to-end in round 1 (see "Settings reality check" below) |
| Not an RGB/gamer aesthetic | Implemented | `src/styles/theme.css` — one accent colour, no glows |

### Settings reality check

Every switch was traced from declaration to effect; two were dead and are now
fixed:

| Setting | Declared | Saved | Read by logic | Effect verified by |
|---|---|---|---|---|
| `close_to_tray` | ✓ | ✓ | ✓ | `apps/desktop/src-tauri/src/lib.rs` — `should_hide_on_close` + test |
| `minimize_to_tray` | ✓ | ✓ | ✓ *(was dead)* | `should_hide_on_minimize` + `window_policy_follows_the_settings` |
| `start_with_windows` | ✓ | ✓ | ✓ | `commands::update_settings` → `autostart::set_enabled`; failure flips the setting back and reports why |
| `polling_interval_ms` / `discovery_interval_ms` | ✓ | ✓ | ✓ | read on every loop iteration |
| `dry_run` | ✓ | ✓ | ✓ | `--dry-run` folds into it at startup (regression test), audited as `simulated` |
| `adapter_settings.mock.enable_gpu_fan_control` | ✓ | ✓ | ✓ *(was a no-op)* | `tests/tests/edge_cases.rs` — `disabling_gpu_fan_control_keeps_monitoring_but_removes_control` |
| `theme`, `log_level`, `history_points`, `automation_enabled`, `experimental_features`, `enable_mock_protocol_device`, `developer_mode`, `start_minimized` | ✓ | ✓ | ✓ | each is read where it applies |

## 三 Adapter / plugin mechanism

| Required | Status | Notes |
|---|---|---|
| Every vendor path goes through an adapter | Implemented | `HardwareAdapter` trait; five adapters; the UI cannot bypass it |
| `discover` / `read` / `write` / `subscribe`·`poll` abstraction | Implemented | `probe`, `discover`, `read_state`, `read_all`, `write`, `shutdown`. Push transports surface events through the runtime bus |
| Third-party plugin loading at runtime | Deferred by design | v0.3, `docs/roadmap.md` |

## 四 Open Device Protocol

| Required | Status | Where |
|---|---|---|
| Reserved protocol module with interface + schema + mock device | Implemented | `crates/ohm-protocol` (45 tests) |
| `GET_DEVICE_INFO`, `GET_CAPABILITIES`, `GET_STATE`, `SET_STATE`, `SUBSCRIBE_EVENT`, `GET_FIRMWARE_INFO`, `ENTER_BOOTLOADER`, `UPDATE_FIRMWARE` | Implemented | `messages.rs`; plus `PING` and `UNSUBSCRIBE_EVENT` |
| USB HID / CDC / serial transports | Partial | `LoopbackTransport` (real framing, used by tests) and `StreamTransport` (any byte stream). **Real USB enumeration is v0.4** |
| Plug in → discover → register → UI appears | Implemented against the simulated device | `adapters/open-protocol`; `tests/tests/protocol_flow.rs` |

## 五 Mock hardware

| Required | Status |
|---|---|
| Mock CPU / GPU / fan / temperature sensor | Implemented (`adapters/mock`, 31 tests) |
| Temperature varies over time | Implemented (first-order thermal model; fans affect it) |
| `Mock GPU Temp → Automation → Mock Fan Speed` runs | Implemented and demonstrated (`ohm-cli demo`, `tests/tests/acceptance.rs::scenario_c`) |
| Usable for dev / tests / CI / demo | Implemented (deterministic clock, fault injection) |

## 六 Monorepo structure

Implemented, with one documented deviation: core crates live in `crates/`, the
adapter crates in `adapters/`, and `crates/adapters` is the facade that decides
which providers run. `docs/architecture.md` explains the split.

## 七 Technical principles

| Principle | Status | Evidence |
|---|---|---|
| Do not reinvent wheels; check licence/activity/Windows support | Implemented | `docs/research.md` (per-claim citations), README licence matrix |
| Software-first (no custom hardware) | Implemented | simulated machine + protocol spec |
| Local-first (no account, no cloud, no telemetry) | Implemented | `ConfigPaths`; nothing in the tree makes a network call except localhost LHM |
| Open source (Apache-2.0) | Implemented | `LICENSE`; rationale in ADR 0002; enforceable via `deny.toml` |
| Do not over-engineer | Implemented | no cloud/account/marketplace/AI chat/node editor; roadmap only |

## 八 AI as Phase 2

Deferred by design, and *reserved*: `crates/ohm-automation/src/lib.rs` documents
the `AI → structured rule → validation → engine → runtime` path, and
`docs/automation.md` states that a generated rule must pass the same
`check_rule` validation. No AI code exists.

## 九 Safety

| Required | Status | Where |
|---|---|---|
| Control-authority judgement | Implemented | `AdapterInfo::requires_admin`, `Transport::needs_elevation_to_write`, permission errors mapped to `permission_denied` |
| Write failure → fallback | Implemented | `Runtime::write_with_origin` (fail-safe duty) **and** the rule's `on_write_failure` |
| Sensor missing → fallback | Implemented | `evaluate_fallback`, plus the configurable grace period |
| Emergency temperature threshold | Implemented | `supervise_safety`, independent of rules |
| Minimum safe fan speed | Implemented | `SafetyPolicy::duty_floor` |
| Pump never driven to 0 | Implemented | unconditional pump floor + the device's own declared range (two layers) |
| Every write logged | Implemented | `audit.jsonl`, including refused writes |
| A write **result** must be distinguishable from a write **request** | Implemented (round 2) | `WriteStatus::{Applied, Unconfirmed, Simulated, Rejected}`. The audit trail records which one happened; only a confirmed write is counted as an applied value, and an unconfirmed one is logged at warn level and excluded from "changed hardware" |
| An unconfirmed write retries, then follows the write-failure policy | Implemented (round 2) | 3 consecutive unconfirmed attempts ⇒ `on_write_failure` (fail-safe duty by default). The audit reason keeps the original cause *and* names the fail-safe action, so the fail-safe is never reported as if it were the original failure |
| A rule that changes target leaves no fan unattended | Implemented (round 2) | `perform_pending_handovers`: the abandoned output is driven to the fail-safe duty at the next tick and the handover is audited |

## 十 Testing

| Required | Status | Evidence |
|---|---|---|
| Unit / integration / mock-hardware / automation tests | Implemented | **413 tests, 0 failed** at `096e13b`, across 15 crates, 8 integration test binaries and doc-tests |
| Temperature→fan mapping, hysteresis, sensor disconnect, actuator failure, invalid value, device hotplug | Implemented | `tests/tests/{acceptance,edge_cases,rule_lifecycle,protocol_flow}.rs` |
| A write result that was never confirmed, a `release` fallback, and rule retargeting | Implemented (round 2) | `tests/tests/{write_confirmation,fallback_release,rule_edit_state}.rs` — 14 tests covering the three defects |
| Passes without special hardware | Implemented | every test uses the simulated provider |

## 十一 Acceptance scenarios

| Scenario | Status | Evidence |
|---|---|---|
| A — start and see CPU/GPU/SSD automatically | Partial | CPU verified on real hardware; GPU/SSD need Windows + a provider. Covered by `scenario_a_the_machine_is_detected_automatically` |
| B — devices and their capabilities | Implemented | `scenario_b_capabilities_are_complete_and_honest` |
| C — Mock GPU temp → rule → fan RPM, dynamically | Implemented | `scenario_c_mock_gpu_temperature_drives_the_fan`, `ohm-cli demo` |
| D — real fan control works, or refuses honestly | **Unverified on hardware** | The path is implemented and tested against a fake LHM server, and since round 2 a write is only called `Applied` when the channel reads back the value it was given; an unreadable channel yields `Unconfirmed`, which is retried and then falls back. Neither of those is hardware evidence: no real SuperIO machine has run it, and no fan has been measured responding. Kit: `docs/windows-validation/` |

## 十二 Development process (Step 1–8)

Implemented: research (`docs/research.md`), skeleton, mock runtime first, UI,
real sensors, real fan read/write attempt, tests, documentation.

---

## Still open

Ordered by what blocks a defensible Windows MVP:

1. **Windows real-hardware validation** (scenario D, NVML, LHM, WMI storage,
   tray, autostart, NSIS install/uninstall). Everything is *Prepared*;
   `docs/windows-validation/` is the kit. Nothing has run on Windows yet.
2. **A physical fan has never responded to a write from this code.** Scenario D
   is unverified in two independent ways — no Windows machine has run it, and no
   fan's **audible or tachometer response** to a commanded duty has ever been
   measured. The read-back added in round 2 confirms only what the *provider*
   reports as the channel's set point; it is not evidence that air moved. Both
   need separate evidence: one Windows run, and one measurement with a real
   tachometer.
3. **CPU package power has no native path** — Windows relies on LHM. Either
   accept that (documented) or add a real collector later; the brief lists the
   reading as "if the system supports it".
4. **Cross-adapter device identity.** One physical GPU can be described by both
   NVML and LHM; today NVML steps aside when LHM is enabled
   (`crates/adapters/src/lib.rs`). The general fix (a stable `physical_id` plus
   adapter priority) is planned in ADR 0005, not implemented.
5. ~~Frontend behaviour tests for the condition editor, tray switches and
   conflict messaging.~~ **Done in round 1**: 4 files / 22 tests in
   `apps/desktop/src/test`, with a mutation check showing they fail when the
   behaviour they describe is broken. Round 2 added a fifth file covering the
   unconfirmed-write states.
6. **Unconsumed interface surface.** `WriteOrigin::{Api, Startup}`,
   `Response::Event`, `CapabilityKind::Event` and `Capability::poll_interval_ms`
   (the *per-capability* hint) are declared and never read.
   `AdapterCapabilities::poll_interval_ms` is now honoured: an adapter that asks
   for a slower cadence is skipped until its own interval elapses, without
   affecting other adapters. `AdapterCapabilities::discovery_interval_ms` is
   **not** honoured — the discovery loop uses the user's
   `settings.discovery_interval_ms` (`crates/ohm-runtime/src/runtime.rs`), so an
   adapter asking for slower re-enumeration is currently ignored. The remainder
   stay reserved with a documented reason rather than being deleted, because they
   are part of the adapter and protocol contracts.
7. **Deferred by design**: plugin loading, scenes/profiles, app and game
   detection, natural-language rules, OpenHub/OpenFan hardware, release signing.
