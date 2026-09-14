# Roadmap

This is the intended path from the current MVP to a hardware operating system.
It is written to be checkable: every version says what already exists in this
repository and what does not. Technical facts with sources are in
[`research.md`](research.md); architectural decisions are in
[`decisions/`](decisions/).

Legend: **shipped** = in this repository and covered by tests · **partial** =
present but limited, or dependent on hardware most machines do not have ·
**roadmap** = planned, no implementation.

---

## v0.1 — Hardware Monitor MVP

**Why.** Nothing else works without it. Before automating anything, the project
needs one model that describes a CPU, a GPU, an SSD and a fan the same way, one
runtime that reads them safely, and one UI that shows the truth — including the
things that cannot be read.

**State: shipped.**

What exists today:

| Deliverable | State | Where |
|---|---|---|
| Unified device / capability / state model | shipped | `crates/ohm-device-model` |
| Hardware runtime: probe, discover, hotplug, poll, state store, event bus | shipped | `crates/ohm-runtime` |
| Safety policy: duty floors, fail-safe, emergency ceiling, audit log | shipped | `crates/ohm-runtime/src/{safety,audit}.rs` |
| Simulated machine with fault injection (CPU/GPU/SSD/fans/pump) | shipped | `adapters/mock` |
| OS sensors: CPU name/load/clock, thermal zones, Windows storage counters | shipped | `adapters/system` |
| NVIDIA telemetry and fan control through the driver's NVML | shipped | `adapters/nvidia` |
| Motherboard sensors **and chassis fan control** through LibreHardwareMonitor's web server | shipped | `adapters/libre-hardware-monitor` |
| Desktop UI: Overview, Devices, Device detail, Automation, Settings, Diagnostics, simulator | shipped | `apps/desktop` |
| Headless CLI: `doctor`, `status`, `watch`, `demo`, `rules`, `audit`, `paths`, `protocol` | shipped | `apps/cli` |
| Cross-crate acceptance and edge-case tests | shipped | `tests/` |
| Deterministic `--selftest` startup check | shipped | `apps/desktop/src-tauri/src/lib.rs` |

**Dependencies and risks.**

* **Elevation.** Motherboard fan I/O requires an elevated process
  (`research.md` §2.3). The app therefore inherits LHM's elevation requirement
  rather than solving it: the LHM GUI runs `requireAdministrator`, and
  OpenHardwareOS talks to its HTTP server. The elevated *helper* architecture
  (a small privileged process behind IPC) is not built — see v0.3.
* **The kernel driver is PawnIO, not WinRing0.** LibreHardwareMonitor swapped
  WinRing0 for PawnIO in September 2025 because Microsoft classifies WinRing0 as a
  vulnerable driver (`CVE-2020-14979`, `VulnerableDriver:WinNT/Winring0`) and the
  recommended blocklist is on by default on Windows 11 (`research.md` §1.5). Any
  local copy of WinRing0 is a dead end; PawnIO is a GPL-2.0-or-later product with
  its own installer, so v1 detects and deep-links to pawnio.eu rather than
  bundling it.
* **A .NET sidecar for LibreHardwareMonitor.** The HTTP-server integration was
  chosen because it needs no .NET toolchain. The better architecture — a small
  `net10.0` sidecar hosting the unmodified `LibreHardwareMonitorLib` over a named
  pipe, keeping MPL-2.0 out-of-process and removing the GUI dependency — is
  planned, and is what `decisions/0003-libre-hardware-monitor-integration.md`
  records (`research.md` §1.2, §6.2). Pin `net10.0`, not `net8.0`: .NET 8 leaves
  support on 2026-11-11.
* **Honest unavailability.** Several readings genuinely cannot be produced:
  laptop fan RPM and laptop fan control, live CPU frequency, SSD temperature on
  macOS. These surface as `unavailable` with a reason, never as zero
  (`research.md` §2.4).

---

## v0.2 — Cooling Automation

