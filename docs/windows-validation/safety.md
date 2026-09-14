# Safety rules for hardware writes during validation

Read this with the person who owns the machine, **before** the session, and agree it out
loud. If anything here cannot be met, the write steps
([`checklist.md`](checklist.md) §10.3 and §12) are not run — the read-only steps still are,
and a report that stops before the first write is a perfectly good report.

These rules are narrower than the application's own safety layer, deliberately. The
application protects the hardware; this document protects the person and the machine from
the *operator's* session.

---

## 1. The four rules that come before everything else

1. **No unattended writes.** A write happens only while a person is standing at the
   machine, watching it. No writes over a remote session, no writes in a script that runs
   later, no writes started and then left.
2. **Never on a machine whose cooling you cannot physically observe.** You must be able
   to see the fans and hear the machine, and you must know where its power switch or PSU
   switch is. If it is in a rack you cannot reach, a case you cannot open or hear, a VM, or
   someone else's machine you cannot touch — stop before the write steps.
3. **No step in this kit is run on someone else's machine without their explicit,
   informed agreement.** "Informed" means they know that the session may change fan or
   pump speeds, that a firmware-overridden header can behave unexpectedly, and that the
   operator can stop at any point. Their agreement is recorded in the sign-off block of
   [`result-template.md`](result-template.md) §9, and it can be withdrawn at any moment —
   including mid-session.
4. **If in doubt, do not write.** An unrun step recorded as **Prepared** costs nothing. A
   written fan that nobody is watching can cost a component.

## 2. The application's own limits, which this session does not override

These are the shipped defaults of `SafetyPolicy` in
[`crates/ohm-runtime/src/safety.rs`](../../crates/ohm-runtime/src/safety.rs). They apply to
*every* write that goes through `Runtime::write_value` — the UI, a rule, or the fail-safe
path — because `SafetyPolicy::check_duty` is the single gate on the write path.

| Limit | Default | What it does |
|---|---|---|
| Fan duty floor (`min_duty_percent`) | **25 %** | Any requested fan duty below the floor is *raised* to it, and the write is reported with `clamped: true` and a note ("raised to the 25 % minimum fan floor"). |
| Pump duty floor (`pump_min_duty_percent`) | **60 %** | Never relaxed, and never below the fan floor (the policy raises it if a configuration would make it lower). A stopped pump is a dead CPU. |
| Emergency ceiling (`emergency_temp_c`) | **90 °C** | At or above this temperature, every controlled actuator is forced to `emergency_duty_percent`. |
| Emergency duty (`emergency_duty_percent`) | **100 %** | The value used above the ceiling. This overrides every rule and, by default, every manual request too (`emergency_override_enabled: true`). |
| Fail-safe duty (`fail_safe_duty_percent`) | **70 %** | Used when a rule loses its sensor, or when a write to a duty control fails — the runtime then immediately attempts this value as an `origin.kind = "safety"` write, so a rejected write cannot leave an output where the caller hoped it would be. Never below the device's floor. |
| Sensor staleness (`sensor_stale_after_s`) | **5 s** | After this, a rule falls back instead of extrapolating. |
| Ramp limiting (`max_write_delta_percent`) | **0 % (disabled)** | Deliberately off by default: some fans stall when ramped too slowly. If you enable it, a write is limited to that many percent per write and reported as clamped. |
| Relinquish on exit (`relinquish_on_exit`) | **true** | The runtime hands control back on shutdown — for LHM channels that is LHM's `SetDefault`. |
| Non-finite values | always refused | A NaN or infinite duty is blocked outright. |

Two consequences for the operator:

- **The floors are not adjustable during the session.** Do not edit `settings.json` to
  "test" a duty below 25 % on a fan or below 60 % on a pump. A value below the floor is
  not evidence about the hardware; it is evidence that the gate worked, and it is already
  covered by the test suite.
- **A `clamped: true` audit entry is a pass, not a bug.** It means the safety layer
  raised your value. Record the requested value *and* the applied one — the report has a
  column for exactly that.

## 3. How a write is performed

In this order, every time, no exceptions:

