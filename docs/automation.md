# Automation

A rule maps a sensor reading through a curve onto an actuator value; the runtime writes
it. A rule is *data*, not code — YAML the user can read, diff and share. Implementation:
`crates/ohm-automation` (`curve.rs`, `rule.rs`, `evaluator.rs`, `engine.rs`, `store.rs`,
`examples.rs`).

```text
sensor ──▶ curve ──▶ rule limits ──▶ hardware range ──▶ hysteresis ──▶ deadband ──▶ write
    └─ missing ──▶ fallback: hold / safe_default / fixed / release
```

## The shipped example

`crates/ohm-automation/src/rule.rs` — `tests::documented_yaml_parses` asserts exactly this
document, including the derived id `gpu-cooling`:

```yaml
name: GPU Cooling
source:
  device: gpu.nvidia.0
  capability: temperature.core
target:
  device: fan.system.0
  capability: fan.speed_percent
curve:
  - [40, 20]
  - [60, 35]
  - [70, 50]
  - [80, 80]
  - [85, 100]
hysteresis: 2
update_interval_ms: 1000
```

`examples.rs` — `gpu_cooling_example()` is the same rule on the simulated machine
(`gpu.mock.0` → `fan.mock.0`); it is what `AutomationEngine::ensure_example` installs and
what `ohm-cli demo` runs. `examples/rules/*.yaml` are the copy-and-paste versions, and
`apps/cli/tests/yaml_examples.rs` — `every_shipped_example_rule_parses_and_validates`
parses and validates all three on every test run.

## Rule schema as implemented

| Field | Type | Default | Enforced by |
|---|---|---|---|
| `id` | string | derived from `name` | `Rule`'s `Deserialize`, then the file name in `RuleStore::load_file` |
| `name` | string | — (required, non-blank) | `Rule::validate` |
| `enabled` | bool | `true` | `evaluate` (→ `Disabled`, no write) and `tick_inner` (skips the rule) |
| `description` | string? | absent | display only |
| `source` | `Source` | — (required) | `Rule::validate`, `check_rule` |
| `when` | `Condition?` | absent (`None`) | `Rule::validate`, `check_rule`, `evaluator.rs` — `resolve_gate` |
| `target` | `Target` | — (required) | `Rule::validate`, `check_rule`, `Runtime::write_value` |
| `curve` | `[[in, out], …]` | — (required, ≥ 2 points) | `Curve::validate` |
| `hysteresis` | f64 | `2.0` | `evaluate` step 6 |
| `deadband` | f64 | `0.5` | `evaluate` step 7 |
| `update_interval_ms` | u64 | `1000` | `tick_inner` via `RuleState::next_due_ms` |
| `min_output` / `max_output` | f64? | absent (no limit) | `Rule::clamp_output` |
| `fallback.on_sensor_missing` | `FallbackAction` | `safe_default` | `evaluate_fallback` |
| `fallback.on_write_failure` | `FallbackAction` | `safe_default` | `handle_write_failure` |
| `fallback.sensor_timeout_s` | u64 | `0` | `stale_value_within_grace` in `evaluator.rs` — a grace period before the fallback |
| `priority` | u32 | `0` | sort order in `load_rules` / `save_rule` |
| `created_at_ms` / `updated_at_ms` | i64? | absent; set by `save_rule` | bookkeeping |

`DEFAULT_UPDATE_INTERVAL_MS = 1_000`, `MIN_UPDATE_INTERVAL_MS = 100`,
`DEFAULT_HYSTERESIS = 2.0`, `DEFAULT_DEADBAND = 0.5`.

**Source** is an untagged enum: one sensor, or several reduced by
`Aggregate::{Max, Min, Avg}` (`#[serde(default)]` = `Max`, the hottest sensor wins).
`Source::sensors()` flattens both shapes to a `Vec<SensorRef>`, so validation and device
collection do not branch. `Aggregate::reduce` drops non-finite readings and returns `None`
for an empty input, so "all sensors missing" becomes the fallback path, not a zero.
`read_source` reads through `Runtime::reading(device, capability)` and applies `reduce`.
The combined form in YAML:

```yaml
source: {aggregate: max, sensors: [{device: cpu.mock.0, capability: temperature.core},
                                  {device: gpu.mock.0, capability: temperature.core}]}
```

**Target** is `Target { device, capability }` — one capability of one device; fans and
pumps are driven in percent. **Fallback**: `FallbackAction::duty(safe_default)` returns
`None` for `hold`/`release` and `Some(..)` otherwise; `FallbackAction::default()` is
`SafeDefault`, because losing a sensor must never leave a fan slow.

