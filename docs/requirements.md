# Requirements traceability

This document maps **every clause of the original OpenHardwareOS brief** to what
exists in this repository, and states honestly what does not. It exists because
two earlier statements in the docs contradicted each other — the roadmap called
v0.1 "shipped" while a review found requirements from the brief's *minimum* list
still missing. The rule from here on:

> A clause is **Implemented** only when the code path exists, is reachable from
> the app or the CLI, and is covered by a test that would fail if it broke.
> Anything else is **Partial**, **Missing**, or **Unverified on hardware**.

Last reviewed: 2026-09-14 (round 6, preparing **v0.1.2**). Test baseline at that
revision: see the round-6 entry of `docs/verification-log.md`, which records the
command, the environment and the result — that file is the authority for any
number quoted here, and numbers in this file are not carried over from an earlier
round. Earlier rounds are kept in the same file as history, not as current status.

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
| Every way a rule can stop driving a channel must hand it over | **Implemented** (round 3) | `RuleChange` + `RuleState::control` + the handover book (`crates/ohm-automation/src/{engine,handover,evaluator}.rs`) | A channel is `(device, capability)` — *device and capability* — so "same fan, different control channel" is a retarget like any other. Retarget, disable, delete, and a rule file that vanishes from disk and is reloaded all end control the same way: the channel is queued for the fail-safe duty, the rule's cache for it is dropped, and the new channel is driven from its own evidence on the first cycle. A source, condition or mapping change is **not** an exit: the same rule still owns the same channel, and a test pins that down |
| A declared owner must not be mistaken for a takeover | **Implemented** (round 4) | `AutomationEngine::{claimant_of,confirmed_owner_of}` + `HandoverState::{AwaitingOwner,NeedsVerification}` | A *claim* (an enabled rule targets the channel) stops the engine writing the fail-safe duty under a rule that owns the channel; only a *takeover* — that rule has driven the channel and the device confirmed a value — resolves the handover. A claimed-but-undriven channel stays owed and visible, naming the claimant and its last status, waits `HANDOVER_OWNER_WAIT_TICKS`, then parks as `Failed` rather than waiting for ever. An owner whose writes are unconfirmed or refused has not taken over either |
| An unfinished handover and an unverified write must survive a restart | **Implemented** (round 4) | `crates/ohm-automation/src/recovery.rs` → `<config>/control-state.json`; `AutomationEngine::{persist_control_state,recover_control_state,persistence_error,retry_failed_handovers}` | Versioned, atomic (temp file + rename), storing only unresolved handovers and issued-but-unconfirmed writes — never confirmed values, curve positions or gate state. Everything recovered comes back as `NeedsVerification`: the device, its capability and the current owner are checked against the hardware as it is *now* before the safety policy applies, and nothing is replayed. A channel that no longer exists fails visibly instead of being dropped; a write that cannot be recorded is reported through `AutomationStats.persistence_error`, `ohm-cli handovers` and the Diagnostics panel rather than presented as saved |
| A handover may only be queued by a rule that really controlled the channel | **Implemented** (round 3) | `AutomationEngine::hand_over` + `owner_of` | The rule's own record of what it drove decides, not what its config used to name, and only the channel it actually held is handed over. A channel that an enabled rule targets again is *superseded* — resolved on the record, and **not written** — so a stale handover can never overwrite the live owner's value (the defect where hardware sat at 70 % while the owning rule displayed 40 %) |
| An unfinished handover must be visible, bounded and recoverable | **Implemented** (round 3) | `crates/ohm-automation/src/handover.rs`; `AutomationEngine::{handovers,unfinished_handovers,retry_failed_handovers}`; `ohm-cli handovers [--retry]`; the Diagnostics panel | Retried every `HANDOVER_RETRY_TICKS` (5 ticks, so nothing spins) up to `MAX_HANDOVER_ATTEMPTS` (5), then **parked as `Failed`**: still on the record, still in the queue, with the first error (the cause) and the last one, and re-armable by an explicit user action. A `Confirmed` state requires a confirmed write; a `Superseded` one requires a live owner. The record names the rule as *data*, so deleting the rule does not hide the channel it left unprotected |
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
| Every write status shown honestly, including "requested, not confirmed" | **Implemented** (round 3) | `src/lib/writeStatus.ts` (tone, label, sentence, applied-value text) consumed by `DeviceDetail.tsx` and `Diagnostics.tsx`. `unconfirmed` is warn, never success; a report with no `applied` value renders `value unknown — not confirmed` rather than the requested value, "nothing" or "not applied"; manual control raises its own notice for an unconfirmed result, distinct from a refusal — one is *known to have failed*, the other is *unknown* — and both carry the backend's reason. The maps are exhaustive over `WriteStatus`, so a new status breaks the build until it is handled (checked by compiling) |
| Rule files adjusted in memory are visible, and never silently rewritten | **Implemented** (round 3) | `RuleFileNote` (`field`, `original`, `effective`, `message`, `hint`) → `rule_compatibility_notes` → the Automation screen's compatibility panel. The user sees which rule, which field, what the file on disk says, what is in force instead and how to fix it, and is told the file was **not** modified and the fail-safe duty still protects the machine |
| The self-test's isolation is enforced by the application, not by its launcher | **Implemented** (round 5) | `ProbeIsolation` in `apps/desktop/src-tauri/src/lib.rs`, checked before the config directory is created and before any provider, runtime or engine exists. `--ipc-selftest` alone used to arm the probe against the real per-user config directory, `--mock` never disabled the real providers, the report command wrote into the user's config directory when unarmed, and the probe re-armed failed handovers globally. Now an explicit, existing, dedicated config directory is required; settings that enable `system`, `lhm` or `nvidia` are refused by name; a simulated provider must be enabled; the run constructs simulated providers only; the report refuses unless armed; and a probe may only re-arm the channel it named. The legitimate simulator controls are unchanged for normal users |
| The desktop is proven to be connected to the backend | **Implemented** (round 4) | `scripts/verify-ipc-roundtrip.sh` + `apps/desktop/src/lib/ipcProbe.ts` — the **real** application (window, WebKit webview, the bundle it ships) is launched against an isolated config directory and the simulator, asked over Tauri's event channel to exercise the command surface, and the frontend's own account of what it saw is compared with the app's audit trail. It covers compatibility notes, a handover created and driven `pending → failed → confirmed` through retry, and an unconfirmed write: the check that the Rust wire test, the mocked frontend test and the headless selftest each cannot be |
| Channels left behind by a rule are visible, with a way to act | **Implemented** (round 3) | `rule_handovers` / `rule_retry_handovers` → the Diagnostics screen's handover panel. Owed handovers (pending, failed) list channel, the rule that left them, the reason, attempts and the cause; `failed` is visually distinct and says retrying stopped and needs the user; a retry control re-arms and refreshes; resolved ones are filed as history, never as owed work |

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
| A rule that changes target leaves no fan unattended | Implemented (round 3, extended) | `perform_pending_handovers`: the abandoned channel is driven to the fail-safe duty at the next tick, and retried on a bounded cadence if that write does not land. Ownership is re-checked before every attempt, so a channel another rule has taken over is never written |
| A responsibility that outlives the process is re-decided, never replayed | Implemented (round 4) | Recovery verifies the device, the capability, the current owner and the freshness of the data before applying the safety policy. No stored value is treated as confirmed, no curve position is restored, and no command from the previous session is re-sent |
| A handover that cannot be completed must not be forgotten | Implemented (round 3) | The handover book keeps it, `ohm-cli handovers` and the Diagnostics panel show it, `retry_failed_handovers` / `ohm-cli handovers --retry` re-arm it. The audit trail records every attempt with its outcome |
| Leaving must release **every** channel the runtime can drive, and say what is actually known | Implemented (round 7) | `Runtime::release_control` writes the fail-safe duty to `DeviceTable::releasable_devices()` — any device with a writable duty control, not just `Fan \| Pump`, which is how a **GPU fan kept its last duty on exit**. The returned `ControlRelease` splits confirmed / unconfirmed / refused / failed / simulated, and separates *the adapter handed control back* from *it never does*: `AdapterCapabilities::hands_back_control_on_shutdown` exists because the trait's `shutdown` defaults to `Ok(())`, so an adapter that does nothing used to be counted as a hand-back. `tests/tests/control_release.rs` (7 tests) pins each claim, including the GPU-fan regression |

