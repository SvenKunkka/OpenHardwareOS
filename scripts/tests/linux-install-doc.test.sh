#!/usr/bin/env bash
#
# Fixture tests for the install commands in docs/linux-install.md.
#
#   ./scripts/tests/linux-install-doc.test.sh
#
# Every bash block in the document is extracted **as written** and run against a
# local release fixture served over `file://`, with `sha256sum`, `sudo` and
# `apt-get` shims so the Debian route can be exercised on a machine that is not
# Debian. The check is therefore on the documented route, not on a paraphrase of
# it: if the document's commands stop working, this fails.
#
# One case matters more than the rest. The published checksum list covers *every*
# Linux asset for the version, so a user following the CLI section downloads one
# file out of four. The documented commands must verify the file they downloaded
# and must not fail because of the three they did not — which is exactly how the
# whole-list form broke when the desktop packages joined the list.
#
# What it does NOT prove: that the published release exists, that `apt-get`
# resolves real dependencies, or that the binary runs on a real Linux kernel.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DOC="$ROOT_DIR/docs/linux-install.md"
[ -f "$DOC" ] || { echo "error: cannot find $DOC" >&2; exit 1; }

VERSION="$(grep -m1 -oE '^ohm_version=v[0-9]+\.[0-9]+\.[0-9]+$' "$DOC" | head -1 | cut -d= -f2)"
[ -n "$VERSION" ] || { echo "error: cannot read ohm_version from $DOC" >&2; exit 1; }
PLATFORM="linux-x86_64"
CLI_ASSET="ohm-cli-$VERSION-$PLATFORM.tar.gz"
DEB_ASSET="OpenHardwareOS-$VERSION-$PLATFORM.deb"
IMAGE_ASSET="OpenHardwareOS-$VERSION-$PLATFORM.AppImage"
METADATA="release-$PLATFORM.json"

# The checksum list's name, taken from the document's own commands, so the fixture
# and the document cannot drift apart. It must carry the platform suffix: one
# GitHub Release holds one asset per name, and `SHA256SUMS` is already the Windows
# list, so a bare name would make the Linux route fetch the wrong file.
SUMS_RAW="$(grep -m1 -oE '^ohm_sums="[^"]+"' "$DOC" | sed -e 's/^ohm_sums="//' -e 's/"$//')"
[ -n "$SUMS_RAW" ] || { echo "error: $DOC must set ohm_sums in its install blocks" >&2; exit 1; }
SUMS="${SUMS_RAW//\$ohm_platform/$PLATFORM}"
case "$SUMS" in
  SHA256SUMS-*) ;;
  *) echo "error: $DOC names the Linux checksum list '$SUMS'; it must be platform-suffixed" >&2; exit 1 ;;
esac

WORK="$(cd "$(mktemp -d "${TMPDIR:-/tmp}/ohm-linux-install-XXXXXX")" && pwd)"
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

# --- the blocks, taken from the document ------------------------------------
# Every ```bash block, in the order the page presents them.
BLOCKS="$WORK/blocks"
mkdir -p "$BLOCKS"
python3 - "$DOC" "$BLOCKS" <<'PY'
import pathlib, re, sys
text = pathlib.Path(sys.argv[1]).read_text(encoding="utf-8")
blocks = re.findall(r"```bash\n(.*?)```", text, re.S)
if len(blocks) < 3:
    raise SystemExit(f"expected at least three bash blocks, found {len(blocks)}")
for index, block in enumerate(blocks, start=1):
    pathlib.Path(sys.argv[2], f"block{index}.sh").write_text(block, encoding="utf-8")
print(f"extracted {len(blocks)} blocks")
PY
BLOCK_COUNT="$(ls "$BLOCKS" | wc -l | tr -d ' ')"
[ "$BLOCK_COUNT" -ge 3 ] || { echo "error: could not extract the install blocks" >&2; exit 1; }

# --- shims -------------------------------------------------------------------
# `sha256sum` for hosts that only have `shasum` (macOS); `sudo` and `apt-get` so
# the Debian route can be *run* here. The apt shim records what it was asked to
# install, which is how a case can prove nothing was installed on a refusal.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/sha256sum" <<'SH'
#!/bin/sh
exec shasum -a 256 "$@"
SH
cat > "$WORK/bin/sudo" <<'SH'
#!/bin/sh
exec "$@"
SH
# The cross-check section asks the *installed* CLI for machine-readable readings,
# so the fixture provides one: a shim answering `status --json` with a small, valid
# snapshot. The script then runs for real — on this host, where /proc and /sys do
# not exist, it reports "no platform source" for every reading and exits 0, which is
# the honest outcome the page describes.
cat > "$WORK/bin/ohm-cli" <<'SH'
#!/bin/sh
if [ "${1:-}" = "status" ] && [ "${2:-}" = "--json" ]; then
  cat "$OHM_TEST_READINGS"
