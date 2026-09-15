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

---

## 2026-09-14 — round 4: who really owns a channel, responsibility that outlives the process, and a proven desktop

**Revision: the commits `17ed274`, `067a2a4`, `9f993f4`, `2756ca3`, `82f687e`, `0f162e5`,
`0ce5f0b` and the documentation commit that contains this entry.** The pass was executed
twice at this revision: once with the test code still carrying an unused import (clippy
failed, and the import was removed), and once with **every tracked file committed and no
modification outstanding** — that second run is the one recorded in the table below, and
its environment block (`HEAD 0ce5f0b`, 477 Rust tests, 42 frontend tests) is the authority
for its own numbers.

One caveat, because it is visible in that run: the packager refused to run, because an
untracked directory `k10max-prospector/` sits in the repository root. It is **not** part of
any work in this repository — it contains a Keychron keyboard firmware image, a ZMK patch
and a reconnaissance log, appeared mid-round, and was left exactly as it was found. Since
the package is assembled with `git archive HEAD`, that directory cannot enter it; the
package for this round was therefore built with `--allow-dirty`, which records the
deviation in the package itself rather than hiding it. The only other untracked path at the
end of the pass was that same directory: `git status --porcelain` shows no modification to
any tracked file.

Round 3 made the engine hand a channel over to the fail-safe duty. Round 4 is about the
three ways that was still not true: a rule could *look* like it had taken the channel
over when it had not, the record of an unfinished handover died with the process, and
the desktop had never been shown to be connected to any of it at all.

### 1. A declared owner was treated as a takeover

* **Old behaviour.** Ownership was decided by declaration: if an enabled rule named the
  channel as its target, the handover was marked `Superseded` — resolved, off the
  record, done. The review's reproduction: R1 has written 55 % to fan A and is
  retargeted to fan B, so A is owed a handover; R2 is enabled and targets A but its
  sensor is gone with `on_sensor_missing: hold`, so R2 writes nothing at all; the next
  tick sees R2's declaration, closes the handover, and fan A sits at R1's abandoned
  55 % with nothing anywhere recording that it is unprotected.
* **Trigger.** Enabling a rule that targets a channel while it produces no output, or
  whose writes are unconfirmed or refused; retargeting a rule onto a channel another
  disabled rule had been pointed at.
* **Behaviour now.** A **claim** (an enabled rule targets the channel) is separated from
  a **takeover** (that rule has driven the channel *and the device confirmed a value*).
  A claim still stops the engine writing under a rule that owns the channel — the write
  race of round 3 — but it no longer resolves anything. A claimed-but-undriven channel
  becomes `AwaitingOwner`: still owed, still visible, naming the claimant and its own
  last status, and saying in words that the channel is not known to be protected. It
  waits a bounded `HANDOVER_OWNER_WAIT_TICKS` and then parks as `Failed` with a reason
  naming the rule that never took control. When the claim disappears — the rule is
  disabled, deleted, or retargeted away — the handover resumes by itself with a fresh
  attempt budget, because the reason it stopped no longer exists; a handover parked by
  its *own* failed writes still needs an explicit retry.
* **Pre-fix evidence.** Of the 9 tests in `tests/tests/handover_owner.rs`, **8 failed**
  against the previous code, each reporting `superseded — r2 owns it now` for a channel
  R2 had never written to. The ninth pins the round-3 guarantee that a *confirmed* owner
  is never overwritten by a stale handover.
* **Tests.** `handover_owner.rs`: the three "owner has not taken over" cases (no output,
  unconfirmed, refused), the bounded wait ending in a visible failure, the confirmed
  takeover resolving without a write, recovery after the claimant is disabled, deleted
  or retargeted, and the round-3 guarantee. One round-3 test needed a second tick: with
  evidence required for resolution, the evidence only exists after the owner has written.
  The desktop learns the new state exhaustively (`awaiting_owner` is *owed*, tone warn,
  never ok) with a panel test checked by mutation.

### 2. An unfinished handover died with the process

* **Old behaviour.** The handover book and the per-rule control records lived in memory
  only. Kill the app while a handover is pending and the next process had no pending
  item, an empty `ohm-cli handovers`, and nothing on the Diagnostics panel — while the
  rule that abandoned the channel might have been deleted in the meantime, so nothing
  was left that knew the channel existed.
* **Trigger.** Any unfinished handover, or any write whose result was never learned,
  followed by a restart.
* **Behaviour now.** `crates/ohm-automation/src/recovery.rs` writes exactly two things
  to `<config>/control-state.json`: unresolved handovers, and writes that were issued
  and never confirmed. Versioned, atomic (temp file in the same directory, then rename),
  refusing a file from a newer version rather than guessing. What is deliberately *not*
  stored matters as much: no confirmed values, no curve positions, no hysteresis
  anchors, no gate state — a value read by a previous process is not evidence about now,
  and a restored curve position would drive a fan to a value nobody asked for today.
  Everything recovered comes back as `NeedsVerification`: checked against the device,
  the capability, the current owner and the freshness of the data before the safety
  policy applies, and never replayed. A channel that no longer exists fails visibly,
  keeping the original cause; a resolved item leaves the record entirely. A write that
  cannot be recorded is reported — `AutomationStats.persistence_error`,
  `ohm-cli handovers` printing "NOT RECORDED", and an error notice on the Diagnostics
  panel — because losing this file is the failure it exists to prevent.
* **Two defects this work found in itself**, both caught by the restart tests rather
  than by inspection: persisting *before* the rule's state was committed to the engine
  wrote an empty record and then marked the books clean; and a handover queued by a
  *reload* (a rule file removed while the app ran) was never flushed at all, so the
  process could die with the responsibility unsaved. Both are fixed and both now have a
  test that fails without the fix (`a_handover_queued_by_a_reload_is_recorded_immediately`
  reads the record back *without ticking*, because the process could die at that moment).
  Every mutation point now goes through one `flush_control_state()`.
* **Tests.** `tests/tests/control_recovery.rs` (9) destroys an engine and builds a new
  one over the same isolated config directory, covering a pending handover, a failed
  one, an issued-but-unconfirmed write, a still-enabled rule that must *not* be
  double-handled, a completed handover that must not be replayed, a damaged record, a
  channel whose device is gone, a storage failure, and the reload path.
  `apps/cli/tests/handover_recovery.rs` (4) runs the real CLI binary against a config
  directory containing a previous session's record: the channel, the deleted rule's
  name, the original cause, the attempt count and the way to re-arm all reach the
  operator. `ohm-cli handovers` polls once and verifies before reporting, so the report
  is about the machine rather than about a file that never advances.

### 3. The desktop IPC path had never been shown to work

* **Old behaviour.** Three kinds of evidence existed and none closed the loop: the Rust
  tests assert the wire contract by serialising commands' return values; the frontend
  tests render the real screens with the IPC module mocked; and `--selftest` returns
  before the Tauri builder is constructed. All three can pass while the application —
  window, webview, `invoke`, command, engine — is disconnected.
* **Behaviour now.** `scripts/verify-ipc-roundtrip.sh` launches the **real** application
  against an isolated `OHM_CONFIG_DIR` and the simulator, built the way the project
  ships (`npx tauri build --no-bundle`). The backend asks the frontend to exercise the
  command surface **over Tauri's own event channel** — the one that carries snapshots to
  the UI, not injected `eval` — and the frontend, running in that webview, calls the same
  `api` functions the UI uses and reports what it saw back through a real command, which
  writes it next to the app's log and audit trail.
* **What it found before it worked.** That a *debug* build points at `devUrl` and expects
  the Vite dev server, so running the binary alone loads an empty page — no error, no
  frontend, no IPC; and that a bare `cargo build --release` does not enable Tauri's
  `custom-protocol` feature, so the release binary embeds no assets either. Only the
  CLI's build produces an application whose webview runs the bundle. Both were invisible
  from "the process started", which is exactly the conclusion the harness exists to
  prevent.
* **Evidence.** `IPC ROUND TRIP VERIFIED`, with 12 command steps all successful, the
  webview reporting `tauri://localhost`, the handover states observed as
  `fan.mock.1=pending -> fan.mock.1=failed -> fan.mock.1=confirmed`, the write returning
  `unconfirmed` with `applied` absent, and the audit trail agreeing; the only devices
  written to were `*.mock.*` and the simulated OpenFan. A new simulator fault
  (`unconfirmed_writes_on`, injectable per channel through `mock_set_channel_fault`)
  makes "accepted but never confirmed" producible end to end without a physical fan.
* **Recorded, not smoothed over.** `mock_set_channel_fault`'s *return value* reached the
  frontend as `undefined` while its effect was applied (proved by the unconfirmed write
  that followed). The harness therefore asserts effects, not return values, and this
  stays an open observation.

### The verification pass

Environment: macOS 26.6.2 (25G83) arm64, pinned `rustc 1.98.0` / `cargo 1.98.0` /
`clippy 0.1.98`; the owner's default toolchain, `PATH` and shell configuration untouched;
`cargo-deny 0.20.2`; Node 26.8.1 / npm 11.19.0.

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.0 cargo fmt --all -- --check` | exit 0 |
| 2 | `rustup run 1.98.0 cargo clippy --workspace --all-targets -- -D warnings` | **exit 0, no warnings** |
| 3 | `rustup run 1.98.0 cargo build --workspace --all-targets` | exit 0 |
| 4 | `rustup run 1.98.0 cargo test --workspace` | **477 passed, 0 failed, 0 ignored**, across 44 test binaries and doc-tests |
| 5 | `cargo deny check` | `advisories ok, bans ok, licenses ok, sources ok` |
| 6 | `npm run typecheck` / `npm run test` / `npm run build` | exit 0 / **5 files, 42 tests passed** / exit 0, `index-*.js` 393.71 kB |
| 7 | `ohm-cli doctor --mock`, `demo --steps 60`, `audit`, `handovers`, `handovers --retry`, `protocol`, `ohm-desktop --selftest --mock` (and `--dry-run`), `rules check` on the four examples | all exit 0; the demo closed its loop at 48 % / 1169 RPM with 45 writes and 0 fallbacks |
| 8 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** — 12 command steps succeeded, webview `tauri://localhost`, handover states `fan.mock.1=pending -> fan.mock.1=failed -> fan.mock.1=confirmed`, write `unconfirmed` with `applied` absent, audit trail agreed, only simulated devices written |
| 9 | `docs/windows-validation/package/scripts/tests/run-script-tests.ps1` | **18 cases, 151 checks, 0 failures**; the harness states in its own header that doubles are not Windows evidence |
| 10 | `scripts/make-acceptance-package.sh` | exit 0 on a clean tree; the package is described in §5 |