**Why.** This is the reason people install the app. A monitoring dashboard is
nice; a machine that keeps itself quiet and cool is the product.

**State: the software half is shipped; the hardware half depends on the machine.**

| Deliverable | State | Notes |
|---|---|---|
| Rule model, YAML storage, atomic writes, load reports | shipped | `crates/ohm-automation` |
| Piecewise-linear curves with interpolation and end clamping | shipped | `curve.rs` |
| Hysteresis and deadband | shipped | `evaluator.rs` |
| Fallbacks: `hold`, `safe_default`, `fixed`, `release` | shipped | `rule.rs`, `evaluator.rs` |
| Combined sources — `MAX` / `MIN` / `AVG` over several sensors | shipped | `rule.rs` — `Source::Combined` |
| GPU → fan and CPU + GPU → fan example rules | shipped | `examples.rs` |
| Validation before save, identical for hand-written and generated rules | shipped | `engine.rs` — `check_rule` |
| Safety floor, fail-safe duty, emergency override, ramp limiting | shipped | `runtime/src/safety.rs` |
| Fan **detection** (RPM) on a desktop board | partial | requires LHM to find a SuperIO chip |
| Fan **control** (PWM write) | partial | only where LHM exposes a writable `Control` sensor; opt-in by design |
| GPU fan duty control (NVIDIA) | partial | NVML `nvmlDeviceSetFanSpeed_v2`, documented for Maxwell+, **requires elevation**; some SKUs refuse third-party control |
| GPU fan duty control (AMD) | partial | via LHM's Overdrive `Control` sensor only — see the licence note below |
| Laptop fan control | roadmap | not claimed, and should not be: the EC is firmware-owned and undocumented |
| Fan curve **table** in the GPU firmware | roadmap | NVML exposes duty, not a curve; the curve is our own poll-and-set loop (`research.md` §3.3) |

**Dependencies and risks.**

* **AMD is a licensing problem, not an engineering one.** ADL and ADLX EULAs
  forbid subjecting SDK source to a licence requiring source disclosure or the
  right to modify (ADL §5(e), ADLX §4(f)), and Apache-2.0 is exactly such a
  licence. No AMD SDK header, binding or transcribed `-sys` crate may enter this
  repository; AMD support is inherited from the MPL-2.0 LHM host
  (`research.md` §4.3, `decisions/0004-vendor-sdks-and-amd.md`).
* **NVIDIA's library must not be redistributed.** `nvml-wrapper` loads the bare
  name `nvml.dll` from the installed driver with `libloading`, so nothing NVIDIA
  is shipped. The Driver Licence Agreement §2.7/§2.9 and the CUDA EULA forbid
  bundling it (`research.md` §3.4).
* **A wrong PWM write is destructive-adjacent.** On an unsupported board, writing
  the wrong SuperIO register is the classic "fans stop" or "spins to 100 %" bug
  (`research.md` §2.2). This is why control is opt-in, why the safety floor is
  unconditional for pumps, and why `relinquish_on_exit` hands control back to the
  firmware on shutdown.
* **The AI layer is deliberately absent.** Natural-language automation is on the
  roadmap, but its shape is already fixed: propose YAML, validate with
  `check_rule`, then run through the engine. No direct hardware access, ever
  (`crates/ohm-automation/src/lib.rs`).

---

## v0.3 — Plugin SDK

**Why.** One team cannot support every motherboard, AIO, lighting controller and
laptop vendor. The adapter trait already exists; the SDK turns it into a
publishable contract and the runtime into a host that loads third-party code.

**State: the trait is shipped, the SDK and loading are roadmap.**

