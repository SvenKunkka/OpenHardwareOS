# Capability roadmap

This document groups hardware capabilities by area. **C1–C8 are capability areas,
not software release numbers.** Actual releases, development versions and future
plans are recorded in the [version tree](versions.md); its only data source is
[versions.json](versions.json). Historical roadmap labels such as v0.2 and v0.4
were capability milestones and did not mean those software versions were released.

The next product release planned for hardware work is **v0.2.0: pump support**.
Its scope, existing foundations and acceptance criteria are in the
[pump support plan](plans/pump-support.md). The version tree separates that plan
from the published v0.1.1 and v0.1.0 previews.

**This page is a capability view, not the delivery order.** The order in which
hardware support is built — six stages and fifty substages, with the v0.2.0 pump
work as stage one — is the [six-stage hardware support plan](plans/hardware-support/README.md),
versioned here as a snapshot of GitHub issues #1–#7. Three sets of labels answer
three different questions and must not be read as one sequence: **C1–C8** (this
page) say what state each capability area is in; **stage 1–6 / S1.1–6.7** say in
what order the work is delivered; **PUMP-01–PUMP-04** say what the v0.2.0 release
must contain. Stage one's substages map onto the pump work packages rather than
replacing them (S1.1–S1.3 ↔ PUMP-01, S1.4–S1.5 ↔ PUMP-02, S1.6–S1.7 ↔ PUMP-03,
S1.8 ↔ PUMP-04).

Legend: **implemented** = source and software tests exist · **partial** = limited
or dependent on target hardware · **roadmap** = planned. Hardware compatibility
requires target-device evidence. Technical sources are in [research.md](research.md)
and architecture decisions in [decisions/](decisions/).

---

## C1 — Hardware Monitor MVP

**Why.** Nothing else works without it. Before automating anything, the project
needs one model that describes a CPU, a GPU, an SSD and a fan the same way, one
runtime that reads them safely, and one UI that shows the truth — including the
things that cannot be read.

**State: implemented.**

What exists today:

| Deliverable | State | Where |
|---|---|---|
| Unified device / capability / state model | implemented | `crates/ohm-device-model` |
| Hardware runtime: probe, discover, hotplug, poll, state store, event bus | implemented | `crates/ohm-runtime` |
| Safety policy: duty floors, fail-safe, emergency ceiling, audit log | implemented | `crates/ohm-runtime/src/{safety,audit}.rs` |
| Simulated machine with fault injection (CPU/GPU/SSD/fans/pump) | implemented | `adapters/mock` |
| OS sensors: CPU name/load/clock, thermal zones, Windows storage counters | implemented | `adapters/system` |
| NVIDIA telemetry and fan control through the driver's NVML | implemented | `adapters/nvidia` |
| Motherboard sensors **and chassis fan control** through LibreHardwareMonitor's web server | implemented | `adapters/libre-hardware-monitor` |
| Desktop UI: Overview, Devices, Device detail, Automation, Settings, Diagnostics, simulator | implemented | `apps/desktop` |
| Headless CLI: `doctor`, `status`, `watch`, `demo`, `rules`, `audit`, `paths`, `protocol` | implemented | `apps/cli` |
| Cross-crate acceptance and edge-case tests | implemented | `tests/` |
| Deterministic `--selftest` startup check | implemented | `apps/desktop/src-tauri/src/lib.rs` |

**Dependencies and risks.**

* **Elevation.** Motherboard fan I/O requires an elevated process
  (`research.md` §2.3). The app therefore inherits LHM's elevation requirement
  rather than solving it: the LHM GUI runs `requireAdministrator`, and
  OpenHardwareOS talks to its HTTP server. The elevated *helper* architecture
  (a small privileged process behind IPC) is not built — see C3.
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

## C2 — Cooling Automation

**Why.** This is the reason people install the app. A monitoring dashboard is
nice; a machine that keeps itself quiet and cool is the product.

**State: the software half is implemented; the hardware half depends on the machine.**