fi
exit 0
SH
cat > "$WORK/bin/dpkg" <<'SH'
#!/bin/sh
# `dpkg -l <name>` / `dpkg -s <name>`: the fixture "installed" the package, so
# answer like dpkg would.
case "${1:-}" in
  -l) echo "ii  ${2:-}  0.0.0  amd64  fixture" ;;
  -s) printf 'Package: %s\nStatus: install ok installed\nDepends: libwebkit2gtk-4.1-0, libgtk-3-0\n' "${2:-}" ;;
esac
exit 0
SH
cat > "$WORK/bin/apt-get" <<'SH'
#!/bin/sh
# `apt-get install -y <file.deb>`: record the request and "install" the binary the
# package would have placed, so the documented command that follows can run.
for argument in "$@"; do
  case "$argument" in
    *.deb) printf '%s\n' "$argument" >> "$OHM_TEST_APT_LOG" ;;
  esac
done
if [ "${1:-}" = "install" ]; then
  printf '#!/bin/sh\nif [ "${1:-}" = "--version" ]; then echo "openhardwareos fixture"; fi\nexit 0\n' \
    > "$OHM_TEST_BINDIR/openhardwareos"
  chmod +x "$OHM_TEST_BINDIR/openhardwareos"
fi
exit 0
SH
chmod +x "$WORK/bin/sha256sum" "$WORK/bin/sudo" "$WORK/bin/apt-get" "$WORK/bin/dpkg" "$WORK/bin/ohm-cli"
export OHM_TEST_READINGS="$WORK/readings.json"
python3 - "$OHM_TEST_READINGS" <<'PYINNER'
import json, sys

def device(device_id, name, kind, readings):
    return {"device": {"id": device_id, "name": name, "type": kind, "adapter": "system",
                       "transport": "system", "model": None, "vendor": None,
                       "capabilities": [], "metadata": {}},
            "enabled": True, "status": "online",
            "state": {"device": device_id, "timestamp_ms": 1, "online": True,
                      "readings": readings},
            "adapter": "system", "first_seen_ms": 1, "last_seen_ms": 2}

# One reading a host can compare (memory) and one it usually cannot (a fan channel),
# so the cross-check has something to agree about and something to explain.
json.dump({"generated_at_ms": 1, "started_at_ms": 0, "adapters": [], "settings": {},
           "stats": {}, "has_controllable_hardware": False,
           "devices": [
               device("memory.system.0", "Memory", "memory",
                      [{"capability": "memory.total", "status": "ok",
                        "value": 17179869184}]),
               device("fan.system.nct6798d_fan1", "Chassis fan", "fan",
                      [{"capability": "fan.rpm", "status": "ok", "value": 1245.0}]),
           ]}, open(sys.argv[1], "w", encoding="utf-8"))
PYINNER
export OHM_TEST_APT_LOG="$WORK/apt.log"
export OHM_TEST_BINDIR="$WORK/bin"
: > "$OHM_TEST_APT_LOG"

# --- a local release fixture -------------------------------------------------
build_fixture() { # directory, tamper (none|cli|deb)
  local dir="$1" tamper="${2:-none}"
  rm -rf "$dir"; mkdir -p "$dir/stage"
  cat > "$dir/stage/ohm-cli" <<SH
#!/bin/sh
case "\$1" in
  --version) echo "ohm-cli ${VERSION#v}" ;;
  doctor) echo "doctor: simulated fixture"; exit 0 ;;
  *) exit 0 ;;
esac
SH
  chmod +x "$dir/stage/ohm-cli"
  printf 'Apache License 2.0 fixture\n' > "$dir/stage/LICENSE"
  printf 'notices fixture\n' > "$dir/stage/THIRD_PARTY_NOTICES.txt"
  printf '{"version":"%s","platform":"%s"}\n' "$VERSION" "$PLATFORM" > "$dir/stage/$METADATA"
  tar -czf "$dir/$CLI_ASSET" -C "$dir/stage" ohm-cli LICENSE THIRD_PARTY_NOTICES.txt "$METADATA"
  cp "$dir/stage/$METADATA" "$dir/"
  # The desktop route hands the file to apt, so the fixture needs the name and the
  # bytes rather than a real Debian package; the AppImage route executes it.
  printf 'not really a Debian package\n' > "$dir/$DEB_ASSET"
  cat > "$dir/$IMAGE_ASSET" <<'SH'