| Deliverable | State | Notes |
|---|---|---|
| `HardwareAdapter` trait (`probe`, `discover`, `read_state`, `read_all`, `write`, `shutdown`, `as_any`) | shipped | `crates/ohm-adapter-api` |
| Adapter facade, registration order, provider enable/disable in Settings | shipped | `crates/adapters`, `Settings.tsx` |
| A versioned, documented SDK crate with a stability promise | roadmap | today the trait can still change freely |
| Loading third-party adapter plugins at runtime | roadmap | the adapter list is compiled in: `build_adapters(&AdapterOptions)` |
| Third-party plugin discovery, permissions, versioning | roadmap | none |
| **OpenRGB adapter** | roadmap — reference only | OpenRGB is GPL-2.0: it must not be linked, vendored or copied into this Apache-2.0 workspace (`research.md` §7.1). Its SMBus/PawnIO documentation is citable; its code is not. |
| More vendors (ASUS/MSI/Gigabyte lighting, AIO pumps, laptop ECs) | roadmap | each depends on the SDK, and on the licence situation for that vendor |
| Elevated helper process for write paths | roadmap | `research.md` §9.5 recommends a Task Scheduler task with `RunLevel=HighestAvailable` for v1 and a Windows service later; neither exists here |

**Dependencies and risks.**

* **An ABI in Rust is not an ABI.** Loading third-party Rust plugins across crate
  versions is fragile; the realistic options are a C ABI, a WASM sandbox, or an
  out-of-process plugin speaking a local protocol. The last one matches the
  runtime's existing shape and is the cheapest to make safe.
* **A plugin can stop a fan.** The safety policy is enforced by the runtime before
  an adapter is called, so a plugin cannot bypass it — but a plugin that lies in
  `discover()` about what it controls still has to be contained. Third-party
  adapters need the same review bar as in-tree ones, plus a way to disable them
  when they misbehave.
* **Licence hygiene.** The SDK has to state clearly that plugins are separate
  works, and that GPL/AGPL code cannot be shipped inside the workspace.

---

## v0.4 — OpenHub reference hardware

**Why.** Software that can only ever consume hardware somebody else made cannot
fix the parts of the problem the industry has left broken: fan headers that
revert, ECs that lie, no standard way to ask a device what it is.

**State: the protocol is shipped, the hardware is roadmap.**
`crates/ohm-protocol` is the executable specification: framing (`0xAA55`, u16
length, CRC-8, JSON payload), message set, descriptors, plus `StreamTransport`
and `LoopbackTransport`. The Open Device Protocol table in
`crates/ohm-protocol/src/lib.rs` marks real USB HID/CDC enumeration as
**roadmap (v0.4)**.

**Deliverables (planned).**

* A USB hub board with: fan PWM headers, tachometer inputs, temperature sensor
  inputs, and a host link over USB HID or USB CDC.
* Firmware implementing `GET_DEVICE_INFO` → `GET_CAPABILITIES` → `GET_STATE` →
  `SET_STATE` → `SUBSCRIBE_EVENT`, with the device's own local fallback curve on
  lost host contact (`DEFAULT_FALLBACK_AFTER_MS`, `DEFAULT_FALLBACK_DUTY`).
* A reference enclosure, a BOM and a flashing procedure.

**Dependencies and risks.**

* **The protocol must not need revision to ship.** A device that self-describes
  lets the runtime register it with no code change; the proof is already in the
  repository, where a simulated OpenFan registers like any other device
  (`tests/tests/protocol_flow.rs` — `an_opd_device_is_registered_like_any_other`).
* **USB HID vs CDC.** HID needs no driver and works from a browser and from user
  mode; CDC gives a byte stream and is easier to debug. `DeviceTransport` is byte
  oriented so both fit; the choice is a hardware decision that has to be made once,
  before the first board.
* **Firmware safety is not optional.** The device must refuse out-of-range writes
  and must run its own fallback, because the host can disappear — a crashed
  process, a sleeping machine, a pulled cable. The mock device already implements
  this and `protocol_flow.rs` — `a_device_that_loses_its_host_keeps_itself_cool`
  asserts it.
* **Certification and cost.** USB VID/PID allocation, EMC, and connector/current
  ratings on fan headers are real money and real lead time.

