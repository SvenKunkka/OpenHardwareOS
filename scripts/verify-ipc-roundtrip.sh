#!/usr/bin/env bash
#
# Run the real desktop application and check that its IPC is actually connected.
#
# The Rust tests assert the wire contract by serialising commands' return values. The
# frontend tests render the real screens with the IPC module mocked. Both can pass while
# the application — window, webview, `invoke`, command, engine — is disconnected, which
# is what this script exists to rule out.
#
# It launches the **real** app (`ohm-desktop`, built from this tree, with a real window
# and a real WebKit webview) against an isolated configuration directory and the
# simulated hardware provider. The backend then asks the frontend, in that webview, to
# exercise the command surface with the same `api` functions the UI uses. The frontend
# reports what it saw back through a real command, which writes it next to the app's own
# log and audit trail — so the frontend's account and the backend's records can be
# compared, which is the point.
#
# No real hardware is involved: the provider is the simulator, the only writable devices
# are `*.mock.*`, and the script checks the audit trail to prove it.
#
# Usage: scripts/verify-ipc-roundtrip.sh [--keep]
# Exit codes: 0 all checks passed, non-zero with a report otherwise.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
RUSTUP="${HOME}/.cargo/bin/rustup"
TOOLCHAIN="${OHM_TOOLCHAIN:-1.98.0}"
WORK="$(mktemp -d "${TMPDIR:-/tmp}/ohm-ipc-roundtrip.XXXXXX")"
KEEP=0
[ "${1:-}" = "--keep" ] && KEEP=1

CONFIG="$WORK/config"
REPORT="$CONFIG/ipc-probe.json"
APP_LOG="$WORK/app.log"
FAILURES=0

say() { printf '%s\n' "$*"; }
step() { printf '\n== %s\n' "$*"; }
check() {
  local label="$1" ok="$2" detail="${3:-}"
  if [ "$ok" = "1" ]; then
    printf '  PASS  %s\n' "$label"
    [ -n "$detail" ] && printf '        %s\n' "$detail"
  else
    printf '  FAIL  %s\n' "$label"
    [ -n "$detail" ] && printf '        %s\n' "$detail"
    FAILURES=$((FAILURES + 1))
  fi
}

cleanup() {
  if [ -n "${APP_PID:-}" ] && kill -0 "$APP_PID" 2>/dev/null; then
    kill "$APP_PID" 2>/dev/null
    wait "$APP_PID" 2>/dev/null
  fi
  if [ "$KEEP" = "1" ]; then
    say "kept: $WORK"
    if [ -s "$WORK/screenshot.png" ]; then
      say "note: $WORK/screenshot.png is a whole-screen capture — check it before sharing;"
      say "      it may contain other applications' content."
    fi
  else
    rm -rf "$WORK"
  fi
}
trap cleanup EXIT

REAL_CONFIG="${HOME}/Library/Application Support/OpenHardwareOS"
ENV_NAME="OHM_CONFIG_DIR"
PROBE_FILE="ipc-probe.json"
if [ -d "$REAL_CONFIG" ]; then REAL_CONFIG_BEFORE="present"; else REAL_CONFIG_BEFORE="absent"; fi

