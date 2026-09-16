"""Tests for the artefact-name checker; every case works on a throwaway copy of the tree.

The checker's whole purpose is to fail when someone renames a built artefact or points
an install entry at a version nobody can download, so the tests that matter are the
mutations: each one breaks a single source of truth in a fixture copy and requires the
checker to say so. A test that only asserts the real tree is clean would pass for a
checker that reads nothing at all.
"""

import importlib.util
import json
import re
import shutil
import tempfile
import unittest
from pathlib import Path

MODULE = Path(__file__).resolve().parents[1] / "check-artefact-names.py"
SPEC = importlib.util.spec_from_file_location("check_artefact_names", MODULE)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

REPO_ROOT = Path(__file__).resolve().parents[2]

# The install entries move with every release, so a test that mutates "the" version must
# read it from the file it is mutating. These tests used to spell it out, and the commit
# that bumped the entries from v0.1.10 to v0.1.11 turned every one of those mutations into
# a no-op: five of them failed in CI while passing locally a commit earlier, because
# "replace v0.1.10 with a planned version" quietly became "replace nothing".
VERSION = re.compile(r"v\d+\.\d+\.\d+")


def version_in(rel):
    """The version this file's install entries name (the first one it mentions)."""
    found = VERSION.search((REPO_ROOT / rel).read_text(encoding="utf-8"))
    if not found:
        raise AssertionError(f"{rel} names no version at all")
    return found.group(0)


def fixture(**mutations):
    """A copy of the files the checker reads, with `mutations` applied.

    Keys are the checker's own relative paths, values are either a replacement text or a
    callable receiving the original text and returning the new one. `None` deletes a file.
    """
    root = Path(tempfile.mkdtemp(prefix="artefact-names-fixture."))
    for rel in checker.READS:
        source = REPO_ROOT / rel
        target = root / rel
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(source, target)
    for rel, change in mutations.items():
        target = root / rel
        if change is None:
            target.unlink()
            continue
        text = target.read_text(encoding="utf-8")
        target.write_text(change(text) if callable(change) else change, encoding="utf-8")
    return root


class CleanTreeTests(unittest.TestCase):
    def test_the_real_tree_has_no_problems(self):
        self.assertEqual(checker.problems(REPO_ROOT), [])

    def test_a_note_in_a_comment_about_the_old_name_is_not_a_problem(self):
        # The IPC script explains what went wrong in round 15 and names the old path in
        # that explanation. Correcting history is not the same as depending on it, so the
        # literal must be tolerated inside a comment and reported only as code.
        script = (REPO_ROOT / checker.IPC_SCRIPT).read_text(encoding="utf-8")
        commented = [line for line in script.splitlines()
                     if line.strip().startswith("#") and "target/release/" in line]
        self.assertTrue(commented, "the history note this test tolerates is gone")

    def test_the_desktop_and_cli_names_come_from_their_manifests(self):
        desktop, problems = checker.desktop_name(REPO_ROOT)
        cli, cli_problems = checker.cli_name(REPO_ROOT)
        self.assertEqual(problems + cli_problems, [])
        self.assertEqual(desktop, "openhardwareos")
        self.assertEqual(cli, "ohm-cli")


class RenameTests(unittest.TestCase):
    def test_renaming_the_desktop_artefact_is_reported(self):
        root = fixture(**{
            checker.DESKTOP_CONFIG: lambda text: text.replace(
                '"mainBinaryName": "openhardwareos"', '"mainBinaryName": "ohm-desktop"'),
        })
        found = checker.problems(root)
        self.assertTrue(found)
        self.assertTrue(any("usr/bin/ohm-desktop" in problem for problem in found), found)

    def test_a_hardcoded_artefact_path_is_reported(self):
        root = fixture(**{
            checker.IPC_SCRIPT: lambda text: text.replace(
                'APP_BIN="$REPO_ROOT/target/release/$APP_NAME"',
                'APP_BIN="$REPO_ROOT/target/release/ohm-desktop"'),
        })
        found = checker.problems(root)
        self.assertTrue(any("target/release/ohm-desktop" in problem for problem in found), found)

    def test_an_empty_main_binary_name_is_reported_with_its_reason(self):
        root = fixture(**{
            checker.DESKTOP_CONFIG: '{"productName": "OpenHardwareOS"}',
        })
        found = checker.problems(root)
        self.assertTrue(any("mainBinaryName" in problem for problem in found), found)

    def test_renaming_the_cli_is_reported_in_every_place_that_names_it(self):
        root = fixture(**{
            checker.CLI_MANIFEST: lambda text: text.replace('name = "ohm-cli"', 'name = "ohm"', 1),
        })
        found = checker.problems(root)
        files = {problem.split(":")[0] for problem in found}
        self.assertIn(checker.README, files)
        self.assertIn(checker.WINDOWS_PAGE, files)
        self.assertIn(checker.LINUX_PAGE, files)
        self.assertIn(checker.LINUX_PACKAGER, files)

    def test_an_unreadable_configuration_is_reported_rather_than_ignored(self):
        root = fixture(**{checker.DESKTOP_CONFIG: None})
        found = checker.problems(root)
        self.assertTrue(any("missing" in problem for problem in found), found)


