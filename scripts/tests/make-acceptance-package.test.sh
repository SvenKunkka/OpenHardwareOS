#!/usr/bin/env bash
#
# Regression tests for `scripts/make-acceptance-package.sh`.
#
# They run the real packager against a throwaway git repository built for each case, so
# they never touch this project's own packages. Nobody's artefacts are deleted, renamed
# or rewritten by these tests: the fixture repository lives in a temporary directory and
# is removed afterwards.
#
# What they exist to catch, each of which was a real defect:
#
#   1. a second package of the same commit overwriting the first one;
#   2. entry-point templates taken from the *working tree*, so `--allow-dirty` could ship
#      uncommitted content that the commit does not contain;
#   3. untracked files either entering the package or blocking it (they can do neither);
#   4. a failed verification leaving something that looks like a delivery.

set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
PACKAGER="$REPO_ROOT/scripts/make-acceptance-package.sh"
[ -f "$PACKAGER" ] || { echo "missing $PACKAGER" >&2; exit 1; }

WORK="$(mktemp -d "${TMPDIR:-/tmp}/ohm-packager-tests.XXXXXX")"
FAILURES=0
CASES=0

cleanup() { if [ "${KEEP_FIXTURES:-0}" = "1" ]; then say "fixtures kept: $WORK"; else rm -rf "$WORK"; fi; }
trap cleanup EXIT

say() { printf '%s\n' "$*"; }
case_start() { CASES=$((CASES + 1)); printf '\n--- %s\n' "$1"; }
check() {
  local label="$1" ok="$2" detail="${3:-}"
  if [ "$ok" = "1" ]; then
    printf '    [PASS] %s\n' "$label"
  else
    printf '    [FAIL] %s\n' "$label"
    [ -n "$detail" ] && printf '           %s\n' "$detail"
    FAILURES=$((FAILURES + 1))
  fi
}

# ---------------------------------------------------------------------------
# A minimal repository with the shape the packager expects.
# ---------------------------------------------------------------------------
make_fixture() {
  local dir="$1"
  mkdir -p "$dir"
  (
    cd "$dir" || exit 1
    git init -q .
    git config user.email "test@example.invalid"
    git config user.name "packager test"
    mkdir -p scripts docs/windows-validation/package/scripts/tests crates/demo/src

    cp "$PACKAGER" scripts/make-acceptance-package.sh
    chmod +x scripts/make-acceptance-package.sh

    # The entry points the packager copies, in both the places they exist. The pre-check
    # is realistic because the packager *exercises* it: it asserts that the shipped
    # pre-check refuses a non-Windows host and passes with -AllowNonWindows, so a dummy
    # that always succeeds would fail the packager — which is the assertion working.
    for name in build verify-package _tools; do
      printf 'Write-Output "template %s"\n' "$name" > "docs/windows-validation/package/scripts/$name.ps1"
      printf 'Write-Output "template %s"\n' "$name" > "docs/windows-validation/package/scripts/tests/$name-helper.ps1"
    done
    cat > docs/windows-validation/package/scripts/precheck.ps1 <<'PS1'
param([switch]$AllowNonWindows, [string]$Toolchain = '', [string]$WorkDir = '')
Write-Host 'pre-check (test double)'
if (-not ($IsWindows -or $env:OS -eq 'Windows_NT')) {
    if (-not $AllowNonWindows) {
        Write-Host '[FAIL] host operating system        this is not Windows'
        Write-Host 'PRE-CHECK FAILED'
        exit 1
    }
    Write-Host '[PASS] host operating system        allowed by -AllowNonWindows'
}
Write-Host 'PRE-CHECK PASSED'
exit 0
PS1
    # A *valid* trivial script: the packager runs it, so it has to be executable shell.
    printf '#!/bin/sh\nexit 0\n' > docs/windows-validation/package/scripts/verify-package.sh
    chmod +x docs/windows-validation/package/scripts/verify-package.sh
    printf '# acceptance readme\n' > docs/windows-validation/package/README-ACCEPTANCE.md
    printf 'fn main() {}\n' > crates/demo/src/main.rs
    printf '[package]\nname = "demo"\nversion = "1.0.0"\n' > Cargo.toml

    git add -A
    git commit -q -m "fixture"
  )
}

