# Proposal

## Why

The GUI retains only its most recent 10-300 samples in memory, so a temperature spike, unexpected duty request, or fault cannot be investigated after the history rolls over or the program exits. Users need optional, low-overhead background files containing observed telemetry and the active control context.

## What Changes

- Add a GUI recording switch that defaults to disabled for new and existing configurations. Toggling it saves the preference and applies it without requiring Save and Apply; subsequent GUI launches restore that preference.
- Record existing fan samples, curve demand, worker requests, active preset, control mode, and relevant lifecycle/fault events in versioned CSV and JSONL files. Missing readings and failed operations remain explicit.
- Continue recording while the GUI is hidden in the tray and while firmware controls the fans. Keep recording independent of the chart's in-memory point limit.
- Use a dedicated writer with a bounded, nonblocking input queue, buffered writes, observable data-loss counters, and bounded file retention. Recording failures must not interrupt control or restoration.
- Show actual recording status, output location, and persistence or I/O errors in the GUI.
- Add backward-compatible configuration defaults and preserve unrelated saved settings and unsaved editing drafts when the recording switch changes.

Non-goals: historical playback inside the GUI, additional hardware sampling, CPU/GPU utilization or power recording, launching `nvidia-smi`, changes to fan algorithms or hardware protocols, CLI session-format changes, cloud upload, and cross-platform support. This change targets the existing Windows x64 application.

## Capabilities

### New Capabilities

- `gui-history-recording`: Opt-in persistent GUI telemetry and control-context recording, including preference persistence, asynchronous storage, lifecycle handling, diagnostics, and retention.

### Modified Capabilities

None. The project currently has no durable capability specs.

## Impact

- `src/config.rs`: add an optional-on-read recording preference; keep strict validation and original-file protection. Older binaries may reject configurations saved with the new field because their readers deny unknown fields; document downgrade recovery using a backup or removal of that field.
- `src/worker.rs`: publish coherent recording observations and lifecycle events without additional hardware calls or blocking file operations; preserve fault, exit, and sleep/resume ordering.
- New recorder module exposed through `src/lib.rs`: file schema, bounded queue, writer lifecycle, rotation, retention, and error reporting.
- `src/ui.rs` and GUI startup: recording switch, serialized preference saves, runtime status, session startup, and shutdown coordination.
- Existing GUI chart behavior and CLI monitoring/tuning remain unchanged. Shared settings compatibility and CLI candidate output require regression checks.
- Update README and architecture documentation; add simulated failure, configuration, and lifecycle coverage. Run the standard Rust checks during implementation, and assess desktop behavior and performance separately. No new dependency or real hardware write is planned.
