# Windows real-hardware validation kit

This kit is what a person with a real Windows machine runs to close the gap between
"the code exists and is unit-tested" and "this Windows-specific path was executed on
Windows hardware". OpenHardwareOS was developed on macOS: the `cfg(windows)` modules,
the NSIS installer, the tray, the `HKCU\...\Run` autostart path, the WMI storage
temperature probe and NVML fan writes have **never been compiled or executed on
Windows by the authors**.

Nothing in this directory changes the product. It contains one read-only PowerShell
collector, one ordered checklist, one fill-in report and one safety agreement.

| File | What it is | Who runs it |
|---|---|---|
| [`checklist.md`](checklist.md) | The ordered, copy-pasteable procedure with exact commands, expected output shape and what a failure means | The operator, on the Windows machine |
| [`collect.ps1`](collect.ps1) | Read-only evidence collector: one timestamped bundle per run | The operator, before and after the checklist |
| [`result-template.md`](result-template.md) | The fill-in report that turns a session into reviewable evidence | The operator, during and after the session |
| [`safety.md`](safety.md) | The rules for any hardware write, agreed **before** the session | The operator and the machine's owner |

Related project documents: [`docs/research.md`](../research.md) (verified research and
citations), [`docs/decisions/0003`](../decisions/0003-libre-hardware-monitor-integration.md)
(LibreHardwareMonitor integration), [`docs/decisions/0006`](../decisions/0006-windows-privileges-and-autostart.md)
(privileges and autostart), [`docs/roadmap.md`](../roadmap.md) (known gaps).

---

## Status vocabulary — use these three words, and only these three words

Every item in this kit carries exactly one of these labels. They are not
interchangeable, and they are not a scale of confidence: each one names a different
kind of evidence.

| Level | Means | Evidence required | What it does **not** mean |
|---|---|---|---|
| **Prepared** | The script, command or step exists and was reviewed against the source. It has **not** been executed on Windows by the authors — or, for a step the operator has not run yet, not executed at all. | The file and the source line it was derived from. Nothing else. | It does **not** mean it works. It does not mean the output shape below is what a real machine prints. |
| **Build/run passed** | Executed on a Windows machine. The command exited 0 (or the documented non-zero, when the point of the step is a rejection) and its output was captured to a file in the evidence bundle. | Command, exit code, captured stdout/stderr, the machine's OS build, and the date. | It does **not** mean any device was read correctly, or that a fan responded. A program that prints nothing but exits 0 is Build/run passed and nothing more. |
| **Verified on real hardware** | A **specific device on a specific machine** was read and/or controlled, with before/after evidence. | Machine and component model; the value before; the value the device reported back after; a timestamp; a screenshot or captured output path. | It does **not** generalise. Verifying fan header #1 on one Nuvoton NCT6687D board verifies that header on that board, in that session. |

Two rules that follow from this:

- **Never upgrade a label without the evidence column above.** "It compiled" is
  Build/run passed. "The fan spun up" is Verified on real hardware. They are different
  claims about different things.
- **A per-SKU or per-vendor uncertainty stays uncertain.** `docs/research.md` marks
  NVIDIA per-SKU fan-control support, AMD fan-control behaviour, and the elevation
  semantics of `Run` keys as **unverified**. A single successful run does not change
  that: record it as one observation on one machine, in the "unexplained observations"
  or limitations section of the report — not as a general claim.

---

## Status of this repository today

**Everything in this kit is Prepared. Nothing is Build/run passed. Nothing is
Verified on real hardware.**

No output produced by any of these commands on a Windows machine exists in this
repository, because the project was developed on macOS. Concretely:

| Area | Level today | Why |
|---|---|---|
| `cargo test --workspace` on Windows | **Prepared** | `docs/verification-log.md` records the passing run as 413 tests at commit `096e13b`, on macOS aarch64 with the pinned `rustc 1.98.0`. No Windows run is recorded, and a macOS pass says nothing about the `cfg(windows)` code paths. |
| `cargo build --workspace --all-targets` on Windows (compiles `cfg(windows)` code) | **Prepared** | `.github/workflows/ci.yml` declares a `windows-latest` job, but this repository holds no captured output from it, and a CI job definition is not a run. |
| `adapters/system/src/windows.rs` (WMI `MSFT_StorageReliabilityCounter`) | **Prepared** | Never executed on Windows. Its `#[cfg(test)]` tests do not touch WMI; they test the variant/`temperature_for` helpers. |
| `apps/desktop/src-tauri/src/autostart.rs` (per-user `Run` value) | **Prepared** | Never executed on Windows. The `not(windows)` test is compiled out there. |
| NSIS bundle, install, uninstall | **Prepared** | Never built or installed on Windows. |
| Tray, close-to-tray, minimize-to-tray | **Prepared** | Never launched on Windows. |
| NVML telemetry and fan write (`adapters/nvidia`) | **Prepared** | Never executed on a machine with an NVIDIA driver. |
| LibreHardwareMonitor web-server integration against a real LHM + SuperIO chip | **Prepared** | Only ever exercised against the in-process `fake_server.rs` and a synthetic `data.json` fixture. Since round 2 the adapter reads the channel back after a write and reports `Unconfirmed` when it cannot, so a wrong claim about a write is much harder — but a fake server is still not a SuperIO chip. |
| Per-device fan write / tachometer response | **Prepared** | No write has ever reached a real fan through this code, and no fan's RPM response has ever been measured. This is a **separate** gap from the Windows gap: a Windows run and a tachometer measurement are two different pieces of evidence, and neither substitutes for the other. |

The line above is the honest baseline. When a session finishes, the operator updates
the status column **in the report for that session** — this file's table is a statement
about the repository, so it changes only when the completed report is committed
alongside it.

---

## What "read-only" means for `collect.ps1`

`collect.ps1` gathers evidence and nothing else. Its header states the same rules:

- it never writes to hardware, never sends a fan or pump value;
- it never touches the registry (reads one value, writes none);
- it never starts, stops or creates services or scheduled tasks;
- it never reads private files outside the application's own config directory;
- the only things it creates are the bundle folder and the files inside it.

Two disclosures, because they are easy to miss:

1. **Prepared, not executed.** `collect.ps1` cannot be run on macOS, so it has never
   been executed by the authors at all. Review it by eye before the first run — it is
   deliberately kept short enough for that. Run it once with `-DryRun` and read the
   printed plan; nothing is written in that mode.
2. **Invoking the application's own `doctor`/`audit` is not side-effect free.** The
   script runs `ohm-cli doctor` and `ohm-cli audit` when a built binary exists. Those
   commands create the config directory if it is missing and may append a `lifecycle`
   line to `audit.jsonl`. That is the application writing its own state in its own
   config directory — never hardware, never the registry — but it is a write, and it is
   disclosed here rather than hidden.

---

## How a session runs

1. Read [`safety.md`](safety.md) with the person who owns the machine. Agree it
   before anything is executed. If nobody can physically watch and hear the machine,
   stop here.
2. Run `collect.ps1` **before** the checklist — the "before" bundle is the baseline
   for the report.
3. Work through [`checklist.md`](checklist.md) in order. Record everything in
   [`result-template.md`](result-template.md) as you go, not afterwards.
4. Run `collect.ps1` again **after** the checklist, into a second bundle.
5. Fill in the sign-off line of the report: which items are Verified on real hardware
   and which are only Build/run passed. Leaving the sign-off blank means the session is
   not finished.
6. If anything fails, or behaves in a way you cannot explain, capture what
   [`checklist.md` §13](checklist.md#13-stop-and-report) lists *verbatim* and stop.
   Do not work around it during the session.

## Where the evidence goes

| Evidence | Path, relative to the bundle folder `collect.ps1` prints |
|---|---|
| OS build, elevation, host/board/CPU/GPU/disk models, driver versions | `environment.txt` |
| PawnIO presence and version; the storage reliability counters the Windows adapter queries | `environment.txt` |
| LHM process state and the `/data.json` HTTP status | `environment.txt` |
| Registry `Run` value, config directory listing | `environment.txt` |
| `ohm-cli doctor` output | `cli-doctor.txt` |
| `ohm-cli audit` output | `cli-audit.txt` |
| Tail of the application's own `audit.jsonl` | `audit-tail.jsonl` |
| Everything the checklist produces | the operator's own files in the same folder, named by step id |

Screenshots are the operator's job: put them in the bundle folder and reference the file
name in the report's evidence column.