| Action | `on_sensor_missing` | `on_write_failure` |
|---|---|---|
| `hold` | keep the last output; `should_write = false` | report the error, change nothing |
| `safe_default` | write `SafetyPolicy::fail_safe_duty_percent` (70 %) | write it with `WriteOrigin::Safety` |
| `fixed: {percent: N}` | write `N`, clamped to the target range | write `N` the same way |
| `release` | set `release`/`released`, write nothing | mark released, publish `rule_released` |

```yaml
fallback: {on_sensor_missing: safe_default, on_write_failure: hold, sensor_timeout_s: 10}
```

**`id` derivation.** `id` is optional; a private `RuleDef` shadow struct computes
`RuleId::new_unchecked(slugify(&name))` when absent, so `GPU Cooling` → `gpu-cooling`.
`slugify` lowercases ASCII alphanumerics, collapses every other character run into one `-`,
trims trailing dashes, and falls back to `"rule"` (`"..." → "rule"`). On load,
`RuleStore::load_file` overrides the declared id with the file stem: the file name is
authoritative, so renaming `a.yaml` to `b.yaml` must not change which rule the UI edits
(`store.rs` — `tests::file_name_wins_over_declared_id`).

## The evaluation pipeline

`evaluator.rs` — `evaluate(rule, state, input)`: pure logic, no hardware, no I/O, no clock
reads (`now_ms` arrives in `EvaluationInput`). Exact order:

1. **Disabled.** `!rule.enabled` → `Disabled`, no output, no write; `next_due_ms` advances.
2. **Missing source.** `input.source_value.filter(|v| v.is_finite())` — `None` or NaN/∞
   first consults `stale_value_within_grace` (see "The sensor grace period"), then branches
   to `evaluate_fallback`.
3. **Curve.** `rule.curve.eval(source_value)`: below the first point returns the first
   output, above the last returns the last, in between linear interpolation. A non-finite
   input returns `Curve::min_output()`, so a broken reading can never command full speed.
4. **Rule limits.** `rule.clamp_output(curved)` clamps to `Rule::effective_output_range()`
   = `(min_output ?? -∞, max_output ?? +∞)`.
5. **Hardware range.** `.clamp(input.target_min, input.target_max)` from the target
   capability's `min`/`max` (defaulting to `0.0`/`100.0`).
6. **Hysteresis** (below).
7. **Deadband.** `delta = |target − state.applied_output|`;
   `should_write = delta >= rule.deadband && delta > f64::EPSILON`. With no previous output
   the first evaluation always writes.
8. Returns `Evaluation { status, output, should_write, message, next_due_ms, release }`.

`evaluate` never touches the runtime; the engine writes. Status is `Applied` when writing,
`Held` otherwise, and the message distinguishes the two holds: `"holding 50 % until
gpu.mock.0/temperature.core drops below 68.0"` versus `"no change worth writing (0.60 %
within the 0.50 % deadband)"`. `evaluate` updates only the *decision* fields (`last_input`,
`armed_input`, `next_due_ms`, `last_status`, `evaluations`); the engine sets
`applied_output`, `writes` and `last_write_ms` only after a successful write, so a failed
write never poisons the control memory.

### Hysteresis and deadband

`RuleState::armed_input` remembers the input that produced the current output. Falling only
happens after a real drop:

```rust
if clamped < applied {                                  // falling
    if armed_input.is_finite() && source_value > armed_input - rule.hysteresis {
        target = applied;                               // hold
    } else { armed_input = source_value; }              // real drop: re-arm here
} else if clamped > applied {                           // rising re-arms immediately
    armed_input = source_value;
}
```

The output falls when `source_value <= armed_input − hysteresis` (the hold test is a strict
`>`, so equality steps down). An anchor that is not finite — for example right after a
fail-safe write — must not freeze the rule, hence the finiteness check. The engine stores
the value the runtime *actually applied*, i.e. after the safety floor, so if the floor
raises a curve value the rule holds at the raised value until the source drops below the
anchor: the alternative is a rule that fights the floor every tick.

**Deadband** is the minimum output change worth writing, in output units against the last
applied output; it stops re-writing a fan for noise the hardware cannot express.

### A worked numeric example