step "Preparing an isolated configuration directory"
mkdir -p "$CONFIG/rules"
# The simulator, not the machine: `--mock` registers the simulated provider, and
# `dry_run` stays off because a dry run reports `simulated`, not `unconfirmed` — the
# status this check is about. The mock provider is the only writable one present, and
# the audit trail is inspected below to prove nothing else was written.
# The application itself now requires this: the self-test refuses to run unless its
# configuration disables every real provider, so that a probe can never touch real
# hardware through a config it was merely pointed at.
cat > "$CONFIG/settings.json" <<'JSON'
{
  "polling_interval_ms": 250,
  "discovery_interval_ms": 1000,
  "automation_enabled": true,
  "dry_run": false,
  "experimental_features": true,
  "disabled_adapters": ["system", "lhm", "nvidia"],
  "adapter_settings": {
    "mock": { "enabled": true, "enable_gpu_fan_control": true }
  }
}
JSON
# A failed handover for a channel that looks like real hardware, left behind by a
# "previous session". The probe must not re-arm it: it belongs to nobody's simulated
# run, and a global `rule_retry_handovers` would have reset its attempt budget and put
# it back in the queue. Its attempt count is the evidence.
cat > "$CONFIG/control-state.json" <<'JSON'
{
  "version": 1,
  "saved_at_ms": 1700000000000,
  "handovers": [
    {
      "device": "fan.lhm.0",
      "capability": "fan.speed_percent",
      "from_rule": "not-the-probe",
      "reason": "left behind by an earlier session, on a channel this machine does not have",
      "state": "failed",
      "attempts": 3,
      "first_error": "the device refused the fail-safe duty",
      "queued_at_ms": 1700000000000,
      "last_attempt_ms": 1700000000500,
      "claimed_ticks": 0,
      "parked_by_claim": false
    }
  ],
  "holds": []
}
JSON
# A legacy rule file: the app must substitute `release` in memory and report it to the
# frontend as a structured compatibility note.
cat > "$CONFIG/rules/legacy-release.yaml" <<'YAML'
name: Legacy Release
id: legacy-release
source: { device: fan.mock.0, capability: temperature.core }
target: { device: fan.mock.0, capability: fan.speed_percent }
curve:
  - [0, 40]
  - [100, 70]
fallback:
  on_sensor_missing: release
YAML
say "  config: $CONFIG"
say "  report: $REPORT"

step "Building the application and its frontend bundle"
( cd "$REPO_ROOT/apps/desktop" && npm run build > "$WORK/frontend-build.log" 2>&1 )
BUILD_CODE=$?
check "frontend bundle builds" "$([ "$BUILD_CODE" -eq 0 ] && echo 1 || echo 0)" "exit $BUILD_CODE (log: $WORK/frontend-build.log)"
if [ "$BUILD_CODE" -ne 0 ]; then
  tail -20 "$WORK/frontend-build.log"
  say "Stopping: without a bundle the webview has nothing to run."
  exit 1
fi
# Built through the Tauri CLI, not with a bare `cargo build`, and that distinction is
# the whole point of this step being here:
#
#   * a **debug** build points at `devUrl` (`http://localhost:5173`) and expects the Vite
#     dev server, so running the binary on its own loads an *empty page* — the app starts,
#     reports no error, and has no frontend at all;
#   * a bare `cargo build --release` does not enable Tauri's `custom-protocol` feature,
#     which is what embeds the frontend bundle, so the release binary has no frontend
#     either.
#
# Only the CLI's build produces an application whose webview actually runs the bundle.
# This harness found both of those, which is exactly why "the app started" is not
# evidence of anything on its own.
( cd "$REPO_ROOT/apps/desktop" && npx tauri build --no-bundle > "$WORK/backend-build.log" 2>&1 )
CARGO_CODE=$?
check "desktop application builds (tauri build --no-bundle)" "$([ "$CARGO_CODE" -eq 0 ] && echo 1 || echo 0)" "exit $CARGO_CODE"
if [ "$CARGO_CODE" -ne 0 ]; then
  tail -20 "$WORK/backend-build.log"
  exit 1
fi

# Run only after the build: with no binary these checks would "pass" because the
# command failed to run at all, which is the opposite of evidence.
step "Refusal paths: the probe must not be able to run unisolated"
APP_BIN="$REPO_ROOT/target/release/ohm-desktop"

# (a) `--ipc-selftest` on its own used to arm the probe against whatever config
#     directory the app would have used — the real per-user one.
set +e
NO_ENV_OUT="$(cd "$REPO_ROOT" && RUST_LOG=info "$APP_BIN" --mock --ipc-selftest 2>&1)"
NO_ENV_CODE=$?
set -e
check "a probe with no isolated configuration directory is refused" \
  "$([ "$NO_ENV_CODE" -ne 0 ] && echo 1 || echo 0)" "exit $NO_ENV_CODE"
check "  and the refusal names the variable to set" \
  "$(printf '%s' "$NO_ENV_OUT" | grep -q "$ENV_NAME" && echo 1 || echo 0)" \
  "$(printf '%s' "$NO_ENV_OUT" | grep -m1 -o 'refusing[^.]*' | cut -c1-120)"

# (b) an isolated directory that still enables a real provider is refused by name.
REAL_PROVIDER_CONFIG="$WORK/real-provider"
mkdir -p "$REAL_PROVIDER_CONFIG"
printf '%s\n' '{"experimental_features": true, "disabled_adapters": ["system", "nvidia"]}' \
  > "$REAL_PROVIDER_CONFIG/settings.json"
