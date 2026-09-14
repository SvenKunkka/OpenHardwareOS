# Pinned upstream license material

These files fill license-text gaps in the exact crate versions in `Cargo.lock`.
`mapping.json` records each crate version, its repository, the Git commit reported
by its downloaded crate's `.cargo_vcs_info.json`, the source URL for each text,
and the SHA-256 of the stored bytes. Repository URLs were checked against the
same crate's `Cargo.toml`.

License and copyright files downloaded from the recorded upstream commits are
stored without textual changes. Identical files from different commits of one
repository are stored once; every crate mapping retains its own source commit
and URL. No SDK implementation or program binary is included here.

## selectors 0.36.1

The recorded `servo/stylo` revision does not contain a root or `selectors/`
LICENSE/NOTICE file. Its crate metadata declares `MPL-2.0`, and its source files
carry the MPL notice. `selectors/MPL-2.0.txt` is the unchanged standard license
text downloaded from the Mozilla license steward at
https://www.mozilla.org/media/MPL/2.0/index.txt (license page:
https://www.mozilla.org/MPL/2.0/). It is not represented as a file supplied by the
selectors repository. `selectors/SOURCE-LICENSE-HEADER.txt` preserves the opening
license comment from `selectors/lib.rs` at the recorded commit; that source was
byte-compared with the downloaded crate. The crate metadata credits
"The Servo Project Developers" as its authors.