`gpu_cooling_curve()` = `[(40,20), (60,35), (70,50), (80,80), (85,100)]`; slopes 0.75 %/°C
on 40–60, 1.5 on 60–70, 3.0 on 70–80, 4.0 on 80–85. Defaults: `hysteresis = 2.0`,
`deadband = 0.5`, target range 0–100, safety floor 25 %.

| Tick | Reading | Curve | Step | `armed_input` | Applied |
|---|---|---|---|---|---|
| 1 | 68.0 °C | `35 + 1.5·8 = 47.00` | no previous output → write | 68.0 | **47.00 %** |
| 2 | 70.0 °C | `50.00` > 47 → rising, re-arm | write (3.00 ≥ 0.5) | 70.0 | **50.00 %** |
| 3 | 69.0 °C | `48.50` < 50; is `69 > 70 − 2`? yes → hold | `Held`, no write | 70.0 | 50.00 % |
| 4 | 67.9 °C | `46.85`; is `67.9 > 68`? no → re-arm | write (\|46.85−50\| = 3.15 ≥ 0.5) | 67.9 | **46.85 %** |

Tick 3 is the hysteresis (a 1 °C wobble does not move the fan); tick 4 is the real 2.1 °C
drop, which does. With `deadband = 2.0` and `applied_output = 50.00`, a reading of 69.5 °C
gives `35 + 1.5·9.5 = 49.25` — a 0.75 delta, held. At `hysteresis = 0` the rule follows the
curve exactly (70 °C → 50.00, 69 °C → 48.50; `evaluator.rs` —
`tests::hysteresis_zero_follows_the_curve_exactly`).

## What `applied` means

A `WriteReport` distinguishes three things, and the difference matters when you
are deciding whether a fan actually moved:

| Field | Meaning |
|---|---|
| `requested` | what the caller asked for |
| `applied` | what the adapter reported the device ended up with, after range clamping and the safety policy |
| `status` | `applied` (the hardware took it), `simulated` (dry run, or simulated hardware), `rejected` (refused, with `error_code` and `detail`) |

`applied` is **not** a read-back unless the adapter does one. Today:

* `adapters/libre-hardware-monitor` **reads the channel back** after every write and
  treats a value more than `READ_BACK_TOLERANCE_PERCENT` (1 %) away as a refusal —
  the failure it guards against is LHM answering `200 OK` while the SuperIO holds
  the old duty, which would otherwise be reported as success. If the channel
  cannot be read back at all (LHM answers `N/A`), the write is reported as
  applied but the `detail` says the value is *requested, not confirmed*;
* `adapters/mock` applies the value to its own state, so `applied` is exact;
* `adapters/nvidia` reports what NVML accepted; NVML offers no per-call read-back
  on the write path, and the next poll is what shows the result.

Anywhere a write looks successful but the hardware disagrees, the next poll will
show it — and the audit trail records both the request and what was reported.

## The engine

**Tick scheduling.** `TICK_INTERVAL_MS = 100`. `start()` spawns one Tokio task that sleeps
100 ms then calls `tick()`. Each rule carries
`RuleState::next_due_ms = now_ms + rule.update_interval_ms`, set at the end of every
evaluation; `tick_inner` skips a rule whose `next_due_ms > now` unless forced. The 100 ms
tick is the *resolution* of scheduling, not the evaluation rate — the default rule runs
once per second. `tick_force()` is the same loop with `force = true`, ignoring the
schedule, for the CLI's time-compressed simulator and for tests. `tick_inner` returns empty
immediately when `settings.automation_enabled` is false (Settings → Features).

**Writing.** The engine resolves the target once (`Runtime::resolve(device, capability)`);
a target that disappeared yields `RuleStatus::Error` with `"target unavailable: …"` and an
`Automation` bus event, without affecting any other rule. A write always goes through
`Runtime::write_value` with `WriteOrigin::Automation { rule_id }`:

```rust
let origin = WriteOrigin::Automation { rule_id: rule.id.clone() };
runtime.write_value(device.id.as_str(), capability.id.as_str(), Value::Number(output), origin).await
```

Automation therefore gets exactly what a manual write gets: resolve, `enabled`,
`capability.writable`, `capability.validate`, `capability.clamp`, the
`SafetyPolicy::check_duty` gate, the dry-run branch, `write_gate` (a `tokio::sync::Mutex`
serialising adapter writes so two rules cannot interleave on one actuator), the audit log
and the event bus. The engine has no privilege the UI lacks. On success it records
`report.applied`, bumps `state.writes`, sets `last_write_ms`, **keeps the evaluation's
status** (a fail-safe write stays `Fallback`, not `Applied`) and publishes `rule_applied`.