## 十 Testing

| Required | Status | Evidence |
|---|---|---|
| Unit / integration / mock-hardware / automation tests | Implemented | **444 tests, 0 failed** at the round-3 revision, across 15 crates, 10 integration test binaries and doc-tests — the round-3 entry of `docs/verification-log.md` is authoritative |
| Temperature→fan mapping, hysteresis, sensor disconnect, actuator failure, invalid value, device hotplug | Implemented | `tests/tests/{acceptance,edge_cases,rule_lifecycle,protocol_flow}.rs` |
| A write result that was never confirmed, a `release` fallback, and rule retargeting | Implemented (round 2) | `tests/tests/{write_confirmation,fallback_release,rule_edit_state}.rs` — 14 tests covering the three defects |
| Control handover integrity, asserted on the hardware calls | Implemented (round 3) | `tests/tests/handover_integrity.rs` (12 tests) and `tests/tests/handover_state.rs` (9 tests), on a shared fake rig whose three channels can apply, accept-without-confirming, or refuse. They assert the values asked of each channel, in order, the number of attempts, what the fake hardware is left at, and the queryable record — not log strings |
| The self-test's refusal paths, and packaging that cannot overwrite evidence | Implemented (round 5) | `apps/desktop/src-tauri/src/lib.rs` (7 refusal-path tests), `scripts/verify-ipc-roundtrip.sh` (end-to-end refusals plus a seeded failed handover for a *real-looking* channel that must survive the run untouched), `scripts/tests/make-acceptance-package.test.sh` (4 cases / 20 checks against throwaway fixture repositories) |
| A claimed-but-undriven channel, a recovered responsibility, and the real IPC path | Implemented (round 4) | `tests/tests/handover_owner.rs` (9), `tests/tests/control_recovery.rs` (9), `apps/cli/tests/handover_recovery.rs` (4, running the real CLI binary), `scripts/verify-ipc-roundtrip.sh` (12 command steps through the real app) |
| The desktop's write-status and record surfaces, rendered | Implemented (round 3) | `apps/desktop/src/test/writeStates.test.tsx` — 17 tests rendering the real screens through the real providers with only the IPC module mocked. Checked by mutation: reverting the unconfirmed tone to `ok` fails two tests, reverting the failed-handover tone to `warn` fails one |
| Passes without special hardware | Implemented | every test uses the simulated provider |

