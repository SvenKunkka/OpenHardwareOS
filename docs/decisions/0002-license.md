# 0002 — License

## Status

Accepted

## Date

2026-09-11

## Context

OpenHardwareOS reads and *writes* PC hardware, so it necessarily sits next to code
it does not own: a signed kernel driver (PawnIO, GPL-2.0-or-later), a hardware
library (LibreHardwareMonitor, MPL-2.0), vendor driver DLLs loaded at runtime
(NVIDIA's `nvml.dll`) and vendor SDKs whose licences are hostile to free software
(AMD ADL/ADLX — see `0004-vendor-sdks-and-amd.md`). The outbound licence has to
make those relationships legible and still be usable commercially.

The workspace already declares the answer: `[workspace.package] license =
"Apache-2.0"` in `Cargo.toml`, inherited by every crate including
`apps/desktop/src-tauri/Cargo.toml`, and `apps/desktop/package.json` carries
`"license": "Apache-2.0"`.

## Decision

**OpenHardwareOS is licensed under Apache-2.0.**

1. **An explicit patent grant.** §3 grants every contributor's patent claims needed
   to use, sell and distribute the work, and terminates that grant for anyone who
   sues over patents. For a project whose purpose is talking to vendor hardware,
   that is the difference between a contribution and a liability. MIT has no
   patent clause at all.
2. **Explicit contributor terms.** §5 puts anything deliberately submitted for
   inclusion under the licence unless marked otherwise, which removes the "did
   they intend to license this?" question that MIT leaves to a `CONTRIBUTING.md`
   convention.
3. **Permissive for commercial use.** Anyone may ship OpenHardwareOS, modified or
   not, in a closed product — what lets OEMs, system integrators and third-party
   fan utilities adopt the runtime without a legal review.
4. **Compatible with the dependencies we actually use.** The permissive set
   (`nvml-wrapper` MIT OR Apache-2.0, `sysinfo` MIT, `wmi` MIT OR Apache-2.0,
   Tauri Apache-2.0 OR MIT, React MIT) drops in without friction.
   LibreHardwareMonitor is MPL-2.0 — file-level copyleft — and consuming the
   **unmodified** NuGet package from a *separate process* means no obligation
   reaches our Apache-2.0 code (`0003-libre-hardware-monitor-integration.md`). If
   we ever modify an LHM `.cs` file, that file's source is republished under
   MPL-2.0; our files stay Apache-2.0.
5. **It matches the ecosystem.** `tao` (a Tauri dependency) is Apache-2.0-only and
   TypeScript is Apache-2.0, so the outbound licence is the same family as the
   inbound ones and the NOTICE story stays comprehensible.

## Obligations this creates

- Ship `LICENSE` and keep it in every distribution.
- Ship a NOTICE/third-party file covering at least LibreHardwareMonitor (MPL-2.0),
  PawnIO (GPL-2.0-or-later, only if bundled — we do not), NSIS (zlib/libpng +
  bzip2 + CPL-1.0 with a linking exception), WiX (MS-RL, build-time only), the
  WebView2 Runtime (Microsoft terms), and every Rust crate and npm package.
- State modifications: changed files carry prominent notices.
- Keep attribution and NOTICE files intact in redistributed derivatives.

## Rule for new dependencies

`cargo-deny` and `cargo-audit` are the enforcement mechanism and the README
third-party table is the human-readable record. All four rules must hold:

1. **Licence is on the allow list:** Apache-2.0, MIT, BSD-2-Clause,
   BSD-3-Clause, ISC, Zlib, Unicode-DFS, MPL-2.0 (unmodified dependency, or out of
   process), or a dual licence including one of them. LGPL-2.1/3.0 only when
   dynamically linked or invoked as a separate process, and the PR must say which.
2. **Nothing that forbids redistribution.** No proprietary SDK EULA code, no "all
   rights reserved" (no `LICENSE` file at any standard path), no AGPL/GPL-3.0
   inside our artefacts. That is why `ADLXWrapper` (no licence file), OpenRGB
   (GPL-2.0), G-Helper (GPL-3.0) and `silicon-monitor` (AGPL-3.0-or-later) are
   reference reading only. GPL-2.0 `smartctl` and GPL-2.0-or-later PawnIO may be
   *detected and deep-linked*; bundling them makes us a GPL distributor and needs
   a source offer and a NOTICE entry.
3. **It is recorded in the README third-party table** — name, version, SPDX
   identifier, and whether it is runtime, build-time only or optional. A dependency
   merged without a row is a bug.
4. **It drags nothing forbidden in transitively.** `NvAPIWrapper` (LGPL-3.0) is
   rejected even though it would solve NVIDIA fan control; `amdgpu-sysfs`
   (LGPL-3.0-or-later) is accepted on Linux only with the re-linking obligation
   understood, and reading sysfs directly is preferred.

## Consequences

**Positive**

- Commercial adoption and OEM integration are possible without negotiation.
- Contributors get a patent grant and clear inbound terms.
- The licence boundary with MPL-2.0 (LHM) and GPL-2.0-or-later (PawnIO) is one
  sentence: *out of process, unmodified, or not at all*.

**Negative**

- Apache-2.0 is incompatible with GPL-2.0-only code, so anything GPL-2.0 we might
  want to reuse (OpenRGB's SMBus notes, smartmontools' drive coverage) can be read
  and cited, never linked or copied.
- The NOTICE obligation that MIT would not impose is real work in an installer
  that ships a WebView2 bootstrapper and third-party binaries.
- Some Linux distributions treat Apache-2.0 as compatible but not identical to
  their preferred GPL; packaging there stays the distributor's problem.

## Alternatives considered

- **MIT.** Rejected: no patent grant, no explicit contribution clause, no NOTICE
  machinery — for a project touching vendor hardware and vendor patents, the
  missing patent grant is decisive rather than a detail.
- **GPL-3.0.** Rejected: it blocks vendor integration (no OEM can ship a
  proprietary bundle around a GPL-3.0 runtime), blocks closed commercial adoption,
  and turns the intended LHM sidecar and any future Windows service into a
  licensing argument instead of an engineering one.
- **MPL-2.0 for the whole project.** Rejected: file-level copyleft still forces
  disclosure of modified files. We consume MPL-2.0 rather than emit it.
- **Dual Apache-2.0 OR MIT.** Rejected: a second set of terms to explain, and
  nothing in the dependency graph requires MIT specifically.
- **No licence file / "source available".** Rejected: not adoptable, forkable or
  contributable, which is the whole point of the project.