**When a write fails**, two layers act in order:

1. **The runtime's fail-safe.** `Runtime::write_with_origin` audits the rejection and then,
   for a duty control with `allow_fail_safe`, writes
   `SafetyPolicy::fail_safe_duty(device_type)` with `WriteOrigin::Safety`, and
   `allow_fail_safe = false` so it cannot recurse. `fail_safe_duty` is
   `fail_safe_duty_percent` raised to the duty floor, so a fan can never be fail-safed
   below its minimum.
2. **The rule's `fallback.on_write_failure`**, in `handle_write_failure`: `Release` →
   `Released`, `released = true`, `rule_released`; `Hold` → keep `Error`, change nothing;
   `SafeDefault`/`Fixed` → write that duty with `WriteOrigin::Safety { reason: "automation
   rule <id> write failure" }`. If that second write also fails, the log says *"Control may
   be lost."* — nothing is hidden.

`engine.rs` — `write_failure_is_reported_and_falls_back`,
`write_failure_with_release_fallback` cover both.

**Release handling.** `release` is set only by `FallbackAction::Release`.
`RuleState::released` latches it; the engine publishes `rule_released` and logs a warning,
and the first finite reading clears `released` and takes control back (`evaluator.rs` —
`tests::fallback_variants_behave`: "when the sensor comes back the rule takes over again").
A release is a statement, not a disable — the firmware's own curve runs until the rule has
data again.

**Rule state across edits.** `RuleState` lives in a `Mutex<HashMap<RuleId, RuleState>>`.
`save_rule` replaces the rule in memory, writes the file, then sets the existing state's
`next_due_ms = 0`, so an edited rule is re-evaluated on the next tick **without losing its
hysteresis anchor**. `load_rules` `retain`s only states whose rule id is still on disk and
inserts fresh states for new ids, so editing a rule never resets a running fan to the curve
value. `delete_rule` removes the state; `reset_states()` clears `applied_output`,
`armed_input` and `released` for every rule; `stop()` keeps the map so a restart resumes
smoothly.

## Validation

`Rule::validate()` is structural and hardware-blind: non-blank name, valid curve (≥ 2
finite points, strictly increasing inputs), at least one source sensor, non-negative finite
`hysteresis` and `deadband`, `update_interval_ms >= 100`, `min_output <= max_output` inside
0–100, and any `Fixed { percent }` inside 0–100.
`AutomationEngine::check_rule(&rule) -> RuleCheck { errors, warnings }` adds what only the
device table knows. **Errors block the save** (`RuleCheck::into_result`); **warnings do
not.**

| Kind | Condition | Message |
|---|---|---|
| error | `Rule::validate` failed | `automation error: …` |
| error | the source does not resolve | `source <ref>: device … was not found` |
| error | source not readable, or not sensor-kind | `<ref> is not a readable sensor` |
| error | the target does not resolve | `target <ref>: …` |
| error | target not writable | `<ref> is read-only; pick an actuator` |
| error | `min_output > max_output` | `output limits are inverted` |
| warn | source unit is not a temperature | `<ref> is a <unit> sensor; a cooling curve normally reads a temperature` |
| warn | neither controllable nor readable | `<device> looks offline` |
| warn | curve range outside the device range | `curve output 20-100 % is outside the device range 40-100 %; values will be clamped` |
| warn | `min_output` below the device minimum | `min_output 10 % is below the device minimum 25 %` |
| warn | target is not a duty control | `<ref> is not a duty control; the safety floor cannot be applied` |
| warn | device wants a safe floor, unit is not duty | `<ref> controls a <type> through a <unit> capability` |
| warn | `hysteresis == 0` | `hysteresis is 0: the fan may oscillate around a curve knee` |
| warn | interval < 250 ms | `update_interval_ms is 200; values below 250 ms can flood the hardware` |

*Curve outside the device range* is a **warning**, not an error: the evaluator clamps, so
the rule still runs and still cools. The `< 250 ms` warning is reachable with a valid rule,
because `MIN_UPDATE_INTERVAL_MS` is 100. `save_rule` always calls `check_rule` first and
never reaches `store.save` on error. **`check_rule` has no privileged mode and no bypass,
so an AI-authored rule passes through the same gate as a hand-written one.**

## Safety interactions

Automation sits *inside* the safety architecture and cannot opt out.

