#!/usr/bin/env bash
#
# Package the Linux delivery for a release: the CLI, and (when they are given)
# the desktop bundles.
#
#   ./scripts/release/package-linux.sh --version v0.1.3 --binary target/release/ohm-cli
#   ./scripts/release/package-linux.sh --version v0.1.4 --binary target/release/ohm-cli \
#       --desktop-deb target/release/bundle/deb/OpenHardwareOS_0.1.4_amd64.deb \
#       --desktop-appimage target/release/bundle/appimage/OpenHardwareOS_0.1.4_amd64.AppImage
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
#     POSIX tooling;
#   * list only the assets that are actually published beside the list, so a
#     plain `sha256sum -c` on a download is a check of the download and not a
#     report of files that were never meant to be there.
#
# One list covers everything Linux publishes, the desktop packages included: a
# user who downloads two files wants one command that answers "is this download
# intact", not one list per artefact kind.
#
# The desktop `.deb` is read with `scripts/release/inspect-deb.py` (an `ar`
# archive holding `control.tar.*` and `data.tar.*`) and checked for the facts a
# user depends on: the declared package name, version and architecture, a real
# x86_64 ELF at `usr/bin/...`, a desktop entry that launches it, and icons. A
# `.deb` that dpkg would install as something else is not published.
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
DESKTOP_DEB=""
DESKTOP_APPIMAGE=""
# A GitHub Release holds one asset per name, and the tooling verifies the asset
# called exactly `release.json` — which the Windows build already publishes — so
# this one is named for its platform.
METADATA="release-$PLATFORM.json"
# The checksum list too: a Release holds one asset per name, and `SHA256SUMS`
# belongs to the Windows assets. The documented Linux commands fetch this name.
SUMS="SHA256SUMS-$PLATFORM"

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

verify_sums() { # run in a directory holding the list and the files it names
  if command -v sha256sum >/dev/null 2>&1; then
    sha256sum -c "$1"
  else
    shasum -a 256 -c "$1"
  fi
}

while [ $# -gt 0 ]; do
  case "$1" in
    --version) VERSION="${2:-}"; shift 2 ;;
    --binary) BINARY="${2:-}"; shift 2 ;;
    --out) OUT="${2:-}"; shift 2 ;;
    --platform) PLATFORM="${2:-}"; shift 2 ;;
    --desktop-deb) DESKTOP_DEB="${2:-}"; shift 2 ;;
    --desktop-appimage) DESKTOP_APPIMAGE="${2:-}"; shift 2 ;;
    --run-binary) RUN_BINARY=1; shift ;;
    -h|--help) sed -n '2,32p' "$0"; exit 0 ;;
    *) die "unknown argument: $1" ;;
  esac
done

[ -n "$VERSION" ] || die "--version is required"
[ -n "$BINARY" ] || die "--binary is required"
[[ "$VERSION" =~ ^v[0-9]+\.[0-9]+\.[0-9]+$ ]] || die "expected vMAJOR.MINOR.PATCH, got '$VERSION'"

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

# The two desktop bundles are a pair: a release that publishes one and not the
# other would leave the install page describing an artefact nobody can download.
if [ -n "$DESKTOP_DEB" ] || [ -n "$DESKTOP_APPIMAGE" ]; then
  [ -n "$DESKTOP_DEB" ] || die "--desktop-appimage needs --desktop-deb as well"
  [ -n "$DESKTOP_APPIMAGE" ] || die "--desktop-deb needs --desktop-appimage as well"
  [ -f "$DESKTOP_DEB" ] || die "no desktop package at $DESKTOP_DEB"
  [ -f "$DESKTOP_APPIMAGE" ] || die "no AppImage at $DESKTOP_APPIMAGE"
fi

[ ! -e "$OUT" ] || die "output already exists; use a clean release build: $OUT"

REPO="$(cd "$(dirname "$0")/../.." && pwd)"
cd "$REPO"

SHA="$(git rev-parse HEAD)"
[[ "$SHA" =~ ^[0-9a-f]{40}$ ]] || die "cannot read the release source commit"