Isolation: every run used `OHM_CONFIG_DIR` under a throwaway root. The first run of this
pass reported that the **real** per-user config directory existed, which the round-3 pass
had recorded as absent — a defect in this round's own harness: its version probe ran the
application without `OHM_CONFIG_DIR`, so the app fell back to the real config directory
and wrote mock-device audit records into it (`fan.mock.*` and the simulated OpenFan only —
no real device was touched). The probe now gets its own throwaway directory, the harness
checks before and after that the real directory has not appeared, the accidental directory
was removed to restore the state round 3 verified, and its 104 audit lines are kept as the
record of what happened. A final run then reported the real directory absent again.

### What round 4 could **not** verify here

* **Windows — still nothing.** No workflow was pushed or triggered, so every
  `windows-latest` job remains **Prepared**, and no NSIS installer has been built
  anywhere.
* **A physical fan.** Unchanged and still the largest gap. The read-back confirms what a
  *provider* reports as a channel's set point, never that air moved.
* **The desktop on anything but this machine.** The round trip was verified on macOS with
  the simulated provider. It says nothing about Windows, about a real GPU or SuperIO
  chip, or about what the window looks like — no screenshot was taken, and the evidence
  is the DOM-level account the frontend reported plus the app's own records.
* **The `undefined` return value** noted above.

### Deliberately **not** done in round 4

No hardware was written to, no autostart or device-control setting was changed, and
nothing was installed on the host. Nothing was pushed, published, tagged or triggered;
no remote CI was touched. The licence decisions (ADR 0002, ADR 0004) remain **Proposed**.
No new architecture: no CPU native collector, no cross-adapter identity work, no plugin
loading.

---

## 2026-09-14 — round 5: the self-test enforces its own isolation, and packaging stops overwriting evidence

**Revision: the commits `65eb335`, `f520685` and the documentation commit that contains
this entry.** Unlike earlier rounds the code changed under the probe's own feet, so the
numbers below are from the **targeted** runs named beside them, not from one full pass:
the full-workspace suite was run at `65eb335` (485 passed), and the frontend, the real
round trip and the packager harness were each run at the revision named in their row.

### 1. The IPC self-test trusted its launcher, and four holes followed

The probe is a tool that *drives* the command surface — including commands that write — so
its safety cannot depend on how it was started. It did.

* **`--ipc-selftest` alone was enough.** It armed the probe against whatever configuration
  directory the app would have used: `ConfigPaths::discover()`, which is the **real
  per-user** directory on a normal machine.
* **`--mock` was not isolation.** It only *adds* the simulated provider; the settings still
  decided whether LibreHardwareMonitor, the operating system provider and NVML were live.
  The round-4 harness wrote its settings with the mock provider enabled and never
  disabled the others, so every "isolated" probe run in round 4 had four adapters and
  fifteen devices — real provider code paths, in a run whose whole purpose was to touch
  nothing real.
* **The probe re-armed handovers globally.** `rule_retry_handovers` re-arms *every* failed
  handover in the engine, and the probe called it even when the operations meant to create
  its handover had failed. On a real installation that would put a real channel's
  abandoned duty back in the queue with a fresh attempt budget.
* **`ipc_probe_report` did not check that a self-test had been asked for.** It fell back to
  `state.paths.root()`, so an ordinary launch could write a "self-test report" into its own
  configuration directory — the user's.

**Behaviour now.** `ProbeIsolation` decides before anything exists: it runs above
`paths.ensure()`, before settings are applied and before a runtime, an engine or any
control loop is constructed, and it *refuses* rather than falling back to anything —

* an explicit configuration directory is required; it must exist; it must be neither the
  real per-user config directory nor anything inside it (a missing directory is refused
  rather than created, because creating configuration is the side effect the check exists
  to prevent);
* the settings in it must not enable a real provider, and the refusal names the ones it
  found and the `disabled_adapters` entry that fixes it;
* it must enable a simulated one, or the probe has nothing to exercise;
* the run then constructs `AdapterOptions::simulated_only()` regardless. The harness's own
  run now reports **2 adapters / 7 devices** where round 4 reported 4 / 15.

At the command layer: the report refuses unless the probe was armed; the event that asks
the frontend to run the probe was already probe-only; and `rule_retry_handovers` takes an
optional channel — named, it re-arms exactly that channel, which is the only form a probe
may use, and the global form is refused inside a probe run (ordinary users keep it).

The frontend probe can no longer paper over a failure: each fault injection returns the
simulator's answer and the fault is verified in force and verified cleared, and the retry
happens only if the steps that create the handover actually succeeded.

**Correction to round 4.** That entry recorded `mock_set_channel_fault` returning
`"undefined"` as an unexplained observation, and said the harness "asserts effects, not
return values". The cause was in the frontend: `injectFault` awaited the response and
dropped it, and the report rendered the absence as a string. The backend always returned
`Option<MockStatus>`. It returns the response now, and the step is checked. The earlier
note was wrong about where the fault was, not merely incomplete.

**Evidence.** Seven new desktop tests cover the refusals (no configuration directory, the
real one, one inside it, a missing one, a real provider enabled, no simulated provider,
and the accepted isolated case). The harness gained the end-to-end refusals — standalone
`--ipc-selftest` exits 1 and names `OHM_CONFIG_DIR`; a settings file that enables LHM exits
1 and names `lhm`; neither writes a report — and a **seeded failed handover for a channel
that looks like real hardware** (`fan.lhm.0`, `failed`, 3 attempts, in the recovery record
before launch). After the whole run that item is still `failed` with 3 attempts, while the
probe's scoped retry returned exactly `1` for its own channel, and the real per-user config
directory was absent before and after.

### 2. Visual acceptance: attempted, and **not** achieved

This machine can capture the screen (`screencapture`, an Aqua session, a 3840×2160 PNG),
so the harness gained an opt-in capture and the probe now renders its findings in the
application's own window (`IpcProbePanel`) so that a picture of the window would show the
round trip's results rather than merely that a webview exists.

The capture could not be used, and this is recorded rather than dressed up: the image was
a **whole-screen** capture, and reading it (by OCR, since this model cannot view images and
the configured vision bridge failed on both providers, twice) showed unrelated private
content — another application with names and conversations — while the region I cropped to
did not contain the application window at all. The file and every copy were deleted. The
capture is therefore opt-in, never kept unless `--keep` is passed, documented as
potentially containing anything else on the screen, and **excluded from every package**.
Visual acceptance remains unverified, and the probe's own account of what it rendered is
not offered as a substitute for a picture.

### 3. Packaging: evidence must not be overwritten, and must come from the commit

The round-4 packager began by deleting its target and took the entry-point templates from
the **working tree** while taking the source from the commit. A second package of the same
commit therefore destroyed the first, and `--allow-dirty` could ship uncommitted content.
It also declared success before verifying, so a run that failed its own checks still left a
directory named `...-windows-acceptance`.

**Behaviour now.** Staging first, delivery last: everything is assembled in
`dist/acceptance/.staging-<sha>-XXXX/`, checked there, and promoted with one `mv` only if
every check passed. An existing package or archive is refused, with `--build-id` offered
for a build that needs its own name, and nothing in `dist/acceptance` is ever removed. Every
file — source, entry-point templates and README — comes from the commit via
`git show <sha>:<path>`, and the two copies of each entry point are asserted byte-identical
so a future slip fails loudly. The dirty-tree rule now separates two facts that are not the
same: a **modified tracked file** means the operator's tests ran against something other
than the commit (refused unless `--allow-dirty`, recorded either way), while an **untracked
file** cannot enter the package at all and is recorded rather than blocking the run.

`EVIDENCE.md` no longer claims anything passed. It said "Passed on macOS at this commit",
which was wrong twice — the count went stale, and the tests are run at the revision the log
names, not necessarily the revision being packaged.

**Correction to round 4**, in full. That entry's pass table recorded the packager step as
"exit 0 on a clean tree". It exited **1**: the tree had one untracked directory (the
unrelated `k10max-prospector/`, which is not mine and is still untouched), the old packager
counted any uncommitted path as dirty and refused. The package for that round was then
produced by a **separate** `--allow-dirty` run, which the entry did say — but the table row
was still wrong. And the round-3 package is gone: its original archive was
`cb6ac7fe8eeba8be7fc3bc8244dffce817c70ed43620d1366bdf4c9cd2ec3d9c`, my own
`rm -rf dist/acceptance` during round 4 deleted it, and the same-named package that exists
now (`53f76a4f…`) is a rebuild from the same commit `27ee667`, not the original artefact.
The original is unrecoverable; no timestamps were faked to make a hash match; nothing was
deleted to hide it. The packager can no longer do this to any package.

### 4. What was run, and where it ran

| # | Command | Revision | Result |
|---|---|---|---|
| 1 | `rustup run 1.98.0 cargo test --workspace` | `65eb335` | **485 passed, 0 failed** |
| 2 | `cargo clippy --workspace --all-targets -- -D warnings` and `cargo fmt --all -- --check` | `65eb335` | both clean |
| 3 | `npm run typecheck` / `npm run test` / `npm run build` | `65eb335` | clean / **5 files, 42 tests** / clean |
| 4 | `scripts/verify-ipc-roundtrip.sh` | `65eb335` | **IPC ROUND TRIP VERIFIED**, with the refusal paths, the seeded real-channel handover untouched, and the real per-user config directory absent before and after; the successful probe run reported 2 adapters / 7 devices |
| 5 | `scripts/tests/make-acceptance-package.test.sh` | `f520685` | **4 cases, 20 checks, 0 failures** |
| 6 | `scripts/make-acceptance-package.sh` (smoke run, `--allow-dirty`) | `65eb335` | exit 0; both pre-existing packages untouched |

Writing the packager harness found two further defects in the packager, both fixed: a
`grep` that matched nothing killed the run **silently** under `set -e` with `pipefail`
(no message, no output — the failure looked like an empty run), and two `find … | grep -q`
probes relied on operator precedence they should not have.

### What round 5 could **not** verify

* **Windows — still nothing.** No workflow was pushed or triggered; every
  `windows-latest` job is **Prepared**.