## 十一 Acceptance scenarios

| Scenario | Status | Evidence |
|---|---|---|
| A — start and see CPU/GPU/SSD automatically | Partial | CPU verified on real hardware; GPU/SSD need Windows + a provider. Covered by `scenario_a_the_machine_is_detected_automatically` |
| B — devices and their capabilities | Implemented | `scenario_b_capabilities_are_complete_and_honest` |
| C — Mock GPU temp → rule → fan RPM, dynamically | Implemented | `scenario_c_mock_gpu_temperature_drives_the_fan`, `ohm-cli demo` |
| D — real fan control works, or refuses honestly | **Unverified on hardware** | The path is implemented and tested against a fake LHM server, and since round 2 a write is only called `Applied` when the channel reads back the value it was given; an unreadable channel yields `Unconfirmed`, which is retried and then falls back. Neither of those is hardware evidence: no real SuperIO machine has run it, and no fan has been measured responding. Kit: `docs/windows-validation/` |

## 十一.bis Deliverable: the Windows acceptance package

The brief requires a reproducible deliverable, and there is no Windows build to ship.
`scripts/make-acceptance-package.sh` produces a **source** acceptance package instead:
the tracked tree at exactly one commit, the entry points an operator needs, an evidence
index, and a SHA-256 manifest of every file. It refuses to package a dirty tree, states
in its own README that it contains no Windows artefacts, and verifies its own output
(manifest match; no VCS data, build output, dependencies, binaries or local runtime
state). The shipped pre-check refuses a non-Windows host, and the package's build
script stops at the first failure rather than continuing past it.

## 十二 Development process (Step 1–8)

Implemented: research (`docs/research.md`), skeleton, mock runtime first, UI,
real sensors, real fan read/write attempt, tests, documentation.