---

## v0.5 — OpenFan

**Why.** The smallest useful piece of hardware: one fan, one MCU, full
self-identification and firmware updates over the same wire. It is the platform's
"hello world" and the cheapest way to prove the protocol on real silicon.

**State: specified and simulated; no hardware.**
`crates/ohm-protocol/src/mock.rs` — `MockOpenFan` already implements a 4-channel
device with capabilities, readings, a local fallback curve, bootloader entry and a
firmware-update flow. `ohm-cli protocol` prints a full exchange against it.

**Deliverables (planned).**

* An MCU fan controller (PWM output, tachometer input, temperature input).
* Self-identification: descriptor with vendor, product, device type, firmware
  version, and a capability list that the runtime turns into UI with no code
  change.
* Firmware update over the protocol (`GET_FIRMWARE_INFO` → `ENTER_BOOTLOADER` →
  `UPDATE_FIRMWARE`), with recovery from an interrupted flash.
* Capabilities advertised honestly per board revision.

**Dependencies and risks.**

* **Bricking on a failed update** is the main risk; a bootloader that always
  enumerates and never requires a valid application image is the mitigation.
* **Fan electrical design.** PWM pull-ups, tach input protection and per-channel
  current limits differ between fan types; a shared design has to pick a
  conservative default.
* The runtime side is already done: `adapters/open-protocol` registers ODP devices
  like any other adapter, and `protocol_flow.rs` —
  `rules_can_target_a_protocol_device` shows an automation rule driving one.

---

## v0.6 — Hardware Automation Platform

**Why.** `temperature -> fan` is one sentence of a much larger language. The
next step is events, conditions and grouping — and a natural-language front end
that compiles to the same validated rule format.

**State: roadmap. Nothing below exists in this repository.**

| Deliverable | Notes |
|---|---|
| `WHEN` conditions — load above X, app running, time window, power state | needs a condition block in `Rule` and a process/window observer |
| Full `IF`/`AND`/`OR` composition | `Source::Combined` (max/min/avg) is arithmetic, not logic |
| Scenes and profiles — groups of rules enabled together | needs grouping and a switcher in the UI |
| Application and game detection | needs a process observer; gaming profiles are the top request |
| Event-driven evaluation | the runtime event bus exists; rules are still polled |
| **Natural Language Automation** | sentence → structured rule (YAML) → `check_rule` → engine → `Runtime::write_value`. The AI never receives a runtime handle or a write path of its own. |

**Dependencies and risks.**

* **Conflict resolution must land before conditions do.** Today `priority` only
  orders the writes within a tick, so the *last* writer — the lowest-priority
  rule — wins a contested target (see `docs/automation.md`, "One honest gap").
  More rules and more conditions make that worse, not better.
* **Per-rule sensor timeouts are done; richer fallbacks are not.**
  `fallback.sensor_timeout_s` is enforced as a per-rule grace period
  (`evaluator.rs` — `stale_value_within_grace`) on top of the global staleness
  check. Retry budgets and escalating fallbacks are still roadmap, and should
  land before conditions multiply the ways a source can go quiet.
* **Event-driven rules need hysteresis on events too.** A process watcher that
  flaps between "game" and "not game" would otherwise flap the fan, which is
  exactly what the curve hysteresis exists to prevent.
* **AI output is untrusted input.** A generated rule goes through the same
  `check_rule` as a hand-written one, is stored as reviewable YAML, and is
  installed disabled. No prompt or model endpoint exists in this repository yet.

---

## v0.7 — Hardware OS SDK

**Why.** If the platform is worth anything, other people should be able to build
on every layer of it: firmware, hardware, plugins and the protocol itself.

**State: roadmap.**

* **Firmware SDK** — a reference stack for MCU vendors so a new device implements
  the protocol without re-reading it: descriptor tables, capability declaration,
  local fallback, bootloader.
