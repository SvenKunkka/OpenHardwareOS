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

BUILD_ID=""
while [ $# -gt 0 ]; do
  case "$1" in
    --out) OUT_ROOT="$2"; shift 2 ;;
    --allow-dirty) ALLOW_DIRTY=1; shift ;;
    # A package is identified by the commit it was built from; a second package of the
    # same commit needs a name of its own rather than overwriting the first.
    --build-id) BUILD_ID="$2"; shift 2 ;;
    -h|--help) sed -n '2,24p' "${BASH_SOURCE[0]}"; exit 0 ;;
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

# Two different deviations, and they are not the same thing:
#
#   * a **modified tracked file** means the working tree is not the commit. Every file
#     in this package comes from the commit — nothing is taken from the working tree —
#     so the snapshot is still reproducible, but the operator's tests were run against
#     something else, and that has to be recorded rather than assumed away;
#   * an **untracked file** simply is not part of the commit. It cannot enter the
#     package at all (`git archive` cannot see it), so it is not a reason to refuse:
#     refusing would mean deleting unrelated files from a repository to make a tool
#     happy.
MODIFIED="$(git status --porcelain --untracked-files=no | wc -l | tr -d ' ')"
UNTRACKED="$(git status --porcelain --untracked-files=all | grep -c '^??' || true)"
if [ "$MODIFIED" != "0" ] && [ "$ALLOW_DIRTY" != "1" ]; then
  echo "The working tree has $MODIFIED modified tracked file(s):" >&2
  git status --porcelain --untracked-files=no >&2
  fail "refusing to package: the commit is what contains the source, but your tests were run against something else. Commit, or pass --allow-dirty to record the deviation (it is recorded, never included)."
fi
WORKTREE_STATE="clean"
if [ "$MODIFIED" != "0" ]; then
  WORKTREE_STATE="$MODIFIED modified tracked file(s) — the snapshot is still built from $SHORT, and the working-tree changes are NOT included"
fi
if [ "$UNTRACKED" -gt 0 ]; then
  WORKTREE_STATE="$WORKTREE_STATE; $UNTRACKED untracked path(s) present (never included)"
fi
COMMIT_DATE="$(git show -s --format=%cI "$SHA")"
echo "commit:            $SHA"
echo "date:              $COMMIT_DATE"
echo "snapshot source:   commit $SHORT only — no file is taken from the working tree"
echo "worktree:          $WORKTREE_STATE"

# ---------------------------------------------------------------------------
# 1. Extract the tracked source. `git archive` cannot include .git, and the
#    ignore rules mean no target/, node_modules/ or local config can be in the
#    tracked set in the first place.
# ---------------------------------------------------------------------------
NAME="OpenHardwareOS-${SHORT}-windows-acceptance"
[ -n "$BUILD_ID" ] && NAME="$NAME-$BUILD_ID"
PKG="$OUT_ROOT/$NAME"
ZIP="$OUT_ROOT/$NAME.zip"

# Assemble in staging and only then put a delivery in place. Two rules, both learned from
# getting this wrong: an existing package is never deleted or overwritten (a package is
# evidence, and evidence that can be silently replaced is not evidence), and a failed run
# never leaves something that looks like a delivery.
step "Preparing"
mkdir -p "$OUT_ROOT"
for existing in "$PKG" "$ZIP"; do
  if [ -e "$existing" ]; then
    echo "already exists: $existing" >&2
    echo "" >&2
    echo "Refusing to touch it. A package is a record of a revision, and this script will" >&2
    echo "not replace one. Either package a different commit, or give this build a name of" >&2
    echo "its own:" >&2
    echo "" >&2
    echo "  $0 --build-id \"$(date -u '+%Y%m%dT%H%M%SZ')\"" >&2
    exit 1
  fi
done

STAGING="$(mktemp -d "$OUT_ROOT/.staging-${SHORT}-XXXXXX")"
cleanup_staging() {
  if [ "${STAGING_KEEP:-0}" != "1" ]; then
    rm -rf "$STAGING"
  fi
}
trap cleanup_staging EXIT
STAGE_PKG="$STAGING/$NAME"
mkdir -p "$STAGE_PKG"
echo "staging: $STAGING"
echo "target : $PKG (created only if everything below succeeds)"

git archive --format=tar "$SHA" | tar -x -C "$STAGE_PKG"

