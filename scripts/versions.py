#!/usr/bin/env python3
"""Inspect and maintain the version catalogue. Mutations require --apply.

Python 3.11+; no third-party dependencies. record-release reads GitHub through
the authenticated gh CLI. It never creates a tag, release, or remote asset.
"""

from __future__ import annotations

import argparse
import copy
from datetime import datetime
import difflib
import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import tomllib

ROOT = Path(__file__).resolve().parents[1]
CATALOG = Path("docs/versions.json")
TAG = re.compile(r"v(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\.(?:0|[1-9]\d*)\Z")
SHA = re.compile(r"[0-9a-f]{40}\Z")
STATUSES = {"released": "已发布", "development": "开发中", "planned": "计划中"}
RELEASE_FIELDS = ("tag", "commit", "release_url", "published_at")
APPLICATION_FILES = tuple(map(Path, ("Cargo.toml", "Cargo.lock", "apps/desktop/package.json",
                                    "apps/desktop/package-lock.json", "apps/desktop/src-tauri/tauri.conf.json")))


class VersionError(ValueError):
    pass


def require(condition, message):
    if not condition:
        raise VersionError(message)


def load(root):
    return json.loads((root / CATALOG).read_text(encoding="utf-8"))


def json_text(value):
    return json.dumps(value, ensure_ascii=False, indent=2) + "\n"


def validate(data):
    require(data.get("schema_version") == 1, "Unsupported catalogue schema_version")
    repo = data.get("repository", "")
    require(re.fullmatch(r"[\w.-]+/[\w.-]+", repo), "Invalid GitHub repository")
    nodes = data.get("versions")
    require(isinstance(nodes, list) and nodes, "versions must be a non-empty list")
    by_id = {}
    required = {
        "id", "parent", "status", "channel", "title", "summary", "tag", "commit",
        "branch", "release_url", "published_at", "changes", "evidence",
        "hardware_validation", "plan",
    }
    for node in nodes:
        require(isinstance(node, dict) and required <= node.keys(), "Incomplete version entry")
        tag = node["id"]
        require(isinstance(tag, str) and TAG.fullmatch(tag), f"Invalid numeric version: {tag}")
        require(tag not in by_id, f"Duplicate version: {tag}")
        by_id[tag] = node
        require(node["status"] in STATUSES, f"Invalid status: {tag}")
        require(node["channel"] in ("preview", "stable"), f"Invalid channel: {tag}")
        require(all(isinstance(node[k], str) and node[k].strip() for k in ("title", "summary")),
                f"Title and summary are required: {tag}")
        require(isinstance(node["changes"], list) and all(isinstance(v, str) for v in node["changes"]),
                f"Invalid changes: {tag}")
        require(node["hardware_validation"] in ("not_verified", "partial", "verified"),
                f"Invalid hardware_validation: {tag}")
        plan = node["plan"]
        require(plan is None or (isinstance(plan, str) and plan.startswith("docs/")
                                and ".." not in Path(plan).parts), f"Invalid plan path: {tag}")
        require(isinstance(node["evidence"], list), f"Invalid evidence: {tag}")
        for item in node["evidence"]:
            require(isinstance(item, dict) and isinstance(item.get("title"), str)
                    and isinstance(item.get("url"), str) and item["url"].startswith("https://"),
                    f"Evidence needs a title and HTTPS URL: {tag}")
        if node["status"] == "released":
            require(node["tag"] == tag, f"Released tag must match its id: {tag}")
            require(isinstance(node["commit"], str) and SHA.fullmatch(node["commit"]),
                    f"Released version needs a full commit SHA: {tag}")
            require(node["release_url"] == f"https://github.com/{repo}/releases/tag/{tag}",
                    f"Released URL must point to this repository and tag: {tag}")
            try:
                published = datetime.fromisoformat(node["published_at"].replace("Z", "+00:00"))
                require(published.tzinfo is not None, f"Published timestamp needs a timezone: {tag}")
            except (AttributeError, TypeError, ValueError) as error:
                raise VersionError(f"Invalid published_at: {tag}") from error
            require(node["branch"] is None and node["evidence"],
                    f"Released version needs evidence and no moving branch: {tag}")
        else:
            require(all(node[k] is None for k in RELEASE_FIELDS),
                    f"Unreleased version cannot claim a tag, SHA, publication or download: {tag}")
            require(not node.get("download_url") and not node.get("assets"),
                    f"Unreleased version cannot advertise downloads: {tag}")
            if node["status"] == "planned":
                require(node["branch"] is None, f"Planned version cannot claim an active branch: {tag}")
            else:
                require(isinstance(node["branch"], str) and bool(node["branch"].strip()),
                        f"Development version needs its branch: {tag}")
    for tag, node in by_id.items():
        require(node["parent"] is None or node["parent"] in by_id, f"Unknown parent: {tag}")
        seen = set()
        cursor = tag
        while cursor is not None:
            require(cursor not in seen, f"Cycle in version tree at {cursor}")
            seen.add(cursor)
            cursor = by_id[cursor]["parent"]
    active = [n["id"] for n in nodes if n["status"] == "development"]
    require(active == ([] if data.get("development") is None else [data["development"]]),
            "development must name the sole development entry, or be null")
    return by_id