* **Hardware SDK** — reference schematics, a device template, and the electrical
  and mechanical constraints a device must respect to be recognised.
* **Plugin SDK, stabilised** — the v0.3 contract frozen with a compatibility
  policy, a conformance test suite, and a registry of known adapters.
* **Protocol certification** — a documented test sequence and a badge for devices
  that pass it, including a conformance tool built on the existing
  `LoopbackTransport` harness.

**Dependencies and risks.** Certification only means something if the protocol is
stable and version negotiation is honest; `ohm-protocol` already carries
`PROTOCOL_MAJOR`/`PROTOCOL_MINOR` and `is_compatible`. The commercial risk is
support load: a certified device list implies a compatibility promise, and that
needs a governance process before it needs code.

---

## v1.0 — Open Hardware OS

**Why.** Cooling is the wedge, not the destination. Once the runtime can describe
and drive hardware safely, the same model covers everything on and around a desk.

**State: roadmap.**

| Domain | Notes |
|---|---|
| Cooling | the current feature; fans, pumps, curves, safety |
| Lighting | needs a plugin/SDK story and a licence-clean path per vendor (OpenRGB stays reference-only, GPL-2.0) |
| Display | case displays and small panels as ODP devices with their own capabilities |
| Input | knobs, macro pads, media decks — ODP devices again |
| Sensors | temperature, flow, current, humidity; the device model already has the kinds |
| Power | PSU telemetry and power limits, where the vendor permits it |
| AI hardware | local inference accelerators and NPU telemetry; a monitoring surface first |
| Desk hardware | monitor arms, sit-stand controllers, the same protocol over the same transport |

**Dependencies and risks.**

* **Elevation, settled.** By v1.0 the elevated helper must exist in a form
  Microsoft supports going forward: a Windows service or a scheduled task with
  `RunLevel=HighestAvailable`, plus config in a location both the user and the
  elevated context can read (`%PROGRAMDATA%`), because Windows 11's
  Administrator protection separates the elevated profile (`research.md` §9.5).
* **Every new domain re-opens the licence question.** Lighting and power are the
  two worst offenders for closed SDKs; the AMD ADL/ADLX outcome is the template
  for how such a case is resolved — find an MPL-2.0 or permissive host, or ship
  nothing.
* **Scope is the real risk.** "Hardware OS" can mean anything. The disciplined
  version is: the same device/capability model, the same runtime, the same safety
  gate and the same audit log — new domains add capabilities, not new
  architectures.

---

## Current state vs roadmap

| Area | v0.1 | v0.2 | v0.3 | v0.4 | v0.5 | v0.6 | v0.7 | v1.0 |
|---|---|---|---|---|---|---|---|---|
| Device / capability model | shipped | | | | | | | |
| Runtime: discovery, polling, state, events | shipped | | | | | | | |
| Safety policy + audit log | shipped | | | | | | | |
| Simulated hardware + fault injection | shipped | | | | | | | |
| OS adapters (CPU, thermal, storage) | shipped | | | | | | | |
| NVML adapter (NVIDIA) | shipped | | | | | | | |
| LibreHardwareMonitor adapter | shipped | | | | | | | |
| Desktop UI + headless CLI | shipped | | | | | | | |
| Rule engine, curves, fallbacks | | shipped | | | | | | |
| Combined `MAX`/`MIN`/`AVG` sources | | shipped | | | | | | |
| GPU → fan / CPU+GPU → fan rules | | shipped | | | | | | |
| Controllable fans on *this* machine | | partial | | | | | | |
| Adapter trait published and stable | | | partial | | | | | |
| Third-party plugin loading | | | roadmap | | | | | |
| OpenRGB adapter | | | roadmap (reference only, GPL-2.0) | | | | | |
| Open Device Protocol | | | | shipped | | | | |
| Real USB HID/CDC enumeration | | | | roadmap | | | | |
| OpenHub hardware | | | | roadmap | | | | |
| OpenFan specification + simulation | | | | | shipped | | | |
| OpenFan hardware | | | | | roadmap | | | |
| WHEN/IF/THEN, scenes, profiles | | | | | | roadmap | | |
| App/game detection | | | | | | roadmap | | |
| Natural language automation | | | | | | roadmap | | |
| Firmware/hardware SDK, certification | | | | | | | roadmap | |
| Lighting, display, input, power, AI hardware | | | | | | | | roadmap |