| Deliverable | State | Notes |
|---|---|---|
| Rule model, YAML storage, atomic writes, load reports | implemented | `crates/ohm-automation` |
| Piecewise-linear curves with interpolation and end clamping | implemented | `curve.rs` |
| Hysteresis and deadband | implemented | `evaluator.rs` |
| Fallbacks: `hold`, `safe_default`, `fixed`, `release` | implemented | `rule.rs`, `evaluator.rs` |
| Combined sources — `MAX` / `MIN` / `AVG` over several sensors | implemented | `rule.rs` — `Source::Combined` |
| GPU → fan and CPU + GPU → fan example rules | implemented | `examples.rs` |
| Validation before save, identical for hand-written and generated rules | implemented | `engine.rs` — `check_rule` |
| Safety floor, fail-safe duty, emergency override, ramp limiting | implemented | `runtime/src/safety.rs` |
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

## C3 — Plugin SDK

**Why.** One team cannot support every motherboard, AIO, lighting controller and
laptop vendor. The adapter trait already exists; the SDK turns it into a
publishable contract and the runtime into a host that loads third-party code.

**State: the trait is implemented, the SDK and loading are roadmap.**

| Deliverable | State | Notes |
|---|---|---|
| `HardwareAdapter` trait (`probe`, `discover`, `read_state`, `read_all`, `write`, `shutdown`, `as_any`) | implemented | `crates/ohm-adapter-api` |
| Adapter facade, registration order, provider enable/disable in Settings | implemented | `crates/adapters`, `Settings.tsx` |
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

## C4 — OpenHub reference hardware

**Why.** Software that can only ever consume hardware somebody else made cannot
fix the parts of the problem the industry has left broken: fan headers that
revert, ECs that lie, no standard way to ask a device what it is.

**State: the protocol is implemented, the hardware is roadmap.**
`crates/ohm-protocol` is the executable specification: framing (`0xAA55`, u16
length, CRC-8, JSON payload), message set, descriptors, plus `StreamTransport`
and `LoopbackTransport`. The Open Device Protocol table in
`crates/ohm-protocol/src/lib.rs` marks real USB HID/CDC enumeration as
**roadmap (C4)**.

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

## C5 — OpenFan

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

## C6 — Hardware Automation Platform

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

## C7 — Hardware OS SDK

**Why.** If the platform is worth anything, other people should be able to build
on every layer of it: firmware, hardware, plugins and the protocol itself.

**State: roadmap.**

* **Firmware SDK** — a reference stack for MCU vendors so a new device implements
  the protocol without re-reading it: descriptor tables, capability declaration,
  local fallback, bootloader.
* **Hardware SDK** — reference schematics, a device template, and the electrical
  and mechanical constraints a device must respect to be recognised.
* **Plugin SDK, stabilised** — the C3 contract frozen with a compatibility
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

## C8 — Open Hardware OS

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

* **Elevation, settled.** By C8 the elevated helper must exist in a form
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

| Area | C1 | C2 | C3 | C4 | C5 | C6 | C7 | C8 |
|---|---|---|---|---|---|---|---|---|
| Device / capability model | implemented | | | | | | | |
| Runtime: discovery, polling, state, events | implemented | | | | | | | |
| Safety policy + audit log | implemented | | | | | | | |
| Simulated hardware + fault injection | implemented | | | | | | | |
| OS adapters (CPU, thermal, storage) | implemented | | | | | | | |
| NVML adapter (NVIDIA) | implemented | | | | | | | |
| LibreHardwareMonitor adapter | implemented | | | | | | | |
| Desktop UI + headless CLI | implemented | | | | | | | |
| Rule engine, curves, fallbacks | | implemented | | | | | | |
| Combined `MAX`/`MIN`/`AVG` sources | | implemented | | | | | | |
| GPU → fan / CPU+GPU → fan rules | | implemented | | | | | | |
| Controllable fans on *this* machine | | partial | | | | | | |
| Adapter trait published and stable | | | partial | | | | | |
| Third-party plugin loading | | | roadmap | | | | | |
| OpenRGB adapter | | | roadmap (reference only, GPL-2.0) | | | | | |
| Open Device Protocol | | | | implemented | | | | |
| Real USB HID/CDC enumeration | | | | roadmap | | | | |
| OpenHub hardware | | | | roadmap | | | | |
| OpenFan specification + simulation | | | | | implemented | | | |
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
`real USB HID / CDC enumeration | roadmap (C4)`.