* **The safety floor.** After curve → rule limits → hardware range,
  `Runtime::write_value` sends duty writes through `SafetyPolicy::check_duty`.
  `min_duty_percent` (25 %) and `pump_min_duty_percent` (60 %, unconditional) raise the
  value, and the report comes back `clamped = true`. The demo shows it: the rule asks for
  `"requested":20.0` and the fan ends at `"applied":25.0`.
* **`safe_default` comes from the safety policy.** `tick_inner` reads
  `settings.safety.fail_safe_duty_percent` once per tick and passes it as
  `EvaluationInput::safe_default_duty`; `SafeDefault.duty(...)` returns it and the
  evaluator clamps it to the target range — a rule with no `fallback:` block still fails
  safe.
* **The emergency supervisor overrides rules, upwards only.** `Runtime::supervise_safety`
  runs after *every* poll, independently of any rule: if the hottest temperature across all
  enabled devices reaches `emergency_temp_c` (90 °C), it writes `emergency_duty_percent`
  (100 %) to every enabled cooling device's writable duty control — but only where the
  current value is below it (`if current.is_some_and(|c| c >= duty) { continue; }`). It
  never lowers an output; releasing back down is the rule's job on the next tick.
  `check_duty` enforces the same asymmetry: `SafetyDecision::Emergency` fires only when
  `requested < duty`, so a rule asking for 100 % is not "clamped" down to a lower emergency
  duty (`safety.rs` — `emergency_overrides_a_low_request`,
  `emergency_does_not_lower_a_higher_request`).
* **Dry run.** `Settings::dry_run` short-circuits `Runtime::write_with_origin` before the
  adapter is called. The value still passes range validation and the safety gate, comes
  back `status: WriteStatus::Simulated` with `detail: "dry run: hardware was not touched"`,
  and is audited and published as usual. The engine cannot tell the difference, so the
  hysteresis anchor advances exactly as with real hardware — what makes dry run a
  rehearsal rather than a simulation. `ohm-cli` forces `dry_run = true` whenever it enables
  `--mock`.

## Storage and editing

Rules live in `<config>/rules/`, one YAML file per rule — on Windows
`%APPDATA%\OpenHardwareOS\rules`; `OHM_CONFIG_DIR` overrides the root
(`ohm-core/src/paths.rs`).

* **File name = rule id.** `RuleStore::path_for(id)` is `<id>.yaml`; `load_file` takes the
  id from the file stem. Only `.yaml`/`.yml` files are read, sorted by path.
* **Atomic writes.** `save` writes `<id>.yaml.tmp` and renames it over the target, after a
  three-line header (`# OpenHardwareOS automation rule` / `# Managed by the app; hand edits
  are welcome.` / `# id: <id>`). A crash mid-write cannot leave a half-written rule.
* **Corrupted files are skipped, not fatal.** `load_report()` returns `LoadReport { rules,
  errors: Vec<RuleFileError { path, message }>, directory }`; `is_clean()` is
  `errors.is_empty()`. `load_rules` logs `"N rule file(s) could not be loaded"` and adopts
  the rules that parsed (`store.rs` — `tests::broken_files_are_reported_not_fatal`: a
  malformed file and one with no `source` are both skipped while the good rule loads). A
  missing `rules/` directory is not an error.
* **Hand edits are first-class:** `tests/tests/rule_lifecycle.rs` —
  `documented_yaml_runs_the_cooling_loop` writes `gpu-cooling.yaml` by hand, reloads from
  disk, and drives the simulated fan.
* **Installing an example.** `AutomationEngine::ensure_example()` (desktop startup) does
  nothing if any rule exists. Otherwise it `check_rule`s the shipped mock example; if the
  mock devices are present it saves it, and if not it installs the first `suggest_for`
  suggestion for the attached hardware. Returns `false` when there is nothing to install.
* **Suggestions.** `examples::suggest_for(&Runtime)` builds rules from the live capability
  index (`Runtime::capability_index()` → `CapabilityRegistry::index`), not a hard-coded
  table, and returns empty when there is no writable target, no readable source, or no
  temperature source. Otherwise: `GPU Cooling` for a `Gpu` temperature source,
  `CPU Cooling` for a `Cpu` one (its own flatter curve), and `System Cooling (MAX of all
  sensors)` when at least two temperature sources exist and both a CPU and a GPU were
  found — the aggregate takes the first four. Targets are `index.targets[0]` and, for the
  CPU rule, `index.targets[1]` when a second exists.

## The UI

