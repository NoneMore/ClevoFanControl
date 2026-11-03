# Development Guide

The application targets Windows x64. See [README.md](README.md) for installation, features, and usage.

## Setup

Install Git, PowerShell 7, Rust/rustup, Visual Studio Build Tools (MSVC C++), and the Windows SDK. Run `rustup show` at the repository root to confirm the pinned toolchain in `rust-toolchain.toml`. Initial builds download dependencies and the bundled FLTK library.

## Checks

Run from the repository root:

```powershell
pwsh -NoProfile -File .\scripts\check.ps1
pwsh -NoProfile -File .\scripts\check.ps1 -Release
```

The script checks formatting, Clippy warnings, and tests; `-Release` also compiles the Windows x64 Release executables. To run a single test target, use `cargo test --locked --test core` or another target listed under `tests/`.

Exit running debug executables before rebuilding, or isolate the output directory:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target/dev-check'
pwsh -NoProfile -File .\scripts\check.ps1 -Release
```

## Manual verification

```powershell
cargo run --locked -- --demo
cargo run --locked -- --demo --smoke
cargo run --locked -- --probe
```

Demo mode uses simulated hardware but can still persist configuration. Isolate its output/configuration or back up your settings first. `--probe` performs read-only hardware sampling.

Real hardware writes, including `--hardware-check` and CLI `tune`, affect fan settings. Only run them on an authorized compatible device, with a plan to return control to firmware. Normal exit attempts restoration, but forced termination and hung drivers cannot guarantee it. Automated checks do not exercise real hardware writes.

The GUI integration test in `tests/gui_history.rs` includes an ignored desktop workflow; it is not run by default and requires an isolated test directory and no other running controller.

Document the checks actually run and any unverified scenarios in a PR. Never commit local device settings, vendor drivers/DLLs, or generated history records. Review `THIRD_PARTY_NOTICES.md` and `licenses/` whenever dependencies change.
