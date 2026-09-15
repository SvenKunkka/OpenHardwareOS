#!/usr/bin/env bash
#
# Compare what OpenHardwareOS reports with what the operating system itself says,
# on the same machine, right now.
#
#   ./scripts/verify-linux-readings.sh                 # run ohm-cli and compare
#   ./scripts/verify-linux-readings.sh --readings f.json
#   ./scripts/verify-linux-readings.sh --root /tmp/fake-sysroot
#
# Why this exists: "the readings are real" is a claim about the *machine*, and the
# only way to check a claim like that is against a source that did not come from
# this project. Sysfs, /proc and df are those sources. For every reading the CLI
# reports, this prints one line saying what it compared against and what happened:
#
#   AGREE        our value equals the platform's own value
#   DIFFER       it does not, beyond the stated tolerance — the exit code is 1
#   NO-SOURCE    the platform has nothing to compare with (and why)
#   NOT-CHECKED  there is no independent source for this reading, by nature
#
# A machine with no fan tachometers is a normal outcome, not a failure: the point
# is that the answer is stated rather than assumed. Exit code is 0 when nothing
# differs, 1 when something does, 2 when the check could not run at all.
#
# `--root` exists so the comparison can be tested on a synthetic tree — including
# trees that disagree. A check that only ever runs on the machine it describes
# cannot be shown to detect anything.
set -uo pipefail

ROOT="/"
READINGS=""
CLI="ohm-cli"
MOCK=0
WORK="$(mktemp -d "${TMPDIR:-/tmp}/ohm-crosscheck-XXXXXX")"
trap 'rm -rf "$WORK"' EXIT

while [ $# -gt 0 ]; do
  case "$1" in
    --root) ROOT="${2:-}"; shift 2 ;;
    --readings) READINGS="${2:-}"; shift 2 ;;
    --cli) CLI="${2:-}"; shift 2 ;;
    --mock) MOCK=1; shift ;;
    -h|--help) sed -n '2,30p' "$0"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

[ -d "$ROOT" ] || { echo "no such root: $ROOT" >&2; exit 2; }
# `--root /` is the default; `${ROOT}/proc` would then read `//proc`, which works
# but reads like a mistake in the very output a user is asked to trust.
ROOT="${ROOT%/}"
# Empty prefix means the live filesystem, so `${PREFIX}/proc/meminfo` is
# `/proc/meminfo` rather than `//proc/meminfo`.
PREFIX="$ROOT"
command -v python3 >/dev/null 2>&1 || {
  echo "python3 is required to read the readings JSON" >&2; exit 2; }

AGREE=0
DIFFER=0
NO_SOURCE=0
NOT_CHECKED=0

report() { # verdict, subject, detail
  printf '%-11s %-34s %s\n' "$1" "$2" "$3"
  case "$1" in
    AGREE) AGREE=$((AGREE + 1)) ;;
    DIFFER) DIFFER=$((DIFFER + 1)) ;;
    NO-SOURCE) NO_SOURCE=$((NO_SOURCE + 1)) ;;
    NOT-CHECKED) NOT_CHECKED=$((NOT_CHECKED + 1)) ;;
  esac
}

# --- the readings -----------------------------------------------------------
if [ -z "$READINGS" ]; then
  READINGS="$WORK/status.json"
  if [ "$MOCK" = "1" ]; then
    "$CLI" status --json --mock > "$READINGS" 2> "$WORK/cli.err"
  else
    "$CLI" status --json > "$READINGS" 2> "$WORK/cli.err"
  fi
  if [ ! -s "$READINGS" ]; then
    echo "could not read readings from '$CLI status --json':" >&2
    sed 's/^/  /' "$WORK/cli.err" >&2
    exit 2
  fi
fi
[ -s "$READINGS" ] || { echo "empty readings file: $READINGS" >&2; exit 2; }

if [ -n "$PREFIX" ]; then
  echo "comparing against the tree at $PREFIX (not the live system)"
fi
echo
printf '%-11s %-34s %s\n' "VERDICT" "READING" "COMPARED WITH"
printf '%-11s %-34s %s\n' "-------" "-------" "-------------"

# NUL-separated records keep values containing spaces (CPU model names, reasons)
# from being split into the wrong fields.
python3 - "$READINGS" <<'PY' > "$WORK/rows"
import json, sys

def emit(*fields):
    sys.stdout.write("\0".join("" if f is None else str(f) for f in fields) + "\n")

