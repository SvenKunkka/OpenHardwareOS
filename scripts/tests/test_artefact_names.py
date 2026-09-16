"""Tests for the artefact-name checker; every case works on a throwaway copy of the tree.

The checker's whole purpose is to fail when someone renames a built artefact or points
an install entry at a version nobody can download, so the tests that matter are the
mutations: each one breaks a single source of truth in a fixture copy and requires the
checker to say so. A test that only asserts the real tree is clean would pass for a
checker that reads nothing at all.
"""

import importlib.util
import json
import shutil
import tempfile
import unittest
from pathlib import Path

MODULE = Path(__file__).resolve().parents[1] / "check-artefact-names.py"
SPEC = importlib.util.spec_from_file_location("check_artefact_names", MODULE)
checker = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(checker)

REPO_ROOT = Path(__file__).resolve().parents[2]


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
            checker.README: lambda text: text.replace("$ohmVersion = 'v0.1.10'",
                                                      "$ohmVersion = 'v0.2.0'"),
        })
        found = checker.problems(root)
        self.assertTrue(any("v0.2.0" in problem and "not downloadable" in problem
                            for problem in found), found)

    def test_an_older_released_version_is_reported_as_not_the_newest(self):
        root = fixture(**{
            checker.WINDOWS_PAGE: lambda text: text.replace("releases/download/v0.1.10/",
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
            checker.README: lambda text: text.replace("'v0.1.10'", "'v0.2.0'"),
            checker.WINDOWS_PAGE: lambda text: text.replace("v0.1.10", "v0.2.0"),
            checker.LINUX_PAGE: lambda text: text.replace("v0.1.10", "v0.2.0"),
        })
        found = [problem for problem in checker.problems(root) if "not downloadable" in problem]
        self.assertEqual(found, [], found)

    def test_every_entry_is_found(self):
        entries = list(checker.install_entries(REPO_ROOT))
        self.assertGreaterEqual(len(entries), 8)
        shapes = {shape for _, _, _, shape in entries}
        self.assertIn("download URL", shapes)
        self.assertIn("PowerShell $ohmVersion", shapes)
        self.assertIn("shell ohm_version", shapes)
        self.assertIn("cargo install --tag", shapes)


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
