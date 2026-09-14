#!/usr/bin/env python3
"""Collect upstream notices from fetched, locked dependencies; no network scraping."""
import argparse
import json
from pathlib import Path
import subprocess


def notice_files(root, explicit=None):
    paths = set()
    if explicit:
        path = (root / explicit).resolve()
        if path.is_file() and path.is_relative_to(root.resolve()):
            paths.add(path)
    for path in root.rglob("*"):
        if not path.is_file():
            continue
        rel = path.relative_to(root)
        # A dependency's own notices, not vendored examples' node_modules.
        if len(rel.parts) > 3 or "node_modules" in rel.parts:
            continue
        name = path.name.upper()
        if name.startswith(("LICENSE", "LICENCE", "COPYING", "NOTICE", "UNLICENSE")):
            paths.add(path.resolve())
    return sorted(paths)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--cargo", default="cargo")
    parser.add_argument("--target", default="x86_64-pc-windows-msvc")
    args = parser.parse_args()
    repo = Path(__file__).resolve().parents[2]
    result = subprocess.run(
        [args.cargo, "metadata", "--locked", "--format-version", "1",
         "--filter-platform", args.target, "--manifest-path", str(repo / "Cargo.toml")],
        cwd=repo, check=True, text=True, encoding="utf-8", stdout=subprocess.PIPE,
    )
    metadata = json.loads(result.stdout)
    selected = {node["id"] for node in metadata["resolve"]["nodes"]}
    entries = []
    upstream_root = repo / "third-party" / "upstream-licenses"
    mapping_path = upstream_root / "mapping.json"
    upstream = json.loads(mapping_path.read_text(encoding="utf-8")) if mapping_path.exists() else {}
    for package in metadata["packages"]:
        if package["id"] not in selected or package["source"] is None:
            continue
        entries.append((
            "Rust", package["name"], package["version"], package.get("license"),
            package.get("repository"), Path(package["manifest_path"]).parent,
            package.get("license_file"),
        ))
    frontend = repo / "apps" / "desktop"
    lock = json.loads((frontend / "package-lock.json").read_text(encoding="utf-8"))
    for location, info in lock["packages"].items():
        if not location or info.get("dev"):
            continue
        root = frontend / location
        manifest = root / "package.json"
        if not manifest.is_file():
            raise SystemExit(f"Dependency is not installed: {location}; run npm ci first")
        package = json.loads(manifest.read_text(encoding="utf-8"))
        if package["version"] != info["version"]:
            raise SystemExit(f"Installed dependency differs from lockfile: {location}")
        repository = package.get("repository")
        if isinstance(repository, dict):
            repository = repository.get("url")
        entries.append(("npm", package["name"], package["version"],
                        package.get("license", info.get("license")), repository, root, None))
    sections = [
        "OpenHardwareOS - third-party license and copyright notices\n"
        "Generated from locked dependency packages. Includes build/test dependencies.\n"
        "Original notices below are retained verbatim; declared SPDX expressions\n"
        "describe upstream packages, not a new license grant by this inventory.\n"
    ]
    missing = []
    for ecosystem, name, version, license_id, repository, root, explicit in sorted(entries):
        files = notice_files(root, explicit)
        source_note = ""
        if not files and ecosystem == "Rust" and f"{name}@{version}" in upstream:
            record = upstream[f"{name}@{version}"]
            root = upstream_root
            files = [(upstream_root / rel).resolve() for rel in record["files"]]
            if any(not path.is_file() or not path.is_relative_to(upstream_root.resolve()) for path in files):
                raise SystemExit(f"Invalid upstream notice mapping: {name}@{version}")
            source_note = f"Upstream notice source: {record['repository']} at {record['revision']}\n"
        if not files:
            missing.append(f"{ecosystem}: {name} {version}")
            continue
        sections.append("\n" + "=" * 78 + f"\n{ecosystem}: {name} {version}\n"
                        + f"Declared license: {license_id or 'see upstream files'}\n"
                        + (f"Repository: {repository}\n" if repository else "") + source_note)
        for path in files:
            sections.append(f"\n--- {path.relative_to(root.resolve()).as_posix()} ---\n")
            sections.append(path.read_text(encoding="utf-8", errors="replace") + "\n")
    if missing:
        raise SystemExit("Missing upstream license/notice files:\n" + "\n".join(missing))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text("".join(sections), encoding="utf-8", newline="\n")
    print(f"Wrote notices for {len(entries)} packages to {args.output}")


if __name__ == "__main__":
    main()