def source_text(root, path, overrides=None):
    path = Path(path)
    text = overrides[path] if overrides is not None and path in overrides else (root / path).read_text(encoding="utf-8")
    return text.replace("\r\n", "\n")


def workspace(root, overrides=None):
    manifest = tomllib.loads(source_text(root, "Cargo.toml", overrides))
    members = {}
    for path in manifest["workspace"]["members"]:
        package = tomllib.loads((root / path / "Cargo.toml").read_text(encoding="utf-8"))["package"]
        require(package.get("version") == {"workspace": True}, f"Member must inherit version: {path}")
        members[package["name"]] = path
    return manifest, members


def internal_dependencies(root, manifest, members):
    for alias, dep in manifest["workspace"]["dependencies"].items():
        if not isinstance(dep, dict) or "path" not in dep:
            continue
        name = dep.get("package", alias)
        if name in members and (root / dep["path"]).resolve() == (root / members[name]).resolve():
            yield alias, dep


def check_application_versions(root, data, overrides=None):
    by_id = validate(data)
    manifest, members = workspace(root, overrides)
    version = manifest["workspace"]["package"]["version"]
    expected = data["development"] or f"v{version}"
    require(expected in by_id, f"Application version is absent from catalogue: {expected}")
    require(by_id[expected]["status"] != "planned", "Application cannot build a planned-only version")
    require(f"v{version}" == expected, f"Workspace version {version} differs from {expected}")
    values = {"Cargo.toml workspace": version}
    for name, dep in internal_dependencies(root, manifest, members):
        values[f"Cargo.toml {name}"] = dep.get("version")
    lock = tomllib.loads(source_text(root, "Cargo.lock", overrides))
    for name in members:
        packages = [p for p in lock["package"] if p["name"] == name and "source" not in p]
        require(len(packages) == 1, f"Cargo.lock must contain exactly one workspace package: {name}")
        values[f"Cargo.lock {name}"] = packages[0]["version"]
    for path in ("apps/desktop/package.json", "apps/desktop/src-tauri/tauri.conf.json"):
        values[path] = json.loads(source_text(root, path, overrides))["version"]
    npm_lock = json.loads(source_text(root, "apps/desktop/package-lock.json", overrides))
    values["package-lock.json root"] = npm_lock["version"]
    values["package-lock.json packages root"] = npm_lock["packages"][""]["version"]
    for location, actual in values.items():
        require(actual == version, f"Version mismatch in {location}: {actual} != {version}")
    return version


def tree_lines(data):
    validate(data)
    lines = []

    def visit(parent, prefix=""):
        children = [node for node in data["versions"] if node["parent"] == parent]
        for index, node in enumerate(children):
            last = index == len(children) - 1
            channel = "" if node["status"] == "planned" else (" · 预览版" if node["channel"] == "preview" else " · 正式版")
            lines.append(f"{prefix}{'└─ ' if last else '├─ '}{node['id']} · {STATUSES[node['status']]}{channel} · {node['title']}")
            visit(node["id"], prefix + ("   " if last else "│  "))

    visit(None)
    return lines


