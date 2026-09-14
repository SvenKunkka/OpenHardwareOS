# OpenHardwareOS — Windows acceptance package

**This directory is a source snapshot, not a product build.**

It contains the OpenHardwareOS source tree at one exact commit, the tooling needed to
build and inspect it on a Windows machine, and the current evidence index. It contains
**no Windows binaries, no installer and no `target` directory** — none exist yet,
because this project has never been built or run on Windows. Nothing in here is a
placeholder for a missing artefact: if you are looking for
`OpenHardwareOS_0.1.0_x64-setup.exe`, it has not been built.

| | |
|---|---|
| Commit | `@COMMIT@` (`@COMMIT_SHORT@`) |
| Commit date | @COMMIT_DATE@ |
| Worktree at packaging time | @WORKTREE@ |
| Packaged (UTC) | @PACKAGED_AT@ |
| Package contents hash | see `MANIFEST.sha256` (verify with `scripts/verify-package.ps1`) |

---

## What is verified, and what is not

Read `EVIDENCE.md` for the index. In one paragraph: the Rust workspace test suite,
clippy at `-D warnings`, the frontend's type check/tests/build and the licence audit all
pass **on macOS**; the whole codebase is **unverified on Windows**, no fan has ever
responded to a write from this code, and no installer has been produced. Everything
Windows-specific in this tree is marked *Prepared* in `docs/windows-validation/README.md`,
which is the honest baseline and is included here unchanged.

No pass count is quoted here on purpose: a number copied into a package goes stale the
next time a test is added. `docs/verification-log.md` is the authority — it records each
command, its environment, its result and the revision it was run at.

## Quick start (about ten minutes, read-only at first)

Run every command from the **extracted package root** in **PowerShell** (not `cmd.exe`),
except where a step says otherwise. Nothing in steps 1–3 writes to hardware.

The build never writes inside this package. It works in a **work directory beside the
extracted package**, by default:

```text
<extract-root>\OpenHardwareOS-@COMMIT_SHORT@-windows-acceptance-build\
    source\          a copy of the package, remade on every run
    target\          cargo's output (CARGO_TARGET_DIR), kept between runs
    evidence\        one timestamped folder per run: logs, environment, installer record
```

That is what makes a second run possible: the package stays exactly the tree the
manifest describes, so it can be verified again afterwards. Pass `-WorkDir <path>` to
`precheck.ps1` and `build.ps1` to put the work directory somewhere else (a fast disk is
a good reason); give both scripts the same value.

```powershell
# 1. Check the machine before anything else. This stops on the first problem and
#    tells you what failed; do not continue past a failure.
.\scripts\precheck.ps1

# 2. Prove the package is intact (every file against MANIFEST.sha256).
.\scripts\verify-package.ps1

# 3. Read-only evidence: what hardware is present, and can it be controlled?
#    Give the collector a directory OUTSIDE the package: it writes a bundle folder
#    into whatever directory it is handed, and a folder inside the package would stop
#    the package matching its own manifest.
$work = '..\OpenHardwareOS-@COMMIT_SHORT@-windows-acceptance-build'
.\docs\windows-validation\collect.ps1 -DryRun -OutputRoot "$work\evidence"   # show the plan, write nothing
.\docs\windows-validation\collect.ps1 -OutputRoot "$work\evidence"           # collect for real

# 4. Build it (this is the first step that compiles, and the only one that can take
#    ten minutes or more). It writes only into the work directory:
#      logs and the installer record -> $work\evidence\<timestamp>\
#      the verified installer       -> $work\target\release\bundle\nsis\
.\scripts\build.ps1
```

If step 1 fails, stop and report — the failure output is itself the evidence. Do not
work around a failed pre-check by editing the scripts; that destroys the value of the
run.

Two behaviours worth knowing before you read the output:

* `build.ps1` verifies the package **before** it copies the source and **again after**
  the build. If the package gained, lost or changed a single file, the run fails and
  names it — including the case where a build wrote its output into the package.