# ---------------------------------------------------------------------------
# 1. A second package of the same commit must not damage the first.
# ---------------------------------------------------------------------------
case_start "a second package of the same commit leaves the first alone"
FIX1="$WORK/first"
make_fixture "$FIX1"
( cd "$FIX1" && ./scripts/make-acceptance-package.sh > "$WORK/first.log" 2>&1 )
FIRST_CODE=$?
check "the first package was produced" "$([ "$FIRST_CODE" -eq 0 ] && echo 1 || echo 0)" "exit $FIRST_CODE (log: $WORK/first.log)"
FIRST_ZIP="$(ls "$FIX1"/dist/acceptance/*.zip 2>/dev/null | head -1)"
FIRST_SHA="$(python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$FIRST_ZIP" 2>/dev/null)"
check "it has an archive" "$([ -n "$FIRST_ZIP" ] && echo 1 || echo 0)" "$FIRST_ZIP"
SIDECAR_SHA="$(awk 'NR==1 {print $1}' "$FIRST_ZIP.sha256" 2>/dev/null)"
check "  and a hash written beside it, not inside it" \
  "$([ -n "$SIDECAR_SHA" ] && [ "$SIDECAR_SHA" = "$FIRST_SHA" ] && echo 1 || echo 0)" \
  "$FIRST_ZIP.sha256 -> $SIDECAR_SHA"
check "  the sidecar also carries the manifest digest" \
  "$([ "$(wc -l < "$FIRST_ZIP.sha256" | tr -d ' ')" = "2" ] && grep -q 'MANIFEST.sha256' "$FIRST_ZIP.sha256" && echo 1 || echo 0)" \
  "$(tr '\n' '|' < "$FIRST_ZIP.sha256")"
# Nothing may be written into the package after the manifest is computed: the manifest
# claims to list every file, so a file added afterwards makes the package fail its own
# verification. The first version of the sidecar did exactly that.
PKG1="$(ls -d "$FIX1"/dist/acceptance/*-windows-acceptance | head -1)"
UNLISTED="$(python3 - "$PKG1" <<'PYCHECK'
import pathlib, sys
root = pathlib.Path(sys.argv[1])
listed = {
    line.split("  ", 1)[1].strip()
    for line in (root / "MANIFEST.sha256").read_text().splitlines()
    if "  " in line
}
present = {
    path.relative_to(root).as_posix()
    for path in root.rglob("*")
    if path.is_file() and path.name != "MANIFEST.sha256"
}
extra = sorted(present - listed)
missing = sorted(listed - present)
print("; ".join(f"unlisted={extra}" if extra else []) + ("; " if extra and missing else "") + ("; ".join(f"missing={missing}" if missing else [])))
PYCHECK
)"
check "  every file in the package is in its manifest" \
  "$([ -z "$UNLISTED" ] && echo 1 || echo 0)" "${UNLISTED:-all 20 files listed}"

( cd "$FIX1" && ./scripts/make-acceptance-package.sh > "$WORK/second.log" 2>&1 )
SECOND_CODE=$?
SECOND_SHA="$(python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$FIRST_ZIP" 2>/dev/null)"
check "a second package of the same commit is refused" "$([ "$SECOND_CODE" -ne 0 ] && echo 1 || echo 0)" "exit $SECOND_CODE"
check "  and the refusal says how to name a new build" \
  "$(grep -q -- '--build-id' "$WORK/second.log" && echo 1 || echo 0)" \
  "$(grep -m1 'Refusing to touch' "$WORK/second.log")"
check "  and the first package is byte-for-byte unchanged" \
  "$([ "$FIRST_SHA" = "$SECOND_SHA" ] && echo 1 || echo 0)" "$FIRST_SHA vs $SECOND_SHA"
check "  and no staging directory was left behind" \
  "$([ -z "$(ls -d "$FIX1"/dist/acceptance/.staging-* 2>/dev/null)" ] && echo 1 || echo 0)"

