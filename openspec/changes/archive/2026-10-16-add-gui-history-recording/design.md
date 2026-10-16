# Design

## Context

See [proposal.md](proposal.md) for motivation and [the capability delta](specs/gui-history-recording/spec.md) for the behavior contract.

- `src/ui/history.rs` holds a bounded `VecDeque<Option<Sample>>`; `Event::Sample` advances it, while frequent `Snapshot` messages update display state. Neither path currently persists GUI history.
- `src/worker.rs` owns hardware and control timing. Its sample event is emitted before subsequent actuation, so pairing that event with the UI's latest snapshot would mix different moments. The worker can also change requests between hardware samples.
- `src/ui.rs` separates a curve editor draft from mutable GUI settings and active worker configuration. Its asynchronous Save and Apply operation submits full settings after persistence. That completion handler cannot be reused unchanged for a recording-only toggle.
- `Settings` denies unknown JSON fields; current saves use atomic replacement after `sync_all`. The CLI loads the same type but runs a separate session loop and its own logs.

This design is warranted by cross-module lifecycle work, preference persistence, and performance/fault isolation. It changes no hardware protocol or control policy.

## Goals / Non-Goals

**Goals:** Maintain coherent observations without extra hardware calls; bound pending data and history growth; preserve control and draft boundaries during recording changes and failures.

**Non-Goals:** See the proposal. The recorder is diagnostic history, not a crash-safe journal: buffered data and a final partial line can be lost on interruption. No recorder operation guarantees restoration after forced termination or a hung driver.

## Decisions

### 1. Persist one independent preference

Add `historyRecordingEnabled: bool` to `Settings`, using `#[serde(default)]` and a false default without changing `schemaVersion`. Wrong JSON types remain errors. Standalone legacy import produces false; the GUI's legacy-import action carries forward its current recording preference just as it preserves other independent runtime settings.

Maintain a last-successfully-saved settings snapshot separately from the mutable GUI settings and editor draft. Initialize it only after a valid load or successful explicit full save. Serialize full saves and recording-only saves through one coordinator; allow one in-flight transaction and defer conflicting actions. A recording transaction changes only the boolean on the latest saved snapshot. On completion it updates that boolean in GUI settings and the saved snapshot, without clearing unrelated dirty flags, registering auto-start, sending `Command::Settings`, or replacing the editor draft. Full-save candidates are constructed after prior transactions complete so they preserve the new preference.

Enabling shows a pending state during persistence and starts only after a successful save, without Save and Apply. Disabling immediately stops accepting observations, then attempts preference persistence. A failed enable leaves recording off; a failed disable leaves this session off and explicitly warns that the saved preference can enable it next launch. If startup used fallback defaults because the original file was invalid, recording preference persistence is unavailable until explicit configuration repair. Reload resynchronizes the preference after a valid load. Legacy import preserves it. External edits require explicit reload.

Alternative: submit the whole settings object on every toggle. Rejected because the current save completion submits control configuration and clears dirty state. Alternative: require Save and Apply for recording. Rejected because the user requested direct activation.

### 2. Give the worker a nonblocking recording sink

Add `src/recording.rs` and expose it through `src/lib.rs`. A GUI-owned recorder service provides a lightweight producer handle to the worker; the writer owns all directory scans, file handles, formatting, buffering, rotation and deletion. The GUI polls a bounded status snapshot rather than receiving an unbounded stream of status messages. Recorder preference commands are separate from control settings commands.

When disabled, do not start a writer thread, allocate a sample queue, scan history, or create output files. When enabled, use a queue of 256 observation batches and `try_send`; an observation includes its channel rows and associated events as one unit. A full queue drops the newest batch, increments a shared atomic observation-loss counter, and returns immediately. This counter remains readable even if no later batch is accepted. The writer emits cumulative loss diagnostics after recovery and at normal finalization; the GUI reports loss independently of writer progress. A disconnected writer reports an error through the same bounded status state.

Stop and finalization signals must not compete for space in a full data queue: use an independent stop flag/control path. Stop detaches the producer first, then drains already accepted data. Only one writer may exist per GUI process. A writer stuck during I/O remains in stopping/error state; a new enable request cannot create another writer until the previous writer has completed, and the UI explains that condition.

Alternatives: synchronous CSV writes in the worker can stall cooling control; UI-driven recording can miss control lifecycle transitions during dialogs; reuse of the CLI recorder adds incompatible session ownership and workload queries. None is used.

### 3. Capture observations at their actual control boundary

Give every physical sampling attempt an increasing observation sequence. Publish telemetry immediately after acquisition, before any subsequent hardware call, with wall time and monotonic elapsed time. Correlate later decisions and operation start/result events by observation sequence; an operation without a result remains unknown. Never borrow a later UI snapshot. Preserve a jump-triggering reading marked as requiring verification and associate the existing retry, without adding hardware calls. A failed attempt emits an event without fabricated values.

