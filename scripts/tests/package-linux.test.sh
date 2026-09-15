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
ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
MAKE_DEB="$ROOT_DIR/scripts/tests/make-fixture-deb.py"
[ -f "$MAKE_DEB" ] || { echo "error: cannot find $MAKE_DEB" >&2; exit 1; }

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

# A synthetic Debian package. Real structure, chosen contents: the packager's
# checks are about what the package *declares*, so a fixture has to be able to
# declare the wrong thing on purpose.
fake_deb() { # path, extra make-fixture-deb.py args…
  local path="$1"; shift
  python3 "$MAKE_DEB" "$path" --version 0.1.3 --binary-elf "$@" >/dev/null
}

fake_appimage() { # path
  fake_elf "$1"
}

# A throwaway repository with the files the packager insists on.
new_repo() {
  local repo="$ROOT/repo-$1"
  rm -rf "$repo"
  mkdir -p "$repo/scripts/release" "$repo/scripts/tests"
  cp "$SCRIPT" "$repo/scripts/release/package-linux.sh"
  # The packager reads a .deb with this tool; the fixture repository is the only
  # checkout the packager sees, so it has to be there too.
  cp "$(dirname "$SCRIPT")/inspect-deb.py" "$repo/scripts/release/inspect-deb.py"
  cp "$ROOT_DIR/scripts/tests/make-fixture-deb.py" "$repo/scripts/tests/make-fixture-deb.py"
  printf 'Apache License 2.0 fixture\n' > "$repo/LICENSE"
  mkdir -p "$repo/artifacts"
  printf 'notices fixture\n' > "$repo/artifacts/THIRD_PARTY_NOTICES.txt"
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
[ -f "$out/SHA256SUMS-linux-x86_64" ] && ok "a checksum list exists" || bad "a checksum list exists"
[ -f "$out/release-linux-x86_64.json" ] && ok "platform metadata exists" || bad "platform metadata exists"

if [ -f "$out/SHA256SUMS-linux-x86_64" ]; then
  if grep -q $'\r' "$out/SHA256SUMS-linux-x86_64"; then
    bad "the checksum list uses LF (POSIX shasum -c can read it)"
  else
    ok "the checksum list uses LF (POSIX shasum -c can read it)"
  fi
  entries=$(grep -c . "$out/SHA256SUMS-linux-x86_64")
  check "the list covers the two published files (found $entries)" "$([ "$entries" = "2" ] && echo 1 || echo 0)"
  # The list must name only what a downloader actually gets. `LICENSE` and
  # `THIRD_PARTY_NOTICES.txt` travel inside the archive; naming them here made a
  # plain `sha256sum -c` on a real download report two missing files — which is
  # what happened to v0.1.3's first Linux build before it was published. So the
  # side directory is filled with the published assets and *nothing else*: a
  # fixture that stages extra files hides the defect it is meant to catch.
  side="$ROOT/side-good"
  mkdir -p "$side"
  cp "$archive" "$out/release-linux-x86_64.json" "$out/SHA256SUMS-linux-x86_64" "$side/"
  if ( cd "$side" && shasum -a 256 -c SHA256SUMS-linux-x86_64 >/dev/null 2>&1 ); then
    ok "shasum -c verifies every entry against the published assets"
  else
    bad "shasum -c verifies every entry against the published assets"
  fi
  if grep -qE '  (LICENSE|THIRD_PARTY_NOTICES\.txt)$' "$out/SHA256SUMS-linux-x86_64"; then
    bad "the list does not name files that are only inside the archive"
  else
    ok "the list does not name files that are only inside the archive"
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
rm "$repo/artifacts/THIRD_PARTY_NOTICES.txt"
if run_packager "$repo" > "$ROOT/notices.log" 2>&1; then
  bad "a package without dependency notices is refused"
else
  ok "a package without dependency notices is refused"
  grep -q "THIRD_PARTY_NOTICES" "$ROOT/notices.log" && ok "  and names the file" || bad "  and names the file"
fi

# ---------------------------------------------------------------- case 6
echo
echo "--- a tar that reports an early-closed pipe does not fail a passing check"
CASES=$((CASES + 1))
# GNU tar reports a write error when its consumer closes the pipe early; BSD tar
# (macOS) does not. `tar -tzf archive | grep -q name` therefore passed here and
# failed the first Linux release build. This shim reproduces the GNU behaviour.
mkdir -p "$ROOT/bin-gnu-tar"
cat > "$ROOT/bin-gnu-tar/tar" <<'SH'
#!/bin/sh
# Write the listing one line at a time with a pause between them, so a consumer
# that exits on its first match (`grep -q`) is gone before the next write — which
# is what makes GNU tar report "stdout: write error" on the Linux runner. A single
# buffered write of the whole listing would slip into the pipe buffer and hide it.
real=/usr/bin/tar
[ -x "$real" ] || real=$(command -v tar)
scratch="$(mktemp)"
"$real" "$@" > "$scratch" || { rc=$?; rm -f "$scratch"; exit $rc; }
status=0
while IFS= read -r line; do
  printf '%s\n' "$line" || { status=2; break; }
  sleep 0.05
done < "$scratch"
rm -f "$scratch"
exit $status
SH
chmod +x "$ROOT/bin-gnu-tar/tar"
repo="$(new_repo gnu-tar)"
if ( export PATH="$ROOT/bin-gnu-tar:$PATH"; run_packager "$repo" ) > "$ROOT/gnu-tar.log" 2>&1; then
  ok "the packager still delivers"
else
  bad "the packager still delivers ($(tail -1 "$ROOT/gnu-tar.log"))"
fi
[ -f "$repo/artifacts/release/ohm-cli-v0.1.3-linux-x86_64.tar.gz" ] \
  && ok "the archive exists" || bad "the archive exists"
if [ -f "$repo/artifacts/release/SHA256SUMS-linux-x86_64" ]; then
  side="$ROOT/side-gnu-tar"
  mkdir -p "$side"
  cp "$repo/artifacts/release/ohm-cli-v0.1.3-linux-x86_64.tar.gz" \
     "$repo/artifacts/release/release-linux-x86_64.json" \
     "$repo/artifacts/release/SHA256SUMS-linux-x86_64" "$side/"
  if ( cd "$side" && shasum -a 256 -c SHA256SUMS-linux-x86_64 >/dev/null 2>&1 ); then
    ok "  and its checksums verify"
  else
    bad "  and its checksums verify"
  fi
fi

# ---------------------------------------------------------------- case 7
echo
echo "--- the desktop bundles are published under this project's names, in one list"
CASES=$((CASES + 1))
repo="$(new_repo desktop)"
fake_deb "$ROOT/desktop-good.deb"
fake_appimage "$ROOT/desktop-good.AppImage"
if run_packager "$repo" --desktop-deb "$ROOT/desktop-good.deb" \
     --desktop-appimage "$ROOT/desktop-good.AppImage" > "$ROOT/desktop.log" 2>&1; then
  ok "the packager exits 0"
else
  bad "the packager exits 0 ($(tail -1 "$ROOT/desktop.log"))"
fi
out="$repo/artifacts/release"
[ -f "$out/OpenHardwareOS-v0.1.3-linux-x86_64.deb" ] \
  && ok "the .deb is published under this project's name" \
  || bad "the .deb is published under this project's name"
[ -f "$out/OpenHardwareOS-v0.1.3-linux-x86_64.AppImage" ] \
  && ok "the AppImage too" || bad "the AppImage too"
if [ -f "$out/SHA256SUMS-linux-x86_64" ]; then
  entries=$(grep -c . "$out/SHA256SUMS-linux-x86_64")
  check "one list covers all four Linux artefacts (found $entries)" \
    "$([ "$entries" = "4" ] && echo 1 || echo 0)"
  side="$ROOT/side-desktop"
  mkdir -p "$side"
  cp "$out"/ohm-cli-v0.1.3-linux-x86_64.tar.gz "$out"/release-linux-x86_64.json \
     "$out"/OpenHardwareOS-v0.1.3-linux-x86_64.deb \
     "$out"/OpenHardwareOS-v0.1.3-linux-x86_64.AppImage \
     "$out"/SHA256SUMS-linux-x86_64 "$side/"
  if ( cd "$side" && shasum -a 256 -c SHA256SUMS-linux-x86_64 >/dev/null 2>&1 ); then
    ok "  and a plain shasum -c verifies every one of them"
  else
    bad "  and a plain shasum -c verifies every one of them"
  fi
fi
grep -q "libwebkit2gtk-4.1-0" "$ROOT/desktop.log" \
  && ok "  and the declared dependencies are printed for the release notes" \
  || bad "  and the declared dependencies are printed for the release notes"

# ---------------------------------------------------------------- case 8
echo
echo "--- a desktop package that does not match the release is refused"
CASES=$((CASES + 1))
for variant in "version:--version 0.1.2" "arch:--arch arm64" "binary:--binary-script" \
               "desktop:--desktop-missing" "exec:--exec-name something-else" \
               "package:--package something-else"; do
  label="${variant%%:*}"; flags="${variant#*:}"
  repo="$(new_repo "bad-$label")"
  fake_deb "$ROOT/desktop-$label.deb" $flags
  if run_packager "$repo" --desktop-deb "$ROOT/desktop-$label.deb" \
       --desktop-appimage "$ROOT/desktop-good.AppImage" > "$ROOT/bad-$label.log" 2>&1; then
    bad "a $label mismatch is refused"
  else
    ok "a $label mismatch is refused ($(grep -m1 '^error:' "$ROOT/bad-$label.log" | sed 's/^error: //'))"
  fi
  # The refusal must not leave something that looks like a delivery: the next
  # run refuses to write into an existing output, so a leftover turns one
  # fixable mistake into a permanent one. This is a real defect this suite found.
  [ ! -e "$repo/artifacts/release" ] \
    && ok "  and nothing is left behind for the next attempt to trip over" \
    || bad "  and nothing is left behind for the next attempt to trip over"
done
# The corrected retry proves the point: same repository, right version.
repo="$(new_repo bad-version)"
fake_deb "$ROOT/desktop-retry.deb"
run_packager "$repo" --desktop-deb "$ROOT/desktop-retry.deb" \
  --desktop-appimage "$ROOT/desktop-good.AppImage" > "$ROOT/retry.log" 2>&1
[ -f "$repo/artifacts/release/OpenHardwareOS-v0.1.3-linux-x86_64.deb" ] \
  && ok "  and a corrected retry then delivers" \
  || bad "  and a corrected retry then delivers"

# ---------------------------------------------------------------- case 8b
echo
echo "--- a desktop entry may carry arguments, but must launch the installed binary"
CASES=$((CASES + 1))
repo="$(new_repo desktop-args)"
# A field code is normal in a freedesktop entry and must not be mistaken for a
# different program.
fake_deb "$ROOT/desktop-args.deb" --exec-args '%U'
if run_packager "$repo" --desktop-deb "$ROOT/desktop-args.deb" \
     --desktop-appimage "$ROOT/desktop-good.AppImage" > "$ROOT/desktop-args.log" 2>&1; then
  ok "an Exec line with a field code is accepted"
else
  bad "an Exec line with a field code is accepted ($(grep -m1 '^error:' "$ROOT/desktop-args.log"))"
fi

# ---------------------------------------------------------------- case 9
echo
echo "--- a desktop package needs its pair, and must be a Debian package"
CASES=$((CASES + 1))
repo="$(new_repo pair)"
fake_deb "$ROOT/desktop-pair.deb"
if run_packager "$repo" --desktop-deb "$ROOT/desktop-pair.deb" > "$ROOT/pair.log" 2>&1; then
  bad "a .deb without its AppImage is refused"
else
  ok "a .deb without its AppImage is refused"
fi
if run_packager "$repo" --desktop-appimage "$ROOT/desktop-good.AppImage" > "$ROOT/pair2.log" 2>&1; then
  bad "an AppImage without its .deb is refused"
else
  ok "an AppImage without its .deb is refused"
fi
printf 'this is not a Debian package\n' > "$ROOT/not-a-deb.deb"
if run_packager "$repo" --desktop-deb "$ROOT/not-a-deb.deb" \
     --desktop-appimage "$ROOT/desktop-good.AppImage" > "$ROOT/notadeb.log" 2>&1; then
  bad "a file that is not a Debian package is refused"
else
  ok "a file that is not a Debian package is refused"
  grep -q "not an ar archive" "$ROOT/notadeb.log" \
    && ok "  and the reason says what it is not" || bad "  and the reason says what it is not"
fi
printf 'not an elf\n' > "$ROOT/not-an-appimage.AppImage"
chmod +x "$ROOT/not-an-appimage.AppImage"
if run_packager "$repo" --desktop-deb "$ROOT/desktop-pair.deb" \
     --desktop-appimage "$ROOT/not-an-appimage.AppImage" > "$ROOT/notapp.log" 2>&1; then
  bad "a file that is not an ELF AppImage is refused"
else
  ok "a file that is not an ELF AppImage is refused"
fi
[ ! -e "$repo/artifacts/release" ] \
  && ok "  and none of those attempts left a delivery behind" \
  || bad "  and none of those attempts left a delivery behind"

echo
echo "================================================================================"
echo "  cases: $CASES   passed: $PASSED   failed: $FAILED"
if [ "$FAILED" -eq 0 ]; then
  echo "RESULT: PASS"
  exit 0
fi
echo "RESULT: FAIL"
exit 1
