# Third-party components

This project uses third-party Rust crates and a bundled FLTK library. Dependency versions are recorded in `Cargo.lock`, and the available license and notice texts are included under `licenses/`.

- fltk-rs / fltk-sys / cfltk: MIT. The bundled FLTK library uses LGPL with static-linking exceptions; see `licenses/FLTK-COPYING.txt`. Notices for bundled PNG, JPEG, and zlib components are also included.
- Serde, serde_json, windows-sys, libloading, sha2, regex, and transitive dependencies retain their respective licenses; see the versioned files in `licenses/`.
- Existing image assets in `assets/` retain their original rights. This repository does not assert new ownership or grant rights beyond those held by their respective owners.
- `InsydeDCHU.dll` and `AcpiBridge.sys` are proprietary vendor components installed separately through the manufacturer's software and are **not distributed** with this application.

The distributable archive includes executable files, the README, this notice, and the `licenses/` directory.