---

## Known gaps of the MVP

Stated plainly, because they are the difference between a demo and a product.

**Autostart is wired, and deliberately per-user.** `update_settings` reacts to a
change of `start_with_windows` by calling `crate::autostart::set_enabled(...)`
(`apps/desktop/src-tauri/src/commands.rs`), which writes or deletes the per-user
`Run` value `HKCU\Software\Microsoft\Windows\CurrentVersion\Run\OpenHardwareOS`
through `reg` (`apps/desktop/src-tauri/src/autostart.rs`). The module documents
why it is the user hive: a `Run` entry can never start an elevated process. If
the registry write fails, the command reconciles the stored setting back to the
real state (`actual`) or to `false`, so the switch cannot silently lie. It is
Windows-only by design; macOS and Linux return an explicit "not supported"
error with a hint instead of failing silently.

**Tray and window behaviour are wired.** `tray::install` builds the menu (Show,
Hide, Rescan hardware, Open config folder, Quit), a left click restores the
window, and `close_to_tray` turns `CloseRequested` into `hide()`. `RunEvent::
ExitRequested` is intercepted so `engine.stop()` and `runtime.shutdown()` — and
therefore `relinquish_on_exit` — run before the process exits. This part is done.

**USB HID/CDC enumeration does not exist.** `crates/ohm-protocol` has framing,
messages, descriptors, `LoopbackTransport` and `StreamTransport`, and
`MockOpenFan` is a working simulated device, but no manifest depends on `hidapi`
or `serialport`, and there is no code that opens a real USB device. Every ODP
device today is simulated. The protocol crate's own status table says
`real USB HID / CDC enumeration | roadmap (v0.4)`.

**There is no plugin loading.** The adapter list is static:
`ohm_adapters::build_adapters(&AdapterOptions)` constructs a fixed set
(LibreHardwareMonitor, NVML, OS, ODP, mock) in `crates/adapters/src/lib.rs`.
There is no `libloading`, no dynamic library search, no manifest and no sandbox.
Third-party adapters require recompiling the workspace.

**No packaging beyond an unsigned NSIS bundle.** `tauri.conf.json` sets
`bundle.targets: ["nsis"]` with `installMode: "perMachine"`, and `bundle.active:
true`, so `cargo tauri build` produces an installer. There is **no code-signing
configuration** (no certificate, no timestamp URL) and no MSI target, so the
resulting installer is unsigned and will trigger SmartScreen. There is no
elevated helper, no Windows service, no scheduled task, and no uninstall hook that
restores firmware fan control — `relinquish_on_exit` covers the normal exit path
only. `research.md` §9.4/§9.5 records the intended approach.

**CI is a gate, as far as this machine can tell.** `.github/workflows/ci.yml`
builds and tests the workspace on Linux, macOS and Windows, and runs five jobs:
`lint` (required `cargo fmt --all -- --check` and
`cargo clippy --workspace --all-targets -- -D warnings`, with the WebKitGTK
packages the desktop crate needs), `rust` (build, test, the headless `ohm-cli
demo`, and `ohm-desktop --selftest --mock` normally and in `--dry-run`), `frontend`
(`npm ci`, typecheck, behaviour tests, production build), `windows-bundle` (`tauri
build`, then the NSIS artefact is listed and uploaded for review) and `hygiene`
(no crate may opt out of `unsafe_code = "deny"`, no ADL/ADLX reference may appear
in code, and `cargo deny check` is **required**, backed by a `deny.toml` that has
been run locally and passes). There is **no `continue-on-error` anywhere** in the
workflow: a check that cannot fail is not a check.