Telemetry fields include `format_version`, `session_id`, `sequence`, `sample_unix_ms`, `elapsed_ms`, `channel`, `temperature_c`, `rpm`, `actual_duty_percent`, `curve_duty_percent`, `requested_duty_percent`, `reading_status`, `mode`, `active_preset`, and `simulated`. CSV requests are the last successfully issued software request at acquisition; later start/result events carry the new request and correlate by observation. They are not measured readback. Empty requests denote unavailable decisions or firmware control. All 1-3 observed channels are supported.

State changes between sampling attempts generate events with their own event sequence, the most recent observation reference when available, wall time and monotonic elapsed time. Capture recording start/end, active preset and effective mode changes, protection entry/recovery, control requests when changed between samples, hardware errors, suspend/resume, and each attempted restoration result. State-transition deduplication compares relevant structured values rather than localized display strings or snapshot timestamps. Recording does not defer an actuator operation while waiting for the writer. Stage diagnostics around existing operations, and preserve command-draining, stale-sample checks and restoration ordering.

Start a recording session with an active-state event and wait for the next real sample; do not backfill the live chart or relabel a cached sample as new. Stop collecting before finalization. A successful restoration event is generated only after the hardware API actually succeeds; a failed or inapplicable attempt remains distinct.

Alternative: dump every `Snapshot`. Rejected because snapshots arrive up to ten times per second and can repeat a sample or pair it with a different request.

### 4. Use portable, versioned text files

Write UTF-8 with LF, properly quoted fixed-header CSV, and JSONL using the existing dependency. Missing values remain empty/null, never zero. Each record identifies version/session/sequence. Every segment begins with a complete effective-state checkpoint, including application version, simulated flag, hardware description and active control settings. Every accepted batch carries its effective state so the next written record can restore context after dropped transitions. Never include unsubmitted drafts or entire personal configuration files. Retention must leave each surviving segment interpretable.

Use a session identifier derived from process start time, PID and a collision counter, and `create_new` for all segments. Name paired files `cfc-history-v1-<UTC-date>-<session>-<segment>.csv` and `.events.jsonl`. Store real files under `<executable-directory>/history/` and simulated files under `history-demo/`. The GUI displays the absolute output directory. No separate path picker or retention setting is added in this version.

CSV and JSONL are independently stream-readable. Pairing by sequence is logical, not a multi-file atomic transaction; parsers tolerate an incomplete last line or a missing counterpart after interruption. Do not attempt repair by overwriting earlier sessions. Include format, unit, request/readback timing, interruption and privacy notes in README. Local history contains preset names, control settings and a hardware description; do not include process lists or machine-wide workload data.

Alternatives: a database or a new serialization dependency add deployment/locking complexity without helping the requested offline analysis. CLI filenames and schema remain unchanged.

### 5. Buffer and rotate with fixed, conservative bounds

Planning defaults, selected without adding UI complexity:

| Item | Decision |
| --- | --- |
| Queue | 256 observation batches; discard newest on saturation |
| Flush | At least once per second when data is pending, and at rotation/finalization |
| Segment rotation | UTC date change or before the next batch would take a paired segment above 8 MiB |
| Retention | Current UTC date and preceding six UTC dates; at most 128 MiB total per real/demo directory |
| Shutdown wait | At most two seconds after the worker's restoration result |

There is no per-sample `sync_all` for history. The one-second interval is a flush scheduling target when storage responds, not a durability guarantee. Flush deadlines must still be checked when the queue stays busy.

Perform retention before opening/growing segments and at rotation. Account for both files and reserve space for the next batch before writing it; remove eligible closed segment pairs oldest first until both budgets are satisfied. Closed earlier segments of the current session are eligible, but its currently open pair is protected. Close/rotate that pair if necessary to meet the budget. Refuse an oversized batch or stop with a clear error if the budget cannot be met; never grow indefinitely. A backward wall-clock jump must not extend retention to more than seven distinct date buckets; retain the newest eligible buckets and treat the active bucket as protected until it closes.

Resolve the dedicated directory under the executable directory and reject reparse-point/symlink entries for managed storage or cleanup. Cleanup is nonrecursive, restricted to regular files matching the complete supported name grammar; do not delete configurations, CLI tuning sessions, directories, links or unknown files. Revalidate each deletion target's absolute path and metadata. An unsafe/failed required cleanup stops recording with an error rather than widening the deletion scope. No history directory is touched when recording is disabled. Add anchored ignores for repository-local `history/` and `history-demo/` during implementation.

