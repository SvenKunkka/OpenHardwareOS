#!/usr/bin/env bash
#
# Package the Linux CLI build for a release.
#
#   ./scripts/release/package-linux.sh --version v0.1.3 --binary target/x86_64-unknown-linux-gnu/release/ohm-cli
#
# The same rules the Windows packager follows, for the same reasons:
#
#   * refuse an output directory that already exists, instead of writing into a
#     previous release;
#   * take the source commit from git, so the metadata names the revision the
#     build came from;
#   * write SHA256SUMS with **LF** — a CRLF list makes `shasum -c` report every
#     entry as a missing file on the very platform this package targets;
#   * verify before delivering: the archive must contain the binary, the sums
#     must describe the files that are there, and the list must be readable by
#     POSIX tooling.
#
# `--run-binary` additionally executes the packaged CLI and checks its version.
# Only the Linux CI job passes it; the fixture test runs everywhere and checks
# everything else.
set -euo pipefail

VERSION=""
BINARY=""
OUT="artifacts/release"
RUN_BINARY=0
PLATFORM="linux-x86_64"
# A GitHub Release holds one asset per name, and the tooling verifies the asset
# called exactly `release.json` — which the Windows build already publishes — so
# this one is named for its platform.
METADATA="release-$PLATFORM.json"

die() { echo "error: $*" >&2; exit 1; }

# `sha256sum` on Linux (coreutils), `shasum` where that is what exists (macOS,
# and the fixture tests). Both print `<digest>  <name>`.
hash_file() {
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum "$1"
  else
    shasum -a 256 "$1"
  fi
}

verify_sums() { # run in a directory holding SHA256SUMS and the files it names
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c SHA256SUMS
  else
    shasum -a 256 -c SHA256SUMS
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --binary) BINARY="${2:-}"; shift 2 ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --platform) PLATFORM="${2:-}"; shift 2 ;;
    --run-binary) RUN_BINARY=1; shift ;;
    -h|--help) sed -n '2,25p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ] || die "--version is required"
[ -n "$BINARY" ] || die "--binary is required"
printf '%s' "$VERSION" | grep -Eq '^v[0-9]+\.[0-9]+\.[0-9]+$' \
  || die "expected vMAJOR.MINOR.PATCH, got '$VERSION'"

[ -f "$BINARY" ] || die "no binary at $BINARY"
[ -s "$BINARY" ] || die "the binary at $BINARY is empty"

# --- the binary must be a Linux x86_64 ELF, not something else --------------
# 0x7f 'E' 'L' 'F', class 2 = 64-bit, machine 0x3e = x86_64 (little endian).
magic=$(od -An -tx1 -N4 "$BINARY" | tr -d ' \n')
class=$(od -An -tu1 -j4 -N1 "$BINARY" | tr -d ' \n')
machine=$(od -An -tx1 -j18 -N2 "$BINARY" | tr -d ' \n')
[ "$magic" = "7f454c46" ] || die "the binary at $BINARY is not an ELF file (magic $magic)"
[ "$class" = "2" ] || die "the binary at $BINARY is not 64-bit (ELF class $class)"
[ "$machine" = "3e00" ] || die "the binary at $BINARY is not x86_64 (ELF machine $machine)"

[ ! -e "$OUT" ] || die "output already exists; use a clean release build: $OUT"

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO"

SHA="$(git rev-parse HEAD)"
printf '%s' "$SHA" | grep -Eq '^[0-9a-f]{40}$' || die "cannot read the release source commit"

for required in LICENSE THIRD_PARTY_NOTICES.txt; do
  [ -s "$REPO/$required" ] || die "$required is missing; generate the dependency notices first"
done

mkdir -p "$OUT"
STAGE="$OUT/.stage-$$"
mkdir -p "$STAGE"
cleanup() { rm -rf "$STAGE"; }
trap cleanup EXIT

cp "$BINARY" "$STAGE/ohm-cli"
cp "$REPO/LICENSE" "$STAGE/LICENSE"
cp "$REPO/THIRD_PARTY_NOTICES.txt" "$STAGE/THIRD_PARTY_NOTICES.txt"
chmod 755 "$STAGE/ohm-cli"

{
  printf '{\n'
  printf '  "version": "%s",\n' "$VERSION"
  printf '  "platform": "%s",\n' "$PLATFORM"
  printf '  "repository": "SvenKunkka/OpenHardwareOS",\n'
  printf '  "source_commit": "%s",\n' "$SHA"
  printf '  "rust": "%s",\n' "$(rustc --version 2>/dev/null || echo unknown)"
  printf '  "workflow_run": "%s"\n' "${GITHUB_RUN_ID:-local}"
  printf '}\n'
} > "$STAGE/$METADATA"

ARCHIVE="$OUT/ohm-cli-$VERSION-$PLATFORM.tar.gz"
METADATA_PATH="$OUT/$METADATA"
cp "$STAGE/$METADATA" "$METADATA_PATH"
tar -czf "$ARCHIVE" -C "$STAGE" ohm-cli LICENSE THIRD_PARTY_NOTICES.txt "$METADATA"

# --- verify before delivering ----------------------------------------------
tar -tzf "$ARCHIVE" | grep -qx 'ohm-cli' || die "the archive does not contain ohm-cli"
tar -tzf "$ARCHIVE" | grep -qx "$METADATA" || die "the archive does not contain $METADATA"

if [ "$RUN_BINARY" = "1" ]; then
  reported=$("$BINARY" --version)
  expected="ohm-cli ${VERSION#v}"
  [ "$reported" = "$expected" ] \
    || die "the binary reports '$reported', expected '$expected'"
  echo "binary reports: $reported"
fi

# LF only, and every entry must describe a file that exists here.
{
  for file in "$ARCHIVE" "$METADATA_PATH" "$REPO/LICENSE" "$REPO/THIRD_PARTY_NOTICES.txt"; do
    printf '%s  %s\n' "$(hash_file "$file" | cut -d' ' -f1)" "$(basename "$file")"
  done
} > "$OUT/SHA256SUMS.tmp"
if grep -q $'\r' "$OUT/SHA256SUMS.tmp"; then
  die "SHA256SUMS would contain a carriage return; shasum -c could not read it"
fi
mv "$OUT/SHA256SUMS.tmp" "$OUT/SHA256SUMS"

# Check the list the way the target platform will: line by line, by name.
SIDE="$OUT/.side-$$"
mkdir -p "$SIDE"
cp "$ARCHIVE" "$SIDE/"
cp "$METADATA_PATH" "$SIDE/"
cp "$REPO/LICENSE" "$SIDE/"
cp "$REPO/THIRD_PARTY_NOTICES.txt" "$SIDE/"
cp "$OUT/SHA256SUMS" "$SIDE/"
( cd "$SIDE" && verify_sums >/dev/null ) \
  || die "the checksum list does not verify against the files it names"
rm -rf "$SIDE"

echo "source commit : $SHA"
echo "archive       : $ARCHIVE"
ls -l "$ARCHIVE" | awk '{ print "archive size  : " $5 " bytes" }'
echo "SHA256SUMS    :"
sed 's/^/  /' "$OUT/SHA256SUMS"