1. **`--dry-run` first.** Launch with
   `cargo run -p ohm-desktop -- --dry-run` and perform the intended change in the UI.
   Confirm the write is reported as `simulated`, the audit entry says
   `"simulated":true` / `"detail":"dry run: hardware was not touched"`, and **nothing on
   the machine moved**. If the fan moves during the dry run, stop the session: the
   dry-run switch is not reaching the write path, which is the one finding this kit exists
   to catch before real hardware is involved.
2. **One output at a time.** Never change two fan channels in the same step, and never a
   fan and a pump together. If two outputs must be compared, do them in separate steps with
   the machine back at its starting state in between.
3. **Start from the current value.** Read the output's present duty before you write, and
   make the first change small and in the direction of more airflow unless you have a
   reason to go the other way. Record the value you started from — the report needs it.
4. **Stay inside the device's declared range.** The capability carries its own `min`/`max`
   (0–100 % for LHM control channels and for NVML fan duty). Staying inside it is not
   optional, and the safety floors in §2 apply on top of it.
5. **One supervised real write, then stop and verify.** Restart without `--dry-run`, make
   the single change, then read the channel back independently
   (`GET /Sensor?action=Get&id=<SensorId>`, and the paired `Fan #N` tachometer) and record
   what the device reported — see [`checklist.md`](checklist.md) §12.3 for why the
   application's own `applied` field is not a read-back.
6. **Wait and watch.** At least 30 seconds of watching and listening after each write,
   with the machine's temperatures visible, before you consider the step finished. A fan
   that responds and then loses its tachometer is a real failure mode; so is a header
   whose firmware overrides you after a few seconds.

### What must never be done during a session

| Never | Why |
|---|---|
| Write to a pump channel as an experiment | The pump floor is 60 % for a reason; a stopped pump damages the CPU in seconds. Pumps are read, not experimented on. |
| Write to an output on a machine you cannot hear | Rule §1.2. A stalled or screaming fan is information you only have if you are there, and an unexpected full-speed fan can mean the firmware fought you. |
| Leave a write in place and walk away | Rule §1.1. Finish with §5 (hand control back) or hand the machine back in a known state. |
| Lower a duty to "see what happens" below the floor | §2: it is refused or clamped, so it tests nothing about the hardware. |
| Write while the machine is under a benchmark or a game | Temperature is already moving and the fan's response is not attributable to your write. Test on a quiet machine. |
| Write to a GPU fan while the driver is mid-update, or the driver is not loaded | NVML returns errors that say nothing about the hardware. Do it on a settled machine. |
| Disable the safety policy to get a value through | The pump floor and emergency ceiling stay active even when `SafetyPolicy::enabled` is false; do not go looking for a way around them. |
| Run a write step on someone's machine without §1.3 | That is the one rule that is about consent rather than hardware, and it is not negotiable. |

## 4. Stop conditions — stop immediately, no analysis first

If any of these happen, stop the current step, return the machine to a known state (§5),
and end the write part of the session. Record what happened, and do not retry the step
"to confirm" until the finding has been reported.

| # | Condition | First action |
|---|---|---|
| 1 | **Any smoke, smell, sparking, or unusual noise from a fan, pump or connector.** | **Cut power at the PSU switch or the wall.** Do not shut down gracefully. Then unplug and do not power the machine back on until it has been inspected. |
| 2 | **A fan reports 0 RPM while its duty is above the floor** (blocked, stalled, or disconnected). | Return control to the firmware immediately (§5). Do not raise the duty to "unstick" it. |
| 3 | **Temperature rising while duty is high** — the fans are commanded up and the component is still getting hotter. | Return control to the firmware (§5), then check the fans physically. If the temperature reaches the emergency ceiling, let the safety layer do its job at 100 % and consider cutting power. |
| 4 | **Unexpected noise**: a fan at full speed that you did not command, a grinding or ticking fan, or a pump that sounds like it is running dry or cavitating. | Return control to the firmware (§5). The usual cause is firmware overriding a header we wrote; the fan and the header must be checked physically before any further write. |
| 5 | **Any output does not respond to a write at all**, while the device claims the value was applied. | Stop; this is a finding (checklist §12.3), not something to work around by writing again. |
| 6 | **A write is reported as applied while `--dry-run` is in force**, or the hardware moves during the dry-run pass. | Stop the whole session immediately; report it before any further step. |
| 7 | **The operator or the machine's owner is no longer comfortable.** | Stop. No justification is required. |

