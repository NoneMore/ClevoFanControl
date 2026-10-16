# ClevoFanControl

A native Windows x64 fan controller for Clevo laptops, written in **Rust + FLTK**, with visual fan curves, real-time monitoring, and process-based configuration presets.

The project was inspired by **myfancontrol**, but the current implementation is largely independent: the UI, control logic, and hardware communication have been reimplemented. This version communicates with the EC through installed vendor ACPI components. It **no longer uses or distributes `NTPortDrvSetup.exe`, `ClevoEcInfo.dll`, or `NVGPU_DLL.dll`**, and does not provide the legacy GPU frequency limiting feature.

> This project was developed by **gpt-6-astra@medium**. **Use at your own risk.** Fan control directly affects cooling; verify your curves and device compatibility. Compatibility with every Clevo model and restoration of firmware automatic control after abnormal termination are not guaranteed.

## Language and Technology Stack

| Purpose | Technology |
| --- | --- |
| Programming language | Rust, Edition 2024; toolchain pinned to 1.97.1 |
| UI | fltk-rs / fltk-sys 1.5.23 with the `fltk-bundled` precompiled FLTK library |
| Configuration serialization | Serde, serde_json |
| Windows integration | windows-sys: tray, single-instance enforcement, process enumeration, system appearance, power events, and more |
| Vendor interface loading and validation | libloading, sha2 |
| Concurrency and scheduling | Rust standard library threads and message channels; dedicated fan control worker thread |
| Build and release | Cargo, winresource, MSVC / Windows SDK, GitHub Actions |

Dependency versions are defined by [Cargo.lock](Cargo.lock); see [rust-toolchain.toml](rust-toolchain.toml) for the toolchain.

Start development with the [development guide](CONTRIBUTING.md). See the [architecture guide](docs/ARCHITECTURE.md) for module responsibilities and state boundaries.

## Main Features

