# Architecture Decision Records

Every decision here was made on **2026-09-11** against verified research
(see [`../research.md`](../research.md)) and against the code in this repository.
Each record is one decision, its context, its consequences — including the
negative ones — and the alternatives that were rejected with the reason.

| ADR | Title | Status | One-line summary |
|---|---|---|---|
| [0001](0001-ui-and-runtime-stack.md) | UI and runtime stack | Accepted | Rust workspace + Tauri v2 + React 19/TypeScript, with the UI never touching hardware, over .NET 8 + WinUI 3. |
| [0002](0002-license.md) | License | **Proposed** | Apache-2.0 for its patent grant and explicit contributor terms, with a dependency allow-list and a README third-party table. |
| [0003](0003-libre-hardware-monitor-integration.md) | LibreHardwareMonitor integration | Accepted | LHM (MPL-2.0) over its HTTP JSON web server as the real-hardware provider, with a sidecar as the planned v0.2+ transport. |
| [0004](0004-vendor-sdks-and-amd.md) | Vendor SDKs and AMD | **Proposed** | No ADL/ADLX code, headers or bindings may enter the repository; AMD comes from the LHM host, NVIDIA telemetry from NVML. |
| [0005](0005-nvml-precedence-and-device-identity.md) | NVML precedence and device identity | Accepted | NVML is registered only when LHM is off, because both would show one GPU twice and LHM reports more. |
| [0006](0006-windows-privileges-and-autostart.md) | Windows privileges and autostart | Accepted | The app runs unprivileged and autostart is a per-user `HKCU\...\Run` entry that can never start an elevated process. |

## Status values

- **Accepted** — the decision is in force and the code follows it.
- **Superseded by NNNN** — replaced; the record is kept for its reasoning.
- **Proposed** — written down, not yet in force.
- **Deprecated** — no longer relevant, kept for history.

Four records are **Accepted** (they describe technical decisions the code already
follows) and two are **Proposed**: 0002 (the Apache-2.0 choice and the dependency
allow-list) and 0004 (the AMD ADL/ADLX restriction). Both are legal decisions for the
project owner; the code follows them so the workspace stays buildable and auditable,
but neither is in force and neither may be cited as approved. Where a record describes something
that is *not* implemented yet, it says so explicitly and names the plan — the
`physical_id`/adapter-priority change in 0005 and the Task Scheduler/service
helper in 0006 are the two examples.

## The shape of a record

```markdown
# NNNN — Title

## Status

Accepted

## Date

2026-09-11

## Context

What forces are at play, with file paths and citations.

## Decision

What we do, precisely enough to be checked against the code.

## Consequences

Positive and negative. A record with no negative consequences is a record that
has not been thought through.

## Alternatives considered

Each option with the reason it was rejected.
```

## Adding a record

1. Take the next free number and a short kebab-case title:
   `docs/decisions/0007-some-decision.md`.
2. Use **today's date** in `Date`, not the date of an older record.
3. Link the files and crates the decision touches, so a reader can verify it in
   one hop.
4. Add the row to the table above — an ADR that is not in the index does not
   exist.
5. If the change reverses an earlier decision, keep the old record and mark it
   **Superseded by NNNN** instead of editing its conclusion away. Decisions
   change; the history of *why* is the point of keeping them.
