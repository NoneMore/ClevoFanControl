# ClevoFanControl

ClevoFanControl is a native Windows x64 fan controller for supported Clevo laptops, built with Rust and FLTK. It provides visual fan curves, live monitoring, process-based presets, optional history recording, and a separate monitoring/tuning CLI.

> **Safety:** Fan control affects laptop cooling. Verify your device and fan curves before enabling software takeover. The program attempts to return control to firmware on normal exit, faults, or sleep, but restoration after a crash, forced termination, or a hung driver is **not guaranteed**.

## Features

- Adjustable CPU/GPU fan curves, stepped or linear control, hysteresis, and a one-shot cooling mode.
- Live temperature, fan RPM, and requested/actual duty monitoring.
- Named presets with automatic switching based on running processes.
- Tray integration, startup options, and configurable UI appearance.
- Configuration save/import and an optional background history recorder.
- Separate CLI for monitoring and experimental fan-curve tuning.

## Compatibility and requirements

- Windows x64; Release builds request administrator privileges.
- Official, device-matched Control Center/FnKey components and the ACPI Bridge driver installed separately. Vendor binaries are **not** bundled.
- A supported device and vendor DLL binary. The application checks compatibility when opening the hardware interface; do not assume that every Clevo model, BIOS/EC, or vendor component version is supported.
- Microsoft Visual C++ x64 runtime, if not already installed.

The application does **not** have exclusive control of the embedded controller. Other vendor utilities may overwrite fan settings. Do not run competing fan controllers or assume normal restoration can occur after an abnormal termination.

## Getting started

1. Install the manufacturer's official Control Center/FnKey components and ACPI Bridge driver for your device.
2. Extract the release to a writable folder and run `clevo-fan-control.exe`.
3. On the first launch, software takeover is disabled. Set suitable curves, then select **Save and Apply**.
4. Enable **Software Takeover** only when the curves and device compatibility have been checked. Disabling takeover attempts to return fans to firmware automatic mode.

The GUI uses Chinese labels. The names above are English descriptions of those controls.

### Configuration

Settings are saved beside the executable in `ClevoFanControl.x64.json`. Keep a backup when replacing the executable or moving to a different directory; configurations are not shared automatically between build directories.

**Import Legacy Configuration** accepts `ClevoFanControl.json` and, when present, `ClevoFanControl.presets.json` from the same directory. Import does not overwrite the original files or immediately apply the imported draft. Review it and save explicitly. A saved configuration's active settings can differ from unsaved edits in the GUI.

### Background history

Enable **Background history recording** in Settings to record telemetry as CSV and control events as JSONL. Recording continues when the window is hidden in the tray and does not require software takeover.

- Files are written to `history/` beside the executable; demo mode uses `history-demo/` and identifies simulated samples.
- History retains at most seven UTC dates and 128 MiB of managed files per directory. Copy any records you need to keep before they expire.
- The Settings panel shows recording state, location, dropped records, and errors. A storage error stops recording without intentionally stopping fan control; switch recording off and on to retry after fixing the problem.
- Buffered data may be lost if the program terminates unexpectedly. History includes device/control context and local preset names, so review it before sharing.

### CLI monitoring and tuning

`clevo-fan-cli.exe` is provided with release packages. Exit the GUI normally before using the CLI: they share a single-instance lock.

```powershell
# Monitor without taking over fan control
.\clevo-fan-cli.exe monitor --out .\monitor-session

# Experimental tuning: changes fan duty on supported hardware
.\clevo-fan-cli.exe tune --out .\tune-session --cpu-target 60 --gpu-target 50

# Inspect or request a normal stop for a session
.\clevo-fan-cli.exe status --out .\tune-session
.\clevo-fan-cli.exe stop --out .\tune-session
```

Use `--help` to inspect arguments and `--demo` for simulated operation. The `tune` command writes real fan settings, currently requires two physical fans, and should be used only with an authorized device and an explicit restoration plan. `monitor` does not take over control.

Session outputs include telemetry, events, a status report, and (for tuning) a candidate configuration. Candidates do **not** overwrite your existing configuration. Back up `ClevoFanControl.x64.json` before manually replacing it with a reviewed `candidate.json`; enable takeover explicitly after restarting.

## Build and checks

On Windows x64, install Rust/rustup, PowerShell 7, and Visual Studio Build Tools with the MSVC C++ toolchain and Windows SDK. The toolchain and dependencies are pinned by `rust-toolchain.toml` and `Cargo.lock`.

```powershell
# Format, lint, and run tests
pwsh -NoProfile -File .\scripts\check.ps1

# Also build Windows x64 Release executables
pwsh -NoProfile -File .\scripts\check.ps1 -Release

# Run the GUI with simulated hardware
cargo run --locked -- --demo
```

See [CONTRIBUTING.md](CONTRIBUTING.md) for development and verification commands. Automated tests and demo mode do not validate real hardware compatibility.

Third-party notices and license texts are in [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/](licenses/).
