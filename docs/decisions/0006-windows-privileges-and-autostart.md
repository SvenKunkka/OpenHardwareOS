# 0006 — Windows privileges and autostart

## Status

Accepted

## Date

2026-09-11

## Context

OpenHardwareOS touches hardware that is either freely readable or guarded by a
kernel driver:

- **No elevation needed:** OS telemetry (WMI, `sysinfo`), NVML reads, NVMe/WMI
  storage temperature, and talking to LibreHardwareMonitor's HTTP web server on
  `127.0.0.1:8085`. `LhmAdapter::info()` advertises `can_write: true` **and**
  `write_requires_admin: true`, with the comment that talking to the web server needs
  no elevation while *LHM itself* must be elevated for writes to reach the SuperIO.
- **Elevation needed:** only paths touching SuperIO/EC registers directly — inside
  LHM, not in our process, because every working implementation goes through a signed
  kernel driver (`0003-libre-hardware-monitor-integration.md`).

The app therefore ships **unprivileged**: `apps/desktop/src-tauri/build.rs` calls
plain `tauri_build::build()` with no custom manifest, so the executable runs
`asInvoker`. Tauri's docs suggest `requireAdministrator` as an example while warning
of a UAC prompt on every launch; we do not do that.

Autostart is a per-user registry value: `apps/desktop/src-tauri/src/autostart.rs`
writes `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` under the value name
`OpenHardwareOS` with `reg add` / `reg delete`, and reads it back with `reg query`. Its
module docs give the reason: *"a `Run` entry can never start an elevated process, so
pretending otherwise would produce a silently useless setting."*

## Decision

1. **The app runs unprivileged and never asks to be elevated at launch** — no
   `requireAdministrator`, no custom manifest, no UAC prompt on start.
   `autostart::is_elevated()` (a `net session` probe) only *reports* the state, via
   `AppInfo.elevated`.
2. **Autostart is a per-user `HKCU\...\Run` entry and stays one.** It cannot gain
   privileges, and neither the documentation nor the UI may imply that it can. Where a
   user genuinely needs elevation (driver-level sensor access), the app says so — the
   hints in `apps/desktop/src/screens/Devices.tsx` and `Overview.tsx`. That a `Run`
   entry launches with the user's normal token is universally observed but
   **unverified** against a verbatim Microsoft statement; the documented mechanism for
   elevated autostart is a Task Scheduler task with
   [`/rl HIGHEST`](https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create).
3. **Non-Windows autostart reports "Windows only" instead of failing silently.**
   `set_enabled(true)` returns `CommandError::unavailable(...)` ("starting with the
   operating system is implemented for Windows only") with a hint pointing at macOS
   Login Items and Linux autostart entries; `is_enabled()` returns `false`.
4. **NSIS with `installMode: "perMachine"`** (`tauri.conf.json`,
   `bundle.targets = ["nsis"]`): admin once, at install, for the helper/driver step,
   and a mixed layout would make the helper's path ambiguous. MSI stays secondary —
   Tauri's WiX template is always per-machine.

## Planned upgrade path

- **v0.2+ — a Task Scheduler task with `RunLevel=HighestAvailable`** for the hardware
  helper, registered by the elevated installer (`NSIS_HOOK_POSTINSTALL`): one UAC
  prompt at install, none afterwards. Not a hack — Tauri's own updater does exactly
  this (`enableElevatedUpdateTask` ships `update.xml` plus `install-task.ps1`, which
  self-elevates and registers the task with `<RunLevel>HighestAvailable</RunLevel>`).
  Weaknesses: an absolute exe path that must be recreated on update, a task a policy
  can disable, and no watchdog.
- **Later — a Windows service plus a named pipe**, installed by the same NSIS hook
  (`windows-service` 0.8.1 plus `tokio::net::windows::named_pipe`, ACLs restricted to
  the interactive user), only if fan control must survive logon/logoff. That helper has
  no UI, so the pipe is the whole contract.


**Windows 11 "Administrator protection"** (shipping since KB5120998, off by default)
shapes this: elevation becomes just-in-time, elevated sessions use a
**profile-separated** account, and Microsoft advises local SYSTEM or dedicated service
accounts for highest-privilege tasks, because *"settings data for applications don't
carry over across the regular (unelevated) and the elevated profiles"*
([Learn](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/administrator-protection/)).
So fan-curve configuration the *elevated* helper must read belongs in a machine-wide
location (`%PROGRAMDATA%\OpenHardwareOS\...`), not only in the per-user config root
(`ConfigPaths`, `%APPDATA%\OpenHardwareOS`), and the IPC design must tolerate an
absent helper, because it starts from its own scheduled task rather than the `Run` key.

## Consequences

**Positive**

- No convenience toggle can be used to gain privileges: `autostart` writes one
  user-hive value and nothing else, and the blast radius stays small — UI, webview and
  network stack unelevated, where Microsoft's guidance is pushing every app.
- Monitoring works fully without elevation: reading sensors, viewing history and
  authoring curves never show a UAC prompt, and autostart is honest on every platform —
  macOS and Linux get an explanation, not a toggle that appears to do nothing.

**Negative**

- **The autostart toggle is wired, but has never been exercised on Windows.**
  `commands::update_settings` calls `autostart::set_enabled` whenever
  `start_with_windows` changes, and the setting is corrected back if the write fails
  (the Run-key path is `cfg(windows)`, so on this development machine that branch is
  compiled out). "Honest reporting" is therefore implemented — a refusal rewrites the
  setting to `false` and logs why — but it is **Prepared, not Build/run passed**:
  see `docs/windows-validation/` step §11, which verifies the registry value with
  `reg query`.
- Per-user autostart plus a machine-wide helper is inherently two mechanisms, and the
  second does not exist yet: a user wanting fan control at logon must start LHM and
  this app themselves. Uninstall must also return fan control to the firmware (LHM's
  `SetDefault`) via the NSIS uninstall hooks.
- `installMode: "perMachine"` prompts for admin even for a monitoring-only user — a
  deliberate trade for an unambiguous helper path; and any future direct-hardware path
  depends on PawnIO, which we detect and prompt for but never bundle (`0003`).

## Alternatives considered

- **`requireAdministrator` on the main executable.** Rejected: a UAC prompt on every
  launch, WebView2 children elevated with it, an autostart `Run` entry that would still
  start unelevated (so autostart would prompt anyway), and a huge blast radius.
- **Register autostart under `HKLM` so it "runs elevated".** Rejected: HKLM needs
  admin to write, and a `Run` entry still launches with the user's token — the
  appearance of privilege and none of the substance.
- **Start the whole GUI at logon from a highest-privileges scheduled task.** Not
  rejected for the *helper* (that is the plan), rejected for the GUI: it would run the
  desktop app elevated and drag the webview into the elevated profile.