A **source acceptance package** for Windows is produced by
`scripts/make-acceptance-package.sh`: the tracked tree at one commit, the pre-check /
build / verify entry points, an evidence index and a SHA-256 manifest of every file,
with the platform's own README stating that it contains no Windows build artefacts
because none exist. What is still missing before a first release: no release workflow,
no code signing, and no generated third-party notices file — which `research.md`
(decision 13) recommends. Note also that a workflow definition is not a run:
nothing in this repository has been executed by GitHub Actions, so every
`windows-latest` job is **Prepared**, not verified. See
`docs/verification-log.md`.

**The example rules are files, not an importer.** `examples/rules/` holds four
ready-to-copy rules (`gpu-cooling.yaml`, `gpu-cooling-gaming-only.yaml`,
`cpu-cooling.yaml`, `system-cooling-max.yaml`) with a `README.md`, and
`apps/cli/tests/yaml_examples.rs` — `every_shipped_example_rule_parses_and_validates`
parses, validates and checks each of them on every test run. What does not exist
is an **import** path: the app has no "load a rule file" button and no
drag-and-drop, so a user must copy the file into `<config>/rules` by hand.

**Workspace test status.** `cargo test --workspace` passes: **444 tests, 0
failed**, exit code 0, measured at the round-3 revision with the pinned toolchain
`rustc 1.98.0` / `cargo 1.98.0` (see `docs/verification-log.md`, which records the
command, the environment and the result, and is the authority if this number ever
disagrees with a fresh run). The suite spans `ohm-core`, `ohm-device-model`,
`ohm-adapter-api`, `ohm-runtime`, `ohm-automation`, `ohm-protocol`, `ohm-adapters`,
all five adapter crates, `ohm-desktop`, `ohm-cli` and the ten cross-crate
integration test binaries (`acceptance`, `edge_cases`, `fallback_release`,
`gate_behaviour`, `handover_integrity`, `handover_state`, `protocol_flow`,
`rule_edit_state`, `rule_lifecycle`, `write_confirmation`).

**Clippy now runs over the whole workspace.** This was an environment gap, not a
project one: the machine's `PATH` toolchain mixed `clippy-driver 0.1.92` with
`rustc 1.98.0`, so clippy aborted with `rustc 1.92.0 is not supported by the
following packages: … requires rustc 1.95` before reading any project code. A
matched `1.98.0` toolchain was installed from the official channel and is
addressed explicitly (`rustup run 1.98.0 cargo clippy --workspace --all-targets
-- -D warnings`), leaving the owner's default toolchain, `PATH` and shell
configuration untouched; `--ignore-rust-version` is not used. It now exits 0 with
no warnings across all fifteen crates, including `ohm-adapter-system` and
`ohm-desktop`, which the mismatched pair could never see. That first clean run
found real lints — a collapsible `if` in `adapters/system`, an unused parameter in
the automation engine, a needless lifetime, a field-reassign-on-default, a useless
conversion, a collapsible `if` and an unused import in `apps/desktop`, and unused
imports, a single-element loop and boolean comparisons in the test files — which
were fixed rather than allowed.

**Resolved since this roadmap was written.** Two items that were listed here as
gaps are now implemented and tested: rules may be gated behind a numeric
`when {source, op, value, otherwise}` condition (a false condition makes the rule
stand down at the fail-safe duty — never an unbounded hold), and a target may be
owned by at most one enabled rule, enforced when creating, enabling, importing
and at tick time. Both are described with evidence in `docs/automation.md`, as is
the per-rule sensor grace period (`fallback.sensor_timeout_s`).

**Still open, and honestly listed in `docs/requirements.md`:** real Windows
hardware validation (nothing has run on Windows yet), CPU package power having no
native collector, cross-adapter device identity, and the remaining reserved but
unconsumed interface surface.