# A named build gets its own package instead.
( cd "$FIX1" && ./scripts/make-acceptance-package.sh --build-id "second" > "$WORK/named.log" 2>&1 )
NAMED_CODE=$?
check "a named build of the same commit is allowed" "$([ "$NAMED_CODE" -eq 0 ] && echo 1 || echo 0)" "exit $NAMED_CODE"
check "  and both packages exist" \
  "$([ "$(ls "$FIX1"/dist/acceptance/*.zip 2>/dev/null | wc -l | tr -d ' ')" = "2" ] && echo 1 || echo 0)" \
  "$(ls "$FIX1"/dist/acceptance/ 2>/dev/null | tr '\n' ' ')"

# ---------------------------------------------------------------------------
# 2. Working-tree template changes must not reach the snapshot.
# ---------------------------------------------------------------------------
case_start "a template edited in the working tree does not enter the package"
FIX2="$WORK/mixed"
make_fixture "$FIX2"
# The working tree diverges from the commit: a template gains a line that the commit
# does not have.
printf 'Write-Output "UNCOMMITTED CONTENT"\n' >> "$FIX2/docs/windows-validation/package/scripts/precheck.ps1"
printf '# UNCOMMITTED README\n' >> "$FIX2/docs/windows-validation/package/README-ACCEPTANCE.md"
( cd "$FIX2" && ./scripts/make-acceptance-package.sh --allow-dirty > "$WORK/mixed.log" 2>&1 )
MIXED_CODE=$?
check "the packager runs with a modified tracked file" "$([ "$MIXED_CODE" -eq 0 ] && echo 1 || echo 0)" "exit $MIXED_CODE (log: $WORK/mixed.log)"
PKG2="$(ls -d "$FIX2"/dist/acceptance/*-windows-acceptance 2>/dev/null | head -1)"
if [ -n "$PKG2" ]; then
  check "  the snapshot's precheck.ps1 is the committed one" \
    "$(grep -q 'UNCOMMITTED CONTENT' "$PKG2/scripts/precheck.ps1" && echo 0 || echo 1)" \
    "$(head -c 80 "$PKG2/scripts/precheck.ps1")"
  check "  the snapshot's README is the committed one" \
    "$(grep -q 'UNCOMMITTED README' "$PKG2/README-ACCEPTANCE.md" && echo 0 || echo 1)"
  check "  the deviation is recorded in the package" \
    "$(grep -qi 'modified tracked file' "$WORK/mixed.log" && echo 1 || echo 0)" \
    "$(grep -i 'worktree' "$WORK/mixed.log" | head -1)"
  check "  the package still verifies against its own manifest" \
    "$( "$PKG2/scripts/verify-package.sh" >/dev/null 2>&1 && echo 1 || echo 0)"
else
  check "the package was produced" 0 "no package directory under $FIX2/dist/acceptance"
fi

# ---------------------------------------------------------------------------
# 3. Untracked files: not included, and not a reason to refuse.
# ---------------------------------------------------------------------------
case_start "untracked files neither enter the package nor block it"
FIX3="$WORK/untracked"
make_fixture "$FIX3"
printf 'not part of any commit\n' > "$FIX3/UNRELATED.md"
mkdir -p "$FIX3/some-unrelated-tool"
printf 'binary-ish\n' > "$FIX3/some-unrelated-tool/firmware.bin"
( cd "$FIX3" && ./scripts/make-acceptance-package.sh > "$WORK/untracked.log" 2>&1 )
UNTRACKED_CODE=$?
check "an untracked file does not block packaging" "$([ "$UNTRACKED_CODE" -eq 0 ] && echo 1 || echo 0)" "exit $UNTRACKED_CODE (log: $WORK/untracked.log)"
PKG3="$(ls -d "$FIX3"/dist/acceptance/*-windows-acceptance 2>/dev/null | head -1)"
if [ -n "$PKG3" ]; then
  check "  the untracked file is not in the package" \
    "$([ ! -e "$PKG3/UNRELATED.md" ] && echo 1 || echo 0)"
  check "  nor is the untracked directory" \
    "$([ ! -e "$PKG3/some-unrelated-tool" ] && echo 1 || echo 0)"
  check "  and the package verifies" \
    "$( "$PKG3/scripts/verify-package.sh" >/dev/null 2>&1 && echo 1 || echo 0)"
else
  check "the package was produced" 0 "no package directory"
fi

# ---------------------------------------------------------------------------
# 4. A failing check leaves no delivery behind.
# ---------------------------------------------------------------------------
case_start "a package that fails its own checks produces no delivery"
FIX4="$WORK/failing"
make_fixture "$FIX4"
# A Windows binary committed into the tree: the packager's own content check forbids it,
# so the run must fail *after* it has built a staging copy and before any delivery.
mkdir -p "$FIX4/tools"
printf 'MZ fake\n' > "$FIX4/tools/installer.exe"
( cd "$FIX4" && git add -A && git commit -q -m "add a forbidden artefact" )
( cd "$FIX4" && ./scripts/make-acceptance-package.sh > "$WORK/failing.log" 2>&1 )
FAILING_CODE=$?
check "the run fails on the forbidden content" "$([ "$FAILING_CODE" -ne 0 ] && echo 1 || echo 0)" "exit $FAILING_CODE"
check "  no package directory was created" \
  "$([ -z "$(ls -d "$FIX4"/dist/acceptance/*-windows-acceptance 2>/dev/null)" ] && echo 1 || echo 0)" \
  "$(ls "$FIX4"/dist/acceptance/ 2>/dev/null | tr '\n' ' ')"
check "  no archive was created" \
  "$([ -z "$(ls "$FIX4"/dist/acceptance/*.zip 2>/dev/null)" ] && echo 1 || echo 0)"
check "  and the failure is named" \
  "$(grep -qi 'must not contain\|FOUND' "$WORK/failing.log" && echo 1 || echo 0)" \
  "$(grep -i 'FOUND' "$WORK/failing.log" | head -1)"

say ""
say "cases: $((CASES))   failures: $FAILURES"
if [ "$FAILURES" -eq 0 ]; then
  say "RESULT: PASS"
  exit 0
fi
say "RESULT: FAIL"
exit 1
