# Contributing to OpenHardwareOS

Thanks for helping. This is a hardware-control project: a mistake here does not
produce a stack trace, it spins a fan to 100 % or stops a pump. Read the adapter
section before you write a write path, and the licence section before you add a
dependency.

## Toolchain

| Requirement | Value | Source |
|---|---|---|
| Rust | **1.95** minimum | `rust-version = "1.95"` in the root `Cargo.toml` (`[workspace.package]`) |
| Edition | 2024 | `edition = "2024"` |
| Node.js | `^20.19.0 \|\| >=22.12.0`, with npm | Vite 7's `engines` field, as installed by `apps/desktop/package.json` |
| Windows SDK / MSVC | for the Windows build | the `wmi` and `nvml-wrapper` dependencies |

The MSRV is set by the dependency floor (`sysinfo` 0.39 requires 1.95, not the
1.93 that an earlier revision of this file claimed). Do not raise it casually — it
is a promise to contributors, not an implementation detail. A
`rust-toolchain.toml` is deliberately not committed;
`.github/workflows/ci.yml` installs a pinned toolchain instead.

**`clippy` must come from the same release as `rustc`.** A mismatched pair does
not lint, and it does not say so clearly: `clippy-driver 0.1.92` next to `rustc
1.98.0` makes cargo apply the `rust-version` gate and abort with `rustc 1.92.0 is
not supported by the following packages: … requires rustc 1.95` before it reads a
line of code. Install a matched pair and call it explicitly, e.g.

```bash
rustup toolchain install 1.98.0 --profile minimal --component clippy,rustfmt
rustup run 1.98.0 cargo clippy --workspace --all-targets -- -D warnings
```

Do not paper over it with `--ignore-rust-version`: that disables the very
assertion that is failing, so the result proves nothing about the MSRV.

## Build and test

```bash
# 1. The whole workspace: unit + cross-crate integration tests. 444 of them at
#    the round-3 revision — docs/verification-log.md records the current figure.
cargo test --workspace

# 2. Lints. Must be warning-free.
cargo clippy --workspace --all-targets

# 3. The desktop frontend: typecheck, then a production bundle.
cd apps/desktop && npm install && npm run build && cd ../..

# 4. The full cooling loop on simulated hardware. No real fan is touched.
cargo run -p ohm-cli -- demo

# 5. Headless desktop startup check — the installation self-test.
cargo run -p ohm-desktop -- --selftest
```

What each command actually proves:

| Command | Proves | Failure means |
|---|---|---|
| `cargo test --workspace` | Unit tests next to the code, plus the acceptance, edge-case, rule-lifecycle and protocol-flow suites in `tests/`, which drive a *real* runtime with real adapters against a deterministic simulated machine | A behaviour or an invariant changed |
| `cargo clippy --workspace --all-targets` | The `[workspace.lints]` set is satisfied on the code and on the tests (`clippy::all` at `warn`, `clippy::todo` at `warn`) | Usually a real bug: `clippy::all` catches comparison mistakes, redundant clones and lossy casts |
| `npm run build` | `tsc --noEmit` — the frontend typechecks against `apps/desktop/src/types.ts`, which mirrors the Rust wire types — then Vite produces `apps/desktop/dist` | A Rust-side type change was not reflected in the TypeScript types |
| `cargo run -p ohm-cli -- demo` | The closed loop end to end: GPU temperature → rule → fan duty → a cooler GPU, through the real engine, safety policy and audit log. Exits non-zero if the rule never wrote | The loop is broken (this is the strongest single check in the repository) |
| `cargo run -p ohm-desktop -- --selftest` | The desktop binary starts the real runtime headlessly, installs the example rule, runs the automation engine for ~1.2 s, and prints adapters, devices and rule outcomes | The app cannot start or the engine cannot tick |

Useful flags: `--mock` registers the simulated providers even when Settings has
them off; `--dry-run` blocks every hardware write whatever the saved settings say.
For CLI work, `OHM_CONFIG_DIR=/tmp/ohm-dev cargo run -p ohm-cli -- …` keeps your
real `%APPDATA%\OpenHardwareOS` untouched — `ConfigPaths::discover` honours that
variable, and every command is deterministic.

Run the whole verification before you open a PR:

```bash
cargo test --workspace && \
cargo clippy --workspace --all-targets && \
(cd apps/desktop && npm run build) && \
cargo run -p ohm-cli -- demo && \
cargo run -p ohm-desktop -- --selftest && \
python scripts/versions.py check --generated && \
python scripts/check-artefact-names.py && \
python -m unittest discover -s scripts/tests -p 'test_*.py'
```

The last three need no toolchain and catch a class of defect no compiler sees: a built
artefact renamed in one place and not the others, or an install entry pointing at a
version nobody can download. Both have already happened here once — the details are in
`docs/verification-log.md` (rounds 15 and 16) and `docs/version-management.md`.

CI (`.github/workflows/ci.yml`) runs the same checks in six jobs: `lint`
(`cargo fmt --all -- --check`, then `cargo clippy --workspace --all-targets -- -D
warnings`), `rust` (`cargo build --workspace --all-targets`, `cargo test
--workspace`, the headless `ohm-cli demo` and `ohm-desktop --selftest --mock`
normally and in `--dry-run`, on Linux, macOS and Windows), `frontend` (`npm ci`,
typecheck, behaviour tests, build), `windows-bundle` (the NSIS installer, uploaded
for review — never published) and `hygiene` (a crate opting out of `unsafe_code =
"deny"` or any ADL/ADLX reference in code fails the build, and `cargo deny check`
runs for licences, advisories and sources). `Version catalogue and lifecycle`
(`scripts/versions.py check --generated`, the `scripts/tests` unit tests and
`scripts/check-artefact-names.py`) is the sixth.

There is **no `continue-on-error` anywhere** in that workflow: an earlier revision
made the clippy and `cargo deny` steps advisory, and that is exactly how a lint
regression or a licence violation would have reached `main` unnoticed. Note also
that a workflow definition is not a run: nothing here has been executed by GitHub
Actions yet, so every `windows-latest` job is *Prepared*, not verified.

## Repository layout

```text
Cargo.toml              workspace: members, shared dependencies, shared lints, MSRV
crates/
  ohm-core              ids, errors, config paths, logging, time — no hardware
  ohm-device-model      Device, Capability, Value, Reading, DeviceState, units
  ohm-adapter-api       the HardwareAdapter trait: the only way hardware enters
  ohm-runtime           discovery, device table, state store, event bus, safety, audit
  ohm-automation        rules, curves, evaluator, engine, YAML rule store
  ohm-protocol          Open Device Protocol: framing, messages, descriptors, mock device
  adapters              facade: which providers run, and in what order
adapters/
  mock                  deterministic simulated PC (CPU/GPU/SSD/fans/pump + fault injection)
  system                OS sensors via sysinfo, plus Windows storage counters
  nvidia                NVML telemetry and (elevated) fan control
  libre-hardware-monitor  LHM web server: motherboard sensors and chassis fan control
  open-protocol         ODP devices (the simulated OpenFan today)
apps/
  cli                   headless runtime: doctor / status / watch / demo / rules / audit / protocol
  desktop               Tauri v2 backend (src-tauri) + React/TypeScript UI (src)
tests/                  cross-crate acceptance, edge case, rule lifecycle, protocol tests
examples/               ready-to-use rules in examples/rules/, checked by apps/cli/tests/yaml_examples.rs
docs/                   architecture, device model, automation, protocol, roadmap, research, ADRs
```

The dependency rule is one-way and worth stating explicitly:

```text
ohm-core → ohm-device-model → ohm-adapter-api → adapters/*
                                      ↓
                                 ohm-runtime → ohm-automation → apps/*
```

`ohm-device-model` does not know `ohm-runtime` exists. An adapter does not know
`ohm-automation` exists. If you find yourself wanting to break that, the code
probably belongs one layer up.

## Coding standards

These are the standards the code actually follows — each one has a lint, a
convention enforced by review, or a test.

**No `unsafe`.** The workspace denies it:

```toml
[workspace.lints.rust]
unsafe_code = "deny"
missing_debug_implementations = "warn"

[workspace.lints.clippy]
all = { level = "warn", priority = -1 }
todo = "warn"
```

