# Example assets

Ready-made, copy-pasteable artefacts for OpenHardwareOS. Everything here is
validated against the real code in this repository — nothing is illustrative
pseudo-config.

```
examples/
└── rules/
    ├── gpu-cooling.yaml          GPU core temperature  -> chassis fan (the documented example)
    ├── cpu-cooling.yaml          CPU package temperature -> a second chassis fan
    └── system-cooling-max.yaml   MAX(CPU, GPU) -> the fans, with a safe fallback
```

## The rule files

Each file is one automation rule: `source -> curve -> target`, plus the safety
knobs that keep it predictable. The engine only ever reads these four things to
decide what to do, so a rule is readable, diffable and shareable without running
the app.

| Field | Meaning |
|---|---|
| `name` | Display name. Also the source of the rule id when `id` is absent (`GPU Cooling` → `gpu-cooling`). |
| `id` | Optional stable id. Used as the file name in the rules directory. |
| `enabled` | `false` keeps the rule loaded but stops it being evaluated. Default `true`. |
| `description` | Optional free text shown in the UI. |
| `source` | Either one sensor (`device` + `capability`) or several reduced by `aggregate: max \| min \| avg` with a `sensors:` list. |
| `target` | The actuator to write: `device` + `capability` (e.g. `fan.speed_percent`). |
| `curve` | A list of `[input, output]` pairs. Inputs must strictly increase; between points the output is interpolated linearly. At least two points are required. |
| `hysteresis` | °C the source must drop below the last anchor before the fan is allowed to slow down. Default `2.0`. |
| `deadband` | Smallest output change worth writing, in %. Default `0.5`. |
| `update_interval_ms` | Re-evaluation period. Minimum `100`. Default `1000`. |
| `min_output` / `max_output` | Optional clamps applied after the curve (0–100). |
| `fallback` | `on_sensor_missing`, `on_write_failure` (one of `hold`, `safe_default`, `fixed` with `percent`, `release`) and `sensor_timeout_s`. |

The three examples here also carry a comment header explaining the shape of their
curve. Comments survive editing because rules are plain YAML.

## Device and capability ids: what they mean

An id is `<kind>.<namespace>.<instance>`. `DeviceId::compose` in
`crates/ohm-core/src/ids.rs` numbers devices that have nothing better to be named
after; an adapter that can name one after the hardware it came from does so:

- `gpu.nvidia.0` — GPU 0 as seen by the NVML provider
- `gpu.lhm.gpu_0` — the same card as seen by LibreHardwareMonitor
- `fan.lhm.lpc_nct6687d_0_1` — chassis fan header 1 through LibreHardwareMonitor,
  named after LHM's own sensor path and the channel number rather than a position,
  so reordering the tree cannot move a rule to another header
- `cpu.mock.0`, `fan.mock.0`, `fan.mock.1` — the **simulated** machine
  (`adapters/mock`), which is why the examples use them: they resolve on any
  machine, including CI, with no hardware at all

Capability ids are namespaced strings from `ohm_core::ids::capability`:
`temperature.core`, `temperature.hotspot`, `temperature.system`, `fan.rpm`,
`fan.speed_percent`, `fan.pwm`, `pump.speed_percent`, `load.cpu`, `power.gpu`,
`clock.mhz`, and so on.

Ids may only contain lowercase ASCII letters, digits and `.`, `_`, `-`, `:`, and
are at most 96 characters — a device called `GPU 0` is rejected, `gpu.0` is not.

**The ids in these files are the simulator's. Replace them with the ids of your
own machine**, which are the authoritative ones:

- the **Devices** page in the app (device id and capability ids are shown per
  device), or
- `ohm-cli doctor` / `ohm-cli status` on the command line.

A rule that names an id you do not have is not an error: the rule loads, and the
engine reports the missing device in its status instead of pretending to cool
something.

## Installing one of these rules

1. Find your configuration directory:

   ```console
   $ ohm-cli paths
   ```

   The `rules:` line is the directory you want. On Windows that is normally
   `%APPDATA%\OpenHardwareOS\rules`; on macOS
   `~/Library/Application Support/OpenHardwareOS/rules`; on Linux
   `~/.config/OpenHardwareOS/rules`. `OHM_CONFIG_DIR` overrides the whole root
   (that is what the test-suite and a portable install use).

2. Check the file before installing it. This parses the YAML, validates the rule
   structurally, and resolves every id against the hardware that is actually
   attached — without running it:

   ```console
   $ ohm-cli rules check examples/rules/gpu-cooling.yaml
     rule: GPU Cooling (gpu-cooling)
     valid against the attached hardware
   ```

   On a machine with no controllable fan, or with LibreHardwareMonitor not
   running, the ids in these examples will not resolve and you get a *hardware*
   error (`device 'gpu.mock.0' was not found`) rather than a YAML error. That is
   the expected result on a real machine: put your own ids in first, or enable
   simulated hardware (`experimental_features` plus
   `adapter_settings.mock.enabled = true` in `settings.json`) to validate the
   file as-is.

3. Copy it in (the file name does not have to match the id, but keeping them the
   same avoids confusion — `rules/gpu-cooling.yaml` for `id: gpu-cooling`):

   ```console
   $ cp examples/rules/gpu-cooling.yaml "$(ohm-cli paths | awk '/^rules:/{print $2}')/"
   $ ohm-cli rules list
   ```

4. Or, without touching the filesystem, `ohm-cli rules suggest` writes starter
   rules for the hardware actually attached (add `--mock` to try it on the
   simulator), and the Automation screen creates and edits rules in the app.

Nothing needs a restart: rule files are re-read from the rules directory, and a
file that fails to parse is reported and skipped rather than stopping the app.

## Safety notes before you enable a curve

- **`fallback: safe_default` is the safe default** and it is what the examples
  use: if the temperature source disappears, the fan is driven to the runtime's
  fail-safe duty (70 % by default) instead of holding a low value.
- **One rule per target.** Two rules writing the same fan will fight each other;
  the runtime serialises writes, so the result is oscillation, not a merge.
- **Fan control is best-effort.** It depends on the motherboard exposing a
  controllable channel (through LibreHardwareMonitor), and firmware can take the
  channel back. `release` (LHM `SetDefault`) hands the fan back to the firmware
  and is the honest way to stop controlling it.
- **Uninstall / "restore defaults" must give the firmware the fans back** rather
  than leaving them at the last written duty.
