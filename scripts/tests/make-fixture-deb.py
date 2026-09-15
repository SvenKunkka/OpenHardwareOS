#!/usr/bin/env python3
"""Build a synthetic `.deb` for the release fixtures. **Test utility, not product code.**

The packager refuses to publish a Debian package that is not a Debian package, and
the checks that do the refusing have to be exercised on every platform — including
the macOS machine where the rest of the suite runs. This builds a `.deb` with the
same structure a real one has (an `ar` container holding `control.tar.*` and
`data.tar.*`), and can leave out or corrupt any single part so a test can prove the
check fails for the right reason rather than for a typo.

Usage:
    make-fixture-deb.py OUT.deb --version 0.1.4 [--arch amd64] [--package openhardwareos]
        [--binary-elf | --binary-script | --binary-missing]
        [--desktop-missing] [--icon-missing] [--depends "..."] [--compression gz|xz|none]
"""

from __future__ import annotations

import argparse
import io
import os
import tarfile

# A minimal but genuine x86_64 ELF header: the packager checks the class and the
# machine field, so the fixture has to carry bytes that answer both.
ELF_X86_64 = bytes.fromhex("7f454c46") + bytes([2, 1, 1, 0]) + bytes(8) + bytes([2, 0, 0x3E, 0])
ELF_X86_64 += bytes(64 - len(ELF_X86_64))

SCRIPT_BINARY = b'#!/bin/sh\necho "this is not an ELF file"\n'

ICON_PNG = bytes.fromhex("89504e470d0a1a0a") + bytes(64)

DESKTOP_ENTRY = """[Desktop Entry]
Type=Application
Name=OpenHardwareOS
Comment=Open runtime, device layer and automation engine for PC hardware
Exec=openhardwareos
Icon=openhardwareos
Terminal=false
Categories=Utility;
""".encode()


def tar_bytes(files: dict[str, tuple[bytes, int]]) -> bytes:
    """Build a tar archive from {name: (content, mode)}."""
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w") as archive:
        for name, (content, mode) in files.items():
            info = tarfile.TarInfo(name)
            info.size = len(content)
            info.mode = mode
            archive.addfile(info, io.BytesIO(content))
    return buffer.getvalue()


def compress(payload: bytes, how: str) -> tuple[str, bytes]:
    if how == "none":
        return ".tar", payload
    if how == "gz":
        import gzip

        return ".tar.gz", gzip.compress(payload, mtime=0)
    if how == "xz":
        import lzma

        return ".tar.xz", lzma.compress(payload)
    raise SystemExit(f"unknown compression {how!r}")


def ar(container: list[tuple[str, bytes]]) -> bytes:
    """Wrap members in the GNU `ar` layout a .deb uses (names end in `/`)."""
    out = bytearray(b"!<arch>\n")
    for name, payload in container:
        header = f"{name + '/':<16}{0:<12}{0:<6}{0:<6}{0o100644:<8}{len(payload):<10}`\n".encode()
        assert len(header) == 60, len(header)
        out += header + payload
        if len(payload) % 2:
            out += b"\n"
    return bytes(out)


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("out")
    parser.add_argument("--version", default="0.1.4")
    parser.add_argument("--arch", default="amd64")
    parser.add_argument("--package", default="openhardwareos")
    parser.add_argument("--depends", default="libwebkit2gtk-4.1-0, libgtk-3-0")
    parser.add_argument("--binary-elf", action="store_true")
    parser.add_argument("--binary-script", action="store_true")
    parser.add_argument("--binary-missing", action="store_true")
    parser.add_argument("--desktop-missing", action="store_true")
    parser.add_argument("--icon-missing", action="store_true")
    parser.add_argument("--exec-name", default="openhardwareos")
    parser.add_argument("--compression", default="gz", choices=["gz", "xz", "none"])
    args = parser.parse_args()

    control = (
        f"Package: {args.package}\n"
        f"Version: {args.version}\n"
        f"Architecture: {args.arch}\n"
        "Maintainer: OpenHardwareOS <noreply@example.invalid>\n"
        f"Depends: {args.depends}\n"
        "Section: utils\n"
        "Priority: optional\n"
        "Description: Open runtime, device layer and automation engine for PC hardware\n"
        " Reads hardware state and automates cooling.\n"
    ).encode()

    files: dict[str, tuple[bytes, int]] = {
        "usr/share/icons/hicolor/128x128/apps/openhardwareos.png": (ICON_PNG, 0o644),
    }
    if not args.binary_missing:
        binary = SCRIPT_BINARY if args.binary_script else ELF_X86_64
        files["usr/bin/openhardwareos"] = (binary, 0o755)
    if not args.desktop_missing:
        entry = DESKTOP_ENTRY.replace(b"Exec=openhardwareos", f"Exec={args.exec_name}".encode())
        files["usr/share/applications/openhardwareos.desktop"] = (entry, 0o644)
    if args.icon_missing:
        files.pop("usr/share/icons/hicolor/128x128/apps/openhardwareos.png")

    control_suffix, control_payload = compress(
        tar_bytes({"./control": (control, 0o644)}), args.compression
    )
    data_suffix, data_payload = compress(tar_bytes(files), args.compression)

    blob = ar(
        [
            ("debian-binary", b"2.0\n"),
            (f"control{control_suffix}", control_payload),
            (f"data{data_suffix}", data_payload),
        ]
    )
    with open(args.out, "wb") as handle:
        handle.write(blob)
    os.chmod(args.out, 0o644)
    print(f"wrote {args.out} ({len(blob)} bytes)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