def markdown(data):
    lines = ["# 版本树", "", "<!-- Generated by scripts/versions.py render; edit docs/versions.json. -->", "",
             "[交互版本树](version-tree.html) · [版本数据](versions.json) · [版本维护说明](version-management.md)", "", "```text",
             *tree_lines(data), "```", "", "预览版已发布不代表真实硬件兼容性已完成验收。", ""]
    for node in data["versions"]:
        lines += [f"## {node['id']} · {node['title']}", "", node["summary"], "",
                  f"- 状态：{STATUSES[node['status']]}"]
        if node["branch"]:
            lines.append(f"- 开发分支：`{node['branch']}`")
        if node["release_url"]:
            lines += [f"- [发布与下载]({node['release_url']})", f"- 源码：`{node['commit']}`",
                      f"- 发布时间：{node['published_at']}"]
        if node["plan"]:
            lines.append(f"- [版本计划]({node['plan'].removeprefix('docs/')})")
        lines += ["", *[f"- {item}" for item in node["changes"]], ""]
        if node["evidence"]:
            lines += ["验证记录：", "", *[f"- [{item['title']}]({item['url']})" for item in node["evidence"]], ""]
    return "\n".join(lines)


def generated(root, data):
    validate(data)
    template = (root / "scripts/templates/version-tree.html").read_text(encoding="utf-8")
    require(template.count("__VERSION_DATA__") == 1, "HTML template needs exactly one __VERSION_DATA__ placeholder")
    payload = json.dumps(data, ensure_ascii=False).replace("&", "\\u0026").replace("<", "\\u003c").replace(">", "\\u003e")
    payload = payload.replace("\u2028", "\\u2028").replace("\u2029", "\\u2029")
    return {Path("docs/version-tree.html"): template.replace("__VERSION_DATA__", payload),
            Path("docs/versions.md"): markdown(data)}


class FileUpdates(dict):
    """Proposed text plus the file identities read while preparing it."""

    def __init__(self, values=(), *, expected=None):
        super().__init__(values)
        self.expected = expected or {}


def file_signature(stat):
    return (stat.st_dev, stat.st_ino, stat.st_size, stat.st_mtime_ns, stat.st_ctime_ns)


def snapshot_file(path):
    try:
        before = path.stat()
    except FileNotFoundError:
        return None, None, 0o644
    require(not path.is_symlink() and path.is_file(), f"Expected a regular file: {path}")
    content = path.read_bytes()
    after = path.stat()
    require(file_signature(before) == file_signature(after), f"Concurrent edit while reading {path}")
    return content, file_signature(after), after.st_mode & 0o777


def unchanged(path, snapshot, *, replaced=False):
    current = snapshot_file(path)
    if replaced:
        # rename changes ctime; inode, size and mtime still identify our staged file.
        return (current[0] == snapshot[0] and current[1] is not None
                and current[1][:4] == snapshot[1][:4] and current[2] == snapshot[2])
    return current == snapshot


def stage_bytes(path, content, mode):
    with tempfile.NamedTemporaryFile(mode="wb", prefix=".versions-", dir=path.parent, delete=False) as output:
        temporary = Path(output.name)
        try:
            output.write(content)
            output.flush()
            os.fsync(output.fileno())
            temporary.chmod(mode)
        except BaseException:
            temporary.unlink(missing_ok=True)
            raise
    return temporary