#!/bin/sh
echo "appimage fixture: selftest report, simulated hardware"
exit 0
SH
  chmod +x "$dir/$IMAGE_ASSET"
  # The list covers every Linux asset of the version — four of them — while each
  # documented route downloads only what it needs.
  {
    printf '%s  %s\n' "$(shasum -a 256 "$dir/$CLI_ASSET" | cut -d' ' -f1)" "$CLI_ASSET"
    printf '%s  %s\n' "$(shasum -a 256 "$dir/$METADATA" | cut -d' ' -f1)" "$METADATA"
    printf '%s  %s\n' "$(shasum -a 256 "$dir/$DEB_ASSET" | cut -d' ' -f1)" "$DEB_ASSET"
    printf '%s  %s\n' "$(shasum -a 256 "$dir/$IMAGE_ASSET" | cut -d' ' -f1)" "$IMAGE_ASSET"
  } > "$dir/$SUMS"
  case "$tamper" in
    cli) printf 'tampered' >> "$dir/$CLI_ASSET" ;;
    deb) printf 'tampered' >> "$dir/$DEB_ASSET" ;;
    none) ;;
  esac
}

# Point the document's download root at the fixture. The scheme is added here,
# once: passing it in as well produced `file://file://…`, which curl rejects.
# Nothing else is rewritten: the blocks install where the page says they do, and
# the page's later snippets use what the earlier ones installed. A block is run
# with its own HOME per case, so cases cannot see each other's installs.
scoped_block() { # fixture dir, block file, output file
  sed -e "s|^ohm_base=.*|ohm_base=\"file://$1\"|" "$2" > "$3"
}

CASE_HOME=""
run_block() { # block file
  PATH="$WORK/bin:$PATH" HOME="$CASE_HOME" bash "$1" > "$WORK/last.log" 2>&1
}

echo "================================================================================"
echo " OpenHardwareOS — documented Linux install routes (FIXTURES ONLY)"
echo "================================================================================"
echo " document : docs/linux-install.md (version $VERSION, list $SUMS)"
echo " blocks   : $BLOCK_COUNT bash blocks, run as written"
echo " work     : $WORK"
echo
echo " The release is a local fixture. This proves the documented commands work and"
echo " refuse tampering; it does not prove the published release exists."

# ---------------------------------------------------------------- case 1
echo
echo "--- every documented route installs from its own download, not from the whole list"
CASES=$((CASES + 1))
fixture="$WORK/release-good"
build_fixture "$fixture"
: > "$OHM_TEST_APT_LOG"
CASE_HOME="$WORK/home-good"
mkdir -p "$CASE_HOME"

for block in "$BLOCKS"/block*.sh; do
  index="$(basename "$block" .sh)"
  scoped_block "$fixture" "$block" "$WORK/$index.scoped.sh"
  if run_block "$WORK/$index.scoped.sh"; then
    ok "$index exits 0"
  else
    bad "$index exits 0 ($(tail -1 "$WORK/last.log"))"
  fi
  # The whole-list form must not be what the page tells users to run: it fails
  # whenever they did not download the other assets.
  if grep -qE 'sha256sum -c "\$ohm_sums"' "$block"; then
    bad "$index does not verify a partial download against the whole list"
  else
    ok "$index does not verify a partial download against the whole list"
  fi
done