Every member crate opts in with `[lints] workspace = true`, so this applies to
your new crate too — add that section. Note the deny covers *our* code only;
`serde_yaml_ng` pulls in `unsafe-libyaml`, which is a dependency, not ours. If you
genuinely need `unsafe`, that is a design discussion, not a `#[allow]`.

**Clippy is at `warn` for `clippy::all`, and the workspace is expected to be
warning-free.** `clippy::todo` is a warning, so `todo!()` will not pass review
silently. There is no crate-level `#![warn(...)]`/`#![deny(...)]` block anywhere:
the lint configuration lives in one place, the root `Cargo.toml`.

**Doc comments on public items.** Every public type, field, function and enum
variant in this repository has a `///` comment that says what it is for, not what
it does. Long-form module docs (`//!`) explain the *why* and usually carry a
diagram or a table — see `crates/ohm-runtime/src/safety.rs`,
`crates/ohm-protocol/src/lib.rs`. `missing_docs` is **not** lint-enforced today;
it is enforced by review, so do not be the one who breaks the pattern.

**Tests live next to the code**, in a `#[cfg(test)] mod tests` at the bottom of
the same file, not in a separate directory. Cross-crate behaviour goes in
`tests/tests/*.rs` on top of the shared harness in `tests/src/lib.rs` — read
`tests/tests/rule_lifecycle.rs` before writing an integration test; `Session`
already gives you a runtime, an engine, a simulated machine and a `step()`
helper.

**Report `UnavailableReason`, never a silent zero.** A missing reading is
`Reading::unavailable(capability, reason, detail)`, and the reason is from a
closed vocabulary (`crates/ohm-device-model/src/state.rs`): `unsupported`,
`not_present`, `permission_denied`, `hardware_limitation`, `vendor_limitation`,
`driver_missing`, `timeout`, `disabled`, `read_error`, `unknown`. The UI turns
those into grey badges (a limitation) or red ones (`is_failure()` is true only for
`read_error`, `timeout`, `unknown`). The CLI renders the same thing:
`ohm-cli` prints `[unsupported]`, never `0` — `apps/cli/src/main.rs` —
`tests::missing_readings_render_as_reasons_not_zero`. **Zero is a value.** If you
cannot read a sensor, say why.

**Adapters must never panic.** The trait documents it: *"Implementors must never
panic on missing hardware: report `UnavailableReason` instead."* No `unwrap()`,
no `expect()`, no indexing on adapter input; a driver that vanishes mid-poll is
normal, not exceptional. The only `expect()` calls in the whole `adapters/` tree
are in `adapters/libre-hardware-monitor/src/fake_server.rs`, which is
`#[cfg(test)]`-gated test support — production adapter code contains none.

**Never hold a lock across `.await`.** The runtime uses `parking_lot` for
synchronous state and `tokio::sync::Mutex` where a guard must be held across an
await. `Runtime::write_with_origin` shows the pattern — resolve under the read
lock, drop it, then act:

```rust
// 1. Resolve without holding the lock across the await.
let (device, capability, enabled) = {
    let table = inner.table.read();
    // ...
};
```

The same applies to the write gate:

```rust
let outcome = {
    let _gate = inner.write_gate.lock().await;
    adapter.write(&device, &capability, &value).await
};
```

A `parking_lot` guard held across an await is not `Send`, so this is usually a
compile error — but it is worth knowing *why*, because the workaround is never
"make it a `std::sync::Mutex`".

**English comments, English identifiers, no emoji.** Comments explain the
decision, not the syntax: *"A stopped pump is a dead CPU"*,
*"Rising is immediate, falling only after a real drop"*. If you write a comment
that restates the next line, delete it.