def write_files(root, updates):
    """Stage every output first, then replace; roll back ordinary I/O failures.

    This is an exception-safe transaction, not crash recovery across power loss.
    Rollback only touches our own unchanged replacements. A concurrent edit or
    rollback I/O error leaves the original-byte backup available for recovery.
    """
    records, committed, retained = {}, [], set()
    try:
        for relative, expected in getattr(updates, "expected", {}).items():
            require(unchanged(root / relative, expected), f"Concurrent edit; refusing to overwrite {root / relative}")
        for relative, text in updates.items():
            path = root / relative
            expected = getattr(updates, "expected", {}).get(relative)
            original = expected if expected is not None else snapshot_file(path)
            require(unchanged(path, original), f"Concurrent edit; refusing to overwrite {path}")
            path.parent.mkdir(parents=True, exist_ok=True)
            record = {"path": path, "original": original, "stage": None, "backup": None}
            records[relative] = record
            record["stage"] = stage_bytes(path, text.encode("utf-8"), original[2])
            record["written"] = snapshot_file(record["stage"])
            if original[0] is not None:
                record["backup"] = stage_bytes(path, original[0], original[2])
        # No destination is changed before all output and backup writes succeed.
        for record in records.values():
            require(unchanged(record["path"], record["original"]),
                    f"Concurrent edit; refusing to overwrite {record['path']}")
            os.replace(record["stage"], record["path"])
            committed.append(record)
        for relative, expected in getattr(updates, "expected", {}).items():
            if relative not in updates:
                require(unchanged(root / relative, expected), f"Concurrent edit to input {root / relative}")
    except BaseException as error:
        problems = []
        for record in reversed(committed):
            path, backup = record["path"], record["backup"]
            try:
                require(unchanged(path, record["written"], replaced=True),
                        f"Concurrent edit preserved at {path}")
                if backup is None:
                    path.unlink()
                else:
                    os.replace(backup, path)
            except (OSError, VersionError) as rollback_error:
                if backup is not None:
                    retained.add(backup)
                problems.append(f"{rollback_error}; original-byte backup: {backup}")
        if problems:
            raise VersionError(f"Update failed ({error}); rollback needs review: {'; '.join(problems)}") from error
        raise
    finally:
        for record in records.values():
            for temporary in (record["stage"], record["backup"]):
                if temporary is not None and temporary not in retained:
                    temporary.unlink(missing_ok=True)


def changed_files(root, updates):
    return FileUpdates({path: content for path, content in updates.items()
                        if not (root / path).exists() or source_text(root, path) != content.replace("\r\n", "\n")},
                       expected=getattr(updates, "expected", {}))


def review_or_apply(root, updates, apply):
    changes = changed_files(root, updates)
    if not changes:
        print("No changes needed.")
        return
    if apply:
        write_files(root, changes)
        print("Updated: " + ", ".join(str(path) for path in changes))
    else:
        for path, content in changes.items():
            before = (root / path).read_text(encoding="utf-8") if (root / path).exists() else ""
            print("".join(difflib.unified_diff(before.splitlines(True), content.replace("\r\n", "\n").splitlines(True),
                                             fromfile=str(path), tofile=str(path))), end="")
        print("Dry run only. Repeat with --apply to write these changes.")


def replace_version_line(text, version, location):
    pattern = r'(?m)^([ \t]*version[ \t]*=[ \t]*["\'])[^"\'\n]+(["\'][ \t]*(?:#[^\n]*)?)$'
    updated, count = re.subn(pattern, lambda m: m[1] + version + m[2], text)
    require(count == 1, f"Expected exactly one version field in {location}; found {count}")
    return updated


