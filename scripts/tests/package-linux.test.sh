#!/usr/bin/env bash
#
# Fixture tests for scripts/release/package-linux.sh.
#
#   ./scripts/tests/package-linux.test.sh
#
# No Linux host and no Linux build are involved: a throwaway git repository is
# built with a fake ELF binary, and the real packager is run against it. What is
# checked is what the packager promises — a refused overwrite, an ELF check that
# rejects anything else, LF checksums a POSIX `shasum -c` can read, metadata
# that names the commit, and a delivery that only appears when it verifies.
#
# This is not evidence that a Linux build succeeds: that needs the Linux CI job.
set -uo pipefail

SCRIPT="$(cd "$(dirname "$0")/../release" && pwd)/package-linux.sh"
[ -f "$SCRIPT" ] || { echo "error: cannot find $SCRIPT" >&2; exit 1; }

ROOT="$(mktemp -d "${TMPDIR:-/tmp}/ohm-linux-package-XXXXXX")"
KEEP="${KEEP_FIXTURES:-0}"
PASSED=0
FAILED=0
CASES=0

cleanup() {
  if [ "$KEEP" = "1" ]; then
    echo "kept: $ROOT"
  else
    rm -rf "$ROOT"
  fi
}
trap cleanup EXIT

ok()   { PASSED=$((PASSED + 1)); printf '    [PASS] %s\n' "$1"; }
bad()  { FAILED=$((FAILED + 1)); printf '    [FAIL] %s\n' "$1"; }
check() { if [ "$2" = "1" ]; then ok "$1"; else bad "$1"; fi; }

# A file whose first bytes are a Linux x86_64 ELF header.
fake_elf() {
  python3 - "$1" <<'PY'
import struct, sys, os
path = sys.argv[1]
data = bytearray(256)
data[0:4] = b"\x7fELF"
data[4] = 2          # 64-bit
data[5] = 1          # little endian
data[6] = 1          # version
struct.pack_into("<H", data, 16, 2)   # ET_EXEC
struct.pack_into("<H", data, 18, 0x3E)  # x86-64
with open(path, "wb") as handle:
    handle.write(data)
os.chmod(path, 0o755)
PY
}

# A throwaway repository with the files the packager insists on.
new_repo() {
  local repo="$ROOT/repo-$1"
  rm -rf "$repo"
  mkdir -p "$repo/scripts/release"
  cp "$SCRIPT" "$repo/scripts/release/package-linux.sh"
  printf 'Apache License 2.0 fixture\n' > "$repo/LICENSE"
  printf 'notices fixture\n' > "$repo/THIRD_PARTY_NOTICES.txt"
  mkdir -p "$repo/artifacts"
  fake_elf "$repo/artifacts/ohm-cli"
  git -C "$repo" init --quiet
  git -C "$repo" config user.email fixture@example.invalid
  git -C "$repo" config user.name Fixture
  git -C "$repo" add -A
  git -C "$repo" commit --quiet -m fixture
  echo "$repo"
}

run_packager() { # repo, extra args…
  local repo="$1"; shift
  ( cd "$repo" && bash scripts/release/package-linux.sh --version v0.1.3 \
      --binary artifacts/ohm-cli --out artifacts/release "$@" )
}

echo "================================================================================"
echo " OpenHardwareOS — Linux packaging tests (FIXTURES ONLY)"
echo "================================================================================"
echo " host      : $(uname -srm)"
echo " temporary : $ROOT"
echo
echo " These fixtures exercise the packager with a fake ELF binary. They are NOT"
echo " evidence that a Linux build succeeds; the Linux CI job is."

# ---------------------------------------------------------------- case 1
echo
echo "--- a good binary is packaged, verified and delivered"
CASES=$((CASES + 1))
repo="$(new_repo good)"
out="$repo/artifacts/release"
if run_packager "$repo" > "$ROOT/good.log" 2>&1; then
  ok "the packager exits 0"
else
  bad "the packager exits 0 ($(tail -1 "$ROOT/good.log"))"
fi
archive="$out/ohm-cli-v0.1.3-linux-x86_64.tar.gz"
[ -f "$archive" ] && ok "the archive exists" || bad "the archive exists"
[ -f "$out/SHA256SUMS" ] && ok "a checksum list exists" || bad "a checksum list exists"
[ -f "$out/release-linux-x86_64.json" ] && ok "platform metadata exists" || bad "platform metadata exists"

if [ -f "$out/SHA256SUMS" ]; then
  if grep -q $'\r' "$out/SHA256SUMS"; then
    bad "the checksum list uses LF (POSIX shasum -c can read it)"
  else
    ok "the checksum list uses LF (POSIX shasum -c can read it)"
  fi
  entries=$(grep -c . "$out/SHA256SUMS")
  check "the list covers four files (found $entries)" "$([ "$entries" = "4" ] && echo 1 || echo 0)"
  # Verify the way the target platform does.
  side="$ROOT/side-good"
  mkdir -p "$side"
  cp "$archive" "$out/release-linux-x86_64.json" "$repo/LICENSE" "$repo/THIRD_PARTY_NOTICES.txt" "$out/SHA256SUMS" "$side/"
  if ( cd "$side" && shasum -a 256 -c SHA256SUMS >/dev/null 2>&1 ); then
    ok "shasum -c verifies every entry"
  else
    bad "shasum -c verifies every entry"
  fi
