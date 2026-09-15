#!/usr/bin/env bash
#
# Fixture tests for scripts/verify-linux-readings.sh.
#
#   ./scripts/tests/verify-linux-readings.test.sh
#
# The cross-check compares OpenHardwareOS readings with the operating system's own
# answer, so it can only be trusted if it is *known* to detect a disagreement. That
# cannot be shown on the machine the check runs on — a working build agrees with
# itself — so these cases feed it a synthetic sysfs/proc tree and a readings file
# written by hand: one that matches, one that does not, one that omits a reading
# without explaining why, and one for a machine that has no fan channels at all.
#
# This is not evidence about any real machine. The live run on a real kernel is the
# CI job and the user's own `./scripts/verify-linux-readings.sh`.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
SCRIPT="$ROOT_DIR/scripts/verify-linux-readings.sh"
[ -f "$SCRIPT" ] || { echo "error: cannot find $SCRIPT" >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ohm-crosscheck-tests-XXXXXX")"
KEEP="${KEEP_FIXTURES:-0}"
PASSED=0
FAILED=0
CASES=0

cleanup() {
  if [ "$KEEP" = "1" ]; then echo "kept: $WORK"; else rm -rf "$WORK"; fi
}
trap cleanup EXIT

ok()  { PASSED=$((PASSED + 1)); printf '    [PASS] %s\n' "$1"; }
bad() { FAILED=$((FAILED + 1)); printf '    [FAIL] %s\n' "$1"; }
check() { if [ "$2" = "1" ]; then ok "$1"; else bad "$1"; fi; }

# --- a synthetic machine -----------------------------------------------------
# 16 GiB of RAM, one SuperIO chip with a fan tachometer and a PWM, one thermal
# zone. Values are chosen so every comparison has a definite answer.
build_root() { # directory
  local root="$1"
  rm -rf "$root"
  mkdir -p "$root/proc" "$root/sys/class/hwmon/hwmon3" "$root/sys/class/thermal/thermal_zone0"
  {
    printf 'MemTotal:       16777216 kB\n'
    printf 'MemFree:         4194304 kB\n'
    printf 'MemAvailable:    8388608 kB\n'
  } > "$root/proc/meminfo"
  printf 'nct6798d\n' > "$root/sys/class/hwmon/hwmon3/name"
  printf '1245\n' > "$root/sys/class/hwmon/hwmon3/fan1_input"
  printf '153\n' > "$root/sys/class/hwmon/hwmon3/pwm1"
  printf '45000\n' > "$root/sys/class/thermal/thermal_zone0/temp"
}

# --- a readings file in the shape `ohm-cli status --json` emits ---------------
# memory.used is 16777216-8388608 = 8388608 kB = 8589934592 B
make_readings() { # file, mode, mount point, free bytes to claim
  python3 - "$1" "$2" "${3:-}" "${4:-0}" <<'PY'
import json, sys

path, mode = sys.argv[1], sys.argv[2]
mount = sys.argv[3]
claimed_free = float(sys.argv[4] or 0)
total = 16777216 * 1024
used = (16777216 - 8388608) * 1024

def device(device_id, name, kind, adapter, readings, metadata=None):
    return {
        "device": {
            "id": device_id, "name": name, "type": kind, "adapter": adapter,
            "transport": "internal", "model": None, "vendor": None,
            "capabilities": [], "metadata": metadata or {},
        },
        "enabled": True, "status": "online",
        "state": {"device": device_id, "timestamp_ms": 1, "online": True, "readings": readings},
        "adapter": adapter, "first_seen_ms": 1, "last_seen_ms": 2,
    }

def reading(capability, value=None, status="ok", reason=None, detail=None):
    out = {"capability": capability, "status": status}
    if value is not None:
        out["value"] = value
    if reason:
        out["reason"] = reason
    if detail:
        out["detail"] = detail
    return out

if mode == "silent":
    # No memory device at all: nothing reported, and no reason given. The one
    # outcome this project promises never to produce.
    devices = [device("cpu.system.0", "Fixture CPU", "cpu", "system", [
        reading("temperature.core", 45.0),
    ])]
elif mode == "nofans":
    devices = [
        device("memory.system.0", "Memory", "memory", "system", [
            reading("memory.total", total), reading("memory.used", used),
        ]),
        device("cpu.system.0", "Fixture CPU", "cpu", "system", [
            reading("temperature.core", 45.0),
        ]),
    ]
else:
    wrong = mode == "differ"
    devices = [
        device("memory.system.0", "Memory", "memory", "system", [
            reading("memory.total", total + (1024 if wrong else 0)),
            reading("memory.used", used),
        ]),
        device("fan.system.nct6798d_fan1", "Chassis fan", "fan", "system", [
            reading("fan.rpm", 1200.0 if wrong else 1245.0),
            reading("fan.pwm", 153.0),
        ]),
        device("cpu.system.0", "Fixture CPU", "cpu", "system", [
            reading("temperature.core", 45.0 if not wrong else 90.0),
        ]),
        device("storage.system.0", "Root filesystem", "storage", "system", [
            # A float, deliberately: JSON has one number type and this is the
            # shape that used to break the comparison.
            reading("storage.free", float(claimed_free)),
        ], metadata={"mount_point": mount}),
    ]

json.dump({"generated_at_ms": 1, "started_at_ms": 0, "devices": devices,
           "adapters": [], "settings": {}, "stats": {},
           "has_controllable_hardware": False}, open(path, "w", encoding="utf-8"))
PY
}

run_check() { # root, readings -> stdout in $WORK/last.log, exit code in $?
  "$SCRIPT" --root "$1" --readings "$2" > "$WORK/last.log" 2>&1
}

echo "================================================================================"
echo " OpenHardwareOS — Linux readings cross-check (FIXTURES ONLY)"
echo "================================================================================"
echo " script : scripts/verify-linux-readings.sh"
echo " work   : $WORK"
echo
echo " The machine is synthetic. These cases prove the check detects a disagreement;"
echo " the live run against a real kernel is the CI job and the user's own machine."

# ---------------------------------------------------------------- case 1
echo
echo "--- a machine whose readings match the kernel's own files"
CASES=$((CASES + 1))
root="$WORK/root-agree"; build_root "$root"
# `df -k` column 4 is "Available"; the storage reading claims exactly that, so the
# comparison has a definite answer on any host.
platform_free_kb="$(df -k "$root" | tail -1 | awk '{ print $4 }')"
platform_free=$((platform_free_kb * 1024))
readings="$WORK/agree.json"; make_readings "$readings" agree "$root" "$platform_free"
if run_check "$root" "$readings"; then
  ok "the check exits 0"
else
  bad "the check exits 0 ($(tail -1 "$WORK/last.log"))"
fi
grep -q "AGREE.*memory.total.*/proc/meminfo MemTotal" "$WORK/last.log" \
  && ok "  memory.total agrees and says what it compared with" \
  || bad "  memory.total agrees and says what it compared with"
grep -q "AGREE.*fan.system.nct6798d_fan1/fan.rpm.*1245" "$WORK/last.log" \
  && ok "  the fan tachometer agrees with fan1_input" \
  || bad "  the fan tachometer agrees with fan1_input"
grep -q "AGREE.*fan.system.nct6798d_fan1/fan.pwm" "$WORK/last.log" \
  && ok "  the PWM reading agrees with pwm1" \
  || bad "  the PWM reading agrees with pwm1"
grep -q "AGREE.*temperature.core" "$WORK/last.log" \
  && ok "  the temperature is inside the platform's own range" \
  || bad "  the temperature is inside the platform's own range"
grep -q "AGREE.*storage.system.0/storage.free.*df" "$WORK/last.log" \
  && ok "  the storage reading agrees with df for the same directory" \
  || bad "  the storage reading agrees with df for the same directory"
grep -q "differed: 0" "$WORK/last.log" && ok "  and nothing differed" || bad "  and nothing differed"

# ---------------------------------------------------------------- case 2
echo
echo "--- a reading that disagrees with the kernel is reported, not smoothed over"
CASES=$((CASES + 1))
readings="$WORK/differ.json"
make_readings "$readings" differ "$root" "$((platform_free + 10737418240))"
if run_check "$root" "$readings"; then
  bad "the check fails when a reading disagrees"
else
  ok "the check fails when a reading disagrees"
fi
grep -q "DIFFER.*memory.total" "$WORK/last.log" \
  && ok "  memory.total is named" || bad "  memory.total is named"
grep -q "DIFFER.*fan.rpm.*kernel says 1245" "$WORK/last.log" \
  && ok "  the fan reading names both numbers" || bad "  the fan reading names both numbers"
grep -q "DIFFER.*temperature.core" "$WORK/last.log" \
  && ok "  a temperature outside the platform's range is caught" \
  || bad "  a temperature outside the platform's range is caught"
grep -q "DIFFER.*storage.free.*df says" "$WORK/last.log" \
  && ok "  a storage figure df disagrees with is caught too" \
  || bad "  a storage figure df disagrees with is caught too"
check "the summary counts the differences" \
  "$(grep -q 'differed: 4' "$WORK/last.log" && echo 1 || echo 0)"

# ---------------------------------------------------------------- case 3
echo
echo "--- a reading we do not report, and do not explain, is a failure"
CASES=$((CASES + 1))
readings="$WORK/silent.json"; make_readings "$readings" silent "$root" 0
if run_check "$root" "$readings"; then
  bad "the check fails when a reading is missing with no reason"
else
  ok "the check fails when a reading is missing with no reason"
fi
grep -q "DIFFER.*memory.total.*no reason" "$WORK/last.log" \
  && ok "  and says what was wrong with it" || bad "  and says what was wrong with it"

# ---------------------------------------------------------------- case 4
echo
echo "--- a machine with no fan channels is not a failure"
CASES=$((CASES + 1))
readings="$WORK/nofans.json"; make_readings "$readings" nofans "$root" "$platform_free"
if run_check "$root" "$readings"; then
  ok "the check exits 0"
else
  bad "the check exits 0 ($(tail -1 "$WORK/last.log"))"
fi
grep -q "NO-SOURCE.*hwmon channels" "$WORK/last.log" \
  && ok "  and says there are no fan channels to compare" \
  || bad "  and says there are no fan channels to compare"
grep -q "AGREE.*memory.total" "$WORK/last.log" \
  && ok "  while still checking what it can" || bad "  while still checking what it can"

# ---------------------------------------------------------------- case 5
echo
echo "--- the synthetic tree is named in the output, so nobody reads it as live"
CASES=$((CASES + 1))
readings="$WORK/agree.json"
run_check "$root" "$readings" || true
grep -q "comparing against the tree at $root" "$WORK/last.log" \
  && ok "  the output says which tree it read" || bad "  the output says which tree it read"

# ---------------------------------------------------------------- case 6
echo
echo "--- a real machine with no /proc is refused rather than reported as empty"
CASES=$((CASES + 1))
mkdir -p "$WORK/root-empty"
if "$SCRIPT" --root "$WORK/root-empty" --readings "$readings" > "$WORK/empty.log" 2>&1; then
  ok "the check still exits 0 (nothing comparable, nothing wrong)"
else
  bad "the check still exits 0 ($(tail -1 "$WORK/empty.log"))"
fi
grep -q "no .*proc/meminfo" "$WORK/empty.log" \
  && ok "  and says the platform has nothing to compare" \
  || bad "  and says the platform has nothing to compare"

echo
echo "================================================================================"
echo "  cases: $CASES   passed: $PASSED   failed: $FAILED"
if [ "$FAILED" -eq 0 ]; then
  echo "RESULT: PASS"
  exit 0
fi
echo "RESULT: FAIL"
exit 1