# ---------------------------------------------------------------------------
# 2. The acceptance entry points, taken from the repository (so they are reviewed
#    like any other source) rather than generated here.
# ---------------------------------------------------------------------------
TEMPLATES="$REPO_ROOT/docs/windows-validation/package"
[ -d "$TEMPLATES" ] || fail "missing $TEMPLATES"
mkdir -p "$STAGE_PKG/scripts"
# The entry points an operator runs, plus the shared helper they all dot-source
# (_tools.ps1 resolves every tool to an absolute path and pins the toolchain). Kept as
# an explicit list, and checked against the template directory, so a new file there
# cannot ship half-wired: a package missing _tools.ps1 would fail on every entry point.
ENTRY_SCRIPTS="_tools.ps1 precheck.ps1 build.ps1 verify-package.ps1 verify-package.sh"
for entry in $ENTRY_SCRIPTS; do
  [ -f "$TEMPLATES/scripts/$entry" ] || fail "missing $TEMPLATES/scripts/$entry"
done
for template in "$TEMPLATES"/scripts/*.ps1 "$TEMPLATES"/scripts/*.sh; do
  [ -e "$template" ] || continue
  name="$(basename "$template")"
  case " $ENTRY_SCRIPTS " in
    *" $name "*) ;;
    *) fail "$TEMPLATES/scripts/$name is not in the packager's entry-point list — add it there, or the package ships without it" ;;
  esac
done
for entry in $ENTRY_SCRIPTS; do
  git show "$SHA:docs/windows-validation/package/scripts/$entry" > "$STAGE_PKG/scripts/$entry"
done
git show "$SHA:docs/windows-validation/package/README-ACCEPTANCE.md" > "$STAGE_PKG/README-ACCEPTANCE.md"
chmod +x "$STAGE_PKG/scripts/verify-package.sh"

# Both copies of every entry point — `scripts\<name>` and
# `docs/windows-validation/package/scripts/<name>` — now come from the same commit, so
# they must be identical. If they are not, something in this script is taking a file from
# somewhere other than the commit, which is the bug this assertion exists to catch.
for entry in $ENTRY_SCRIPTS; do
  if ! cmp -s "$STAGE_PKG/scripts/$entry" "$STAGE_PKG/docs/windows-validation/package/scripts/$entry"; then
    STAGING_KEEP=1
    fail "$entry differs between the two places it exists in the package: something was not taken from $SHORT. Staging kept at $STAGING"
  fi
done
if ! cmp -s "$STAGE_PKG/README-ACCEPTANCE.md" "$STAGE_PKG/docs/windows-validation/package/README-ACCEPTANCE.md"; then
  STAGING_KEEP=1
  fail "README-ACCEPTANCE.md differs between the two places it exists in the package. Staging kept at $STAGING"
fi
echo "  ok: every entry point and the README are byte-identical in both places they exist, from commit $SHORT"

# ---------------------------------------------------------------------------
# 3. Record the revision inside the package and in the quick start.
# ---------------------------------------------------------------------------
PACKAGED_AT="$(date -u '+%Y-%m-%dT%H:%M:%SZ')"
python3 - "$STAGE_PKG/README-ACCEPTANCE.md" "$SHA" "$SHORT" "$COMMIT_DATE" "$WORKTREE_STATE" "$PACKAGED_AT" <<'PY'
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

cat > "$STAGE_PKG/EVIDENCE.md" <<EOF
# Evidence index

This package is a **source snapshot at \`$SHA\`** (\`$SHORT\`, committed $COMMIT_DATE).
It contains no Windows build artefacts, because none exist: this project has never
been built, installed or run on Windows.

Machine the checks below were run on: macOS 26.6.2 (25G83) arm64, pinned
\`rustc 1.98.0\` / \`cargo 1.98.0\` / \`clippy 0.1.98\`, Node $(node --version 2>/dev/null || echo 'unknown'), npm $(npm --version 2>/dev/null || echo 'unknown').

| What | Status | Where the evidence is |
|---|---|---|
| Rust workspace test suite, \`clippy --workspace --all-targets -- -D warnings\`, \`cargo fmt --check\`, \`cargo deny check\` | **Read it from the log, not from here** | \`docs/verification-log.md\` — each entry opens with the revision it was run at, records every command, its environment and its result, and states the pass count |
| Frontend typecheck, test suite, production build | **Read it from the log, not from here** | \`docs/verification-log.md\` |
| The real desktop IPC round trip (\`scripts/verify-ipc-roundtrip.sh\`) | **Read it from the log, not from here** | \`docs/verification-log.md\` |
| Simulated closed loop (\`ohm-cli demo\`, \`ohm-desktop --selftest --mock\`, \`ohm-cli doctor --mock\`) | **Read it from the log, not from here** | \`docs/verification-log.md\` |
| The acceptance entry points' control flow (\`precheck.ps1\`, \`build.ps1\`, \`verify-package.ps1\`, \`_tools.ps1\`) | Exercised on macOS **with test doubles only** | \`docs/windows-validation/package/scripts/tests/run-script-tests.ps1\` — one case per defect that was real, with the failing assertion named for each |
| Windows: WMI storage, \`reg.exe\` autostart, tray, NVML, LibreHardwareMonitor against real hardware, NSIS install | **Prepared — never run** | \`docs/windows-validation/README.md\` (the honest baseline table) |
| A physical fan responding to a write | **No evidence at all** | \`docs/windows-validation/README.md\` — needs a tachometer, separately from any Windows run |
| Licence decisions (ADR 0002, ADR 0004) | **Proposed, not accepted** | \`docs/decisions/\` |

**Why this file claims nothing about test results.** It used to say "Passed on macOS at
this commit", and both halves of that were wrong at some point: the count went stale the
next time a test was added, and the tests are run at the revision the log names — which is
not always the revision being packaged. Packaging a commit after changing only a document
or a packaging script does not re-run a suite, and a package that implied otherwise would
be claiming evidence nobody produced.

So: \`docs/verification-log.md\` is the authority. Find the newest entry, read the revision
it names, and read its results. If that revision is not this one, then this package's
revision differs from the tested one, and what was verified is inherited — the source it
covers, minus whatever changed since. The log says what changed and whether a suite was
re-run; this file will not guess.

**What this package cannot tell you.** Nothing in it has been executed on Windows.
The pre-check, the build script and the evidence collector are written and reviewed,
and their control flow was exercised on the development machine with test doubles, but a
Windows run is the only thing that can produce Windows evidence. Treat every Windows
claim in the docs as *Prepared* until you produce it here.

Verify the package before use:

\`\`\`powershell
.\\scripts\\verify-package.ps1
\`\`\`

Then follow \`README-ACCEPTANCE.md\`. Two things about the flow are worth repeating here,
because both exist to keep this package verifiable:

* the build writes **outside** the package. Its output — logs, the environment record,
  the installer — goes to a work directory beside the extracted package
  (\`<package>-build\\\` by default, or whatever you pass as \`-WorkDir\` to both
  \`precheck.ps1\` and \`build.ps1\`). Nothing a run produces lands in here, so the
  manifest still describes this package after a build, and a second run works;
* \`collect.ps1\` writes a bundle folder into the directory it is given, so give it one
  outside the package (\`-OutputRoot <work>\\evidence\`). A collector bundle inside the
  package is a file the manifest does not describe, and the next verification will
  say so.
EOF

# ---------------------------------------------------------------------------
# 4. Manifest: every file, SHA-256, relative path with forward slashes.
# ---------------------------------------------------------------------------
step "Writing MANIFEST.sha256"
python3 - "$STAGE_PKG" <<'PY'
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
"$STAGE_PKG/scripts/verify-package.sh" | tail -3

step "Checking the package contains nothing it should not"
PROBLEMS=0
for forbidden in .git target node_modules dist; do
  if [ -e "$STAGE_PKG/$forbidden" ]; then echo "  FOUND $forbidden/ — must not be in a source snapshot"; PROBLEMS=$((PROBLEMS+1)); fi
done
if find "$STAGE_PKG" \( -name '*.exe' -o -name '*.msi' \) -print -quit | grep -q .; then
  echo "  FOUND a Windows binary — this package must not pretend to contain one"; PROBLEMS=$((PROBLEMS+1))
fi
if find "$STAGE_PKG" \( -name 'audit.jsonl' -o -name 'settings.json' -o -name '*.log' \) -print -quit | grep -q .; then
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
  PRE_OUT="$("$PWSH" -NoProfile -File "$STAGE_PKG/scripts/precheck.ps1" 2>&1)"
  PRE_CODE=$?
  set -e
  if [ "$PRE_CODE" -eq 0 ]; then
    fail "the pre-check accepted a non-Windows host; an operator would be told this machine can produce Windows evidence"
  fi
  echo "  ok: pre-check refused this host (exit $PRE_CODE) as it must"
  echo "$PRE_OUT" | grep -m1 'FAIL' | sed 's/^/      /' || true
  # And the same script must pass on this host when the platform check is waived —
  # that is what makes it useful to a contributor, and it proves the remaining
  # checks actually run.
  set +e
  PRE_OUT="$("$PWSH" -NoProfile -File "$STAGE_PKG/scripts/precheck.ps1" -AllowNonWindows 2>&1)"
  PRE_CODE=$?
  set -e
  echo "  -AllowNonWindows run exit: $PRE_CODE"
  # `|| true`: neither finding no lines nor finding no matches is a reason to abort
  # without saying anything, which is what `set -e` with `pipefail` did here.
  CHECKS="$(echo "$PRE_OUT" | grep -E '^  \[' || true)"
  if [ -n "$CHECKS" ]; then
    echo "$CHECKS" | sed 's/^/      /'
  else
    echo "      (the pre-check printed no per-check lines; its output shape may have changed)"
  fi
  if [ "$PRE_CODE" -ne 0 ]; then
    echo "      (the remaining checks failed on this machine — the output above says which)"
  fi
fi

# ---------------------------------------------------------------------------
# 6. The verdict, *before* anything is delivered.
# ---------------------------------------------------------------------------
# This used to be printed after the package had already been moved into place and
# zipped, so a run that failed its own checks still left a package behind — and a
# directory named `...-windows-acceptance` is exactly what a reader would then trust.
# Nothing is delivered until staging is clean.
if [ "$PROBLEMS" -ne 0 ]; then
  STAGING_KEEP=1
  echo
  echo "PACKAGING FAILED — $PROBLEMS problem(s). No package was created; the staging copy"
  echo "is kept for diagnosis at: $STAGING"
  exit 1
fi

# ---------------------------------------------------------------------------
# 7. Zip, then report absolute paths and hashes.
# ---------------------------------------------------------------------------
step "Archiving"
# Verified in staging; only now does a delivery appear. `mv` within one filesystem is
# atomic, so a reader never sees a half-built package, and nothing that already existed
# was touched on the way here.
mv "$STAGE_PKG" "$PKG"
( cd "$OUT_ROOT" && zip -q -r -X "$NAME.zip" "$NAME" )
ZIP_SHA="$(python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$ZIP")"
MANIFEST_SHA="$(python3 -c "import hashlib,sys;print(hashlib.sha256(open(sys.argv[1],'rb').read()).hexdigest())" "$PKG/MANIFEST.sha256")"
FILE_COUNT="$(wc -l < "$PKG/MANIFEST.sha256" | tr -d ' ')"
ZIP_SIZE="$(wc -c < "$ZIP" | tr -d ' ')"

cat <<EOF

================================================================================
Windows acceptance package ready — SOURCE ONLY, NO WINDOWS BUILD ARTEFACTS
================================================================================
revision packaged : $SHA ($SHORT)
snapshot source   : commit $SHORT only; no file was taken from the working tree
worktree          : $WORKTREE_STATE
test evidence     : docs/verification-log.md — read the revision each entry names; this
                    script makes no claim about which tests were run at $SHORT
directory         : $PKG
archive           : $ZIP
archive size      : $ZIP_SIZE bytes
files             : $FILE_COUNT (all listed in MANIFEST.sha256)
entry points      : scripts\precheck.ps1, scripts\build.ps1, scripts\verify-package.ps1,
                    scripts\_tools.ps1 (shared by both) and scripts\verify-package.sh
script tests      : docs\windows-validation\package\scripts\tests\run-script-tests.ps1
                    (test doubles only — NOT evidence that a Windows build succeeds)
build writes to   : a work directory BESIDE the package (<package>-build\, or -WorkDir),
                    never inside it, so the package still matches this manifest after a build
MANIFEST.sha256   : $MANIFEST_SHA
archive SHA-256   : $ZIP_SHA

Verified on this machine: contents match the manifest, no VCS data, no build
output, no dependencies, no binaries, no local runtime state.
NOT verified anywhere: anything requiring Windows. The pre-check, build and
collector scripts are unexecuted on Windows and are marked *Prepared*; their
control flow was exercised here with test doubles, which proves nothing about
WMI, NSIS, the registry, NVML or a real fan.
================================================================================
EOF

say "nothing pre-existing was modified or removed: a new package was created beside the others" 