**There is no plugin loading.** The adapter list is static:
`ohm_adapters::build_adapters(&AdapterOptions)` constructs a fixed set
(LibreHardwareMonitor, NVML, OS, ODP, mock) in `crates/adapters/src/lib.rs`.
There is no `libloading`, no dynamic library search, no manifest and no sandbox.
Third-party adapters require recompiling the workspace.

**Windows distribution is published.** The [v0.1.1 preview](https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/v0.1.1)
contains a Windows x64 CLI ZIP, an unsigned NSIS desktop installer, an installation
script, checksums, source metadata and generated dependency notices. The desktop
uses per-machine installation. Code signing, an elevated helper and a Windows
service remain separate work. Normal runtime exit requests restoration of firmware
control; the result still requires target-device validation.

**CI has run on Windows, Linux and macOS.** The
[v0.1.1 source CI](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34825301892)
passed all eight jobs: formatting/Clippy, three Rust platforms, frontend,
Windows NSIS packaging, dependency/license checks and version lifecycle checks. The
[release workflow](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34825296140)
built the actual Windows CLI and installer, generated notices, and tested CLI
installation in isolation. The
[public Windows PowerShell 5.1 install check](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34826241020)
then downloaded that release, verified checksums and the source commit, installed
it and ran diagnostics and a simulated demo. These checks did not verify a physical
fan or pump on the user's PC.

Historical source acceptance ZIPs from scripts/make-acceptance-package.sh
contain source, build entry points and evidence manifests. They remain distinct
from the published binary packages. New source versions and their release evidence
are tracked in [versions.md](versions.md), without rewriting previous tags.

**The example rules are files, not an importer.** `examples/rules/` holds four
ready-to-copy rules (`gpu-cooling.yaml`, `gpu-cooling-gaming-only.yaml`,
`cpu-cooling.yaml`, `system-cooling-max.yaml`) with a `README.md`, and
`apps/cli/tests/yaml_examples.rs` — `every_shipped_example_rule_parses_and_validates`
parses, validates and checks each of them on every test run. What does not exist
is an **import** path: the app has no "load a rule file" button and no
drag-and-drop, so a user must copy the file into `<config>/rules` by hand.

**Verified release baseline.** The v0.1.1 Windows run recorded **490 Rust tests
passed, 0 failed, 0 ignored**, including one documentation test. Frontend testing
recorded **42 passed** on Ubuntu/jsdom; the version tools passed **30 tests**. Later changes require their own validation;
current results belong in the version tree. Earlier round-specific counts and
local toolchain investigations in [verification-log.md](verification-log.md) are
historical evidence, not the current release status.

**Clippy now runs over the whole workspace.** This was an environment gap, not a
project one: the machine's `PATH` toolchain mixed `clippy-driver 0.1.92` with
`rustc 1.98.0`, so clippy aborted with `rustc 1.92.0 is not supported by the
following packages: … requires rustc 1.95` before reading any project code. A
matched toolchain was installed from the official channel; from round 6 the
repository pins the exact release CI uses in `rust-toolchain.toml` (`1.98.1`), so
a checkout, a local shell and a workflow all resolve the same compiler, and
`--ignore-rust-version` is not used. It exits 0 with no warnings across all
fifteen crates, including `ohm-adapter-system` and `ohm-desktop`, which the
mismatched pair could never see. That first clean run found real lints — a
collapsible `if` in `adapters/system`, an unused parameter in
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
*hardware* validation — the code has been built, tested and installed by CI on
`windows-latest` since v0.1.0, but no fan or pump on a machine somebody uses has
ever responded to a write from it — CPU package power having no native collector,
cross-adapter device identity, and the remaining reserved but unconsumed interface
surface.
