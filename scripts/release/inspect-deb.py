#!/usr/bin/env python3
"""Read a Debian package and report what is inside it, as JSON.

The release pipeline has to say *what* it is publishing, not just that some file
appeared in `target/release/bundle`. A `.deb` is an `ar` archive holding
`control.tar.*` (the metadata dpkg installs) and `data.tar.*` (the files), so the
facts worth checking — the declared package name and version, the architecture,
the dependencies, the executable, the desktop entry and the icons — are all in
there and need no dpkg to read.

Reading it here, in Python, and not with `dpkg-deb`, has one purpose: the fixture
tests that guard this path run on macOS too, so the *same* code that validates a
real release artefact also validates a synthetic one. A check that only runs on
the machine that builds the package cannot be mutation-tested.

Usage:
    inspect-deb.py <file.deb>

Prints JSON on stdout. Exits non-zero, with a reason on stderr, when the file is
not a Debian package this tool can read — never a half-empty report.
"""

from __future__ import annotations

import io
import json
import shutil
import subprocess
import sys
import tarfile

AR_MAGIC = b"!<arch>\n"
HEADER_SIZE = 60
ELF_MAGIC = b"\x7fELF"

MACHINES = {
    0x03: "i386",
    0x28: "arm",
    0x3E: "x86_64",
    0xB7: "aarch64",
}
ELF_CLASS = {1: "32-bit", 2: "64-bit"}


class Unreadable(Exception):
    """The file is not a Debian package this tool can report on."""


def ar_members(blob: bytes) -> dict[str, bytes]:
    """Split an `ar` archive into {name: content}.

    Handles both the GNU and the BSD layout: the names are plain ASCII in the
    fixed-width header, which is all a `.deb` uses.
    """
    if not blob.startswith(AR_MAGIC):
        raise Unreadable("not an ar archive (a .deb starts with '!<arch>')")
    members: dict[str, bytes] = {}
    offset = len(AR_MAGIC)
    while offset + HEADER_SIZE <= len(blob):
        header = blob[offset : offset + HEADER_SIZE]
        if header[58:60] != b"`\n":
            raise Unreadable(f"broken ar header at offset {offset}")
        raw_name = header[0:16].decode("ascii", "replace").strip()
        if not raw_name:
            break
        # GNU stores `name/`; BSD stores `#1/<len>` with the name in the payload.
        name = raw_name.rstrip("/")
        try:
            size = int(header[48:58].decode("ascii").strip())
        except ValueError as error:
            raise Unreadable(f"unreadable member size for {name!r}") from error
        start = offset + HEADER_SIZE
        payload = blob[start : start + size]
        if raw_name.startswith("#1/"):
            try:
                name_length = int(raw_name[3:])
            except ValueError as error:
                raise Unreadable(f"unreadable BSD name length {raw_name!r}") from error
            name = payload[:name_length].decode("ascii", "replace")
            payload = payload[name_length:]
        members[name] = payload
        offset = start + size + (size % 2)  # members are padded to an even offset
    if not members:
        raise Unreadable("the ar archive has no members")
    return members


def decompress_tar(name: str, payload: bytes) -> tarfile.TarFile:
    """Open a tar member, whatever compression dpkg used for it."""
    try:
        return tarfile.open(fileobj=io.BytesIO(payload), mode="r:*")
    except tarfile.ReadError as error:
        if not name.endswith(".zst"):
            raise Unreadable(f"{name} is not a readable tar archive: {error}") from error
        # dpkg 1.21+ defaults to zstd, which Python's tarfile cannot read on its
        # own. Use the system tool when it is there and say so plainly when it
        # is not, rather than reporting an empty package.
        zstd = shutil.which("zstd")
        if zstd is None:
            raise Unreadable(
                f"{name} is zstd-compressed and the `zstd` tool is not installed; "
                "install zstd (Debian/Ubuntu: apt-get install zstd) so the package "
                "can be read"
            ) from error
        result = subprocess.run(
            [zstd, "-dc"], input=payload, stdout=subprocess.PIPE, stderr=subprocess.PIPE
        )
        if result.returncode != 0:
            raise Unreadable(f"{name} could not be decompressed: {result.stderr.decode().strip()}")
        return tarfile.open(fileobj=io.BytesIO(result.stdout), mode="r:")