def version_updates(root, tag, sources=None):
    require(TAG.fullmatch(tag), "Expected numeric tag vMAJOR.MINOR.PATCH")
    version = tag[1:]
    manifest, members = workspace(root, sources)
    cargo = source_text(root, "Cargo.toml", sources)
    section = re.compile(r"(?ms)(^\[workspace\.package\]\n)(.*?)(?=^\[|\Z)")
    cargo, count = section.subn(lambda m: m[1] + replace_version_line(m[2], version, "workspace.package"), cargo)
    require(count == 1, "Cannot locate workspace.package")
    for name, _dep in internal_dependencies(root, manifest, members):
        pattern = rf'(?m)^({re.escape(name)}\s*=\s*\{{[^\n]*\bversion\s*=\s*")[^"]+("[^\n]*\}})'
        cargo, count = re.subn(pattern, lambda m: m[1] + version + m[2], cargo)
        require(count == 1, f"Cannot synchronize internal dependency: {name}")
    updates = {Path("Cargo.toml"): cargo}
    lock = source_text(root, "Cargo.lock", sources)
    seen = set()

    def replace_package(match):
        block = match[0]
        package = tomllib.loads(block)["package"][0]
        if package["name"] in members and "source" not in package:
            require(package["name"] not in seen, "Duplicate workspace lock package")
            seen.add(package["name"])
            return replace_version_line(block, version, f"Cargo.lock {package['name']}")
        return block

    lock = re.sub(r"(?ms)^\[\[package\]\]\n.*?(?=^\[\[package\]\]|\Z)", replace_package, lock)
    require(seen == set(members), "Cargo.lock is missing workspace packages")
    # Cargo may disambiguate dependencies by version; update only local names.
    for name in members:
        old_versions = {p["version"] for p in tomllib.loads(source_text(root, "Cargo.lock", sources))["package"]
                        if p["name"] == name and "source" not in p}
        for old in old_versions:
            lock = lock.replace(f'"{name} {old}"', f'"{name} {version}"')
    updates[Path("Cargo.lock")] = lock
    for path in ("apps/desktop/package.json", "apps/desktop/package-lock.json", "apps/desktop/src-tauri/tauri.conf.json"):
        value = json.loads(source_text(root, path, sources))
        value["version"] = version
        if path.endswith("package-lock.json"):
            value["packages"][""]["version"] = version
        updates[Path(path)] = json_text(value)
    return updates


def prepare(root, data, tag, branch=None):
    originals = {path: snapshot_file(root / path) for path in (*APPLICATION_FILES, CATALOG)}
    require(all(snapshot[0] is not None for snapshot in originals.values()), "Required version input file is missing")
    sources = {path: snapshot[0].decode("utf-8") for path, snapshot in originals.items()}
    require(json.loads(sources[CATALOG]) == data,
            "Concurrent edit to version catalogue since it was read; reload before preparing")
    by_id = validate(data)
    require(tag in by_id, "Add the intended version to docs/versions.json before preparing it")
    require(by_id[tag]["status"] != "released", "A published version cannot be prepared or overwritten")
    require(data["development"] in (None, tag), "Finish the current development version before preparing another")
    updated = copy.deepcopy(data)
    node = next(n for n in updated["versions"] if n["id"] == tag)
    node["status"] = "development"
    node["branch"] = branch or node["branch"]
    require(node["branch"], "Preparing a planned version requires --branch")
    updated["development"] = tag
    validate(updated)
    updates = FileUpdates(version_updates(root, tag, sources), expected=originals)
    updates[CATALOG] = json_text(updated)
    check_application_versions(root, updated, updates)
    # Preserve a Windows checkout's CRLF style without making the TOML parser
    # or textual version matching depend on the host's checkout configuration.
    for path, content in updates.items():
        original = originals[path][0]
        if b"\r\n" in original and b"\n" not in original.replace(b"\r\n", b""):
            updates[path] = content.replace("\r\n", "\n").replace("\n", "\r\n")
    return updates


def github_json(endpoint, *, asset=False):
    command = ["gh", "api", endpoint]
    if asset:
        command += ["-H", "Accept: application/octet-stream"]
    result = subprocess.run(command, check=True, capture_output=True)
    return json.loads(result.stdout.decode("utf-8-sig"))


def published_release(repo, tag, api=github_json):
    require(TAG.fullmatch(tag), "Expected numeric tag vMAJOR.MINOR.PATCH")
    base = f"repos/{repo}"
    release = api(f"{base}/releases/tags/{tag}")
    require(not release.get("draft") and bool(release.get("published_at")) and release.get("tag_name") == tag,
            "GitHub release is not published for this tag")
    reference = api(f"{base}/git/ref/tags/{tag}")["object"]
    for _ in range(8):
        if reference["type"] == "commit":
            break
        require(reference["type"] == "tag", "Release tag does not resolve to a commit")
        reference = api(f"{base}/git/tags/{reference['sha']}")["object"]
    require(reference["type"] == "commit" and SHA.fullmatch(reference["sha"]), "Invalid release tag commit")
    sha = reference["sha"]
    target = release.get("target_commitish", "")
    require(not SHA.fullmatch(target) or target == sha, "Release target differs from tag SHA")
    assets = [a for a in release.get("assets", []) if a["name"] == "release.json" and a.get("state") == "uploaded"]
    require(len(assets) == 1, "Published release needs exactly one uploaded release.json")
    # gh handles authentication and GitHub's asset redirects; no untrusted URL is executed.
    metadata = api(f"{base}/releases/assets/{assets[0]['id']}", asset=True)
    require(metadata.get("version") == tag and metadata.get("repository") == repo
            and metadata.get("source_commit") == sha, "Published release.json does not match repository, version and tag SHA")
    require(release.get("html_url") == f"https://github.com/{repo}/releases/tag/{tag}", "Unexpected release URL")
    return release, sha


