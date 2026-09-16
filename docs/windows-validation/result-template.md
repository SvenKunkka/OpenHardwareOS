# Windows real-hardware validation — result report

Fill this in **during** the session, not afterwards. A reviewer must be able to tell the
three status levels apart at a glance, so every row carries exactly one of
**Prepared**, **Build/run passed** or **Verified on real hardware** — defined in
[`README.md`](README.md). The rules that make this report worth anything:

- a row is **Build/run passed** only with a captured exit code and output;
- a row is **Verified on real hardware** only with a before/after value, a timestamp, and
  the machine and component model;
- an unrun row stays **Prepared** — that is an acceptable, honest state;
- an unexplained observation goes in §7 as written, and is **not** quietly folded into a
  pass.

Companion documents: [`checklist.md`](checklist.md) (the procedure),
[`safety.md`](safety.md) (the write rules, agreed before the session),
[`collect.ps1`](collect.ps1) (the evidence bundles).

---

## 1. Session identity

| Field | Value |
|---|---|
| Date and local time of session start | |
| Operator (name / handle) | |
| Machine owner (name / handle) — the person who agreed to [`safety.md`](safety.md) | |
| Machine description (make, model, form factor) | |
| Machine use right now (idle desktop / nothing else running / shared) | |
| Repository commit (`git rev-parse HEAD`) | |
| Branch / tag | |
| "Before" evidence bundle folder (from `collect.ps1`) | |
| "After" evidence bundle folder | |
| Where the screenshots live | |

## 2. Environment

| Item | Value | How it was obtained |
|---|---|---|
| Windows edition | | `environment.txt` |
| Windows version | | `environment.txt` |
| Windows build number | | `environment.txt` |
| Architecture (x64) | | `environment.txt` |
| PowerShell version used | | `environment.txt` |
| Session elevated? (`True`/`False`) | | `environment.txt`; needed for NVML write steps only |
| `rustc --version` | | checklist §1.2 |
| `cargo --version` | | checklist §1.2 |
| `node --version` | | checklist §1.3 |
| MSVC toolchain present | | `rustup show` |
| CPU model | | `environment.txt` (Win32_Processor) |
| Motherboard model (needed to interpret SuperIO fan channels) | | `environment.txt` (Win32_BaseBoard) |
| GPU model(s) | | `environment.txt` (Win32_VideoController) |
| NVIDIA driver version | | `environment.txt` |
| `nvml.dll` present at `System32` | | `environment.txt` |
| Disk model(s) | | `environment.txt` |
| LibreHardwareMonitor version | | `environment.txt` |
| LibreHardwareMonitor running **as Administrator**? | | LHM's own window title / Task Manager elevation column |
| LHM web server on port | | LHM `Options -> Remote Web Server -> Interface / Port` |
| `http://127.0.0.1:8085/data.json` HTTP status | | `environment.txt` |
| LHM Basic authentication enabled? | | `environment.txt` status 401 or the LHM options dialog |
| PawnIO installed? Version | | `environment.txt`; must be **PawnIO**, never WinRing0 (see checklist §1.6) |
| Any security policy in play (memory integrity/HVCI, Smart App Control, S mode, Defender blocklist alerts) | | Windows Security UI — record as observed |
| Config root in use (default or `--config-dir` / `OHM_CONFIG_DIR`) | | `ohm-cli paths` output |

## 3. Per-step results

One row per step you actually ran. Add rows freely; do not merge steps, because the whole
point is to keep the evidence granular. Copy the step id from [`checklist.md`](checklist.md).

