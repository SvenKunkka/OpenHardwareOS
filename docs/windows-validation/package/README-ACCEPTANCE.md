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

Read `EVIDENCE.md` for the index. In one paragraph: the Rust workspace, its 443 tests,
clippy at `-D warnings`, the frontend's type check/tests/build and the licence audit all
pass **on macOS**; the whole codebase is **unverified on Windows**, no fan has ever
responded to a write from this code, and no installer has been produced. Everything
Windows-specific in this tree is marked *Prepared* in `docs/windows-validation/README.md`,
which is the honest baseline and is included here unchanged.

## Quick start (about ten minutes, read-only at first)

Run these from the extracted package root in **PowerShell** (not `cmd.exe`). Nothing in
steps 1–3 writes to hardware.

```powershell
# 1. Check the machine before anything else. This stops on the first problem and
#    tells you what failed; do not continue past a failure.
.\scripts\precheck.ps1

# 2. Prove the package is intact (every file against MANIFEST.sha256).
.\scripts\verify-package.ps1

# 3. Read-only evidence: what hardware is present, and can it be controlled?
#    `collect.ps1` writes one timestamped folder of text files and touches nothing else.
.\docs\windows-validation\collect.ps1 -DryRun     # show the plan, write nothing
.\docs\windows-validation\collect.ps1             # collect for real

# 4. Build it (this is the first step that compiles, and the only one that can take
#    ten minutes or more). It writes evidence under .\evidence\.
.\scripts\build.ps1
```

If step 1 fails, stop and report — the failure output is itself the evidence. Do not
work around a failed pre-check by editing the scripts; that destroys the value of the
run.

## Then follow the checklist

`docs/windows-validation/checklist.md` is the full procedure, in order, with what to
capture at each step and what a failure means. `docs/windows-validation/result-template.md`
is what to fill in. Do not duplicate either of them here.

## What the scripts do

| Script | Writes hardware? | Purpose |
|---|---|---|
| `scripts/precheck.ps1` | No | Refuses to let a run start on a machine that cannot produce trustworthy evidence: checks the OS, the Rust toolchain against the workspace MSRV, Node, npm, free disk space, and whether the package itself is intact. Exits non-zero on the first failure. |
| `scripts/verify-package.ps1` | No | Verifies every file against `MANIFEST.sha256`. Run it before trusting anything else in the package. |
| `scripts/build.ps1` | No (build only) | Runs the pre-check, then builds: frontend install and build, `cargo build --workspace --all-targets`, `cargo test --workspace`, and the NSIS installer. Captures output to `.\evidence\`. Stops on the first failing command. |
| `docs\windows-validation\collect.ps1` | No | The existing read-only evidence collector (unchanged from the repository). Reads hardware state, the LHM web server's reachability and the app's config directory; writes one folder of text. |

`build.ps1` never launches the app, never writes a fan value and never installs
anything. Installing the NSIS bundle is a deliberate, separate, manual step
(`checklist.md` §7.2) because it needs administrator rights.

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