Alternative: per-sample flushes provide little value for these diagnostic records and add writes; unbounded retention permits silent disk growth. The chosen constants are reviewable defaults rather than measured optima.

### 6. Separate desired state, runtime state and control safety

Expose desired preference plus `off`, `starting`, `recording`, `degraded`, `stopping` or `error` status, output directory, dropped count and last error. I/O errors close/detach the producer and stop that recording attempt without changing the saved preference or the fan fault latch. Recovery is an explicit off/on action; there is no tight automatic retry loop. An enabled preference after a prior failure is tried again at the next normal GUI startup.

Hiding the window changes neither the recorder nor the worker. Suspend records the actual worker restoration outcome and a pause; resume records a boundary and subsequent real samples, with no gap-filling rows. A recorder cannot make a hung hardware call return; missing events in that situation are a documented limitation.

On exit, send the existing worker exit command first. The worker attempts its existing restoration and publishes its result without waiting for the recorder. Then detach producers, append final outcome data if the writer remains healthy, and allow up to two seconds to drain/flush. Do not use an unconditional blocking join in shutdown or a destructor. If the deadline expires, report incomplete finalization and allow the process to exit; prior complete lines remain analyzable. Off/on retry also waits for actual completion of the old writer before replacement, preventing accumulated blocked threads.

### 7. Keep the CLI independent and verify with simulation first

The CLI accepts the new optional field but does not instantiate the GUI recorder. Candidates should explicitly set recording disabled, consistently with their existing disabled takeover/auto-start/matching switches, so copying a candidate does not unexpectedly enable logging. The source user configuration remains untouched.

Use an injectable clock/storage sink to deterministically exercise queue saturation, write/flush failures, rotation, blocked finalization and cleanup limits. Extend existing worker test doubles to compare hardware operation counts and ordering with recording off, enabled and stalled. Confirm there are no extra sample calls, writes or restoration operations attributable to recording. Extend configuration/import tests and GUI save-coordinator tests for draft isolation and write ordering. These tests provide simulated evidence only.

For desktop acceptance use a separate build directory and configuration, exit other controllers normally, then test demo recording, tray behavior, restart, import, malformed configuration, failure notices and clean exit. Measure Release demo operation with recording off/on at 2-second and 200-ms sampling for visible and hidden windows, for at least 60 seconds per case. Record process CPU time, working-set trend, managed file sizes, sample intervals, drops and finalization time. Compare matched cases; do not invent CPU or power guarantees from earlier six-second GUI checks or GPU-query duration. Aim for at most 0.5 percentage points additional total-machine CPU in the hidden default-interval case; investigate a larger difference rather than claiming acceptance. Saturation testing must demonstrate bounded queue growth and no control backpressure regardless of that performance target.

The 2026-10-15 hardware record demonstrates one CLI session on the documented NH5xAx baseline, not this GUI recorder. Real sleep/resume, long-duration GUI operation, filesystem behavior under real failures, and hardware timing remain separate acceptance items. Hardware writes require authorization for the machine/scenario and explicit restoration steps; they are not part of routine planning or simulated checks.

## Risks / Trade-offs

- [Buffered tail loss or split CSV/event writes] -> Sequence identifiers, tolerant parsing, explicit loss counters and finalization status; no crash-durability claim.
- [Filesystem I/O cannot always be cancelled] -> Nonblocking producers, one writer per process, finite exit wait, and visible stopping/error state.
- [Preference persistence adds a second save intent] -> One serialized coordinator and a committed settings snapshot; test interactions with full saves, imports and errors.
- [Retention removes diagnostic history] -> Fixed documented budgets and conservative recognized-file cleanup; users copy needed evidence elsewhere before expiry.
- [Shared settings deny unknown fields in older binaries] -> Document the new field and downgrade procedure; retain configuration backups and never migrate files on read.
- [Instrumentation changes control timing] -> Small typed batches, disabled fast path, no writer I/O in control, and operation-order plus matched performance checks.

## Migration Plan

1. Add optional preference parsing and tests before UI integration. Existing valid configurations load with recording disabled; loading alone rewrites nothing.
2. Add recorder and worker hooks, then the independent preference save path and status UI. Keep normal Save and Apply, takeover, import and CLI behavior covered by regressions.
3. Document storage, budgets, privacy, data-loss/error semantics and demo separation in README, and record module/state boundaries in `docs/ARCHITECTURE.md`.
4. Run standard checks and separate demo/Release performance acceptance. Record unverified real-hardware scenarios explicitly.
5. For downgrade, exit normally and restore a backup configuration or remove `historyRecordingEnabled` before using a binary that rejects the new field. Do not delete history as part of rollback; existing text files remain offline evidence until separately managed.