* **Visual acceptance of the desktop** — see §2. Attempted; the capture was unusable and
  deleted; still unverified.
* **A physical fan** — unchanged: no write has reached real hardware.
* The full round-4 discrepancy list is unchanged: `mock_set_channel_fault`'s return value
  is now explained and fixed, but the wider statement stands — nothing here has been
  exercised on Windows.

### Deliberately **not** done in round 5

No hardware was written to, no autostart or device-control setting was changed, nothing was
installed on the host, and nothing was pushed, published, tagged or triggered. The licence
decisions (ADR 0002, ADR 0004) remain **Proposed**. The unrelated untracked directory
`k10max-prospector/` was left exactly as found.

---

## 2026-09-14 — round 6: the published pipeline, the first Windows CI run, and v0.1.2

**Revision: `2dfb340`.** Everything in the pass below was run at that commit in one go
(`/tmp/ohm-verify/pass-v0.1.2.sh`), with the working tree clean apart from the untracked
`k10max-prospector/`. The documentation commit that contains this entry follows `2dfb340`
and changes no code, no packaging and no version number.

### 1. The repository was pushed, and the first CI run was red

Round 5's last commit (`4a72112`) was the first thing ever pushed, so it produced the first
CI run this project ever had. macOS, Linux, the frontend, clippy and the licence audit
passed; **both Windows jobs failed**, and the log names the reason:

```
error[E0433]: cannot find `COMLibrary` in `wmi`
  --> adapters\system\src\windows.rs:67:20
error[E0061]: this function takes 0 arguments but 1 argument was supplied
  --> adapters\system\src\windows.rs:68:22
error: could not compile `ohm-adapter-system` (lib) due to 2 previous errors
```

`adapters/system/src/windows.rs` sits behind `#[cfg(windows)]`, so no build on this machine
had ever read it: `wmi` 0.18 had removed `COMLibrary` and changed `WMIConnection::new()`.
Five rounds of "Windows: Prepared" had never included "Windows: compiles", and nothing in
those rounds could have caught it. After the API fix (`34db43b`) the next Windows run failed
on a **test** instead:

```
test recovery::tests::saving_into_an_unwritable_place_fails_loudly ... FAILED
test result: FAILED. 91 passed; 1 failed
panicked at crates\ohm-automation\src\recovery.rs:311:28:
writing into a read-only directory must fail: ()
```

That test was written in round 2 and encoded Unix semantics — a read-only *directory* stops
a write; on Windows it does not. It was replaced with a platform-independent case
(`6e37954`). A third Windows-only defect followed in the release workflow (Cargo metadata
decoded as non-UTF-8, `e64fe22`), after which every run has been green and v0.1.0 and v0.1.1
were published by the maintainer.

**The gate that was missing** is added this round: `cargo check --locked --target
x86_64-pc-windows-msvc -p ohm-adapter-system` type-checks the crate that broke, from macOS,
against the real `cfg(windows)` code, with no Windows SDK. It cannot cover the whole
workspace — `ohm-adapters → ureq → rustls → ring` compiles C for MSVC, which no non-Windows
host can do — so `apps/desktop`'s Windows files (`autostart.rs`, `shell.rs`) still depend on
the CI job. That is stated as a dependency, not as verification.

### 2. Two defects in the published artefacts themselves

Both were found by auditing the **released files** rather than the repository.

* **`SHA256SUMS` was CRLF.** `package-windows.ps1` wrote it with
  `[IO.File]::WriteAllLines`, which uses the host newline. On macOS and Linux `shasum -c
  SHA256SUMS` then reads each line as `<digest><two spaces><name>\r` and reports **every**
  entry as a missing file. The digests were always right: the published CLI archive hashes
  to `7640fab80689fd31005971ac73cf730a0fde2c7cafe679d77b92dfff26954934`, exactly the value
  in the list. Fixed by test, not by reading.
* **`install.ps1` was not the committed script.** It was copied out of the Windows checkout,
  whose `core.autocrlf` had converted it, so the published file was CRLF while the blob is
  LF. The content is identical and the digest is not reproducible from the repository: the
  published `493f8adae428f77e970364a8aab68a502652ce6e207c8b9e28d85de28d1e1a53` equals the
  committed blob `ddefa83800bbb40f5e3ad41874d4f5e3e16ad512628ecd577a510cb1c6782950` only
  after `tr -d '\r'`.

`package-windows.ps1` now normalises that script, proves the result **is** the blob in this
commit (`git hash-object` against `git rev-parse HEAD:scripts/install.ps1`), writes
`SHA256SUMS` with LF, and refuses to produce a package when either check fails — a genuinely
CRLF blob fails loudly instead of being published as something nobody can trace.
`.gitattributes` (`* text=auto eol=lf`) makes a checkout reproduce the committed bytes on
every platform, so the class of problem cannot return through another copied file. A new
fixture harness, `scripts/release/test-packaging.ps1`, runs before packaging in the release
workflow.

### 3. The toolchain was pinned in CI, not in the repository

CI and the release workflow both set `RUSTUP_TOOLCHAIN: 1.98.1`, this machine used a locally
installed `1.98.0`, and `rust-version` says `1.95`. `rust-toolchain.toml` now pins the exact
release CI uses. The qualification is measured rather than assumed: inside the repository
`rustup show active-toolchain` reports `1.98.1 (overridden by …/rust-toolchain.toml)`, while a
plain `cargo --version` still reports **1.98.0 (Homebrew)** — `/opt/homebrew/bin/cargo` is not
a rustup shim and never reads the file. The pass therefore invokes `rustup run 1.98.1`
explicitly, and outside the repository the owner's default (`stable`, 1.92.0) is unchanged.

### 4. Documentation that had stopped being true

* `docs/roadmap.md` claimed "real Windows hardware validation (nothing has run on Windows
  yet)" and `docs/requirements.md` said the same in three places. Since v0.1.0 CI has built,
  tested and installed on `windows-latest`. Both now separate the two claims that were being
  conflated: software on Windows *has* run; hardware on a machine somebody uses has not.
* `requirements.md` no longer describes the project as being at a deliverable stop, and
  closes the Windows-compile gap in place (item 11) rather than leaving it implicit.
* `docs/windows-validation/README.md` now separates "CI has run" from "a target machine has
  run" row by row — the NSIS bundle is *built* on CI and has still never been installed.
* The six-stage hardware plan, which existed only as GitHub issues #1–#7, is versioned in
  `docs/plans/hardware-support/` (commit `3dc235f`, before this pass): seven files, each body
  byte-identical to its issue, plus the authority rule and the three label sets (C1–C8,
  stage 1–6 / S1.1–6.7, PUMP-01–PUMP-04) written down so they cannot be misread as one.

### 5. v0.1.2

Prepared with the project's own tool — `scripts/versions.py prepare v0.1.2 --branch
codex/v0.1.2 --apply` — which synchronized the workspace, the internal dependencies,
`Cargo.lock`, the desktop npm package and its lock, and `tauri.conf.json`, and moved the
catalogue node from planned to development. v0.2.0 (pump support) stays planned and now
hangs off v0.1.2 in the tree. README and `docs/windows-install.md` name v0.1.2, which the
public install check requires of the tagged README.

### The verification pass (all commands re-run at `2dfb340`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `rustup run 1.98.1 cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0, no warnings |
| 3 | `rustup run 1.98.1 cargo test --workspace --locked` | **488 passed, 0 failed, 0 ignored**, across 44 test-result lines including doc tests |
| 4 | `cargo-deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check` and `check --generated` | OK: 4 catalogue entries; application version 0.1.2 |
| 6 | `python3 -m unittest scripts/tests/test_versions.py` | **30 tests, OK** |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / **5 files, 42 tests** / clean, 397.20 kB bundle |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | **4 cases, 20 checks, 0 failures** |
| 9 | `scripts/release/test-packaging.ps1` (new) | **6 cases, 0 failures** |
| 10 | `docs/windows-validation/package/scripts/tests/run-script-tests.ps1` | **18 cases, 151 checks, 0 failures** (test doubles only) |
| 11 | `cargo check --locked --target x86_64-pc-windows-msvc -p ohm-adapter-system` | exit 0 — the new local gate |
| 12 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** at app version 0.1.2: real window, real webview, real commands; the probe reported 2 adapters / 7 devices; the real per-user config directory was absent before and after; the seeded `fan.lhm.0` handover was still failed with 3 attempts while the scoped retry returned 1 |
| 13 | `shasum -a 256 -c MANIFEST.sha256` in the `4a72112` and `5fd8c23` packages | every file still matches |

### What round 6 could **not** verify

* **Windows on a machine somebody uses** — unchanged, and now stated precisely: CI runs exist
  and are linked from `docs/versions.json` and the acceptance entry, but a runner is not
  hardware acceptance and no fan or pump has ever responded to a write.
* **The v0.1.2 artefacts do not exist.** Nothing was published in this round: no tag, no
  GitHub Release, no uploaded asset. Everything above verifies the source and the packaging
  logic; the Windows CLI, the NSIS installer and the LF `SHA256SUMS` become facts only when
  the release workflow runs on `windows-latest` and the maintainer publishes. In particular
  `scripts/release/test-packaging.ps1` proves the packaging steps and their guards against
  fixtures — **not** that a Windows build succeeds.
* **Visual acceptance of the desktop** — unchanged: attempted in round 5, unusable, still
  unverified.
* **A physical fan** — unchanged.
* **The Windows-target gate's coverage** — `ohm-adapter-system` only, for the `ring` reason
  in §1.

### Deliberately **not** done in round 6

No hardware was written to; no autostart or global environment value was changed; nothing was
installed on the host; nothing was pushed, tagged, published or triggered remotely; no
licence was approved on the user's behalf. The unrelated untracked directory
`k10max-prospector/` was left exactly as found. ADR 0002 and ADR 0004 remain **Proposed**.

---

## 2026-09-14 — round 7: v0.1.2 is published, and three control claims are made honest

**Revision: `a885fd0`.** The pass below ran at that commit in one go
(`/tmp/ohm-verify/pass-r7.sh`) with 0 non-zero steps. The publication in §1 happened before it,
from `release/v0.1.2` at `1219457`; the commit carrying this entry follows `a885fd0` and changes
documentation only.

### 1. v0.1.2 is published, and the packaging fixes are proven on Windows output

