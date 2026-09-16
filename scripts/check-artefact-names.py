#!/usr/bin/env python3
"""Check that every place naming a built artefact agrees with its single source.

Round 15's post-mortem: `29b95dd` renamed the desktop artefact through
`tauri.conf.json`'s `mainBinaryName`, and `scripts/verify-ipc-roundtrip.sh` kept
looking for the old path. Nothing in this repository failed: the script simply
stopped reaching the code it was supposed to exercise, and the two checks above
the missing-file guard passed on exit 127. It was found by a person reading a log,
which is the weakest kind of coverage there is.

This checker exists so that the same class of change fails a test instead:

* the **desktop** artefact's name has exactly one source — `mainBinaryName` in
  `apps/desktop/src-tauri/tauri.conf.json`. Every other mention must either derive
  from it (no literal `target/release/<name>`) or match it (the Linux install page,
  the packager's expectations).
* the **CLI** artefact's name has exactly one source — the `[package] name` in
  `apps/cli/Cargo.toml` — and the files that stage, archive or document it must use
  that name.
* every **install entry** (a download URL, `$ohmVersion`, `ohm_version`, or
  `cargo install --tag`) must name a version someone can actually download: the
  newest released version, or the version currently being prepared. The original
  review of this project opened with the opposite happening — the README pointed at
  a version that had never been published.

Reads only; exits non-zero with one line per problem. Paths are relative to `--root`
so the test suite can run it against a mutated fixture.
"""

from __future__ import annotations

import argparse
import json
import re
import sys
import tomllib
from pathlib import Path

DESKTOP_CONFIG = "apps/desktop/src-tauri/tauri.conf.json"
CLI_MANIFEST = "apps/cli/Cargo.toml"
CATALOGUE = "docs/versions.json"
IPC_SCRIPT = "scripts/verify-ipc-roundtrip.sh"
LINUX_PACKAGER = "scripts/release/package-linux.sh"
LINUX_PAGE = "docs/linux-install.md"
WINDOWS_PAGE = "docs/windows-install.md"
README = "README.md"
RELEASE_WORKFLOW = ".github/workflows/release.yml"

# Everything this module reads. The test suite copies exactly these into a fixture
# tree and mutates one of them, so the fixture can never drift from what is checked.
READS = (
    DESKTOP_CONFIG,
    CLI_MANIFEST,
    CATALOGUE,
    IPC_SCRIPT,
    LINUX_PACKAGER,
    LINUX_PAGE,
    WINDOWS_PAGE,
    README,
    RELEASE_WORKFLOW,
)

SEMVER = re.compile(r"v(\d+)\.(\d+)\.(\d+)\Z")

# The four shapes an install entry takes in this repository's documentation.
ENTRY_PATTERNS = (
    (re.compile(r"releases/download/(v\d+\.\d+\.\d+)/"), "download URL"),
    (re.compile(r"\$ohmVersion\s*=\s*'(v\d+\.\d+\.\d+)'"), "PowerShell $ohmVersion"),
    (re.compile(r"^\s*ohm_version=(v\d+\.\d+\.\d+)\s*$", re.MULTILINE), "shell ohm_version"),
    (re.compile(r"--tag\s+(v\d+\.\d+\.\d+)"), "cargo install --tag"),
)

# What the CLI artefact's name must look like in each file that names it.
CLI_EXPECTATIONS = (
    (README, "'{name}.exe'", "the Windows CLI executable as the README runs it"),
    (README, "--locked {name}", "the documented `cargo install` target"),
    (WINDOWS_PAGE, "{name}.exe", "the Windows CLI executable as the install page names it"),
    (LINUX_PAGE, '{name}-$ohm_version-$ohm_platform.tar.gz', "the documented Linux archive name"),
    (LINUX_PAGE, '"$ohm_dir/{name}"', "the documented Linux CLI path"),
    (LINUX_PACKAGER, '"$STAGE/{name}"', "the name the packager stages the CLI under"),
    (LINUX_PACKAGER, '"$STAGE" {name} LICENSE', "the name the packager archives"),
    (LINUX_PACKAGER, "grep -qx '{name}'", "the packager's own archive assertion"),
    (RELEASE_WORKFLOW, "target/release/{name}", "the binary the release workflow packages"),
)


def read(root: Path, rel: str) -> str | None:
    path = root / rel
    try:
        return path.read_text(encoding="utf-8")
    except FileNotFoundError:
        return None


def _version_key(text: str) -> tuple[int, int, int]:
    match = SEMVER.fullmatch(text)
    if not match:
        raise ValueError(f"not a version: {text!r}")
    return tuple(int(part) for part in match.groups())  # type: ignore[return-value]


def desktop_name(root: Path):
    """Return (name, problems) for the desktop artefact."""
    problems: list[str] = []
    raw = read(root, DESKTOP_CONFIG)
    if raw is None:
        return None, [f"{DESKTOP_CONFIG}: missing; the desktop artefact's name has no source"]
    try:
        config = json.loads(raw)
    except json.JSONDecodeError as error:
        return None, [f"{DESKTOP_CONFIG}: not valid JSON ({error})"]
    name = config.get("mainBinaryName")
    if not isinstance(name, str) or not name.strip():
        problems.append(
            f"{DESKTOP_CONFIG}: declares no `mainBinaryName`; the built application's name "
            "is then whatever cargo would have called it, and every file that expects a "
            "name is guessing"
        )
        return None, problems
    return name, problems