`apps/desktop/src/screens/Automation.tsx`. **Rule cards** show name, status and priority
badges, an enable switch, Edit, Delete behind a confirm step, the source and target, the
**live source value and the curve value it produces**, hysteresis / deadband / interval /
output limits, both fallback actions, per-rule counters and the last message, beside a
`CurvePreview` with the current operating point marked. **The form** covers name, priority,
description, enabled; source as *Single sensor* or *Aggregate*; a target from writable
capabilities only; a curve table with *Load GPU template* / *Load chassis template* / *Add
control point*; and hysteresis, deadband, update interval, min/max output, sensor timeout,
both fallback dropdowns and the fixed percentage. **Check & save** calls `api.checkRule`
first: blocking errors stop the save, warnings do not. "Suggested for your hardware" adds a
suggestion with `enabled: false`. The header shows an automation-enabled flag and a **dry
run** badge; the switches live in `Settings.tsx` under *Features* and *Safety policy* (all
eleven `SafetyPolicy` fields).

## CLI

`ohm-cli` runs the same runtime and engine as the desktop app, is deterministic, and exits
non-zero on failure. The subcommands are `list`, `suggest`, `delete`, `set-enabled` and
`check` (plus `demo`, `doctor`, `status`, `watch`, `audit`, `paths`, `protocol` at the top
level). `rules suggest --mock` is the only rules subcommand that enables the simulated
providers; the rest validate against the machine you are on.

```console
$ ohm-cli rules suggest --mock
  • GPU Cooling (gpu-cooling)   gpu.mock.0/temperature.core -> fan.mock.0/fan.speed_percent
      (5 points, hysteresis 2.0, every 1000 ms)
  • CPU Cooling (cpu-cooling)   cpu.mock.0/temperature.core -> fan.mock.1/fan.speed_percent
  • System Cooling (MAX of all sensors) (system-cooling)  MAX(cpu.mock.0/…, …) -> fan.mock.0/…
  rules are stored in /tmp/ohm/rules

$ ohm-cli rules list
  gpu-cooling     idle  gpu.mock.0/temperature.core -> fan.mock.0/fan.speed_percent
      not evaluated yet
  system-cooling  idle  MAX(cpu.mock.0/temperature.core, cpu.system.0/temperature.core,
      fan.openhardwareos_of4_sim_0001.0/temperature.core, gpu.mock.0/temperature.core)

$ ohm-cli rules delete gpu-temperature-case-fan   ->  deleted `gpu-temperature-case-fan`
$ ohm-cli rules delete nope                       ->  no such rule: `nope`

$ ohm-cli rules check /tmp/ohm-rule-bad.yaml
  rule: Broken (broken)
  error:   automation error: update_interval_ms must be at least 100
  error:   source gpu.mock.0/fan.rpm: device `gpu.mock.0` was not found
  error:   target fan.mock.0/fan.rpm: device `fan.mock.0` was not found
  warning: hysteresis is 0: the fan may oscillate around a curve knee
  warning: update_interval_ms is 50; values below 250 ms can flood the hardware
Error: the rule has 3 error(s)
```

`rules set-enabled` takes the boolean explicitly (`#[arg(action = clap::ArgAction::Set,
value_parser = clap::value_parser!(bool))]`, because a bare `bool` field derives as a
flag). It calls `AutomationEngine::set_rule_enabled` → `save_rule` → `check_rule`, so **the
toggle fails when the rule's devices are not attached**, and only `rules suggest` registers
the simulated providers: `rules set-enabled gpu-temperature-case-fan false` exits 1 with
`Error: automation error: source gpu.mock.0/temperature.core: device … was not found`, and
`rules set-enabled nope true` exits 1 with `Error: automation error: no rule with id
\`nope\``. `rules delete` does not validate, so a rule for absent hardware can still be
removed; the desktop toggle has no such limit, because the app has the hardware by
definition.

## End to end: "GPU Temperature → Case Fan" on simulated hardware

`ohm-cli demo` installs exactly the documented rule (`gpu.mock.0/temperature.core` →
`fan.mock.0/fan.speed_percent`, `gpu_cooling_curve()`, `hysteresis: 2`, `deadband: 1`) on the
deterministic simulated machine, starts it hot under a gaming load profile, and steps the
simulated clock in the same order the desktop app uses: advance the simulation,
`runtime.poll_once()`, then `engine.tick_force()`.