[ -s "$REPO/LICENSE" ] || die "LICENSE is missing"
# The generator writes the notices here; the Windows packager reads the same path.
NOTICES="$REPO/artifacts/THIRD_PARTY_NOTICES.txt"
[ -s "$NOTICES" ] || die "$NOTICES is missing; generate the dependency notices first"
[ -n "$(head -c 1 "$NOTICES")" ] || die "$NOTICES is empty"

mkdir -p "$OUT"
STAGE="$OUT/.stage-$$"
mkdir -p "$STAGE"
COMPLETED=0
cleanup() {
  rm -rf "$STAGE"
  # A refused package must not leave a directory that looks like a delivery. The
  # packager refuses to write into an existing output — that is what stops one
  # release from overwriting another — so a half-written one left behind by a
  # failure turns a fixable mistake into a permanent one: the corrected rerun
  # refuses to start. Everything under $OUT was created by this run, because the
  # run refuses to begin when it already exists.
  if [ "$COMPLETED" != "1" ] && [ -n "$OUT" ] && [ -d "$OUT" ]; then
    echo "note: removing the incomplete delivery at $OUT" >&2
    rm -rf "$OUT"
  fi
}
trap cleanup EXIT

cp "$BINARY" "$STAGE/ohm-cli"
cp "$REPO/LICENSE" "$STAGE/LICENSE"
cp "$NOTICES" "$STAGE/THIRD_PARTY_NOTICES.txt"
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
# The listing goes to a file: `tar -tzf … | grep -q` lets grep exit as soon as it
# matches, closing the pipe; GNU tar takes that as a write error, and `pipefail`
# then fails a check that actually passed. (That is exactly how the first Linux
# release build failed.)
LIST="$OUT/.listing-$$"
tar -tzf "$ARCHIVE" > "$LIST" || die "the archive cannot be listed"
grep -qx 'ohm-cli' "$LIST" || die "the archive does not contain ohm-cli"
grep -qx "$METADATA" "$LIST" || die "the archive does not contain $METADATA"
grep -qx 'LICENSE' "$LIST" || die "the archive does not contain LICENSE"
grep -qx 'THIRD_PARTY_NOTICES.txt' "$LIST" || die "the archive does not contain the dependency notices"
rm -f "$LIST"

# --- the desktop bundles, when they were given ------------------------------
# Checked here rather than trusted because the file that the install page tells a
# user to run is the one thing they cannot inspect themselves before installing.
DESKTOP_DEB_OUT=""
DESKTOP_APPIMAGE_OUT=""
if [ -n "$DESKTOP_DEB" ]; then
  INSPECT="$REPO/scripts/release/inspect-deb.py"
  [ -f "$INSPECT" ] || die "missing $INSPECT"
  REPORT="$OUT/.deb-report-$$.json"
  python3 "$INSPECT" "$DESKTOP_DEB" > "$REPORT" \
    || die "the desktop package at $DESKTOP_DEB cannot be read as a Debian package"

  read_field() { # JSON report, dotted path — printed by the small reader below
    python3 - "$REPORT" "$1" <<'PY_READ'
import json, sys
report = json.load(open(sys.argv[1], encoding="utf-8"))
node = report
for part in sys.argv[2].split("."):
    if isinstance(node, list):
        node = node[0] if node else None
    if not isinstance(node, dict) or part not in node:
        print("")
        raise SystemExit(0)
    node = node[part]
print("" if node is None else node)
PY_READ
  }

  declared_version="$(read_field 'control.Version')"
  declared_package="$(read_field 'control.Package')"
  declared_arch="$(read_field 'control.Architecture')"
  declared_depends="$(read_field 'control.Depends')"
  deb_binary="$(read_field 'binary')"
  deb_machine="$(read_field 'binaries.elf.machine')"
  deb_class="$(read_field 'binaries.elf.class')"
  deb_desktop="$(read_field 'desktop_entry')"
  deb_exec="$(read_field 'desktop_fields.Exec')"
  deb_icon="$(read_field 'desktop_fields.Icon')"
  deb_icons="$(read_field 'icons')"

  # The installed binary's name is not this script's choice. `mainBinaryName` in
  # tauri.conf.json is what the bundler uses, and the Linux install page tells users
  # to run `/usr/bin/openhardwareos`. Nothing used to connect the two: this comment
  # claimed the relationship while no check enforced it, so a rename would have
  # shipped a package whose binary no documented command could find — the same
  # defect that had silently disabled the desktop IPC round trip since v0.1.4.
  DESKTOP_CONFIG="$REPO/apps/desktop/src-tauri/tauri.conf.json"
  [ -f "$DESKTOP_CONFIG" ] || die "missing $DESKTOP_CONFIG"
  declared_binary_name="$(python3 - "$DESKTOP_CONFIG" <<'PY_NAME'
