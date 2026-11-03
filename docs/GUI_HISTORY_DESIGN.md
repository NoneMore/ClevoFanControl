# GUI History Design Constraints

This document preserves the long-lived behavioral requirements for optional GUI history recording without requiring a specification-workflow tool. User-facing file formats, switches, and retention limits are described in the [README](../README.md#gui-history-recording); implementation boundaries are in the [architecture guide](ARCHITECTURE.md). The [validation record](GUI_HISTORY_VALIDATION.md) separates observed evidence from unverified hardware scenarios.

## Preference and configuration boundaries

- Recording is opt-in. If `historyRecordingEnabled` is absent, its default is `false`; non-boolean values are invalid.
- A recording toggle persists **only** its preference, based on the latest successfully committed configuration. It must not apply unsaved curve edits, alter active fan control or takeover, trigger auto-start registration, or overwrite newer successful saves.
- Enabling starts only after preference persistence succeeds. Disabling stops accepting observations even if saving the disabled preference fails; report the possibility that a restart may re-enable it. Never silently overwrite an invalid original configuration.
- GUI legacy import preserves the independent recording preference and the original legacy files. CLI candidate configurations disable GUI recording; the CLI does not launch the GUI recorder.

## Captured data and meaning

- Reuse each GUI worker sampling attempt exactly once, without extra hardware reads, workload queries, or dependence on the live chart's point limit. Continue recording while the window is hidden and while firmware controls the fans.
- Capture available per-channel temperature, RPM, actual duty, curve demand, last successfully requested software duty, effective mode/preset, and relevant hardware context. An unavailable measurement is **not** zero. Failed samples produce failure events, not invented successful readings.
- Use versioned UTF-8 CSV telemetry and JSONL control events with session identifiers, ordered sequences, wall-clock timestamps, and monotonic elapsed times. Mark simulated records explicitly.
- Sample values describe the readback **at acquisition**, not a subsequent write. Correlate later write-start and write-result events with the observation; missing results remain unknown. Preserve the relationship between a verification-triggering read and its existing retry without adding a read.
- Record meaningful control-state transitions, faults, restoration attempts and results, suspend/resume, and session lifecycle events. Avoid duplicate events from unchanged display refreshes. Include complete state checkpoints at segment starts and after lost context so retained segments are interpretable.

## Isolation, retention, and shutdown

- Recording uses a bounded, nonblocking queue. A slow or failing filesystem must not delay UI interaction, fan sampling, hardware writes, or restoration. Count dropped records rather than blocking producers.
- Display the saved preference separately from actual recording state, output path, lost-record count, and error. Writer or cleanup failure stops the recording attempt, not fan control. An explicit off/on toggle can retry after fixing the error.
- Write into a dedicated local history directory, with unique session/date segments and UTC-day/size retention as documented in the README. Cleanup must only touch recognized closed managed files, without following reparse points or deleting unrelated files.
- On normal exit, attempt fan restoration before waiting for recorder finalization; the wait is bounded. Buffered records or a final session-end event may be missing after interruption. Never treat a crash, hung driver, or unfinished write as evidence that restoration or durable flushing succeeded.

## Verification boundaries

- Primary implementation: `src/recording.rs`, `src/preferences.rs`, `src/worker.rs`, `src/ui.rs`; regression coverage: `tests/gui_history.rs` and worker/configuration tests.
- Routine checks and simulated GUI runs do not prove real hardware behavior. Evaluate actual recording, persistence failures, sleep/resume, recovery, and long-running resource use at the acceptance level relevant to the change; keep the original evidence and unverified scenarios scoped in the [validation record](GUI_HISTORY_VALIDATION.md).
