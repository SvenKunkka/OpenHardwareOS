# Verification log

An append-only record of what was actually executed, on which machine, and what
came back. It exists so a reviewer does not have to trust a summary: every entry
names the command, the environment, and the result — including the checks that
could **not** be run here.

Machine for every entry unless stated otherwise: macOS 26.6.2 (build 25G83),
Darwin 25.6.0 arm64 (Apple M5), Node 26.8.1, npm 11.19.0. The **Rust toolchain is
named in each entry**, because this machine has more than one — see the next
section. Development happens on macOS; **Windows has never been exercised** — see
`docs/windows-validation/`.

## Toolchains on this machine

Three toolchains are reachable, with different capabilities. This matters because
a check run with the wrong one fails for a reason that has nothing to do with the
project, and an entry that does not say which one it used is not reproducible.

| Toolchain | rustc | cargo | clippy | Role |
|---|---|---|---|---|
| `stable-aarch64-apple-darwin` | 1.92.0 (ded5c06cf 2025-12-08) | 1.92.0 | 0.1.92 (ded5c06cf2) | The owner's rustup default, **left exactly as it was**. Cannot build this workspace — see below |
| `/opt/homebrew/bin/{rustc,cargo}` | 1.98.0 (88d9e12ae 2026-08-18) (Homebrew) | 1.98.0 (797e8a9bc 2026-08-05) | reports **0.1.92** | First on `PATH`, so an unqualified `cargo build` in this shell uses it |
| `1.98.0-aarch64-apple-darwin` | 1.98.0 (88d9e12ae 2026-08-18) | 1.98.0 (797e8a9bc 2026-08-05) | 0.1.98 (88d9e12ae1) | Added in round 2. The matched pair used for every Rust check from round 2 on |

Two facts, each reproduced in round 2 rather than recalled:

1. **The owner's default cannot build the workspace.** `rust-version = "1.95"` is
   declared workspace-wide, and `sysinfo 0.39.6` requires 1.95 as well:

   ```
   $ rustup run stable cargo check -p ohm-adapter-system
   error: rustc 1.92.0 is not supported by the following packages:
     ohm-adapter-api@0.1.0 requires rustc 1.95
     ohm-adapter-system@0.1.0 requires rustc 1.95
     ohm-core@0.1.0 requires rustc 1.95
     ohm-device-model@0.1.0 requires rustc 1.95
     sysinfo@0.39.6 requires rustc 1.95
   [exit code: 101]
   ```

   (Round 1 recorded `sysinfo 0.39` as requiring "≥ 1.93". The resolver reports
   **1.95**; this entry replaces that number.)

2. **Clippy via `/opt/homebrew/bin` cannot check this project at all**, because
   the clippy it invokes is from a different release than the rustc it sits next
   to. `clippy-driver 0.1.92` reports itself as rustc 1.92, so cargo applies the
   `rust-version` gate and refuses before reading any code:

   ```
   $ PATH=/opt/homebrew/bin:/usr/bin:/bin /opt/homebrew/bin/cargo clippy -p ohm-core --all-targets
   error: rustc 1.92.0 is not supported by the following packages:
     ohm-core@0.1.0 requires rustc 1.95
   [exit code: 101]
   ```

   Round 1 worked around this with `--ignore-rust-version`. Round 2 does **not**:
   a lint run that has to disable a version assertion is not a whole-workspace
   run, so it was replaced by a matched toolchain instead.

**What round 2 changed, and what it did not.** With the owner's approval the
missing matched toolchain was installed from the official rustup channel:

```bash
~/.cargo/bin/rustup toolchain install 1.98.0 --profile minimal --component clippy,rustfmt
~/.cargo/bin/rustup run 1.98.0 cargo clippy --workspace --all-targets -- -D warnings
```

That was the *only* change to this machine's toolchains. The default toolchain is
still `stable-aarch64-apple-darwin`, `PATH` is unmodified, `~/.cargo/bin` was not
prepended to anything, and no shell configuration file was touched. Nothing was
installed with Homebrew. The pinned toolchain is addressed explicitly
(`rustup run 1.98.0 …`), so a plain `cargo` in this shell still resolves exactly
as it did before.

Date of the entries below: all say `2026-09-14`.

---

## 2026-09-14 — round 1: gates, conflict enforcement, dead-switch fixes

**Revision this entry describes:** commits `03407cf` … `e81b558`
(`git log --oneline 03407cf^..e81b558`). Everything below was run against that
range; a reviewer can check out `e81b558` and re-run it. Numbers in this entry are
what the commands printed *at that revision* — they were not re-measured in round
2, and the round-2 entry reports its own.

### Baseline (taken before any change in this round)

| Command | Result |
|---|---|
| `cargo test --workspace` | **372 passed, 0 failed** |
| `git rev-parse --is-inside-work-tree` | not a repository (initialised in this round) |

Final state of this round: **397 Rust tests pass, 0 fail**, `cargo fmt --check`
clean, `cargo build --workspace` with 0 warnings, `cargo deny check` clean, 22
frontend tests pass. Five local commits, no remote.