data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    model = device["device"]
    state = device.get("state") or {}
    for reading in state.get("readings", []):
        value = reading.get("value")
        emit(
            model["id"],
            model.get("adapter", ""),
            model.get("name", ""),
            model.get("transport", ""),
            reading.get("capability", ""),
            reading.get("status", ""),
            "" if value is None else value,
            reading.get("reason", ""),
            reading.get("detail", ""),
        )
PY

to_int() { # a JSON number as an integer, or empty when it is not one
  # JSON has one number type, so a byte count can arrive as `82222657536.0`. Bash
  # arithmetic on that string is not a comparison — it is a syntax error waiting to
  # make the check say "differs" about a reading that is fine.
  awk -v v="$1" 'BEGIN {
    if (v ~ /^-?[0-9]+(\.[0-9]+)?([eE][-+]?[0-9]+)?$/) printf "%.0f\n", v
  }'
}

value_of() { # device id, capability -> value or empty
  python3 - "$READINGS" "$1" "$2" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    if device["device"]["id"] != sys.argv[2]:
        continue
    for reading in (device.get("state") or {}).get("readings", []):
        if reading.get("capability") == sys.argv[3] and reading.get("status") == "ok":
            print(reading.get("value"))
            raise SystemExit(0)
print("")
PY
}

reason_of() { # device id, capability -> reason/detail, for an absent reading
  python3 - "$READINGS" "$1" "$2" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    if device["device"]["id"] != sys.argv[2]:
        continue
    for reading in (device.get("state") or {}).get("readings", []):
        if reading.get("capability") == sys.argv[3] and reading.get("status") != "ok":
            print((reading.get("detail") or reading.get("reason") or "no reason given"))
            raise SystemExit(0)
print("")
PY
}

name_of() { # device id -> the device's own name
  python3 - "$READINGS" "$1" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    if device["device"]["id"] == sys.argv[2]:
        print(device["device"].get("name", ""))
        raise SystemExit(0)
print("")
PY
}

# --- memory: /proc/meminfo is the platform's own answer ---------------------
meminfo() { # key -> bytes
  local key="$1"
  local line
  line="$(grep -m1 "^$key:" "$PREFIX/proc/meminfo" 2>/dev/null || true)"
  [ -n "$line" ] || return 1
  awk '{ print $2 * 1024 }' <<<"$line"
}

our_total="$(to_int "$(value_of 'memory.system.0' 'memory.total')")"
platform_total="$(meminfo MemTotal || true)"
if [ -z "$platform_total" ]; then
  report NO-SOURCE "memory.total" "no $ROOT/proc/meminfo on this machine"
elif [ -z "$our_total" ]; then
  if [ -n "$(reason_of 'memory.system.0' 'memory.total')" ]; then
    report NO-SOURCE "memory.total" "we report nothing: $(reason_of 'memory.system.0' 'memory.total')"
  else
    report DIFFER "memory.total" "we report nothing and give no reason for it"
  fi
elif [ "$our_total" = "$platform_total" ]; then
  report AGREE "memory.total" "ours $our_total B = /proc/meminfo MemTotal"
else
  report DIFFER "memory.total" "ours $our_total B, /proc/meminfo MemTotal $platform_total B"
fi

# "Used" has no single platform definition, so this compares against the one the
# runtime documents (total - available) with a tolerance for the machine moving
# between the two reads.
our_used="$(to_int "$(value_of 'memory.system.0' 'memory.used')")"
platform_total="$(meminfo MemTotal || true)"
platform_avail="$(meminfo MemAvailable || true)"
if [ -z "$platform_total" ]; then
  report NO-SOURCE "memory.used" "no $ROOT/proc/meminfo on this machine"
elif [ -n "$our_used" ] && [ -n "$platform_avail" ]; then
  platform_used=$((platform_total - platform_avail))
  difference=$((our_used - platform_used))
  [ "$difference" -lt 0 ] && difference=$((-difference))
  tolerance=$((platform_total / 100 + 268435456))  # 1 % of RAM or 256 MB
  if [ "$difference" -le "$tolerance" ]; then
    report AGREE "memory.used" "ours $our_used B vs MemTotal-MemAvailable $platform_used B (Δ $difference B)"
  else
    report DIFFER "memory.used" "ours $our_used B, MemTotal-MemAvailable $platform_used B (Δ $difference B)"
  fi
elif [ -n "$our_used" ]; then
  report NO-SOURCE "memory.used" "$ROOT/proc/meminfo has no MemAvailable"