* `BUILD COMPLETE` is printed only when the installer **that run** was supposed to
  produce exists: the name comes from `apps/desktop/src-tauri/tauri.conf.json`
  (`productName`, `version`, the `nsis` target), the file must be non-empty, and a
  leftover installer from an earlier build does not count. The absolute path, size and
  SHA-256 are printed and written to the run's `91-installer.txt`.

## Then follow the checklist

A one-page Chinese entry (what evidence exists today, the package versions and hashes,
the read-only pre-check steps, what to send back if something fails, and what each
remaining gate still needs) is at `docs/windows-validation/ACCEPTANCE-ENTRY.md`.

`docs/windows-validation/checklist.md` is the full procedure, in order, with what to
capture at each step and what a failure means. `docs/windows-validation/result-template.md`
is what to fill in. Do not duplicate either of them here.

## What the scripts do

| Script | Writes hardware? | Purpose |
|---|---|---|
| `scripts/precheck.ps1` | No | Refuses to let a run start on a machine that cannot produce trustworthy evidence: checks the OS, that the package matches its manifest, that the work directory is writable and separate from the package, the Rust toolchain against the workspace MSRV and against clippy, Node, npm, and free disk space on the volume the build writes to. Exits non-zero on the first problem. |
| `scripts/verify-package.ps1` | No | Verifies every file against `MANIFEST.sha256`. Any missing file, any changed byte and **any file the manifest does not describe** fails, naming it. There is no exemption list and no `-Force`. |
| `scripts/build.ps1` | No (build only) | Runs the pre-check, copies the source into the work directory, then builds: frontend install and build, `cargo build --workspace --all-targets`, `cargo test --workspace`, and the NSIS installer. Verifies the package again at the end and refuses to print `BUILD COMPLETE` unless this run produced the declared installer. Stops on the first failing command. |
| `docs\windows-validation\collect.ps1` | No | The existing read-only evidence collector (unchanged from the repository). Reads hardware state, the LHM web server's reachability and the app's config directory; writes one folder of text into the directory you give it with `-OutputRoot`. |

`build.ps1` never launches the app, never writes a fan value and never installs
anything. Installing the NSIS bundle is a deliberate, separate, manual step
(`checklist.md` §7.2) because it needs administrator rights.

`scripts\_tools.ps1` is the shared helper every entry point dot-sources: it resolves
`rustup`, `cargo`, `rustc`, `node`, `npm` and `npx` to **absolute paths** (failing with a
message naming the tool and every directory searched, rather than falling back to a bare
name) and it runs every native command in an explicit working directory with an explicit
`$LASTEXITCODE` check. With `-Toolchain <name>`, every child process — including the
cargo inside `npx tauri build` — is pinned to that toolchain through
`RUSTUP_TOOLCHAIN`. Nothing is written to the User or Machine environment and no shell
profile is touched: the variable is set in the script process only.

`docs\windows-validation\package\scripts\tests\run-script-tests.ps1` is the regression
harness for all of the above. It exercises the scripts' control flow with **test doubles**
only (a fake rustup/cargo/rustc/node/npm/npx and a fake Tauri build), writes solely inside
one temporary directory, and prints a PASS/FAIL summary. It is **not** evidence that a
Windows build succeeds — that needs a real Windows machine — and its own header says so.

## Known limits of this package

* **No build artefacts.** Everything here must be built on the target machine.
* **No code signing.** The installer, once built, is unsigned and will trigger
  SmartScreen. Record the exact warning text (see `checklist.md` §7.2).
* **No vendored binaries.** LibreHardwareMonitor and PawnIO are downloaded from their
  own projects by the operator, never bundled — see `docs/decisions/0003` and the
  licence table in `README.md`.
* **The example rules target the simulated machine.** `examples/rules/*.yaml`
  reference `*.mock.*` ids, so `ohm-cli rules check` against them fails on real
  hardware by design; `checklist.md` §5.5 explains the two honest ways to check a rule
  against real hardware.
* **The scripts are unexecuted on Windows.** Their logic was exercised on the
  development machine with test doubles, and the package's manifest covers every file
  in them, but a Windows run is the only thing that can produce Windows evidence.