import json, sys
config = json.load(open(sys.argv[1], encoding="utf-8"))
name = config.get("mainBinaryName")
if not name:
    raise SystemExit("tauri.conf.json declares no mainBinaryName")
print(name)
PY_NAME
)" || die "cannot read mainBinaryName from $DESKTOP_CONFIG"

  # Print the facts before judging them: a refusal in the release log then shows
  # *what* the package declared, not only which sentence rejected it.
  echo "desktop package : $declared_package $declared_version ($declared_arch)"
  echo "desktop binary  : $deb_binary ($deb_class $deb_machine)"
  echo "desktop name    : mainBinaryName declares '$declared_binary_name'"
  echo "desktop entry   : $deb_desktop -> $deb_exec"
  echo "desktop icons   : $deb_icons"
  echo "desktop depends : $declared_depends"

  [ "$declared_version" = "${VERSION#v}" ] \
    || die "the desktop package declares version '$declared_version', expected '${VERSION#v}'"
  # The bundler derives the Debian package name from `productName`
  # ("OpenHardwareOS" -> "open-hardware-os"); dpkg requires lower case. Enforced
  # rather than accepted: the install page tells users what to remove, and a
  # bundler upgrade that renames the package would silently break that sentence.
  # The installed binary is `openhardwareos` because `mainBinaryName` says so.
  [ "$declared_package" = "open-hardware-os" ] \
    || die "the desktop package is named '$declared_package', expected 'open-hardware-os'"
  case "$declared_arch" in
    amd64|x86_64) ;;
    *) die "the desktop package is built for '$declared_arch', expected amd64" ;;
  esac
  [ -n "$declared_depends" ] || die "the desktop package declares no dependencies"
  [ -n "$deb_binary" ] || die "the desktop package installs no ELF binary under usr/bin"
  [ "$deb_binary" = "usr/bin/$declared_binary_name" ] \
    || die "the desktop package installs '$deb_binary', but mainBinaryName declares '$declared_binary_name' (expected 'usr/bin/$declared_binary_name')"
  [ "$deb_class" = "64-bit" ] || die "the desktop binary is $deb_class, expected 64-bit"
  [ "$deb_machine" = "x86_64" ] || die "the desktop binary is for $deb_machine, expected x86_64"
  [ -n "$deb_desktop" ] || die "the desktop package installs no .desktop entry"
  [ -n "$deb_exec" ] || die "the desktop entry launches nothing"
  # A freedesktop Exec line may carry arguments and field codes (`%U`, `%F`), and
  # may be quoted; the thing that must match is the program it launches, not the
  # rest of the line.
  deb_program="$(printf '%s' "$deb_exec" | awk '{print $1}' | tr -d '"'"'"'')"
  [ "$deb_program" = "$(basename "$deb_binary")" ] \
    || die "the desktop entry runs '$deb_program' but the package installs '$(basename "$deb_binary")'"
  [ -n "$deb_icon" ] || die "the desktop entry names no icon"
  [ -n "$deb_icons" ] || die "the desktop package installs no icon files"
  rm -f "$REPORT"

  # Published under this project's own names, not the bundler's
  # (`OpenHardwareOS_0.1.4_amd64.deb`): the install page has to name a file that
  # does not change shape when the bundler changes its mind, and both platforms
  # then follow the same pattern as the CLI archive.
  DESKTOP_DEB_OUT="$OUT/OpenHardwareOS-$VERSION-$PLATFORM.deb"
  cp "$DESKTOP_DEB" "$DESKTOP_DEB_OUT"

  # The AppImage is the same application in one file: an ELF that mounts itself.
  magic=$(od -An -tx1 -N4 "$DESKTOP_APPIMAGE" | tr -d ' \n')
  class=$(od -An -tu1 -j4 -N1 "$DESKTOP_APPIMAGE" | tr -d ' \n')
  machine=$(od -An -tx1 -j18 -N2 "$DESKTOP_APPIMAGE" | tr -d ' \n')
  [ "$magic" = "7f454c46" ] || die "the AppImage is not an ELF file (magic $magic)"
  [ "$class" = "2" ] || die "the AppImage is not 64-bit (ELF class $class)"
  [ "$machine" = "3e00" ] || die "the AppImage is not x86_64 (ELF machine $machine)"
  [ -x "$DESKTOP_APPIMAGE" ] || die "the AppImage is not executable; it cannot be run after download"
  DESKTOP_APPIMAGE_OUT="$OUT/OpenHardwareOS-$VERSION-$PLATFORM.AppImage"
  cp "$DESKTOP_APPIMAGE" "$DESKTOP_APPIMAGE_OUT"
  echo "published as    : $DESKTOP_DEB_OUT"
  echo "published as    : $DESKTOP_APPIMAGE_OUT"
