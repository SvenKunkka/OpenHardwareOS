#!/usr/bin/env bash
#
# Verifies every file in this package against MANIFEST.sha256.
#
# The PowerShell twin (`verify-package.ps1`) is what an operator runs on Windows; this
# one exists so the package can be checked on any machine, including before it is
# handed over. Both implement the same rule: the manifest must describe every file
# present, and every file present must match its recorded hash.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
MANIFEST="$ROOT/MANIFEST.sha256"
QUIET=0
[ "${1:-}" = "--quiet" ] && QUIET=1

[ -f "$MANIFEST" ] || { echo "FAIL: MANIFEST.sha256 not found in $ROOT" >&2; exit 1; }

PROBLEMS=0
CHECKED=0
EXPECTED_LIST="$(mktemp)"
trap 'rm -f "$EXPECTED_LIST"' EXIT

hash_of() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

while IFS= read -r line; do
  [ -z "$line" ] && continue
  hash="${line%%  *}"
  rel="${line#*  }"
  if [ -z "$hash" ] || [ -z "$rel" ] || [ "${#hash}" -ne 64 ]; then
    echo "  * unparseable manifest line: $line" >&2
    PROBLEMS=$((PROBLEMS+1))
    continue
  fi
  printf '%s\n' "$rel" >> "$EXPECTED_LIST"
  if [ ! -f "$ROOT/$rel" ]; then
    echo "  * missing: $rel" >&2
    PROBLEMS=$((PROBLEMS+1))
    continue
  fi
  actual="$(hash_of "$ROOT/$rel")"
  CHECKED=$((CHECKED+1))
  if [ "$actual" != "$hash" ]; then
    echo "  * changed: $rel" >&2
    echo "      expected $hash" >&2
    echo "      actual   $actual" >&2
    PROBLEMS=$((PROBLEMS+1))
  fi
done < "$MANIFEST"

# Anything the manifest does not describe is a finding too.
while IFS= read -r file; do
  rel="${file#"$ROOT"/}"
  case "$rel" in
    MANIFEST.sha256|.precheck-*) continue ;;
  esac
  if ! grep -Fqx "$rel" "$EXPECTED_LIST"; then
    echo "  * not in the manifest: $rel" >&2
    PROBLEMS=$((PROBLEMS+1))
  fi
done < <(find "$ROOT" -type f)

if [ "$PROBLEMS" -gt 0 ]; then
  if [ "$QUIET" = "1" ]; then
    echo "$PROBLEMS problem(s) against $CHECKED checked file(s)"
  else
    echo
    echo "PACKAGE VERIFICATION FAILED — $PROBLEMS problem(s) against $CHECKED checked file(s)."
    echo "Do not use this package for acceptance evidence: it is not the tree the manifest describes."
  fi
  exit 1
fi

if [ "$QUIET" = "1" ]; then
  echo "$CHECKED file(s) verified"
else
  echo "PACKAGE VERIFIED — $CHECKED file(s) match MANIFEST.sha256"
fi
