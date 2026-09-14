# 0001 — UI and runtime stack

## Status

Accepted

## Date

2026-09-11

## Context

OpenHardwareOS reads PC hardware sensors and automates cooling (temperature → fan
speed). It is Windows-first, Apache-2.0, and meant to be contributed to by
strangers. Three constraints drive the stack choice:

1. **Hardware access must be one auditable door.** Motherboard fan I/O needs a
   kernel driver, and a wrong register write can spin a fan to 100 % or stop it,
   so the hardware layer must be small, testable without hardware, and impossible
   to bypass.
2. **Licence hygiene is a hard requirement.** AMD's ADL/ADLX SDKs cannot appear in
   an Apache-2.0 repository (`0004`), and the Windows App SDK redistributable is
   not permissive either: the shipped `Microsoft.WindowsAppSDK` NuGet package sets
   `requireLicenseAcceptance=true` with a file licence requiring you to "add
   significant primary functionality", to pass protective terms to end users, to
   **indemnify and defend Microsoft**, and forbidding distribution under a licence
   that requires source disclosure ([research.md §6.1](../research.md)).
3. **The community we want is the one that can read the code.** Rust + React +
   YAML needs a Rust toolchain and Node; C#/XAML/WinUI needs the Windows SDK and
   Visual Studio workloads, plus reading the Windows App SDK EULA to redistribute.

The .NET option was attractive on one axis only: a C# app can host
`LibreHardwareMonitorLib` **in-process**, with no IPC and no second process to
supervise. The decision below does not pretend otherwise — it says that advantage
is reachable without making .NET the foundation.

## Decision

**The product shell is a Rust workspace plus Tauri v2 plus React 19/TypeScript.**
The workspace already looks like this (`Cargo.toml`, edition 2024, MSRV 1.95):

| Layer | Path |
|---|---|
| Core types, ids, paths, errors | `crates/ohm-core` |
| Device model and capabilities | `crates/ohm-device-model` |
| Adapter contract | `crates/ohm-adapter-api` |
| Runtime: poll, discovery, safety, audit | `crates/ohm-runtime` |
| Automation: curves, rules, engine | `crates/ohm-automation` |
| Providers | `adapters/{mock,system,nvidia,libre-hardware-monitor,open-protocol}` |
| Apps | `apps/cli`, `apps/desktop` |

**The UI never touches hardware.** `apps/desktop/src-tauri/src/commands.rs` states
the contract in its module docs: every `#[tauri::command]` is a thin adapter over
the runtime or the automation engine, reads return owned snapshots, and writes go
through `Runtime::write_value`. `crates/ohm-runtime/src/runtime.rs` makes this
structural: *"Nothing outside this crate is allowed to talk to an adapter
directly"* — the UI, the automation engine and the CLI all pass through range
validation, the `SafetyPolicy` gate and the audit log. The frontend mirrors the
contract in `apps/desktop/src/lib/ipc.ts`, the only module that decides between
the real backend and the in-browser demo backend.

**A .NET sidecar is used only for LibreHardwareMonitor**, out of process, from
v0.2+ (`0003`). The .NET dependency is an isolated leaf: one small process hosting
an unmodified MPL-2.0 NuGet package, targeting **`net10.0`** (LTS to 2028-11-15),
not `net8.0` (retires 2026-11-11). Nothing in `crates/` or `apps/` depends on the
CLR, so losing the sidecar degrades one provider instead of the product.

## Consequences

**Positive**

- One hardware door: every write is validated, safety-checked and audited in
  `ohm-runtime` regardless of who asked — UI, CLI or a rule.
- The same core is exercised by three front ends (the Tauri app, `ohm-cli`, and
  the `tests/` crate), so behaviour cannot diverge between them.
- A small installer: Tauri's claim is a minimal app under 600 KB
  ([v2.tauri.app](https://v2.tauri.app/start/)), and WebView2 defaults to
  `downloadBootstrapper` (~2 MB) rather than the ~127 MB `offlineInstaller`. An
  official self-contained WinUI 3 size is **unverified**; the structural
  difference (loose .NET runtime plus framework package, no single-file publish
  for WinUI 3) is not.
- Cross-platform stays cheap; Linux `amdgpu` sysfs is a licence-free AMD fan path ([kernel docs](https://docs.kernel.org/gpu/amdgpu/thermal.html)).
- Contribution is one crate plus one line in `build_adapters`: the adapter
  contract is a trait.

**Negative**

- **The Tauri glue is what a reviewer must verify by hand.** CI covers the
  runtime, adapters, engine, protocol and CLI, and launches the desktop binary
  headlessly (`cargo run -q -p ohm-desktop -- --selftest --mock` in
  `.github/workflows/ci.yml`); it does not drive the React UI. A PR touching
  `commands.rs`, `src/state.rs`, `src/events.rs`, `src/tray.rs` or
  `tauri.conf.json` must be read against `apps/desktop/src/lib/ipc.ts` and run.
- Every hardware read crosses a process boundary: extra serialization plus a
  supervision problem the in-process .NET design would not have (~1–5 ms added
  latency, **unverified** estimate).
- Two toolchains in CI once the sidecar lands, and its self-contained publish size
  (~15–70 MB, **unverified**) dwarfs the rest of the app.
- Tauri's trademark rules are separate from its licence: forks must strip the logo
  and must not ship the default icon ([trademark](https://v2.tauri.app/about/trademark/)).

## Alternatives considered

- **.NET 8/10 + WinUI 3.** Rejected: it wins on in-process LHM hosting and control
  loop latency, but the Windows App SDK redistributable imposes obligations we
  will not accept (indemnity, "significant primary functionality", no
  source-disclosure licensing), locks the product to Windows, raises contribution
  friction, and makes it *tempting* to link the AMD SDKs directly — the outcome
  `0004` forbids. The .NET advantage survives where it is actually needed: out of
  process.
- **Rust core + WinUI/WPF over a C ABI.** Rejected: inherits the same licence
  problem, adds an FFI boundary with no tooling support, loses the React
  ecosystem.
- **Electron.** Rejected: tens of MB more than a WebView2 host, and buys nothing.
- **A native Rust GUI (`egui`, `iced`, `slint`).** Rejected for v1: no shareable
  component ecosystem for charts and curve editors, and a far smaller contributor
  pool for a fan-curve UI. Revisit only if a webview becomes a liability.
- **In-process CLR hosting (`netcorehost` 0.22.0, MIT).** Rejected for now: an
  unhandled .NET exception or a driver fault would take the UI down, and elevation
  would become all-or-nothing. See `0003` for where it sits on the roadmap.