```console
$ ohm-cli demo --steps 45 --step-seconds 2
  installed rule  : demo-gpu-cooling (GPU temp -> chassis fan)

    sim s   cpu °C   gpu °C   fan duty   fan RPM        rule
        2     30.5     60.2        35%       965     applied
       12     54.9     55.6        32%       907     applied
  WARN thermal emergency: forcing cooling to maximum detail="90.1 °C is at or above the 90 °C emergency threshold"
       22     67.4     73.4       100%      2000     applied
       32     32.3     40.4        25%       800     applied
       90     40.5     39.0        25%       800        held

  start GPU temperature : 60.2 °C   hottest GPU   : 73.4 °C
  final GPU temperature : 39.0 °C   final fan duty: 25 % (800 RPM)
  rule    : holding 25 % until gpu.mock.0/temperature.core drops below 23.0 — 45
            evaluations, 33 writes, 12 skipped, 0 fallbacks
```

* The loop closes: 60.2 °C → 39.0 °C, the fan following the curve (60 °C → 35 %, 73 °C →
  ~69 %, 90 °C → 100 %). The **25 % floor is the safety policy, not the curve**: the audit
  entry shows `"requested":20.0, "applied":25.0, "clamped":true, "status":"simulated"` with
  `"origin":{"kind":"automation","rule_id":"demo-gpu-cooling"}`.
* The final message is a **hysteresis** hold, not a deadband hold: at 25 °C the curve wants
  20 %, the applied output is 25 %, and the rule holds until the source drops below
  `armed_input − hysteresis` = 23.0 °C — the safety floor participating in hysteresis.
* The **emergency supervisor fired on its own** at 90.1 °C and drove the fan to 100 % — no
  automation rule involved in that decision. `33 writes / 12 skipped` out of 45 evaluations:
  hysteresis and the deadband absorbed the rest. The demo exits non-zero if the rule never
  wrote, so a broken loop fails rather than printing a nice table.
* `0 fallbacks`: the source never disappeared. `tests/tests/edge_cases.rs` —
  `sensor_disconnect_falls_back_to_a_safe_duty` covers the other case: the reading goes
  unavailable, the rule writes `fail_safe_duty_percent` (70 %) and the fan reaches 70 %.

## The sensor grace period

`fallback.sensor_timeout_s` (default `0`) is a per-rule grace period, implemented by
`evaluator.rs` — `stale_value_within_grace`:

* `0` — strict: the first missing reading takes the fallback path.
* `N > 0` — while the source has been silent for less than `N` **wall-clock** seconds, the
  rule keeps steering on its last known value and says so in its message
  (`… (source silent for 1.2 s, steering on the last known value)`).

Two details that matter:

* the anchor (`RuleState::last_input_ms`) is only moved by a *fresh* reading. Refreshing it
  on a stale one would slide the window forward on every evaluation and the rule would never
  reach its fallback;
* a rule that has never had a reading falls back immediately — the grace period reuses a
  value, it never invents one.

`tests/tests/edge_cases.rs` — `a_configured_grace_period_rides_out_a_sensor_blip` and
`the_default_reacts_to_the_first_missing_reading` pin both down, and the global
`Runtime::is_stale(3)` check in `tick_inner` still covers the case where polling itself
stopped (`engine.rs` — `tests::stale_runtime_forces_the_fallback_path`).

## The `when` gate

A rule may be gated behind one numeric condition, so "keep the GPU cool **while
gaming**" needs no second rule:

```yaml
name: GPU Cooling (gaming only)
source: { device: gpu.mock.0, capability: temperature.core }
when:
  source: { device: gpu.mock.0, capability: load.gpu }
  op: gt                    # gt | gte | lt | lte | eq | ne
  value: 60                 # in the unit of the condition's capability
  otherwise: safe_default   # or: fixed: { percent: 40 }
target: { device: fan.mock.0, capability: fan.speed_percent }
curve:
  - [40, 20]
  - [60, 35]
  - [70, 50]
  - [80, 80]
  - [85, 100]
hysteresis: 2
update_interval_ms: 1000
fallback:
  on_sensor_missing: safe_default
  sensor_timeout_s: 5
```

Semantics, every one covered by a test in `crates/ohm-automation/src/evaluator.rs`:

* **Condition true** → the rule behaves exactly like an ungated one, driving the
  curve.
* **Condition false** → the rule *stands down*: it stops steering and drives the
  target to `otherwise` — the runtime's fail-safe duty (`safe_default`, the
  default) or an explicit `fixed.percent`. The status is `gated`, and the message
  quotes the reading that failed the comparison.
* **No unbounded hold.** Leaving a fan wherever it happened to be because a load
  condition went quiet is the failure mode the safety layer exists to prevent, so
  `OtherwiseAction` does not offer it.