installed_version="$("$CASE_HOME/.local/share/OpenHardwareOS/cli-$VERSION/ohm-cli" --version 2>/dev/null || echo none)"
check "the CLI is installed and reports $VERSION (said: $installed_version)" \
  "$([ "$installed_version" = "ohm-cli ${VERSION#v}" ] && echo 1 || echo 0)"
grep -q "$DEB_ASSET" "$OHM_TEST_APT_LOG" \
  && ok "the .deb route hands the downloaded package to apt" \
  || bad "the .deb route hands the downloaded package to apt"

# ---------------------------------------------------------------- case 2
echo
echo "--- a tampered CLI archive is refused before anything is extracted"
CASES=$((CASES + 1))
fixture="$WORK/release-tampered-cli"
build_fixture "$fixture" cli
CASE_HOME="$WORK/home-tampered"; mkdir -p "$CASE_HOME"
scoped_block "$fixture" "$BLOCKS/block1.sh" "$WORK/tampered.sh"
if run_block "$WORK/tampered.sh"; then
  bad "the block fails on a checksum mismatch"
else
  ok "the block fails on a checksum mismatch"
  grep -qi "FAILED\|does not match" "$WORK/last.log" \
    && ok "  and the checksum tool said so" || bad "  and the checksum tool said so"
fi
[ ! -e "$CASE_HOME/.local/share/OpenHardwareOS/cli-$VERSION" ] \
  && ok "nothing was installed" || bad "nothing was installed"

# ---------------------------------------------------------------- case 3
echo
echo "--- a tampered .deb is refused before apt is asked to install it"
CASES=$((CASES + 1))
fixture="$WORK/release-tampered-deb"
build_fixture "$fixture" deb
: > "$OHM_TEST_APT_LOG"
CASE_HOME="$WORK/home-tampered-deb"; mkdir -p "$CASE_HOME"
scoped_block "$fixture" "$BLOCKS/block3.sh" "$WORK/tampered-deb.sh"
if run_block "$WORK/tampered-deb.sh"; then
  bad "the desktop block fails on a checksum mismatch"
else
  ok "the desktop block fails on a checksum mismatch"
fi
[ ! -s "$OHM_TEST_APT_LOG" ] \
  && ok "  and apt was never asked to install anything" \
  || bad "  and apt was never asked to install anything"

# ---------------------------------------------------------------- case 4
echo
echo "--- a checksum list with two matching entries is refused"
CASES=$((CASES + 1))
fixture="$WORK/release-duplicate"
build_fixture "$fixture"
first="$(grep "$CLI_ASSET" "$fixture/$SUMS")"
printf '%s\n' "$first" >> "$fixture/$SUMS"
CASE_HOME="$WORK/home-duplicate"; mkdir -p "$CASE_HOME"
scoped_block "$fixture" "$BLOCKS/block1.sh" "$WORK/duplicate.sh"
if run_block "$WORK/duplicate.sh"; then
  bad "the block refuses a duplicated entry"
else
  ok "the block refuses a duplicated entry"
  grep -q "应有且仅有一条" "$WORK/last.log" && ok "  and says what it expected" || bad "  and says what it expected"
fi
[ ! -e "$CASE_HOME/.local/share/OpenHardwareOS/cli-$VERSION" ] \
  && ok "nothing was installed" || bad "nothing was installed"

# ---------------------------------------------------------------- case 5
echo
echo "--- an existing install directory is preserved"
CASES=$((CASES + 1))
fixture="$WORK/release-existing"
build_fixture "$fixture"
CASE_HOME="$WORK/home-existing"; mkdir -p "$CASE_HOME"
mkdir -p "$CASE_HOME/.local/share/OpenHardwareOS/cli-$VERSION"
printf 'keep me\n' > "$CASE_HOME/.local/share/OpenHardwareOS/cli-$VERSION/ohm-cli"
scoped_block "$fixture" "$BLOCKS/block1.sh" "$WORK/existing.sh"
if run_block "$WORK/existing.sh"; then
  bad "the block refuses to overwrite an existing install"
else
  ok "the block refuses to overwrite an existing install"
fi
check "the existing file is untouched" \
  "$([ "$(cat "$CASE_HOME/.local/share/OpenHardwareOS/cli-$VERSION/ohm-cli")" = "keep me" ] && echo 1 || echo 0)"

# ---------------------------------------------------------------- case 6
echo
echo "--- the page says what the list covers, and what was not verified"
CASES=$((CASES + 1))
check "the page says the list covers the whole version" \
  "$(grep -q '覆盖整版 Linux 产物' "$DOC" && echo 1 || echo 0)"
check "the page names what is not verified" \
  "$(grep -q '没有验证的' "$DOC" && echo 1 || echo 0)"
check "the page documents the AppImage route" \
  "$(grep -q 'AppImage（免安装）' "$DOC" && echo 1 || echo 0)"
# One page, one version. A copy-paste snippet left pointing at the previous
# release tells the reader to install something that is not what the page is
# about — this suite found exactly that in the PATH hint.
versions="$(grep -oE 'v[0-9]+\.[0-9]+\.[0-9]+' "$DOC" | sort -u | tr '\n' ' ')"
check "every version on the page is $VERSION (found: $versions)" \
  "$([ "$versions" = "$VERSION " ] && echo 1 || echo 0)"

echo
echo "================================================================================"
echo "  cases: $CASES   passed: $PASSED   failed: $FAILED"
if [ "$FAILED" -eq 0 ]; then
  echo "RESULT: PASS"
  exit 0
fi
echo "RESULT: FAIL"
exit 1
