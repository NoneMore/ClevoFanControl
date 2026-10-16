# Tasks

## 1. Preference model and compatibility

- [x] 1.1 Add the optional-on-read `historyRecordingEnabled` boolean with a false default in `src/config.rs`; verify omitted fields, true/false round trips, invalid types, duplicate/unknown fields and unchanged schema-version validation in configuration tests.
- [x] 1.2 Keep standalone legacy imports recording-disabled and original files untouched; verify legacy import and failed save/load tests compare original bytes and cover the new preference.
- [x] 1.3 Explicitly disable GUI recording in CLI candidate configurations without changing the source configuration or CLI recording behavior; verify argument/candidate tests and simulated monitor/tune runs with both preference values create no GUI history files.
- [x] 1.4 Document the preference default and older-binary downgrade procedure in README; verify names match serialized JSON and the documented backup/removal procedure preserves history files.

## 2. Asynchronous recorder and file format

- [x] 2.1 Add the recorder module and bounded status/producer interfaces, with no writer, queue, directory scan or file creation on the disabled path; verify disabled startup and enable/disable unit tests with injectable storage.
- [x] 2.2 Implement the 256-batch nonblocking queue, drop-newest counting, independent stop signaling and single-writer lifecycle; verify a deliberately stalled sink cannot block producers, saturates at the bound, reports tail losses, and prevents spawning replacement writers while one remains stalled.
- [x] 2.3 Implement versioned UTF-8/LF CSV and JSONL records, acquisition timestamps, monotonic elapsed times, sequences, control context and explicit missing values; verify parsing of 1-3 channels, commas/quotes/newlines, measured zero, failed writes, wall-clock changes and simulated session markers.
- [x] 2.4 Add unique session/segment creation and one-second scheduled flushing, including continuous-input operation and finalization; verify collisions never overwrite existing files and injected open/write/flush failures become bounded status errors.
- [x] 2.5 Document the schema, units, sample/request timing, loss accounting, interrupted-file interpretation and privacy in README; update the recorder's responsibility in `docs/ARCHITECTURE.md` and verify documentation links and schema examples.

## 3. Rotation and retention

- [x] 3.1 Implement dedicated executable-relative real/demo directories and paired segment rotation at UTC boundaries or before 8 MiB; verify clock-boundary, restart, repeated enable and segment-size cases using temporary directories and an injectable clock.
- [x] 3.2 Implement seven-date/128-MiB retention with pre-write accounting and oldest closed-pair eviction; verify active files stay protected, backward clock jumps cannot extend the date-bucket bound, large batches cannot exceed the budget and cleanup failure stops growth.
- [x] 3.3 Restrict cleanup to regular files matching the complete managed filename grammar in a verified directory, reject reparse points, and revalidate absolute deletion targets; verify unrelated files, directories, configurations, CLI sessions and link targets survive cleanup attempts, recording any environment limitation on link tests.
- [x] 3.4 Add repository-local history ignores and document paths, fixed retention, deletion scope, error behavior and copying evidence before expiry; verify `.gitignore` matches only intended roots and documented defaults match recorder constants.

## 4. Worker observations and safety lifecycle

- [x] 4.1 Add a dedicated recording command/sink to the worker without routing through full control settings; verify toggling recording preserves the active curves, takeover state, sampling schedule and existing draft-independent command semantics.
- [x] 4.2 Publish telemetry before subsequent hardware calls and correlate later control start/result events; preserve verification-triggering readings and retries. Verify blocked writes cannot suppress the acquired sample, context checkpoints survive rotation/drop recovery, no rows arise from snapshots, and all channels/missing readings remain explicit.
- [x] 4.3 Emit active preset/configuration, control mode, protection, fault, suspend/resume and actual restoration-result events; verify structured transition deduplication, no synthetic resume samples, and each recorded restoration outcome matches the test hardware's result.
- [x] 4.4 Integrate normal exit so existing restoration precedes recorder draining and finalization waits at most two seconds; verify normal, failed restoration, full queue and blocked-writer cases with deterministic worker/recorder test doubles and no unconditional shutdown join.
- [x] 4.5 Compare hardware operation counts and ordering with recording off, on and stalled; verify recording introduces no extra hardware samples, writes or restoration attempts and existing worker takeover/fault/recovery tests still pass. Document observation and shutdown boundaries in `docs/ARCHITECTURE.md`.

## 5. GUI preference transactions and status

- [x] 5.1 Introduce the latest-committed settings snapshot and serialize recording-only and full-save transactions; verify interleaved save requests preserve newer settings while recording-only saves cannot submit drafts, register auto-start, reset worker control or clear unrelated dirty flags.
- [x] 5.2 Add the recording switch with pending-save feedback and startup/reload synchronization; verify default-off, enable after successful save, immediate stop even on save failure with a next-launch warning, restart persistence and protection of invalid originals.
- [x] 5.3 Preserve the recording preference through legacy import and keep imported curves as drafts; verify pending curve/appearance/preset/takeover edits remain unchanged by recording-only toggles and full Save and Apply retains the latest saved recording preference.
- [x] 5.4 Show desired preference independently of actual off/starting/recording/degraded/stopping/error status, output directory, dropped count and last error; verify startup/write/cleanup errors, stalled finalization and explicit off/on retries never claim successful recording or stop control.
- [x] 5.5 Keep recording active while hidden and separate simulated files; document switch persistence, failure/retry behavior, tray recording and simulated output in README, and verify the GUI labels and documented behavior agree.

## 6. Integrated acceptance and evidence

- [x] 6.1 Run `pwsh -NoProfile -File ./scripts/check.ps1 -Release` on Windows x64 after implementation; verify formatting, Clippy, unit/integration checks and Release builds pass, and resolve regressions rather than treating demo results as hardware evidence.
- [x] 6.2 In a desktop session with a separate build/configuration and other controllers exited normally, exercise demo default-off/on, tray hide/reopen, restart persistence, import/reload, full-save ordering, storage errors and exit; record actual results and any unavailable cases without modifying the user's ordinary runtime configuration.
- [x] 6.3 Measure matched Release demo cases with recording off/on, visible/hidden windows and 2-second/200-ms intervals for at least 60 seconds per case; record CPU time, working-set trend, file growth, sampling intervals, drops and shutdown wait in a separate session directory. Verify bounded pending data, no storage backpressure and the design's hidden/default-interval CPU target, or document and investigate a failure before claiming acceptance.
- [x] 6.4 Record implementation acceptance conclusions in a linked document under `docs/`, including raw evidence paths, environment/build, tested defaults and outstanding real sleep/resume, long GUI soak and hardware scenarios. Verify historical NH5xAx evidence retains its date and baseline; perform no real fan writes without separate machine/scenario authorization and explicit restoration steps.
- [x] 6.5 Validate the finished change with `openspec validate add-gui-history-recording --strict` and check documentation links plus `git diff --check`; verify all required artifacts and implementation tasks are consistent with the delivered behavior and clearly identify any unverified acceptance items.
