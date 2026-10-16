# Development Guide

This project targets Windows x64. See the [README](README.md) for usage, [architecture guide](docs/ARCHITECTURE.md) for entry points and module responsibilities, and [NH5xAx](docs/NH5XAX.md) for hardware validation records.

## Environment Setup

1. Install Git, PowerShell 7, Rust/rustup, and Visual Studio Build Tools with the MSVC C++ toolchain and Windows SDK.
2. After cloning, run `rustup show` at the repository root and confirm the toolchain and target specified in `rust-toolchain.toml`.
3. Initial Cargo commands need network access to download dependencies and the `fltk-bundled` precompiled library. Dependency versions are defined by `Cargo.lock`.

Building, automated tests, and demo mode do not require vendor DLLs or drivers. Real hardware access requires the corresponding official components; see the README compatibility and usage sections.

## Routine Checks

Run from the repository root:

```powershell
# Formatting, Clippy with warnings treated as errors, unit and integration tests
pwsh -NoProfile -File .\scripts\check.ps1

# Run the checks above, then build Windows x64 Release for CI and releases
pwsh -NoProfile -File .\scripts\check.ps1 -Release
```

The script switches to the repository root and restores the caller's working directory on completion. It stops immediately and reports failure if any step fails. CI and release workflows use the same script; maintain shared checks there.

If a program under `target/debug/` is running, Windows may prevent Cargo from replacing its executable. Exit the program normally before checking, or use a separate build directory in a separate PowerShell session:

```powershell
$env:CARGO_TARGET_DIR = Join-Path $PWD 'target/dev-check'
pwsh -NoProfile -File .\scripts\check.ps1 -Release
```

This setting affects only the current session and its child processes. Exiting that session restores the default build directory. A separate directory requires recompiling dependencies on first use and stores its outputs there.

For individual iterations, you can run:

```powershell
cargo fmt
cargo test --locked --test core
cargo test --locked --test nh5xax
cargo test --locked --lib worker::tests
cargo test --locked --bin clevo-fan-cli
```

Before submitting, validate according to the change scope. Run default checks for Rust changes; add `-Release` for build, resource, script, or release workflow changes. Documentation-only changes require content, link, and Git diff checks. When changing dependencies or the toolchain, also review the lockfile, README technology stack, third-party notices, and `licenses/`.

## Execution and Validation Levels

| Validation | Entry point | What it establishes |
| --- | --- | --- |
| Automated checks | `scripts/check.ps1` | Formatting, static checks, simulated hardware, and data logic regressions |
| Release build | `scripts/check.ps1 -Release` | Windows target compilation, linking, and resource compilation |
| Demo GUI | `cargo run --locked -- --demo` | UI interactions and simulated control flow; requires a desktop session |
| Demo GUI smoke check | `cargo run --locked -- --demo --smoke` | Startup, temporary screenshot, and exit paths; confirm normal process termination |
| Hardware sampling | `cargo run --locked -- --probe` | Current model, DLL, and telemetry reads |
| Hardware writes | README `--hardware-check` and CLI `tune` | Writes, readback, and restoration on the corresponding machine and scenario |

The shared check script performs only automated checks and an optional build. GUI checks, real sampling, and real writes are separate acceptance steps. The demo GUI and CLI share the single-instance lock; exit existing control programs normally before running them. The demo UI can still save configuration beside its executable, so use a separate build directory or back up configuration before experimenting.

Hardware writes change fan state. Establish the machine, operations, and restoration steps before validation. Record the model, BIOS/EC, vendor components, DLL hash, operations, observations, and restoration result. Automated tests or one successful hardware test do not establish acceptance for other models, sleep/resume, or long-term operation. Restoration after forced termination or a hung driver is not guaranteed.

## Changes and Review

- For configuration changes, check legacy defaults, strict validation, migration, and original file protection. For control changes, check fault, takeover cancellation, exit, and sleep/resume paths.
- UI editing drafts, saved configuration, and the worker's active configuration have different lifecycles. Explain when affected actions submit, save, and take effect.
- Commit or PR descriptions should state the problem, resulting behavior, verification actually performed, and scenarios still awaiting acceptance. Tests should cover behavior or fault boundaries.

## Files and Artifacts

| Path | Management |
| --- | --- |
| `src/`, `tests/`, `assets/`, build files | Source and necessary inputs; version controlled |
| `Cargo.lock`, `rust-toolchain.toml` | Application dependency and toolchain baselines; version controlled |
| `docs/` | Architecture and validation records; version controlled; link new documents from an appropriate entry point |
| `CONTROL_OWNERSHIP_RESEARCH.md` | Historical protocol and ownership research; preserve dates, baseline, and evidence boundaries |
| `licenses/`, `THIRD_PARTY_NOTICES.md` | Distribution notices and third-party licenses; review with dependency changes |
| `target/` | Builds, raw tuning records, and local experiment outputs; ignored by Git |
| Root `ClevoFanControl*.json` | Local runtime configuration; ignored by the specific filenames in `.gitignore` |
| Root `ClevoFanControl-*.zip` | Local release or tuning archives; ignored by Git |

Use `target/tuning/<unique-session-name>/` for tuning to avoid overwriting existing records. Preserve conclusions in `docs/` and identify the original evidence location. A local archive of raw records does not mean they are stored in the repository. Do not commit vendor DLLs, drivers, or personal runtime configurations with source or release packages.

See `.editorconfig` for shared editor conventions; use `cargo fmt` for Rust formatting. Use UTF-8 and LF for new files without bulk rewriting existing files solely for cleanup.
