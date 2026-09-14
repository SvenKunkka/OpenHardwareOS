# Verification log

An append-only record of what was actually executed, on which machine, and what
came back. It exists so a reviewer does not have to trust a summary: every entry
names the command, the environment, and the result — including the checks that
could **not** be run here.

Machine for every entry unless stated otherwise: macOS 26.6 arm64 (Apple M5),
`rustc 1.98.0`, `cargo 1.98.0`, `cargo-clippy 0.1.92`, Node 26.8.1. Development
happens on macOS; **Windows has never been exercised** — see
`docs/windows-validation/`.

---

## 2026-09-14 — round: gates, conflict enforcement, dead-switch fixes

### Baseline (taken before any change in this round)

| Command | Result |
|---|---|
| `cargo test --workspace` | **372 passed, 0 failed** |
| `git rev-parse --is-inside-work-tree` | not a repository (initialised in this round) |

`cargo clippy --version` reports **0.1.92** while `rustc --version` reports
**1.98.0**. The Homebrew toolchain ships them out of step, so `cargo clippy
--workspace` aborts before reading project code with:

```
error: rustc 1.92.0 is not supported by the following packages:
  ohm-adapter-api@0.1.0 requires rustc 1.95
```

This is an environment defect, not a project defect. It is why CI installs
`dtolnay/rust-toolchain@stable` as a matching pair and why the CI lint job is
**required** rather than `continue-on-error`: a check that cannot fail is not a
check.

### Changes verified in this round

| # | Change | Verification |
|---|---|---|
| 1 | `minimize_to_tray` wired to the real minimise behaviour, `close_to_tray` kept distinct | `cargo test -p ohm-desktop` → 15 passed, incl. `window_policy_follows_the_settings` |
| 2 | `adapter_settings.mock.enable_gpu_fan_control` was a no-op branch; now removes the GPU fan *control* channel while keeping monitoring | `cargo test -p ohm-adapter-mock -p ohm-adapters` → 31 + 8 passed; end to end in `tests/tests/edge_cases.rs::disabling_gpu_fan_control_keeps_monitoring_but_removes_control` (persists → reloads → device still monitored → no writable channel → index has no target → write refused with `capability_read_only`) |
| 3 | Numeric condition gate `Rule.when`, backwards compatible | `cargo test -p ohm-automation` → 79 passed, incl. 9 new gate tests (false ⇒ stand-down at the fail-safe duty; custom `otherwise`; reopen resumes the curve; exact boundary behaviour for all six operators; NaN/∞ never satisfy; missing condition source follows the sensor policy; the condition source gets the same grace period; unconditional rules unaffected; non-finite threshold rejected) |
| 4 | One output, one writer | 5 new engine tests: refusal on save, refusal on enable, disabled rules own nothing, file-level conflicts resolved deterministically with a `RuleConflict` diagnostic, and a conflict reaching the active set is skipped (`RuleStatus::Error`, `writes == 0`, the output carries exactly the owner's value) |
| 5 | `AdapterCapabilities::poll_interval_ms` honoured (was declared and unread) | `cargo test -p ohm-runtime` → 62 passed, incl. `an_adapter_hint_slows_its_own_polling_without_affecting_others` and `an_adapter_without_a_hint_is_polled_every_cycle` |
| 6 | CPU package power states its source instead of vanishing | `cargo run -p ohm-cli -- doctor` prints a **Capability notes** section: with no provider, "CPU package power: not available … comes from LibreHardwareMonitor (Provider: lhm) … reported as missing rather than as 0 W"; with the simulated provider, it names the device and adapter supplying it |
| 7 | CI no longer masks failures; `deny.toml` added; Windows bundle job added | Not runnable here (GitHub Actions); reviewed by hand. The lint job is required, and the Windows job builds the NSIS installer and uploads it as a review artifact without publishing anything |

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
  checked with the local clippy (0 warnings); `ohm-adapter-system` and
  `ohm-desktop` depend on `sysinfo 0.39`, which needs rustc ≥ 1.93 and is
  therefore invisible to clippy 0.1.92.
* **The NSIS installer**: the configuration, icons and frontend build are in
  place, but no bundle has been produced on this machine (it is a Windows target).
* **Frontend behaviour tests** for the new condition editor and conflict
  messaging: added in this round by the frontend workstream; see the frontend
  section of the final report for the command and result.

### Deliberately **not** done

No real fan was written to, no drive was stopped, and no hardware setting was
changed on the host machine. `--dry-run` was used for every non-simulated pass,
and the simulated provider for every closed-loop demonstration.
