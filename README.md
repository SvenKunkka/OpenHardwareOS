# OpenHardwareOS

**An open runtime, device layer and automation engine for PC hardware.**

OpenHardwareOS reads what your machine is actually doing — CPU and GPU
temperature, load, power, storage temperature, fan RPM — and lets you automate
it: *when this sensor does that, change this device*. The first release focuses
on **cooling**, because that is where the value is obvious and where hardware
access is hardest.

This is an early preview. The runtime, automation engine, desktop UI and CLI are
implemented; Windows release builds are tested by GitHub Actions. Real fan-control
compatibility still needs verification on each motherboard and GPU. See the
[Windows acceptance guide](docs/windows-validation/ACCEPTANCE-ENTRY.md).

- **Local first.** No account, no cloud, no telemetry. Everything lives in one
  config directory on your machine.
- **Open source.** Apache-2.0, no vendor SDK is redistributed, and no code that
  forbids redistribution is shipped.
- **Honest about hardware.** When a sensor or a control channel is not available,
  OpenHardwareOS says *why* (`unsupported`, `permission_denied`,
  `driver_missing`, `hardware_limitation`) instead of showing a zero or faking a
  success.
- **Testable without hardware.** A deterministic simulated machine lets the whole
  `temperature -> rule -> fan` loop run anywhere, including CI.

---

## Windows: install from PowerShell

The prebuilt x64 release does not require Rust, Node.js or a source checkout.
Paste this block into Windows PowerShell 5.1 or newer. It downloads a fixed release,
verifies its checksum and installs to a new per-user version directory:

```powershell
& {
    $ErrorActionPreference = 'Stop'
    if ($env:PROCESSOR_ARCHITECTURE -ne 'AMD64' -and $env:PROCESSOR_ARCHITEW6432 -ne 'AMD64') { throw 'Windows x64 is required.' }
    $ohmVersion = 'v0.1.9'
    $ohmAsset = "ohm-cli-$ohmVersion-windows-x86_64.zip"
    $ohmUrl = "https://github.com/SvenKunkka/OpenHardwareOS/releases/download/$ohmVersion"
    $ohmInstall = Join-Path $env:LOCALAPPDATA "OpenHardwareOS\cli-$ohmVersion"
    if (Test-Path -LiteralPath $ohmInstall) { throw "Already exists; preserved: $ohmInstall" }
    $ohmDownload = Join-Path $env:TEMP ('ohm-download-' + [Guid]::NewGuid().ToString('N'))
    New-Item $ohmDownload -ItemType Directory | Out-Null
    $ohmZip = Join-Path $ohmDownload $ohmAsset
    Invoke-WebRequest -UseBasicParsing "$ohmUrl/$ohmAsset" -OutFile $ohmZip
    Invoke-WebRequest -UseBasicParsing "$ohmUrl/SHA256SUMS" -OutFile (Join-Path $ohmDownload 'SHA256SUMS')
    $ohmMatch = @(Get-Content (Join-Path $ohmDownload 'SHA256SUMS') | Where-Object { $_ -match ('^[0-9a-fA-F]{64}  ' + [Regex]::Escape($ohmAsset) + '$') })
    if ($ohmMatch.Count -ne 1) { throw 'Expected exactly one matching SHA256 entry.' }
    if ((Get-FileHash $ohmZip -Algorithm SHA256).Hash -ne $ohmMatch[0].Substring(0, 64)) { throw 'SHA256 mismatch; installation stopped.' }
    Expand-Archive -LiteralPath $ohmZip -DestinationPath $ohmInstall
    Remove-Item -LiteralPath $ohmDownload -Recurse -Force
    & (Join-Path $ohmInstall 'ohm-cli.exe') --version
    if ($LASTEXITCODE -ne 0) { throw 'Installed CLI could not start.' }
    & (Join-Path $ohmInstall 'ohm-cli.exe') doctor
    if ($LASTEXITCODE -ne 0) { throw 'Device diagnostic failed; keep the output for diagnosis.' }
}
```

The CLI is installed at `%LOCALAPPDATA%\OpenHardwareOS\cli-v0.1.9\ohm-cli.exe`.
An existing version directory is preserved and stops installation. These commands
do not change PowerShell execution policy or PATH; `doctor` reads device capabilities.
The download checksum is checked before extraction or execution.

For environments whose existing policy permits local PowerShell scripts, the
optional installer supports CLI updates in a separate managed `cli` directory and
desktop installation with `-Desktop`:

```powershell
Invoke-WebRequest -UseBasicParsing 'https://github.com/SvenKunkka/OpenHardwareOS/releases/download/v0.1.9/install.ps1' -OutFile "$env:TEMP\OpenHardwareOS-install.ps1"
& "$env:TEMP\OpenHardwareOS-install.ps1" -Version v0.1.9 -Desktop
```

The optional script and desktop installer are unsigned. The desktop installer
asks for administrator access and does not launch the application. Keep Windows security settings enabled; if a
policy blocks installation, retain the error for diagnosis. Hardware access through
LibreHardwareMonitor requires that separate application; it is not bundled.
The CLI is also available to Rust users directly from source:

```powershell
cargo install --git https://github.com/SvenKunkka/OpenHardwareOS --tag v0.1.9 --locked ohm-cli
```

[Release assets and checksums](https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/v0.1.9)
· [Windows installation details](docs/windows-install.md)
· [Linux installation details](docs/linux-install.md)
· [Running the rules in the background](docs/background-service.md)
· [Validating a real machine (field checklist)](docs/field-checklist.md)

## Versions and next steps

[Interactive version tree](https://svenkunkka.github.io/OpenHardwareOS/) ·
[Text version tree](docs/versions.md) · [Changelog](CHANGELOG.md) ·
[Version management](docs/version-management.md)

The tree separates published releases, current development and future plans.
Each published node links to its exact source, Windows installation guide and
verification evidence. Old release tags and downloads remain available.
The next feature milestone is **v0.2.0: pump support**, beginning with one
identified LibreHardwareMonitor pump channel; see the [pump plan](docs/plans/pump-support.md).

## Build from source

### Requirements

| | |
|---|---|
| Rust | 1.95 or newer (`rust-version` in `Cargo.toml`) |
| Node.js | 20+ (only for the desktop UI) |
| OS | Windows 10/11 (primary target), macOS and Linux for the runtime, CLI and simulated hardware |

### Run it

```bash
git clone https://github.com/SvenKunkka/OpenHardwareOS
cd OpenHardwareOS

# 1. See the whole cooling loop work with no hardware at all
cargo run -p ohm-cli -- demo --steps 90

# 2. See what this machine exposes, and whether it can be controlled
cargo run -p ohm-cli -- doctor

# 3. Desktop app
cd apps/desktop
npm ci
npx tauri dev                       # starts Vite and the desktop together
# For a packaged desktop build: npx tauri build
# From the repository root: cargo run -p ohm-desktop -- --selftest --mock
```

`ohm-cli demo` runs the simulated machine: the GPU heats under load, a rule maps
its temperature to a chassis fan, the fan spins up, and the GPU cools down —
including hysteresis, the safety floor and the audit log.

### What the flags do

| Flag | Effect |
|------|--------|
| `--mock` | also register the simulated hardware providers |
| `--dry-run` | never write to hardware, whatever the saved settings say |
| `--selftest` | start the runtime headlessly, run one automation cycle, print a report, exit |

---

## What is implemented today

| Area | State |
|------|-------|
| Unified device / capability / state model | **done** — `crates/ohm-device-model` |
| Hardware runtime: discovery, hotplug, polling, state store, event bus | **done** — `crates/ohm-runtime` |
| Safety layer: duty floors, fail-safe, emergency override, audit log | **done** — `crates/ohm-runtime/src/safety.rs` |
| Automation engine: curves, hysteresis, deadband, fallbacks, YAML rules | **done** — `crates/ohm-automation` |
| Simulated hardware (CPU, GPU, SSD, fans, pump, faults) | **done** — `adapters/mock` |
| OS sensors: CPU name/load/clock, thermal zones, storage | **done** — `adapters/system` |
| NVIDIA GPU telemetry and fan control (NVML) | **done** — `adapters/nvidia` |
| Motherboard sensors **and chassis fan control** (LibreHardwareMonitor) | **done** — `adapters/libre-hardware-monitor` |
| Open Device Protocol + simulated OpenFan | **done** — `crates/ohm-protocol`, `adapters/open-protocol` |
| Desktop UI (Overview, Devices, Automation, Settings, Diagnostics, simulator panel) | **done** — `apps/desktop` |
| Headless CLI (`doctor`, `watch`, `demo`, `rules`, `audit`, `protocol`) | **done** — `apps/cli` |
| Real USB HID / CDC enumeration for ODP devices | planned capability: physical ODP transport |
| Loading third-party adapter plugins at runtime | planned capability: runtime plugin loading |
| Scenes, profiles, WHEN/IF/THEN, app and game detection | planned capability: advanced automation |
| Natural language automation | planned capability: advanced automation |

Details, and the honest list of gaps, are in [`docs/roadmap.md`](docs/roadmap.md).

---

## The five things this repository does

```text
1. Reads PC hardware state      CPU / GPU / SSD / fans, through pluggable adapters
2. Unifies it                   one Device + Capability model, no vendor branches
3. Runs it                       discover, register, read, write, publish, persist
4. Automates it                  sensor -> curve -> actuator, with real safety
5. Shows it                      a desktop dashboard, and a headless CLI
```

### 1. It reads real hardware

```text
$ cargo run -p ohm-cli -- doctor
  Providers
  lhm      unavailable  0   device(s) — could not reach the LibreHardwareMonitor web
                              server at http://127.0.0.1:8085 (io: Connection refused).
                              Open LibreHardwareMonitor, choose Options -> Remote Web
                              Server -> Run, and make sure the port matches (default 8085).
  system   available    7   device(s)

  Devices
  Apple M5                        online     69.6 °C · load 18.9 %
  Storage: Macintosh HD           degraded   [unsupported] · free 124438907298 B
  Thermal Zone: NAND CH0 temp     online     —
```

That `[unsupported]` is the point: the drive temperature genuinely cannot be read
on that platform, and the app says so instead of inventing a number.

### 2. It abstracts correctly

```json
{
  "id": "gpu.nvidia.0",
  "name": "NVIDIA GeForce RTX 5090",
  "type": "gpu",
  "vendor": "NVIDIA",
  "transport": "nvidia",
  "adapter": "nvidia",
  "capabilities": [
    { "id": "temperature.core", "kind": "sensor",   "unit": "celsius", "readable": true, "writable": false },
    { "id": "fan.speed_percent","kind": "actuator", "unit": "percent", "readable": true, "writable": true,
      "min": 0, "max": 100 }
  ]
}
```

Business logic never mentions a vendor or a model. A rule written against
`gpu.mock.0/temperature.core` and `fan.mock.0/fan.speed_percent` drives an RTX
card and a SuperIO fan header through exactly the same code path.

### 3. It runs the hardware through one door

The UI, the CLI and the automation engine all call `Runtime::write_value`. That
means every write — manual or automated — passes the same range validation, the
same safety policy and the same audit log. The desktop app has no way to bypass
them, because it has no other way to reach a fan.

### 4. It automates with curves that behave

```yaml
name: GPU Cooling
source: { device: gpu.mock.0, capability: temperature.core }
target: { device: fan.mock.0, capability: fan.speed_percent }
curve:
  - [40, 20]
  - [60, 35]
  - [70, 50]
  - [80, 80]
  - [85, 100]
hysteresis: 2
update_interval_ms: 1000
fallback:
  on_sensor_missing: safe_default   # never leave a fan at an unknown speed
  on_write_failure: safe_default
```

Rules can also be gated on another sensor — "only while gaming" needs no second
rule — and a target may be owned by at most one enabled rule, enforced when
creating, enabling, importing and at tick time:

```yaml
when:
  source: { device: gpu.mock.0, capability: load.gpu }
  op: gt
  value: 60
  otherwise: safe_default   # when the gate is false: the fail-safe duty, never a stale fan value
```

Rules are plain YAML in `<config>/rules`, so they are readable, diffable and
shareable. `hysteresis` and `deadband` stop the fan from oscillating; losing the
sensor drives the fail-safe duty instead of holding a stale value; and the safety
layer refuses to drive a fan below its floor or a pump anywhere near zero.

### 5. It shows you what is going on

A calm dashboard — Overview with live sparklines, Devices with every capability
and its status, a device detail page with charts and a manual override,
Automation with curves and live rule status, Settings (including the safety
limits), Diagnostics with the audit trail, and a clearly labelled simulator panel
for demonstrating the closed loop on any machine.

---

## Architecture

```text
┌──────────────────────────── apps/desktop (Tauri v2 + React) ────────────────┐
│  Overview · Devices · Automation · Settings · Diagnostics · Simulator       │
└───────────────────────────────┬─────────────────────────────────────────────┘
                                │ commands + events (no hardware access)
┌───────────────────────────────▼─────────────────────────────────────────────┐
│ crates/ohm-runtime       device table · capability registry · state store    │
│                          event bus · safety policy · audit log · settings    │
├─────────────────────────────────────────────────────────────────────────────┤
│ crates/ohm-automation    rules · curves · hysteresis · fallbacks             │
├─────────────────────────────────────────────────────────────────────────────┤
│ crates/ohm-adapter-api   the HardwareAdapter SPI (probe/discover/read/write) │
├─────────────────────────────────────────────────────────────────────────────┤
│ adapters/  mock · system · nvidia · libre-hardware-monitor · open-protocol   │
├─────────────────────────────────────────────────────────────────────────────┤
│ crates/ohm-protocol      Open Device Protocol: framing, descriptors, mock    │
└───────────────────────────────┬─────────────────────────────────────────────┘
                                │ Windows APIs · NVML · LHM web server · USB HID
                            your hardware
```

Full details: [`docs/architecture.md`](docs/architecture.md),
[`docs/device-model.md`](docs/device-model.md),
[`docs/automation.md`](docs/automation.md),
[`docs/protocol.md`](docs/protocol.md).

### Repository layout

```text
crates/
  ohm-core            ids, errors, units, config paths, logging
  ohm-device-model    Device, Capability, Value, DeviceState, Reading
  ohm-adapter-api     the adapter trait every hardware provider implements
  ohm-runtime         discovery, device table, state store, event bus, safety, audit
  ohm-automation      rules, curves, evaluator, engine, YAML store
  ohm-protocol        Open Device Protocol: framing, messages, descriptors, mock device
  adapters            facade: which providers run, and in what order
adapters/
  mock                deterministic simulated PC (CPU/GPU/SSD/fans/pump + fault injection)
  system              OS sensors via sysinfo, plus Windows storage counters
  nvidia              NVML telemetry and (elevated) fan control
  libre-hardware-monitor  LHM web server: motherboard sensors and chassis fan control
  open-protocol       ODP devices (the simulated OpenFan today)
apps/
  cli                 headless runtime: doctor / status / watch / demo / rules / audit
  desktop             Tauri v2 backend + React/TypeScript UI
tests/                cross-crate acceptance, edge case, rule lifecycle and protocol tests
examples/rules/       ready to use automation rules
docs/                 architecture, device model, automation, protocol, roadmap, research, ADRs
```

---

## Why this stack

**Rust workspace + Tauri v2 + React/TypeScript.**

- The runtime must poll sensors, hold state, evaluate rules and never block the
  UI. Rust gives that with no runtime to ship, and the same code runs headless
  (`ohm-cli`) and inside the desktop app.
- The UI must not be able to touch hardware. Tauri keeps the boundary real: the
  webview talks to Rust commands, and the Rust side only exposes runtime APIs.
- .NET 8 + WinUI 3 was considered and rejected: the Windows App SDK's NuGet
  redistributable is proprietary (despite the repo being MIT), and it would tie
  the whole application to Windows. The one place .NET is genuinely useful —
  hosting LibreHardwareMonitor — is isolated to an optional helper process
  instead of becoming the foundation.

The full comparison, including bundle size and licensing, is in
[`docs/decisions/0001-ui-and-runtime-stack.md`](docs/decisions/0001-ui-and-runtime-stack.md),
and the underlying research with per-claim citations is in
[`docs/research.md`](docs/research.md).

---

## Hardware support, honestly

| What you want | What it takes | State |
|---|---|---|
| CPU temperature, load, clock | OS thermal zones / SMC | varies by platform; the app reports when a zone is missing |
| GPU temperature, load, power, fan RPM | NVML (NVIDIA) or LHM | NVML: no elevation. GPU fan *setting*: needs Administrator |
| GPU hotspot temperature | NVAPI, via LHM | **not** available from NVML |
| SSD temperature | Windows storage reliability counters; LHM elsewhere | `unsupported` on platforms that do not expose it |
| Chassis fan RPM **and** control | SuperIO access through a kernel driver — LibreHardwareMonitor | the only real path on Windows; requires LHM running as Administrator |
| Any of it, with no hardware | the simulated machine | always available |

Two consequences worth stating plainly:

1. **A refusal is not a bug.** NVML answers `NVML_ERROR_NO_PERMISSION` when the
   process is not elevated, and some SKUs refuse third-party fan control
   entirely. OpenHardwareOS reports `permission_denied` or `vendor_limitation`
   and never pretends the fan moved.
2. **No vendor SDK is redistributed.** AMD's ADL/ADLX licences forbid use in a
   project under a free-software licence, so AMD support comes through
   LibreHardwareMonitor; NVIDIA telemetry loads the driver's own NVML library.
   See [`docs/decisions/0004-vendor-sdks-and-amd.md`](docs/decisions/0004-vendor-sdks-and-amd.md).

---

## Safety

This is a Hardware OS, so hardware protection is a first-class feature, not a
checkbox. Every write passes through all of these:

| Risk | Mitigation |
|---|---|
| Fan driven to a dangerous low duty | `min_duty_percent` floor, applied to manual and automated writes |
| Pump stopped | a pump floor that is never relaxed, plus the device's own declared range |
| Sensor lost while controlling | fail-safe duty (default 70 %), configurable per rule |
| Thermal runaway | an emergency ceiling (default 90 °C) that forces every controlled output to 100 % — even with no rules configured |
| Adapter write failure | fallback duty, explicit error, and an audit entry — never a silent success |
| Fan left at an unknown speed on exit | control released to the firmware on shutdown |
| Silent hardware changes | every write is appended to `audit.jsonl` with its origin (manual / automation / safety) |

---

## Development

```bash
cargo test --workspace                    # unit + integration tests
cargo clippy --workspace --all-targets    # lints (the workspace denies `unsafe`)
cargo run -p ohm-cli -- demo              # the closed loop, headless
cargo run -p ohm-desktop -- --selftest    # the app, headless
cd apps/desktop && npm install && npm run build
```

Nothing in the test suite needs real hardware, elevated privileges or a network
connection, and CI runs the whole thing on Linux, macOS and Windows.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) — including the adapter contribution
guide and the licensing rules for new dependencies.