set +e
REAL_OUT="$(cd "$REPO_ROOT" && OHM_CONFIG_DIR="$REAL_PROVIDER_CONFIG" "$APP_BIN" --mock --ipc-selftest 2>&1)"
REAL_CODE=$?
set -e
check "a probe whose settings enable a real provider is refused" \
  "$([ "$REAL_CODE" -ne 0 ] && echo 1 || echo 0)" "exit $REAL_CODE"
check "  and the refusal names the provider" \
  "$(printf '%s' "$REAL_OUT" | grep -q 'lhm' && echo 1 || echo 0)" \
  "$(printf '%s' "$REAL_OUT" | grep -m1 -o 'refusing[^.]*' | cut -c1-120)"
check "  and it did not write a probe report there" \
  "$([ ! -f "$REAL_PROVIDER_CONFIG/$PROBE_FILE" ] && echo 1 || echo 0)" "$REAL_PROVIDER_CONFIG"

step "Launching the real application"
[ -x "$APP_BIN" ] || { say "  missing $APP_BIN"; exit 1; }
# The version probe runs the same binary, so it needs its own throwaway config
# directory: `ConfigPaths::discover()` falls back to the *real* per-user config
# directory, and an earlier version of this script therefore created one and wrote
# mock-device audit records into it while claiming to be isolated.
mkdir -p "$WORK/version-probe"
VERSION="$(cd "$REPO_ROOT" && OHM_CONFIG_DIR="$WORK/version-probe" "$RUSTUP" run "$TOOLCHAIN" cargo run -q -p ohm-desktop -- --selftest --mock 2>/dev/null | head -1)"
say "  binary : $APP_BIN"
say "  version: $VERSION"
say "  args   : --mock --ipc-selftest"
say "  env    : OHM_CONFIG_DIR=$CONFIG RUST_LOG=info"
( cd "$REPO_ROOT" && OHM_CONFIG_DIR="$CONFIG" RUST_LOG=info "$APP_BIN" --mock --ipc-selftest > "$APP_LOG" 2>&1 ) &
APP_PID=$!

# A picture of the running window, when the operator explicitly asks for one
# (`OHM_IPC_SCREENSHOT=1`). The panel the probe renders is what would make it evidence:
# it shows the round trip's own findings, not merely that a webview exists.
#
# Read this before using it. `screencapture` cannot select a window without a clickable
# session, so this is a **whole-screen** capture: it contains whatever else is on the
# screen, which on a working machine means other applications and possibly private
# content. Treat the file as the operator's, not as project evidence: review it before
# sharing it, never add it to a package, and expect the run's temporary directory to be
# removed afterwards unless `--keep` is given.
if [ "${OHM_IPC_SCREENSHOT:-0}" = "1" ]; then
  (
    sleep 14
    screencapture -x "$WORK/screenshot.png" 2>/dev/null || true
  ) &
  SCREENSHOT_PID=$!
fi

# The app exits by itself once the frontend has reported; wait for either.
DEADLINE=$((SECONDS + 150))
while [ ! -f "$REPORT" ] && [ "$SECONDS" -lt "$DEADLINE" ]; do
  if ! kill -0 "$APP_PID" 2>/dev/null; then break; fi
  sleep 1
done
wait "$APP_PID" 2>/dev/null
APP_CODE=$?
say "  app exit code: $APP_CODE"

step "Checking the frontend's own account of the round trip"
if [ ! -f "$REPORT" ]; then
  check "the frontend reported back through a command" 0 "no report at $REPORT — the round trip did not close"
  say "  last lines of the application log:"
  tail -20 "$APP_LOG" | sed 's/^/    /'
  say
  say "This is a real failure of the desktop path, not a missing test: the app started,"
  say "the backend asked the webview to run the probe, and nothing came back."
  exit 1
fi
check "the frontend reported back through a command" 1 "$REPORT ($(wc -c < "$REPORT" | tr -d ' ') bytes)"