When a stop condition fires, capture [`checklist.md` §13](checklist.md#13-stop-and-report)
verbatim and mark the affected rows of the report as failed, not verified.

## 5. Recovery: handing control back

Do these in order. Each step is verifiable; do not skip to the reboot.

### 5.1 Quit the application properly

Use the tray menu's **Quit**, not the window's X (with `close_to_tray` on, the X only
hides the window and the runtime keeps running — and keeps owning the channels).

On the way out the app stops the automation engine and calls `Runtime::shutdown()`. With
`relinquish_on_exit` enabled (the default), that reaches the LHM adapter's own `shutdown()`,
which calls `SetDefault` on **every** control channel it knows about — i.e.
`GET /Sensor?action=Set&id=<SensorId>&value=null` — and returns the SuperIO's own fan curve.
The LHM adapter logs a warning naming any channel it could not release; check the log:

```powershell
PS> Get-Content "$env:APPDATA\OpenHardwareOS\logs\openhardwareos.log.*" -Tail 40 |
      Select-String -Pattern 'released|channel|SetDefault|shutdown'
```

Confirm for each channel you wrote:

```powershell
PS> (Invoke-WebRequest "http://127.0.0.1:8085/Sensor?action=Get&id=<SensorId>" -UseBasicParsing).Content
```

The duty should follow the firmware again (typically tracking temperature), not sit at the
value you last wrote.

### 5.2 Use LibreHardwareMonitor's own control reset

If a channel is still pinned — for example because the app was killed rather than quit, or
its release failed — release it directly in LHM:

- in the LHM GUI, reset the control channel to its default (its control sliders have a
  reset-to-default action), or
- send the same request the app would have sent:
  `http://127.0.0.1:8085/Sensor?action=Set&id=<SensorId>&value=null`

For an NVIDIA GPU fan written through NVML, the equivalent is
`nvmlDeviceSetDefaultFanSpeed_v2` — which this application calls on its own shutdown path
for channels it owns; if the process was killed, restoring the driver's automatic control
is done from the vendor tool (NVIDIA App / `nvidia-smi`'s fan control) or by a reboot.

### 5.3 Reboot, as the last resort

A reboot makes the firmware re-assert control at POST for motherboard fan headers, and
reloads the GPU driver's own fan policy. Use it when §5.1 and §5.2 did not restore control.
Record it in the report as the recovery you used, with the reason.

### 5.4 Check afterwards, before you finish

- [ ] Every channel you wrote reports a duty consistent with firmware control, not with the
      last value you sent.
- [ ] Every fan you touched has a plausible RPM at idle, and none is at 0.
- [ ] Temperatures are where they were before the session started (compare with the "before"
      `collect.ps1` bundle and the report's §2 values).
- [ ] No unexpected noise, and nothing is running at full speed.
- [ ] `audit.jsonl` contains a `write` entry for every write you performed — applied,
      clamped **and** rejected — plus the `lifecycle` entries for the shutdown.
- [ ] The machine's owner has seen the machine in this state and agrees it is back to normal.

## 6. Agreement

To be completed before the first write, and kept with the report.

| | |
|---|---|
| Machine (make, model, serial or asset tag) | |
| Owner / responsible person | |
| Operator | |
| Date and time of agreement | |
| Confirmed: a person will be present and watching for every write | Yes / No |
| Confirmed: the fans are physically observable and audible from where the operator stands | Yes / No |
| Confirmed: the power cut-off (PSU switch / wall switch / plug) is known and reachable | Yes / No |
| Confirmed: the machine is not running anything whose loss matters (work, backups, other people's sessions) | Yes / No |
| Confirmed: nobody else depends on this machine being up right now | Yes / No |
| Confirmed: the stop conditions in §4 are understood and will be acted on without discussion | Yes / No |
| Confirmed: the owner knows a firmware-overridden fan header can behave unexpectedly, and that the operator can stop at any point | Yes / No |
| Owner signature | |
| Operator signature | |

**If any row above is "No", the write steps are not performed.** The read-only steps
([`checklist.md`](checklist.md) §1–§9, §11) are unaffected and still produce a useful
report with every item honestly labelled.