fi

if [ "$RUN_BINARY" = "1" ]; then
  reported=$("$BINARY" --version)
  expected="ohm-cli ${VERSION#v}"
  [ "$reported" = "$expected" ] \
    || die "the binary reports '$reported', expected '$expected'"
  echo "binary reports: $reported"
fi

# LF only, and every entry must describe a file that is **published next to this
# list**: the archive and the metadata. `LICENSE` and `THIRD_PARTY_NOTICES.txt`
# are inside the archive — they are not separate Linux assets (the Windows build
# publishes its own copies, and a Release holds one asset per name), so naming
# them here made a plain `sha256sum -c` report two missing files on a download
# that was in fact intact. The archive's contents are checked by the listing
# above; this list is about the bytes a user downloads.
PUBLISHED=("$ARCHIVE" "$METADATA_PATH")
if [ -n "$DESKTOP_DEB_OUT" ]; then
  PUBLISHED+=("$DESKTOP_DEB_OUT" "$DESKTOP_APPIMAGE_OUT")
fi
{
  for file in "${PUBLISHED[@]}"; do
    printf '%s  %s\n' "$(hash_file "$file" | cut -d' ' -f1)" "$(basename "$file")"
  done
} > "$OUT/$SUMS.tmp"
if grep -q $'\r' "$OUT/$SUMS.tmp"; then
  die "$SUMS would contain a carriage return; the checksum tool could not read it"
fi
mv "$OUT/$SUMS.tmp" "$OUT/$SUMS"

# Check the list the way the target platform will: line by line, by name, in a
# directory holding **only what is published**. Staging anything else here would
# hide exactly the defect this check exists for.
SIDE="$OUT/.side-$$"
mkdir -p "$SIDE"
for file in "${PUBLISHED[@]}"; do cp "$file" "$SIDE/"; done
cp "$OUT/$SUMS" "$SIDE/"
( cd "$SIDE" && verify_sums "$SUMS" >/dev/null ) \
  || die "the checksum list does not verify against the files it names"
rm -rf "$SIDE"

echo "source commit : $SHA"
echo "archive       : $ARCHIVE"
ls -l "$ARCHIVE" | awk '{ print "archive size  : " $5 " bytes" }'
echo "$SUMS:"
sed 's/^/  /' "$OUT/$SUMS"
# From here the delivery is complete and verified: the trap may leave it alone.
COMPLETED=1
if [ -n "$DESKTOP_DEB_OUT" ]; then
  echo
  echo "Install on Debian/Ubuntu with: sudo apt-get install ./$(basename "$DESKTOP_DEB_OUT")"
  echo "Run the AppImage with       : chmod +x $(basename "$DESKTOP_APPIMAGE_OUT") && ./$(basename "$DESKTOP_APPIMAGE_OUT")"
fi
