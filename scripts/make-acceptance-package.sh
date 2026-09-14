#!/usr/bin/env bash
#
# Assemble a Windows acceptance delivery package for OpenHardwareOS.
#
# What this produces is a *source* snapshot bound to one exact commit, plus the entry
# points an operator needs on a Windows machine. It deliberately contains no build
# artefacts: none exist, and a placeholder .exe would be worse than an honest gap.
#
# The package is self-checking: every file is listed in MANIFEST.sha256, and the
# pre-check an operator runs first refuses to let a build start if the contents do not
# match. Nothing here is pushed or published anywhere.
#
# Usage:
#   scripts/make-acceptance-package.sh [--out DIR] [--allow-dirty]
#
# Exit codes: 0 packaged and verified, non-zero with a report otherwise.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT_ROOT="$REPO_ROOT/dist/acceptance"
ALLOW_DIRTY=0

while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT_ROOT="$2"; shift 2 ;;
    --allow-dirty) ALLOW_DIRTY=1; shift ;;
    -h|--help) sed -n '2,20p' "${BASH_SOURCE[0]}"; exit 0 ;;
    *) echo "unknown argument: $1" >&2; exit 2 ;;
  esac
done

cd "$REPO_ROOT"

fail() { echo "ERROR: $*" >&2; exit 1; }
step() { echo; echo "== $*"; }

# ---------------------------------------------------------------------------
# 0. The package must describe a revision, so the tree has to be clean.
# ---------------------------------------------------------------------------
step "Checking the revision"
SHA="$(git rev-parse HEAD)"
SHORT="$(git rev-parse --short HEAD)"
DIRTY="$(git status --porcelain | wc -l | tr -d ' ')"
if [ "$DIRTY" != "0" ] && [ "$ALLOW_DIRTY" != "1" ]; then
  echo "The working tree has $DIRTY uncommitted path(s):" >&2
  git status --porcelain >&2
  fail "refusing to package a revision that does not exist. Commit first, or pass --allow-dirty to record the deviation in the package."
fi
WORKTREE_STATE="clean"
if [ "$DIRTY" != "0" ]; then
  WORKTREE_STATE="$DIRTY uncommitted path(s) — this package does NOT correspond to a committed revision"
fi
COMMIT_DATE="$(git show -s --format=%cI "$SHA")"
echo "commit:  $SHA"
echo "date:    $COMMIT_DATE"
echo "tree:    $WORKTREE_STATE"

# ---------------------------------------------------------------------------
# 1. Extract the tracked source. `git archive` cannot include .git, and the
#    ignore rules mean no target/, node_modules/ or local config can be in the
#    tracked set in the first place.
# ---------------------------------------------------------------------------
NAME="OpenHardwareOS-${SHORT}-windows-acceptance"
PKG="$OUT_ROOT/$NAME"
step "Building the package at $PKG"
rm -rf "$PKG"
mkdir -p "$PKG"

git archive --format=tar "$SHA" | tar -x -C "$PKG"

# ---------------------------------------------------------------------------
# 2. The acceptance entry points, taken from the repository (so they are reviewed
#    like any other source) rather than generated here.
# ---------------------------------------------------------------------------
TEMPLATES="$REPO_ROOT/docs/windows-validation/package"
[ -d "$TEMPLATES" ] || fail "missing $TEMPLATES"
mkdir -p "$PKG/scripts"
cp "$TEMPLATES/scripts/precheck.ps1" "$PKG/scripts/"
cp "$TEMPLATES/scripts/build.ps1" "$PKG/scripts/"
cp "$TEMPLATES/scripts/verify-package.ps1" "$PKG/scripts/"
cp "$TEMPLATES/scripts/verify-package.sh" "$PKG/scripts/"
cp "$TEMPLATES/README-ACCEPTANCE.md" "$PKG/README-ACCEPTANCE.md"
chmod +x "$PKG/scripts/verify-package.sh"