**Errors are typed, not strings.** `OhmError` carries a machine-readable `code()`
that reaches the UI and the audit log; `thiserror` derives the `Display` text.
Return `Err(OhmError::…)`, do not `anyhow!` inside a library crate. (`anyhow` is
for the CLI's `main`, where a human reads the message.)

## Adapter contribution guide

An adapter is the only way hardware enters OpenHardwareOS. Put a new one in
`adapters/<name>/` (matching the directory convention: `system`, `nvidia`,
`libre-hardware-monitor`, `open-protocol`, `mock`), name the package
`ohm-adapter-<name>`, and depend on `ohm-adapter-api` — never on `ohm-runtime` or
`ohm-automation`.

### The trait

`crates/ohm-adapter-api/src/lib.rs` — `HardwareAdapter`:

| Method | Called | Must do |
|---|---|---|
| `info(&self) -> AdapterInfo` | once per UI refresh, and at registration | return a stable `id` (= the device id namespace), a display name, a description, `requires_admin` and `capabilities` |
| `probe(&self) -> AdapterStatus` | before `discover`, and periodically | answer "can this work on this machine *right now*" with `available` / `degraded` / `unavailable` / `error` plus an `UnavailableReason` and a human-readable `detail` that the UI shows verbatim |
| `discover(&self) -> Result<Vec<Device>>` | every discovery cycle | enumerate the devices that exist **now**; this is also the hotplug detection point, so a device that disappeared simply is not returned |
| `read_state(&self, &Device) -> Result<DeviceState>` | per device per poll | return one `Reading` per capability the device declares |
| `read_all(&self, &[Device]) -> BatchStateResult` | per adapter per poll | the default polls serially; **override it** when the backend has a batch API, so a poll cycle stays cheap |
| `write(&self, device, capability, value) -> Result<WriteOutcome>` | on every manual, automated, emergency or shutdown write | apply the value or refuse it, explicitly |
| `shutdown(&self) -> Result<()>` | on runtime shutdown | hand control back to the firmware |
| `as_any(&self) -> &dyn Any` | UI only | `self` — this is how the mock hardware panel reaches its own controls |

### The rules an adapter must follow

1. **Report `UnavailableReason` instead of failing.** A sensor you cannot read is
   a `Reading::unavailable(...)`, not an error and not a zero. Reserve `Err(..)`
   for the whole device being unreadable, and even then prefer
   `AdapterStatus::degraded`/`unavailable` with a reason.
2. **Reject out-of-range writes rather than clamping silently.** The runtime
   already clamps to the declared range and records `clamped: true` in the audit
   report; an adapter that quietly clamps something else hides a real
   disagreement. Validate, and return an error or `WriteOutcome::rejected(detail)`
   with an explanation.
3. **Declare the range honestly.** `Capability { min, max, step, values }` is what
   the runtime clamps against, what the safety floor is compared to, and what the
   UI shows. A wrong `max` is a real hazard.
4. **Set the identity metadata you have.** `Device::with_vendor`, `with_model`,
   `with_tag`, and `with_metadata(key, value)` for backend-specific ids — the LHM
   adapter stores `lhm_id` and `lhm_channel`, the mock adapter tags everything
   `simulated`. This is what lets a user, and a bug report, tell two identical
   fans apart. Device ids must be stable across restarts: a device that changes
   id loses its history and breaks the rules that point at it.
5. **Set `requires_admin` and `write_requires_admin` honestly.** They drive the
   Settings table and the UI's advice. `AdapterCapabilities::read_only()` and
   `::cooling_control()` are the two canonical shapes.
6. **Override `shutdown` when you own an actuator.** Releasing a channel is not
   optional: `Runtime::shutdown` calls `adapter.shutdown()` after
   `relinquish_on_exit`, and the LHM adapter uses it to push every control channel
   back to the firmware's own curve.
7. **Never block the runtime.** `read_all` is awaited inside the poll loop; a
   synchronous HTTP call with no timeout stalls every other provider. Set
   timeouts and let them become `UnavailableReason::Timeout`.

### Registering it

1. Add the crate to `members` in the root `Cargo.toml` and to
   `[workspace.dependencies]`.
2. Add the dependency to `crates/adapters/Cargo.toml` and re-export it from
   `crates/adapters/src/lib.rs` under `pub mod prelude`.
3. Wire it into `AdapterOptions`: add a `bool` field, a default, an entry in
   `enabled_ids()` (its position **is** its priority — see the module docs on why
   NVML steps aside when LibreHardwareMonitor is present), a branch in
   `build_adapters`, and an entry in `adapter_catalogue()`.
4. Read its settings in `AdapterOptions::from_settings` so
   `settings.adapter_enabled("<id>")` and `set_adapter_enabled` work; the Settings
   screen is generated from the catalogue, so a new provider appears with no UI
   change.
5. Extend the tests in that file (`default_options_are_production_safe`,
   `simulated_profiles`, `building_adapters_follows_the_options`,
   `catalogue_lists_every_provider`) — they will fail loudly if you forget a step.

### Tests an adapter must ship

Four categories, at minimum. `adapters/libre-hardware-monitor` is the reference
for a **write** path: `fake_server.rs` is an in-process HTTP server that speaks
LHM's `/data.json` contract, so the tests exercise real HTTP without needing
LibreHardwareMonitor installed. Its test module
(`adapters/libre-hardware-monitor/src/lib.rs`, `mod tests`) is the template:

| Category | LHM test | What it pins down |
|---|---|---|
| Success path | `probe_and_discovery_work_against_a_live_server`, `readings_arrive_for_every_device`, `writes_reach_the_control_channel` | probe returns `available`; every declared capability has a reading; a write actually arrives |
| Unavailable path | `an_unreachable_server_is_explained_not_hidden` | no server → `unavailable` with a reason and a detail, never a panic and never a zero |
| Refusal path | `a_refused_write_is_reported_as_rejected` | the backend says no → `WriteOutcome::rejected` with the detail, not a fake success |
| Invalid value | `writes_are_validated_before_touching_the_server` | an out-of-range or malformed value is refused **before** the backend is contacted |

Plus `shutdown_releases_every_control_channel` (release is a feature) and
`adapter_metadata_is_complete` (ids, names, `requires_admin`, non-empty
descriptions).

`adapters/system` is the reference for a **read-only, cross-platform** adapter:
12 tests that never touch a fan, `write()` returning
`Err(OhmError::CapabilityNotWritable { .. })` for everything, and an explicit
table in its module docs of what each OS can and cannot answer. If your adapter is
telemetry-only, copy its shape and its honesty about the last row of that table.

Use the `ohm-adapter-mock` fault injection (`MockFaults`,
`MockConfig::deterministic()`, manual clock) for adapter-independent behaviour, and
add a `MockFaults` variant if you need a new failure mode to test the runtime.

## Licensing rules

**OpenHardwareOS is Apache-2.0** (root `Cargo.toml`,
`apps/desktop/package.json`, and the `LICENSE` file). Contributions are accepted
under Apache-2.0 §5, which is why the project chose it — an explicit patent grant
and explicit inbound contributor terms. See
[`docs/decisions/0002-license.md`](docs/decisions/0002-license.md).

### Dependencies: what is welcome

| Licence | Verdict | Obligation |
|---|---|---|
| MIT, Apache-2.0, BSD-2/3-Clause, ISC, Zlib | **fine** | attribution in the notices file |
| Apache-2.0 OR MIT (dual) | **fine** — prefer the Apache-2.0 arm | none beyond attribution |
| **MPL-2.0** | **fine, with care** | file-level copyleft: consuming an *unmodified* package is fine; if you patch a `.cs`/source file, that file stays MPL-2.0 |
| LGPL-2.1 / LGPL-3.0 | **ask first** | relink obligations; no LGPL crate is a dependency today |
| GPL-2.0 / GPL-3.0 / AGPL-3.0 | **no** | incompatible with shipping inside an Apache-2.0 product. This is why **OpenRGB (GPL-2.0)** and **G-Helper (GPL-3.0)** are reference reading only: their documentation is citable, their code must not be copied or linked. |
| Proprietary SDKs with redistribution restrictions | **no** | see below |

### Two hard blockers you must not work around

**AMD ADL / ADLX.** Their EULAs explicitly forbid subjecting SDK source to a
licence that requires source disclosure or the right to modify — ADL §5(e),
ADLX §4(f) — and Apache-2.0 is exactly such a licence. They also permit object
code only (ADL §3(d), ADLX §2(c)), forbid publishing the SDK for others to copy
(ADL §5(f)), and mandate a restrictive end-user agreement (ADL §4(a), ADLX §3,
including naming AMD a third-party beneficiary). **No ADL/ADLX header, sample,
binding, wrapper, or hand-written or generated `-sys` crate transcribed from those
headers may be added to this workspace**, in source or object form. AMD GPU
telemetry and fan control come exclusively from the MPL-2.0 LibreHardwareMonitor
host. Details and verbatim citations: `docs/decisions/0004-vendor-sdks-and-amd.md`
and [`docs/research.md`](research.md) §4.3.

**NVIDIA binaries must not be redistributed.** `nvml-wrapper` loads the bare name
`nvml.dll` from the installed driver with `libloading`, so nothing NVIDIA is
shipped. The NVIDIA Driver Licence Agreement §2.7 (*"you may not sell, rent,
sublicense, distribute or transfer the SOFTWARE"*) and §2.9 (*"You may not use
the SOFTWARE in any manner that would cause it to become subject to an open source
software license"*) forbid bundling it; NVML is not on the CUDA EULA's
Attachment A redistributable list, and CUDA EULA §1.2 forbids copying any portion
of the SDK. **Never commit, vendor, bundle or `include_bytes!` a vendor driver
DLL** — load the driver-installed copy at runtime, as the existing adapter does.
`docs/research.md` §3.4.

`WinRing0` is also out, for a different reason: Microsoft classifies it as a
vulnerable driver (`CVE-2020-14979`, `VulnerableDriver:WinNT/Winring0`) and it is
on the recommended blocklist that is on by default on Windows 11. PawnIO is the
current, supported path, and its installer is GPL-2.0-or-later, so the project
detects and tells the user where to get it rather than bundling it
(`docs/research.md` §1.5).

### Updating the notices

When you add a dependency, update the **"Third-party components and licences"**
table in [`README.md`](README.md) — it lists every dependency, its licence and
how it is used, including the row that says NVIDIA NVML is *not* redistributed.
Add a row for anything new; if you remove a dependency, delete its row. If your
change means the project now ships something it did not before, say so explicitly
in the PR description and add the licence text to the notices.

`cargo deny check` runs in CI as a **required** step in the `hygiene` job, backed
by the committed `deny.toml`; it passes today (`advisories ok, bans ok, licenses
ok, sources ok`). It gates a merge. Two things it still does **not** do, both
recommended by `docs/research.md` (decision 13): it does not generate a
third-party notices file — the `README.md` table is maintained by hand — and the
licence decision for the project itself (ADR 0002) and for vendor SDKs (ADR 0004)
is still **Proposed**, not accepted. Adding notices generation is a welcome
contribution; do not treat it as permission to add a dependency whose licence the
ADRs have not settled.

## Pull requests

* **Keep the workspace green.** `cargo test --workspace`,
  `cargo clippy --workspace --all-targets` and (for frontend changes)
  `npm run build` all clean. If a test cannot pass on your machine, say which and
  why in the PR rather than deleting it.
* **Add tests.** New behaviour needs a test next to the code; a bug fix needs a
  test that fails before the fix. Safety-relevant changes (anything on the write
  path, the safety policy, the evaluator, the runtime's fail-safe) need a test in
  `tests/tests/` as well, because those are cross-crate invariants.
* **Document behaviour changes.** If you change a default, a message, a YAML
  field or a public function, the matching document changes in the same PR:
  `docs/automation.md` for rules and curves, `docs/device-model.md` for the model,
  `docs/protocol.md` for ODP, `docs/architecture.md` for the runtime. A decision
  that is hard to reverse deserves an ADR in `docs/decisions/` (`NNNN-title.md`,
  status / date / context / decision / consequences — follow the existing four).
* **One logical change per PR.** A refactor and a behaviour change go in separate
  PRs, because a reviewer cannot approve one while questioning the other. If you
  need a preparatory refactor, land it first.
* **No drive-by reformatting.** Do not reformat files you are not otherwise
  changing, do not reorder imports across a file, and do not fix unrelated
  warnings in the same PR. It makes the diff unreadable and hides the real change.
* **Say what you verified.** The PR description should name the commands you ran
  and, for hardware-touching changes, whether you ran them against simulated
  hardware only or against a real machine — and which one.
* **New adapters get a note about the machine you tested on.** "Works on my
  motherboard" is not testable; a PCI/SMBus id, the OS build and the reading you
  got are.
* **Be precise about what is not done.** This project documents its gaps on
  purpose (`docs/roadmap.md`, `docs/automation.md` → "Two honest gaps"). A PR that
  adds a field it does not implement should say so in the docs, not leave the next
  reader to find out by grepping.
