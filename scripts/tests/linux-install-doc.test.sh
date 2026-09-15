#!/usr/bin/env bash
#
# Fixture tests for the install commands documented in docs/linux-install.md.
#
#   ./scripts/tests/linux-install-doc.test.sh
#
# The block is extracted from the document itself and run against a local release
# fixture (served through `file://`), with a `sha256sum` shim for hosts that only
# have `shasum`. So the check is on the documented route, not on a paraphrase of
# it: if the document's commands stop working, this fails.
#
# What it does NOT prove: that the published release exists, or that the real
# binary runs on a real Linux kernel. Those need the release and a Linux machine.
set -uo pipefail

ROOT_DIR="$(cd "$(dirname "$0")/../.." && pwd)"
DOC="$ROOT_DIR/docs/linux-install.md"
[ -f "$DOC" ] || { echo "error: cannot find $DOC" >&2; exit 1; }

VERSION="$(grep -m1 -oE '`v[0-9]+\.[0-9]+\.[0-9]+`' "$DOC" | tr -d '`')"
[ -n "$VERSION" ] || { echo "error: cannot read the version from $DOC" >&2; exit 1; }
PLATFORM="linux-x86_64"
ASSET="ohm-cli-$VERSION-$PLATFORM.tar.gz"

# `mktemp` under a TMPDIR that ends in a slash yields a path with `//` in it, and
# curl rejects a file:// URL whose path contains that.
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

# --- the block, taken from the document -------------------------------------
BLOCK="$WORK/block.sh"
python3 - "$DOC" "$BLOCK" <<'PY'
import re, sys
text = open(sys.argv[1], encoding="utf-8").read()
match = re.search(r"```bash\n(.*?)```", text, re.S)
if not match:
    raise SystemExit("no bash block in the document")
open(sys.argv[2], "w", encoding="utf-8").write(match.group(1))
PY
[ -s "$BLOCK" ] || { echo "error: could not extract the install block" >&2; exit 1; }

# A `sha256sum` for hosts that only have `shasum` (macOS); Linux has both.
mkdir -p "$WORK/bin"
cat > "$WORK/bin/sha256sum" <<'SH'
#!/bin/sh
exec shasum -a 256 "$@"
SH
chmod +x "$WORK/bin/sha256sum"

# --- a local release fixture -------------------------------------------------
build_fixture() { # directory, tamper?
  local dir="$1" tamper="${2:-no}"
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
  printf '{"version":"%s","platform":"%s"}\n' "$VERSION" "$PLATFORM" > "$dir/stage/release-$PLATFORM.json"
  tar -czf "$dir/$ASSET" -C "$dir/stage" ohm-cli LICENSE THIRD_PARTY_NOTICES.txt "release-$PLATFORM.json"
  # The list is written from the pristine archive; only then is a tampered
  # fixture made, so the checksum no longer describes the bytes on disk — which
  # is the case the documented commands have to refuse.
  {
    printf '%s  %s\n' "$(shasum -a 256 "$dir/$ASSET" | cut -d' ' -f1)" "$ASSET"
    printf '%s  %s\n' "$(shasum -a 256 "$dir/stage/LICENSE" | cut -d' ' -f1)" "LICENSE"
    printf '%s  %s\n' "$(shasum -a 256 "$dir/stage/THIRD_PARTY_NOTICES.txt" | cut -d' ' -f1)" "THIRD_PARTY_NOTICES.txt"
  } > "$dir/SHA256SUMS"
  if [ "$tamper" = "yes" ]; then
    printf 'tampered' >> "$dir/$ASSET"
  fi
}

# The block, pointed at the fixture directory instead of GitHub. The scheme is
# added here, once: passing it in as well produced `file://file://…`, which curl
# rejects as a bad file URL — which is how this test failed the first time.
scoped_block() { # fixture dir, install dir, output file
  sed -e "s|^ohm_base=.*|ohm_base=\"file://$1\"|" \
      -e "s|^ohm_dir=.*|ohm_dir=\"$2\"|" "$BLOCK" > "$3"
}