fi

if [ -f "$archive" ]; then
  tar -tzf "$archive" | grep -qx 'ohm-cli' && ok "the archive contains the CLI" || bad "the archive contains the CLI"
  tar -tzf "$archive" | grep -qx 'release-linux-x86_64.json' && ok "the archive carries its metadata" || bad "the archive carries its metadata"
fi

commit=$(git -C "$repo" rev-parse HEAD)
recorded=$(python3 -c "import json,sys;print(json.load(open('$out/release-linux-x86_64.json'))['source_commit'])" 2>/dev/null || echo none)
check "the metadata names the commit the build came from ($commit)" \
  "$([ "$recorded" = "$commit" ] && echo 1 || echo 0)"
platform=$(python3 -c "import json;print(json.load(open('$out/release-linux-x86_64.json'))['platform'])" 2>/dev/null || echo none)
check "the metadata names the platform ($platform)" "$([ "$platform" = "linux-x86_64" ] && echo 1 || echo 0)"

# ---------------------------------------------------------------- case 2
echo
echo "--- a second run refuses to write into the first delivery"
CASES=$((CASES + 1))
before=$(shasum -a 256 "$archive" | cut -d' ' -f1)
if run_packager "$repo" > "$ROOT/again.log" 2>&1; then
  bad "the packager refuses an existing output directory"
else
  ok "the packager refuses an existing output directory"
  grep -q "output already exists" "$ROOT/again.log" \
    && ok "  and says how to proceed" || bad "  and says how to proceed"
fi
after=$(shasum -a 256 "$archive" | cut -d' ' -f1)
check "the first delivery is untouched" "$([ "$before" = "$after" ] && echo 1 || echo 0)"

# ---------------------------------------------------------------- case 3
echo
echo "--- anything that is not a Linux x86_64 ELF is refused"
CASES=$((CASES + 1))
repo="$(new_repo notelf)"
printf '#!/bin/sh\necho ohm-cli 0.1.3\n' > "$repo/artifacts/ohm-cli"
chmod 755 "$repo/artifacts/ohm-cli"
if run_packager "$repo" > "$ROOT/notelf.log" 2>&1; then
  bad "a shell script is not accepted as a Linux binary"
else
  ok "a shell script is not accepted as a Linux binary"
  grep -q "not an ELF file" "$ROOT/notelf.log" && ok "  and the message says why" || bad "  and the message says why"
fi
[ ! -e "$repo/artifacts/release" ] && ok "nothing was delivered" || bad "nothing was delivered"

# 32-bit, and the wrong architecture: both have to be caught.
printf '\177ELF' > "$repo/artifacts/ohm-cli"
python3 - "$repo/artifacts/ohm-cli" <<'PY'
import struct, sys
path = sys.argv[1]
data = bytearray(open(path, "rb").read()) + bytearray(252)
data[4] = 1                      # 32-bit
struct.pack_into("<H", data, 18, 0x3E)
open(path, "wb").write(data)
PY
if run_packager "$repo" > "$ROOT/class32.log" 2>&1; then
  bad "a 32-bit ELF is refused"
else
  ok "a 32-bit ELF is refused"
fi

fake_elf "$repo/artifacts/ohm-cli"
python3 - "$repo/artifacts/ohm-cli" <<'PY'
import struct, sys
path = sys.argv[1]
data = bytearray(open(path, "rb").read())
data[4] = 2
struct.pack_into("<H", data, 18, 0x28)   # ARM
open(path, "wb").write(data)
PY
if run_packager "$repo" > "$ROOT/arm.log" 2>&1; then
  bad "an ARM ELF is refused"
else
  ok "an ARM ELF is refused"
  grep -q "not x86_64" "$ROOT/arm.log" && ok "  and the message names the architecture" || bad "  and the message names the architecture"
fi

# ---------------------------------------------------------------- case 4
echo
echo "--- a version that is not vMAJOR.MINOR.PATCH is refused"
CASES=$((CASES + 1))
repo="$(new_repo version)"
if ( cd "$repo" && bash scripts/release/package-linux.sh --version 0.1.3 \
      --binary artifacts/ohm-cli --out artifacts/release ) > "$ROOT/version.log" 2>&1; then
  bad "an unversioned release is refused"
else
  ok "an unversioned release is refused"
  grep -q "expected vMAJOR.MINOR.PATCH" "$ROOT/version.log" && ok "  with the expected shape" || bad "  with the expected shape"
fi

# ---------------------------------------------------------------- case 5
echo
echo "--- missing notices block the delivery"
CASES=$((CASES + 1))
repo="$(new_repo notices)"
rm "$repo/THIRD_PARTY_NOTICES.txt"
if run_packager "$repo" > "$ROOT/notices.log" 2>&1; then
  bad "a package without dependency notices is refused"
else
  ok "a package without dependency notices is refused"
  grep -q "THIRD_PARTY_NOTICES" "$ROOT/notices.log" && ok "  and names the file" || bad "  and names the file"
fi

echo
echo "================================================================================"
echo "  cases: $CASES   passed: $PASSED   failed: $FAILED"
if [ "$FAILED" -eq 0 ]; then
  echo "RESULT: PASS"
  exit 0
fi
echo "RESULT: FAIL"
exit 1