class InstallEntryTests(unittest.TestCase):
    def test_a_planned_version_is_not_downloadable(self):
        root = fixture(**{
            checker.README: lambda text: re.sub(
                r"\$ohmVersion = 'v[\d.]+'", "$ohmVersion = 'v0.2.0'", text),
        })
        found = checker.problems(root)
        self.assertTrue(any("v0.2.0" in problem and "not downloadable" in problem
                            for problem in found), found)

    def test_an_older_released_version_is_reported_as_not_the_newest(self):
        root = fixture(**{
            checker.WINDOWS_PAGE: lambda text: text.replace(
                f"releases/download/{version_in(checker.WINDOWS_PAGE)}/",
                "releases/download/v0.1.5/"),
        })
        found = checker.problems(root)
        self.assertTrue(any("v0.1.5" in problem and "newest released" in problem
                            for problem in found), found)

    def test_the_version_being_prepared_is_a_valid_target(self):
        # During preparation the install entries lead the download on purpose: the
        # release build's own documentation has to describe the version being released.
        # `record-release` clears the development pointer, so the published state is
        # checked strictly.
        root = fixture(**{
            checker.CATALOGUE: lambda text: json.dumps(
                _catalogue_with(text, development="v0.2.0"), ensure_ascii=False),
            # Every mention, not only the quoted one: `cli-v<version>` paths, download
            # URLs, `--tag` and release-page links are entries too, and a fixture that
            # moved only some of them would be testing a half-bumped page.
            checker.README: lambda text: text.replace(
                version_in(checker.README), "v0.2.0"),
            checker.WINDOWS_PAGE: lambda text: text.replace(
                version_in(checker.WINDOWS_PAGE), "v0.2.0"),
            checker.LINUX_PAGE: lambda text: text.replace(
                version_in(checker.LINUX_PAGE), "v0.2.0"),
        })
        found = [problem for problem in checker.problems(root) if "not downloadable" in problem]
        self.assertEqual(found, [], found)

    def test_a_stale_release_page_link_is_reported(self):
        root = fixture(**{
            checker.README: lambda text: text.replace(
                f"releases/tag/{version_in(checker.README)}", "releases/tag/v0.1.8"),
        })
        found = checker.problems(root)
        self.assertTrue(any("v0.1.8" in problem and "newest released" in problem
                            for problem in found), found)

    def test_a_versioned_install_path_that_did_not_move_is_reported(self):
        # The installed CLI lives in `…/cli-vX.Y.Z/`, so those paths have to move with
        # the version above them; a partially bumped page sends users to a directory
        # that is not there.
        root = fixture(**{
            checker.WINDOWS_PAGE: lambda text: text.replace(
                f"cli-v{version_in(checker.WINDOWS_PAGE)[1:]}", "cli-v0.1.9"),
        })
        found = checker.problems(root)
        self.assertTrue(any("versioned CLI install path" in problem for problem in found), found)

    def test_a_sentence_about_an_older_layout_is_not_an_entry(self):
        root = fixture(**{
            checker.WINDOWS_PAGE: lambda text: text + (
                "\nCLI 目录自 cli-v0.1.3 起就是版本化的。\n"),
        })
        found = [problem for problem in checker.problems(root) if "not downloadable" in problem]
        self.assertEqual(found, [], found)

    def test_every_entry_is_found(self):
        entries = list(checker.install_entries(REPO_ROOT))
        self.assertGreaterEqual(len(entries), 15)
        shapes = {shape for _, _, _, shape in entries}
        self.assertIn("download URL", shapes)
        self.assertIn("release page link", shapes)
        self.assertIn("PowerShell $ohmVersion", shapes)
        self.assertIn("shell ohm_version", shapes)
        self.assertIn("cargo install --tag", shapes)
        self.assertIn("versioned CLI install path", shapes)


def _catalogue_with(text, **changes):
    data = json.loads(text)
    data.update(changes)
    if "development" in changes:
        # The version being prepared has to exist as a node, or the catalogue's own
        # validation is what rejects it rather than this checker.
        for node in data["versions"]:
            if node["id"] == changes["development"]:
                node["status"] = "development"
    return data


if __name__ == "__main__":
    unittest.main()