The first release build for v0.1.2 **failed on my own new fixture suite** (step 19) and produced
no assets. The test read the committed blob as *text* (`git cat-file | Out-String`) and asserted
it held no CR: PowerShell joins a native command's output with the host newline, so on Windows
the blob reads as CRLF whatever it contains, and the run failed on a repository that is entirely
LF. Two further problems came out of the same investigation: `git hash-object <file>` applies
the end-of-line conversion to a *file* argument, so a CRLF file hashes like its LF blob, and the
assertions were text-based where they had to be byte-based. Fixed in `1219457`; the suite now
compares sizes and raw-byte hashes (`--no-filters`), and passes on the Windows runner.

v0.1.2 = `1219457`, tagged and published as a preview with seven assets. The duplicate build the
tag push triggered was cancelled; the reviewed build is [run 34839424804](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34839424804).

Verified from the **download**, not from the workflow's own claim:

* `SHA256SUMS` is LF-only, and `shasum -a 256 -c SHA256SUMS` validates all six entries — the
  check that failed on *every* entry for v0.1.0 and v0.1.1.
* the published `install.ps1` hashes to the committed blob `f2617d08`, which is the **same blob**
  v0.1.1 published as CRLF (`493f8ada…`). The commit did not change; the publication did.
* `release.json` names `source_commit = 1219457…`, `version = v0.1.2`, `rustc 1.98.1`.

The public install check passed on Windows PowerShell 5.1
([run 34840668305](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34840668305)):
download, checksum, versioned install, source-commit check, `doctor` and a ten-step simulated
demo.

CI failed **once** on the release commit:
`writeStates.test.tsx > manual control: a write the device never confirmed > reports the request
as unconfirmed` never saw its `writeCapability` call. It passed on re-run (all eight jobs) and
passes locally (42/42). The helper's change-then-click sequence is a race in the test, not in the
product; it is recorded here rather than re-run away, and fixed in the next version.

A source package was built at the release commit: `OpenHardwareOS-1219457-windows-acceptance`,
245 files, archive `c11c0849fcfc08b06de27d7ad6a29f273526412c5e8a64474f11f54d33580cd1`, manifest
`a259c6f17899cd1de2bced370fd44828067b45eea46e9c31c0fc6a2ba447c118`.

### 2. The exit path reported what it attempted as what it achieved

`release_control()` returned a count of writes it had *tried*, and `shutdown()` recorded that
number as "outputs handed back to firmware". A write the adapter accepted but could never read
back — a board that answers `200 OK` and has no readable channel, which is what real hardware
produces — was therefore audited as a successful release.

Reading that code found a second, worse defect: the target list came from
`DeviceTable::cooling_devices()`, which is `Fan | Pump`. A GPU is a `Gpu` device with a writable
fan channel, so **a GPU fan under the runtime's control kept its last duty on exit** while the
audit said everything had been handed back. On this simulated machine the exit path now drives 3
channels instead of 2; restoring `cooling_devices()` makes the new regression test fail with
"2 of 3", which is how the fix was checked.

`ControlRelease` (`crates/ohm-runtime/src/release.rs`) keeps confirmed / unconfirmed / refused /
failed / simulated apart, with `problems()` and `caveats()` for the two kinds of bad news, and
`Runtime::shutdown` writes one audit entry per claim: `control_released`,
`control_release_unconfirmed`, `control_release_failed`, `control_release_skipped`,
`control_relinquished`, `control_not_handed_back`, `adapter_shutdown_failed`. `runtime_stopped`
says "clean shutdown" only when it was.

`AdapterCapabilities::hands_back_control_on_shutdown` exists because `HardwareAdapter::shutdown`
defaults to `Ok(())`: an adapter that does nothing returned success and was counted as a
hand-back. LHM opts in (it sends `SetDefault` per channel), the mock and the simulated OPD
transport opt in, and **NVML stays out** — it has no `shutdown` implementation, so its channels
are now reported as *not* handed back instead of being claimed as released. That gap is
`docs/requirements.md` item 12.

Seven tests in `tests/tests/control_release.rs` pin each claim: a confirmed write is a release; a
simulated one is not a confirmed one; an unconfirmed one is never counted as released and its
audit entry says "not a confirmed release"; a refusal is a refusal and not a failure; with
`relinquish_on_exit` off nothing is written and nothing is claimed; an adapter that does not hand
back is listed as such; a failed hand-back is reported separately from the write that succeeded.

### 3. LHM fan channels were paired by coincidence

The pairing took a number from the display name when there was one and otherwise from the sensor
*path*, then keyed both sides into a `BTreeMap`. Two consequences: two sensors resolving to the
same key silently replaced one another (a board reporting two `Fan #1` sensors, or several
sensors with no number at all, lost a channel from the model without a word), and a tachometer
named `Fan #1` could be paired with a control numbered by its *position*, which is an enumeration
artefact rather than an identity — and the control is what a rule writes to.

The anchor is now explicit (`Anchor::Name` / `Anchor::Path`), nothing is merged, and a channel
whose control cannot be shown to belong to the tachometer beside it is **read-only**: the
tachometer stays visible, the control is withheld, and the reason lands on the device
(`lhm_control_withheld`) and on the adapter's status, which `probe()` reports as Degraded with the
specific reason. An unnumbered control is not exposed as a device at all — there would be nothing
to show and nothing anyone could safely write — but it is reported, not hidden.

Four new mapping tests: duplicate numbers, a control paired only by path position, unnumbered
sensors kept apart instead of merged, and reorder/reconnect leaving the pairing and the
writability unchanged.

**Still open, and stated as such:** the device *id* is positional (`fan.lhm.<n>` over the tree
order), so a reorder can still change which physical channel a saved rule targets. That half needs
a stable identity in the id itself, which is what rules, audit entries and the UI all key on, and
is not changed in this round.

### 4. A provider was chosen by intent, not by availability

NVML was not constructed at all when the settings enabled LibreHardwareMonitor:

```rust
nvidia: settings.adapter_enabled(NVML) && (nvidia_forced || !lhm_enabled)
```

Both describe the same GPU, so one must step aside — but the decision came from *intent*. On
Linux, and on any Windows machine with LibreHardwareMonitor closed, enabling LHM therefore
removed the only GPU provider, and nothing in the UI said why.

The relationship is now declared (`AdapterInfo::yields_to`) and the decision is made from
evidence: `DiscoveryManager::discover_all` probes every adapter first, then leaves a fallback out
of the cycle while its primary probed usable *in the same cycle*, reporting it as
`Unavailable` / `disabled` with the reason and the setting that forces it. Because it is
re-decided every cycle, a primary that appears or disappears later is handled without a restart.

Three tests in `tests/tests/provider_fallback.rs` drive the hand-over both ways and back; the
`crates/adapters` tests cover the settings mapping and that the declaration reaches the adapter.

### The verification pass (all commands re-run at `a885fd0`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `rustup run 1.98.1 cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0, no warnings |
| 3 | `rustup run 1.98.1 cargo test --workspace --locked` | **508 passed, 0 failed, 0 ignored**, across 46 test-result lines including doc tests |
| 4 | `cargo-deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check --remote --generated` | OK: 4 catalogue entries; **published releases verified on GitHub** |
| 6 | `python3 -m unittest scripts/tests/test_versions.py` | **30 tests, OK** |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / **5 files, 42 tests** / clean, 397.20 kB |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | **4 cases, 0 failures** |
| 9 | `scripts/release/test-packaging.ps1` | **6 cases, 0 failures** |
| 10 | `docs/windows-validation/package/scripts/tests/run-script-tests.ps1` | **18 cases, 151 checks, 0 failures** (doubles only) |
| 11 | `cargo check --locked --target x86_64-pc-windows-msvc -p ohm-adapter-system` | exit 0 |
| 12 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** — real window, real webview, real commands; the seeded `fan.lhm.0` handover untouched; the real per-user config directory absent before and after |
| 13 | `shasum -a 256 -c SHA256SUMS` on the **downloaded** v0.1.2 assets | all six entries OK |
| 14 | `shasum -a 256 -c MANIFEST.sha256` in the `1219457`, `4a72112` and `5fd8c23` packages, plus the `1219457` sidecar | every file still matches |

### What round 7 could **not** verify

* **Windows on a machine somebody uses** — unchanged. The CI runs are evidence about software
  (build, tests, packaging, install), not about any physical fan or pump.
* **NVML still hands nothing back on exit** (requirements item 12). It is now *reported* instead of
  being claimed as a release; making it true needs NVML's default-fan-speed call and a fake-NVML
  harness the adapter does not have.
* **The positional device-id half of the LHM identity problem** (§3).
* **The flaky desktop test's root cause** — observed once on a CI runner, not reproduced here.
* **Visual acceptance and a physical fan** — unchanged.

### Deliberately **not** done in round 7

No hardware was written to; no autostart or global environment value was changed; nothing was
installed on the host; no licence was approved on the user's behalf; ADR 0002 and ADR 0004 remain
**Proposed**; the unrelated untracked directory `k10max-prospector/` was left exactly as found.

Publication — tag, GitHub Release and the CI dispatches — **was** this round's objective and is the
first time anything left this machine. It was confined to the project's own release flow, from a
reviewed build whose `release.json` names the commit the tag points at, and the two packages the
maintainer already published (v0.1.0, v0.1.1) were left untouched.

---

## 2026-09-15 — round 8: Linux monitoring, the first release that ships Linux, and two defects the release build found

**Revision: `29f1c34`.** Every number below comes from a pass run at that commit
(`~/.local/share/ohm-verify/pass-r12.log`), the commit the tag points at. The entry
follows it and changes documentation only.

### 1. Linux fan tachometers, from the kernel's own interface

The system adapter had declared no fan capability on any platform, with a reason: "no OS API
exposes chassis tachometers or PWM control on consumer hardware". On Linux that is not true — the
kernel's **hwmon** subsystem exposes `fan<N>_input` (and `pwm<N>`) for every board whose driver
implements it, which is exactly the interface the platform hands us.

`adapters/system/src/hwmon.rs` reads it. The module takes the directory as an argument
(`/sys/class/hwmon` in production, a fixture tree in the tests) and does pure path and parse work,
so the whole path is exercised on macOS — the pattern the LHM adapter already uses for its fake
server.

* **Read-only, deliberately.** Writing `pwm<N>` needs root, and which `pwm` maps to which physical
  header depends on the board and driver; a wrong value there is the classic "fans stop" failure.
  The current duty is exposed as a *sensor* (`fan.pwm`), and `pwm<N>_enable` is reported as words —
  "the driver controls this channel", "software is expected to set this channel", or the number
  verbatim when this build does not recognise it.
