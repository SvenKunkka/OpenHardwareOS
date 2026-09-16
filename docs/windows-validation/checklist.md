# Windows validation checklist

Ordered, copy-pasteable, and honest about what each step proves. Read
[`README.md`](README.md) for the three status levels and [`safety.md`](safety.md)
before any step that can write to a fan.

Conventions used below:

- **All commands run from the repository root** unless the step says otherwise
  (`cd apps/desktop` is called out explicitly).
- **Status** at the top of each step is the level of the *step as written here* —
  today, every one of them is **Prepared**.
- **Evidence** means: paste the command, its exit code and its output into the bundle
  folder, then reference the file name in
  [`result-template.md`](result-template.md). A step without captured evidence stays
  `Prepared`, whatever you saw on screen.
- `PS>` marks a PowerShell command; the rest are shell-agnostic (run them in PowerShell
  too — the flags are plain ASCII and contain no quoting hazards).
- To keep a validation session from touching your real settings, every CLI command
  accepts `--config-dir <path>` (equivalently the `OHM_CONFIG_DIR` environment
  variable, which is what CI uses). Prefix it when you want a clean slate:
  `cargo run -p ohm-cli -- --config-dir .\ohm-validation-cfg doctor`.

---

## 1. Prerequisites

| # | Requirement | How to check | Expected |
|---|---|---|---|
| 1.1 | Windows 10 or 11, x64 | `PS> [System.Environment]::OSVersion.Version` and `PS> $env:PROCESSOR_ARCHITECTURE` | Windows 10 1809+/11; `AMD64`. Record the exact build (`PS> (Get-CimInstance Win32_OperatingSystem).BuildNumber`) in the report — several steps below are policy-sensitive on Windows 11 builds. |
| 1.2 | Rust **1.95 or newer** | `rustc --version` and `cargo --version` | `rustc 1.95.0` or higher. The floor is `rust-version = "1.95"` in the root `Cargo.toml`, set by `sysinfo`; an older toolchain fails dependency resolution, not project code. |
| 1.3 | Node.js **20 or newer** | `node --version` | `v20.x`, `v22.x` or newer. Only the desktop frontend needs it. |
| 1.4 | MSVC build toolchain (`x86_64-pc-windows-msvc`) | `rustup show` | An installed `stable-x86_64-pc-windows-msvc` toolchain with the MSVC linker available. The repository README lists only Rust and Node; this kit adds the toolchain requirement explicitly because linking on Windows needs it. |
| 1.5 | LibreHardwareMonitor installed **and running as Administrator**, with its web server **on** | `PS> Get-Process LibreHardwareMonitor` then `PS> (Invoke-WebRequest http://127.0.0.1:8085/data.json -UseBasicParsing).StatusCode` | A process exists, and the request returns `200`. In the LHM GUI: `Options -> Remote Web Server -> Run` (it is **off by default**), port left at the default **8085**. Without elevation LHM cannot reach the SuperIO, so it will start but expose no motherboard fans. |
| 1.6 | Kernel driver: **PawnIO**, not WinRing0 | `PS> Get-ItemProperty 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO' -ErrorAction SilentlyContinue \| Select-Object DisplayVersion` | A version present (`2.x` or newer; LHM warns below `2.0.0`). **Record this**, because it is a licensing and support fact, not a detail: current LibreHardwareMonitor loads a **signed PawnIO** driver (WinRing0 was removed in LHM PR #1857), and WinRing0 is on Microsoft's recommended vulnerable-driver blocklist, enforced by default on Windows 11 since the 2022 update where memory integrity/HVCI, Smart App Control or S mode is active. Consequence for this project: we **detect and prompt**, we never bundle `PawnIO_setup.exe` (it is GPL-2.0-or-later, so bundling makes us a GPL distributor). See [`docs/research.md`](../research.md) §1.5 and `docs/decisions/0003`. |
| 1.7 | NVIDIA driver, for the NVML steps | `PS> (Get-CimInstance Win32_VideoController \| Select-Object Name, DriverVersion)` | An NVIDIA adapter with a driver version. `nvml-wrapper` loads the driver-installed `nvml.dll` at runtime (`C:\Windows\System32\nvml.dll` on a DCH install) and ships nothing; a machine without NVIDIA is a **normal** state, not a failure — record it and mark §10 "not applicable". |

**What a failure means here**

| Symptom | Meaning |
|---|---|
| `rustc` below 1.95 | Environment, not project. Install a newer toolchain; do not lower `rust-version`. |
| LHM running but no motherboard fans appear in §11 | Expected if LHM is not elevated, or if the board exposes no SuperIO fan channel. Not a project defect; record it as a hardware/LHM limitation. |
| `http://127.0.0.1:8085/data.json` returns `401` | LHM's optional Basic auth is enabled. Reconfigure LHM, or set `adapter_settings.lhm.username`/`password` (the client sends Basic auth when they are present). Record which you chose. |
| Connection refused | The web server is off, or on another port. Fix LHM before continuing; §11 cannot be attempted without it. |

**Status: Prepared.** Evidence: `environment.txt` from `collect.ps1` covers 1.1, 1.5, 1.6 and 1.7 mechanically; paste 1.2–1.4 by hand.

---

## 2. Environment capture, and whether the shell is elevated

```powershell
PS> Set-ExecutionPolicy -Scope Process -ExecutionPolicy Bypass
PS> .\docs\windows-validation\collect.ps1
```

Expected: the script prints what it collected and ends with the full path of a
timestamped bundle folder. Review it with `-DryRun` first if you want to see the plan
without writing anything:

```powershell
PS> .\docs\windows-validation\collect.ps1 -DryRun
```

Elevation, by hand (the script records this too):

```powershell
PS> ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)
```

Expected: `True` or `False`. It matters in exactly three places, and knowing which is the
point:

| Path | Needs an elevated shell? |
|---|---|
| Reading sensors, `doctor`, `status`, `watch`, talking to LHM's web server | **No.** LHM is the elevated process; the client is not (the adapter advertises `write_requires_admin: true` for the *LHM* side). |
| Writing a motherboard fan through LHM | **No** for the caller — but LHM itself must run elevated, or the SuperIO is unreachable and the write is refused. |
| Writing a GPU fan through NVML (`nvmlDeviceSetFanSpeed_v2`) | **Yes** — the process making the NVML call must be elevated, or NVML returns `NVML_ERROR_NO_PERMISSION`. Run §10 from an elevated PowerShell. |

**What a failure means:** `collect.ps1` exiting non-zero means the bundle is incomplete —
read the script's own message; it names the section that failed to collect. A missing
section is not a product failure. Never "fix" it by editing the script mid-session;
record it and report.

**Status: Prepared** (the script has not been executed on Windows by the authors; see
[`README.md`](README.md)).

---

## 3. `cargo test --workspace`

```bash
cargo test --workspace 2>&1 | Tee-Object -FilePath "$env:TEMP\ohm-test-windows.txt"
```

Expected shape — the important part is the **shape**, not a number:

```text
     Running unittests src/lib.rs (...\target\debug\deps\ohm_runtime-<hash>.exe)
test result: ok. <N> passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in <t>s
     Running tests\acceptance.rs (...\target\debug\deps\acceptance-<hash>.exe)
test result: ok. <M> passed; 0 failed; ...
... one such block per crate and per integration test binary ...
```

Acceptance criteria: **every** `test result:` line reads `ok`, and `0 failed` appears
everywhere. Record the exact number of `passed` tests each line printed **as printed by
the command** — do not compare against a figure quoted in a document. For reference,
`docs/roadmap.md` records the macOS aarch64 run as 368 tests; the Windows count is
expected to differ, because some tests are `cfg`-gated per platform in both directions
(see §4), and a different total is not by itself a failure.

The Windows-only tests this run must include:

| Crate | Test names to look for | Why they matter |
|---|---|---|
| `ohm-adapter-system` | `failure_reasons_map_to_the_runtime_vocabulary`, `temperature_matching_is_forgiving`, `variant_conversion_handles_the_common_types` | These three live inside `adapters/system/src/windows.rs`, which carries `#![cfg(windows)]` at the top of the file. If the file were not compiled, the tests would not exist and would not run. Their presence in the output is the cheapest proof that the Windows module really compiled. |
| `ohm-desktop` | `elevation_probe_never_panics`, `run_value_name_is_stable` | `apps/desktop/src-tauri/src/autostart.rs`. The third test in that file, `autostart_is_reported_as_windows_only`, is `#[cfg(not(windows))]` and **must be absent** from a Windows run — its absence is correct behaviour, not a lost test. |

**What a failure means**

| Symptom | Meaning |
|---|---|
| A `test result: FAILED` line | A real defect on Windows. Capture the failing test's full output; do not skip the step. |
| Compile error inside `windows.rs` or `autostart.rs` | The Windows-specific path does not build. This is exactly the gap this kit exists to find. Stop and report. |
| `variant_conversion_handles_the_common_types` fails | The `wmi::Variant` → `f64` mapping in `windows.rs` no longer matches the `wmi` crate version in `Cargo.lock`. Windows-only API drift; report with the failing assertions. |
| The `ohm-adapter-system` unit-test block is missing entirely | The Windows module was not compiled — check that you are on Windows and that `cargo test` was run for that crate. |
| `autostart_is_reported_as_windows_only` **passes** on Windows | Contradiction: it cannot compile there. Something is wrong with the toolchain target or the test filter. |

**Status: Prepared.**

---

## 4. Windows-specific code actually compiling — and actually running

```bash
cargo build --workspace --all-targets
```

Expected: exit code 0, ending in `Finished \`dev\` profile ...`. Then confirm the two
`cfg(windows)` modules were not merely skipped:

```bash
cargo test -p ohm-adapter-system -- --nocapture
cargo test -p ohm-desktop -- --nocapture
```

Expected: the three `windows.rs` tests and the two `autostart.rs` tests named in §3,
run and passing.

**Compiling is not running.** Two Windows paths need a live invocation, and each has its
own way to prove it was really exercised:

| Path | File | How to prove it ran | What "ran" looks like |
|---|---|---|---|
| WMI storage temperature (`MSFT_StorageReliabilityCounter.Temperature`) | `adapters/system/src/windows.rs`, called from `SystemAdapter::disk_temperature` | `cargo run -p ohm-cli -- --log-level debug doctor`, then look at each `Storage: ...` device line | Either a temperature in the line, **or** one of the four reason codes the WMI probe maps to. The reason codes are the evidence that the probe executed and answered: `[hardware_limitation]` = WMI answered but returned no usable rows (`ProbeFailure::NoData`), `[permission_denied]` = `AccessDenied`, `[read_error]` = `WMI unavailable` or a failed query. The human-readable detail ("the driver reports no reliability counters for this device", "WMI is unavailable: …", "query failed: …") is logged at debug level — that is why this step passes `--log-level debug`. |
| Per-user autostart `Run` value | `apps/desktop/src-tauri/src/autostart.rs` | §9 below | `reg query` showing the value, written by `reg add` from the running app. |

Note on §4's storage path: a machine whose drives do not implement the counters is a
**correct** `hardware_limitation`, not a failure. The step proves the probe ran, not that
every drive reports a temperature.

**What a failure means**

| Symptom | Meaning |
|---|---|
| Build fails only with `--all-targets` | A test-only or example-only target is broken on Windows (often a path or a `cfg`). Report the target name. |
| Storage devices show `[unsupported]` on Windows | The `cfg(windows)` branch was not compiled — you are either not on Windows or building for a non-Windows target triple. |
| No `Storage:` device lines at all | The OS adapter found no disks. Record it; it is a machine observation, not a defect in the WMI path. |

**Status: Prepared.**

---

## 5. CLI surface

Every command below exists in `apps/cli/src/main.rs` (clap definitions, `Command` enum).
Global flags available on all of them: `--config-dir <DIR>`, `--log-level <error|warn|info|debug|trace>`
(default `warn`). There is **no** CLI subcommand that writes to hardware: `demo` is
simulated-only by construction, and the remaining commands read, validate or report.
Deliberate writes to a real fan are performed in the desktop UI (§12).

### 5.1 `doctor`

```bash
cargo run -p ohm-cli -- doctor
```

Expected shape (the section headers and the adapter line layout are fixed by the
source; the values are not):

```text
──────────────────────────────────────────────────────────────────────────────
  OpenHardwareOS doctor
──────────────────────────────────────────────────────────────────────────────
config:   C:\Users\<you>\AppData\Roaming\OpenHardwareOS
version:  0.1.0
platform: windows x86_64

  Providers
  lhm      available    23  device(s)
  nvidia   unavailable  0   device(s) — <reason text>
  system   available    7   device(s)

  Devices
  <device name>              online     58.0 °C · load 12.0 %
  Storage: <disk>            degraded   [hardware_limitation] · free 123456789012 B

  Capabilities
  <N> readable sensor(s), <M> writable actuator(s)
  • <device id>/<capability id>

  Rules
  <rule id>  <status>  <message>

  Runtime
  poll cycles:      <n>
  discovery cycles: <n>
  writes:           <a> applied, <r> rejected
  safety actions:   <s>
```

Correct looks like:

- `platform:` reads `windows x86_64`;
- adapter states are drawn from `not_probed | available | degraded | unavailable | error`;
  `nvidia unavailable` with a reason is **correct** on a machine with no NVIDIA driver;
- `lhm unavailable` is **correct** while LHM is not running or its web server is off — the
  detail text tells you to enable `Options -> Remote Web Server -> Run` and names the
  base URL it tried (`http://127.0.0.1:8085` by default);
- if no writable actuator is found, the command prints a paragraph beginning
  "This machine exposes no controllable output." — that is expected on a machine with no
  reachable SuperIO channel, and `ohm-cli demo` is the documented way to exercise the
  loop anyway;
- exit code 0.

```bash
cargo run -p ohm-cli -- doctor --mock
```

Expected additionally: a line `note:     simulated providers are enabled and writes are dry-run`,
the `mock` and `opd` adapters in the Providers list, mock devices (`gpu.mock.0`,
`fan.mock.0`, …) in Devices, and at least one entry under Capabilities even on a machine
with no real hardware. `--mock` forces `dry_run = true` in this session, so nothing can
reach a fan. Exit code 0.

**What a failure means**

| Symptom | Meaning |
|---|---|
| Non-zero exit | Runtime construction or start failed (most often the config directory is not writable). The error text names the path. Report it — this is a real Windows failure mode. |
| `lhm degraded` ("reachable but exposes no sensors") | LHM is answering but returning an empty tree — nearly always LHM not running elevated. Check the PawnIO row in §1.6. |
| Adapters missing entirely from Providers | They were disabled in `settings.json` (`disabled_adapters`), or `--mock` was not passed for the simulated ones. Check before reporting a defect. |
| Writable actuator list empty on a board you know exposes a Control channel | LHM found no `Control` sensor; on NVIDIA that also happens when NVML steps aside because LHM is present (see §10). |

**Status: Prepared.**

### 5.2 `status`, `watch`

```bash
cargo run -p ohm-cli -- status
cargo run -p ohm-cli -- watch --count 3
```

`status` prints the header `Status` followed by one line per device:
`<name truncated to 34> <status> <readings joined by " · ">`. Missing readings render as
`[reason]`, never as zero; a device with no state renders `—`.

`watch --count 3` prints three blocks, then exits 0:

```text
  <rfc3339 timestamp>   poll #<n> · <w> writes · <s> safety actions
  <device lines, same format as status>
```

`--interval <seconds>` is clamped to 0.1–60; `--count 0` (the default) means "forever",
so use `Ctrl+C` if you omit it.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `watch --count 3` never stops | You passed `--count 0`. Stop it and rerun. |
| Timestamps identical across all three blocks | The interval collapsed to its 0.1 s floor; harmless, note the interval you used. |
| A device shows `offline` | Discovery found it but the last poll failed. Cross-check against `doctor` and the adapter detail. |

**Status: Prepared.**

### 5.3 `demo` — the closed loop, simulated only

```bash
cargo run -p ohm-cli -- demo
```

Expected shape:

```text
  OpenHardwareOS demo — simulated cooling loop
    profile         : Gaming
    ambient         : 25 °C
    simulated step  : 1.0 s × 90 steps
    installed rule  : demo-gpu-cooling (GPU temp -> chassis fan)
     sim s   cpu °C   gpu °C  fan duty   fan RPM        rule
         0      ...     78.0       20%       ...   nochange
  ...
  Result
    start GPU temperature : 78.0 °C
    hottest GPU           : <a value ≤ the start>
    final GPU temperature : <lower than the start>
    final fan duty        : <higher than the idle value> % (<rpm> RPM)
    rule                  : ... — <e> evaluations, <w> writes, <s> skipped, <f> fallbacks
    closed loop           : <x> sensor(s) drove <y> actuator(s)
    last audit entry      : ...
```

This command uses `AdapterOptions::simulated_only()` — no real provider is registered, so
no real fan can be touched — and it **fails loudly** (`the demo never wrote to the
simulated fan; the loop is broken`) if no write happened. Correct = exit 0, a descending
GPU temperature, and at least one write in the rule line.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `the demo never wrote to the simulated fan` | The automation loop is broken for this build. This is the project's own acceptance scenario C; report immediately with the full table. |
| Final GPU temperature not below the start | The curve, the safety floor or the simulated thermal model regressed. Capture the whole table and the audit tail. |
| Non-zero exit with a config error | Set `--config-dir .\ohm-validation-cfg` and retry; the default config directory is not writable. |

**Status: Prepared.**

### 5.4 `protocol`, `paths`

```bash
cargo run -p ohm-cli -- protocol
cargo run -p ohm-cli -- paths
```

`protocol` drives the in-process simulated OpenFan over the loopback transport and
prints, in order: `host -> GET_DEVICE_INFO` with the descriptor summary and a derived
device id, `host -> GET_CAPABILITIES` (one line per capability: id, kind, unit,
`writable`/`read-only`, and a `(min-max)` range when the device declares one),
`host -> GET_STATE` (one line per capability, `[reason]` for unavailable ones),
`host -> SET_STATE fan.speed_percent = 85` with the device's `Response` debug form, and
finally the note about the device-side fallback after 5 s without host contact.
Correct = exit 0 and a `SetState` response that is not an error. This proves the protocol
plumbing, **not** any hardware.

`paths` prints the header `Configuration layout` followed by five lines —
`root:`, `settings:`, `rules:`, `logs:`, `audit:` — and the "Local first" line. On Windows
`root:` must be `%APPDATA%\OpenHardwareOS` (or the `--config-dir`/`OHM_CONFIG_DIR` value
you passed). This is a cheap, decisive check that the platform config path is right.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `root:` is not under `%APPDATA%` | Either the environment override is set, or the platform config resolution is wrong on Windows. Print `$env:APPDATA` next to it in the report. |
| `protocol` reports an error response | The Open Device Protocol mock regressed. No hardware involvement; report the exchange verbatim. |

**Status: Prepared.**

### 5.5 `rules list | suggest | check`

```bash
cargo run -p ohm-cli -- rules list
cargo run -p ohm-cli -- rules suggest
cargo run -p ohm-cli -- rules check .\my-rule.yaml
```

- `rules list` — header `Automation rules`; either `none installed yet`, or one two-line
  block per rule: `  <rule id padded to 22> <status lowercased> <source> -> <target>`
  followed by an indented message.
- `rules suggest` — header `Suggested rules`; either `nothing to suggest: hardware is
  missing or the rules already exist`, or `• <name> (<id>)` plus a summary per rule,
  each one **saved** to the rules directory; the command ends by printing where:
  `rules are stored in <config>\rules`.
- `rules check <path>` — parses a YAML rule and validates it against the hardware the
  runtime can currently see. Prints `rule: <name> (<id>)`, then
  `valid against the attached hardware` when there are no errors, then `error:` lines and
  `warning:` lines. It exits non-zero when any error is present.

**Wait — the shipped example rules target the simulated machine.** `examples/rules/*.yaml`
reference ids such as `gpu.mock.0` and `fan.mock.1`. Two honest options on real hardware:

1. Write a small rule of your own against the ids printed by `doctor`
   (device id + `fan.speed_percent`), and check that file. This is the better test: it
   validates the LHM bus you actually intend to drive.
2. Check a shipped example with simulated providers visible:
   `cargo run -p ohm-cli -- rules check .\examples\rules\gpu-cooling.yaml` passes only
   when the mock adapter is registered, which for `rules check` means setting
   `"experimental_features": true` in `<config>\settings.json` (the `--mock` flag exists
   on `doctor`, `status`, `watch` and `rules suggest` — **not** on `rules check`). Use a
   `--config-dir` scratch directory so you do not disturb your real settings.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `rules check` exits non-zero on an example rule, with `source … device not found` | Expected on a real machine: the example targets `*.mock.*`. Not a defect — follow option 1 or 2 above. |
| `rules suggest` suggests nothing even though `doctor` lists controllable fans | The suggestion set is derived from the runtime's capability index. Capture `doctor` and `rules list` together and report; this would be a real defect. |
| Errors about capability kinds/units | The rule references a capability that is not a readable temperature sensor, or a target that is not writable. The message names it. |

**Status: Prepared.**

### 5.6 `audit`

```bash
cargo run -p ohm-cli -- audit
cargo run -p ohm-cli -- audit --limit 50
```

Expected shape:

```text
  Audit log
  file: C:\Users\<you>\AppData\Roaming\OpenHardwareOS\audit.jsonl
  <rfc3339>  write      <device id> <capability> = <value> [<status>]
  <rfc3339>  lifecycle  <action> — <detail>
```

or `nothing recorded yet`. `--limit` defaults to 20 and takes the **last** N entries.
Every line is one JSON object in `audit.jsonl`, with `kind` = `write` (carrying a nested
`report`) or `lifecycle`, and `report.status` ∈ `applied | simulated | rejected`.
Correct = the file path is under your config root and entries appear in time order.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `nothing recorded yet` after §5.3 `demo` | The audit log is not where the CLI looks. Compare the printed path with `paths`. Real defect; report both. |
| A `rejected` entry with a `detail` you cannot explain | Exactly the kind of thing §13 exists for. Capture the line verbatim. |
| The file exists but the CLI prints nothing and exits non-zero | Parsing failure on an existing line — capture the whole file plus the error. |

**Status: Prepared.**

---

## 6. Desktop application

Build the frontend first — the Tauri context embeds `apps/desktop/dist`, so
`cargo run -p ohm-desktop` fails to compile without it:

```bash
cd apps/desktop
npm install
npm run build
cd ../..
```

### 6.1 Launch it

```bash
cargo run -p ohm-desktop
```

Expected: a window titled `OpenHardwareOS` (1360×860, minimum 1024×640, centred). It runs
**unelevated** — no UAC prompt. Confirm that: if a UAC dialog appears, something is wrong,
because `apps/desktop/src-tauri/build.rs` is a plain `tauri_build::build()` with no custom
manifest, i.e. `asInvoker`.

### 6.2 Headless self-test

```bash
cargo run -p ohm-desktop -- --selftest
cargo run -p ohm-desktop -- --selftest --mock --dry-run
```

Expected shape (no window is opened; the process exits 0):

```text
OpenHardwareOS 0.1.0 selftest
config:   C:\Users\<you>\AppData\Roaming\OpenHardwareOS
adapters: <N> registered, <M> usable
devices:  <D> discovered, <C> controllable
  - <adapter id> <state> <detail>
  rule <rule id> <status> <message>
```

With `--mock`, the simulated providers are registered and the mock devices appear.
With `--dry-run` the invocation must be read-only towards hardware, and here is the
**only** place that is provable, because the self-test report itself does not print the
dry-run flag:

- the startup log line records it — in
  `<config>\logs\openhardwareos.log.<YYYY-MM-DD>` you should find a line containing
  `starting OpenHardwareOS` with `dry_run=true` (it is an `info` line, so the setting
  `log_level` must be `info` or more verbose; the default is `info`);
- every write this run recorded in `<config>\audit.jsonl` must have
  `"status":"simulated"` and `"simulated":true`, and **no** entry from that run may read
  `"status":"applied"`.

Count the audit file's lines **before** the run and inspect only the appended lines, so
older entries from earlier sessions cannot be mistaken for this run's. Do not delete the
file.

If the run recorded **no** writes at all — a self-test only runs about 1.2 s, and the
automation engine may not have had a reason to write — then this step proves nothing about
dry-run, and you should say so in the report. A dry-run that definitely produces a write is
the UI pass in §12.2; do that one before touching hardware.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `cargo run -p ohm-desktop` fails with a missing `dist`/`index.html` error | `npm run build` was not run in `apps/desktop`. Not a project defect. |
| `adapters: 0 registered` | Adapter construction failed; the log file has the reason. Defect. |
| An audit entry from the `--dry-run` run reads `"applied"` | **The safety switch did not reach the write path.** This is the most serious outcome in this whole checklist: stop, do not run §12, and report. |
| `devices: 0 discovered, 0 controllable` with LHM healthy | Discovery failed on Windows. Capture `doctor` from §5.1 next to it. |

**Status: Prepared.**

---

## 7. NSIS packaging, install, uninstall

The Tauri CLI is a devDependency of `apps/desktop` (`@tauri-apps/cli`), so it is invoked
with `npx`; `package.json` has no `tauri` script.

### 7.1 Build

```bash
cd apps/desktop
npm install
npx tauri build
```

Expected: it runs `npm run build` (the `beforeBuildCommand`), then the release build, then
bundling; exit code 0, and the last lines of the output list the artefacts it produced —
**capture those lines verbatim**, they are the authoritative path.

Where to look for the artefacts if you want them independently: `apps/desktop/src-tauri` is a
member of the root Cargo workspace, so the bundle lands in the **workspace target directory**,
which is `<repo>\target` — there is no `apps\desktop\src-tauri\target`. Do not assume it: ask
cargo, then list what is there (this is the same source of truth
`.github/workflows/ci.yml` uses in its `windows-bundle` job):

```powershell
PS> $target = (cargo metadata --format-version 1 --no-deps | ConvertFrom-Json).target_directory
PS> $target
PS> if (Test-Path "$target\release\bundle") {
      Get-ChildItem "$target\release\bundle" -Recurse | Select-Object FullName, Length, LastWriteTime
    } else {
      Write-Warning "nothing at $target\release\bundle — the build did not bundle. Record that."
    }
```

Expected: `$target` ends in `\target` and is the repository's own `target` directory (record
the exact string), and the listing contains
`$target\release\bundle\nsis\OpenHardwareOS_0.1.0_x64-setup.exe` (the version tracks
`tauri.conf.json`). If the warning above fires instead, the build did not bundle — record
that, rather than looking for the file somewhere else. Do not read a missing `bundle`
directory as "the installer is elsewhere": there is no second target directory in this
workspace.