| Step | Command (as typed) | Exit code | Observed (short, factual) | Status level | Evidence file / screenshot |
|---|---|---|---|---|---|
| 2 | `collect.ps1` (dry run) | | | Prepared | |
| 2 | `collect.ps1` | | | Prepared | |
| 3 | `cargo test --workspace` | | passed count **as printed**: | | |
| 4 | `cargo build --workspace --all-targets` | | | | |
| 4 | `cargo test -p ohm-adapter-system` | | the 3 `windows.rs` tests present? | | |
| 4 | `cargo test -p ohm-desktop` | | `autostart_is_reported_as_windows_only` absent? | | |
| 5.1 | `cargo run -p ohm-cli -- doctor` | | | | |
| 5.1 | `cargo run -p ohm-cli -- doctor --mock` | | | | |
| 5.2 | `cargo run -p ohm-cli -- status` | | | | |
| 5.2 | `cargo run -p ohm-cli -- watch --count 3` | | | | |
| 5.3 | `cargo run -p ohm-cli -- demo` | | writes observed: | | |
| 5.4 | `cargo run -p ohm-cli -- protocol` | | | | |
| 5.4 | `cargo run -p ohm-cli -- paths` | | root: | | |
| 5.5 | `cargo run -p ohm-cli -- rules list` | | | | |
| 5.5 | `cargo run -p ohm-cli -- rules suggest` | | | | |
| 5.5 | `cargo run -p ohm-cli -- rules check <path>` | | | | |
| 5.6 | `cargo run -p ohm-cli -- audit` | | | | |
| 6.1 | `cargo run -p ohm-desktop` | | UAC prompt appeared? (must be **no**) | | |
| 6.2 | `cargo run -p ohm-desktop -- --selftest` | | | | |
| 6.2 | `… --selftest --mock --dry-run` | | any audit entry `"status":"applied"`? (must be **no**) | | |
| 7.1 | `npx tauri build` | | artefact path: | | |
| 7.2 | install `…_x64-setup.exe` | | UAC prompts: | | |
| 7.2 | launch installed app | | | | |
| 7.3 | uninstall | | config dir survived? | | |
| 8.1 | close window (`close_to_tray` on) | | window hidden, process alive? | | |
| 8.2 | left-click tray icon | | | | |
| 8.3 | tray Show / Hide | | | | |
| 8.4 | tray Rescan hardware | | | | |
| 8.5 | tray Open config folder | | | | |
| 8.6 | tray Quit | | channels released? | | |
| 8.7 | minimise (`minimize_to_tray` on) | | | | |
| 8.8 | close with `close_to_tray` off | | app exited? | | |
| 9.1 | enable Start with Windows | | toggle stayed on? | | |
| 9.2 | `reg query "HKCU\Software\Microsoft\Windows\CurrentVersion\Run" /v OpenHardwareOS` | | exact value data: | | |
| 9.3 | `reg query "HKLM\…\Run" /v OpenHardwareOS` | | not found? (must be) | | |
| 9.4 | disable Start with Windows | | value deleted? | | |
| 9.5 | sign out / sign in | | started elevated? (must be **no**) | | |
| 10.1 | route used to reach NVML (LHM stopped, or `nvidia.always`) | | | | |
| 10.2 | `… doctor` (NVML telemetry) | | hotspot reason: | | |
| 10.3 | NVML fan write from an elevated shell | | applied / refused | | |
| 11.1 | `http://127.0.0.1:8085/data.json` | | status: | | |
| 11.1 | `… doctor` with LHM running | | lhm device count: | | |
| 11.2 | mapping checks by eye (§11.2 table) | | all six checked? | | |
| 11.3 | LHM stopped → `… doctor` again | | reason shown (expect `not_present`): | | |
| 12.2 | `cargo run -p ohm-desktop -- --dry-run` + one UI write | | fan moved? (must be **no**) | | |
| 12.3 | one supervised real write (per output, see §4) | | | | |
| 12.6 | hand control back (tray Quit) | | channels back under firmware? | | |

For every non-`Prepared` row, name the evidence file. "It looked right" is not evidence.

## 4. Per-device fan control table (acceptance scenario D)