* **Identity is the chip name and channel number** (`fan.system.nct6798d_fan1`), never an
  enumeration index. A kernel that renumbers `hwmon*` cannot move a rule's target — which is the
  requirement from round 7, met here by construction rather than by repair.
* **Every absence carries a reason**: a missing file (`NotPresent`), non-numeric content
  (`ReadError`), a chip without a `name` (skipped, with a note), a channel that vanishes after
  discovery (offline with its reason), and an unreadable control mode.
* Zero RPM is a *reading*, not a missing sensor — a fan the driver reports as stopped is exactly
  what a user needs to see.

Memory joins as `memory.system.0` with `memory.used` and `memory.total` in bytes: one device,
because the OS reports totals and inventing per-module devices from a total would be a guess.

The cross-target gate now covers Linux as well as Windows
(`cargo check --target x86_64-unknown-linux-gnu -p ohm-adapter-system`): the
`#[cfg(target_os = "linux")]` wiring is invisible to a macOS build, which is the mistake that hid
the Windows compile error for five rounds.

Nine hwmon tests plus four adapter tests; 25 in the crate.

### 2. The two defects the release build found

Both were found by *running the release build more than once* and reading what came back, and both
are the kind this project exists to remove: the software did something other than what it said.

**A reading arriving after an edit replaced what the user typed.** `WriteControl` followed the
device's reported value on every change, and the first snapshot always arrives asynchronously — a
moment after the window opens. Type 80 into the duty field, and the arriving reading put the
device's own 45 back in the box; Apply then wrote **45 %**, the value the device was already at,
while the screen looked entirely normal. The window is a race, so it passed every local run and
failed on a loaded CI runner, whose assertion said it in one line: `expected 80, received 45`.

The field now follows the device only until the user touches it, shows "your change is not applied
yet" while an edit is outstanding, and follows the device again after a confirmed write. An
*unconfirmed* write keeps the request on screen next to a value that is still unknown, which is
the whole point of that panel. The regression test forces the order CI produced — edit first, then
deliver a snapshot — instead of waiting for a slow machine to reproduce it.

**A slow disk was reported as a missing sensor.** The engine asked "are my readings too old?"
*after* doing the tick's own file I/O (persisting handover records and control state). The window
is three poll intervals — 300 ms at the shipped 100 ms — so any filesystem slower than that, which
is what a CI runner scanning every new file is, made every rule fall back to the fail-safe duty.
Both Windows failures were this single cause: a rule reloaded from disk wrote 70 % (the fail-safe)
instead of its curve's 55 %, and a gated rule's verdict read as a missing sensor. Reproduced on
purpose — a blocking 400 ms gap between the poll and the evaluation flips the outcome to `Fallback`
with duty 100 and a message that no longer contains "condition not met", which is the CI failure
reproduced in one process.

The question is now asked **before** the tick does any work of its own, so what the window detects
is the *runtime* falling behind, which is what it was always for. The fallback duty itself is
unchanged: falling back is the safe direction, and the safety supervisor lives in the poll loop
being watched.

**And it says which of the two it is.** Readings that stopped arriving are now reported as "the
runtime has not refreshed … recently, so its readings are too old to act on" rather than as a
missing sensor. One is a runtime fact, the other is hardware or a driver; one sentence for both
sent the user looking for the wrong thing.

The shared test sessions now use a 2 s poll interval, so a test that polls and asserts in one
breath is no longer decided by the host's scheduler; staleness itself stays pinned against a
runtime deliberately configured with a tight interval. This is also why the earlier "CI flake, re-run
green" note was wrong as a *category*: both "flaky" tests were reporting real defects, and labelling
them races cost two rounds.

### 3. A release pipeline that ships Linux, and what its four earlier failures taught

`scripts/release/package-linux.sh` builds the Linux CLI the way the Windows packager does — refuse
an existing output, take the source commit from git, verify before delivering, refuse anything that
is not a 64-bit x86_64 ELF — and the release workflow gained a `linux` job that runs it after the
workspace tests, Clippy, the frontend tests and two fixture suites.

The job had never run, and its first runs failed. Each failure was mine, and each is now a test:

1. **`glib-sys` build failure** — the workspace contains the desktop crate, so a Linux build needs
   GTK/WebKit development packages. Nothing in the repository could have caught this: it happens in
   a dependency's build script.
2. **The notices path** — the generator writes `artifacts/THIRD_PARTY_NOTICES.txt`; the Linux
   packager looked at the repository root. The fixture suite had created its fixtures at the root
   *because the script looked there* — a fixture that copies the mistake it is meant to catch.
3. **`tar -tzf … | grep -q`** — `grep -q` exits on its first match, so tar's next write lands in a
   closed pipe: GNU tar reports a write error and exits non-zero, and `pipefail` failed a check
   that had just passed. BSD tar does not report it, which is why the same script passed here. The
   listing now goes to a file, and the fixture reproduces the GNU behaviour with a `tar` shim that
   writes line by line with a pause — verified by mutation.
4. **Frontend tests "timing out" on the Windows runner** — the 5 s default was measuring the
   machine, and one of the four was not a timeout at all but the defect in §2. `testTimeout` and
   `hookTimeout` are 30 s now; the defect needed a fix, not a longer wait.

A fifth finding came from the pass rather than the release: **RUSTSEC-2026-0285** against
`rustls 0.23.44` (via `ureq`, in the LibreHardwareMonitor adapter), fixed by 0.23.45. The pass also
reported "non-zero steps: 0" while that failure was real — the step piped `runcmd` into `tail`,
which runs it in a subshell where the failure counter is invisible. The harness pipes inside the
command now.

### 4. Publication, and two more faults found by verifying the download

v0.1.3 = `29f1c34`, tagged and published as a preview with Windows **and Linux** assets. Verifying
the downloaded artefacts rather than trusting the job that produced them found two more defects:

* **The Linux checksum list was named `SHA256SUMS`** — the name the Windows assets already use,
  and a GitHub Release holds one asset per name. It is `SHA256SUMS-linux-x86_64` now, and so is the
  name the install page tells users to fetch.
* **The list named files that are never published.** `LICENSE` and `THIRD_PARTY_NOTICES.txt` travel
  *inside* the archive, so a user running the natural `sha256sum -c SHA256SUMS-linux-x86_64` on a
  perfectly intact download was told two files were missing, which reads as tampering. The list now
  contains exactly the published assets (the archive and `release-linux-x86_64.json`), the
  packager's own post-write check stages *only* those files next to it, and the documented route
  runs a whole-list `sha256sum -c` instead of picking the one line it hoped was right.

The Linux archive was verified from the download: an x86_64 ELF, both checksum entries OK under a
whole-list `sha256sum -c`, and `release-linux-x86_64.json` naming the tagged commit. The documented
Linux commands were then run against those real downloaded bytes — download by asset name, verify,
extract — and stopped exactly where they must on macOS: `cannot execute binary file`. The public
Windows install check ran against this tag and passed
([run 34928385056](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34928385056)), and the
acceptance source package for the release commit is
`dist/acceptance/OpenHardwareOS-29f1c34-windows-acceptance.zip`,
`26c99521ac54e1222dafa8c7066c3f64629a2bb3956470d431b7fca7135002e1`. Exact run links and digests are
in `docs/versions.json` and on the release page.

### The verification pass (all commands re-run at `29f1c34`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| 3 | `cargo test --workspace --locked` | **522 passed, 0 failed**, 46 test-result lines |
| 4 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check --remote --generated` | OK; published releases verified on GitHub |
| 6 | `scripts/tests/test_versions.py` | 30 tests, OK |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / 43 tests / clean |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | 4 cases, 0 failures |
| 9 | `scripts/release/test-packaging.ps1` | 6 cases, 0 failures |
| 10 | `scripts/tests/package-linux.test.sh` | 6 cases, 28 checks, 0 failures |
| 11 | `scripts/tests/linux-install-doc.test.sh` | 5 cases, 15 checks, 0 failures |
| 12 | `docs/windows-validation/.../run-script-tests.ps1` | 18 cases, 151 checks, 0 failures (doubles only) |
| 13 | cross-target `cargo check` for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` | exit 0, plus Linux-target Clippy |
| 14 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** |
| 15 | delivered packages still match their manifests | every file |

### What round 8 could **not** verify

* **A Linux machine.** Everything above is macOS, CI runners and fixtures. No reading has been
  cross-checked against `sensors`/sysfs on a real distribution or kernel, and no fan has been read
  from a real board's hwmon.
* **A Linux desktop package** (`.deb`, AppImage): not built yet. v0.1.3 ships the CLI.
* **Writing PWM on Linux**, and therefore Linux fan *control* — deliberately absent (§1).
* **A physical fan or pump**, on any platform: unchanged.
* **The positional device ids in the LHM path** (`fan.lhm.<n>`), the other half of round 7's identity
  work: the Linux path is stable by construction, the LHM path still is not.
* **NVML handing nothing back on exit** (requirements item 12): reported, not fixed.
* **Visual acceptance of the desktop**: unchanged. The §2 input defect was found by an assertion,
  not by looking at the screen; a screenshot check would have caught it earlier.
* **Whether the engine's 3 × poll-interval staleness window is the right size.** It is now asked at
  the right moment, but the question "how old may a reading be before a rule stops following its
  curve" is a safety policy decision that deserves its own analysis rather than a number chosen to
  make a test pass. Recorded as open.

### Deliberately **not** done in round 8

No hardware was written to; no autostart or global environment value was changed; nothing was
installed on the host beyond a portable PowerShell under `~/.local/share/ohm-verify/`; no licence
was approved on the user's behalf; ADR 0002 and ADR 0004 remain **Proposed**; the untracked
directories `k10max-prospector/` and `Prospector/` were left exactly as found (the second appeared
during this round and is not mine).

One process note, recorded because it cost a pass: a verification pass was invalidated by editing
files while it ran — the harness read the new script with the old test in the same step and
reported a failure that belonged to neither revision. Verification and editing are now sequenced,
and the pass's log names the revision it read.

---

## 2026-09-15 — round 9: the Linux desktop package, and what a .deb has to prove

**Revision: `febc02a`** — the pass below ran at that commit, which is the commit
the tag points at. The commits carrying this entry and the round's documentation follow it.