`cargo clippy --version` reported **0.1.92** while `rustc --version` reported
**1.98.0** — the two releases installed side by side in `/opt/homebrew/bin` are
out of step, so `cargo clippy --workspace` aborts before reading project code
(the exact command, output and exit code are in "Toolchains on this machine"
above). Round 1 called this an environment defect rather than a project defect,
and that judgement stands; round 1 also worked around it with
`--ignore-rust-version`.

**Superseded by round 2.** Round 1 recommended installing a matching toolchain
and stated that this "was **not** done here" because it would restructure the
machine owner's toolchain. Round 2 did it, with the owner's approval and without
restructuring anything: a pinned `1.98.0` toolchain addressed by absolute path,
with the default toolchain, `PATH` and shell configuration untouched. The
authoritative clippy run is now `rustup run 1.98.0 cargo clippy --workspace
--all-targets -- -D warnings`, which covers **all** crates — round 1 could only
lint the ten crates the mismatched pair could see.

### Changes verified in this round

| # | Change | Verification |
|---|---|---|
| 1 | `minimize_to_tray` wired to the real minimise behaviour, `close_to_tray` kept distinct | `cargo test -p ohm-desktop` → 15 passed, incl. `window_policy_follows_the_settings` |
| 2 | `adapter_settings.mock.enable_gpu_fan_control` was a no-op branch; now removes the GPU fan *control* channel while keeping monitoring | `cargo test -p ohm-adapter-mock -p ohm-adapters` → 31 + 8 passed; end to end in `tests/tests/edge_cases.rs::disabling_gpu_fan_control_keeps_monitoring_but_removes_control` (persists → reloads → device still monitored → no writable channel → index has no target → write refused with `capability_read_only`) |
| 3 | Numeric condition gate `Rule.when`, backwards compatible | `cargo test -p ohm-automation` → 79 passed, incl. 9 new gate tests (false ⇒ stand-down at the fail-safe duty; custom `otherwise`; reopen resumes the curve; exact boundary behaviour for all six operators; NaN/∞ never satisfy; missing condition source follows the sensor policy; the condition source gets the same grace period; unconditional rules unaffected; non-finite threshold rejected) |
| 4 | One output, one writer | 5 new engine tests: refusal on save, refusal on enable, disabled rules own nothing, file-level conflicts resolved deterministically with a `RuleConflict` diagnostic, and a conflict reaching the active set is skipped (`RuleStatus::Error`, `writes == 0`, the output carries exactly the owner's value) |
| 5 | `AdapterCapabilities::poll_interval_ms` honoured (was declared and unread) | `cargo test -p ohm-runtime` → 62 passed, incl. `an_adapter_hint_slows_its_own_polling_without_affecting_others` and `an_adapter_without_a_hint_is_polled_every_cycle` |
| 6 | CPU package power states its source instead of vanishing | `cargo run -p ohm-cli -- doctor` prints a **Capability notes** section: with no provider, "CPU package power: not available … comes from LibreHardwareMonitor (Provider: lhm) … reported as missing rather than as 0 W"; with the simulated provider, it names the device and adapter supplying it |
| 7 | CI no longer masks failures; `deny.toml` added; Windows bundle job added | `cargo-deny 0.20.2` installed locally and **run**: `cargo deny check` → `advisories ok, bans ok, licenses ok, sources ok`. The first attempt failed to parse — the file had been written against an older schema — which is exactly why running the tool matters. GitHub Actions itself cannot run from this machine |
| 8 | LHM writes are confirmed by reading the channel back | `cargo test -p ohm-adapter-lhm` → 37 passed, incl. `a_write_is_confirmed_by_reading_the_channel_back` and `a_channel_that_ignores_writes_is_reported_as_refused` (the fake server models a board that answers `200 OK` and keeps its old duty) |
| 9 | Frontend: condition editor, conflict surface, behaviour tests | In `apps/desktop`: `npm run typecheck` clean, `npm run test` → **4 files, 22 tests passed**, `npm run build` clean (376 kB). Independently re-run by me, not only reported by the workstream. A mutation check confirms the tests bite |
| 10 | The `when` gate end to end, plus a shipped gated example | `cargo test -p ohm-integration-tests --test gate_behaviour` → 6 passed; `cargo run -p ohm-cli -- rules check examples/rules/gpu-cooling-gaming-only.yaml` → "valid against the attached hardware" |

Three test bugs were found by these tests and fixed, each worth recording because
it taught something about the system: a gate threshold written as `0.99` when the
capability reports **percent** (the unit of a threshold is the unit of its
source); an assertion that assumed the curve output is always above the
stand-down duty (it is not); and an assertion that ignored the deadband, by which
the applied value legitimately lags the instantaneous curve.

Commands executed for the above:

```bash
cargo fmt --all -- --check
cargo build --workspace --all-targets
cargo test --workspace
cargo clippy -p <crates that the local clippy can build> --all-targets --ignore-rust-version
(cd apps/desktop && npm run typecheck && npm run build)
OHM_CONFIG_DIR=/tmp/... cargo run -q -p ohm-desktop -- --selftest --mock [--dry-run]
OHM_CONFIG_DIR=/tmp/... cargo run -q -p ohm-cli -- doctor [--mock] | demo | protocol
```

### What this round could **not** verify here

* **Windows, at all.** No `windows-latest` run has happened; the WMI storage path,
  the `reg.exe` autostart path, the tray on Windows, NVML and LibreHardwareMonitor
  against real hardware remain **Prepared** (`docs/windows-validation/`).
* **Full-workspace clippy**, for the toolchain reason above. Ten crates were
  checked with the local clippy (0 warnings) using `--ignore-rust-version`;
  `ohm-adapter-system` and `ohm-desktop` were invisible to clippy 0.1.92 because
  the workspace requires rustc 1.95 and `sysinfo 0.39.6` requires 1.95 as well.
  *(Round 1 wrote "≥ 1.93" here; the resolver reports 1.95. Round 2 removed this
  limitation entirely — see the round-2 entry.)*
* **The NSIS installer**: the configuration, icons and frontend build are in
  place, but no bundle has been produced on this machine (it is a Windows target).
* **Frontend behaviour on a real Windows machine.** The frontend suites *were*
  run in round 1 — `npm run test` → 4 files / 22 tests passed, and row 9 above
  records the independent re-run, so the earlier version of this bullet
  ("cannot verify frontend tests") contradicted row 9 and was wrong. What remains
  unverified is narrower and specific: no browser on Windows has rendered the UI,
  the tray and close/minimise behaviours have never been exercised as a window,
  and round 2's `unconfirmed` states have only been tested against the component
  contract.

### Deliberately **not** done

No real fan was written to, no drive was stopped, and no hardware setting was
changed on the host machine. `--dry-run` was used for every non-simulated pass,
and the simulated provider for every closed-loop demonstration.

---

## 2026-09-14 — round 2: unconfirmed writes, a no-op release, stale rule state, toolchain, packaging

**Revision: `c4e8486`, `8f78a06`, `096e13b`** (plus the documentation commit that
follows them). The verification pass below was executed with a clean worktree at
`096e13bebedc2ba6e66c568cc8d47212c870926a`, which is the revision every number in
this entry belongs to.

Round 2 had one job: fix the defects a review found, then make the docs, the UI
and the CLI agree with what the code actually does. No new architecture was added
and no requirement from the brief was expanded.

### The three defects, each with a failing test written first

Every entry is in the form **old behaviour → trigger → behaviour now → test that
would catch a regression**. In all three cases the tests were written and observed
failing before the fix, and the fix was made until they passed.

#### D1 — a write the provider never confirmed was reported as applied

* **Old behaviour.** `Runtime::write_value` treated *"the adapter accepted the
  request"* as *"the value is in effect"*. The LHM adapter returned `Applied`
  after `Set` regardless of what the channel then contained, and when the
  adapter's own read-back could not be performed, the runtime filled the missing
  applied value in with the *requested* one (`unwrap_or_else(|| value.clone())`).
  The engine then recorded `applied_output = Some(requested)`, counted the write,
  discarded `report.detail`, and — by comparing the next cycle's curve output
  against that value — treated the write as done and skipped it. A fan that
  ignored an 80 % command was displayed, audited and recorded as 80 %.
* **Trigger.** Write to a channel that cannot be read back: `Set` returns `200 OK`,
  `Get` returns `Ok(None)` (sensor absent from the tree) or an error. The
  `fake_server` fixture models exactly this (`ReadMode::{NotAvailable, Error}`).
* **Behaviour now.** Three outcomes are distinguishable end to end —
  request accepted, read-back confirmed, failed/unknown — via
  `WriteStatus::Unconfirmed` (which carries **no** value) and
  `WriteOutcome::{is_applied,is_confirmed,is_unconfirmed}`. The runtime never
  substitutes the request for a missing applied value, so an unconfirmed write
  cannot become `applied_output` and cannot be deduplicated away. The engine
  counts it, publishes `rule_unconfirmed`, retries, and after
  `MAX_CONSECUTIVE_UNCONFIRMED` (3) consecutive attempts hands the output to the
  write-failure policy, which drives the safety fail-safe duty. The audit trail
  records the unconfirmed outcome at warn level, and `changed_hardware()`
  excludes it. A confirmed LHM write reports the *channel set point* and says in
  its detail that the fan's actual speed is a separate reading — a read-back is
  not airflow.
* **Tests.** `tests/tests/write_confirmation.rs`:
  `an_unconfirmed_write_is_not_recorded_as_the_applied_value`,
  `an_unconfirmed_write_is_retried_instead_of_being_deduplicated`,
  `repeated_unconfirmed_writes_trigger_the_write_failure_policy`; in
  `adapters/libre-hardware-monitor`: `an_unreadable_channel_does_not_claim_the_
  requested_value`, `a_confirmed_set_point_says_nothing_about_airflow`,
  `confirmation_resumes_once_the_channel_can_be_read_again`; frontend
  `apps/desktop/src/test/writeStates.test.ts`.

#### D2 — `release` was accepted, evaluated, and did nothing

* **Old behaviour.** `FallbackAction::Release` was declarable in a rule file and
  the evaluator returned `release: true`, but the engine only logged and published
  an event. Nothing released anything: no adapter in this build can hand a channel
  back to firmware mid-run, so a rule asking for release silently held its last
  value while reporting that it had released.
* **Trigger.** Any rule with `fallback: { on_sensor_missing: release }` (or
  `on_write_failure: release`) reaching the fallback path.
* **Behaviour now.** The option is refused rather than faked. `release` cannot be
  saved, imported, enabled, or loaded into the active set; `Rule::validate`
  rejects it on either fallback field. A legacy rule file that already contains
  it is **left byte-for-byte untouched on disk**, substituted in memory with the
  fail-safe duty, and reported as a per-field `RuleFileNote` (surfaced by
  `compatibility_notes()` and by `ohm-cli rules check`), so the machine is
  protected and the user is told why. The UI no longer offers `release` in either
  fallback selector. Whole-device `shutdown()` is explicitly *not* used as a
  channel release.
* **Tests.** `tests/tests/fallback_release.rs` (4 tests, incl.
  `a_release_rule_imported_as_yaml_is_refused_by_validation`,
  `a_release_rule_file_reports_its_substitution_through_the_store`,
  `a_legacy_release_rule_loads_with_a_diagnostic_and_still_protects_the_machine`);
  `engine::tests::{release_cannot_be_saved_or_enabled,
  a_stray_release_rule_still_protects_the_machine}`.

#### D3 — editing a rule inherited the previous target's state

* **Old behaviour.** `save_rule` reset only `next_due_ms`. Retargeting a rule from
  output A to output B therefore carried A's `applied_output` and hysteresis
  anchor into the rule's state, so the first evaluation against B was deduplicated
  as "unchanged" and **B was never written**, while the UI showed A's value as the
  rule's output. The same stale-state problem applied to changing the source or
  the gate, including a `when` that had just turned false with an identical
  `otherwise`.
* **Trigger.** Edit a saved rule and change `target` (or `source`, `when`,
  `otherwise`), then run one engine tick.
* **Behaviour now.** `save_rule` captures the previous rule and produces a
  `RuleChange`; `apply_rule_change` invalidates the affected state so the new
  output is driven from the new target's own evidence on the first cycle, and the
  abandoned output is queued as a `PendingHandover` and driven to the fail-safe
  duty at the next tick, with the reason recorded in the audit trail.
  `load_rules` runs the same reconciliation, so a retarget that arrives via the
  file store behaves identically to one made in the UI. Metadata-only edits
  (name, description, interval) do not interrupt control.
* **Tests.** `tests/tests/rule_edit_state.rs` (6 tests, incl.
  `retargeting_a_rule_drives_the_new_output_and_hands_over_the_old_one`,
  `the_handover_of_an_abandoned_output_is_audited`).

### Toolchain (round 2)

`rustup toolchain install 1.98.0 --profile minimal --component clippy,rustfmt`,
then every Rust check via `rustup run 1.98.0`. The owner's default toolchain,
`PATH` and shell configuration are unchanged, and no Homebrew package was
installed — the reasoning, the before/after table and the two reproduced failures
of the old pair are in "Toolchains on this machine" at the top of this file.

Clippy revealed **real** problems that the mismatched pair had never been able to
see — a collapsible `if` in `adapters/system`, an unused parameter in the
automation engine, and five lints in `apps/desktop/src-tauri` (needless lifetime,
field-reassign-with-default, a useless conversion, a collapsible `if`, an unused
import). They were fixed rather than allowed; `--ignore-rust-version` was not
used. Commit `8f78a06`.

### Packaging and clean-checkout paths (round 2)

The Windows bundle job looked for the installer in
`apps/desktop/src-tauri/target/release/bundle`. `apps/desktop/src-tauri` is a
member of the root Cargo workspace, so `cargo metadata` reports the target
directory as `<repo>/target` and nothing is ever written to the path the job
used: the listing step produced nothing and the upload step would have failed
with "no files found". Both now derive the directory from
`cargo metadata --format-version 1 --no-deps`, the listing step fails loudly when
`<target>/release/bundle` is absent, and `docs/windows-validation/checklist.md`
§7.1 and `collect.ps1` use the same source of truth. The `lint` and Linux `rust`
jobs now install the WebKitGTK/GTK development packages that compiling and
linting the desktop crate needs. Commit `096e13b`; evidence in the table below.

### The verification pass (all commands re-run at `096e13b`, nothing carried over)

Environment: the pinned toolchain from "Toolchains on this machine"; `cargo-deny
0.20.2`; Node 26.8.1 / npm 11.19.0; macOS 26.6.2 (25G83). Every command below was
executed in this round; the log is `/tmp/ohm-verify/pass/full-pass.log` on the
machine that ran it (not committed — it is reproducible with the commands shown).

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.0 cargo fmt --all -- --check` | exit 0, no output |
| 2 | `rustup run 1.98.0 cargo clippy --workspace --all-targets -- -D warnings` | **exit 0, no warnings** — whole workspace, including `ohm-adapter-system` and `ohm-desktop`, which round 1 could not lint |
| 3 | `rustup run 1.98.0 cargo build --workspace --all-targets` | exit 0 |
| 4 | `rustup run 1.98.0 cargo test --workspace` | **413 passed, 0 failed, 0 ignored**, across 39 test binaries and doc-tests |
| 5 | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` (exit 0; duplicate-crate warnings only) |
| 6 | `npm ci` in `apps/desktop` | exit 0, 141 packages — the lockfile is in sync with `package.json`, which is what the CI `npm ci` step depends on |
| 7 | `npm run typecheck` | exit 0 |
| 8 | `npm run test` | **5 files, 27 tests passed** (round 1: 4 files, 22 tests) |
| 9 | `npm run build` | exit 0, `dist/assets/index-*.js` 376.14 kB (gzip 111.82 kB), CSS 22.80 kB |
| 10 | `OHM_CONFIG_DIR=<tmp> ohm-cli doctor --mock` | exit 0: 4 providers (LHM unavailable on macOS, stated with a fix hint), 15 devices, 27 readable sensors / 4 writable actuators |
| 11 | `OHM_CONFIG_DIR=<tmp> ohm-cli demo --steps 60 --profile gaming` | exit 0: GPU 69.1 °C → 68.7 °C, final duty 48 % (1169 RPM), 60 evaluations, 45 rule writes, 15 skipped, 0 fallbacks |
| 12 | `ohm-cli audit --limit 3` | 51 records in the demo config dir: 48 writes, all to `fan.mock.0` / `fan.mock.1` / `fan.openhardwareos_of4_sim_0001.0` — **simulated devices only**; 47 `simulated` + 1 `applied`, origins 45 `automation` + 3 `shutdown` |
| 13 | `ohm-cli rules check examples/rules/*.yaml` (4 files) | **exit 1 on each**, `device 'gpu.mock.0' was not found` — correct and already documented: the examples target the simulated machine, and `rules check` has no `--mock` flag |
| 14 | the same 4 files with `{"experimental_features": true}` in a scratch config dir | exit 0, **`valid against the attached hardware`** for all four — which also verifies the workaround `docs/windows-validation/checklist.md` §5.5 tells the operator to use |
| 15 | `ohm-cli protocol` | exit 0, full ODP exchange incl. `SET_STATE` → `StateSet` and the device-side fallback note |
| 16 | `cargo run -p ohm-desktop -- --selftest --mock` | exit 0, 15 devices, 4 controllable, rule applied |
| 17 | `cargo run -p ohm-desktop -- --selftest --mock --dry-run` | exit 0, same report |

Isolation evidence for the pass: every run used `OHM_CONFIG_DIR` under a throwaway
root; all files it created live there (`audit.jsonl`, `logs/`, `rules/`); the real
per-user config directory `~/Library/Application Support/OpenHardwareOS` does not
exist. The only worktree change during the pass was `docs/requirements.md`, edited
by hand while it ran — no test wrote into the repository. The 4 non-zero exits in
row 13 are the documented, expected refusal and are reported as such rather than
counted as a pass; row 14 is the same four files passing once the simulated
providers are visible.

### What round 2 could **not** verify here

* **Windows — still nothing at all.** No workflow was pushed or triggered, so the
  `windows-latest` jobs are **Prepared / not run**, and no NSIS installer has been
  built anywhere. The CI fixes were verified as *logic*: the two `pwsh` step
  bodies were extracted from `ci.yml` and executed under portable PowerShell
  7.4.20 (see below).
* **A physical fan responding.** The read-back added for D1 confirms what the
  *provider* reports as a channel's set point. No fan's RPM or airflow has been
  measured in response to a commanded duty, and no write has ever reached real
  hardware. Scenario D remains **Unverified on hardware**.
* **The new CLI/store diagnostics on Windows** — `rules check` names release
  substitutions on macOS from the store, but no Windows file-system path has
  exercised them.
* **Homebrew's clippy on the full workspace.** It still cannot run at all here
  (reproduced above); the authoritative lint is the pinned toolchain locally and
  a matching pair in CI.

### Evidence for the CI and collector changes (executed, not asserted)

No GitHub Actions workflow can run from this machine, so the workflow itself is
Prepared. Its PowerShell logic was executed for real instead. Portable PowerShell
7.4.20 was unpacked into `/tmp` from the official release tarball — nothing was
installed system-wide, `PATH` was not modified, and the shell configuration was
not touched.

| Check | Result |
|---|---|
| `collect.ps1` parses | 0 syntax errors (PowerShell AST parser) |
| Collector path helpers, extracted from the shipped file via the AST and run against 6 fixtures | **12 named checks pass, 0 fail**, plus an inline guard asserting the reported directory is not the foreign workspace's: real repository agrees with `cargo metadata`; a fake workspace with built binaries finds `release` then `debug`; a `build.target-dir` override is honoured; with `cargo` off `PATH` it falls back to `<repo>\target`; from an unrelated cwd it still reports **this** repository; from inside a foreign cargo workspace it reports **this** repository, not the foreign one |
| `collect.ps1 -DryRun` | exit 0, printed the plan, wrote nothing (the output root stayed empty and the worktree was untouched) |
| `ci.yml` step "Read the workspace target directory" | exit 0; wrote exactly `target_dir=/Users/keychron/Documents/OpenHardwareOS/target` to `GITHUB_OUTPUT`, equal to `cargo metadata` |
| `ci.yml` step "Report what was produced" with a fabricated bundle present | exit 0 and listed the artefact |
| the same step with no bundle directory | **exit 1** with `no bundle directory at <target>/release/bundle — the packaging configuration or the target path changed` |
| the checklist §7.1 snippet, both branches | extracted from the doc and run verbatim: with no bundle it prints the target directory and warns `nothing at …\release\bundle — the build did not bundle. Record that.`; with a fabricated bundle it prints the directory and the artefact. (The first version of that snippet relied on `Get-ChildItem` erroring for a missing directory, which PowerShell does for a missing parent but not consistently for a missing leaf, so the present version tests the path explicitly) |
| every bash step body in `ci.yml` | `bash -n` clean (18 bodies) |
| the Linux package list | matches the Debian/Ubuntu list at <https://v2.tauri.app/start/prerequisites/> verbatim, including `libayatana-appindicator3-dev` |
| the desktop self-test needs no display | `--selftest` returns before `tauri::Builder::default()` is constructed (`apps/desktop/src-tauri/src/lib.rs`), so the Linux CI job needs no Xvfb |

Two defects in the *harness* were found this way and fixed in the harness, not the
product: a `Resolve-Path`-based path comparison that did not follow the macOS
`/tmp` symlink, and a PowerShell single-element-collection unroll that made `[0]`
index a character.

### Deliberately **not** done in round 2

* No real hardware was written to, no autostart or device-control setting was
  changed, and nothing was installed on the host beyond the pinned Rust toolchain
  (with the owner's approval) and a portable PowerShell unpacked into `/tmp`.
* Nothing was pushed, published, tagged or triggered; there is no remote.
* The licence decisions (ADR 0002, ADR 0004) remain **Proposed**; no licence
  question was resolved.
* No new architecture: no CPU native collector, no cross-adapter identity work, no
  plugin loading. CPU package power remains at the LHM support boundary, and the
  per-capability `Capability::poll_interval_ms` and
  `AdapterCapabilities::discovery_interval_ms` hints remain unimplemented and are
  now documented as such in `docs/requirements.md`.


---

## 2026-09-14 — round 3: control-handover integrity, desktop feedback, a delivery package

**Revision: the commits `544d38d`, `d38600c`, `67d2af9`, `5521e82`/`e1eba21` and the
documentation commit that contains this entry.** The pass below was executed with a
clean worktree; the revision is recorded in the table itself, because it was re-run
after the docs were written.

Round 2 fixed "the rule was retargeted from fan A to fan B". A read-only review found
that the same idea — *a rule leaving a channel behind it* — still had three groups of
holes, plus a desktop that had never been told about the round-2 status at all. Round 3
closes those, and adds a delivery package, without adding a feature the brief does not
ask for.

### 1. Only some ways of leaving control were covered

* **Old behaviour.** `RuleChange` compared `target.device` alone, so switching
  *capability* on the same device was not a change at all: the old channel kept the
  abandoned duty and the new one inherited the old one's `applied_output`, which
  suppressed its first write. `delete_rule` removed the rule's state without handing
  anything over, `set_rule_enabled(false)` let the rule stop being evaluated with its
  channel left at the last curve value, and `load_rules` iterated only over rules that
  were still on disk — a file that had been deleted simply vanished, state and all.
* **Trigger.** Any of: retarget to another capability on the same device; disable a
  running rule; delete it; delete its file and reload.
* **Behaviour now.** A channel is `(device, capability)` everywhere. Every path that
  ends control — retarget, disable, delete, a file that disappeared — runs the same
  reconciliation, and each rule records the channel it is *actually* driving
  (`RuleState::control`, a `ControlHold`), which is what makes "did this rule ever take
  control of it?" answerable at all. A source, condition or mapping change is *not* an
  exit: the same rule still owns the same channel and keeps driving it.
* **Tests.** `tests/tests/handover_integrity.rs` — `switching_capability_on_the_same_
  device_hands_over_the_old_channel`, `disabling_a_rule_hands_its_channel_over`,
  `deleting_a_rule_hands_its_channel_over`,
  `a_rule_file_removed_from_disk_is_handed_over_on_reload`, and — the counterpart —
  `changing_only_the_source_does_not_hand_the_channel_over`.

### 2. Handovers were queued for channels the rule never drove, and executed blindly

* **Old behaviour.** Queueing looked only at whether the *config* named a different
  target: it did not check that the rule was enabled, or that it had ever written
  anywhere. Execution did not re-check ownership either. The review's reproduction: R2
  (enabled) drives fan 0 at 40 %; R1 is disabled and points at the same fan; R1 is
  retargeted to another fan; the handover writes 70 % onto R2's channel, and R2's cache
  still says 40 % — so the hardware is at 70 % while the rule, and therefore the UI,
  reports 40 %. A second: A→B→C edited twice before the first tick wrote the fail-safe
  duty onto B, a channel nothing had ever driven.
* **Trigger.** Retargeting a disabled rule; retargeting a rule whose channel another
  enabled rule also targets; editing a rule twice before the engine runs.
* **Behaviour now.** A handover is queued only for the channel the rule's own record
  says it held. Ownership is re-checked before *every* attempt: a channel that an
  enabled rule targets again is **superseded** — on the record, and not written — so a
  stale handover can never overwrite the live owner. Queueing for a channel that
  already has an unfinished handover merges into it rather than accumulating entries,
  and the attempt budget deliberately survives the edit.
* **Tests.** `a_disabled_rule_that_never_drove_a_channel_cannot_hand_it_over`,
  `a_handover_is_superseded_by_the_rule_that_now_owns_the_channel`,
  `retargeting_twice_before_the_first_tick_never_touches_the_middle_channel`,
  `a_rule_deleted_before_its_first_write_hands_over_nothing`,
  `a_retry_does_not_replay_a_handover_whose_channel_has_a_new_owner`, and in the state
  suite `a_superseded_handover_names_the_rule_that_took_over`,
  `repeated_edits_of_one_channel_stay_one_handover`.

### 3. A failed handover disappeared

* **Old behaviour.** `perform_pending_handovers` drained the queue with `mem::take`
  *before* attempting the write; an `Unconfirmed` result only published an event and an
  `Err` only logged. Nothing was left to retry, so a channel whose handover failed
  stayed at the abandoned duty for good — the review's reproduction: A→B with A's
  handover unreadable, and once the link recovered A was still at the old low duty.
* **Trigger.** A refused, or accepted-but-unconfirmed, handover write.
* **Behaviour now.** The handover book keeps the work: retried every
  `HANDOVER_RETRY_TICKS` (5 ticks — nothing spins), up to `MAX_HANDOVER_ATTEMPTS` (5),
  after which it is **parked as `Failed`**: still in the queue, still readable, with the
  first error (the cause) and the last one, re-armable by
  `AutomationEngine::retry_failed_handovers` / `ohm-cli handovers --retry`. Nothing
  re-arms itself. `Confirmed` requires a confirmed write; `Superseded` requires a live
  owner — nothing else completes a handover. The rule id is kept as *data*, so a
  deleted rule does not hide the channel it left unprotected.
* **Tests.** `a_refused_handover_is_retried_on_a_bounded_cadence`,
  `a_recovered_channel_completes_its_handover_exactly_once`,
  `an_unconfirmed_handover_stays_pending_with_its_reason`,
  `an_exhausted_handover_is_parked_and_can_be_re_armed`,
  `a_confirmed_handover_is_recorded_with_its_value`, `a_handover_outlives_its_rule`,
  `an_owed_handover_is_listed_before_it_is_attempted`,
  `a_failing_handover_records_why_and_how_often`.

**What the pre-fix run showed.** Of the 12 tests in `handover_integrity.rs`, **8 failed
against the unfixed code**, each with the symptom above and nothing else: the old
channel left at 55 % instead of 70 %; 70 % written onto a channel another rule owns;
the middle channel of A→B→C written once though never driven; the unconfirmed handover
never completed. The other four passed, two of them for the wrong reason (no retry
existed at all), which is why the state suite was added afterwards: it asserts the
record, not just the hardware calls.

### 4. The desktop had not been told any of this

* **Old behaviour.** `types.ts` declared `WriteStatus = 'applied' | 'simulated' |
  'rejected'`. An `unconfirmed` result therefore fell through to the *success* branch in
  `DeviceDetail` (green, "applied"), and a missing `applied` value was rendered as
  "nothing" and "not applied" — the UI asserting an outcome nobody had established,
  which is exactly the defect round 2 removed from the backend. `compatibility_notes()`
  had no Tauri command, no IPC method and no screen: a legacy `release` rule was
  substituted in memory where only the CLI could see it.
* **Behaviour now.** `unconfirmed` is part of the contract and is warn, never ok; one
  exhaustive module (`src/lib/writeStatus.ts`) owns tone, label, sentence and
  applied-value text, and a report with no `applied` value says `value unknown — not
  confirmed`. The manual-control path raises its own warn notice for an unconfirmed
  result, distinct from the danger notice for a refusal, both carrying the backend's
  `detail` verbatim. `RuleFileNote` is structured (`field`, `original`, `effective`,
  `message`, `hint`) and reaches the Automation screen, which states that the file was
  not modified and the fail-safe duty still protects the machine. Handovers reach a
  Diagnostics panel with a retry control.
* **Two defects found while fixing it.** (a) Pushed `write_performed` events are merged
  into the same recent-writes list as the audit rows and were all levelled `info`, so an
  unconfirmed write would still have rendered green through the event path. (b)
  `HandoverReport`'s optional fields serialised as `null` while TypeScript declares
  them `?` — a value the frontend never expects to read. They are omitted when absent
  now, matching `WriteReport`, and the exact key sets are asserted in a test.
* **Tests.** `apps/desktop/src/test/writeStates.test.tsx` (17 tests, replacing the
  helper-only `writeStates.test.ts`) renders the real screens through the real
  providers with only the IPC module mocked; `apps/desktop/src-tauri/src/commands.rs`
  adds three tests, including `compatibility_notes_arrive_structured` (a real legacy
  rule file on disk, read back through the command) and
  `the_ipc_payload_shape_matches_the_typescript_contract`.

### 5. The verification pass

Everything below was executed in this round, on this machine, after the fixes. The two
non-zero exits in the earlier run of this pass were both real and both are visible in
the history above: `cargo fmt --check` finding my newly added test code unformatted,
and `make-acceptance-package.sh` refusing to package an uncommitted tree. Both were
fixed before the record below.

Environment: macOS 26.6.2 (25G83) arm64, pinned `rustc 1.98.0` / `cargo 1.98.0` /
`clippy 0.1.98` (`rustup run 1.98.0`); the owner's default toolchain `stable` (1.92.0),
`PATH` and shell configuration untouched; `cargo-deny 0.20.2`; Node 26.8.1 / npm
11.19.0.

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.0 cargo fmt --all -- --check` | exit 0 |
| 2 | `rustup run 1.98.0 cargo clippy --workspace --all-targets -- -D warnings` | **exit 0, no warnings** |
| 3 | `rustup run 1.98.0 cargo build --workspace --all-targets` | exit 0 |
| 4 | `rustup run 1.98.0 cargo test --workspace` | **444 passed, 0 failed, 0 ignored**, 41 test binaries and doc-tests |
| 5 | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |
| 6 | `npm run typecheck` / `npm run test` / `npm run build` | exit 0 / **5 files, 39 tests passed** / exit 0, `index-*.js` 387.88 kB |
| 7 | `ohm-cli doctor --mock` | exit 0: 4 providers (LHM unavailable on macOS, with a fix hint), 15 devices, 27 readable sensors / 4 writable actuators |
| 8 | `ohm-cli demo --steps 60 --profile gaming` | exit 0: GPU 69.1 → 68.7 °C, 48 % / 1169 RPM, 60 evaluations, 45 writes, 15 skipped, 0 fallbacks |
| 9 | `ohm-cli audit --limit 2` | clean shutdown, `control_released — 3 outputs handed back to firmware` |
| 10 | `ohm-cli handovers` and `--retry` | exit 0 both; reports nothing outstanding on a fresh config directory and re-arms 0 |
| 11 | `ohm-cli protocol` | exit 0, full ODP exchange including `SET_STATE` → `StateSet` |
| 12 | `ohm-desktop --selftest --mock` and `--dry-run` | exit 0, 15 devices, 4 controllable, rule applied |
| 13 | `ohm-cli rules check examples/rules/*.yaml` (4 files, with `experimental_features` in a scratch config dir) | exit 0, `valid against the attached hardware` for all four |
| 14 | `scripts/make-acceptance-package.sh` | exit 0 on a clean tree; see §6 |

Isolation: every run used `OHM_CONFIG_DIR` under a throwaway root, the real per-user
config directory does not exist, and the only worktree changes during the pass were the
documentation files being written by hand.

### 6. The Windows acceptance package

`scripts/make-acceptance-package.sh` assembles a **source** package for one exact
commit, and verifies itself. From the run recorded in this entry:

* the package verified against its own manifest — every file hashed and matched;
* no VCS data, no `target/`, no `node_modules/`, no Windows binary and no local runtime
  state (audit trail, settings, logs) is present;
* the manifest detects tampering: appending one line to a source file fails
  verification naming that file, and adding an unlisted file fails naming that file;
* the **shipped pre-check refuses a non-Windows host** (exit 1) as it must, and with
  `-AllowNonWindows -Toolchain 1.98.0` it passes every check;
* run against the machine's `PATH` pair it **fails**, correctly: `/opt/homebrew/bin`
  has `rustc 1.98.0` beside `clippy 0.1.92`, and that pair cannot lint this workspace.
  That is the round-2 lesson encoded as a pre-flight check rather than a paragraph.

The package states in its own README that it contains no Windows build artefacts, and
its evidence index marks the whole Windows surface *Prepared — never run*.

### What round 3 could **not** verify here

* **Windows — still nothing.** No workflow was pushed or triggered; every
  `windows-latest` job is **Prepared**, and no NSIS installer has been built anywhere.
  The package's PowerShell entry points were executed here **on macOS only**, with
  `-AllowNonWindows`; on Windows they are unexecuted.
* **The desktop against a live Tauri IPC round trip.** The frontend tests mock the IPC
  module, and no GUI session was launched. What *is* verified is both halves of the
  contract: the producer side by a Rust test asserting the exact JSON key set, and the
  consumer side by rendering the real screens. A live round trip remains unverified.
* **A physical fan.** Unchanged and still the largest gap: no write has reached real
  hardware, and no fan's response has been measured. The read-back confirms what a
  *provider* reports as a channel's set point, which is not airflow.
* **One unreproduced observation.** In a single run of the frontend suite with a
  deliberately mutated handover tone, two tests failed where three subsequent runs of
  the same mutation failed exactly one; the unmutated suite then passed 39/39 three
  times in a row. It is recorded here because it happened, not because it was explained.

### Deliberately **not** done in round 3

No hardware was written to, no autostart or device-control setting was changed, and
nothing was installed on the host. Nothing was pushed, published, tagged or triggered.
The licence decisions (ADR 0002, ADR 0004) remain **Proposed**. No new architecture: no
CPU native collector, no cross-adapter identity work, no plugin loading — CPU package
power stays at the LHM support boundary.