---

## Third-party components and licences

OpenHardwareOS is Apache-2.0. The application links Rust dependencies and embeds
its frontend; release assets include their [copyright and license notices](THIRD_PARTY_NOTICES.md).
Separately installed hardware libraries and vendor drivers are not bundled:

| Component | Licence | How it is used |
|---|---|---|
| [LibreHardwareMonitor](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor) | MPL-2.0 | Optional, out-of-process: the app talks to its HTTP web server. Nothing from LHM is linked or redistributed. |
| [Tauri](https://github.com/tauri-apps/tauri) / wry / tao | MIT OR Apache-2.0 | Desktop shell |
| [tokio](https://github.com/tokio-rs/tokio) | MIT | Async runtime |
| [serde](https://github.com/serde-rs/serde) / serde_json | MIT OR Apache-2.0 | Serialisation |
| [serde_yaml_ng](https://crates.io/crates/serde_yaml_ng) | MIT OR Apache-2.0 | Rule files (see `docs/decisions` for the maintained alternatives) |
| [sysinfo](https://github.com/GuillaumeGomez/sysinfo) | MIT | OS level sensors |
| [nvml-wrapper](https://github.com/Cldfire/nvml-wrapper) | MIT OR Apache-2.0 | NVML bindings; loads the driver's own library |
| [ureq](https://github.com/algesten/ureq) | MIT OR Apache-2.0 | LibreHardwareMonitor HTTP client |
| [clap](https://github.com/clap-rs/clap) | MIT OR Apache-2.0 | CLI |
| [tracing](https://github.com/tokio-rs/tracing) | MIT | Logging |
| [chrono](https://github.com/chronotope/chrono) | MIT OR Apache-2.0 | Timestamps |
| [parking_lot](https://github.com/Amanieu/parking_lot) | MIT OR Apache-2.0 | Locks |
| [dirs](https://github.com/dirs-dev/dirs-rs) | MIT OR Apache-2.0 | Config paths |
| [React](https://github.com/facebook/react) / [Vite](https://github.com/vitejs/vite) / TypeScript | MIT | UI build |
| NVIDIA NVML | NVIDIA proprietary | **Not** redistributed: loaded from the installed driver at runtime |

`ohm-cli protocol` and the simulated hardware exist so that no part of the
project requires any of these to be present.

---

## Documentation

| Document | Contents |
|---|---|
| [`docs/architecture.md`](docs/architecture.md) | Layers, data flow, concurrency, safety architecture |
| [`docs/device-model.md`](docs/device-model.md) | Device, Capability, Value, state, units, naming rules |
| [`docs/automation.md`](docs/automation.md) | Rules, curves, hysteresis, fallbacks, validation |
| [`docs/protocol.md`](docs/protocol.md) | Open Device Protocol: framing, messages, descriptors |
| [`docs/roadmap.md`](docs/roadmap.md) | Capability roadmap C1–C8, with what exists today |
| [`docs/plans/hardware-support/`](docs/plans/hardware-support/README.md) | Six-stage hardware support plan (versioned snapshot of issues #1–#7): order, substages, acceptance |
| [`docs/plans/pump-support.md`](docs/plans/pump-support.md) | v0.2.0 single-pump scope and acceptance criteria |
| [`docs/research.md`](docs/research.md) | Tech research with per-claim citations |
| [`docs/decisions/`](docs/decisions/) | Architecture decision records |

---

## Licence

Apache-2.0. See [`LICENSE`](LICENSE).