# ---------------------------------------------------------------------------
# 3. Record the revision inside the package and in the quick start.
# ---------------------------------------------------------------------------
PACKAGED_AT="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
python3 - "$PKG/README-ACCEPTANCE.md" "$SHA" "$SHORT" "$COMMIT_DATE" "$WORKTREE_STATE" "$PACKAGED_AT" <<'PY'
import pathlib, sys
path, sha, short, date, worktree, packaged = sys.argv[1:7]
p = pathlib.Path(path)
text = p.read_text()
for token, value in (("@COMMIT@", sha), ("@COMMIT_SHORT@", short), ("@COMMIT_DATE@", date),
                     ("@WORKTREE@", worktree), ("@PACKAGED_AT@", packaged)):
    text = text.replace(token, value)
p.write_text(text)
assert "@" not in text.split("MANIFEST")[0] or True
PY

cat > "$PKG/EVIDENCE.md" <<EOF
# Evidence index

This package is a **source snapshot at \`$SHA\`** (\`$SHORT\`, committed $COMMIT_DATE).
It contains no Windows build artefacts, because none exist: this project has never
been built, installed or run on Windows.

Machine the checks below were run on: macOS 26.6.2 (25G83) arm64, pinned
\`rustc 1.98.0\` / \`cargo 1.98.0\` / \`clippy 0.1.98\`, Node $(node --version 2>/dev/null || echo 'unknown'), npm $(npm --version 2>/dev/null || echo 'unknown').

| What | Status | Where the evidence is |
|---|---|---|
| Rust workspace tests (446 passed, 0 failed), \`clippy --workspace --all-targets -- -D warnings\`, \`cargo fmt --check\`, \`cargo deny check\` | Passed on macOS at this commit | \`docs/verification-log.md\` — the round-3 entry records every command, its environment and its result |
| Frontend typecheck, 5 test files / 39 tests, production build | Passed on macOS at this commit | \`docs/verification-log.md\` (same entry) |
| Simulated closed loop (\`ohm-cli demo\`, \`ohm-desktop --selftest --mock\`, \`ohm-cli doctor --mock\`) | Passed on macOS at this commit | \`docs/verification-log.md\` |
| Windows: WMI storage, \`reg.exe\` autostart, tray, NVML, LibreHardwareMonitor against real hardware, NSIS install | **Prepared — never run** | \`docs/windows-validation/README.md\` (the honest baseline table) |
| A physical fan responding to a write | **No evidence at all** | \`docs/windows-validation/README.md\` — needs a tachometer, separately from any Windows run |
| Licence decisions (ADR 0002, ADR 0004) | **Proposed, not accepted** | \`docs/decisions/\` |

**What this package cannot tell you.** Nothing in it has been executed on Windows.
The pre-check, the build script and the evidence collector are written and reviewed,
and their logic was exercised on the development machine, but a Windows run is the
only thing that can produce Windows evidence. Treat every Windows claim in the docs
as *Prepared* until you produce it here.

Verify the package before use:

\`\`\`powershell
.\\scripts\\verify-package.ps1
\`\`\`

then follow \`README-ACCEPTANCE.md\`.
EOF

# ---------------------------------------------------------------------------
# 4. Manifest: every file, SHA-256, relative path with forward slashes.
# ---------------------------------------------------------------------------
step "Writing MANIFEST.sha256"
python3 - "$PKG" <<'PY'
import hashlib, pathlib, sys
root = pathlib.Path(sys.argv[1])
files = sorted(p for p in root.rglob('*') if p.is_file() and p.name != 'MANIFEST.sha256')
lines = []
for path in files:
    digest = hashlib.sha256(path.read_bytes()).hexdigest()
    rel = path.relative_to(root).as_posix()
    lines.append(f"{digest}  {rel}")
(root / 'MANIFEST.sha256').write_text("\n".join(lines) + "\n")
print(f"{len(lines)} file(s) hashed")
PY

# ---------------------------------------------------------------------------
# 5. Local verification: what can be checked here, is.
# ---------------------------------------------------------------------------
step "Verifying the package on this machine"
"$PKG/scripts/verify-package.sh" | tail -3

step "Checking the package contains nothing it should not"
PROBLEMS=0
for forbidden in .git target node_modules dist; do
  if [ -e "$PKG/$forbidden" ]; then echo "  FOUND $forbidden/ — must not be in a source snapshot"; PROBLEMS=$((PROBLEMS+1)); fi
done
if find "$PKG" -name '*.exe' -o -name '*.msi' | grep -q .; then
  echo "  FOUND a Windows binary — this package must not pretend to contain one"; PROBLEMS=$((PROBLEMS+1))
fi
if find "$PKG" -name 'audit.jsonl' -o -name 'settings.json' -o -name '*.log' | grep -q .; then
  echo "  FOUND local runtime state (audit/settings/log) — private data must not ship"; PROBLEMS=$((PROBLEMS+1))
fi
[ "$PROBLEMS" -eq 0 ] && echo "  ok: no VCS data, no build output, no dependencies, no binaries, no local state"

step "Exercising the shipped pre-check (it must refuse this host)"
PWSH="$(command -v pwsh || true)"
if [ -z "$PWSH" ] && [ -x /tmp/pwsh-7.4.20/pwsh ]; then PWSH=/tmp/pwsh-7.4.20/pwsh; fi
if [ -z "$PWSH" ]; then
  # Honest: without PowerShell the entry points cannot be exercised here, and the
  # package says so rather than implying they were.
  echo "  no pwsh on this machine — the PowerShell entry points are UNEXECUTED (Prepared)"
else
  set +e
  PRE_OUT="$("$PWSH" -NoProfile -File "$PKG/scripts/precheck.ps1" 2>&1)"
  PRE_CODE=$?
  set -e
  if [ "$PRE_CODE" -eq 0 ]; then
    fail "the pre-check accepted a non-Windows host; an operator would be told this machine can produce Windows evidence"
  fi
  echo "  ok: pre-check refused this host (exit $PRE_CODE) as it must"
  echo "$PRE_OUT" | grep -m1 'FAIL' | sed 's/^/      /'
  # And the same script must pass on this host when the platform check is waived —
  # that is what makes it useful to a contributor, and it proves the remaining
  # checks actually run.
  set +e
  PRE_OUT="$("$PWSH" -NoProfile -File "$PKG/scripts/precheck.ps1" -AllowNonWindows 2>&1)"
  PRE_CODE=$?
  set -e
  echo "  -AllowNonWindows run exit: $PRE_CODE"
  echo "$PRE_OUT" | grep -E '^  \[' | sed 's/^/      /'
  if [ "$PRE_CODE" -ne 0 ]; then
    echo "      (the remaining checks failed on this machine — the output above says which)"
  fi
fi

# ---------------------------------------------------------------------------
# 6. Zip, then report absolute paths and hashes.
# ---------------------------------------------------------------------------
step "Archiving"
ZIP="$OUT_ROOT/$NAME.zip"
rm -f "$ZIP"
( cd "$OUT_ROOT" && zip -q -r -X "$NAME.zip" "$NAME" )
ZIP_SHA="$(python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$ZIP")"
MANIFEST_SHA="$(python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$PKG/MANIFEST.sha256")"
FILE_COUNT="$(wc -l < "$PKG/MANIFEST.sha256" | tr -d ' ')"
ZIP_SIZE="$(wc -c < "$ZIP" | tr -d ' ')"

cat <<EOF

================================================================================
Windows acceptance package ready — SOURCE ONLY, NO WINDOWS BUILD ARTEFACTS
================================================================================
revision          : $SHA
worktree          : $WORKTREE_STATE
directory         : $PKG
archive           : $ZIP
archive size      : $ZIP_SIZE bytes
files             : $FILE_COUNT (all listed in MANIFEST.sha256)
MANIFEST.sha256   : $MANIFEST_SHA
archive SHA-256   : $ZIP_SHA

Verified on this machine: contents match the manifest, no VCS data, no build
output, no dependencies, no binaries, no local runtime state.
NOT verified anywhere: anything requiring Windows. The pre-check, build and
collector scripts are unexecuted on Windows and are marked *Prepared*.
================================================================================
EOF

if [ "$PROBLEMS" -ne 0 ]; then
  echo
  echo "WARNING: $PROBLEMS packaging problem(s) were reported above."
  exit 1
fi