run_block() { # block file
  PATH="$WORK/bin:$PATH" HOME="$WORK/home" bash "$1" > "$WORK/last.log" 2>&1
}

echo "================================================================================"
echo " OpenHardwareOS — documented Linux install route (FIXTURES ONLY)"
echo "================================================================================"
echo " document : docs/linux-install.md (version $VERSION)"
echo " work     : $WORK"
echo
echo " The release is a local fixture. This proves the documented commands work and"
echo " refuse tampering; it does not prove the published release exists."

# ---------------------------------------------------------------- case 1
echo
echo "--- the documented commands install the fixture release"
CASES=$((CASES + 1))
fixture="$WORK/release-good"
build_fixture "$fixture"
scoped_block "$fixture" "$WORK/install-good" "$WORK/good.sh"
if run_block "$WORK/good.sh"; then
  ok "the block exits 0"
else
  bad "the block exits 0 ($(tail -1 "$WORK/last.log"))"
fi
[ -x "$WORK/install-good/ohm-cli" ] && ok "the CLI is installed" || bad "the CLI is installed"
installed_version="$("$WORK/install-good/ohm-cli" --version 2>/dev/null || echo none)"
check "the installed CLI reports $VERSION (said: $installed_version)" \
  "$([ "$installed_version" = "ohm-cli ${VERSION#v}" ] && echo 1 || echo 0)"
grep -q "doctor" "$WORK/last.log" && ok "  and doctor ran" || bad "  and doctor ran"

# ---------------------------------------------------------------- case 2
echo
echo "--- a tampered archive is refused before anything is extracted"
CASES=$((CASES + 1))
fixture="$WORK/release-tampered"
build_fixture "$fixture" yes
scoped_block "$fixture" "$WORK/install-tampered" "$WORK/tampered.sh"
if run_block "$WORK/tampered.sh"; then
  bad "the block fails on a checksum mismatch"
else
  ok "the block fails on a checksum mismatch"
  grep -qi "FAILED\|does not match\|checksum" "$WORK/last.log" \
    && ok "  and the checksum tool said so" || bad "  and the checksum tool said so"
fi
[ ! -e "$WORK/install-tampered/ohm-cli" ] && ok "nothing was installed" || bad "nothing was installed"

# ---------------------------------------------------------------- case 3
echo
echo "--- a checksum list with two matching entries is refused"
CASES=$((CASES + 1))
fixture="$WORK/release-duplicate"
build_fixture "$fixture"
first="$(grep "$ASSET" "$fixture/SHA256SUMS")"
printf '%s\n' "$first" >> "$fixture/SHA256SUMS"
scoped_block "$fixture" "$WORK/install-duplicate" "$WORK/duplicate.sh"
if run_block "$WORK/duplicate.sh"; then
  bad "the block refuses a duplicated entry"
else
  ok "the block refuses a duplicated entry"
  grep -q "应有且仅有一条" "$WORK/last.log" && ok "  and says what it expected" || bad "  and says what it expected"
fi
[ ! -e "$WORK/install-duplicate/ohm-cli" ] && ok "nothing was installed" || bad "nothing was installed"

# ---------------------------------------------------------------- case 4
echo
echo "--- an existing install directory is preserved"
CASES=$((CASES + 1))
fixture="$WORK/release-existing"
build_fixture "$fixture"
mkdir -p "$WORK/install-existing"
printf 'keep me\n' > "$WORK/install-existing/ohm-cli"
scoped_block "$fixture" "$WORK/install-existing" "$WORK/existing.sh"
if run_block "$WORK/existing.sh"; then
  bad "the block refuses to overwrite an existing install"
else
  ok "the block refuses to overwrite an existing install"
fi
check "the existing file is untouched" \
  "$([ "$(cat "$WORK/install-existing/ohm-cli")" = "keep me" ] && echo 1 || echo 0)"

echo
echo "================================================================================"
echo "  cases: $CASES   passed: $PASSED   failed: $FAILED"
if [ "$FAILED" -eq 0 ]; then
  echo "RESULT: PASS"
  exit 0
fi
echo "RESULT: FAIL"
exit 1