### 1. Half of the Linux preview was missing, and it was the half a user double-clicks

Round 8 shipped the Linux CLI. The goal's Linux preview asked for a CLI **and a first
desktop package**, and "installable without compiling" is a claim about the app, not
about a tarball. This round adds the package and, more importantly, makes the release
pipeline prove things about it that a user cannot check before installing.

`scripts/release/package-linux.sh` now takes `--desktop-deb` and `--desktop-appimage`:

* **Both or neither.** Publishing a `.deb` without the AppImage (or the reverse) would
  leave the install page describing an artefact nobody can download, so the packager
  refuses the pair when only one is given.
* **Published under this project's names** — `OpenHardwareOS-<version>-linux-x86_64.deb`
  and `.AppImage`, not the bundler's `OpenHardwareOS_0.1.4_amd64.*`. The page has to
  name a file whose name does not move when the bundler changes its mind, and both
  platforms then follow the same pattern as the CLI archive.
* **One checksum list for the whole platform** (`SHA256SUMS-linux-x86_64`), because a
  user who downloads two Linux files wants one command that answers "is this intact".

### 2. Reading the package instead of trusting it

`scripts/release/inspect-deb.py` reads a `.deb` — an `ar` container holding
`control.tar.*` and `data.tar.*` — and reports what is inside: the declared package
name, version, architecture and dependencies, every file, the desktop entry and the
program it launches, the icons, and the ELF class and machine of anything under
`usr/bin/`. The packager then requires, before publishing:

* the package declares **this release's version**, the name `open-hardware-os` and `amd64`;
* it installs a **64-bit x86_64 ELF** under `usr/bin/` — the file a user will run;
* it installs a `.desktop` entry whose `Exec` launches *that* binary (field codes and
  quoting allowed, the program compared);
* it installs icons, and declares dependencies at all;
* the AppImage is a 64-bit x86_64 ELF **and executable**, or it cannot be run after
  download.

Reading it in Python rather than with `dpkg-deb` is deliberate: the fixture suites that
guard this path also run on macOS, so the *same* code that validates a real artefact
validates synthetic ones — including deliberately wrong ones. `make-fixture-deb.py`
builds packages that declare the wrong version, the wrong architecture, a shell script
where the binary should be, no desktop entry, an `Exec` that names something else and a
different package name. `package-linux.test.sh` is **10 cases / 54 checks** (was 6 / 28);
removing the version and `Exec` checks fails four of them.

### 3. The defect this found: a refused run made the fix impossible

Probing the new path by hand produced a failure that had nothing to do with the check
being tested: a refused packaging run left a half-written `artifacts/release` behind,
and since the packager refuses to write into an existing output — the rule that stops
one release overwriting another — the *corrected* rerun refused to start. A fixable
mistake became a permanent one, and every earlier failed Linux job will have left such a
directory on its runner. The trap now removes what the failed run created, a case proves
nothing is left behind for each refusal, and another proves a corrected retry delivers.

### 4. The one place where guessing was wrong

The first Linux job to reach the new step built both bundles successfully and then the
packager refused: the Debian package is named **`open-hardware-os`** (the bundler
kebab-cases `productName`; dpkg requires lower case), not `openhardwareos` as I had
written. Three things came out of that single line of output:

* the expected name is now `open-hardware-os`, enforced with the reason recorded — the
  install page tells users what to remove, and a bundler upgrade that renamed the package
  would silently break that sentence;
* `mainBinaryName: "openhardwareos"` in `tauri.conf.json` makes the installed executable
  explicit rather than derived, so `/usr/bin/openhardwareos` on the install page is a
  fact instead of a hope;
* the packager now prints everything the package declares **before** judging it, so one
  release cycle answers every question. The previous cycle could not: it refused with one
  sentence and no facts, which is why the fix needed a second look rather than a second
  run.

### 5. Installing it on a real distribution, in the release run

The Linux release job now builds the bundles, and then, on `ubuntu-latest`:

* installs the packaged `.deb` with `apt-get install ./<file>` — the documented route, so
  dependencies are resolved the way a user's machine resolves them;
* derives the installed path from the package itself and runs
  `/usr/bin/openhardwareos --selftest --mock`, and once with `--dry-run`;
* runs the packaged AppImage the same way (`APPIMAGE_EXTRACT_AND_RUN=1`, so no FUSE is
  needed);
* prints the runner's distribution and kernel, so the claim "verified on X" names the X
  that this run actually was.

The run that did it: [34931865680](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34931865680),
which reports `PRETTY_NAME="Ubuntu 24.04.5 LTS"` and kernel `6.17.0-1022-azure`, prints
`installed as /usr/bin/openhardwareos`, and ends both steps with the application having
started, read simulated hardware and exited. The same run's log prints what the package
declares, so the release page's claims are read from the build rather than remembered:

```text
desktop package : open-hardware-os 0.1.4 (amd64)
desktop binary  : usr/bin/openhardwareos (64-bit x86_64)
desktop entry   : usr/share/applications/OpenHardwareOS.desktop -> openhardwareos
desktop depends : libayatana-appindicator3-1, libwebkit2gtk-4.1-0, libgtk-3-0
```

That is the evidence for "installable without compiling": the artefact a user downloads,
installed and started on a distribution the log names. What it is not: a desktop session.
`--selftest` returns before the Tauri builder is constructed, so the window, the tray and
the menu entry remain unverified — which the install page says in as many words.

### 6. Documentation that runs

`docs/linux-install.md` now carries three routes (CLI, `.deb`, AppImage) and states what
the release run verifies and what it does not. Two changes there came from the fixture
suite rather than from review:

* the checksum list covers **every** Linux asset of the version while each route downloads
  one file, so the documented commands now verify the file being installed (exactly one
  matching entry, then compare that digest) instead of failing on the three files the user
  did not need. The suite runs a fixture whose list names four files and proves the
  commands pass;
* the page had a `PATH` hint still pointing at `v0.1.3` — the "install entry points at a
  downloadable version" requirement, caught by extracting and running every block. The
  suite now also fails if the page ever names two versions at once.

`linux-install-doc.test.sh`: **6 cases / 28 checks** (was 4 / 12), with `sha256sum`,
`sudo`, `apt-get` and `dpkg` shims so the Debian route runs on a machine that is not
Debian.

### The verification pass (all commands re-run at `RELEASE_COMMIT`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| 3 | `cargo test --workspace --locked` | **522 passed, 0 failed**, 46 test-result lines |
| 4 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check --remote --generated` | OK; 6 catalogue entries |
| 6 | `scripts/tests/test_versions.py` | 30 tests, OK |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / 43 tests / clean |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | 4 cases, 0 failures |
| 9 | `scripts/release/test-packaging.ps1` | 6 cases, 0 failures |
| 10 | `scripts/tests/package-linux.test.sh` | **10 cases, 54 checks**, 0 failures |
| 11 | `scripts/tests/linux-install-doc.test.sh` | **6 cases, 28 checks**, 0 failures |
| 12 | `docs/windows-validation/.../run-script-tests.ps1` | 18 cases, 151 checks, 0 failures (doubles only) |
| 13 | cross-target `cargo check` for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` | exit 0, plus Linux-target Clippy |
| 14 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** |
| 15 | delivered packages still match their manifests | every file |

### What round 9 could **not** verify

* **The desktop window, tray and menu entry on Linux.** `--selftest` exits before the
  GUI is built; a packaged app that starts headlessly is not a packaged app that draws.
* **A real Linux machine, and real motherboard sensors.** Everything here is a CI runner
  with simulated hardware.
* **Distributions other than Ubuntu 24.04.** No RPM, no Arch, no Flatpak, no Snap.
* **Fan control on Linux** — unchanged: reading only, by design.
* **A physical fan or pump on any platform**, and the desktop on Windows: unchanged.
* **Whether the AppImage runs without `APPIMAGE_EXTRACT_AND_RUN`** (i.e. with FUSE). The
  release run uses the extract-and-run path, which is what the install page documents.

### Publication

v0.1.4 = `febc02a`, tagged and published as a preview with **12 assets**: the Windows set
from v0.1.3 plus the Linux CLI archive, the Debian package and the AppImage. Every fact
above was re-checked *from the download* before the tag existed — both checksum lists
verify as a whole, `install.ps1` is byte-identical to the tagged blob, the published `.deb`
still declares `open-hardware-os 0.1.4 amd64` with `usr/bin/openhardwareos` (64-bit x86_64)
behind `usr/share/applications/OpenHardwareOS.desktop`, the AppImage is an executable
64-bit x86_64 ELF, and both metadata files name the tagged commit. The public Windows
install check ran against the tag
([run 34933031037](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34933031037)),
`main` was fast-forwarded to the release tip so the version-tree page names v0.1.4, and the
acceptance source package was rebuilt from the release commit.

### Deliberately **not** done in round 9

No hardware was written to; no autostart or global environment value was changed; nothing
was installed on the host; no licence was approved on the user's behalf (ADR 0002 and ADR
0004 remain **Proposed**); no new version was published without the artefacts having been
verified from the download first.

One process note worth recording: staging this release with `git add -A` swept the user's
own untracked `Prospector/` and `k10max-prospector/` working directories — keyboard
firmware work that has nothing to do with this repository's release. They were removed
from the index before the commit and left exactly as found. `git add -A` is not a safe
way to stage a release in a working tree that contains somebody else's work.

---

## 2026-09-15 — round 10: readings that can be checked, and a field that kept losing what was typed

**Revision: `5ec543a`** — the pass below ran at that commit, which is the commit the
tag points at. The commits carrying this entry follow it and change documentation only.

### 1. The Linux preview's last claim: "readings can be cross-checked"

Round 9 delivered the package; the remaining unmet criterion was that a reading can be
compared with something that did not come from this project. Until now the honest
answer was "read the source": every value came from sysfs, /proc or `sysinfo`, and
nothing checked it against what the operating system itself reports.

* **`ohm-cli status --json` and `ohm-cli doctor --json`** print one JSON object and
  nothing else — providers with status and reason, devices with their readings, the
  reason for each missing reading, and the capability notes. It is the *same*
  snapshot the desktop renders from, so the two cannot disagree about what the runtime
  reported. `crates/ohm-runtime/src/snapshot.rs` pins the paths a script reads, because
  renaming a field silently changes what is being compared.