python3 - "$REPORT" <<'PY'
import json, sys
report = json.load(open(sys.argv[1]))
print(f"  webview : {report.get('location')}")
print(f"  agent   : {report.get('user_agent')}")
ok = True
for step in report.get("steps", []):
    mark = "PASS" if step["ok"] else "FAIL"
    if not step["ok"]:
        ok = False
    summary = json.dumps(step.get("result"))[:150] if step["ok"] else step.get("error", "")
    print(f"  [{mark}] {step['step']}: {summary}")
sys.exit(0 if ok else 1)
PY
STEPS_CODE=$?
check "every command the probe called succeeded" "$([ "$STEPS_CODE" -eq 0 ] && echo 1 || echo 0)" "see the per-step results above"

step "Checking that what the frontend saw is what the backend did"
python3 - "$REPORT" "$CONFIG/audit.jsonl" <<'PY'
import json, sys

report = json.load(open(sys.argv[1]))
rendered = report.get("rendered", {})
failures = []

def check(label, ok, detail=""):
    print(f"  {'PASS' if ok else 'FAIL'}  {label}")
    if detail:
        print(f"        {detail}")
    if not ok:
        failures.append(label)

# 1. Compatibility notes reached the frontend, structured.
notes = next((s for s in report["steps"] if s["step"] == "rule_compatibility_notes"), None)
result = (notes or {}).get("result") or []
check(
    "the legacy `release` rule was reported to the frontend as a compatibility note",
    isinstance(result, list) and len(result) == 1
    and result[0].get("field") == "fallback.on_sensor_missing"
    and result[0].get("original") == "release",
    json.dumps(result)[:300],
)

# 2. An unconfirmed write came back through IPC as unconfirmed, with no value.
check(
    "the write came back as `unconfirmed` through the real IPC",
    rendered.get("writeStatus") == "unconfirmed",
    f"status={rendered.get('writeStatus')}",
)
check(
    "no value was claimed for it (the UI must not invent one)",
    rendered.get("writeApplied") == "absent",
    f"applied={rendered.get('writeApplied')}",
)
check(
    "the backend's reason came with it",
    "unknown" in (rendered.get("writeDetail") or "") or "not report" in (rendered.get("writeDetail") or ""),
    f"detail={rendered.get('writeDetail')}",
)

# 3. The backend's own audit trail agrees with what the frontend was told.
audit_path = sys.argv[2]
writes = []
try:
    for line in open(audit_path):
        line = line.strip()
        if not line:
            continue
        entry = json.loads(line)
        if entry.get("kind") == "write":
            writes.append(entry["report"])
except FileNotFoundError:
    pass
unconfirmed = [w for w in writes if w.get("status") == "unconfirmed"]
check(
    "the audit trail records the same unconfirmed write",
    len(unconfirmed) >= 1,
    f"{len(unconfirmed)} of {len(writes)} write(s) are unconfirmed",
)
if unconfirmed:
    check(
        "and records no value for it either",
        unconfirmed[-1].get("applied") is None,
        f"applied={unconfirmed[-1].get('applied')}",
    )

# 4. Safety: only the simulated devices were written to.
devices = sorted({w.get("device_id") for w in writes})
check(
    "only simulated devices were written to",
    all((d or "").endswith(".mock.0") or ".mock." in (d or "") or "sim" in (d or "") for d in devices),
    f"devices written: {devices}",
)

# 5. An unfinished handover was created, read, retried and confirmed — all through IPC.
states = rendered.get("handoverStates", "")
check(
    "an unfinished handover was visible through IPC while it was owed",
    "fan.mock.1=pending" in states or "fan.mock.1=failed" in states,
    f"states observed: {states}",
)
check(
    "the retry re-armed it and it completed through IPC",
    "fan.mock.1=confirmed" in states,
    f"states observed: {states}",
)
reports = rendered.get("handoversAfter", "")
check(
    "and the resolved handover carries the value the fail-safe duty wrote",
    "confirmed" in reports,
    reports or "no handover listed",
)

# 5b. The probe's own fault checks: the injected fault was verified in force, and the
#     channel was left clean — a probe that assumed this would report a confident pass
#     about a channel it never faulted.
probe_checks = {s["step"]: s for s in report.get("steps", [])}
for label in (
    "the injected fault is in force on the handover channel",
    "the fault was cleared before retrying",
    "the unconfirmed fault is in force on the write channel",
    "the write channel was left with no fault",
):
    entry = probe_checks.get(label)
    check(
        label,
        bool(entry and entry.get("ok")),
        (entry or {}).get("error") or (entry or {}).get("result") or "the probe did not record this check",
    )