else
  report NO-SOURCE "memory.used" "we report nothing: $(reason_of 'memory.system.0' 'memory.used')"
fi

# --- hwmon channels: the exact file the kernel wrote ------------------------
# Device ids are `fan.system.<chip>_fan<N>`; the chip name and N are the identity,
# so the comparison is against the same file the reading came from — the point is
# that our parsing and labelling of it is checkable by hand.
hwmon_file() { # chip, file -> path or empty
  local chip="$1" file="$2" dir
  for dir in "$PREFIX"/sys/class/hwmon/hwmon*; do
    [ -d "$dir" ] || continue
    [ "$(cat "$dir/name" 2>/dev/null)" = "$chip" ] || continue
    [ -f "$dir/$file" ] || continue
    printf '%s\n' "$dir/$file"
    return 0
  done
  return 1
}

hwmon_checked=0
while IFS= read -r row; do
  [ -n "$row" ] || continue
  device_id="${row%%$'\t'*}"
  channel="${device_id##*_fan}"
  chip="${device_id#fan.system.}"
  chip="${chip%_fan*}"
  case "$device_id" in fan.system.*_fan*) ;; *) continue ;; esac
  hwmon_checked=$((hwmon_checked + 1))

  rpm="$(to_int "$(value_of "$device_id" 'fan.rpm')")"
  if [ -n "$rpm" ]; then
    file="$(hwmon_file "$chip" "fan${channel}_input" || true)"
    if [ -z "$file" ]; then
      report NO-SOURCE "$device_id/fan.rpm" "no ${chip} fan${channel}_input in $ROOT/sys/class/hwmon"
    elif [ "$(cat "$file" 2>/dev/null | tr -d '[:space:]')" = "$(printf '%.0f' "$rpm")" ]; then
      report AGREE "$device_id/fan.rpm" "ours ${rpm} rpm = $(basename "$(dirname "$file")")/fan${channel}_input"
    else
      report DIFFER "$device_id/fan.rpm" "ours ${rpm} rpm, kernel says $(cat "$file")"
    fi
  else
    report NO-SOURCE "$device_id/fan.rpm" "we report nothing: $(reason_of "$device_id" 'fan.rpm')"
  fi

  pwm="$(to_int "$(value_of "$device_id" 'fan.pwm')")"
  if [ -n "$pwm" ]; then
    file="$(hwmon_file "$chip" "pwm${channel}" || true)"
    if [ -z "$file" ]; then
      report NO-SOURCE "$device_id/fan.pwm" "no ${chip} pwm${channel} in $ROOT/sys/class/hwmon"
    elif [ "$(cat "$file" 2>/dev/null | tr -d '[:space:]')" = "$(printf '%.0f' "$pwm")" ]; then
      report AGREE "$device_id/fan.pwm" "ours ${pwm} = $(basename "$(dirname "$file")")/pwm${channel}"
    else
      report DIFFER "$device_id/fan.pwm" "ours ${pwm}, kernel says $(cat "$file")"
    fi
  fi
done < <(python3 - "$READINGS" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    print(device["device"]["id"])
PY
)
[ "$hwmon_checked" -gt 0 ] || report NO-SOURCE "hwmon channels" \
  "this machine exposes no fan.system.<chip>_fan<N> device (no driver or no fans)"

# --- temperatures: is our value one the machine actually reports? -----------
# The CPU zone comes from the platform's sensor list, not from one hwmon file, so
# the honest comparison is a range: a reading outside every temperature the
# platform reports by more than 2 °C is wrong, whatever it was derived from.
platform_temps="$WORK/platform-temps"
: > "$platform_temps"
for file in "$PREFIX"/sys/class/hwmon/hwmon*/temp*_input "$PREFIX"/sys/class/thermal/thermal_zone*/temp; do
  [ -f "$file" ] || continue
  raw="$(cat "$file" 2>/dev/null | tr -d '[:space:]')"
  case "$raw" in ''|*[!0-9-]*) continue ;; esac
  # hwmon reports milli-degrees; thermal zones do too.
  awk -v t="$raw" 'BEGIN { printf "%.1f\n", t / 1000 }' >> "$platform_temps"
done