* **`scripts/verify-linux-readings.sh`** compares each reading with the platform on the
  same host and prints one line per reading: `AGREE`, `DIFFER` (exit 1), `NO-SOURCE`
  (with the reason), `NOT-CHECKED` (no independent source exists). Memory against
  `/proc/meminfo`; fan tachometers and PWM against **the very hwmon file the reading
  came from**; temperatures against the range the platform reports; free space against
  `df` for the same mount point. A machine with no fan channels is a normal `NO-SOURCE`
  rather than a failure — the point is that it is stated — and `cpu.load` /
  `cpu.frequency` are declared uncheckable from one sample instead of being dressed up.

* **It runs on a real kernel**, in CI and in the release pipeline. Live result from this
  round's release run on `ubuntu-latest` (Ubuntu 24.04.5, kernel 6.17.0-1022-azure):
  `memory.total` equal to `/proc/meminfo` exactly, `memory.used` within 3.3 MB of
  `MemTotal − MemAvailable`, three mount points agreeing with `df` to 0, 0 and 16384
  bytes, the temperature reading inside the platform's own range, and `NO-SOURCE` for
  hwmon because that runner has no fan driver. The same check ran on the macOS and
  Windows runners in CI.

* **The install page carries a version a user can run themselves**: which system file or
  command corresponds to each reading, what a mismatch would mean, and the four
  verdicts. Its fixture suite extracts and runs *every* bash block on the page.

### 2. What the check found about itself

The documented-route fixture suite failed on Linux, correctly: its `ohm-cli` shim fed
**invented** readings, the documented check ran against the real /proc and /sys, found
them wrong and exited 1. A fixture for a check whose whole purpose is catching a reading
that does not match the machine has to tell the truth about the host it runs on, so it
now reads `/proc/meminfo`, the hwmon fan/PWM files and `df` for the root filesystem.

Doing that immediately exposed a real ordering bug in the check: with no
`/proc/meminfo` at all, "we report nothing and give no reason for it" was reported as a
**difference**. A missing platform source means there is nothing to disagree about; it
is only a defect to report nothing when the platform *had* an answer. Two smaller ones
went with it: readings can arrive as JSON floats (`82222657536.0`), which broke the
comparison's bash arithmetic until values were coerced, and the storage comparison used
the GNU-only `df --output=avail`, which made it untestable off Linux.

### 3. The duty field, for the second time

Two consecutive release builds failed on `expected 80, received 45` — the same assertion
that failed a release build in round 8, where the cause was real (the field followed
every reading, so a late one replaced what the user had typed). Round 8's guard was, and
is, in place: probed by pushing a snapshot, by pushing one 120 ms late, and by reading
the DOM, the field kept 80 in every case.

So the helper was changed to say what the field *actually contained* instead of
asserting on the write's arguments, and the next run said it in one line: after the
change event, the field held 45. The control keeps its draft in local state, so a render
that **unmounts and re-mounts** it — the device dropping out of one snapshot and coming
back in the next, which a discovery cycle, a disable or a blip can do — threw the edit
away and re-initialised the field from the reading.

The draft now lives on the screen and the field's content is *derived*
(`drafts[capability] ?? the reading`): there is no effect to lose it and no local state
to reset. The regression test drives exactly that sequence — the device absent from a
pushed snapshot, then present again — and it fails against the previous component with
CI's own message, which was checked before committing the fix (git stash the component,
run the test, see `expected '45' to be '80'`, restore).

Two lessons, both paid for: an assertion on a *symptom* (the write carried the wrong
value) hides the mechanism, while an assertion on the *state* names it; and this control
has now lost a user's input twice, both times in a way that looked like the write path.

### 4. Publication

v0.1.5 = `5ec543a`, tagged and published as a preview with 12 assets. Every fact was
re-checked from the download before the tag existed: both checksum lists verify as a
whole, `install.ps1` is byte-identical to the tagged blob, the published `.deb` declares
`open-hardware-os 0.1.5 amd64` with a 64-bit x86_64 ELF behind
`usr/share/applications/OpenHardwareOS.desktop`, and both metadata files name the tagged
commit. The public Windows install check passed against the tag
([run 34939368500](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34939368500)),
`main` was fast-forwarded so the version-tree page names v0.1.5, and the acceptance source
package was rebuilt from the release commit (
`dist/acceptance/OpenHardwareOS-5ec543a-windows-acceptance.zip`, 258 files).

### The verification pass (all commands re-run at `5ec543a`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| 3 | `cargo test --workspace --locked` | **523 passed, 0 failed**, 46 test-result lines |
| 4 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check --remote --generated` | OK; 7 catalogue entries |
| 6 | `scripts/tests/test_versions.py` | 30 tests, OK |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / **44 tests** / clean |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | 4 cases, 0 failures |
| 9 | `scripts/release/test-packaging.ps1` | 6 cases, 0 failures |
| 10 | `scripts/tests/package-linux.test.sh` | 10 cases, 54 checks, 0 failures |
| 11 | `scripts/tests/linux-install-doc.test.sh` | 6 cases, 32 checks, 0 failures |
| 12 | `scripts/tests/verify-linux-readings.test.sh` | **6 cases, 21 checks**, 0 failures |
| 13 | `docs/windows-validation/.../run-script-tests.ps1` | 18 cases, 151 checks, 0 failures (doubles only) |
| 14 | cross-target `cargo check` for `x86_64-pc-windows-msvc` and `x86_64-unknown-linux-gnu` | exit 0, plus Linux-target Clippy |
| 15 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** |
| 16 | delivered packages still match their manifests | every file |
| 17 | `scripts/verify-linux-readings.sh` against the kernel's own sources | on CI's Ubuntu, macOS and Windows runners: every comparable reading `AGREE` |

### What round 10 could **not** verify

* **A machine whose readings are interesting.** The CI runners have no fan driver, so the
  hwmon half of the cross-check reports `NO-SOURCE` there; the pathway is tested against
  synthetic trees that agree and disagree, but a real SuperIO board is still unverified.
* **The desktop window, tray and menu entry on Linux** — unchanged: the headless
  self-test returns before the GUI is built.
* **Real hardware of any kind**: no fan, no pump, no physical response.
* **Whether a user's own cross-check would find a *stale* reading**: the check compares
  values, not their age, and the engine's staleness window is still a policy question
  (carried over from round 8).

### Deliberately **not** done in round 10

No hardware was written to; no autostart or global environment value was changed; nothing
was installed on the host; no licence was approved on the user's behalf (ADR 0002 and ADR
0004 remain **Proposed**); `Prospector/` and `k10max-prospector/` were left exactly as
found, and nothing from them was staged.

---

## 2026-09-15 — round 11: the rules keep running with no window open, and one writer per channel

**Revision: `0c6e20c`** — the pass below ran at that commit, which is the commit the
tag points at. The commits carrying this entry follow it and change documentation only.

### 1. A service, and the problem underneath it

Closing the desktop window stopped the rules: the engine lived in the application
process. `ohm-cli service run` is the process that keeps them going — headless, until
`Ctrl-C` or `SIGTERM` — and `service status` reports from what it writes about itself
(pid, start, heartbeat age, cycles, rules, simulated/dry-run), exiting 1 when nothing
is running so a script can ask.

The service could not be written before the thing it makes dangerous was solved. The
desktop and the service run the same rules against the same channels, and two
processes alternating values onto one fan is worse than no automation at all:

* the owner claims the state file with `create_new`, which is atomic on every platform
  this build targets — two processes starting together, or two that both decide an
  abandoned file is reclaimable, cannot both win;
* liveness is a **heartbeat**, not a pid guess: the owner rewrites the file every
  second, so a file whose heartbeat stopped belongs to a process that died, including
  one killed with `SIGKILL` that never got to clean up. On Linux `/proc/<pid>` settles
  it exactly; elsewhere only the heartbeat decides, because a wrong guess in the
  "alive" direction blocks a restart and a wrong guess the other way lets two
  processes fight over a fan;
* an unparseable file counts as unowned. Refusing to start because a hand-edited file
  is broken would leave a machine that cannot run its rules until somebody deletes it;
* **the desktop stands down.** `AutomationEngine::start` checks the file, and when it
  names a different live process the engine does not start its loop: it logs, publishes
  an `engine_blocked` event, and records the reason in `stats().blocked_by` so a UI can
  show it instead of appearing to work while doing nothing.

### 2. Stopping is the part that matters

`SIGTERM` — what a service manager sends — and `Ctrl-C` stop the rule loop, shut the
runtime down (which hands control back and reports what that achieved: confirmed,
unconfirmed, refused, failed and simulated separately), and remove the state file.

The signal handler is installed **before** the state file is claimed. That ordering
came out of a failing test, not out of review: the test sent `SIGTERM` the moment the
state file appeared, and the process died by default termination with an empty log —
a service that looks alive but cannot yet hear a stop signal. Claiming channels before
being able to release them is the wrong order in production too, and now the code says
so where it happens.

The page states plainly why `kill -9` is the wrong way to stop it: nothing hands
control back, so the fans stay at the last value written, and the next start takes over
a state file that describes a dead process.

### 3. Two failures the CI found, both mine, both the same shape

Neither was in the service. Both were fixtures whose usefulness depended on the machine
they ran on:

* the unit fixtures named made-up pids (4242, 5001). Liveness is checked exactly on
  Linux, so a "fresh" state file looked abandoned there and two tests failed while
  macOS — where that check is deliberately skipped — was green. They now name this
  process, and a comment says why a made-up number is not good enough.
* `clippy -D warnings` refused two test helpers on Windows because every test using
  them needs `SIGTERM`, so they were dead code there. The gate now says that the
  *service* is not Unix-only, only those tests.

The local pass cannot see either: it runs on macOS, and the one cross-target gate that
exists checks compilation of a crate that has no platform-specific tests. Worth
recording as a limit of the current setup rather than pretending the pass covers it.

### 4. Publication

v0.1.6 = `0c6e20c`, tagged and published as a preview with 12 assets. Both checksum
lists verify as a whole from the download, `install.ps1` is byte-identical to the
tagged blob, both metadata files name the tagged commit, and the released CLI carries
the new subcommands. The public Windows install check passed against the tag
([run 34951399465](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34951399465)),
`main` was fast-forwarded so the version-tree page names v0.1.6, and the acceptance
source package was rebuilt from the release commit
(`dist/acceptance/OpenHardwareOS-0c6e20c-windows-acceptance.zip`, 262 files).

### The verification pass (all commands re-run at `0c6e20c`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| 3 | `cargo test --workspace --locked` | **538 passed, 0 failed**, 48 test-result lines |
| 4 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check --remote --generated` | OK; 8 catalogue entries |
| 6 | `scripts/tests/test_versions.py` | 30 tests, OK |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / 44 tests / clean |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | 4 cases, 0 failures |
| 9 | `scripts/release/test-packaging.ps1` | 6 cases, 0 failures |
| 10 | `scripts/tests/package-linux.test.sh` | 10 cases, 54 checks, 0 failures |
| 11 | `scripts/tests/linux-install-doc.test.sh` | 6 cases, 32 checks, 0 failures |
| 12 | `scripts/tests/verify-linux-readings.test.sh` | 6 cases, 21 checks, 0 failures |
| 13 | `docs/windows-validation/.../run-script-tests.ps1` | 18 cases, 151 checks, 0 failures (doubles only) |
| 14 | cross-target `cargo check` (Windows and Linux targets) | exit 0, plus Linux-target Clippy |
| 15 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** |
| 16 | delivered packages still match their manifests | every file |
| 17 | `scripts/verify-linux-readings.sh` against the kernel's own sources | every comparable reading `AGREE` |