One row per **controllable output**, from the capability list in
`cargo run -p ohm-cli -- doctor`. This is the table that decides whether scenario D is
Verified on real hardware. Read checklist §12 before filling it in — in particular §12.3,
which is why the "value the device reported back" column must come from an **independent
read** (`GET /Sensor?action=Get&id=<SensorId>`, or LHM's own tree), not from the
application's `applied` field: the write path treats LHM's answer as the verdict and does
not read the channel back itself.

| Device id | Capability | Machine / component model (board, SuperIO chip, GPU, or header label) | Requested value | Value the device reported back (and how you read it) | RPM before | RPM after | Applied or refused | Exact refusal reason / detail text | Verdict (Verified / Build-run passed / not attempted) |
|---|---|---|---|---|---|---|---|---|---|
| e.g. `fan.lhm.lpc_nct6687d_0_1` | `fan.speed_percent` | ASUS … / Nuvoton NCT6687D / `Fan #1` header | 40 % | 40 % (`/Sensor?action=Get`, SensorId `/lpc/nct6687d/control/0`) | 810 | 1180 | applied | — | |
| | | | | | | | | | |
| | | | | | | | | | |
| | | | | | | | | | |
| | | | | | | | | | |

Additional required notes:

| Question | Answer |
|---|---|
| Did the app's reported `applied` value ever differ from the value you read back from the device? | |
| Did any output report a *different* value than requested (firmware clamp)? Was `"clamped":true` in the audit entry? | |
| Did a rejection produce a follow-up `origin.kind = "safety"` fail-safe write, as the runtime is documented to do? | |
| Was every write — applied **and** rejected — present in `audit.jsonl`? (any missing write is a defect) | |
| Was any write attempted where `"simulated":true` although you did **not** pass `--dry-run`? | |
| After quitting the app, did every channel you touched return to firmware control (§12.6)? | |

Device-level verdict for scenario D as a whole:

- [ ] **Verified on real hardware** — at least one specific output on this machine accepted
      a requested value, reported it back, and its tachometer responded, with before/after
      evidence.
- [ ] **Build/run passed only** — the write path executed and was audited, but no output
      was confirmed to have moved (e.g. every attempt was refused, or no controllable
      output exists on this machine).
- [ ] **Not attempted** — and why:

State which it is in one sentence, here:

> Scenario D verdict:

## 5. Known limitations confirmed on this machine

Mark each as *confirmed* (you observed it), *not observed*, or *not applicable*. These are
documented limitations, not defects — but this is where a session records whether they are
what the documentation says.

| # | Documented limitation | Confirmed? | Evidence / note |
|---|---|---|---|
| 5.1 | **CPU package power comes only from LibreHardwareMonitor.** No other provider in this build reports it (`adapters/system` exposes no power capability; NVML reports GPU power as `power.gpu`). With LHM off, CPU power is absent, not zero. | | |
| 5.2 | **GPU hotspot temperature is not available from NVML** and is reported `unsupported`; LHM can read it through NVAPI, which is a different provider. | | |
| 5.3 | **Motherboard fan RPM and control require LHM running as Administrator** with the web server on; no in-box Windows API provides either. | | |
| 5.4 | **Elevation is required for the NVML fan *setter*** even though NVML reads are unprivileged. | | |
| 5.5 | **The kernel driver is PawnIO, not WinRing0**; WinRing0 is on Microsoft's vulnerable-driver blocklist and must not appear anywhere in the install. | | |
| 5.6 | **CPU live frequency is not available**; only the rated clock from the OS. | | |
| 5.7 | **CPU temperature from OS thermal zones is unreliable on desktops** — may be absent, coarse, or bogus; LHM is the reliable source. | | |
| 5.8 | **SSD/NVMe temperature depends on the drive implementing `MSFT_StorageReliabilityCounter.Temperature`**; drives that do not are reported `hardware_limitation`, not 0 °C. | | |
| 5.9 | **NVML reports a *commanded* fan speed, not a tachometer** — a blocked fan still reports a value. | | |
| 5.10 | **A `Run` entry cannot start an elevated process**; autostart is per-user by design and the app never installs a scheduled task or service. | | |
| 5.11 | **There is no elevated helper, service or scheduled task in this build**, so fan control does not survive logon; only the normal `relinquish_on_exit` path hands control back. | | |
| 5.12 | **The installer is unsigned** (no code-signing configuration), so SmartScreen may warn. | | |
| 5.13 | **No uninstall hook exists**, so uninstall does not return fan control to firmware, delete a task, or remove the per-user config directory. Record what actually happened at uninstall (checklist §7.3). | | |
| 5.14 | Anything `docs/research.md` marks unverified that you *may* have evidence about — NVIDIA per-SKU fan control, AMD fan-control behaviour, the elevation semantics of `Run` keys. Record as a **single observation on this machine only**; it does not upgrade the project's stated uncertainty. | | |

## 6. Safety events

Complete even if nothing happened.

| Question | Answer |
|---|---|
| Was [`safety.md`](safety.md) read and agreed before the first write? By whom? | |
| Did any stop condition occur (unexpected noise, temperature rising at high duty, 0 RPM at high duty, smoke/smell)? | |
| Was the emergency ceiling reached (default 90 °C)? Did the emergency override fire (100 % duty, audited as `origin.kind = "safety"`)? | |
| Did the safety layer clamp or block any value? Which, and to what? | |
| Was any recovery step needed (LHM `SetDefault`, reboot)? Describe exactly what you did. | |
| Is the machine now in a known-good state (fans under firmware control, no pinned duty)? | |

## 7. Unexplained observations

Anything you saw that you cannot attribute to a specific line of source or a documented
limitation goes here, **verbatim** — the output, the timestamp, the device. Do not
summarise it into a pass, and do not attempt a fix during the session.

| # | What happened | Reproduction (exact commands) | Evidence file | Suspected area (or "unknown") |
|---|---|---|---|---|
| 1 | | | | |
| 2 | | | | |
| 3 | | | | |

## 8. Deviations from the checklist

Steps skipped, reordered, or run differently — and why.

| Step | Deviation | Reason |
|---|---|---|
| | | |

## 9. Sign-off

Write this by hand; a blank sign-off means the session is unfinished.

| Statement | Answer |
|---|---|
| Items **Verified on real hardware** (list the step ids, and for §4 the device ids): | |
| Items **Build/run passed** only (list the step ids): | |
| Items still **Prepared** — never executed in this session (list the step ids): | |
| Items that **failed** (list the step ids and the §7 observation numbers): | |
| Hardware writes performed in this session (count, device ids, values, and whether each was independently confirmed): | |
| Machine returned to a known-good state? | |
| Does this session change any claim in [`README.md`](README.md)'s status table, or in `docs/research.md`? If yes, say exactly which claim and on what evidence: | |
| Operator signature and date | |
| Machine owner signature and date (writes were performed on their hardware) | |

> A report that says "not attempted", or that stops at a failure with a complete
> [`checklist.md` §13](checklist.md#13-stop-and-report) capture, is a valid and useful
> outcome. A report that upgrades a label without the evidence for it is not.