---

## Still open

Ordered by what blocks a defensible Windows MVP:

1. **Windows real-hardware validation** (scenario D, NVML, LHM, WMI storage,
   tray, autostart, NSIS install/uninstall). Everything is *Prepared*;
   `docs/windows-validation/` is the kit. **Software on Windows has run since
   v0.1.0**: GitHub Actions runs the workspace tests, Clippy, the NSIS bundle and a
   public PowerShell 5.1 download-install-check on `windows-latest` (v0.1.1:
   490 Rust tests, all eight CI jobs green — see the release evidence in
   `docs/versions.json`). What has *not* run is this code on a Windows machine
   somebody actually uses, with real fans to read and drive. A CI runner is not
   that machine, and its "install passed" is not hardware acceptance.
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
7. ~~The handover record does not survive a restart.~~ **Closed in round 4**:
   unresolved handovers and issued-but-unconfirmed writes are written to
   `<config>/control-state.json` atomically, and come back as `NeedsVerification` to be
   checked against the machine before the safety policy applies. What is still *not*
   stored: an ordinary rule's confirmed values, curve anchors and gate state — restoring
   those would drive hardware from stale decisions, so the next evaluation recomputes
   them from live readings instead.
8. ~~A handover is superseded when an enabled rule merely declares the channel.~~
   **Closed in round 4**: a declaration is a claim, not a takeover, and a claimed channel
   whose rule never drives it stays owed. What replaces it as a limit is narrower: the
   wait is bounded (`HANDOVER_OWNER_WAIT_TICKS`) and ends in a visible `Failed`, so a
   claimant that is merely slow — its first evaluation still pending when the wait runs
   out — parks a handover that would have resolved itself. Re-arming clears it.
9. **Visual acceptance of the desktop is unverified.** The screen-capture route was
   attempted in round 5 and abandoned: a whole-screen capture showed unrelated private
   content and missed the application window, so the file was deleted and the approach is
   opt-in only. Until a person looks at the window and keeps a picture, the interface rests
   on the frontend's own account of what it rendered — which is not a substitute.
10. **Two external dependencies, and nothing else blocks a release.** Everything
    that still needs *hardware evidence* needs either a Windows machine someone may use,
    or a person with a tachometer next to a real fan; no further work on this repository
    moves either one. That is a limit on **hardware acceptance**, not a claim that the
    repository is finished: since the v0.1.1 publication it has gained the version
    catalogue and tree, versioned six-stage planning
    (`docs/plans/hardware-support/`), the Windows CI defects below, and the release
    packaging fixes in v0.1.2. Statement by statement, what each gate still needs is in
    `docs/windows-validation/ACCEPTANCE-ENTRY.md`.
11. ~~The Windows code path had never been compiled in this repository.~~
    **Closed in round 6**: `adapters/system/src/windows.rs` sits behind
    `#[cfg(windows)]`, so no macOS or Linux build ever read it, and the first
    GitHub CI run (on round 5's commit) failed to compile it against `wmi` 0.18 —
    `wmi::COMLibrary` no longer exists and `WMIConnection::new()` takes no
    argument. A round-2 test also asserted that a read-only *directory* blocks a
    write, which is true on Unix and false on Windows. Both were fixed (in
    `34db43b` and `6e37954`) before v0.1.1, and round 6 added the missing local
    gate: `cargo check --target x86_64-pc-windows-msvc` now runs as part of the
    full pass, so a Windows-only file can no longer rot unseen between releases.
12. **NVML hands nothing back on exit.** `adapters/nvidia` declares
    `hands_back_control_on_shutdown: false` and has no `shutdown` implementation, so a
    GPU fan this adapter drove keeps its last duty when the app exits — the runtime's
    fail-safe write is the only release. The exit report now says this instead of
    counting a no-op `Ok(())` as a hand-back, and the audit carries a
    `control_not_handed_back` entry. Fixing it means calling NVML's default-fan-speed
    entry point in `shutdown`, and doing that honestly needs a fake-NVML harness the
    adapter does not have yet.
13. **Deferred by design**: plugin loading, scenes/profiles, app and game
   detection, natural-language rules, OpenHub/OpenFan hardware, release signing.
