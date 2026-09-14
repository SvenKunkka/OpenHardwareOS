# OpenHardwareOS — Technical Research

Scope: feasibility and licensing research for an Apache-2.0, Windows-first desktop app (Rust workspace core + Tauri v2 + React/TypeScript) that reads PC hardware sensors and automates cooling (temperature → fan speed curves).

**All versions, licenses and API facts below were verified on 2026-09-11** unless explicitly marked **unverified**. The task framing said "2025"; the ecosystem has moved since — every version number here is the state of the world in September 2026, and where a 2025-era fact has changed (notably LibreHardwareMonitor's kernel driver) that is called out explicitly. Anything that could not be confirmed against a primary source is marked **unverified** rather than guessed. Repository file paths are cited as `blob`/`raw` GitHub links so each claim can be re-checked line by line.

---

## Summary of decisions

1. **Keep Rust + Tauri v2 + React as the product shell** — do not switch to .NET 8 + WinUI 3; the only reason to prefer .NET is in-process hosting of LibreHardwareMonitorLib, which is achievable out-of-process.
2. **Never link AMD ADL/ADLX or NVIDIA NVAPI headers or libraries into our Apache-2.0 code** — the ADL/ADLX SDK licences forbid distributing SDK-derived source under any "Excluded License" (i.e. Apache-2.0). Route all vendor GPU interop through the MPL-2.0 LibreHardwareMonitor host instead.
3. **Ship the sensor/fan layer as a separate process that hosts unmodified `LibreHardwareMonitorLib` 0.9.6** (target `net10.0`, not `net8.0`), speaking a local IPC protocol to the Rust core; validate `lhm-service`/`lhm-client` (MIT) as a ready-made version of this pattern before writing our own. Cite the MPL-2.0 notice for the library.
4. **Scope the MVP to read-only telemetry + curve authoring + writes only where the hardware exposes them**: treat motherboard/SuperIO fan write as *best-effort and board-dependent*, GPU fan write as *not guaranteed*, and say so in the UI instead of promising control.
5. **Do not bundle WinRing0 in any form** — Microsoft Defender flags it as a vulnerable driver; require/prompt for **PawnIO** instead, and in v1 *detect and prompt* (point users at pawnio.eu) rather than redistributing `PawnIO_setup.exe` (GPL-2.0-or-later).
6. **Do not put `requireAdministrator` on the Tauri executable.** Install an elevated helper from the installer: first release = Task Scheduler task with `RunLevel=HighestAvailable` (the same mechanism Tauri's own updater uses); later = a `windows-service` + named pipe if fan control must survive logon/logoff.
7. **Ship NSIS as the primary installer with `installMode: "perMachine"`** (a driver/helper install needs admin anyway); treat MSI as secondary because Tauri's WiX template is always per-machine and needs the VBSCRIPT optional feature.
8. **Set workspace MSRV to 1.95** — `sysinfo` 0.39.6 requires 1.95, above Tauri's 1.77.2 and `serde-saphyr`'s 1.89.
9. **Use `sysinfo` 0.39.6 for CPU/RAM/disk and `raw-cpuid` 11.6.0 for static CPU identity**; do not use `sysinfo::Components` for CPU temperature (Windows backend is WMI `MSAcpi_ThermalZoneTemperature`, usually empty or bogus) and do not expect fan RPM from it.
10. **Read SSD/NVMe temperature via WMI `MSFT_StorageReliabilityCounter.Temperature` first**, NVMe `IOCTL_STORAGE_QUERY_PROPERTY` with `StorageDeviceTemperatureProperty` second; never use `Win32_TemperatureProbe` (Microsoft: `CurrentReading` is not populated) or `Win32_Fan` (Microsoft: `SetSpeed` not implemented).
11. **Use `nvml-wrapper` 0.13.0 for NVIDIA telemetry and for NVML duty-% fan writes** — both are documented for Maxwell-and-newer and both load the driver's `nvml.dll` at runtime (we ship nothing) — but plan around the fact that **NVML has no fan-*curve* API**: a curve is our own poll-and-set loop, and the curve-capable vendor path (NVAPI) is undocumented and cannot be vendored.
12. **Pin `serde-saphyr` 1.2.0 exactly for YAML** (pure Rust, `unsafe` denied) behind a single config module, with `serde_norway` 0.9.42 as the drop-in fallback; keep TOML as the escape hatch.
13. **Gate CI on `cargo-deny` + `cargo-audit` and generate a THIRD-PARTY notices file** covering LibreHardwareMonitor (MPL-2.0), PawnIO (GPL-2.0-or-later), NSIS (zlib/libpng + bzip2 + CPL-1.0), WiX (§MS-RL, build-time only) and the WebView2 Runtime (Microsoft terms).

---

## 1. LibreHardwareMonitor (LHM)

### 1.1 Upstream and licence

| Item | Value |
|---|---|
| Upstream repo | https://github.com/LibreHardwareMonitor/LibreHardwareMonitor (org `LibreHardwareMonitor`, 9,039 ★, not archived, last push 2026-09-10) |
| Licence (repo/app) | **MPL-2.0** — the `LICENSE` file is the verbatim Mozilla Public License 2.0 |
| Licence (NuGet package) | `MPL-2.0` via `PackageLicenseExpression` in the csproj |
| Library NuGet id | `LibreHardwareMonitorLib` |
| Library target frameworks | `net472;netstandard2.0;net8.0;net9.0;net10.0` (RIDs: `win-x64;win-x86;win-arm64;linux-x64;linux-arm64`) |
| Latest **stable** release | **0.9.6** (tag `v0.9.6`, published 2026-02-14) |
| Latest NuGet version | **0.9.7-pre736** (prerelease; 747 versions published in total) |
| GUI app target frameworks | `net472;net10.0-windows` (Windows Forms) |
| Third-party notices | `THIRD-PARTY-NOTICES.txt` — Aga.Controls (BSD), PawnIO.Modules (**LGPL-2.1**) |

Sources: [repo](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor), [README](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/README.md), [LICENSE](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LICENSE), [LibreHardwareMonitorLib.csproj](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/LibreHardwareMonitorLib.csproj), [NuGet flat-container index](https://api.nuget.org/v3-flatcontainer/librehardwaremonitorlib/index.json), [NuGet package page](https://www.nuget.org/packages/LibreHardwareMonitorLib/), [THIRD-PARTY-NOTICES.txt](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/THIRD-PARTY-NOTICES.txt).

**MPL-2.0 in practice (this is the good news):** MPL-2.0 is *file-level* copyleft. Consuming the **unmodified** NuGet package from our sidecar creates no obligation on our Apache-2.0 code. If we ever patch an LHM `.cs` file we must publish that file's source under MPL-2.0. Keeping LHM out-of-process and unmodified keeps the Apache-2.0 core clean and is the main reason to prefer a sidecar over any form of static incorporation.

### 1.2 Can `LibreHardwareMonitorLib` be used from a Rust/Tauri app?

`LibreHardwareMonitorLib` is a **C# assembly**, so "using it from Rust" always means "get a CLR involved". Three realistic integration options, in order of recommendation:

| # | Option | Mechanics | Verdict |
|---|---|---|---|
| 1 | **Sidecar .NET process** | A small `net10.0` console/service that references `LibreHardwareMonitorLib` and exposes sensors over a local transport (named pipe, stdio JSON-lines, or loopback HTTP). Tauri launches/supervises it (`bundle.externalBin`, https://v2.tauri.app/develop/sidecar/) or installs it as a service. | **Recommended.** Process isolation keeps MPL-2.0 boundaries trivial, lets the sidecar run elevated while the UI does not, and survives driver faults. |
| 2 | **LHM's built-in HTTP web server** | Run the *GUI* binary (`LibreHardwareMonitor.exe`) with its web server enabled and poll `http://127.0.0.1:8085/data.json`. | **Good for a prototype / fallback**, not for a product: the server lives in the Windows Forms assembly, not in the library, it is **off by default**, and the host process is a GUI app whose manifest demands administrator. |
| 3 | **In-process CLR hosting from Rust** | Host the .NET runtime in our own process via `hostfxr`/`hostpolicy` and call `LibreHardwareMonitorLib` through FFI. The Rust crate `netcorehost` **0.22.0** (MIT, updated 2026-07-26, https://crates.io/crates/netcorehost, https://github.com/OpenByteDev/netcorehost) exists for exactly this. `lhm-service`/`lhm-sys` (MIT) is a working example of a Rust↔C# bridge built this way. | **Possible but not for v1**: an unhandled .NET exception or a driver fault takes the whole UI down with it, and elevation has to apply to the entire process. |

There is also an existing Rust bridge worth evaluating before writing our own: **`lhm-service` 0.2.0 / `lhm-client` 0.3.0 / `lhm-sys` 0.1.1** (all MIT, https://github.com/jacobtread/lhm-service, crates.io verified). It installs a Windows service once (the only admin step), loads `LibreHardwareMonitorLib` over FFI, and answers user-mode clients over the named pipe `\\.\pipe\LHMLibreHardwareMonitorService` in MsgPack — i.e. it implements the recommended architecture *and* the "no admin for the app" property. Its README states the goal plainly: *"Access CPU/GPU temperatures and other hardware data on Windows without needing to run your app as Administrator."* Note it requires **.NET SDK 8.0** and **Visual Studio 2022** (with the Desktop C++ workload) to build `lhm-sys`, and it is a single-maintainer, low-download (≈1–2 k) project — evaluate, don't assume.

### 1.3 LHM's built-in HTTP web server — exact contract

Source of truth: [`HttpServer.cs`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Utilities/HttpServer.cs) and [`MainForm.cs`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/UI/MainForm.cs).

| Property | Value |
|---|---|
| Default port | **8085** (`_settings.GetValue("listenerPort", 8085)`) |
| Default bind address | `"?"` sentinel → falls back to `+` (all interfaces) if the configured IP is not a local address. Comment in source: `_settings.GetValue("listenerIp", "?")` |
| Enabled by default? | **No.** `_runWebServer = new UserOption("runWebServerMenuItem", false, runWebServerMenuItem, _settings)` — the default is `false`. |
| Backend | `System.Net.HttpListener`, realm `"Libre Hardware Monitor"`, `AuthenticationSchemes.Anonymous` unless auth is enabled |
| Auth | Optional HTTP **Basic**; the stored secret is a SHA-256 of the password (`PasswordSHA256`, compared against `ComputeSHA256(identity.Password)`). Unauthenticated requests get `401`. |
| Required privilege | The GUI's manifest is `requireAdministrator`, so the server runs elevated. Binding `+` on Windows additionally needs admin/URL-ACL. |

**Endpoints** (all after the auth check):

| Method + path | Behaviour |
|---|---|
| `GET /data.json` | Full sensor tree as JSON (details below). Sends `Cache-Control: no-cache`, `Access-Control-Allow-Origin: *`, and gzip when `Accept-Encoding: gzip`. |
| `GET /data.json` … any request with `Sensor` in the path | `/Sensor?action=Get&id=<id>` → `{"value":..,"min":..,"max":..,"format":".."}`; `action=ResetMinMax`; `action=Set&value=<float>` → `Control.SetSoftware(float)`; `value=null` → `Control.SetDefault()`. Responses are `{"result":"ok",...}` or `{"result":"fail","message":"..."}`. |
| `POST /Sensor` | Same as above but REST-styled; the only supported base URL is `/Sensor`. |
| `GET /metrics` | **Prometheus / OpenMetrics** text. Query params: `archivelength` (0–10, clamped), `timestamps` (0/1), `lastvalue` (0/1); the effective values are echoed in `X-archivelength` / `X-timestamps` / `X-lastvalue` headers. Metric names are `lhm_<hardwareType>_<sensorType>_<unit>` (e.g. `lhm_cpu_temperature_celsius`) with labels `sensorName`, `sensorAlias`, `hardwareName`, `hardwareAlias`, `sensorId`, `hardwareId`, `host`. |
| `GET /ResetAllMinMax` | Resets min/max on every sensor and returns the JSON tree. |
| `GET /` or `/index.html`, `images_icon/<file>` | The embedded built-in web UI and its icons (served out of assembly resources). |

**JSON shape of `/data.json`** (from `SendJsonAsync` + `GenerateJsonForNode`). The root is a synthetic node whose `Children[0]` is a single wrapper node containing the hardware tree:

```json
{
  "id": 0,
  "Version": "0.9.6.0",
  "Text": "Sensor",
  "Min": "Min", "Value": "Value", "Max": "Max",
  "ImageURL": "",
  "Children": [
    {
      "id": 1, "Text": "Sensor",
      "Min": "", "Value": "", "Max": "",
      "ImageURL": "images_icon/computer.png",
      "Children": [
        {
          "id": 2, "Text": "Nuvoton NCT6798D",
          "HardwareId": "/lpc/nct6798d/0",
          "ImageURL": "images_icon/chip.png",
          "Min": "", "Value": "", "Max": "",
          "Children": [
            {
              "id": 3, "Text": "Fans",
              "ImageURL": "images_icon/fan.png",
              "Min": "", "Value": "", "Max": "",
              "Children": [
                {
                  "id": 4, "Text": "Fan #1",
                  "SensorId": "/lpc/nct6798d/0/fan/0",
                  "Type": "Fan",
                  "Min": "620 RPM", "Value": "1180 RPM", "Max": "2400 RPM",
                  "RawMin": 620.0, "RawValue": 1180.0, "RawMax": 2400.0,
                  "ImageURL": "images/transparent.png",
                  "Children": []
                }
              ]
            }
          ]
        }
      ]
    }
  ]
}
```

Node-type nuances that matter for a Rust parser:

- **Sensor nodes** carry `SensorId`, `Type` (the `SensorType` enum name, e.g. `Temperature`, `Fan`, `Control`, `Load`, `Power`), plus **both** formatted (`Min`/`Value`/`Max`, e.g. `"1180 RPM"`) and numeric (`RawMin`/`RawValue`/`RawMax`) values. **Use the `Raw*` fields**; the formatted ones embed units and are culture-formatted.
- **Hardware nodes** carry `HardwareId` (e.g. `/gpu-nvidia/0`, `/lpc/nct6798d/0`).
- **Type nodes** (the `Temperatures` / `Fans` / `Controls` groupings) have no extra fields.
- `Min`/`Value`/`Max` are `string.Empty` on non-sensor nodes.
- The serialiser sets `JsonNumberHandling.AllowNamedFloatingPointLiterals`, so a NaN sensor value is emitted as the **string literal `"NaN"`**, not as a JSON number and not as bare `NaN`. A strict `serde_json` struct with `f64` fields will fail to deserialize that; use `Option<f64>` with a custom deserializer, or deserialize `RawValue` as an untagged enum of number-or-"NaN".
- `Control`-type sensors are the writable ones: the `SensorNode.Control` is a LHM `IControl` (`MinSoftwareValue` = 0, `MaxSoftwareValue` = 100) and `/Sensor?action=Set` maps straight onto `SetSoftware(value)` — this is the whole fan-write API surface. `IControl` is defined at [`LibreHardwareMonitorLib/Hardware/IControl.cs`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/Hardware/IControl.cs).

### 1.4 Enabling the server: GUI and config file

- **GUI:** `Options → Remote Web Server → Run` (checkbox), `Options → Remote Web Server → Interface / Port`, and `Options → Remote Web Server → Authentication`. Menu item names in source are exactly `"Remote Web Server"`, `"Run"`, `"Interface / Port"`, `"Authentication"` ([MainForm.Designer.cs](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/UI/MainForm.Designer.cs)).
- **Config file:** settings are loaded from **`Path.ChangeExtension(Application.ExecutablePath, ".config")`** — i.e. `LibreHardwareMonitor.config` **next to the executable** — and saved on exit. The format is .NET `appSettings` XML ([PersistentSettings.cs](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Utilities/PersistentSettings.cs)):

```xml
<?xml version="1.0" encoding="utf-8"?>
<configuration>
  <appSettings>
    <add key="runWebServerMenuItem" value="true" />
    <add key="listenerIp" value="127.0.0.1" />
    <add key="listenerPort" value="8085" />
    <add key="authenticationEnabled" value="false" />
    <add key="startMinMenuItem" value="true" />
  </appSettings>
</configuration>
```

Setting `runWebServerMenuItem=true` pre-seeds the checkbox so the server starts with the app without a user click; the GUI rewrites this file on exit, so treat it as a file we generate *before* first launch, not one we own.

### 1.5 Administrator rights, the kernel driver, and what that means for distribution

- **The LHM GUI requires administrator**: [`app.manifest`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Resources/app.manifest) contains `<requestedExecutionLevel level="requireAdministrator" uiAccess="false" />`. The README also states plainly that *"Some sensors require administrator privileges to access the data."*
- **The kernel driver is PawnIO — not WinRing0.** LHM merged **PR #1857 "Swap WinRing0 to PawnIO"** on **2025-09-16** ([PR](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/pull/1857)); current `master` contains a whole `LibreHardwareMonitorLib/PawnIo/` backend (`PawnIo.cs`, `LpcIO.cs`, `IsaBridgeEc.cs`, `IntelMsr.cs`, `RyzenSmu.cs`, `Nvidia.cs`, …) with **signed modules embedded as resources** from `PawnIO.Modules` release **0.2.11** ([`Resources/PawnIo/README`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/Resources/PawnIo/README)). The known regression noted in that PR is that *"Gigabyte EC is unimplemented for now"*.
  - Driver detection: `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\PawnIO` → `DisplayVersion`; LHM warns if the version is `< 2.0.0`.
  - Installation flow: LHM embeds **`PawnIO_setup.exe`** as a resource, prompts with a `MessageBox` (*"PawnIO is not installed, do you want to install it?"*), extracts it to the current directory and runs it with `-install`, then deletes it ([`PawnIo.cs`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/PawnIo/PawnIo.cs), `MainForm.cs`). Runtime access is via `CreateFile(@"\\?\GLOBALROOT\Device\PawnIO")` + `DeviceIoControl` (`ControlCode.LoadBinary`, `ControlCode.Execute`).
- **Why WinRing0 is gone:** Microsoft Defender classifies `WinRing0` as a vulnerable driver as documented in **CVE-2020-14979**, with the detection name `VulnerableDriver:WinNT/Winring0`; Microsoft's advisory explicitly names the affected applications — *"CapFrameX, EVGA Precision X1 (older versions), FanCtrl, HWiNFO, **Libre Hardware Monitor**, MSI Afterburner, Open Hardware Monitor, OpenRGB, OmenMon …"* ([Microsoft support article](https://support.microsoft.com/en-us/windows/security/threat-malware-protection/microsoft-defender-antivirus-alert-vulnerabledriver-winnt-winring0), [CVE-2020-14979](https://nvd.nist.gov/vuln/detail/CVE-2020-14979)). The WinRing0 driver also appears on Microsoft's [recommended vulnerable driver blocklist](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/app-control-for-business/design/microsoft-recommended-driver-block-rules); since the Windows 11 2022 update that blocklist is **on by default**, and it is enforced whenever memory integrity/HVCI, Smart App Control or S mode is active. FanControl's release notes record the field impact: *"as of 09/04/2025, WinRing0 (FanControl.sys) used in V237 and below is flagged as Trojan:Win32/Vigorf.A by Windows Defender, causing sensors to not be detected"* ([FanControl README](https://github.com/Rem0o/FanControl.Releases)).

**Distribution and signing implications:**

1. **PawnIO is a separate product with its own licence and its own installer.** Its README states: *"This program is free software; you can redistribute it and/or modify it under the terms of the GNU General Public License … version 2 … or (at your option) any later version"*, with a linking exception that lets independent programs talk to it **only through the device IOCTL interface** and explicitly *"does not include programs that communicate with PawnIO over the Pawn interface"* ([namazso/PawnIO README](https://github.com/namazso/PawnIO)). PawnIO ships in four flavours per [pawnio.eu](https://pawnio.eu/): official (digitally signed), unrestricted (unsigned, signature checking disabled), open-source, and custom-licensed. **Bundling `PawnIO_setup.exe` inside our installer is a redistribution act with GPL-2.0-or-later obligations** (source offer, licence text, no additional restrictions). The low-friction, low-risk v1 choice is to detect PawnIO and deep-link the user to pawnio.eu.
2. **If we ever ship our own PawnIO module** (e.g. a custom SuperIO module), that module is loaded *over the Pawn interface*, so it must be GPL/LGPL-2.1-compatible — PawnIO's README recommends **LGPL-2.1**. That means a dedicated LGPL-2.1 module crate in our workspace, not Apache-2.0.
3. **A signed kernel driver of our own is a multi-quarter project**: since Windows 10 1607 *"Windows will not load any new kernel-mode drivers which are not signed by the Dev Portal"*, and getting there needs an EV code-signing certificate plus a Hardware Dev Center account; cross-signed drivers load only when Secure Boot is off ([Driver Signing Policy](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later-)). Do not plan on it.
4. **Consequence for the installer:** the "install the driver" step must be a visible, consented, UAC-elevated step. Do not silently drop a driver; users' AV and Microsoft's blocklist will fight it, and reputationally we would inherit exactly the problem LHM just spent a release escaping.

---

## 2. Windows fan read/write reality (2026)

### 2.1 What actually reads fan RPM

| Path | Reads RPM? | Needs admin? | Notes |
|---|---|---|---|
| **Motherboard SuperIO via LHM** (`Nuvoton NCT67xx`, `ITE IT87xx`, …) | Yes, for headers wired to the SuperIO fan tach inputs | Yes (PawnIO device + LPC I/O) | The only broadly working desktop path. LHM enumerates the chip, reads tach registers, and exposes `SensorType.Fan` sensors; `/lpc/<chip>/0/fan/N` in `/data.json`. |
| **Embedded controller (EC)** on laptops/OEM boards | Sometimes | Yes | LHM has `IsaBridgeEc`, `LpcACPIEC`, `LpcCrOS` PawnIO modules; EC register maps are per-vendor and undocumented. |
| **GPU fans via NVML** | Yes — `nvmlDeviceGetFanSpeed`, `nvmlDeviceGetFanSpeed_v2`, `nvmlDeviceGetFanSpeedRPM` | **No** for reading | Reading is unprivileged; see §3. |
| **GPU fans via NVAPI/ADL** | Yes | No | What LHM itself uses (see §4). |
| **`Win32_Fan` (WMI `root\CIMV2`)** | **No** | — | Microsoft documents `SetSpeed` as *"Not implemented"*, and the current speed "is determined by a sensor (`CIM_Tachometer`)" — no in-box provider populates it ([Win32_Fan](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-fan)). |
| **ACPI thermal zones via WMI** | **No** | — | `MSAcpi_ThermalZoneTemperature` is temperatures only, and usually one coarse chassis zone. |
| **smartmontools / `smartctl`** | **No** (temperatures only) | Often | GPL-2.0 external process; see §5. |

**There is no in-box, documented Windows API that returns fan RPM for motherboard fans.** Every working implementation goes through a signed kernel driver that maps I/O ports or the EC. That is the single most important architectural fact in this document.

### 2.2 What actually writes fan speed

| Path | Writes? | How | Constraints |
|---|---|---|---|
| **SuperIO PWM via LHM** | Yes, when the board exposes it | LHM creates a `SensorType.Control` sensor (0–100) bound to an `IControl`; `SetSoftware(value)` writes the PWM duty register. In LHM this is `CreateControlSensors(...)` in [`SuperIOHardware.cs`](https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/Hardware/Motherboard/SuperIOHardware.cs), driven through `ControlMode.Software` → `superIO.SetControl(index, byte)`. | Board- and header-dependent: many boards expose no Control sensor at all, or expose it for only some headers. Some headers are firmware-controlled and revert. A wrong register write on an unsupported board is the classic "spins to 100 %" or "fans stop" failure. |
| **GPU fan via ADL (AMD)** | Yes, via Overdrive API levels | LHM's `AmdGpu` builds a `Control` sensor and drives Overdrive (`_currentOverdriveApiLevel`, Overdrive8 paths) through `atiadlxx.dll`. | AMD-only; vendor EULA constraints on *our* code (see §4); some mobile/APU parts are locked. |
| **GPU fan via NVAPI (NVIDIA)** | Yes, in practice | FanControl uses **NvAPIWrapper** for *"Nvidia GPU fan control and sensor reading"* ([FanControl README](https://github.com/Rem0o/FanControl.Releases)); LHM's `NvidiaGpu` also P/Invokes `NvApi` (`NvAPI_GPU_GetThermalSettings`, fan handles). | NVAPI is **closed and largely undocumented for fan control**; NvAPIWrapper is LGPL-3.0 (see §7). |
| **GPU fan via NVML** | Yes, documented | `nvmlDeviceSetFanSpeed_v2` — see §3.3. | Requires admin/root (`NVML_ERROR_NO_PERMISSION` otherwise); documented for Maxwell+ with no platform carve-out, but **no curve API** (duty % only) and no per-SKU support matrix. |
| **Vendor WMI/ACPI (OEM laptops)** | Partially | Vendor-specific WMI gadgets (e.g. Lenovo Legion "GameZone"/WMI gaming methods, ASUS WMI, Dell/HP BIOS-setting WMI) expose thermal *profiles*, and the Linux kernel has upstream drivers for several of these families. | Largely undocumented on Windows, model-specific, and not abstracted by any library. **Treat as unverified / out of scope for the MVP.** |
| **Liquid-cooler / fan-controller USB devices** | Yes | LHM has device backends: NZXT Kraken V2/V3 & Grid V3, Corsair-style Aquacomputer (Octo, Quadro, Farbwerk360), MSI CoreLiquid, Arctic, Razer, AeroCool, T-Balancer. | Per-device protocols over HID; these are the *most* reliable write paths because the device is designed to be controlled. |

### 2.3 Hard limits, bluntly

1. **Admin is unavoidable for motherboard fan I/O.** The PawnIO device is opened from an elevated process; LHM's own GUI runs `requireAdministrator`; OpenRGB's documentation is explicit for the same driver: *"You must run the application as Administrator in order for PawnIO to be able to access SMBus"* ([OpenRGB SMBusAccess.md](https://github.com/CalcProgrammer1/OpenRGB/blob/master/Documentation/SMBusAccess.md)).
2. **Driver signing is a hard wall.** New kernel drivers need Dev Portal signing; cross-signed drivers only load with Secure Boot off. So the driver is either a third-party signed one (PawnIO) or none.
3. **The vulnerable-driver blocklist actively removes the old approach.** WinRing0 is flagged and blocklisted by Microsoft (CVE-2020-14979), which broke real products in 2025 and forced FanControl and LHM to migrate to PawnIO. Any design that leans on WinRing0 is dead on arrival.
4. **Laptop EC ≠ desktop SuperIO.** On most laptops the EC is the only fan controller and it is firmware-owned; there is no generic, safe write path. Vendors expose at most thermal profiles. LHM's EC modules are best-effort and the PawnIO migration dropped Gigabyte EC support entirely.
5. **Vendor lock-in is real on the GPU side.** NVIDIA fan *duty* control is documented through NVML, but curve/policy semantics live in **undocumented NVAPI**; AMD's control path is ADL/ADLX-only, with licences actively hostile to open source (§4.3). We cannot legally vend either SDK, so our GPU fan support is exactly as good as the MPL-2.0 LHM host plus the documented NVML surface.
6. **Risks we must design around:** writing a PWM value a board's firmware also controls can cause oscillation or a full-speed fan; a helper that dies mid-write can leave a fan at a fixed low duty (thermal risk); a wrong register write can hang the SMBus or the whole machine; anti-cheat and AV consider these drivers suspicious by design; and users will blame the app for firmware that ignores us.

### 2.4 What the MVP can and cannot do

| Capability | MVP verdict |
|---|---|
| CPU package temperature, clocks (rated), load, per-core load | **Can do** (LHM sidecar; `sysinfo` for load/RAM/disk). |
| CPU *live* frequency | **Cannot do reliably.** `sysinfo` on Windows returns the rated clock via `CallNtPowerInformation(ProcessorInformation).CurrentMhz`; live boost needs MSR/APERF-MPERF, i.e. the driver. Highest turbo from `raw-cpuid` leaf `0x16` is available. |
| Motherboard/chipset temperatures, fan RPM, voltages | **Can do when LHM finds a SuperIO chip**; must degrade gracefully. |
| Motherboard fan PWM write | **Can do only where a `Control` sensor exists.** Must be opt-in, with "restore defaults / restore BIOS control" always one click away. |
| GPU temperature / power / utilisation / VRAM / fan RPM | **Can do** (NVML for NVIDIA; LHM for AMD/Intel). |
| GPU fan curve write | **Duty-% control is achievable on both vendors** (NVIDIA: NVML `nvmlDeviceSetFanSpeed_v2`, documented for Maxwell+ but requiring elevation; AMD: through LHM's Overdrive `Control` sensor, best effort). **A hardware curve *table* is not available through NVML** — the curve is our own poll-and-set loop. Ship it as experimental, with a "driver auto" fallback that always works. |
| SSD/NVMe temperature | **Can do** (WMI reliability counter first, NVMe IOCTL second) — §5. |
| Laptop fan control | **Cannot do generically.** Do not claim it. |
| Fan RPM on a laptop without LHM support | **Cannot do.** |

---

## 3. NVIDIA

### 3.1 `nvml-wrapper`

| Field | Value |
|---|---|
| Crate | `nvml-wrapper` |
| Latest version | **0.13.0** (published 2026-08-31) |
| Licence | **MIT OR Apache-2.0** |
| MSRV | **1.60.0** (`rust-version` on the 0.13.0 manifest) |
| Edition | 2021 |
| Repo | https://github.com/Cldfire/nvml-wrapper |
| Bindings crate | `nvml-wrapper-sys` **0.10.0**, MIT OR Apache-2.0, `rust_version 1.60.0`, features `default=[]`, `legacy-functions` |
| Linking | **Dynamic, via `libloading ^0.8.1`.** The crate hardcodes only a bare filename — `#[cfg(not(target_os = "linux"))] const LIB_PATH: &str = "nvml.dll";` — so the Windows loader search order resolves it to `System32`. The README: *"the NVML library gets loaded upon calling `Nvml::init` and can return an error if NVML isn't present"*. We link no NVIDIA binary at build time and redistribute nothing. |
| Feature flags (0.13.0) | `default = []`; `legacy-functions` (enables pre-`_v2`/`_v3` NVML entry points); `serde` |
| Init API | `Nvml::init()` (loads all symbols up front; per-call errors if a symbol was missing) or `Nvml::builder()` → `NvmlBuilder` for flexible initialisation |
| NVML version targeted | NVML 12 (wrapper README); NVML is backwards-compatible across driver versions. |

Sources: https://crates.io/api/v1/crates/nvml-wrapper, https://github.com/Cldfire/nvml-wrapper, https://docs.rs/nvml-wrapper/latest/nvml_wrapper/.

Typical Rust surface (`Nvml::init()` → `nvml.device_by_index(0)` → `Device` methods such as `device.fan_speed(0)?`, `device.power_limit()`), per the README example.

### 3.2 Reading telemetry — NVML functions

All function names below are from the official **NVML API Reference Guide, Release 615** PDF (https://docs.nvidia.com/deploy/pdf/NVML_API_Reference_Guide.pdf); the HTML API reference moved and the old `group__nvmlFanSpeed.html` URLs now 404, so the PDF is the citable primary source.

| Metric | NVML C function | Notes from the reference |
|---|---|---|
| GPU (edge) temperature | `nvmlDeviceGetTemperature` *(deprecated)* / **`nvmlDeviceGetTemperatureV`** | *"For all products."* `nvmlTemperatureSensors_t` has only `NVML_TEMPERATURE_GPU`, `NVML_TEMPERATURE_GPU_MAX`, `NVML_TEMPERATURE_COUNT` — **there is no memory or hotspot sensor type.** |
| Temperature thresholds (slowdown/shutdown/memory-max/GPU-max) | `nvmlDeviceGetTemperatureThreshold`, or `nvmlDeviceGetFieldValues` with `NVML_FI_DEV_TEMPERATURE_{SHUTDOWN,SLOWDOWN,MEM_MAX,GPU_MAX}_TLIMIT` on Ada+ | *"This API is no longer the preferred interface for retrieving … on Ada and later architectures … Support for reading these temperature thresholds for Ada and later architectures would be removed from this API in future releases."* |
| **Hotspot / junction temperature** | — | **Not exposed by NVML.** Tools that display "GPU Hot Spot" use NVAPI (`NvAPI_GPU_GetThermalSensors`-style calls); that surface is undocumented by NVIDIA and **unverified** here. Do not promise hotspot in the MVP. |
| Power draw | `nvmlDeviceGetPowerUsage` (mW), plus `nvmlDeviceGetPowerUsage_v2`, and field values `NVML_FI_DEV_POWER_AVERAGE` / `NVML_FI_DEV_POWER_INSTANT` | — |
| Utilisation | `nvmlDeviceGetUtilizationRates` (`nvmlUtilization_t {gpu, memory}`) | — |
| Clocks | `nvmlDeviceGetClockInfo` (`nvmlClockType_t`) | On Fermi, current P0 clocks can differ from max clocks. |
| Fan speed (percent of max noise-tolerance speed) | `nvmlDeviceGetFanSpeed`, `nvmlDeviceGetFanSpeed_v2` | *"For all discrete products with dedicated fans. … The reported speed is the intended fan speed. If the fan is physically blocked and unable to spin, the output will not match the actual fan speed."* Value **may exceed 100 %**. |
| Fan speed in RPM | `nvmlDeviceGetFanSpeedRPM` (`nvmlFanSpeedInfo_t`) | Maxwell or newer; again the *intended* speed, not a tachometer. |
| Fan count | `nvmlDeviceGetNumFans` | Needed to enumerate multi-fan cards. |
| Identity / memory | `nvmlDeviceGetName`, `nvmlDeviceGetUUID`, `nvmlDeviceGetMemoryInfo` | — |

**Caveat worth repeating in the UI:** NVML fan values are *commanded* speeds, not measured RPM. A blocked or failing fan still reports the intended value.

**The exact `nvml-wrapper` 0.13.0 surface** (method names on `Device` → NVML C function), which is what we would actually call:

| Need | Rust method | NVML C function |
|---|---|---|
| Temperature (edge only) | `temperature(sensor: TemperatureSensor) -> Result<u32>` | `nvmlDeviceGetTemperature` (**deprecated in NVML API 13**; superseded by `nvmlDeviceGetTemperatureV`) |
| Thresholds | `temperature_threshold(threshold_type) -> Result<u32>` | `nvmlDeviceGetTemperatureThreshold` |
| Power (mW) | `power_usage() -> Result<u32>` | `nvmlDeviceGetPowerUsage` |
| Utilisation | `utilization_rates() -> Result<Utilization>` (`{gpu, memory}`) | `nvmlDeviceGetUtilizationRates` |
| Fan % | `fan_speed(fan_idx) -> Result<u32>` | `nvmlDeviceGetFanSpeed_v2` |
| Fan RPM | `fan_speed_rpm(fan_idx) -> Result<u32>` | `nvmlDeviceGetFanSpeedRPM` |
| Fan count / allowed range | `num_fans()`, `min_max_fan_speed() -> Result<(u32,u32)>` | `nvmlDeviceGetNumFans`, `nvmlDeviceGetMinMaxFanSpeed` |
| **Set fan (duty %)** | `set_fan_speed(fan_idx, speed)` | **`nvmlDeviceSetFanSpeed_v2`** |
| **Restore automatic** | `set_default_fan_speed(fan_idx)` | `nvmlDeviceSetDefaultFanSpeed_v2` |
| Fan policy read/write | `fan_control_policy(fan_idx)`, `set_fan_control_policy(fan_idx, policy)` | `nvmlDeviceGetFanControlPolicy_v2`, `nvmlDeviceSetFanControlPolicy` |
| Clocks (MHz) / memory / name / UUID | `clock_info(clock_type)`, `memory_info()`, `name()`, `uuid()` | `nvmlDeviceGetClockInfo`, `nvmlDeviceGetMemoryInfo`, `nvmlDeviceGetName`, `nvmlDeviceGetUUID` |

Relevant wrapper enums: `TemperatureSensor` = **only `Gpu`** (no memory/hotspot variant exists); `TemperatureThreshold` = `Shutdown, Slowdown, MemoryMax, GpuMax, AcousticMin/Curr/Max, GpsCurr`; `Clock` = `Graphics, SM, Memory, Video`; `FanControlPolicy` = `TemperatureContinousSw = 0, Manual = 1` ([docs.rs](https://docs.rs/nvml-wrapper/latest/nvml_wrapper/enum_wrappers/device/index.html)).

**Two gaps that matter for a cooling app:**
1. **No hotspot / memory-junction temperature** — the safe wrapper exposes only `TemperatureSensor::Gpu`, and NVML's sensor enum has no hotspot member. Tools that display "GPU hot spot" use undocumented NVAPI calls; that surface is **unverified** and not something we can vendor (§4.3 has the licensing reason). Reaching NVML field values means dropping to `nvml-wrapper-sys` or `nvmlDeviceGetFieldValues`.
2. **Threshold reads move on Ada and later** — for `NVML_TEMPERATURE_THRESHOLD_SHUTDOWN`/`SLOWDOWN`/`MEM_MAX`/`GPU_MAX`, the reference says: *"This API is no longer the preferred interface … Support for reading these temperature thresholds for Ada and later architectures would be removed from this API in future releases. Please use `nvmlDeviceGetFieldValues` with `NVML_FI_DEV_TEMPERATURE_*` fields."*

### 3.3 Setting fan speed with NVML — what is and is not possible

- **Functions:** `nvmlDeviceSetFanSpeed_v2(device, fan, speed)` (duty 0–100 %), `nvmlDeviceSetDefaultFanSpeed_v2(device, fan)` (restore automatic), `nvmlDeviceSetFanControlPolicy(device, fan, policy)`. The reference's own warning on the setter: *"WARNING: This function changes the fan control policy to manual. It means that YOU have to monitor the temperature and adjust the fan speed accordingly. If you set the fan speed too low you can burn your GPU! Use `nvmlDeviceSetDefaultFanSpeed_v2` to restore default control policy."*
- **Documented eligibility:** *"For all cuda-capable discrete products with fans that are Maxwell or Newer."* `nvmlDeviceSetDefaultFanSpeed_v2` is *"For all cuda-capable discrete products with fans."* `nvmlDeviceSetFanControlPolicy` is *"For Maxwell™ or newer fully supported devices. Requires privileged user."* The only documented unsupported case is **"older than Maxwell"**.
- **There is no documented platform restriction.** No "Linux-only", "datacenter-only" or "not supported on Windows" wording appears on any fan setter — checked against the current Device Commands page (updated 2026-09-09), `nvml.h` 13.4.61 and the NVML Known Issues page. *(An earlier assumption in this project — that consumer GeForce fan setting is unavailable through NVML — is **not supported by the vendor documentation**: the API is documented for GeForce-class Maxwell+ products. Treat per-SKU behaviour as a runtime probe, not a design assumption, because NVIDIA publishes no fan-control support matrix per SKU — **unverified**.)*
- **Privilege is the hard constraint:** the whole **Device Commands** chapter says *"Each of these requires root/admin access. Non-admin users will see an `NVML_ERROR_NO_PERMISSION` error code when invoking any of these methods."* On Windows that means the writing process must be **elevated** — which is exactly the helper-process architecture in §9.
- **It does work in the field:** the MIT-licensed **green-curve** project (https://github.com/aufkrawall/green-curve) states *"Windows x64 and Linux x64 are both tested on real NVIDIA hardware, including VF-curve writes, power, and fan control"*, resolves `nvmlDeviceSetFanSpeed_v2` / `nvmlDeviceSetDefaultFanSpeed_v2` from `nvml.dll`, runs an **elevated background service**, and explicitly *"Does not ship NVIDIA driver binaries."*
- **But NVML has no fan _curve_ API — only per-fan duty % plus a policy enum** (`TEMPERATURE_CONTINOUS_SW` / `MANUAL`). A "curve" on NVML is a poll-and-set control loop that we own, with all the safety implications that carries.
- **The mainstream Windows stack does not use NVML for writes.** LibreHardwareMonitor writes NVIDIA fans through **NVAPI**: `NvApi.NvAPI_GPU_SetCoolerLevels(_handle, index, ref coolerLevels)` with `NvLevelPolicy.Manual`, and `NvAPI_GPU_ClientFanCoolersSetControl(...)`, loading `nvapi64.dll` from System32 with `DllImportSearchPath.System32` and resolving via `nvapi_QueryInterface`; it uses NVML only for reads (power, PCIe throughput). FanControl likewise uses **NvAPIWrapper** for *"Nvidia GPU fan control and sensor reading"*. NVAPI's fan **setting** surface is **undocumented**: NVIDIA's public "GPU Cooler Interface" page describes getting and setting the fan level but lists exactly one function, `NvAPI_GPU_GetTachReading`, and the Rust `nvapi-sys` crate files `NvAPI_GPU_SetCoolerLevels` / `GetCoolerSettings` under `gpu::cooler::private` marked *"Undocumented API"*.
- **MVP ruling:** implement NVML fan control as **duty-% with a runtime capability probe**, not as a vendor curve. Concretely: probe `num_fans()` and `min_max_fan_speed()`; on `NVML_SUCCESS` run our own control loop (target temperature → duty, with hysteresis and a slew limit); keep a **"driver auto" fallback** that always works; and call `set_default_fan_speed()` on exit, on panic/crash recovery, and on uninstall. Never present it as "hardware fan curve", and never make it the product's headline promise — and note the asymmetric stakes: if our loop dies while the duty is pinned low, the GPU can overheat, so the helper must be supervised and fail safe.

### 3.4 Where `nvml.dll` lives, and why we must not redistribute it

- **Locations, per NVIDIA's own NVML documentation:** *"The NVML library can be found at the following locations on Windows: Standard driver install: `%ProgramW6432%\NVIDIA Corporation\NVSMI\` — DCH driver install: `\Windows\System32` … To dynamically load NVML, call LoadLibrary with this path."* So: `C:\Windows\System32\nvml.dll` (DCH) or `C:\Program Files\NVIDIA Corporation\NVSMI\nvml.dll` (standard install). Since `nvml-wrapper` loads the bare name `"nvml.dll"` through the Windows loader search order, the System32 copy is what resolves. *(Whether current drivers still create the NVSMI folder, and the exact `nvidia-smi.exe` path per driver type, are **unverified**.)*
- NVML ships **with the display driver** ([developer.nvidia.com](https://developer.nvidia.com/nvidia-management-library-nvml)); the CUDA Toolkit ships only the headers/library. Since CUDA 13.1 the Windows driver is no longer bundled with the Toolkit ([CUDA install guide](https://docs.nvidia.com/cuda/cuda-installation-guide-microsoft-windows/)).
- **Redistribution: no.** The NVIDIA Driver License Agreement §2.7 states *"Except as expressly granted in this Agreement, you may not sell, rent, sublicense, distribute or transfer the SOFTWARE…"*, and §2.9 *"You may not use the SOFTWARE in any manner that would cause it to become subject to an open source software license…"*; the only distribution grant covers Linux kernel-module components ([NVIDIA driver licence](https://www.nvidia.com/en-us/drivers/nvidia-license/)). The CUDA EULA's redistributable list is **Attachment A**, and NVML is not on it; the fallback clause §1.2 forbids copying, distributing or creating derivative works of any portion of the SDK ([CUDA EULA](https://docs.nvidia.com/cuda/eula/index.html)).
- **So the design is non-negotiable:** load the driver-installed DLL at runtime with `libloading` (which `nvml-wrapper` already does), ship **no** NVIDIA binary, and treat "NVML not present" (AMD/Intel-only machine, no driver) as a normal state that degrades the UI gracefully rather than an error. *(This is technical research, not legal advice — the driver licence and CUDA EULA should be reviewed by counsel before shipping.)*

---

## 4. AMD

### 4.1 What exists

| Route | Platform | Telemetry (exact calls) | Fan control (exact calls) | Licence |
|---|---|---|---|---|
| **ADL (ADL2)** via `atiadlxx.dll` | Windows-only SDK (**18.1**, now labelled *"Now superceded by ADLX"* on GPUOpen) | `ADL2_Overdrive5/6/N_Temperature_Get`, `ADL2_Overdrive5/6_FanSpeed_Get`, `ADL2_Overdrive6_CurrentPower_Get`, `ADL2_OverdriveN_SystemClocks_Get`; bulk telemetry via `ADL2_New_QueryPMLogData_Get` (deprecated in 18.1) → `ADL2_Overdrive8_PMLog_ShareMemory_Read`. This is what LHM's `AmdGpu`/`AmdGpuGroup` use. | **Yes:** `ADL2_Overdrive5_FanSpeed_Set`, `ADL2_Overdrive5_FanSpeedToDefault_Set`, `ADL2_Overdrive6_FanSpeed_Set`, `ADL2_Overdrive6_FanSpeed_Reset`, `ADL2_Overdrive6_FanPWMLimitData_Set`, `ADL2_OverdriveN_FanControl_Set`, `ADL2_CustomFan_Set` | **Proprietary AMD EULA — blocking for us** (§4.3) |
| **ADLX** (successor SDK) | Windows | Performance-monitoring interfaces (`IPerformanceMonitoring`) | GPU tuning / fan interfaces; supports function-pointer (dynamic) initialisation | **Proprietary AMD EULA — blocking for us** (§4.3) |
| **ROCm SMI / `amdsmi`** | **Linux only.** AMD's ROCm-on-Windows component table lists System Management (ROCm SMI, RDC, rocminfo) for Linux and only `hipInfo` for Windows; the AMD SMI docs state *"This AMD SMI project supports Linux bare metal and Linux virtual machine guest environments."* | Yes on Linux | Limited | MIT (library) | 
| **`amdgpu` sysfs** | **Linux only**, and clean | `temp1_input`/`temp2_input`/`temp3_input` in **millidegrees C**, with `temp1_label="edge"`, `temp2="junction"`, `temp3="mem"` (temp2/3 on SOC15 dGPUs only); `power1_average`/`power1_input` in **microWatts**; `fan1_input` in **RPM**; `gpu_busy_percent`, `mem_busy_percent`, `pp_dpm_sclk`, `pp_dpm_mclk` | Yes: `pwm1` is **0–255** and `pwm1_enable` is *"0: no fan speed control, 1: manual fan speed control using pwm interface, 2: automatic"* — manual mode must be enabled first or the write is rejected. Modern RDNA also exposes a real `fan_curve` interface plus `fan_zero_rpm_enable` and `acoustic_limit_rpm_threshold` — prefer those to raw `pwm1`. | Kernel interface — no vendor SDK, no licence problem. **This is the clean AMD path for the Linux future.** Resolve the hwmon index by globbing `/sys/class/drm/card0/device/hwmon/hwmon*/` and matching `temp1_label`; never hardcode `hwmon0`. |
| **Rust crates** | — | — | — | **No maintained Windows binding exists.** `amd-adl-sys`, `adl-sys`, `libamdgpu`, `rocm_smi`, `radeon` do not exist on crates.io; the crate actually named **`adl` is unrelated** (phodal/adl, an architecture-description language, 0.1.0 MIT, 2021). What does exist: `amdgpu-sysfs` **0.21.0** — **LGPL-3.0-or-later**, 2026-05-16 (Linux; copyleft, so weigh it against Apache-2.0); `libdrm_amdgpu_sys` 0.9.0 MIT; `amdsmi`/`amdsmi-sys` 0.1.0 MIT (Linux-only); `rocm_smi_lib` 0.3.2 MIT OR Apache-2.0; `amdgpu` 1.0.12 (stale, 2023-11); `adlx` **0.0.0-alpha.1** MIT (2024-05-01, stale alpha, Traverse-Research/adlx-rs). On the NVIDIA side `nvapi-sys` 0.1.3 / `nvapi` 0.1.4 are MIT but effectively unmaintained (last release 2022-06). |

### 4.2 What LHM gives us for free

LHM's AMD backend is complete and MPL-2.0: `LibreHardwareMonitorLib/Hardware/Gpu/AmdGpuGroup.cs` and `AmdGpu.cs` create `SensorType.Fan` **and** `SensorType.Control` sensors, choose an Overdrive API level, and read temperature/power/clock/utilisation — all through `atiadlxx.dll`. **If our sidecar hosts LHM, we inherit AMD GPU fan control without ever touching an AMD SDK ourselves.** That is the single best argument for the sidecar architecture in this project.

### 4.3 The licensing constraint (this is the blocker)

The **ADL SDK EULA** ("SOFTWARE DEVELOPMENT KIT LICENSE AGREEMENT (ADL SDK)", Schedule A: *ADL SDK — libraries and header files (may be built into your software and distributed only as object code)*) says:

- §3(d): you may only *"distribute, in object code form the Distributed Software"*;
- §4(a): you must *"require all distributors and any/or third party end users to agree to use the Distributed Software in accordance with terms and conditions that are substantially similar to the terms and conditions contained in Schedule B"* (a restrictive EULA banning reverse engineering and derivative works);
- **§5(e): you may not *"modify or distribute the Source Code of any of the Distributed Software so that any part of it becomes subject to an Excluded license"* — and an "Excluded License" is defined as any licence that requires code to be *"disclosed or distributed in source code form"* or that grants *"the right to modify it"*.**
- §5(f): you may not *"publish the SDK or Documentation for others to use or copy"*.
- **Schedule A**: libraries and header files *"may be built into your software and distributed only as object code"*, while tools and documentation carry *"no redistribution of any kind"*.

Apache-2.0 grants exactly the rights that §5(e) calls "Excluded". Therefore ADL-derived material — **including a hand-written or generated `-sys` crate whose declarations are transcribed from ADL headers** — cannot be distributed inside an Apache-2.0 repository. Source: [ADL SDK EULA (verbatim licence text)](https://chromium.googlesource.com/chromiumos/overlays/portage-stable/+/b1ad75f083a189fdc9b70fc2e1daf08ecd45eada/licenses/AMD-ADL) and [ADL SDK EULA PDF](https://gpuopen-librariesandsdks.github.io/adl/ADL%20SDK%20EULA.pdf). The runtime `atiadlxx.dll` is installed by the driver in System32, so it can be `libloading`-ed exactly like NVML with nothing shipped — but the **headers, samples and documentation must stay out of the tree**.

The **ADLX SDK** licence (shipped as `ADLX SDK License Agreement.pdf` in the repo root, https://github.com/GPUOpen-LibrariesAndSDKs/ADLX) is the same shape plus more: it grants only a *"nonexclusive, royalty-free, **revocable**, non-transferable, non-assignable limited copyright license"*; §2(c) permits distribution *"solely in Object Code form"* subject to an end-user agreement meeting §3's requirements (including *"notification to the end user that Your Software is subject to a restricted license"* and *"AMD is a third party beneficiary"*); and **§4(f)** prohibits using the Licensed Materials *"in way that requires that the Licensed Materials or any portion thereof be licensed under a Free Software License"*, where §1.3 defines Free Software License as any licence requiring source disclosure, derivative works, **or that the result be *"redistributable at no charge"***. ADLX's Schedule A lists only the ADLX SDK. ADL/ADLX are **not OSI licences, are not redistributable as source, and are incompatible with shipping them inside an Apache-2.0 project.**

Two operational AMD caveats worth recording now, because they shape the feature matrix rather than the licence position: capability differs per Overdrive generation, and AMD's own OD8 sample says *"Before setting item's value, please first use `ADL2_Overdrive_Caps` ADL call to check its capability. Only the visible item could be set."* and *"The latest driver introduced Fan Curve feature and when it is available the legacy Fan controls will disabled."* So even inside LHM, AMD fan control is a capability-probing state machine across OD5/OD6/ODN/OD8 — and the elevation requirement and per-RDNA-generation support are **unverified** (AMD documents neither). LHM covers this for us; our code should simply not assume that an AMD `Control` sensor implies a working write path.

### 4.4 Verdict for an Apache-2.0 project

- **Do not add `ADL`/`ADLX` headers, bindings or wrappers to the workspace.** Not the C headers, not hand-written Rust equivalents transcribed from them (including any generated `-sys` crate), and not `ADLXWrapper` (Rem0o/ADLXWrapper ships **no LICENSE file** at any standard path → all rights reserved, which is workable for the closed-source FanControl but not for us).
- **Do not hand-roll NVAPI fan control either.** NVAPI's fan-setting surface is undocumented (public docs list only `NvAPI_GPU_GetTachReading`), and a hand-written FFI declaration derived from a reverse-engineered interface is both a maintenance and a legal risk. LHM already does it under MPL-2.0; consume LHM where a curve/policy write is genuinely required.
- **Delegate AMD GPU support to the MPL-2.0 LHM host.** LHM's own ADL usage is LHM's compliance problem (and its own licensing decision), not ours; we consume its output over IPC.
- **For NVIDIA, prefer the documented path:** NVML duty-% control via `nvml-wrapper` (documented for Maxwell+, needs elevation) as our own poll-and-set loop; keep the LHM host as the fallback/complement for cards or features NVML does not cover. The MIT-licensed **green-curve** project (https://github.com/aufkrawall/green-curve) is a useful, licence-compatible reference implementation of exactly this pattern on Windows, including running the writer as an elevated service and shipping no NVIDIA binaries.
- **Linux** should eventually use `amdgpu` sysfs directly — no vendor SDK, no licence issue — and that is a clean reason for the cross-platform architecture. Watch the crate choice there: `amdgpu-sysfs` 0.21.0 is **LGPL-3.0-or-later**.

---

## 5. Cross-platform Rust crates for telemetry

### 5.1 Crate comparison

| Crate | Latest | SPDX | MSRV | Maintenance | CPU name | CPU load | CPU freq (live) | Memory | Disks | Temps | SSD temp | Fan RPM |
|---|---|---|---|---|---|---|---|---|---|---|---|---|
| **`sysinfo`** | **0.39.6** (2026-07-09) | **MIT** | **1.95** (edition 2024) | Very active (2,740 ★, pushed 2026-09-10) | ✅ `Cpu::name()/brand()/vendor_id()` | ✅ per-core + `global_cpu_usage()` (refresh ≥2×) | ⚠️ **rated clock only** — `CallNtPowerInformation(ProcessorInformation).CurrentMhz` | ✅ total/used/free/available/swap (⚠️ *free* and *available* are the same value on Windows) | ✅ list, space, `DiskKind::{HDD,SSD,Unknown}` (via `DEVICE_SEEK_PENALTY_DESCRIPTOR`) | ⚠️ `Components` on Windows = WMI `root\WMI:MSAcpi_ThermalZoneTemperature` — usually empty/garbage | ❌ | ❌ |
| **`nvml-wrapper`** | **0.13.0** (2026-08-31) | MIT OR Apache-2.0 | 1.60.0 | Active (maintained by Cldfire) | ❌ | ❌ (GPU util ✅) | ❌ | ✅ (`memory_info`) | ❌ | ✅ GPU edge temp only | ❌ | ✅ GPU fan % and RPM (intended, not tach) |
| **`wmi`** (wmi-rs) | **0.18.4** (2026-03-27) | MIT OR Apache-2.0 | not declared (**unverified**; edition 2024 ⇒ ≥1.85 in practice) | Maintained, low cadence (124 ★) | ✅ via `Win32_Processor` | ✅ via `Win32_PerfFormattedData_*` | ❌ | ✅ `Win32_PhysicalMemory` | ✅ | ⚠️ only via ACPI thermal zone (usually useless) | ✅ via `MSFT_StorageReliabilityCounter.Temperature` | ❌ |
| **`raw-cpuid`** | **11.6.0** (2025-09-05) | **MIT** | not declared (**unverified**) | Active-ish (~1 release/yr, 182 ★) | ✅ vendor id + `ProcessorBrandString` (leaves `0x8000_0002..4`) | ❌ | ⚠️ rated: leaf `0x16` `processor_base_frequency()`, `tsc_frequency()`; **no MSR/APERF/MPERF** | ❌ | ❌ | ❌ (has `get_thermal_power_info()` bits, not a temperature) | ❌ | ❌ |

Sources: https://crates.io/api/v1/crates/sysinfo, https://docs.rs/sysinfo/latest/sysinfo/, https://github.com/GuillaumeGomez/sysinfo, https://github.com/GuillaumeGomez/sysinfo/blob/master/src/windows/cpu.rs, https://github.com/GuillaumeGomez/sysinfo/blob/master/src/windows/component.rs, https://github.com/GuillaumeGomez/sysinfo/blob/master/src/windows/disk.rs, https://github.com/ohadravid/wmi-rs, https://docs.rs/wmi/, https://github.com/gz/rust-cpuid, https://docs.rs/nvml-wrapper/latest/nvml_wrapper/.

**`wmi` crate dependency licences** (verified on crates.io): `windows` 0.62.2 and `windows-core` 0.100.0 (MIT OR Apache-2.0), `serde` (MIT OR Apache-2.0), `thiserror` 2.0.20 (MIT OR Apache-2.0), `futures` 0.3.34 (MIT OR Apache-2.0), `log` 0.4.34 (MIT OR Apache-2.0), optional `chrono` (MIT OR Apache-2.0) / `time` 0.3.55 (MIT OR Apache-2.0). One integration note: `wmi` 0.18.4 pins **`windows >=0.59, <0.63`**, which constrains the `windows` crate version the workspace can use alongside it.

### 5.2 WMI classes: which are real

| Class | Namespace | Verdict |
|---|---|---|
| `MSFT_PhysicalDisk` | `root\Microsoft\Windows\Storage` | **Reliable** — model, media/bus type, size, health. |
| `MSFT_StorageReliabilityCounter` | `root\Microsoft\Windows\Storage` | **Has `Temperature` (UInt8, °C)**, plus `TemperatureMax`, `Wear`, `PowerOnHours`, `ReadErrorsUncorrected`. Minimum Windows 8 / Server 2012 ([Microsoft Learn](https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/msft-storagereliabilitycounter)). Must be joined through an association class (`MSFT_PhysicalDiskToStorageReliabilityCounter`). Elevation requirement **unverified**. |
| `Win32_Fan` | `root\CIMV2` | **Useless** — `SetSpeed` "Not implemented"; only a requested `DesiredSpeed`; the current speed would come from `CIM_Tachometer`, which no in-box provider populates ([Learn](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-fan)). |
| `Win32_TemperatureProbe` | `root\CIMV2` | **Dead by Microsoft's own words** — *"current implementations of WMI do not populate the `CurrentReading` property. The `CurrentReading` property's presence is reserved for future use."* ([Learn](https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-temperatureprobe)). Never ship it. |
| `MSAcpi_ThermalZoneTemperature` | `root\WMI` | Occasionally returns one coarse chassis zone; usually empty. Not documented on current Microsoft Learn (**unverified** as to an official spec page). |
| `MSStorageDriver_ATAPISmartData` / `MSStorageDriver_FailurePredictStatus` | `root\WMI` | ATA/SATA only; silent on NVMe. Widely reported as absent on modern machines. |
| `Win32_VideoController` | `root\CIMV2` | Good for GPU name/VRAM/driver version; **no temperature, no fan, no load**. |

### 5.3 SSD/NVMe temperature on Windows — ranked

1. **WMI `MSFT_StorageReliabilityCounter.Temperature`** — documented, works for drives that report it, no driver, likely no elevation (**unverified**). Best first implementation.
2. **NVMe direct from the device** (documented on Learn, https://learn.microsoft.com/en-us/windows/win32/fileio/working-with-nvme-devices):
   - `IOCTL_STORAGE_QUERY_PROPERTY` with `PropertyId = StorageDeviceTemperatureProperty` → `STORAGE_TEMPERATURE_DATA_DESCRIPTOR` (signed °C per sensor index) ([STORAGE_PROPERTY_ID](https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ne-winioctl-storage_property_id));
   - `StorageDeviceProtocolSpecificProperty` with `ProtocolType=Nvme`, `DataType=NVMeDataTypeLogPage`, `ProtocolDataRequestValue=NVME_LOG_PAGE_HEALTH_INFO` (`ProtocolDataLength` ≥ 512) → the full SMART/Health log (temperature, wear, percentage used, unsafe shutdowns). Decode shown in Microsoft's own sample: `((Temperature[1] << 8) | Temperature[0]) - 273`.
   - Elevation requirement **unverified**.
3. **LHM storage support** (MPL-2.0, backed by `DiskInfoToolkit` MPL-2.0) — free if we already host LHM, and it is what LHM's own Storage sensors use.
4. **`smartctl` (smartmontools, GPL-2.0)** as an external process — only if broad USB-bridge coverage is needed; adds a GPL binary redistribution obligation, a brittle CLI parse, and likely elevation.
5. **`nvme` crate on crates.io — unusable: every published version (0.1.0–0.2.2) is yanked** (verified). Other NVMe crates are Linux-kernel-oriented.

---

## 6. Tauri v2 vs .NET 8 + WinUI 3

### 6.1 Licensing

| Option | Code licence | Runtime/redistributable |
|---|---|---|
| **Tauri v2** | `tauri` 2.11.5: **Apache-2.0 OR MIT**; `tauri-build` 2.6.3, `wry` 0.57.0 (Apache-2.0 OR MIT), `tao` 0.37.0 (Apache-2.0), `tray-icon` 0.24.2 (MIT OR Apache-2.0), `@tauri-apps/cli` 2.11.4 and `@tauri-apps/api` 2.11.1 (**Apache-2.0 OR MIT**) | **WebView2 Runtime** — Microsoft proprietary, but *distribution is explicitly supported*: the Evergreen bootstrapper (~2 MB) or standalone installer may be linked or bundled, and the Runtime ships as part of Windows 11 ([Microsoft Learn](https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution)). Tauri defaults to `webviewInstallMode: downloadBootstrapper`. |
| **.NET 8/10 + WinUI 3** | .NET is **MIT** ([dotnet/runtime LICENSE.TXT](https://github.com/dotnet/runtime/blob/main/LICENSE.TXT)); the Windows App SDK **repo** is MIT ([LICENSE](https://github.com/microsoft/WindowsAppSDK/blob/main/LICENSE)) | ⚠️ **The shipped `Microsoft.WindowsAppSDK` NuGet package is not MIT.** It sets `requireLicenseAcceptance=true` with a file licence: *"MICROSOFT SOFTWARE LICENSE TERMS — MICROSOFT WINDOWS APP SDK"* — develop/test **"solely for use on Windows"**, a data-collection clause, a DISTRIBUTABLE CODE section requiring you to *"add significant primary functionality"*, to pass protective terms to end users, and to **indemnify and defend Microsoft**, a ban on distributing the code under a licence requiring source disclosure, plus US arbitration and a $5 damages cap. |

Also note: **.NET 8 leaves support on 2026-11-11**; if a .NET sidecar is used, target **.NET 10** (LTS to 2028-11-15) ([.NET lifecycle](https://learn.microsoft.com/en-us/lifecycle/products/microsoft-net-and-net-core)). Tauri's trademark rules are separate from its code licence: TAURI is a registered trademark, forks must strip logos, and *"Care must be taken, not to ship your application with the default ICON."* (https://v2.tauri.app/about/trademark/).

### 6.2 Windows hardware access

- **.NET:** can host `LibreHardwareMonitorLib` **in-process with zero IPC** and P/Invoke Win32 directly. This is a genuine engineering advantage: no IPC protocol, no serialization, no second process to supervise, lowest latency for a control loop.
- **Tauri:** needs a sidecar (`bundle.externalBin`, https://v2.tauri.app/develop/sidecar/) or FFI/in-process CLR hosting. Every hardware read crosses a process boundary; every write goes through an elevated helper.
- Counter-weight: **the sidecar boundary is a feature for licensing and safety.** It keeps MPL-2.0 and GPL-2.0 components out of our binary, keeps driver crashes from killing the UI, and lets the elevated component be tiny and auditable.

### 6.3 Bundle size, cross-platform future, contributors, frontend

| Dimension | Tauri v2 + React | .NET + WinUI 3 |
|---|---|---|
| Installer size | Small: Tauri's own claim is *"a minimal Tauri app can be less than 600KB"* (https://v2.tauri.app/start/); realistic app ~5–15 MB + WebView2 bootstrapper (or 0 if present). WebView2 `offlineInstaller` ≈ +127 MB, `fixedRuntime` ≈ +180 MB if we ever bundle it. | Much larger structurally: .NET runtime + Windows App SDK framework package as loose files next to the exe. `dotnet publish` **cannot** produce a single-file EXE for WinUI 3 ([Learn](https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/self-contained-deploy/deploy-self-contained-apps)). An official MB figure for self-contained WinUI 3 is **unverified**. |
| Cross-platform future | **Yes** — macOS/Linux today (Linux is where `amdgpu` sysfs gives clean AMD fan control). | **No** — Windows only, forever. |
| AI/community contribution | Rust + React/TS + JSON config; a contributor needs only a Rust toolchain and Node. Large, public, well-documented plugin ecosystem (`plugins-workspace`). | C#/XAML + Windows SDK + Visual Studio workloads; contributors wanting to redistribute must also read the Windows App SDK EULA. |
| Frontend ecosystem | React 19.3.0 (MIT), Vite 8.3.0 (MIT), TypeScript 7.0.2 (Apache-2.0) — charts, curve editors, component libraries, HMR. | XAML: better native feel and no webview, but a much smaller pool of contributors for a *fan-curve UI* and no shareable component ecosystem. |
| Hardware control loop | Needs IPC + elevated helper; adds ~1–5 ms of latency (**unverified** estimate) and a supervision problem. | In-process, lower latency, simpler control loop — the strongest argument for .NET. |

### 6.4 Recommendation

**Keep Tauri v2 + React, and put the hardware layer behind a separate, elevated host process that itself hosts LibreHardwareMonitorLib.**

Reasons, in priority order:

1. **Licence hygiene.** ADL/ADLX cannot be shipped in an Apache-2.0 repo (§4.3). A .NET app would be *tempted* to link them for AMD GPU control; an LHM-hosting sidecar gives us AMD GPU fan control under MPL-2.0 instead, with no AMD SDK in our tree.
2. **Cross-platform future is real and cheap.** macOS (dev machine!) and Linux are reachable with the same Rust core; Linux `amdgpu` sysfs is a licence-free AMD fan-control path.
3. **The privileged surface stays small.** The elevated component is a sensor/fan daemon; the UI, webview and network stack stay unelevated — which matters more every year on Windows (see §9.4 Administrator protection).
4. **Contribution friction is lower** with Rust + React + Vite than with C#/XAML/VS workloads, and this project's growth strategy is (per the brief) open-source community.

**Tradeoffs we are accepting by needing a .NET sidecar:**

- **Two toolchains in CI** (Rust + .NET SDK 10) and a second runtime to publish; a self-contained, single-file .NET sidecar is ~15–70 MB (**unverified** figure).
- **A real IPC contract to design, version and test** (recommend: length-prefixed JSON/MsgPack over a named pipe with ACLs restricted to the interactive user; `tokio::net::windows::named_pipe`).
- **Supervision complexity**: the UI must detect a dead/hung helper, restart it, and — critically — **fail safe**, restoring BIOS/default fan control rather than leaving a fan stuck at a low duty.
- **.NET 8 is the trap**: it retires 2026-11-11, so pin `net10.0` (which `LibreHardwareMonitorLib` supports) and do not copy `lhm-service`'s .NET 8 prerequisite blindly.
- **If we go the in-process route instead** (`netcorehost` 0.22.0), we lose process isolation and make elevation all-or-nothing; that is a later optimisation, not a starting point.

---

## 7. License matrix

Legend for "OK to ship in an Apache-2.0 product?": **Yes** = no obligations beyond attribution where applicable; **Care** = usable but with a defined obligation; **No** = cannot be redistributed/shipped in our Apache-2.0 artefact.

### 7.1 Hardware/sensor components (the interesting ones)

| Component | Version | SPDX / licence | Commercial use + redistribution | Obligations & flags |
|---|---|---|---|---|
| **LibreHardwareMonitorLib** | 0.9.6 (stable), 0.9.7-pre736 (prerelease) | **MPL-2.0** | **Care** — yes | File-level copyleft: modified LHM files must be republished under MPL-2.0 (and MPL-2.0 §3.2 patent grant applies). Unmodified NuGet consumption from a separate process = no effect on our code. Ship the MPL text + notice. |
| **LibreHardwareMonitor GUI zip** (if we ever redistribute it) | 0.9.6 / nightly | MPL-2.0 + bundled third parties | **Care** | Pulls in Aga.Controls (**BSD-3-Clause**), **DiskInfoToolkit & RAMSPDToolkit-NDD (MPL-2.0)**, **HidSharp (Apache-2.0)**, `System.Management`/`System.IO.Ports`/`Microsoft.Win32.Registry`/`System.Threading.AccessControl` (**MIT**), `Mono.Posix.NETStandard` (link-only licence URL), `Microsoft.Windows.CsWin32` (**MIT**, `PrivateAssets=all` = build-time only), **PawnIO.Modules (LGPL-2.1)**, and an embedded **`PawnIO_setup.exe` (GPL-2.0-or-later)**. Redistributing the zip means honouring all of them. |
| **PawnIO kernel driver** | 0.2.x / 2.x installed by `PawnIO_setup.exe` | **GPL-2.0-or-later** + linking exception | **Care/No for bundling** | Its README: *"Linking PawnIO statically or dynamically with other modules is making a combined work based on PawnIO. Thus, the terms and conditions of the GNU General Public License cover the whole combination."* The exception permits independent modules that talk **only over the device IOCTL interface**; it *"does not include programs that communicate with PawnIO over the Pawn interface"* (i.e. loaded modules must be GPL/LGPL-2.1-compatible). **Bundling the installer = redistributing GPL software** (source offer, licence text, no extra restrictions). **Recommendation: detect + deep-link, do not bundle in v1.** |
| **PawnIO.Modules** | 0.2.11 (as embedded by LHM) | **LGPL-2.1** | **Care** | Only relevant if we ship modules; §6 LGPL requires allowing relinking (ship module sources/objects under LGPL-2.1). |
| **WinRing0 / WinRing0x64.sys** | legacy | Custom BSD-style (**exact text unverified**) | **No** | **Do not ship.** Flagged as a vulnerable driver (CVE-2020-14979), Defender detection `VulnerableDriver:WinNT/Winring0`, on Microsoft's recommended blocklist, on by default since Win11 2022. Reputational and functional poison. |
| **NVIDIA NVML (`nvml.dll`)** | driver-provided (NVML docs Release 615; `nvml.h` 13.4.61) | NVIDIA proprietary | **Yes — by not shipping it** | Location per NVIDIA: `%ProgramW6432%\NVIDIA Corporation\NVSMI\` (standard driver) or `\Windows\System32` (DCH). `nvml-wrapper` loads the bare name `nvml.dll` via `libloading`, so we redistribute nothing. **Redistribution is not permitted:** NVIDIA Driver License Agreement §2.7 (*"you may not sell, rent, sublicense, distribute or transfer the SOFTWARE"*) and §2.9 (*"You may not use the SOFTWARE in any manner that would cause it to become subject to an open source software license"*); NVML is not on the CUDA EULA's Attachment A redistributable list, and CUDA EULA §1.2 forbids copying/distributing any portion of the SDK. **Do not bundle `nvml.dll`.** |
| **NVAPI (`nvapi64.dll`)** | driver-provided | NVIDIA proprietary; fan-**setting** API undocumented (public docs list only `NvAPI_GPU_GetTachReading`; `nvapi-sys` files `NvAPI_GPU_SetCoolerLevels` under `gpu::cooler::private`) | **Good — if we do not write FFI for it** | Loading the driver's DLL at runtime mirrors the NVML posture. Hand-writing NVAPI FFI declarations for fan control is a reverse-engineering/maintenance risk we should avoid; LHM already carries that (MPL-2.0). Prefer documented NVML duty-% control and consume LHM for anything NVML cannot do. |
| **NvAPIWrapper** (what FanControl uses) | — | **LGPL-3.0** | **No for us** | LGPL-3.0 brings link/relink obligations we do not want in a permissive workspace; it is also C#. |
| **green-curve** (reference implementation, not a dependency) | — | **MIT** | Yes | Windows + Linux NVIDIA fan control via runtime-loaded `nvml.dll`, driven by an elevated service, explicitly *"Does not ship NVIDIA driver binaries."* Useful as a licence-compatible architectural reference. |
| **nvidia `nvapi-sys` / `nvapi` crates** | 0.1.3 / 0.1.4 | MIT | Care | Effectively unmaintained (last release 2022-06). Declares undocumented NVAPI entry points — see the NVAPI row above. |
| **AMD ADL SDK** (`atiadlxx.dll` + headers) | SDK 18.1 (now *"superceded by ADLX"*) | **Proprietary AMD EULA** ("ADL SDK") | **NO — blocker** | §3(d) object-code-only distribution; §4(a) requires a restrictive end-user EULA; **§5(e)** forbids making any SDK source subject to an "Excluded License" (source disclosure or the right to modify) — Apache-2.0 **is** such a licence; §5(f) forbids *"publish the SDK or Documentation for others to use or copy"*; Schedule A gives tools/docs *"no redistribution of any kind"*. Cannot appear in our repo, binaries or installer. |
| **AMD ADLX SDK** | SDK v1.5 manuals | **Proprietary AMD EULA** ("ADLX SDK") | **NO — blocker** | Revocable, non-transferable licence; §2(c) object-code-only; §3 mandatory restrictive EULA (incl. "AMD is a third party beneficiary"); **§4(f)** forbids use *"in way that requires that the Licensed Materials … be licensed under a Free Software License"*, and §1.3 includes licences requiring the result be *"redistributable at no charge"*. Same conclusion. |
| **ADLXWrapper** (Rem0o) | — | **No LICENSE file at any standard path → all rights reserved** | **No** | Closed-source; and it wraps ADLX, so it inherits the ADLX problem regardless. |
| **`amdgpu-sysfs` crate** (Linux) | 0.21.0 | **LGPL-3.0-or-later** | **Care** | The practical Linux AMD interface, but copyleft — dynamic linking (a normal Rust dependency) is acceptable under LGPL, yet it complicates a strictly Apache-2.0-only distribution story. Consider reading sysfs directly instead. |
| **ROCm SMI (`amdsmi` / `librocm_smi64`)** | — | MIT (library) | Care | Windows support **unverified**; not part of the Windows-first plan. |
| **OpenRGB** | 0.9.x | **GPL-2.0** | **No** | GPL-2.0 is incompatible with shipping inside an Apache-2.0 product (and with a permissive-only workspace). Useful as *documentation* (its SMBus/PawnIO notes are the best public description of the admin requirement); do not copy code or link it. |
| **G-Helper** (ASUS laptop control) | — | **GPL-3.0** | **No** | Same rule. Reference reading only. |
| **FanControl (Rem0o)** | V238+ | **no LICENSE file; "Sources for this software are closed"** | **No** | Freeware; exact licence **unverified**. Cannot reuse any code. Its README is, however, an excellent citable source of ecosystem behaviour (driver migration, NVAPI/ADLX usage, service mode). |
| **smartmontools / `smartctl`** | 7.x | **GPL-2.0** | **Care** | Invoking an external GPL binary is aggregation, not linking, but redistributing it means GPL obligations (source offer) and a hard external dependency. Prefer the WMI/NVMe paths. |
| **silicon-monitor** (crate) | 5.1.0 | **AGPL-3.0-or-later** | **No** | AGPL is a hard no for a desktop product. |
| **DiskInfoToolkit** | 2.1.4 | **MPL-2.0** | Care | Transitive dependency of LibreHardwareMonitorLib; same MPL reasoning. |
| **RAMSPDToolkit-NDD** | 1.6.1 | **MPL-2.0** | Care | Same. |
| **HidSharp** | 2.6.4 | **Apache-2.0** (verified in the package's `LICENSE.txt`) | Yes | Same licence family as our project — no friction. |
| **Microsoft.Windows.CsWin32** | 0.3.333 | **MIT** | Yes | `PrivateAssets=all` in LHM's csproj → build-time only, not redistributed. |

### 7.2 Rust / frontend / build tooling

| Component | Version | SPDX | Apache-2.0 commercial use + redistribution | Obligations / flags |
|---|---|---|---|---|
| `tauri` | 2.11.5 | Apache-2.0 OR MIT | Yes (pick Apache-2.0) | Trademark conditions are separate (no default icon, no implied endorsement, forks strip logos). |
| `tauri-build` | 2.6.3 | Apache-2.0 OR MIT | Yes | — |
| `wry` | 0.57.0 | Apache-2.0 OR MIT | Yes | — |
| `tao` | 0.37.0 | **Apache-2.0** (only) | Yes | Apache-2.0 matches our outbound licence exactly. |
| `tray-icon` | 0.24.2 | MIT OR Apache-2.0 | Yes | — |
| `tauri-plugin-autostart` | 2.5.1 | Apache-2.0 OR MIT | Yes | Pulls `auto-launch` 0.6.0 (MIT). |
| `@tauri-apps/cli` / `@tauri-apps/api` | 2.11.4 / 2.11.1 | Apache-2.0 OR MIT | Yes | Build-time / bundled JS. |
| `tokio` | 1.53.1 | **MIT** | Yes | — |
| `serde` | 1.0.229 | MIT OR Apache-2.0 | Yes | — |
| `serde_json` | 1.0.151 | MIT OR Apache-2.0 | Yes | — |
| `serde-saphyr` (recommended YAML) | 1.2.0 | MIT OR Apache-2.0 | Yes | Denies `unsafe`; pure Rust; MSRV 1.89. |
| `serde_norway` (fallback YAML) | 0.9.42 | MIT OR Apache-2.0 | Yes | Pulls `unsafe-libyaml-norway` (auto-translated C, heavy `unsafe`); dormant upstream. |
| `serde_yaml_ng` (not recommended) | 0.10.0 | **MIT** (0.9.x were dual) | Yes | Frozen upstream; pulls `unsafe-libyaml` (unmaintained). |
| `serde_yaml` | 0.9.34+deprecated | MIT OR Apache-2.0 | Yes | **Archived** by dtolnay — do not adopt. |
| `ureq` | 3.4.1 | MIT OR Apache-2.0 | Yes | Direct deps: `base64`, `log`, `percent-encoding`, `ureq-proto`, `utf8-zero`; TLS is optional (`rustls`/`native-tls` behind features). For our localhost-only LHM HTTP calls, **disable TLS** to shrink the dependency tree and avoid shipping a TLS stack. Note: for the *sidecar* transport, prefer a named pipe over HTTP. |
| `clap` | 4.6.6 | MIT OR Apache-2.0 | Yes | MSRV 1.85. |
| `tracing` | 0.1.44 | **MIT** | Yes | — |
| `chrono` | 0.4.45 | MIT OR Apache-2.0 | Yes | — |
| `dirs` | 7.0.0 | MIT OR Apache-2.0 | Yes | Repo moved to Codeberg (`codeberg.org/dirs/dirs-rs`). |
| `windows-service` | 0.8.1 | MIT OR Apache-2.0 | Yes | Needed only if we adopt the service helper pattern. |
| `runas` | 1.2.0 | **Apache-2.0** | Yes | Windows elevation helper; GUI-mode only, no output capture, cwd forced to `system32` (**verified in its docs**). |
| `libloading` | 0.9.0 | **ISC** | Yes | ISC is permissive; attribution only. Used by `nvml-wrapper`. |
| `netcorehost` | 0.22.0 | **MIT** | Yes | Only if we host the CLR in-process. |
| `sysinfo` | 0.39.6 | **MIT** | Yes | MSRV 1.95 — sets our workspace MSRV. |
| `wmi` (+ `windows`, `windows-core`, `thiserror`, `futures`, `log`, `time`) | 0.18.4 | MIT OR Apache-2.0 (all verified) | Yes | `windows` version pin `>=0.59,<0.63` is a workspace constraint. |
| `raw-cpuid` | 11.6.0 | **MIT** | Yes | — |
| `nvml-wrapper` / `nvml-wrapper-sys` | 0.13.0 / 0.10.0 | MIT OR Apache-2.0 | Yes | Runtime-loads NVIDIA's `nvml.dll`; we ship no NVIDIA binary. |
| `lhm-service` / `lhm-client` / `lhm-sys` | 0.2.0 / 0.3.0 / 0.1.1 | **MIT** | Yes | Requires .NET 8 SDK + VS2022 (C++ workload) to build `lhm-sys`. Note `lhm-client` 0.4.0 exists but is **yanked**; `lhm-sys` stays at 0.1.1. |
| **React / React DOM** | 19.3.0 | **MIT** | Yes | — |
| **Vite** | 8.3.0 | **MIT** | Yes | Build-time only. |
| **TypeScript** | 7.0.2 | **Apache-2.0** | Yes | Build-time only. |
| `.NET runtime` (sidecar) | **10.0** (LTS) | **MIT** | Yes | .NET 8 retires 2026-11-11 — use 10. Self-contained publish redistributes the MIT runtime. |
| **Windows App SDK / WinUI 3** (not chosen) | 2.4.0 | Repo MIT, **NuGet redistributable = proprietary Microsoft EULA** | **Care** | Indemnity + "significant primary functionality" + no source-disclosure licensing + arbitration clause. Another reason not to take the WinUI path. |
| **WebView2 Runtime** | Evergreen | Microsoft proprietary (distribution permitted) | Yes | Ship the bootstrapper or standalone installer, or rely on it being present (included in Windows 11). |
| **NSIS** (installer) | 3.x | zlib/libpng (+ bzip2 licence; **CPL-1.0** for the LZMA module, with an explicit linking exception) | Yes | Attribution/notice obligations only; the LZMA exception means our installer is not CPL-encumbered ([NSIS Appendix I](https://nsis.sourceforge.io/Docs/AppendixI.html)). |
| **WiX Toolset v3** (MSI) | 3.14.x | **MS-RL** (Microsoft Reciprocal License) | Yes | **Build-time tool only**: MS-RL is file-level reciprocal, so it obliges nothing unless we redistribute WiX source/binaries or ship modified WiX files. If we ship a custom `.wxs` derived from Tauri's template, that file is Tauri's (MIT/Apache-2.0), not MS-RL ([WiX v3 licence](https://github.com/wixtoolset/web/blob/master/src/Docusaurus/versioned_docs/version-v3/main/license.md)). |

### 7.3 Things that block or complicate an Apache-2.0 Windows installer

1. **BLOCKER — AMD ADL / ADLX SDKs.** Their EULAs explicitly forbid subjecting SDK source to a licence that requires source disclosure or the right to modify (ADL §5(e), ADLX §4(f)), forbid publishing the SDK for others to copy (ADL §5(f)), permit object-code-only distribution, and mandate restrictive end-user terms. They must not appear in our repository, our binaries or our installer, in source or object form — including transcribed header content in a generated `-sys` crate. **Mitigation: get AMD GPU data and fan control exclusively from the MPL-2.0 LHM host.**
2. **CONFIRMED NO — redistributing `nvml.dll`** (or any NVIDIA driver component). NVIDIA Driver License Agreement §2.7/§2.9 and the CUDA EULA (NVML is not on Attachment A) prohibit it. **Mitigation: `libloading` the driver-installed copy; ship nothing.**
3. **COMPLICATION — PawnIO (GPL-2.0-or-later) and its LGPL-2.1 modules.** Bundling `PawnIO_setup.exe` makes us a GPL distributor of a kernel driver. Manageable (the source is public) but requires a deliberate, documented decision, a source offer, and a NOTICE. **Mitigation for v1: detect-and-prompt; let the user install PawnIO from pawnio.eu.**
4. **COMPLICATION — MPL-2.0 file-level copyleft (LHM, DiskInfoToolkit, RAMSPDToolkit).** Fine as long as we do not modify those files, or we republish modifications under MPL-2.0. Keep the sidecar code in its own repository/directory with its own MPL notice so a future patch cannot accidentally contaminate the Apache-2.0 core.
5. **COMPLICATION — the vulnerable-driver blocklist.** Any installer that drops WinRing0 will be blocked by Defender and by Microsoft's recommended blocklist on default Windows 11 configurations. We ship none.
6. **NOTICE TIER — attribution.** We must ship a THIRD-PARTY notices file (MPL-2.0 texts, Apache-2.0/MIT texts, BSD for Aga.Controls if we ever repackage LHM's GUI, NSIS zlib/bzip2, WebView2 terms). Automate it in CI with `cargo-deny`/`cargo-about` plus an npm licence scan.
7. **NOT A BLOCKER — WiX (MS-RL), NSIS (zlib/CPL-1.0+LZMA exception), WebView2 (Microsoft terms), .NET (MIT), green-curve-style NVML control (MIT).** All are either build-time-only or explicitly redistributable; none impose conditions on our source licence.

---

## 8. Maintained YAML crate for Rust (2026)

`serde_yaml` is archived (dtolnay, last release 0.9.34+deprecated on 2024-03-25), so the field is forks.

| Crate | Latest | SPDX | MSRV | Last release | Upstream activity | Notes |
|---|---|---|---|---|---|---|
| `serde_yaml` | 0.9.34+deprecated | MIT OR Apache-2.0 | 1.64 | 2024-03-25 | **ARCHIVED** | Baseline only; do not adopt. |
| `serde_yaml_ng` | 0.10.0 | **MIT** | 1.64 | 2024-05-26 | Repo `acatton/serde-yaml-ng`; effectively frozen (no commits since early 2026), 3 releases ever, ~113 ★ | Depends on `unsafe-libyaml` (unmaintained, auto-translated C). |
| `serde_norway` | 0.9.42 | MIT OR Apache-2.0 | 1.71.1 | 2024-12-21 | Repo `cafkafk/serde-yaml`; dormant since the 2024-12 release, ~56 ★ | Behaviour-compatible `serde_yaml` drop-in; **explicitly recommended by RUSTSEC** as a safe replacement; depends on `unsafe-libyaml-norway`. |
| `serde_yaml_bw` | 2.5.8 | MIT OR Apache-2.0 | none declared | 2026-09-07 | Very active (~24 releases), ~37 ★ | Forked lineage; **its own docs point users at `serde-saphyr`**. |
| **`serde-saphyr`** | **1.2.0** | **MIT OR Apache-2.0** | **1.89** | **2026-08-30** | Very active (~36 releases, pushed 2026-09-09), ~220 ★ | **Pure Rust on `granit-parser` 1.2.1; `unsafe` forbidden**; panic-free design; fuzz + Miri CI; serde derive; configurable duplicate-key policy, indentation enforcement, budget limits, precise error snippets. |
| `saphyr` | 0.0.12 | MIT OR Apache-2.0 | 1.85.0 | 2026-08-18 | Active | Parser only — **no serde integration of its own** (use `serde-saphyr`). |
| `yaml-rust2` | 0.13.0 | MIT OR Apache-2.0 | 1.85.0 | 2026-09-11 | Active | README states it *"will receive only basic maintenance"* and redirects to `saphyr`; parser only. |
| `serde_yml` | 0.0.13 | MIT OR Apache-2.0 | 1.85.0 | 2026-05-27 | **ARCHIVED** | **RUSTSEC-2025-0068**: `ser::Serializer.emitter` can segfault → unsound; project archived after the report. **Never use.** |
| `figment` | 0.10.19 | MIT OR Apache-2.0 | — | 2024-05-17 | Stale | Its `yaml` feature still depends on the **archived** `serde_yaml ^0.9`. Avoid. |
| `config` | 0.15.25 | MIT OR Apache-2.0 | 1.85.0 | 2026-06-26 | Active | Uses `yaml-rust2 ^0.11` (not serde_yaml) — viable if a layered config system is wanted. |
| `toml` (the alternative) | — | MIT OR Apache-2.0 | — | — | Active | Zero YAML-ecosystem risk. |

Evidence: https://github.com/acatton/serde-yaml-ng, https://github.com/cafkafk/serde-yaml, https://github.com/bourumir-wyngs/serde-saphyr, https://github.com/bourumir-wyngs/serde-yaml-bw, https://github.com/saphyr-rs/saphyr, https://github.com/dtolnay/serde-yaml, https://github.com/rustsec/advisory-db/blob/main/crates/serde_yml/RUSTSEC-2025-0068.md, https://crates.io/api/v1/crates/serde-saphyr.

### Recommendation

**Use `serde-saphyr` 1.2.0 (MIT OR Apache-2.0), pinned to the exact version.**

- It is the only actively maintained, **pure-Rust, `unsafe`-denied** option, whereas every `serde_yaml` descendant carries `unsafe-libyaml`/`unsafe-libyaml-norway` — auto-translated C with heavy `unsafe`, and in `serde_yaml_ng`'s case an upstream that has effectively stopped.
- Licence matches our outbound Apache-2.0 exactly.
- Quality-of-error features matter here: fan-curve files are hand-edited, and duplicate-key detection plus indentation enforcement turn silent misconfiguration into a clear error.
- **Residual risks to manage:** it is effectively single-maintainer (the same author owns `serde_yaml_bw` and the `granit-parser` fork of the saphyr parser), so (a) pin exactly, (b) wrap every YAML call in one `config` module exposing `load_config`/`save_config` so swapping parsers is a one-file change, (c) run `cargo-deny`/`cargo-audit` in CI.
- **Fallback:** `serde_norway` 0.9.42 — RUSTSEC-endorsed, dual MIT/Apache-2.0, behaviour-compatible with `serde_yaml`; accept its dormancy and its `unsafe` C-derived parser knowingly.
- **Escape hatch:** if YAML comments turn out not to be essential, `toml` removes this entire risk class.

---

## 9. Windows autostart, tray and installer (Tauri v2)

### 9.1 Versions and licensing

| Artifact | Latest | Licence | MSRV |
|---|---|---|---|
| `tauri` crate | **2.11.5** | Apache-2.0 OR MIT | 1.77.2 |
| `tauri-build` | 2.6.3 | Apache-2.0 OR MIT | 1.77.2 |
| `tauri-plugin-autostart` | **2.5.1** | Apache-2.0 OR MIT | 1.77.2 |
| `@tauri-apps/cli` | **2.11.4** | Apache-2.0 OR MIT | — |
| `@tauri-apps/api` | 2.11.1 | Apache-2.0 OR MIT | — |
| `@tauri-apps/plugin-autostart` | 2.5.1 | MIT OR Apache-2.0 | — |

The Rust crate and the CLI are **not the same version number** — pin them independently.

### 9.2 Tray icon and staying resident

- Enable the Cargo feature: `tauri = { version = "2.11", features = ["tray-icon"] }` (https://v2.tauri.app/learn/system-tray/). `tauri::tray` is documented as *"Available on desktop and crate feature tray-icon only"* and exposes `TrayIcon`, `TrayIconBuilder`, `TrayIconId`, `MouseButton`, `MouseButtonState`, `TrayIconEvent` (https://docs.rs/tauri/latest/tauri/tray/index.html). Windows is supported; the underlying `tray-icon` crate notes that on Windows an event loop must be running on the thread — Tauri's own loop satisfies that.
- Typical wiring: `TrayIconBuilder::new().icon(app.default_window_icon().unwrap().clone()).menu(&menu).show_menu_on_left_click(false).on_tray_icon_event(...)`.
- **Hiding the window without exiting takes two separate APIs**, and both are needed:
  1. `WindowEvent::CloseRequested { api, .. }` → `api.prevent_close()` plus `window.hide()` (https://docs.rs/tauri/latest/tauri/enum.WindowEvent.html);
  2. `RunEvent::ExitRequested { api, .. }` → `api.prevent_exit()` — with all windows hidden, Tauri otherwise exits (https://docs.rs/tauri/latest/tauri/enum.RunEvent.html). Note `prevent_exit` is ignored for programmatic exit, so a real **Quit** menu item must call `app.exit(0)`.

### 9.3 Autostart

- Init: `app.handle().plugin(tauri_plugin_autostart::init(MacosLauncher::LaunchAgent, Some(vec!["--tray"])));` or the builder form `tauri_plugin_autostart::Builder::new().args([...]).app_name("OpenHardwareOS").build()` (https://v2.tauri.app/plugin/autostart/, https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/autostart/README.md).
- **What it writes on Windows:** the plugin delegates to the `auto-launch` crate 0.6.0 (MIT), which writes a **registry `Run` value** under `SOFTWARE\Microsoft\Windows\CurrentVersion\Run` — **HKLM** (needs admin) with a silent fallback to **HKCU** — and touches `…\Explorer\StartupApproved\Run` so Task Manager shows the entry as enabled (https://github.com/zzzgydi/auto-launch).
- **It cannot register an elevated autostart.** A `Run` entry is launched by Explorer with the user's normal token (behaviour is universally observed but **I could not find a verbatim Microsoft statement** confirming the token — **unverified**). Microsoft's supported mechanism for elevated autostart is **Task Scheduler with `/rl HIGHEST`** (https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create).
- **Therefore:** use `tauri-plugin-autostart` for the *unelevated tray UI*, and register a **separate Task Scheduler task** for the elevated hardware helper.

### 9.4 Installer: NSIS vs MSI, and elevation

- Build: `npm run tauri build` / `cargo tauri build`, or `cargo tauri build --bundles nsis,msi`. `bundle.targets` defaults to `"all"` (https://v2.tauri.app/reference/config/#bundletarget).
- **NSIS `bundle.windows.nsis.installMode`** (https://v2.tauri.app/reference/config/#nsisinstallermode, https://v2.tauri.app/distribute/windows-installer/):
  - `"currentUser"` — **default**; no admin; installs under `%LOCALAPPDATA%`; metadata in HKCU.
  - `"perMachine"` — installs to `Program Files`; **requires admin**; metadata in HKLM.
  - `"both"` — user chooses; *"this mode will require Administrator access even if the user wants to install it for the current user only."*
  - The shipped template emits `RequestExecutionLevel admin` (perMachine) / `user` (currentUser) / `MULTIUSER_EXECUTIONLEVEL Highest` (both).
- **MSI/WiX is always per-machine** — `main.wxs` hardcodes `InstallScope="perMachine"` + `ProgramFiles64Folder`, so there is **no per-user MSI**; building MSI additionally requires the Windows **VBSCRIPT optional feature** or `light.exe` fails.
- **Artifact names:** NSIS `OpenHardwareOS_<version>_x64-setup.exe`; MSI `OpenHardwareOS_<version>_x64_en-US.msi` (under `target/release/bundle/`).
- **WebView2 install mode:** default `downloadBootstrapper`; `offlineInstaller` ≈ +127 MB and `fixedRuntime` ≈ +180 MB. Keep the default.
- **Installer hooks are the supported place to install a service or helper:** `bundle.windows.nsis.installerHooks: "./windows/hooks.nsh"` with the macros `NSIS_HOOK_PREINSTALL`, `NSIS_HOOK_POSTINSTALL`, `NSIS_HOOK_PREUNINSTALL`, `NSIS_HOOK_POSTUNINSTALL`.

**Recommendation:** `installMode: "perMachine"` (we need admin anyway for the helper/driver step, and a mixed per-user/per-machine layout makes the helper path ambiguous), NSIS as the primary artefact, MSI optional for enterprise deployment.

### 9.5 Runtime elevation and the recommended helper pattern

Tauri exposes **no elevation API** — `tauri-plugin-shell` has no `runas`/elevate option. Options:

- `runas` crate **1.2.0** (Apache-2.0, mitsuhiko): *"running a command in an elevated context"*; on Windows it is always GUI mode, cannot capture output, and *"the working directory is always the system32 folder on windows"* (https://docs.rs/runas/1.2.0/runas/).
- Or `ShellExecuteExW` with `lpVerb = "runas"` directly.
- Custom manifest: officially supported via `tauri_build::WindowsAttributes::new().app_manifest(include_str!("app.manifest"))` in `build.rs` (levels: `asInvoker` / `requireAdministrator` / `highestAvailable`, per https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests). Tauri's own docs use `requireAdministrator` as the example while warning that *"every time it is executed, a Windows UAC dialog will appear"*, and note a custom manifest **must re-declare the Common-Controls v6 dependency** or Tauri's dialogs break. **We will not do this** on the main exe.

| Pattern | Installer work | UAC the user sees | Failure modes |
|---|---|---|---|
| (a) Whole app elevated | `requireAdministrator` manifest | **Every launch** | WebView2 children elevated; autostart Run entry starts unelevated (so autostart would still prompt); huge blast radius. **Rejected.** |
| (b) Elevated **Windows service** + IPC | Create/start the service (`NSIS_HOOK_POSTINSTALL`), ACL the pipe | Once, at install | Runs in session 0 with no UI/tray; pipe ACLs must let the user-session client reach a LocalSystem server; a crashed helper silently stops fan control. Rust: `windows-service` 0.8.1 + `tokio::net::windows::named_pipe`. **Precedent: FanControl ships a "start on boot as a service, without any user session" mode.** |
| (c) **Task Scheduler task with `RunLevel=HighestAvailable`** | Register the task from the elevated installer | Once, at install; none per session | Absolute exe path → must be recreated on update/relocation; user or policy can disable it; no watchdog. |
| (d) Signed kernel driver of our own | EV cert + Dev Portal signing | Once | Multi-quarter, expensive; not viable. |

**Recommended:** **pattern (c) for v1, pattern (b) later.** This is not a hack — **Tauri's own updater uses exactly (c)**: `bundle.windows.wix.enableElevatedUpdateTask: true` ships `update.xml` + `install-task.ps1`, which self-elevates with `Start-Process -Verb RunAs` and registers `SCHTASKS.EXE /CREATE /XML update.xml /TN "Update {product} - Skip UAC" /F` with `<LogonType>InteractiveToken</LogonType>` and `<RunLevel>HighestAvailable</RunLevel>` (tauri-bundler MSI templates, https://v2.tauri.app/reference/config/#wixconfig). We can mirror that mechanism for the hardware helper: one UAC prompt at install, no prompts afterwards, no kernel driver of our own, and the elevated surface is one small binary.

**Design constraints imposed by modern Windows (important, and easy to miss):**

- **Administrator protection** (Windows 11; shipping since the KB5120998 update, off by default) makes elevation *just-in-time*: the user is deprivileged and *"Users need to interactively authorize every admin operation"*, elevated sessions use a **profile-separated** account, and Microsoft explicitly advises: *"Use local SYSTEM or dedicated service accounts for scheduled tasks or scripts set to run 'with highest privileges'. Essentially, redesign scripts to avoid expecting an always-on admin token."* It also warns that *"Settings data for applications don't carry over across the regular (unelevated) and the elevated profiles"* and that network drives/UNC paths are inaccessible from elevated apps ([Microsoft Learn](https://learn.microsoft.com/en-us/windows/security/application-security/application-control/administrator-protection/)). **Consequences for us:** store fan-curve config in a machine-readable location both contexts can reach (e.g. `%PROGRAMDATA%\OpenHardwareOS\config.yaml`), never keep state only in the elevated profile, and never assume the helper can read the user's mapped drives.
- **Autostart only covers the tray UI**: the elevated helper is started by its own scheduled task, not by the `Run` key. Design the IPC so the UI tolerates the helper being absent and retries.
- **Uninstall must restore fan control to firmware/BIOS defaults** and delete the scheduled task, the service and the driver prompt state — the uninstaller hook (`NSIS_HOOK_PREUNINSTALL`) is the place.

---

## Sources

**LibreHardwareMonitor**
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/README.md
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LICENSE
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/THIRD-PARTY-NOTICES.txt
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/LibreHardwareMonitorLib.csproj
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Utilities/HttpServer.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Utilities/PersistentSettings.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/UI/MainForm.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/UI/MainForm.Designer.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/UI/StartupManager.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitor.Windows.Forms/Resources/app.manifest
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/Hardware/IControl.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/Hardware/Motherboard/SuperIOHardware.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/PawnIo/PawnIo.cs
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/blob/master/LibreHardwareMonitorLib/Resources/PawnIo/README
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/pull/1857
- https://github.com/LibreHardwareMonitor/LibreHardwareMonitor/issues/2140
- https://api.nuget.org/v3-flatcontainer/librehardwaremonitorlib/index.json
- https://www.nuget.org/packages/LibreHardwareMonitorLib/

**Kernel driver, signing and vulnerable-driver policy**
- https://github.com/namazso/PawnIO
- https://github.com/namazso/PawnIO.Modules
- https://pawnio.eu/
- https://support.microsoft.com/en-us/windows/security/threat-malware-protection/microsoft-defender-antivirus-alert-vulnerabledriver-winnt-winring0
- https://nvd.nist.gov/vuln/detail/CVE-2020-14979
- https://learn.microsoft.com/en-us/windows/security/application-security/application-control/app-control-for-business/design/microsoft-recommended-driver-block-rules
- https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-policy--windows-vista-and-later-
- https://learn.microsoft.com/en-us/windows/security/application-security/application-control/administrator-protection/
- https://learn.microsoft.com/en-us/windows/win32/sbscs/application-manifests

**Ecosystem precedents**
- https://github.com/Rem0o/FanControl.Releases
- https://github.com/CalcProgrammer1/OpenRGB/blob/master/Documentation/SMBusAccess.md
- https://github.com/CalcProgrammer1/OpenRGB
- https://github.com/seerge/g-helper
- https://github.com/falahati/NvAPIWrapper
- https://github.com/jacobtread/lhm-service

**NVIDIA**
- https://docs.nvidia.com/deploy/pdf/NVML_API_Reference_Guide.pdf (NVML API Reference Guide, Release 615 — the citable primary source; the older `group__nvmlFanSpeed.html` HTML URLs now 404)
- https://docs.nvidia.com/deploy/nvml-api/api/group__nvmlDeviceCommands.html (Device Commands — fan setters, "requires root/admin access")
- https://docs.nvidia.com/deploy/nvml-api/api/group__nvmlDeviceQueries.html (Device Queries — fan getters, temperature thresholds)
- https://docs.nvidia.com/deploy/nvml-api/nvml-api-reference.html (NVML library locations on Windows)
- https://docs.nvidia.com/deploy/nvml-api/known-issues.html
- https://raw.githubusercontent.com/NVIDIA/go-nvml/main/pkg/nvml/nvml.h (NVML headers, 13.4.61)
- https://www.nvidia.com/en-us/drivers/nvidia-license/ (NVIDIA Driver License Agreement §2.7, §2.9)
- https://docs.nvidia.com/cuda/eula/index.html (CUDA EULA, Attachment A redistributables)
- https://developer.nvidia.com/nvidia-management-library-nvml
- https://docs.nvidia.com/cuda/cuda-installation-guide-microsoft-windows/
- https://docs.nvidia.com/nvapi/group__gpucooler.html (GPU Cooler Interface — only `NvAPI_GPU_GetTachReading` is public)
- https://github.com/Cldfire/nvml-wrapper
- https://docs.rs/nvml-wrapper/latest/nvml_wrapper/ · https://docs.rs/nvml-wrapper/latest/src/nvml_wrapper/lib.rs.html · https://docs.rs/nvml-wrapper/latest/nvml_wrapper/enum_wrappers/device/index.html · https://docs.rs/crate/nvml-wrapper/latest/features
- https://crates.io/crates/nvml-wrapper · https://crates.io/crates/nvml-wrapper-sys
- https://github.com/aufkrawall/green-curve (MIT reference implementation of elevated NVML fan control)
- https://docs.rs/nvapi-sys/latest/nvapi_sys/gpu/cooler/index.html · https://github.com/arcnmx/nvapi-rs
- https://raw.githubusercontent.com/LibreHardwareMonitor/LibreHardwareMonitor/master/LibreHardwareMonitorLib/Hardware/Gpu/NvidiaGpu.cs
- https://raw.githubusercontent.com/LibreHardwareMonitor/LibreHardwareMonitor/master/LibreHardwareMonitorLib/Interop/NvApi.cs

**AMD**
- https://gpuopen.com/adl/ (ADL SDK 18.1, "Now superceded by ADLX")
- https://gpuopen-librariesandsdks.github.io/adl/group__OVERDRIVENAPI.html (ADL Overdrive N API — fan setters)
- https://gpuopen-librariesandsdks.github.io/adl/group__OVERDRIVE8API.html (OD8 PMLog telemetry)
- https://gpuopen-librariesandsdks.github.io/adl/Overdrive8-example.html (capability-probing and fan-curve caveats)
- https://gpuopen-librariesandsdks.github.io/adl/ADL%20SDK%20EULA.pdf
- https://chromium.googlesource.com/chromiumos/overlays/portage-stable/+/b1ad75f083a189fdc9b70fc2e1daf08ecd45eada/licenses/AMD-ADL (verbatim ADL SDK EULA text, incl. §5(e), §5(f), Schedule A)
- https://github.com/GPUOpen-LibrariesAndSDKs/ADLX
- https://raw.githubusercontent.com/GPUOpen-LibrariesAndSDKs/ADLX/main/ADLX%20SDK%20License%20Agreement.pdf
- https://gpuopen.com/manuals/adlx/
- https://rocm.docs.amd.com/projects/install-on-windows/en/latest/reference/component-support.html (ROCm on Windows has no SMI)
- https://rocm.docs.amd.com/projects/amdsmi/en/latest/ ("supports Linux bare metal and Linux virtual machine guest environments")
- https://docs.kernel.org/gpu/amdgpu/thermal.html (sysfs: temp*_input, power1_average, fan1_input, pwm1, pwm1_enable, fan_curve)
- https://github.com/Traverse-Research/adlx-rs

**Rust crates and Windows APIs**
- https://crates.io/api/v1/crates/sysinfo · https://docs.rs/sysinfo/latest/sysinfo/ · https://github.com/GuillaumeGomez/sysinfo
- https://github.com/GuillaumeGomez/sysinfo/blob/master/src/windows/cpu.rs
- https://github.com/GuillaumeGomez/sysinfo/blob/master/src/windows/component.rs
- https://github.com/GuillaumeGomez/sysinfo/blob/master/src/windows/disk.rs
- https://github.com/ohadravid/wmi-rs · https://docs.rs/wmi/
- https://github.com/gz/rust-cpuid
- https://crates.io/crates/netcorehost
- https://github.com/mullvad/windows-service-rs · https://docs.rs/runas/1.2.0/runas/ · https://crates.io/crates/libloading
- https://learn.microsoft.com/en-us/windows-hardware/drivers/storage/msft-storagereliabilitycounter
- https://learn.microsoft.com/en-us/windows/win32/fileio/working-with-nvme-devices
- https://learn.microsoft.com/en-us/windows/win32/api/winioctl/ne-winioctl-storage_property_id
- https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-fan
- https://learn.microsoft.com/en-us/windows/win32/cimwin32prov/win32-temperatureprobe
- https://learn.microsoft.com/en-us/windows/win32/setupapi/run-and-runonce-registry-keys
- https://learn.microsoft.com/en-us/windows-server/administration/windows-commands/schtasks-create

**YAML**
- https://github.com/dtolnay/serde-yaml
- https://github.com/acatton/serde-yaml-ng
- https://github.com/cafkafk/serde-yaml
- https://github.com/bourumir-wyngs/serde-saphyr
- https://github.com/bourumir-wyngs/serde-yaml-bw
- https://github.com/saphyr-rs/saphyr
- https://github.com/rustsec/advisory-db/blob/main/crates/serde_yml/RUSTSEC-2025-0068.md
- https://crates.io/api/v1/crates/serde-saphyr

**Tauri, packaging and platform licences**
- https://v2.tauri.app/ · https://v2.tauri.app/start/
- https://v2.tauri.app/distribute/windows-installer/
- https://v2.tauri.app/reference/config/#nsisinstallermode
- https://v2.tauri.app/reference/config/#wixconfig
- https://v2.tauri.app/learn/system-tray/
- https://v2.tauri.app/plugin/autostart/
- https://v2.tauri.app/develop/sidecar/
- https://v2.tauri.app/about/trademark/
- https://docs.rs/tauri/latest/tauri/tray/index.html
- https://docs.rs/tauri/latest/tauri/enum.WindowEvent.html
- https://docs.rs/tauri/latest/tauri/enum.RunEvent.html
- https://github.com/tauri-apps/plugins-workspace/blob/v2/plugins/autostart/README.md
- https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/nsis/installer.nsi
- https://github.com/tauri-apps/tauri/blob/dev/crates/tauri-bundler/src/bundle/windows/msi/main.wxs
- https://github.com/tauri-apps/tray-icon
- https://github.com/zzzgydi/auto-launch
- https://learn.microsoft.com/en-us/microsoft-edge/webview2/concepts/distribution
- https://nsis.sourceforge.io/Docs/AppendixI.html
- https://github.com/wixtoolset/web/blob/master/src/Docusaurus/versioned_docs/version-v3/main/license.md
- https://github.com/dotnet/runtime/blob/main/LICENSE.TXT
- https://learn.microsoft.com/en-us/lifecycle/products/microsoft-net-and-net-core
- https://github.com/microsoft/WindowsAppSDK
- https://learn.microsoft.com/en-us/windows/apps/windows-app-sdk/deploy-unpackaged-apps
- https://learn.microsoft.com/en-us/windows/apps/package-and-deploy/self-contained-deploy/deploy-self-contained-apps
- https://github.com/microsoft/CsWin32
- https://api.nuget.org/v3-flatcontainer/microsoft.windowsappsdk/2.4.0/microsoft.windowsappsdk.2.4.0.nupkg
- https://registry.npmjs.org/react · https://registry.npmjs.org/vite · https://registry.npmjs.org/typescript