def cli_name(root: Path):
    raw = read(root, CLI_MANIFEST)
    if raw is None:
        return None, [f"{CLI_MANIFEST}: missing; the CLI artefact's name has no source"]
    data = tomllib.loads(raw)
    name = data.get("package", {}).get("name")
    if not isinstance(name, str) or not name:
        return None, [f"{CLI_MANIFEST}: [package] declares no name"]
    return name, []


def install_entries(root: Path):
    """Yield (file, line number, version, shape) for every install entry in the docs."""
    for rel in (README, WINDOWS_PAGE, LINUX_PAGE):
        text = read(root, rel)
        if text is None:
            continue
        for line_no, line in enumerate(text.splitlines(), start=1):
            for pattern, shape in ENTRY_PATTERNS:
                for match in pattern.finditer(line):
                    yield rel, line_no, match.group(1), shape


def literal_target_paths(root: Path):
    """Non-comment shell lines naming a path under `target/release`."""
    findings = []
    for path in sorted((root / "scripts").rglob("*.sh")):
        rel = path.relative_to(root).as_posix()
        for line_no, line in enumerate(path.read_text(encoding="utf-8").splitlines(), start=1):
            stripped = line.strip()
            if stripped.startswith("#") or "target/release/" not in line:
                continue
            for match in re.finditer(r"target/release/(\S*)", line):
                tail = match.group(1)
                if not tail.startswith("$"):
                    findings.append(
                        f"{rel}:{line_no}: names `target/release/{tail[:40]}` literally; the "
                        "artefact's name has one source (tauri.conf.json's `mainBinaryName` "
                        "for the desktop, Cargo.toml for the CLI) and a shell script must "
                        "derive it or take it as an argument"
                    )
    return findings


def problems(root: Path) -> list[str]:
    found: list[str] = []

    name, name_problems = desktop_name(root)
    found += name_problems

    cli, cli_problems = cli_name(root)
    found += cli_problems

    if name:
        page = read(root, LINUX_PAGE)
        if page is None:
            found.append(f"{LINUX_PAGE}: missing; the Linux install route is unverifiable")
        else:
            for needle, why in (
                (f"usr/bin/{name}", "the installed desktop binary's path"),
                (f"{name} --selftest", "the headless desktop self-test the page tells users to run"),
                (f"{name} --version", "the version check the page tells users to run"),
            ):
                if needle not in page:
                    found.append(
                        f"{LINUX_PAGE}: does not mention `{needle}` ({why}); the artefact is "
                        f"named by `mainBinaryName` as `{name}`"
                    )
        script = read(root, IPC_SCRIPT)
        if script is None:
            found.append(f"{IPC_SCRIPT}: missing")
        elif "mainBinaryName" not in script:
            found.append(
                f"{IPC_SCRIPT}: does not read `mainBinaryName`; the name it tests must come "
                "from the configuration, not from memory"
            )
        packager = read(root, LINUX_PACKAGER)
        if packager is None:
            found.append(f"{LINUX_PACKAGER}: missing")
        elif "mainBinaryName" not in packager:
            found.append(
                f"{LINUX_PACKAGER}: does not read `mainBinaryName`; the desktop package's "
                "installed binary must be checked against the name the configuration declares"
            )

    if cli:
        for rel, template, why in CLI_EXPECTATIONS:
            text = read(root, rel)
            if text is None:
                found.append(f"{rel}: missing")
                continue
            needle = template.format(name=cli)
            if needle not in text:
                found.append(
                    f"{rel}: does not use `{needle}` ({why}); the CLI artefact is named "
                    f"`{cli}` by {CLI_MANIFEST}"
                )

    found += literal_target_paths(root)

    # Install entries must point at something downloadable.
    raw_catalogue = read(root, CATALOGUE)
    if raw_catalogue is None:
        found.append(f"{CATALOGUE}: missing; install entries cannot be checked against it")
    else:
        catalogue = json.loads(raw_catalogue)
        released = [node["id"] for node in catalogue["versions"] if node["status"] == "released"]
        development = catalogue.get("development")
        if not released:
            found.append(f"{CATALOGUE}: records no released version at all")
            newest = None
            allowed: set[str] = set()
        else:
            newest = max(released, key=_version_key)
            # The newest released version, or the one being prepared. Anything older is a
            # stale entry: the reader is sent to a download that exists but is no longer
            # what this tree is about, which is how the README came to point at a version
            # that had never been published at all.
            allowed = {newest}
            if development:
                allowed.add(development)
        for rel, line_no, version, shape in install_entries(root):
            if version in allowed:
                continue
            if version in {node["id"] for node in catalogue["versions"]}:
                why = "it is not the newest released version"
            else:
                why = "no such version is recorded"
            found.append(
                f"{rel}:{line_no}: the {shape} names `{version}` — not downloadable: {why} "
                f"(newest released: {newest}, being prepared: {development or 'nothing'})"
            )

    return found


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    parser.add_argument("--root", default=".", help="tree to check (default: the current directory)")
    args = parser.parse_args(argv)

    root = Path(args.root).resolve()
    found = problems(root)
    if found:
        for problem in found:
            print(f"problem: {problem}", file=sys.stderr)
        print(f"FAILED: {len(found)} problem(s)", file=sys.stderr)
        return 1

    name, _ = desktop_name(root)
    cli, _ = cli_name(root)
    entries = len(list(install_entries(root)))
    print(
        f"OK: desktop artefact `{name}` and CLI `{cli}` agree with their sources; "
        f"{entries} install entry/entries point at a downloadable version"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