The fifteen new tests this round: seven in `crates/ohm-runtime/src/service.rs`
(claiming, refusing, atomic takeover, a corrupt file, heartbeats, the window), four
process-level in `apps/cli/tests/service_lifecycle.rs` (a bounded run, a `SIGTERM`
stop, a second instance refused, a stale file taken over) and four in
`tests/tests/service_ownership.rs` (the engine stands down and says why, does not
block itself, is not blocked by an abandoned file, and reads the state file of the
runtime it belongs to).

### What round 11 could **not** verify

* **Real hardware**: every test uses simulated providers, and `--mock` forces dry-run.
  The service has never run on a machine with a fan.
* **Suspend and resume**: no test suspends a machine. The two mechanisms it depends on
  are each covered — a heartbeat that stops means "stopped", and readings older than
  the engine's window make rules fall back — but "suspended for eight hours and
  resumed" has not been executed.
* **Sensor loss at the service level**: covered by the engine's own tests, which the
  service drives; the restart-and-recover path through the service is not.
* **Windows service integration**: `service run` works in a console there; nothing
  installs or supervises it, and that path has not been run on a Windows machine.
* **Permissions**: reading needs none, and this build does not write `pwm<N>` at all,
  so there is no privilege story to verify yet.
* **Two processes racing in the same millisecond**: the atomic claim is reasoned about
  and unit-tested for the sequential cases; a true simultaneous start is not exercised.

### Deliberately **not** done in round 11

No hardware was written to; no autostart entry was created and no global environment
value was changed (the systemd unit on the page is a template a user installs, not
something this build does); nothing was installed on the host; no licence was approved
on the user's behalf (ADR 0002 and ADR 0004 remain **Proposed**); `Prospector/` and
`k10max-prospector/` were left exactly as found.

---

## 2026-09-15 — round 12: the service learns to say what it is doing, and the outages get tested

**Revision: `1c427e7`** — the pass below ran at that commit, which is the commit the tag
points at. The commits carrying this entry follow it and change documentation only.

### 1. "3 rules loaded" is not a status report

Round 11 gave the machine a background service. What it could not do was answer the
question the person running it actually has: *is this machine being cooled the way I
asked?* The state file said how many rules existed, which cannot distinguish cooling
along the curve from sitting on a fail-safe because a sensor went away.

The state file now carries every rule's status, its confirmed value and its own
sentence, and `service status` prints them:

```text
rules:
  chassis      fallback  at 70  fan.system.fakechip_fan1/fan.rpm is missing: falling back to 70 %
  gpu-cooling  held      at 64  no change worth writing (0.00 % within the 0.00 % deadband)
```

### 2. Two gaps the tests found on the way

* **The service wrote no log file.** The CLI initialised logging without paths, which is
  right for a command that prints a report and wrong for a process nobody watches:
  started by a service manager, its output goes to a journal nobody configured. It now
  logs into the config directory's `logs/`, and its banner names the state file and the
  log directory.
* **`OHM_HWMON_ROOT`**, a read-only override that points a whole process at another
  hwmon tree. This build writes no `pwm<N>`, so it changes what is read and never what
  is written — and it is what makes "a fan channel disappears while the process keeps
  running" something a test can do rather than something a person has to imagine.

### 3. The two outage paths the goal asks for, now covered at the service level

`apps/cli/tests/service_resilience.rs` drives the real binary against a prepared tree:

* **sensor loss and recovery.** The rule reads a tachometer from that tree and drives
  the simulated fan. Delete the tachometer: the state file reports `fallback`, the
  fail-safe 70 %, and names `fan.system.fakechip_fan1/fan.rpm` as the sensor that went
  away. Put it back: the rule reads again — the status leaves `fallback` and the message
  returns to that reading.
* **suspend and resume.** After `SIGSTOP` for longer than the heartbeat window,
  `service status` reports "not running" and names whose stale file it is: the state in
  which another process may take the channels over, which is what a sleeping machine
  looks like. After `SIGCONT` the same process is still the owner, with the heartbeat
  and the cycle counter moving again.

### 4. An hour spent on a rule that was working

The first version of the sensor test waited for a new *write* after the tachometer came
back, and timed out. The rule, it turned out, was evaluating the restored reading
perfectly well and had decided to **hold** the fail-safe 70 % rather than step to the
curve's 65 % — its hysteresis rule, holding until the reading passes the anchor, and
holding the *higher* (safer) value while it does. The lesson is in the outcome of §1:
with nothing reporting what a rule was doing, "no write for 20 seconds" and "the rule
never recovered" look identical. The page now states the behaviour, with the message
that says it, so the next person reads it instead of rediscovering it.

### 5. The same mistake, three times, and what is now written down

Every CI failure this round and last has one shape: **an assertion or a helper whose
meaning depends on the machine it is compiled or run on.** In order: unit fixtures that
named made-up pids (liveness is exact on Linux, so the tests passed on macOS for the
wrong reason); test helpers used only by a `#[cfg(unix)]` test, which are dead code on
Windows under `clippy -D warnings`; twice more of the same, in the new file; and a test
asserting a *clean* shutdown on a platform where it can only kill the process, because
Windows offers no way to deliver a console CTRL+C to another process.

Two things came out of the third occurrence. The rule is written in the file where it
keeps happening — *a helper used only by a `cfg`-gated test carries the same `cfg`* —
and the limits of the local pass are recorded rather than assumed: the pass runs on
macOS, where these helpers are used, and the one cross-target gate checks a crate with
no platform-specific tests, so no local run can see this class at all. It is worth
saying that the CI is doing its job here: each failure was a real difference in
behaviour between the platforms, not noise.

### 6. Publication

v0.1.7 = `1c427e7`, tagged and published as a preview with 12 assets. Both checksum
lists verify as a whole from the download, `install.ps1` is byte-identical to the tagged
blob, and both metadata files name the tagged commit. The public Windows install check
passed against the tag
([run 34956723135](https://github.com/SvenKunkka/OpenHardwareOS/actions/runs/34956723135)),
`main` was fast-forwarded so the version-tree page names v0.1.7, and the acceptance
source package was rebuilt from the release commit
(`dist/acceptance/OpenHardwareOS-1c427e7-windows-acceptance.zip`, 263 files).

### The verification pass (all commands re-run at `1c427e7`, nothing carried over)

| # | Command | Result |
|---|---|---|
| 1 | `rustup run 1.98.1 cargo fmt --all -- --check` | exit 0 |
| 2 | `cargo clippy --workspace --all-targets --locked -- -D warnings` | exit 0 |
| 3 | `cargo test --workspace --locked` | **541 passed, 0 failed**, 49 test-result lines |
| 4 | `cargo deny check` | advisories ok, bans ok, licenses ok, sources ok |
| 5 | `scripts/versions.py check --remote --generated` | OK; 9 catalogue entries |
| 6 | `scripts/tests/test_versions.py` | 30 tests, OK |
| 7 | `npm run typecheck` / `npm test` / `npm run build` | clean / 44 tests / clean |
| 8 | `scripts/tests/make-acceptance-package.test.sh` | 4 cases, 0 failures |
| 9 | `scripts/release/test-packaging.ps1` | 6 cases, 0 failures |
| 10 | `scripts/tests/package-linux.test.sh` | 10 cases, 54 checks, 0 failures |
| 11 | `scripts/tests/linux-install-doc.test.sh` | 6 cases, 32 checks, 0 failures |
| 12 | `scripts/tests/verify-linux-readings.test.sh` | 6 cases, 21 checks, 0 failures |
| 13 | `docs/windows-validation/.../run-script-tests.ps1` | 18 cases, 151 checks, 0 failures (doubles only) |
| 14 | cross-target `cargo check` (Windows and Linux targets) | exit 0, plus Linux-target Clippy |
| 15 | `scripts/verify-ipc-roundtrip.sh` | **IPC ROUND TRIP VERIFIED** |
| 16 | delivered packages still match their manifests | every file |
| 17 | `scripts/verify-linux-readings.sh` against the kernel's own sources | every comparable reading `AGREE` |

Three tests were added this round (`apps/cli/tests/service_resilience.rs`): sensor loss
with recovery, suspend with resume, and the `OHM_HWMON_ROOT` override those two depend
on. The runtime's service module also gained the outcome reporting, covered by the
existing heartbeat unit test.

### What round 12 could **not** verify

* **Real hardware**, still: the "sensor" is a file in a prepared tree and the fan is the
  simulated one. Nothing in this project has touched a real fan.
* **A real suspend**: `SIGSTOP` is the process-level equivalent, not the kernel path
  (devices re-initialising, drivers reloading, the clock jumping).
* **A real driver's channel disappearing and returning** — the same file trick stands in
  for it.
* **Windows service supervision**: `service run` works in a console there; nothing
  installs or supervises it.
* **Permissions**: still nothing to verify, because this build writes no `pwm<N>`.

### Deliberately **not** done in round 12

No hardware was written to; no autostart entry was created and no global environment
value was changed; nothing was installed on the host; no licence was approved on the
user's behalf (ADR 0002 and ADR 0004 remain **Proposed**); `Prospector/` and
`k10max-prospector/` were left exactly as found.