def recorded_release(data, tag, api=github_json):
    by_id = validate(data)
    require(tag in by_id, "Version must already exist in the catalogue")
    release, sha = published_release(data["repository"], tag, api)
    old = by_id[tag]
    if old["status"] == "released":
        require(old["commit"] == sha, "Refusing to replace an existing release with a different SHA")
        require(old["channel"] == ("preview" if release["prerelease"] else "stable")
                and old["published_at"] == release["published_at"]
                and old["release_url"] == release["html_url"],
                "Remote publication metadata differs from the preserved release record")
        return copy.deepcopy(data)
    updated = copy.deepcopy(data)
    node = next(n for n in updated["versions"] if n["id"] == tag)
    node.update(status="released", tag=tag, commit=sha, branch=None,
                release_url=release["html_url"], published_at=release["published_at"],
                channel="preview" if release["prerelease"] else "stable")
    item = {"title": "GitHub 已发布版本与构建来源", "url": release["html_url"]}
    if item not in node["evidence"]:
        node["evidence"].append(item)
    if updated["development"] == tag:
        updated["development"] = None
    validate(updated)
    return updated


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--root", type=Path, default=ROOT, help=argparse.SUPPRESS)
    commands = parser.add_subparsers(dest="command", required=True)
    commands.add_parser("tree", help="Print the version tree")
    show = commands.add_parser("show", help="Show a catalogue entry")
    show.add_argument("version")
    check = commands.add_parser("check", help="Check catalogue and application version consistency")
    check.add_argument("--remote", action="store_true", help="Also verify published releases and tag SHAs on GitHub")
    check.add_argument("--generated", action="store_true", help="Also check generated HTML and Markdown")
    commands.add_parser("render", help="Regenerate HTML and Markdown from the catalogue")
    prep = commands.add_parser("prepare", help="Preview application version synchronization")
    prep.add_argument("version")
    prep.add_argument("--branch", help="Development branch when starting a planned version")
    prep.add_argument("--apply", action="store_true")
    record = commands.add_parser("record-release", help="Read an actual GitHub publication into the catalogue")
    record.add_argument("version")
    record.add_argument("--apply", action="store_true")
    args = parser.parse_args(argv)
    try:
        root = args.root.resolve()
        data = load(root)
        by_id = validate(data)
        if args.command == "tree":
            print("\n".join(tree_lines(data)))
        elif args.command == "show":
            require(args.version in by_id, f"Unknown version: {args.version}")
            print(json_text(by_id[args.version]), end="")
        elif args.command == "check":
            version = check_application_versions(root, data)
            if args.remote:
                for node in data["versions"]:
                    if node["status"] == "released":
                        recorded_release(data, node["id"])
            if args.generated:
                require(not changed_files(root, generated(root, data)), "Generated views are stale; run versions.py render")
            print(f"OK: {len(by_id)} catalogue entries; application version {version}" + ("; published releases verified on GitHub" if args.remote else "; publication fields checked offline"))
        elif args.command == "render":
            review_or_apply(root, generated(root, data), True)
        elif args.command == "prepare":
            review_or_apply(root, prepare(root, data, args.version, args.branch), args.apply)
        elif args.command == "record-release":
            updated = recorded_release(data, args.version)
            review_or_apply(root, {CATALOG: json_text(updated)}, args.apply)
        return 0
    except (VersionError, OSError, KeyError, TypeError, json.JSONDecodeError, tomllib.TOMLDecodeError,
            subprocess.CalledProcessError) as error:
        print(f"Error: {error}", file=sys.stderr)
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