temperature_reported=0
while IFS= read -r row; do
  [ -n "$row" ] || continue
  device_id="${row%%$'\t'*}"
  capability="${row#*$'\t'}"
  case "$capability" in temperature.*) ;; *) continue ;; esac
  value="$(value_of "$device_id" "$capability")"
  [ -n "$value" ] || continue
  case "$value" in
    ''|*[!0-9.eE+-]*) report NOT-CHECKED "$device_id/$capability" \
      "not a number ($value), so there is nothing to compare" ; continue ;;
  esac
  temperature_reported=$((temperature_reported + 1))
  if [ ! -s "$platform_temps" ]; then
    report NO-SOURCE "$device_id/$capability" "the platform reports no temperature sensor here"
    continue
  fi
  verdict="$(python3 - "$value" "$platform_temps" <<'PY'
import sys
value = float(sys.argv[1])
temps = [float(line) for line in open(sys.argv[2]) if line.strip()]
if not temps:
    print("none")
elif min(temps) - 2.0 <= value <= max(temps) + 2.0:
    print(f"inside {min(temps):.1f}..{max(temps):.1f}")
else:
    print(f"outside {min(temps):.1f}..{max(temps):.1f}")
PY
)"
  case "$verdict" in
    inside*) report AGREE "$device_id/$capability" "ours ${value} °C, platform reports $verdict °C" ;;
    none) report NO-SOURCE "$device_id/$capability" "no readable platform temperature" ;;
    *) report DIFFER "$device_id/$capability" "ours ${value} °C is $verdict °C reported by the platform" ;;
  esac
done < <(python3 - "$READINGS" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    for reading in (device.get("state") or {}).get("readings", []):
        if reading.get("capability", "").startswith("temperature."):
            print(f"{device['device']['id']}\t{reading['capability']}")
PY
)
[ "$temperature_reported" -gt 0 ] || report NO-SOURCE "temperatures" "we report no temperature at all"

# --- storage: df for the same mount ----------------------------------------
storage_checked=0
while IFS= read -r row; do
  [ -n "$row" ] || continue
  device_id="${row%%$'\t'*}"
  mount="$(python3 - "$READINGS" "$device_id" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    if device["device"]["id"] == sys.argv[2]:
        print((device["device"].get("metadata") or {}).get("mount_point", ""))
        raise SystemExit(0)
print("")
PY
)"
  free="$(to_int "$(value_of "$device_id" 'storage.free')")"
  [ -n "$free" ] || continue
  storage_checked=$((storage_checked + 1))
  if [ -z "$mount" ]; then
    report NOT-CHECKED "$device_id/storage.free" "the device carries no mount point to compare with"
    continue
  fi
  if [ ! -d "$mount" ]; then
    report NO-SOURCE "$device_id/storage.free" "$mount is not present on this machine"
    continue
  fi
  # Column 4 of `df -k` is "Available" in 1K blocks on both GNU and BSD; the
  # GNU-only `--output=avail` would make this check untestable off Linux.
  platform_free="$(df -k "$mount" 2>/dev/null | tail -1 | awk '{ print $4 }' | tr -d '[:space:]')"
  if [ -z "$platform_free" ]; then
    report NO-SOURCE "$device_id/storage.free" "df could not report $mount"
    continue
  fi
  platform_bytes=$((platform_free * 1024))
  difference=$((free - platform_bytes))
  [ "$difference" -lt 0 ] && difference=$((-difference))
  if [ "$difference" -le $((platform_bytes / 100 + 104857600)) ]; then
    report AGREE "$device_id/storage.free" "ours $free B vs df $platform_bytes B (Δ $difference B)"
  else
    report DIFFER "$device_id/storage.free" "ours $free B, df says $platform_bytes B (Δ $difference B)"
  fi
done < <(python3 - "$READINGS" <<'PY'
import json, sys
data = json.load(open(sys.argv[1], encoding="utf-8"))
for device in data.get("devices", []):
    if device["device"]["type"] == "storage":
        print(device["device"]["id"])
PY
)
[ "$storage_checked" -gt 0 ] || report NO-SOURCE "storage.free" "we report no storage device"

# --- readings with no independent source, said out loud ---------------------
report NOT-CHECKED "cpu.load" \
  "no independent source: /proc/stat is a delta over time and this check takes one sample"
report NOT-CHECKED "cpu.frequency" "advisory: the governor changes it between any two reads"

echo
echo "agreed: $AGREE   differed: $DIFFER   no platform source: $NO_SOURCE   not comparable: $NOT_CHECKED"
if [ "$DIFFER" -gt 0 ]; then
  echo "RESULT: differences found — this build reports something the platform does not"
  exit 1
fi
echo "RESULT: every comparable reading agrees with the platform's own source"
exit 0