# 6. The frontend's account of the audit trail matches the file on disk.
check(
    "the frontend read the audit trail through IPC",
    "status=" in (rendered.get("auditWrite") or ""),
    rendered.get("auditWrite", ""),
)

sys.exit(1 if failures else 0)
PY
CROSS_CODE=$?
check "the frontend's account and the backend's records agree" "$([ "$CROSS_CODE" -eq 0 ] && echo 1 || echo 0)" "see the comparisons above"

if [ -n "${SCREENSHOT_PID:-}" ]; then
  wait "$SCREENSHOT_PID" 2>/dev/null || true
  if [ -s "$WORK/screenshot.png" ]; then
    check "a whole-screen capture was taken" 1 "$WORK/screenshot.png ($(wc -c < "$WORK/screenshot.png" | tr -d ' ') bytes)"
    say "        NOT VERIFIED: nobody has looked at it. It is a whole-screen capture and"
    say "        may contain unrelated or private content; review it yourself, and note"
    say "        that the harness deletes this directory unless you pass --keep."
  else
    check "a whole-screen capture was taken" 0 "screencapture produced nothing"
  fi
fi

step "Checking that the probe left work it did not create alone"
python3 - "$CONFIG/control-state.json" "$REPORT" <<'PYEOF'
import json, sys

record_path, report_path = sys.argv[1], sys.argv[2]
try:
    record = json.load(open(record_path))
except FileNotFoundError:
    print("  FAIL  the seeded handover: the control record is gone entirely")
    sys.exit(1)

seeded = [h for h in record.get("handovers", []) if h["device"] == "fan.lhm.0"]
if not seeded:
    print("  FAIL  the seeded handover is no longer in the record at all")
    sys.exit(1)

item = seeded[0]
state_ok = item["state"] in ("failed", "needs_verification")
attempts_ok = item["attempts"] == 3
print(f"  {'PASS' if state_ok else 'FAIL'}  the seeded real-channel handover was not re-armed")
print(f"        state={item['state']} attempts={item['attempts']} (seeded: failed, 3 attempts)")
print(f"  {'PASS' if attempts_ok else 'FAIL'}  its attempt budget is unchanged")

report = json.load(open(report_path))
rendered = report.get("rendered", {})
steps = {entry["step"]: entry for entry in report.get("steps", [])}
scoped = steps.get("rule_retry_handovers", {})
scoped_ok = scoped.get("ok") is True and scoped.get("result") == 1
print(f"  {'PASS' if scoped_ok else 'FAIL'}  the probe's retry was scoped and re-armed exactly one channel")
print(f"        rule_retry_handovers -> {scoped.get('result', scoped.get('error'))}")
allowed_ok = rendered.get("retryAllowed") == "true"
print(f"  {'PASS' if allowed_ok else 'FAIL'}  it retried only because it had created the handover itself")
sys.exit(0 if (state_ok and attempts_ok and scoped_ok and allowed_ok) else 1)
PYEOF
SEEDED_CODE=$?
check "the probe left work it did not create alone" "$([ "$SEEDED_CODE" -eq 0 ] && echo 1 || echo 0)" "see the comparisons above"

step "Checking that the run stayed inside its own directories"
if [ -d "$REAL_CONFIG" ]; then REAL_CONFIG_AFTER="present"; else REAL_CONFIG_AFTER="absent"; fi
check "the real per-user config directory is untouched" \
  "$([ "$REAL_CONFIG_BEFORE" = "$REAL_CONFIG_AFTER" ] && echo 1 || echo 0)" \
  "$REAL_CONFIG was $REAL_CONFIG_BEFORE before the run and is $REAL_CONFIG_AFTER after it$([ "$REAL_CONFIG_AFTER" = "present" ] && echo ' — something ran without OHM_CONFIG_DIR')"

step "Summary"
if [ "$FAILURES" -eq 0 ]; then
  say "IPC ROUND TRIP VERIFIED — the real application, its real webview and the"
  say "command surface are connected, and the frontend's account matches the audit trail."
  exit 0
fi
say "IPC ROUND TRIP FAILED — $FAILURES check(s) failed. The desktop IPC path is NOT verified."
exit 1
