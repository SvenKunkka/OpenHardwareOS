"""Version lifecycle and synchronization tests; all writes use temporary files."""

import contextlib
import copy
import importlib.util
import io
import json
from pathlib import Path
import tempfile
import tomllib
import unittest
from unittest import mock

MODULE = Path(__file__).resolve().parents[1] / "versions.py"
SPEC = importlib.util.spec_from_file_location("versions", MODULE)
versions = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(versions)
def catalogue():
    nodes = []
    for tag, parent, status in (("v0.1.0", None, "released"),
                                ("v0.1.1", "v0.1.0", "development"),
                                ("v0.2.0", "v0.1.1", "planned")):
        released = status == "released"
        nodes.append({
            "id": tag, "parent": parent, "status": status, "channel": "preview",
            "title": "测试版本", "summary": "临时测试数据", "tag": tag if released else None,
            "commit": "d" * 40 if released else None,
            "branch": "codex/version-tree" if status == "development" else None,
            "release_url": f"https://github.com/SvenKunkka/OpenHardwareOS/releases/tag/{tag}" if released else None,
            "published_at": "2026-09-14T09:00:00Z" if released else None,
            "changes": ["测试变更"],
            "evidence": [{"title": "发布证据", "url": "https://github.com/SvenKunkka/OpenHardwareOS"}] if released else [],
            "hardware_validation": "not_verified", "plan": None,
        })
    return {"schema_version": 1, "repository": "SvenKunkka/OpenHardwareOS",
            "development": "v0.1.1", "versions": nodes}


def fake_release(tag="v0.1.1", sha="b" * 40, *, draft=False, metadata_sha=None, annotated=False):
    repo = "SvenKunkka/OpenHardwareOS"
    responses = {
        f"repos/{repo}/releases/tags/{tag}": {
            "draft": draft, "published_at": "2026-09-14T09:00:00Z", "tag_name": tag,
            "target_commitish": sha, "html_url": f"https://github.com/{repo}/releases/tag/{tag}",
            "prerelease": True, "assets": [{"id": 10, "name": "release.json", "state": "uploaded"}],
        },
        f"repos/{repo}/git/ref/tags/{tag}": {
            "object": {"type": "tag" if annotated else "commit", "sha": "a" * 40 if annotated else sha}
        },
        f"repos/{repo}/git/tags/{'a' * 40}": {"object": {"type": "commit", "sha": sha}},
        f"repos/{repo}/releases/assets/10": {
            "version": tag, "repository": repo, "source_commit": metadata_sha or sha,
        },
    }

    def api(endpoint, *, asset=False):
        if endpoint.endswith("/assets/10"):
            assert asset, "The metadata asset must be read as content"
        return copy.deepcopy(responses[endpoint])

    return api


class CatalogueTests(unittest.TestCase):
    def test_catalogue_has_one_active_development_version(self):
        data = catalogue()
        self.assertEqual(set(versions.validate(data)), {"v0.1.0", "v0.1.1", "v0.2.0"})

    def test_rejects_duplicate_versions(self):
        data = catalogue()
        data["versions"].append(copy.deepcopy(data["versions"][0]))
        with self.assertRaisesRegex(versions.VersionError, "Duplicate"):
            versions.validate(data)

    def test_rejects_cycles(self):
        data = catalogue()
        data["versions"][0]["parent"] = "v0.2.0"
        with self.assertRaisesRegex(versions.VersionError, "Cycle"):
            versions.validate(data)

    def test_rejects_unknown_parent(self):
        data = catalogue()
        data["versions"][-1]["parent"] = "v9.0.0"
        with self.assertRaisesRegex(versions.VersionError, "Unknown parent"):
            versions.validate(data)

    def test_planned_version_cannot_claim_publication_or_download(self):
        for field, value in (("release_url", "https://example.com/download"),
                             ("commit", "b" * 40), ("download_url", "https://example.com/download")):
            with self.subTest(field=field):
                data = catalogue()
                data["versions"][-1][field] = value
                with self.assertRaisesRegex(versions.VersionError, "Unreleased"):
                    versions.validate(data)

    def test_released_version_needs_complete_evidence(self):
        for field, value in (("commit", "abc123"), ("published_at", None), ("evidence", [])):
            with self.subTest(field=field):
                data = catalogue()
                data["versions"][0][field] = value
                with self.assertRaises(versions.VersionError):
                    versions.validate(data)

    def test_development_pointer_cannot_name_a_plan(self):
        data = catalogue()
        data["development"] = "v0.2.0"
        with self.assertRaisesRegex(versions.VersionError, "sole development"):
            versions.validate(data)

    def test_annotated_published_tag_and_metadata_are_verified_before_recording(self):
        data = catalogue()
        recorded = versions.recorded_release(data, "v0.1.1", fake_release(annotated=True))
        self.assertEqual(data["versions"][1]["status"], "development")
        self.assertEqual(recorded["versions"][1]["status"], "released")
        self.assertEqual(recorded["versions"][1]["commit"], "b" * 40)
        self.assertEqual(recorded["versions"][1]["hardware_validation"], "not_verified")
        self.assertIsNone(recorded["development"])

    def test_draft_release_is_not_recorded(self):
        with self.assertRaisesRegex(versions.VersionError, "not published"):
            versions.recorded_release(catalogue(), "v0.1.1", fake_release(draft=True))

    def test_mismatching_asset_commit_is_not_recorded(self):
        with self.assertRaisesRegex(versions.VersionError, "does not match"):
            versions.recorded_release(catalogue(), "v0.1.1", fake_release(metadata_sha="c" * 40))

    def test_existing_release_commit_cannot_be_replaced(self):
        with self.assertRaisesRegex(versions.VersionError, "different SHA"):
            versions.recorded_release(catalogue(), "v0.1.0", fake_release(tag="v0.1.0"))

    def test_same_commit_cannot_hide_remote_channel_or_date_changes(self):
        for field, changed in (("channel", "stable"), ("published_at", "2026-09-13T09:00:00Z")):
            with self.subTest(field=field):
                data = catalogue()
                data["versions"][0][field] = changed
                with self.assertRaisesRegex(versions.VersionError, "metadata differs"):
                    versions.recorded_release(data, "v0.1.0", fake_release(tag="v0.1.0", sha="d" * 40))


class FilesTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        contents = {
            "Cargo.toml": '[workspace]\nmembers = ["crates/core"]\n\n[workspace.package]\nversion = "0.1.0"\n\n[workspace.dependencies]\nohm-core = { path = "crates/core", version = "0.1.0" }\nexternal = "0.1.0"\n',
            "crates/core/Cargo.toml": '[package]\nname = "ohm-core"\nversion.workspace = true\n',
            "Cargo.lock": 'version = 4\n\n[[package]]\nname = "ohm-core"\nversion = "0.1.0"\ndependencies = ["external"]\n\n[[package]]\nname = "external"\nversion = "0.1.0"\nsource = "registry+https://github.com/rust-lang/crates.io-index"\nchecksum = "leave-this-unchanged"\n',
            "apps/desktop/package.json": '{"name":"desktop","version":"0.1.0","dependencies":{"external":"0.1.0"}}',
            "apps/desktop/package-lock.json": '{"version":"0.1.0","packages":{"":{"version":"0.1.0"},"node_modules/external":{"version":"0.1.0","integrity":"keep-me"}}}',
            "apps/desktop/src-tauri/tauri.conf.json": '{"version":"0.1.0"}',
            "scripts/templates/version-tree.html": '<script type="application/json" id="data">__VERSION_DATA__</script>',
            "docs/versions.json": versions.json_text(catalogue()),
        }
        versions.write_files(self.root, {Path(path): value for path, value in contents.items()})

    def snapshot(self):
        return {str(p.relative_to(self.root)): p.read_bytes() for p in self.root.rglob("*") if p.is_file()}

    def test_prepare_is_dry_run_unless_apply_is_explicit(self):
        before = self.snapshot()
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(versions.main(["--root", str(self.root), "prepare", "v0.1.1"]), 0)
        self.assertEqual(before, self.snapshot())

    def test_third_replace_failure_restores_every_original_byte(self):
        before = self.snapshot()
        replace = versions.os.replace
        calls = 0

        def fail_third(source, destination):
            nonlocal calls
            calls += 1
            if calls == 3:
                raise OSError("simulated third replacement failure")
            return replace(source, destination)

        with mock.patch.object(versions.os, "replace", side_effect=fail_third):
            with contextlib.redirect_stderr(io.StringIO()):
                result = versions.main(["--root", str(self.root), "prepare", "v0.1.1", "--apply"])
        self.assertEqual(result, 1)
        self.assertEqual(before, self.snapshot())
        self.assertEqual(calls, 5, "two committed files must be rolled back")

    def test_rollback_preserves_a_concurrent_external_edit(self):
        before = self.snapshot()
        updates = versions.prepare(self.root, catalogue(), "v0.1.1")
        replace = versions.os.replace
        calls = 0
        external = b"A concurrent editor owns this content\n"

        def fail_after_edit(source, destination):
            nonlocal calls
            calls += 1
            if calls == 3:
                (self.root / "Cargo.toml").write_bytes(external)
                raise OSError("simulated third replacement failure")
            return replace(source, destination)

        with mock.patch.object(versions.os, "replace", side_effect=fail_after_edit):
            with self.assertRaisesRegex(versions.VersionError, "Concurrent edit preserved"):
                versions.write_files(self.root, updates)
        self.assertEqual((self.root / "Cargo.toml").read_bytes(), external)
        for path, content in before.items():
            if path != "Cargo.toml":
                self.assertEqual((self.root / path).read_bytes(), content)
        backups = list(self.root.glob(".versions-*"))
        self.assertEqual(len(backups), 1)
        self.assertEqual(backups[0].read_bytes(), before["Cargo.toml"])

    def test_edit_between_preparation_and_apply_is_not_overwritten(self):
        updates = versions.prepare(self.root, catalogue(), "v0.1.1")
        path = self.root / "Cargo.toml"
        path.write_text(path.read_text() + "# concurrent edit\n")
        before = self.snapshot()
        with self.assertRaisesRegex(versions.VersionError, "Concurrent edit"):
            versions.write_files(self.root, updates)
        self.assertEqual(before, self.snapshot())

    def test_catalogue_edit_between_load_and_prepare_is_not_overwritten(self):
        old = versions.load(self.root)
        edited = copy.deepcopy(old)
        edited["versions"][1]["summary"] += " concurrent edit"
        (self.root / versions.CATALOG).write_text(versions.json_text(edited), encoding="utf-8")
        before = self.snapshot()
        with self.assertRaisesRegex(versions.VersionError, "Concurrent edit to version catalogue"):
            versions.prepare(self.root, old, "v0.1.1")
        self.assertEqual(before, self.snapshot())

    def test_version_line_comment_is_preserved_and_all_outputs_are_valid(self):
        path = self.root / "Cargo.toml"
        path.write_text(path.read_text().replace('version = "0.1.0"\n', 'version = "0.1.0" # original version\n'))
        updates = versions.prepare(self.root, catalogue(), "v0.1.1")
        self.assertIn('version = "0.1.1" # original version\n', updates[Path("Cargo.toml")])
        versions.write_files(self.root, updates)
        self.assertEqual(versions.check_application_versions(self.root, catalogue()), "0.1.1")

    def test_crlf_checkout_can_prepare_without_changing_line_endings(self):
        for path in self.root.rglob("*"):
            if path.is_file():
                path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
        cargo = self.root / "Cargo.toml"
        cargo.write_bytes(cargo.read_bytes().replace(b'version = "0.1.0"\r\n',
                                                    b'version = "0.1.0" # original version\r\n'))
        updates = versions.prepare(self.root, catalogue(), "v0.1.1")
        versions.write_files(self.root, updates)
        self.assertEqual(versions.check_application_versions(self.root, catalogue()), "0.1.1")
        for path in (self.root / "Cargo.toml", self.root / "Cargo.lock", self.root / versions.CATALOG):
            content = path.read_bytes()
            self.assertIn(b"\r\n", content)
            self.assertNotIn(b"\n", content.replace(b"\r\n", b""))
        self.assertIn(b'version = "0.1.1" # original version\r\n', cargo.read_bytes())

    def test_generated_check_ignores_windows_checkout_line_endings(self):
        versions.write_files(self.root, versions.prepare(self.root, catalogue(), "v0.1.1"))
        versions.write_files(self.root, versions.generated(self.root, catalogue()))
        for path in self.root.rglob("*"):
            if path.is_file():
                path.write_bytes(path.read_bytes().replace(b"\n", b"\r\n"))
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(versions.main(["--root", str(self.root), "check", "--generated"]), 0)

    def test_nonmatching_version_field_fails_before_any_write(self):
        path = self.root / "Cargo.toml"
        path.write_text(path.read_text().replace('version = "0.1.0"\n', '"version" = "0.1.0"\n'))
        before = self.snapshot()
        with self.assertRaisesRegex(versions.VersionError, "exactly one version field"):
            versions.prepare(self.root, catalogue(), "v0.1.1")
        self.assertEqual(before, self.snapshot())

    def test_inconsistent_proposed_output_is_rejected_before_writing(self):
        original = versions.version_updates

        def broken_output(*args, **kwargs):
            updates = original(*args, **kwargs)
            updates[Path("apps/desktop/src-tauri/tauri.conf.json")] = '{"version":"9.9.9"}'
            return updates

        before = self.snapshot()
        with mock.patch.object(versions, "version_updates", side_effect=broken_output):
            with self.assertRaisesRegex(versions.VersionError, "Version mismatch"):
                versions.prepare(self.root, catalogue(), "v0.1.1")
        self.assertEqual(before, self.snapshot())

    def test_version_sync_preserves_third_party_dependencies_and_old_release(self):
        old = catalogue()["versions"][0]
        with contextlib.redirect_stdout(io.StringIO()):
            self.assertEqual(versions.main(["--root", str(self.root), "prepare", "v0.1.1", "--apply"]), 0)
        data = versions.load(self.root)
        self.assertEqual(versions.check_application_versions(self.root, data), "0.1.1")
        self.assertEqual(data["versions"][0], old)
        cargo = tomllib.loads((self.root / "Cargo.toml").read_text())
        self.assertEqual(cargo["workspace"]["dependencies"]["external"], "0.1.0")
        lock = tomllib.loads((self.root / "Cargo.lock").read_text())
        self.assertEqual(lock["package"][1]["version"], "0.1.0")
        self.assertEqual(lock["package"][1]["checksum"], "leave-this-unchanged")
        npm = json.loads((self.root / "apps/desktop/package-lock.json").read_text())
        self.assertEqual(npm["packages"]["node_modules/external"], {"version": "0.1.0", "integrity": "keep-me"})

    def test_check_detects_unsynchronized_npm_lock_root(self):
        versions.write_files(self.root, versions.prepare(self.root, catalogue(), "v0.1.1"))
        path = self.root / "apps/desktop/package-lock.json"
        value = json.loads(path.read_text())
        value["packages"][""]["version"] = "0.1.0"
        path.write_text(json.dumps(value))
        with self.assertRaisesRegex(versions.VersionError, "Version mismatch"):
            versions.check_application_versions(self.root, catalogue())

    def test_external_path_dependency_is_not_synchronized(self):
        path = self.root / "Cargo.toml"
        path.write_text(path.read_text() + 'third-party = { path = "../external", version = "9.4.0" }\n')
        versions.write_files(self.root, versions.prepare(self.root, catalogue(), "v0.1.1"))
        cargo = tomllib.loads(path.read_text())
        self.assertEqual(cargo["workspace"]["dependencies"]["third-party"]["version"], "9.4.0")
        self.assertEqual(versions.check_application_versions(self.root, catalogue()), "0.1.1")

    def test_cannot_prepare_a_published_tag(self):
        before = self.snapshot()
        with self.assertRaisesRegex(versions.VersionError, "published version"):
            versions.prepare(self.root, catalogue(), "v0.1.0")
        self.assertEqual(before, self.snapshot())

    def test_cannot_start_second_version_before_current_is_finished(self):
        with self.assertRaisesRegex(versions.VersionError, "Finish the current"):
            versions.prepare(self.root, catalogue(), "v0.2.0", "codex/pump-support")

    def test_can_prepare_next_planned_version_after_publication(self):
        data = versions.recorded_release(catalogue(), "v0.1.1", fake_release())
        versions.write_files(self.root, {versions.CATALOG: versions.json_text(data)})
        updates = versions.prepare(self.root, data, "v0.2.0", "codex/pump-support")
        versions.write_files(self.root, updates)
        new = versions.load(self.root)
        self.assertEqual(versions.check_application_versions(self.root, new), "0.2.0")
        self.assertEqual(new["versions"][-1]["branch"], "codex/pump-support")
        self.assertEqual(new["versions"][-1]["status"], "development")
        self.assertIsNone(new["versions"][-1]["release_url"])

    def test_render_cannot_close_the_json_script_element(self):
        data = catalogue()
        data["versions"][1]["title"] = '</script><img src=x onerror="alert(1)">\u2028&'
        output = versions.generated(self.root, data)[Path("docs/version-tree.html")]
        self.assertEqual(output.count("</script>"), 1)
        self.assertNotIn("<img", output)
        payload = output.split('id="data">', 1)[1].split("</script>", 1)[0]
        self.assertEqual(json.loads(payload), data)

    def test_planned_tree_node_has_no_preview_channel_label(self):
        text = "\n".join(versions.tree_lines(catalogue()))
        planned = next(line for line in text.splitlines() if "v0.2.0" in line)
        self.assertIn("计划中", planned)
        self.assertNotIn("预览版", planned)
        self.assertIn("version-management.md", versions.markdown(catalogue()))


if __name__ == "__main__":
    unittest.main()