* **No `release` either.** No adapter in this build advertises a channel it can
  hand back mid-run (`LibreHardwareMonitorAdapter::shutdown` releases on exit,
  which is a different thing), so offering it would be a promise the code cannot
  keep.
* **Missing or stale condition sensor** → the rule's existing sensor policy:
  inside `fallback.sensor_timeout_s` the last known condition reading is reused
  (the anchor only moves on a *fresh* reading, so the window cannot slide), and
  past it the rule falls back. An unknown gate never counts as open.
* **Boundaries are exact.** `gt`/`gte`/`lt`/`lte` at the threshold behave as
  named; `eq`/`ne` use `Comparator::EQUALITY_TOLERANCE` (1e-6) and `check_rule`
  warns when they are used on an analog reading. A non-finite reading satisfies
  nothing, and a non-finite threshold is a validation error.
* **The emergency ceiling is unaffected.** The supervisor in
  `crates/ohm-runtime/src/runtime.rs` runs independently of rules, and a gated
  rule's write is still clamped by `SafetyPolicy::check_duty`.
* **Reopening the gate resumes from the curve**: the hysteresis anchor is cleared
  while the rule stands down, so a stale anchor cannot pin the fan at the
  stand-down value.

## One output, one writer

An earlier revision of this document recorded this as a gap rather than fixing
it: `priority` only ordered *evaluation*, so on a shared target the **last** write
in a tick won — the lowest-priority rule — while the UI implied arbitration that
did not exist. It is now an enforced rule instead:

> **A target may be owned by at most one enabled rule.**

Enforced at every entry point, with one wording
(`AutomationEngine::conflict_message`):

| Entry point | Behaviour |
|---|---|
| Creating or editing a rule (`save_rule`) | Refused; the error names both rules and the target |
| Enabling a rule (`set_rule_enabled(id, true)`) | Refused, same message |
| The form, before saving (`check_rule`) | The same clash appears in `errors[]`, so the UI can warn before the click |
| Rule files loaded at startup (`load_rules`) | Deterministic resolution: highest `priority` (ties by lowest id) keeps the output; the others are **disabled for the session only**. Files are untouched, and `AutomationEngine::conflicts()` returns a `RuleConflict` per clash (`owner`, `blocked`, `target`, `resolution`) for the UI |
| A conflict that still reaches the active set (`tick`) | The second writer is skipped for that cycle, reported as `RuleStatus::Error` (`another enabled rule already drove … this cycle`) and published as `rule_conflict_skipped`. **Two values are never alternated onto one channel in a cycle** |

So `priority` still means "evaluate and write first", and no longer has to
arbitrate anything; the UI hint says exactly that. Real arbitration (scenes,
weighting, intent merging) is v0.6 work — see `docs/roadmap.md`.

## Not implemented (roadmap)

The MVP implements `sensor -> curve -> actuator` and nothing else. Nothing below has a
field, a flag or a code path in this repository today.

* **WHEN / IF / THEN conditions.** No `condition` block: no "only while a game is running",
  no "only above 20 % CPU load", no time windows, no AND/OR composition.
  `Source::Combined` (`max`/`min`/`avg`) is the only multi-input form, and it is arithmetic,
  not logic.
* **Scenes and profiles.** No grouping of rules that enable together, no scene switcher, no
  per-application profile. Rules are independent top-level files.
* **Application and game detection.** Nothing enumerates running processes for automation.
* **Natural language automation.** The planned shape, stated in `lib.rs`: a sentence such as
  *"keep the GPU cool while gaming, quiet otherwise"* is translated into exactly this YAML,
  which then goes through `check_rule` before it can be saved or run:

  ```text
  sentence ──▶ structured rule (YAML) ──▶ check_rule ──▶ save_rule ──▶ engine ──▶ Runtime::write_value
  ```

  The AI gets a *proposal* path and nothing else: no runtime handle, no adapter access, no
  write path of its own, no way to skip the safety policy or the audit log. The rule schema
  is already fully serialisable and hardware-agnostic, which makes the AI layer a parsing
  problem rather than a safety problem — but no such module, prompt, model or endpoint
  exists in this repository.

Also missing around the edges, and tracked in [`roadmap.md`](roadmap.md): no import/export
button for rule files (the files themselves are portable), no drag-handle curve editor (the
preview is a preview), and no "restore the firmware's original fan curve" action beyond
`relinquish_on_exit`.
