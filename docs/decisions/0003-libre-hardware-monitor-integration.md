# 0003 — LibreHardwareMonitor integration

## Status

Accepted

## Date

2026-09-11

## Context

Motherboard fan tachometers and PWM channels live behind SuperIO/EC registers, and
on Windows there is **no in-box documented API** for either: `Win32_Fan` documents
`SetSpeed` as "Not implemented"
([Learn](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-fan)).
Every working implementation needs a signed kernel driver, and Microsoft's
recommended driver blocklist — on by default since Windows 11 2022 — flags WinRing0
(`VulnerableDriver:WinNT/Winring0`, CVE-2020-14979), which is why LHM migrated to
**PawnIO** in PR #1857. LHM already owns the problem: it enumerates SuperIO chips,
reads fan tachometers, and creates `SensorType.Control` channels bound to an
`IControl` whose `SetSoftware(value)` writes the PWM duty register. It is **MPL-2.0**.

## Decision

**Use LHM as the real-hardware provider for motherboard, fan and control sensors,
over its built-in HTTP JSON web server**, implemented in
`adapters/libre-hardware-monitor`. The contract, verified against
[`HttpServer.cs`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Utilities/HttpServer.cs)
(see [research.md §1.3](../research.md)); default port **8085**:

| Purpose | Request |
|---|---|
| Sensor tree | `GET /data.json` |
| Set duty | `GET /Sensor?action=Set&id=<SensorId>&value=<0..100>` → `Control.SetSoftware` |
| Release to firmware | `GET /Sensor?action=Set&id=<SensorId>&value=null` → `Control.SetDefault` |

`src/lhm.rs` parses `/data.json` (`LhmNode`, `LhmSensorKind`), deriving readings
from LHM's formatted `Value` string and returning *no reading* for its "no value"
spellings — a `NaN` sensor is `unavailable`, never a fake zero. (Research §1.3
prefers the numeric `RawValue` fields.) `src/mapping.rs` maps the tree to
`Device`/`Capability`, including writability; `src/web.rs` is the client and the
write path; `src/fake_server.rs` is an **in-process fake LHM server** (ephemeral
port, records writes, can refuse them) over `adapters/libre-hardware-monitor/tests/fixtures/data.json`, so HTTP,
parsing, mapping and writes are tested without LHM, Windows or a fan controller.

**Why not re-implement SuperIO access.** It needs a kernel driver, and a new
kernel-mode driver must be signed through the Dev Portal (EV certificate plus a
Hardware Dev Center account); cross-signed drivers load only with Secure Boot off.
That is multi-quarter work with no payoff over reusing LHM — and WinRing0, the
shortcut, is blocklisted.

**Why the web server rather than hosting the .NET library in-process.** MPL-2.0 is
*file-level* copyleft: consuming the **unmodified** NuGet package from a separate
process creates no obligation on our Apache-2.0 code, whereas linking LHM in drags
MPL-2.0 sources and a .NET runtime into our artefact. Process isolation also keeps a
driver fault from taking the UI down, and it makes `0002-license.md` one sentence.

**The limitation is surfaced, not hidden.** LHM ships its web server **disabled by
default** (`runWebServerMenuItem = false`) and its GUI runs elevated
(`requireAdministrator`), so a working setup needs the user to install LHM, run it
as Administrator and enable `Options → Remote Web Server → Run`. When the server is
unreachable, `LhmAdapter::probe` returns `AdapterStatus::unavailable` with the
`UnavailableReason` and a `detail()` naming that menu path and the expected port
(`src/web.rs`) — an app that hides why it has no data is worse than one that says so.

## Consequences

Positive: the only broadly working Windows path to motherboard fan RPM *and* fan
control is available without a driver of our own and without a vendor SDK (AMD GPU
telemetry and fan control arrive with it, `0004`), and the in-process fake server
makes the whole integration testable on macOS and Linux CI.

**Negative**

- **Fan control requires the user to install and run LibreHardwareMonitor**, reported
  as `unavailable` with instructions rather than hidden — which is why
  `ohm-cli doctor` shows a degraded provider instead of an error.
- The dependency is architectural: the web server lives in LHM's Windows Forms
  assembly, not the library, so cooling stops when the GUI is closed or its config
  beside the executable is reset — `LibreHardwareMonitor.config` can only be seeded
  *before* first launch.
- Re-reading the whole tree over HTTP per refresh is heavier than an IPC call (fine
  at 1 Hz, not the design we want at 10 Hz), and writes are best-effort and
  board-dependent: many boards expose no `Control` sensor, or one for only some
  headers, and firmware can reclaim a channel — `ControlRefused` becomes
  `UnavailableReason::VendorLimitation`.

## Planned upgrade (v0.2+)

Replace the HTTP dependency with a **named-pipe or stdio sidecar hosting the
unmodified `LibreHardwareMonitorLib`** (`bundle.externalBin`), removing the GUI
requirement and the checkbox step without changing the licence boundary.

- Target **`.NET 10`** (LTS to 2028-11-15); `.NET 8` retires 2026-11-11, so
  `lhm-service`'s .NET 8 prerequisite must not be copied blindly. Ship the MPL-2.0
  notice, and keep the sidecar in its own directory so a patch to an LHM file cannot
  contaminate the Apache-2.0 core.
- **PawnIO is detected and prompted for, never redistributed.** LHM embeds
  `PawnIO_setup.exe` (GPL-2.0-or-later) and runs it after a prompt; bundling it makes
  us a GPL distributor, so v1 points the user at [pawnio.eu](https://pawnio.eu/).
  Detection reads `HKLM\...\Uninstall\PawnIO` → `DisplayVersion` (LHM warns below
  2.0.0), and PawnIO's linking exception covers programs talking to it over the
  device IOCTL interface only — loaded modules must be GPL/LGPL-2.1 compatible
  ([PawnIO README](https://github.com/namazso/PawnIO)).
- `lhm-service` 0.2.0 / `lhm-client` 0.3.0 / `lhm-sys` 0.1.1 (MIT) is a ready-made
  version of this pattern (a service over `\\.\pipe\LHMLibreHardwareMonitorService`);
  evaluate it before writing our own.

## Alternatives considered

- **Re-implement SuperIO/EC access ourselves.** Rejected: a signed kernel driver is
  a multi-quarter project needing an EV certificate and Dev Portal signing.
- **Host `LibreHardwareMonitorLib` in-process via `netcorehost` 0.22.0 (MIT).**
  Rejected for v1: MPL-2.0 sources and a CLR inside our artefact, and an unhandled
  .NET exception taking the UI down. Kept as a possible later optimisation.
- **`smartctl` (GPL-2.0) for storage sensors, or bundling LHM's GUI.** Rejected: a
  GPL binary plus a brittle CLI parse when the WMI and NVMe IOCTL paths are
  documented, and the GUI zip pulls Aga.Controls (BSD-3-Clause) and embedded
  GPL/LGPL artefacts — we would become a driver-installer distributor.