- **Visual curve editing**: separate CPU/GPU curves with 2–16 nodes each; dragging, double-click insertion, keyboard adjustments, and numeric editing.
- **Control policies**: stepped or linear curves, cooling hysteresis, and smooth duty changes; software takeover and return to firmware automatic control. ~~The third fan uses the GPU target~~.
- **One-shot forced cooling**: runs at 95% duty until temperature drops below the configured threshold; takes priority over normal curves and is not saved as a persistent toggle.
- **Real-time monitoring**: temperature, RPM, actual and target duty. CPU/GPU history advances by sample, supports 10–300 points, and defaults to 60. Missing samples leave gaps; a dashed line reconnects after recovery.
- **Optional background history**: independently saved recording switch, CSV telemetry and JSONL control events; continues in the tray without additional hardware or GPU workload queries. See [GUI history recording](#gui-history-recording).
- **Adjustable sampling**: 200–10000 ms, default 2000 ms; the actual period depends on hardware call duration.
- **Presets and automatic switching**: up to 32 presets, manual application, ordering, and running-process matching. Rules support `*` / `?` wildcards; the first match in priority order wins, falling back to global configuration when none match. Automatic switching does not overwrite the editing draft.
- **Desktop integration and appearance**: tray show/hide/exit, close to tray, Task Scheduler login startup; light, dark, or system theme; system or installed fonts and 8–16pt sizing.
- **Configuration and fault feedback**: atomic JSON saves and legacy import; sampling or write failures stop curve writes and attempt automatic restoration, with an error shown in the UI. Normal exit and suspension for sleep also attempt restoration.

## Validated Environment

The following describes one locally validated environment, **not a general compatibility list**.

| Item | Version / model | Evidence |
| --- | --- | --- |
| Operating system | Windows 11 Pro for Workstations, 24H2, 26100.7462, x64 | Local system query |
| Board / model | Notebook `NP5x_6x_7x_SNx` | Local SMBIOS; no valid board revision reported |
| BIOS | `1.07.27RTR6` | Local SMBIOS |
| EC | `1.07.07TR3` | Local EC |
| Control Center | Control Center Package / ControlCenter 3.0 Package `v6.021` | Local installation records |
| FnKey / hotkey and OSD package | `CLEVOCO.FnhotkeysandOSD` `7.88.1.0` | Package and `FnKey.exe` versions agree |
| ACPI Bridge driver | `AcpiBridge.sys` `1.0.0.9` | Device `ACPI\CLV0001\1` |
| Validated communication DLL | x64 `InsydeDCHU.dll` `1.0.0.1` in the FnKey directory | Binary identified by SHA256 allowlist |
| Physical fans | CPU/GPU dual fans | Third fan tested only in simulation |

SHA256 of the FnKey `InsydeDCHU.dll` used for hardware validation:

```text
75a47020d3a9d052e94dcb4e3ad61fb69f20177dd46e20c9a20883d92b83981b
```

Identical file versions do not imply identical binaries. The program first searches the installed FnKey package, then Control Center directories, and loads only DLLs matching the code allowlist. Official component updates may require reviewing the protocol and hash again; version `1.0.0.1` alone does not establish compatibility.

Existing hardware records validate specified-duty writes to both fans, actual-value readback, and automatic restoration while FnKey/Control Center remained running in the background. Other models, BIOS versions, a third physical fan, real sleep/resume, logoff/shutdown, actual login triggers, Explorer restart, cross-monitor DPI, and long-term stability still require validation in the corresponding scenarios.

## Usage

The interface currently uses Chinese labels. Action names below are English translations of those labels.

1. Install the official Control Center/FnKey and ACPI Bridge driver matching your machine; vendor components are not bundled. They are usually already installed.
2. Ensure the Microsoft Visual C++ x64 runtime (VCRUNTIME140.dll / UCRT) is available. Place the program in a writable directory so it can save configuration. The runtimes are usually already installed.
3. Run `clevo-fan-control.exe`. Release builds request administrator privileges.
4. First launch leaves takeover disabled. Edit curves and click Save and Apply, then enable Software Takeover as needed. Disabling takeover returns control to firmware automatic mode.

Duty is not a constant RPM. Adjust curves for your device's cooling capacity before use.

The main window's editing draft is independent of the active configuration. Save and Apply submits edits; the takeover toggle does not implicitly submit them, and automatic preset switching does not overwrite them. Restore Global uses the saved global configuration in memory; Reload reads configuration from disk.

### Configuration and Migration

Configuration is stored beside the executable as `ClevoFanControl.x64.json`, using `schemaVersion: 1`. Global settings and presets share one file. The size limit is 64 KiB; unknown fields, duplicate fields, incorrect types, and out-of-range data are rejected.

When moving to a build in another directory, copy your existing configuration beside the new executable before launching it, keeping the original as a backup. A directory without this file starts with defaults; configurations are not shared automatically between build directories.

Import Legacy Configuration accepts the old `ClevoFanControl.json` and reads `ClevoFanControl.presets.json` from the same directory if present. Import preserves the original files and only updates the draft. The imported configuration has takeover, auto-start, and automatic matching disabled; the old forced cooling toggle is not restored. Changes take effect only after saving.

## How It Works

```text
FLTK UI: edit draft, submit commands, display snapshots and history
    ↕ Rust message channels
Control worker: sampling, preset matching, curve calculation, writes and readback checks
    ↓
Installed x64 InsydeDCHU.dll
    ↓
AcpiBridge.sys → ACPI DCHU._DSM → PK04 / EC.ECMD → fans
```

The program loads the hash-validated vendor library using its full path and a restricted DLL search. Commands `0x0C` / `0x0D` read fan count, temperature, actual duty, and tachometer counters. RPM is calculated with the counter formula used by the official FanSpeedSetting UI.

Writes send EC `C1` commands per channel through `PK04(1)`, avoiding the `0x68` bulk write that affects four channels at once. Returning control to firmware uses the `0x69` bitmask. The algorithms and UI do not directly access low-level I/O ports or install a custom kernel driver.

**The program does not have exclusive EC ownership.** FnKey/Control Center may overwrite targets at startup, resume, or configuration changes. The control worker checks actual duty in later samples and corrects deviations. This does not prevent other programs from writing, nor guarantee automatic restoration after a crash, forced termination, or a hung driver. See the [hardware research record](CONTROL_OWNERSHIP_RESEARCH.md) for protocol and ownership analysis.

## Build and Test

Requires Windows x64, Rust/rustup, and Build Tools with the MSVC C++ toolchain and Windows SDK. `rust-toolchain.toml` pins the toolchain. The first build downloads Rust dependencies and the `fltk-bundled` precompiled library.

Run in PowerShell from the repository root:

```powershell
pwsh -NoProfile -File .\scripts\check.ps1 -Release
```

The check script requires PowerShell 7. It runs `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`, and `cargo test --locked` by default; `-Release` adds a Windows x64 Release build. See the development guide for individual checks and environment setup.

The GUI output is `target/x86_64-pc-windows-msvc/release/clevo-fan-control.exe`. GitHub Actions uses the same check script. The manual release workflow requires the tag to match the `Cargo.toml` version. Release packages include the programs, README, third-party notices, and license directory, without vendor DLLs/drivers.

Current automated tests cover curves and hysteresis, forced cooling, third-fan following, fault recovery, correction after external overwrites, takeover cancellation, configuration validation and import, history charts, and some UI appearance behavior. They use simulated hardware and do not replace hardware acceptance.

Development diagnostics:

```powershell
cargo run -- --demo                 # Simulated hardware with the full UI
cargo run -- --demo --smoke         # Demo startup, temporary screenshot, and exit
cargo run -- --probe                # Read-only hardware sample: DLL path, hash, and telemetry
cargo run -- --hardware-check       # Write 40%, wait 2 seconds, read back, then attempt automatic restoration
```

Debug uses `asInvoker`; Release uses `requireAdministrator`. Real hardware diagnostics may require an administrator terminal. Release is recommended for routine GUI use because it has no console window; Debug keeps the console for diagnostics. `--hardware-check` changes fan duty and is not a routine test command. `--smoke` writes a screenshot to the system temporary directory; earlier records included an exit timeout, so it does not establish acceptance for all GUI scenarios. Normal execution does not generate error logs or screenshots.

### GUI History Recording

In Settings, enable **Background history recording**. New configurations and older files without `historyRecordingEnabled` default to disabled. Enabling saves that preference before starting, independently of Save and Apply. The setting survives restart; each enable or subsequent launch starts a new session. Toggling recording does not submit curve drafts, save other pending settings, change takeover, or register auto-start. Disabling immediately stops accepting observations and then saves the preference. If saving fails, the current session remains off and the GUI warns that the saved preference might enable recording next launch. Enabling fails without starting if the preference cannot be saved. An invalid original configuration must be explicitly repaired before recording preferences can be saved.

Files are local to `history/` beside the executable; `--demo` uses `history-demo/` and marks data simulated. The recording status in Settings shows the directory, actual state, lost-record count and errors, independently of the saved switch. Recording continues when the window is hidden and when firmware controls the fans. Live chart point limits do not limit file history. Output/open/flush/rotation/cleanup failures stop recording and show an error while fan control continues. After fixing an error, switch off and on to retry. If an old writer is still stopping, the requested new session waits for its completion; it never creates additional stuck writers.

- Paired `cfc-history-v1-<UTC-date>-<session>-<segment>.csv` and `.events.jsonl` segments rotate at UTC midnight or before exceeding 8 MiB combined. Names are unique and previous sessions are never overwritten.
- Keep the current UTC date plus the preceding six dates and at most **128 MiB per history directory**. Oldest closed managed segments are automatically deleted to keep recording. Copy evidence you want to retain elsewhere. Cleanup is nonrecursive, rejects managed reparse points, and preserves unrelated files and configurations. Once recording is off and accepted data has finished finalizing, no retention cleanup runs.
- The writer uses a **256-record bounded queue**, discards newest records on saturation, and exposes a cumulative loss count. A full effective-state checkpoint starts every segment and follows state changes or loss recovery, so retained segments remain interpretable. A loss can include a control event, not just a temperature sample.
- Buffers flush on a one-second schedule when storage responds, at rotation and at finalization. There is no per-sample durable disk sync. Forced termination can leave a buffered tail, incomplete CSV/JSON record or missing session-end event. An operation with a start event and no result remains unknown. Normal exit attempts fan restoration first and waits at most two seconds for recorder finalization; a hung driver is outside that timeout.

CSV format version 1 fields are `format_version`, `session_id`, `sequence`, `sample_unix_ms`, `elapsed_ms`, `channel` (1-based), `temperature_c`, `rpm`, `actual_duty_percent`, `curve_duty_percent`, `requested_duty_percent`, `reading_status`, `mode`, `active_preset`, and `simulated`. Times use Unix milliseconds and monotonic session milliseconds. Empty measurements mean unavailable; measured zero remains zero. Quoted CSV fields can contain commas, quotes or newlines; use a CSV parser rather than splitting lines.

`curve_duty_percent` is the evaluated demand for an accepted sample. `requested_duty_percent` is the last successfully issued software request at acquisition, empty when unavailable or firmware-controlled. Subsequent `write_start` / `write_result` JSONL events contain new requested duties and correlate through `observation` to that sample. The readings are not a readback of that subsequent request. A temperature jump preserves the first read as `verification_required`; the existing retry references it through `verification_of`. This adds no hardware reads. Failed samples are events without fabricated channel rows.

JSONL records include `format_version`, `session_id`, `sequence`, `observation`, `unix_ms`, `elapsed_ms`, `kind`, and `detail`; checkpoint records carry full effective control settings, mode, preset, hardware description, application version and cumulative drops. Events cover state changes, sampling failures, write/restore start and actual results, suspend/resume, and session end. No samples are synthesized across sleep. Sequence gaps and drop counters help distinguish loss from gaps in sampling. CSV and JSONL writes are not a cross-file transaction: after interruption, consume complete records and tolerate a missing counterpart.

History contains local preset names, effective fan curves and hardware description. It does not include process lists, workload utilization/power, or unsaved drafts, and does not launch `nvidia-smi`. Existing CLI outputs remain separate; CLI candidates explicitly disable the GUI recorder. Older binaries with strict JSON parsing can reject `historyRecordingEnabled`: exit normally and restore a configuration backup or remove that field before downgrading. Removing it defaults recording off and does not delete history.

See the [GUI history validation record](docs/GUI_HISTORY_VALIDATION.md) for simulated acceptance, performance measurements and outstanding hardware scenarios.

### CLI Sampling and Ordinary-Workload Tuning

The separate console program is `clevo-fan-cli.exe`. Debug outputs are under `target/debug/`; Release outputs share the GUI directory. It shares the GUI's single-instance lock. Exit the GUI normally and disable other tools' software takeover before running it.

```powershell
cargo build --locked --bin clevo-fan-cli
# Record for 4 hours without taking over fans; use a unique output directory each time
.\target\debug\clevo-fan-cli.exe monitor --out .\target\tuning\monitor-01
# Tune for 4 hours with a 10-minute firmware baseline; CPU target 60°C (stable band 60–65°C), GPU about 50°C
.\target\debug\clevo-fan-cli.exe tune --out .\target\tuning\session-01 --cpu-target 60 --gpu-target 50
# Inspect records / stop early (stop does not need the hardware lock)
.\target\debug\clevo-fan-cli.exe status --out .\target\tuning\session-01
.\target\debug\clevo-fan-cli.exe stop --out .\target\tuning\session-01
```

Defaults: `--duration-secs` is 14400, `--baseline-secs` is 600, and `--interval-secs` is 2. Real tuning requires at least a 60-second baseline. Use `--config FILE` for a configuration, `--demo` for simulated flow, and `--help` for arguments. Only `tune` writes hardware and currently requires two physical fans; `monitor` supports 1–3 channels.

Tuning prioritizes stable fan speed: temperatures use a 15-second median and 30-second EMA; duty has a two-percentage-point deadband. Normal changes are at least 30 seconds apart, increasing by at most two percentage points or decreasing by at most one; decreases also wait 120 seconds. Sustained excess heat accelerates increases. Raw CPU ≥80°C / GPU ≥75°C bypasses filtering and delays to request 100%. After protection triggers, temperature must remain at least 5°C below the trigger for 30 seconds before restoring the higher of pre-protection duty and current curve demand, avoiding a prolonged full-speed period under ordinary decrease limits. The GUI displays curve demand and actual request separately, with a high-temperature protection indicator. CPU ≥85°C / GPU ≥80°C ends the session and attempts firmware automatic restoration. These are experimental protection thresholds for this session, not chip vendor temperature ratings.

A window is evaluated every three minutes. Only curve nodes near the visited temperature range are adjusted: decrease by two percentage points when cool and stable; increase by four when persistently hot. The CPU target allows a +5°C stable band; GPU allows +2°C. Curve seed minimums are CPU 25% and GPU 40%; high-temperature nodes are preserved. On takeover or control resumption, duty is initialized directly from the curve to avoid prolonged deviation inherited from firmware fan-stop or temporary high speed; raw-temperature protection still takes precedence. Subsequent changes use the stable response cadence. CPU and GPU are tuned separately without assuming their cooling capacity is interchangeable. The method does not measure sound pressure or tone, and ordinary-workload results do not establish an optimal curve for all workloads.

Each session outputs:

- `telemetry.csv`: time, phase, channel, raw temperature, actual duty, RPM, and next requested duty; flushed on every sample.
- `events.jsonl`: start, curve adjustments, and termination reason.
- `report.json`: status updated every 30 seconds. Baseline and adaptive phases separately track temperature distributions, sums needed for means, duty change counts, and total change magnitude. Restoration results are written at completion. Workloads may differ between phases, so these figures alone cannot establish noise savings or cooling improvements.
- `candidate.json`: a usable current-format configuration with `stableResponse` enabled, retaining temperature filtering and change cadence. Takeover, auto-start, and automatic matching are disabled; it does not overwrite the original configuration. Updated atomically when curves change and saved again at completion.
- `session.json`: session process and mode. `stop` creates a `STOP` request; inspect `report.json` for final state and restoration results.
- `load.csv`: real mode starts a separate recorder that samples total CPU utilization every five seconds, plus NVIDIA GPU 0 core utilization, memory controller utilization, power, core/memory clocks, temperature, and query duration.

Load recording calculates CPU utilization from differences in Windows aggregate kernel/user/idle time; CPU package power is not included. Memory controller utilization is not memory capacity usage. Align fan records using `unix_ms`. GPU queries use `nvidia-smi.exe` from the installed NVIDIA driver. Missing measurements retain empty fields and error codes rather than becoming zero. Query timeouts or failures do not affect EC control; recorder errors appear in `load.stderr.log`.

The recorder exits when the control process exits or its report indicates completion. `--demo` does not start load recording. For a run without a recorder, use `clevo-fan-cli load --out DIR --interval-secs 5`; only one load recorder is allowed per directory. The tuning algorithm currently uses temperature feedback; load data supports later comparisons at similar loads. Per-process load is not collected.

`stableResponse` is optional and defaults to disabled for old configurations. The GUI worker also supports it, taking precedence over the original Smooth Control toggle. There is currently no dedicated UI toggle. To apply a candidate, first exit the program and back up `ClevoFanControl.x64.json` beside the executable, then copy `candidate.json` there under that filename. Restart, enable Software Takeover, and use Save and Apply to save the configuration for everyday takeover. Auto-start is configured separately.

Sampling failure, sampling interruption (including a long gap after resume), repeated readback mismatches, or tachometer anomalies end tuning and attempt automatic restoration. Ctrl+C, stop requests, and timed exit also perform restoration. Forced termination, power loss, or a permanently hung DLL/driver still cannot guarantee restoration. The program does not change system sleep settings.

See the [local validation record](docs/NH5XAX.md#four-hour-ordinary-workload-cli-session) for one four-hour NH5xAx ordinary-workload session.

## References and Acknowledgments

Thanks to the technologies and maintainers used by the current implementation:

- [Rust / Cargo](https://www.rust-lang.org/)
- [FLTK](https://www.fltk.org/) and [fltk-rs](https://github.com/fltk-rs/fltk-rs)
- [Serde](https://serde.rs/) and [serde_json](https://github.com/serde-rs/json)
- [windows-rs / windows-sys](https://github.com/microsoft/windows-rs)
- [libloading](https://github.com/nagisa/rust_libloading)
- [RustCrypto / sha2](https://github.com/RustCrypto/hashes)
- [winresource](https://github.com/BenjaminRi/winresource)

References and acknowledgments retained from the original project:

1. [zuyan9 / RLECViewer](https://github.com/zuyan9/RLECViewer/tree/master)
2. [wangsihan158 / myfancontrol](https://github.com/wangsihan158/myfancontrol)
3. [xl-Synapse / MyFanControl](https://github.com/xl-Synapse/MyFanControl)
4. [Original Baidu Tieba thread](https://tieba.baidu.com/p/5971634018)

Inherited icons are in `assets/`. Legacy MFC source and binaries have been removed from the current source tree and remain in Git branch 'codex/backup-before-rewrite'. See [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md) and [licenses/](licenses/) for third-party licensing and vendor component distribution boundaries.