`bundle.targets` is `["nsis"]` in `tauri.conf.json`, so no MSI is produced — that is
deliberate (see `docs/research.md` §9.4).

### 7.2 Install — this needs administrator

`tauri.conf.json` sets `bundle.windows.nsis.installMode: "perMachine"`: the installer
writes under `Program Files` and registers its metadata machine-wide, so **installing
requires an administrator**. Expect one UAC prompt. The installer is **unsigned** (no
code-signing configuration exists), so SmartScreen may warn — record the exact warning
text rather than disabling protection.

After installing:

| Check | Expected |
|---|---|
| Install directory | Under `C:\Program Files\OpenHardwareOS` (perMachine). Record the exact path. |
| Launch from the Start Menu | The window opens, unelevated, no UAC prompt. |
| Config directory created | `%APPDATA%\OpenHardwareOS` exists after launch, containing `settings.json`, `logs\`, `rules\` and possibly `audit.jsonl`. `%APPDATA%` is per-user, so an elevated and an unelevated launch of the same user share it. |
| Tray icon | Present in the notification area (see §8). |

### 7.3 Config-directory policy after uninstall

Run the uninstaller (`Settings -> Apps -> OpenHardwareOS -> Uninstall`, or
`Uninstall OpenHardwareOS.exe` in the install directory) and then check what happened:

```powershell
PS> Test-Path "$env:APPDATA\OpenHardwareOS"
PS> Get-ChildItem "$env:APPDATA\OpenHardwareOS"
```

**What the docs actually claim, and what to verify.** There is no `NSIS_HOOK_*`
customisation in this repository (no `windows/hooks.nsh`, no uninstall hook of any kind),
so the uninstaller has no code of ours that removes configuration, returns fan control to
the firmware, or deletes a scheduled task. Concretely, the expectation is:

- the per-user config directory **survives** uninstall (an unsigned, hook-free NSIS
  uninstaller removes what it installed, not the user's data);
- **fan control does not need destroying at uninstall** for this build, because there is no
  elevated helper, no service and no scheduled task to remove. What *does* need to be true
  is that quitting the app hands the channels back — `relinquish_on_exit` calls LHM's
  `SetDefault` for every control channel (`shutdown()` in the LHM adapter). Verify that
  separately in §12.6, before uninstalling.
- `docs/roadmap.md` explicitly lists the missing uninstall hook that would restore firmware
  fan control as a known gap. If you observe the config directory being deleted, or a
  scheduled task or service being left behind, that contradicts the documentation —
  capture it as an unexplained observation.

**What a failure means**

| Symptom | Meaning |
|---|---|
| No `setup.exe` produced | Bundling failed. The output names the missing tool; on Windows this is usually the NSIS download or a missing WebView2 bootstrapper download. Capture the tail verbatim. |
| Installer refuses to run without elevation | Expected with `installMode: "perMachine"` — that is the setting, not a bug. |
| App installs but crashes on launch | Read `%APPDATA%\OpenHardwareOS\logs\openhardwareos.log.<date>`. That file is the single most useful artefact; put it in the bundle. |
| Uninstall removes the config directory | Contradicts the documentation above. Report as a documentation defect with the evidence. |

**Status: Prepared.**

---

## 8. Tray and window behaviour

Defaults come from `Settings::default()`: `close_to_tray: true`,
`minimize_to_tray: true`, `start_minimized: false`. `start_minimized` hides the window at
startup; the other two are the two switches you are testing.

| # | Action | Expected | What a failure means |
|---|---|---|---|
| 8.1 | With `close_to_tray` on, click the window's **X** | The window disappears, the tray icon remains, and the process is still running (confirm with `PS> Get-Process OpenHardwareOS`). The runtime is deliberately not shut down: `RunEvent::ExitRequested` is intercepted precisely so a close does not leave fans at a written value. | The process exits → `close_to_tray` did not take effect, and every written fan channel was released by `Runtime::shutdown` instead of being kept under control. Capture the log. |
| 8.2 | Left-click the tray icon | The window is shown, un-minimised and focused. | Nothing happens → the tray click handler failed; capture the log. |
| 8.3 | Tray menu → **Show OpenHardwareOS** / **Hide window** | Shows/hides the window. | As above. |
| 8.4 | Tray menu → **Rescan hardware** | A discovery cycle runs; the device list refreshes. With LHM running, start LHM *after* the app to make this observable, then rescan and look for the new LHM devices. | No refresh → `refresh_devices()` failed; the log has `tray rescan failed` with the error. |
| 8.5 | Tray menu → **Open config folder** | Explorer opens `%APPDATA%\OpenHardwareOS`. `explorer` returns a non-zero exit code even on success, so judge by the window opening, not by a return code. | Nothing opens → `open_path` failed; the log has `could not open the config folder`. |
| 8.6 | Tray menu → **Quit** | The app stops the automation engine, calls `runtime.shutdown()` (which releases every LHM control channel via `SetDefault`), and exits. | The process lingers → the shutdown path is blocked. Capture the log tail and the still-open channels. |
| 8.7 | With `minimize_to_tray` on, minimise the window | The window is hidden (not merely minimised, no taskbar entry). Tauri has no minimised event, so the check happens on resize. | The window stays in the taskbar → the resize/minimise check did not fire. |
| 8.8 | Turn `close_to_tray` **off** in Settings, then close the window | The app exits (with `relinquish_on_exit` releasing control first), instead of hiding. | It hides anyway → the setting is not reaching the window event handler. |

Record which of 8.1–8.8 you ran, with a screenshot for at least 8.1, 8.4, 8.5 and 8.7.

**Status: Prepared.**

---

## 9. Autostart (per-user `Run` value)

The setting is `start_with_windows`. `update_settings` reacts to a change of it by calling
`autostart::set_enabled(...)`, which runs `reg add` / `reg delete` against
`HKCU\Software\Microsoft\Windows\CurrentVersion\Run` under the value name `OpenHardwareOS`,
with the value data being the quoted absolute path of the running executable.

| # | Action | Expected |
|---|---|---|
| 9.1 | Note the setting's initial state; then enable **Start with Windows** in Settings | The toggle stays on (if the registry write fails, the command reconciles the stored setting back to the real state and logs a warning — the switch cannot silently lie). |
| 9.2 | `PS> reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v OpenHardwareOS` | Exit code 0 and a `REG_SZ` value whose data is the quoted path to `OpenHardwareOS.exe`. Paste the output verbatim. |
| 9.3 | Inspect the hive | It is **`HKCU`** — the user hive — and it must stay that way. Record that no `HKLM` value was created: `PS> reg query "HKLM\Software\Microsoft\Windows\CurrentVersion\Run" /v OpenHardwareOS` must report that the value is not found. |
| 9.4 | Disable the setting | The value is deleted; `reg query` returns non-zero ("unable to find the specified registry key or value"). Deleting a missing value is not an error, so a disabled toggle with no value present is correct. |
| 9.5 | Verify the privilege claim | The entry is per-user and **can never start an elevated process**: Explorer launches a `Run` entry with the user's normal token. This is universally observed but, as `docs/research.md` §9.3 and `docs/decisions/0006` record, **unverified** against a verbatim Microsoft statement — so verify it here rather than assuming it: sign out/in and confirm the app starts **unelevated** (no UAC prompt, and the elevation probe in Diagnostics reads not-elevated). Microsoft's documented mechanism for elevated autostart is a scheduled task with `/rl HIGHEST`; this project deliberately does not use one for the GUI. |
| 9.6 | Check the settings file | `<config>\settings.json` contains `"start_with_windows": true` while enabled, `false` after 9.4. This is the value the app reads back on the next launch. |

**What a failure means**

| Symptom | Meaning |
|---|---|
| The toggle flips back off immediately | The `reg add` failed, or the read-back disagreed. Check for a policy that blocks per-user `Run` entries (`reg query` will confirm), and capture the warning in the log. |
| A value appears under `HKLM` | Directly contradicts `autostart.rs` and `docs/decisions/0006`. Stop; report with the registry export. |
| The app starts **elevated** after sign-in | Contradicts the entire privilege design. Capture the elevation probe, the manifest state and the registry value. |
| `reg query` exits non-zero after enabling | The write did not land even though the UI may have shown success that was later corrected. Capture both the UI state and the query. |

> Note for reviewers: `docs/decisions/0006` still lists "the autostart toggle is not yet
> wired to the UI" under its *Negative* consequences, and `docs/roadmap.md` §"Known gaps"
> states the opposite ("Autostart is wired"). The code agrees with the roadmap:
> `commands.rs` calls `autostart::set_enabled` and `screens/Settings.tsx` renders the
> `start_with_windows` checkbox. This checklist follows the code; the stale decision note
> is recorded here so the discrepancy is not mistaken for a test failure.

**Status: Prepared.**

---

## 10. NVML (NVIDIA GPU)

### 10.1 Which adapter reports your GPU

NVML **steps aside when LibreHardwareMonitor is active**: `AdapterOptions::from_settings`
registers the NVIDIA adapter only if `nvidia` is not disabled *and* (LHM is disabled *or*
`adapter_settings.nvidia.always` is `true`). The reason is in
`crates/adapters/src/lib.rs`: both would report the same physical GPU with half the
readings each, and LHM reports more (it reads the hotspot through NVAPI, which NVML does
not expose).

To exercise NVML telemetry while LHM runs, add to `<config>\settings.json`:

```json
{ "adapter_settings": { "nvidia": { "always": true } } }
```

Otherwise stop LHM's web server for the duration of this section. Record which route you
took — the two produce different device lists.

### 10.2 Telemetry

```bash
cargo run -p ohm-cli -- --config-dir .\ohm-validation-cfg status
cargo run -p ohm-cli -- doctor
```

Expected device JSON shape for an NVML GPU (in `status`/`doctor` output the readings appear
as a line; the full JSON is what the desktop UI's device view shows and what
`commands::get_device` returns):

```json
{
  "id": "gpu.nvidia.0",
  "name": "NVIDIA GeForce RTX <model>",
  "type": "gpu",
  "vendor": "NVIDIA",
  "transport": "nvidia",
  "adapter": "nvidia",
  "capabilities": [
    { "id": "temperature.core", "kind": "sensor", "unit": "celsius", "readable": true, "writable": false },
    { "id": "temperature.hotspot", "kind": "sensor", "unit": "celsius", "readable": true, "writable": false },
    { "id": "load.gpu", "kind": "sensor", "unit": "percent", "readable": true, "writable": false },
    { "id": "power.gpu", "kind": "sensor", "unit": "watt", "readable": true, "writable": false },
    { "id": "memory.used", "kind": "sensor", "unit": "byte", "readable": true, "writable": false },
    { "id": "clock.mhz", "kind": "sensor", "unit": "megahertz", "readable": true, "writable": false },
    { "id": "fan.rpm", "kind": "sensor", "unit": "rpm", "readable": true, "writable": false },
    { "id": "fan.speed_percent", "kind": "actuator", "unit": "percent", "min": 0, "max": 100,
      "readable": true, "writable": true }
  ]
}
```

Files to inspect for that JSON: `adapters/nvidia/src/lib.rs` (device construction, ids,
capabilities), `crates/ohm-device-model/src/` (the `caps` constants, the `Device` struct —
where the field is `device_type` but serialises as `"type"` — and the reading states), and,
for a live copy, the desktop **Devices** screen or `Diagnostics`.

The example above is abridged. A real device object also carries `name`, and optionally
`model`, `metadata` and `tags`; each capability also carries `name` and `safety_critical`,
and optionally `step`, `values`, `description` and `poll_interval_ms`. Compare against the
live JSON rather than assuming the field list is complete.

Honest expectations — these are **not** bugs:

| Reading | Expectation |
|---|---|
| `temperature.hotspot` | **Always unavailable**, with reason `unsupported`, because NVML has no hotspot sensor; the message says so. Tools that display a hotspot use undocumented NVAPI calls. Do not report this as a defect; if LHM is active it can read a hotspot through NVAPI, which is a different provider. |
| `fan.rpm` | Present as a *commanded* fan speed, not a tachometer: NVML reports the intended speed, so a physically blocked fan still reports a value. Treat it as intent. |
| `fan.speed_percent` present but a write refused | Expected without elevation: the write is the one NVML path needing an administrator token, and the refusal is reported as `permission_denied` with the advice to run elevated. |
| Card/driver that refuses third-party fan control | Reported `vendor_limitation`/`unsupported` with a message pointing at the vendor tool. NVIDIA publishes no per-SKU fan-control support matrix, so **per-SKU support stays unverified** — record your card, driver version and the outcome as one observation. |
| Machine with no NVIDIA driver | `nvidia unavailable` (`driver_missing` / `library_not_found`). Normal. |

### 10.3 The fan write itself

Run from an **elevated** PowerShell, with LHM either stopped or `nvidia.always` set:

```bash
cargo run -p ohm-cli -- --log-level debug doctor
```

then perform the write in the desktop UI (§12) — the CLI has no write subcommand. Correct
outcomes are exactly two, and both are honest:

- **applied**: `audit.jsonl` gains an entry with `"status":"applied"`, `"origin":{"kind":"manual"}`,
  `"capability":"fan.speed_percent"`, and `"simulated":false`, and the GPU's reported fan
  percentage changes (verify independently — see §12.3 for why the app's own `applied`
  field is not a read-back);
- **refused**: `"status":"rejected"` with `error_code`/`detail` naming the cause —
  `permission_denied` (run elevated), `unsupported` (driver/GPU refuses), or
  `vendor_limitation`. A refusal recorded with a reason is a **pass** for this step: the
  requirement is that the refusal is honest and audited, never faked.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `gpu.nvidia.0` absent although an NVIDIA driver is installed | Either LHM is suppressing it (see 10.1) or the adapter is disabled in settings. Check both before reporting. |
| A write reported as `applied` and audited as applied, but the card's fan does not change | **Serious.** Report it: either the value never reached the driver, or the card ignores it while claiming success. Capture the audit line, the elevation state, the driver version and the card model. |
| `hotspot` reported as a number | Contradicts the documented NVML limitation. Report with the reading. |
| Any write reaching hardware while `--dry-run` is in force | Stop the session. See §6.2. |

**Status: Prepared.**

---

## 11. LibreHardwareMonitor: reachability and mapping

### 11.1 Reachability

```powershell
PS> (Invoke-WebRequest http://127.0.0.1:8085/data.json -UseBasicParsing).StatusCode
```

Expected: `200`. Other statuses and their meanings: `401` = LHM's optional Basic auth is
enabled (the adapter sends Basic auth when `adapter_settings.lhm.username`/`password` are
set); a connection error = server off or wrong port. The adapter's configured base URL is
`http://127.0.0.1:8085` (`DEFAULT_BASE_URL`) and can be changed with
`adapter_settings.lhm.base_url`; the client appends `/data.json`.

Then, with LHM running:

```bash
cargo run -p ohm-cli -- --log-level debug doctor
```

Expected: `lhm available <N> device(s)`, with motherboard, CPU, GPU and fan devices
present. Device ids carry the `lhm` namespace, and they are built from LibreHardwareMonitor's own
path plus the channel number it reports — not from the order devices came back in:
`cpu.lhm.cpu_0`, `gpu.lhm.gpu_0`, `motherboard.lhm.lpc_nct6687d_0`,
`fan.lhm.lpc_nct6687d_0_1`. A fan whose LHM sensor name carries no number, as NVIDIA's
GPUs report it, is named after its sensor path (`fan.lhm.gpu_0_0`). Record the ids you
see verbatim: they are what a rule targets, and comparing them with a second run is how
you find out whether the provider is numbering by position rather than by path.

### 11.2 Mapping rules to check by eye

The mapping is documented in `adapters/libre-hardware-monitor/src/mapping.rs`. Verify each:

| Rule | What you should see |
|---|---|
| One device per top-level hardware node | Each LHM hardware node (CPU, each GPU, each drive, the motherboard) is one device, and descendant sensors attach to it — a `System` temperature under `Motherboard / Nuvoton …` still lands on the motherboard device. |
| **Fans become their own devices** | `Fan #N` (an RPM sensor) is **paired** with `Fan Control #N` (a duty control) inside the same hardware node, produced as one device carrying both `fan.rpm` and `fan.speed_percent`. The pairing key comes from the sensor names and ids, so a channel with only a control and no tach still becomes a device (writable, no RPM). |
| Naming | The device is named with LibreHardwareMonitor's own spelling and the hardware it belongs to, e.g. `Fan #1 — Nuvoton NCT6687D`. The metadata carries `lhm_channel` (the LHM label) and `lhm_id` (the hardware node id). |
| Capability ids are normalised | `temperature.core`, `fan.rpm`, `fan.speed_percent`, `pump.rpm`, `pump.speed_percent` — so a rule written against the mock works against real hardware. Duplicates get a suffix instead of being dropped. |
| **The vendor sensor id is preserved per capability** | That original LHM `SensorId` (e.g. `/lpc/nct6687d/control/0`) is what the write path sends to `/Sensor?action=Set&id=…`. It appears in the device metadata, not in any logic. |
| Pump detection | A channel whose label contains "pump" becomes a **pump** device; `pump.speed_percent` is flagged safety-critical and the runtime's pump floor applies (see [`safety.md`](safety.md)). |

### 11.3 The expected failure is informative, not silent

Stop LHM (or turn its web server off) and run `doctor` again. Expected:

- `lhm unavailable 0 device(s) — could not reach the LibreHardwareMonitor web server at
  http://127.0.0.1:8085 (<detail>). Open LibreHardwareMonitor, choose Options -> Remote Web
  Server -> Run, and make sure the port matches (default 8085).`
- any LHM device that had already been discovered reports the reason **`not_present`** on
  every capability while the server is away, with the same explanatory detail — not zero,
  and not a stale value.

An unreachable LHM maps to `not_present` (adapter `unavailable`), an HTTP `401`/non-2xx to
`permission_denied` (adapter `unavailable`), and a malformed body to `read_error`.

**What a failure means**

| Symptom | Meaning |
|---|---|
| `lhm available` but zero fan devices | LHM found no SuperIO fan channel — common on laptops, some boards, and whenever LHM is not elevated. Record the board model; not necessarily a project defect. |
| Values shown as `0` instead of `[not_present]` when LHM is stopped | **Serious**: a fake zero where a reason is required. Report with both `doctor` outputs. |
| The reason shown is not `not_present` while the server is down | The error classification drifted from `LhmError::reason()`. Report which reason appeared. |
| A writable `fan.speed_percent` appears for a channel whose write is then refused by LHM | Possible and expected on boards that expose a control register the firmware overrides; §12 covers how to record it. |

**Status: Prepared.**

---

## 12. Per-device fan control verification (acceptance scenario D)

**Read [`safety.md`](safety.md) before this section.** Scenario D, as defined in
`tests/tests/acceptance.rs`, is: *real fan control works where it exists, and where it does
not, the refusal explains itself and nothing is faked.* The automated test covers this with
simulated hardware. This section is the real-hardware version of it.

### 12.1 Prepare

1. Nobody else is using the machine; you can see and hear it; you know how to cut its power.
2. LHM is running elevated with its web server on (§11.1 answers `200`).
3. Record every controllable device from `doctor`'s capability list:
   `PS> cargo run -p ohm-cli -- doctor` → the `• <device>/<capability>` lines.
4. Note the starting state of each output **before touching anything**: its current duty
   (if it reports one) and its current RPM.
5. Agree the stop conditions in [`safety.md`](safety.md) out loud.

### 12.2 The first pass is always `--dry-run`

Launch the desktop app with the flag:

```bash
cargo run -p ohm-desktop -- --dry-run
```

Perform the intended write in the UI (Devices → the fan device → its `fan.speed_percent`
control → set a value → Apply). Expected:

- the UI reports the write as `simulated`;
- **nothing on the machine changes** — the fan does not move, and the tachometer does not
  follow;
- `audit.jsonl` gains an entry with `"status":"simulated"`, `"simulated":true`, and
  `"detail":"dry run: hardware was not touched"`.

If the fan moves during this pass, stop the session immediately: the dry-run switch is not
reaching the write path (§6.2, same finding).

### 12.3 The supervised single real write — and why the app's "applied" is not proof

Relaunch without `--dry-run`, in an **elevated** shell if you are testing NVML, and make
**one** change to **one** output, starting from its current value and staying inside the
device's declared `min`/`max`.

Then verify **independently**, because the write path's answer is not a read-back:

- For LHM channels, `LhmAdapter::write` calls `GET /Sensor?action=Set&id=<SensorId>&value=<v>`
  and treats a 2xx response whose body is neither `null` nor `N/A` as success. The
  `applied` value in the audit entry is the value **sent**, not a value read back from the
  device.
- So read the channel back yourself:

```powershell
PS> (Invoke-WebRequest "http://127.0.0.1:8085/Sensor?action=Get&id=<SensorId>" -UseBasicParsing).Content
```

  where `<SensorId>` is the LHM sensor id from the device metadata (for example
  `/lpc/nct6687d/control/0`). And watch the paired tachometer — the same device's
  `fan.rpm` reading, or `Fan #N` in LHM's own tree — for a change that matches the
  direction and magnitude you asked for.

A request that "succeeded" while the reported value stays put, and the RPM does not move,
means the channel accepted nothing. That is a finding: record both values.

### 12.4 What a correct refusal looks like

A refusal is a **pass**, provided it is specific and audited. Expected shapes:

| Refusal source | Where it shows | Reason code |
|---|---|---|
| LHM's channel returns `null`/`N/A` after a Set | `LhmError::ControlRefused` → write rejected | `vendor_limitation`, with the message "LibreHardwareMonitor refused the write: … SuperIO fan control requires LHM to run as Administrator and the motherboard to expose a controllable channel." |
| LHM answers non-2xx or `401` | write rejected | `permission_denied` |
| LHM unreachable at write time | write rejected | `not_present` |
| NVML refuses (`NVML_ERROR_NO_PERMISSION`) | write rejected | `permission_denied` with the "run OpenHardwareOS elevated" advice |
| NVML/driver refuses third-party control | write rejected | `unsupported` / `vendor_limitation` |
| The safety policy blocks or clamps the value | write reported with `clamped: true`, or blocked with the policy's own message | — |

Every one of these must appear in `audit.jsonl` with `"status":"rejected"` (or
`applied`+`clamped`) — a refusal that leaves no trace is a failure of the audit
requirement, and a refusal that gets upgraded into a silent success is the worst outcome in
this kit.

Note the runtime's fail-safe: when a write to a duty control fails, the runtime
immediately attempts the fail-safe duty (`fail_safe_duty_percent`, default 70 %, or the
device-type floor, whichever is higher) as an origin-`safety` write. Seeing a *second*
audit entry right after a rejection is therefore **expected**, and it should say so in its
`origin.kind`. Confirm that the fail-safe write was itself audited.

### 12.5 The per-device table

For **each** controllable output, fill one row of the fan-control table in
[`result-template.md`](result-template.md): device id, capability, machine/component model,
requested value, value the device reported back, observed RPM before and after, applied or
refused, the exact refusal text, and the verdict. One row per output. An empty row means
that output is not verified.

### 12.6 Hand control back

1. Quit the app through the tray (**Quit**, not the window's X, if `close_to_tray` is on).
   `Runtime::shutdown` releases every LHM control channel with `SetDefault`, which returns
   the SuperIO's own fan curve.
2. Confirm each channel is back under firmware control: re-read
   `GET /Sensor?action=Get&id=<SensorId>` for the channels you changed and confirm the
   duty tracks the board again (RPM should follow the firmware's curve, not the value you
   last wrote).
3. If any output is still pinned, use LHM's own `SetDefault` (its GUI's control reset, or
   `…&value=null`), and confirm again.
4. Only if that fails: reboot, and let the firmware re-assert control at POST. Record the
   reboot as a last-resort recovery in the report.

### 12.7 What a failure means

| Symptom | Meaning |
|---|---|
| The write is refused with `vendor_limitation` and LHM is **not** elevated | Expected on Windows — the SuperIO is unreachable without the elevated LHM. Re-run the step with LHM correct before recording an outcome. |
| A fan goes to 0 RPM at a duty above the floor | **Stop the session.** See [`safety.md`](safety.md). This is a hardware risk, not a report item. |
| `applied` audited, device value unchanged, RPM unchanged | The channel accepted nothing. Report with both values and the SensorId. |
| The device reports a *different* value than requested (clamped by firmware/BIOS) | Record both; check `clamped` in the audit entry. Firmware-overridden headers are a known SuperIO behaviour, not necessarily a defect. |
| A refusal is not in `audit.jsonl` | The audit requirement is broken. Report immediately. |

**Status: Prepared.** No write has ever reached a real fan through this code.

---

## 13. Stop and report

When a step fails, or behaves in a way you cannot explain, **stop the session and capture
the following verbatim**. Do not clean it up, do not retry variations that could hide the
first failure, and do not work around it.

For each failure, put all of this in the bundle folder and reference it from the report:

1. **The exact command**, as typed, including the directory it was run from and the shell
   (elevated or not).
2. **The full stdout and stderr** — not a summary, and not just the last line. Redirect to
   a file so nothing is lost:
   `PS> cargo run -p ohm-cli -- doctor *> "$bundle\step-5.1-doctor.txt"`
3. **The exit code**: `PS> $LASTEXITCODE` (or `$?` for cmdlets).
4. **The tail of `audit.jsonl`** — the application's own record of every write:
   `PS> Get-Content "$env:APPDATA\OpenHardwareOS\audit.jsonl" -Tail 40` (or the
   `--config-dir` you used). Include the whole lines as JSON, not a reformatting.
5. **The adapter statuses from `doctor`** — the Providers block in full, with the reason
   text, plus the device lines for the device in question.
6. **The OS build and the machine identity**:
   `PS> (Get-CimInstance Win32_OperatingSystem) | Select-Object Caption, Version, BuildNumber, OSArchitecture`
   plus, when the step touched a component, its model: the motherboard
   (`Get-CimInstance Win32_BaseBoard`), the CPU (`Win32_Processor`), the GPU
   (`Win32_VideoController`), the disk (`Win32_DiskDrive` / `MSFT_PhysicalDisk`).
7. **The relevant log file**: `%APPDATA%\OpenHardwareOS\logs\openhardwareos.log.<YYYY-MM-DD>`,
   especially the lines around the failure timestamp.
8. **A screenshot or captured output** for anything visual: a window that did not hide, a
   tray menu, a UI write report, an installer or SmartScreen prompt.

Then, in the report:

- fill in the row for the failed step with status `Prepared` (or `Build/run passed` if the
  command itself exited as documented but the behaviour was wrong) — **never** leave a
  failed step labelled Verified;
- write the observation in the **unexplained observations** section if you cannot attribute
  it to a specific line of source;
- leave the sign-off line unsigned until the failure is reported to the maintainers. An
  unsigned report is a report that says "the session stopped here", which is a legitimate
  and expected outcome for this kit's first run.