def parse_control(text: str) -> dict[str, str]:
    """Parse a Debian control file into its fields.

    Continuation lines (a field whose value spans lines, indented by one space —
    `Description` always does) are folded into the field they belong to.
    """
    fields: dict[str, str] = {}
    current: str | None = None
    for line in text.splitlines():
        if not line.strip():
            continue
        if line.startswith((" ", "\t")) and current:
            fields[current] += "\n" + line.strip()
            continue
        if ":" not in line:
            continue
        key, _, value = line.partition(":")
        current = key.strip()
        fields[current] = value.strip()
    return fields


def parse_desktop_entry(text: str) -> dict[str, str]:
    """Parse a `.desktop` file's `[Desktop Entry]` group.

    Freedesktop entries are `Key=Value`, not control-file `Key: Value`, so this is
    deliberately not the control parser: reading them with the wrong one silently
    returns nothing, which is how "the desktop entry is there" can be true while
    "the desktop entry says what application it launches" is unchecked.
    """
    fields: dict[str, str] = {}
    in_group = False
    for line in text.splitlines():
        stripped = line.strip()
        if stripped.startswith("["):
            in_group = stripped == "[Desktop Entry]"
            continue
        if not in_group or not stripped or stripped.startswith("#") or "=" not in line:
            continue
        key, _, value = line.partition("=")
        fields[key.strip()] = value.strip()
    return fields


def elf_profile(payload: bytes) -> dict[str, object] | None:
    """Describe an ELF file, or return None when it is not one."""
    if not payload.startswith(ELF_MAGIC) or len(payload) < 20:
        return None
    return {
        "class": ELF_CLASS.get(payload[4], f"unknown({payload[4]})"),
        "machine": MACHINES.get(
            int.from_bytes(payload[18:20], "little"),
            f"unknown({int.from_bytes(payload[18:20], 'little')})",
        ),
    }


def report(path: str) -> dict[str, object]:
    with open(path, "rb") as handle:
        blob = handle.read()
    members = ar_members(blob)

    control_name = next((n for n in members if n.startswith("control.tar")), None)
    data_name = next((n for n in members if n.startswith("data.tar")), None)
    if control_name is None or data_name is None:
        raise Unreadable(
            "a .deb needs a control.tar.* and a data.tar.* member; found "
            + ", ".join(sorted(members))
        )

    with decompress_tar(control_name, members[control_name]) as archive:
        control_text = ""
        for entry in archive.getmembers():
            if entry.name.lstrip("./") == "control":
                extracted = archive.extractfile(entry)
                control_text = extracted.read().decode("utf-8", "replace") if extracted else ""
                break
    if not control_text:
        raise Unreadable("control.tar has no `control` file")

    data_files: list[dict[str, object]] = []
    linux_binary = ""
    with decompress_tar(data_name, members[data_name]) as archive:
        for entry in archive.getmembers():
            clean = entry.name.lstrip("./")
            item: dict[str, object] = {"path": clean, "mode": entry.mode, "size": entry.size}
            if entry.isfile() and clean.startswith("usr/bin/"):
                handle = archive.extractfile(entry)
                profile = elf_profile(handle.read(64)) if handle else None
                item["elf"] = profile
                if profile and linux_binary == "":
                    linux_binary = clean
                elif not profile:
                    item["elf"] = None
            data_files.append(item)

    desktop = next((f for f in data_files if str(f["path"]).endswith(".desktop")), None)
    desktop_fields: dict[str, str] = {}
    if desktop:
        with decompress_tar(data_name, members[data_name]) as archive:
            for entry in archive.getmembers():
                if entry.name.lstrip("./") == desktop["path"]:
                    handle = archive.extractfile(entry)
                    if handle:
                        desktop_fields = parse_desktop_entry(
                            handle.read().decode("utf-8", "replace")
                        )
    icons = [str(f["path"]) for f in data_files if "/icons/" in str(f["path"])]

    return {
        "file": path,
        "size": len(blob),
        "members": sorted(members),
        "debian_binary": members.get("debian-binary", b"").decode("ascii", "replace").strip(),
        "control": parse_control(control_text),
        "data_file_count": len(data_files),
        "data_files": [str(f["path"]) for f in data_files],
        "binary": linux_binary,
        "binaries": [f for f in data_files if f.get("elf")],
        "desktop_entry": desktop["path"] if desktop else "",
        "desktop_fields": desktop_fields,
        "icons": icons,
    }


def main() -> int:
    if len(sys.argv) != 2:
        print("usage: inspect-deb.py <file.deb>", file=sys.stderr)
        return 2
    try:
        print(json.dumps(report(sys.argv[1]), indent=1, sort_keys=True))
    except Unreadable as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    except OSError as error:
        print(f"error: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
