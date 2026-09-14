# Third-party notices

OpenHardwareOS source is distributed under the existing [Apache-2.0 license](LICENSE).

Windows release assets include `THIRD_PARTY_NOTICES.txt`, generated from the locked
Rust dependency metadata and the installed production frontend dependencies. It
contains dependency names, versions, declared licenses, and the upstream license
and notice files present in those packages. For packages that omit their notice
files, [pinned upstream copies](third-party/upstream-licenses/README.md) record the
source commit or official license source and the original file checksums.
The inventory includes build and test
dependencies as well as runtime dependencies; inclusion is not a claim that every
listed package is linked into every executable.

To reproduce the notices after fetching Cargo dependencies and running `npm ci`
inside `apps/desktop`:

```sh
python scripts/release/generate-notices.py --output artifacts/THIRD_PARTY_NOTICES.txt
```

LibreHardwareMonitor, PawnIO, NVIDIA drivers, and AMD SDKs are not included in the
release. Hardware adapters communicate with separately installed components.
The Windows desktop installer can download Microsoft's WebView2 runtime when it
is missing; its own license and installer terms apply. The release does not grant
licenses to those external products.
