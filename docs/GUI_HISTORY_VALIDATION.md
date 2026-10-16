# GUI History Recording Validation

## Scope and environment

Validation date: 2026-10-16. This record covers the optional GUI history implementation on Windows x64 using simulated hardware, plus a read-only review of a user-run real session documented below. It is not a new hardware compatibility or restoration claim.

- Windows 10, build 19045; 16 available logical processors.
- Rust `1.97.1 (8bab26f4f 2026-07-14)`.
- Independent build directory: `target/dev-check/`.
- GUI acceptance uses the production UI included in `tests/gui_history.rs`, test-only control references and an injected legacy file selection. The worker uses `Demo`; no vendor DLL, EC reads or real fan writes occur.
- Release measurements use the optimized GUI test executable, including the same UI/worker/recorder code. They do not measure real vendor driver overhead or GPU power effects.

## Automated and functional checks

`pwsh -NoProfile -File ./scripts/check.ps1 -Release` passed with `CARGO_TARGET_DIR=target/dev-check`: formatting, Clippy with warnings denied, 61 passing test executions, one explicitly ignored desktop workflow, and Windows x64 Release builds. The total includes three UI checks reused by the GUI test target. The desktop workflow was run separately. Finalization also has a regression test that admits a submission concurrently with stopping and verifies it is written before the session-end summary.

Simulated automated coverage includes configuration omission/type/duplicate validation, preservation of originals, committed-setting isolation, queue saturation/tail losses, continuing flush deadlines, output and cleanup failures, rotation and retention budgets, safe filename scope, interrupted/orphan files, missing RPM versus measured zero, verification-read association, and a writer that cannot finish within the shutdown budget. Worker tests verify that a blocked control write does not suppress the preceding acquired sample and that recorder instrumentation adds no hardware calls. Protection, fault, suspension/resumption and successful/failed restoration events are covered with test doubles.

CLI demo monitor/tune sessions passed with the preference both disabled and enabled. Candidates disabled GUI history; source configuration hashes did not change; the GUI history directory was not created.

The explicit GUI workflow passed preference persistence, queued full saves, preservation of unrelated pending settings, tray-style hiding, reload, legacy import, discarded imported drafts, normal exit, failure to save a disable preference, invalid-original protection, and output-path failure feedback. Restart from a saved enabled preference produced a new session. A captured settings view showed the independent switch, recording status and output directory without clipping in the tested default-size layout.

## Local evidence

These directories are local ignored artifacts, not repository-stored evidence:

- `target/verification/gui-history-cli-20261016-150411/`: four CLI demo sessions, source configurations and candidates.
- `target/verification/gui-functional-20261016-151306/`: successful GUI on/off, draft-independent preference saves and background operation; CSV/JSONL sessions and `settings.ppm` / losslessly converted `settings.png`.
- Later isolated directories prefixed `gui-import-`, `gui-queued-save-`, `gui-save-failure-`, `gui-invalid-`, `gui-open-failure-` and `gui-visible-preflight-` contain the respective reruns, configurations and history.
- `target/verification/gui-history-final-check-tail.txt`: captured tail of the final shared check command; earlier command output is in the implementation tool transcript.
- `target/verification/gui-history-performance-20261016-154301/`: final Release measurement transcript, per-case standard output/error, configuration, native visibility assertions, history and shutdown timing. Earlier partial matrices are excluded from the final comparison.

An initial native cross-process screenshot/input script selected hidden or inaccessible windows. Those screenshots are not acceptance evidence. The subsequent FLTK-driven tests and internal capture replaced that method. Test-created native demo instances were cleaned up; no existing real controller was force-terminated.

Post-archive cleanup removed six early failed or superseded experiment directories, obsolete native desktop automation helpers and a duplicate preliminary soak report (38 files, 11,691,843 bytes). The accepted evidence directories listed above, the frozen real-session snapshot and the performance/review scripts remain available. A local cleanup manifest is stored at `target/verification/gui-history-cleanup-20261016.json`. Both ordinary and delivered runtime configurations and the delivered executable were hash-checked before and after cleanup and remained unchanged.

## Release performance

The matrix compares recording off/on at 2-second and 200-ms intervals, with the main window shown/hidden. Each case runs for 65 seconds; CPU time and working set are sampled for at least 60 seconds after a two-second startup warmup. Both FLTK and native Windows visibility are asserted by the test. CPU percentages divide process CPU seconds by elapsed seconds and 16 logical processors.

The eight-case matrix completed. Each measured window was 60.71-60.81 seconds. All cases used simulated firmware control with automatic preset matching disabled. The following values are percentages of total machine CPU; the difference column is percentage points.

| Nominal interval | Main window | Recording off | Recording on | Difference |
| --- | --- | ---: | ---: | ---: |
| 2 s | Shown | 0.05943% | 0.06105% | +0.00162 |
| 2 s | Hidden | 0.00000% | 0.00161% | +0.00161 |
| 200 ms | Shown | 1.16846% | 1.16538% | -0.00308 |
| 200 ms | Hidden | 0.00000% | 0.00642% | +0.00642 |

Zero and very small differences are near the observed CPU-time granularity; they do not mean no instructions execute. The negative difference is measurement noise, not evidence that recording saves CPU. The high-frequency shown-window cost is already present with recording disabled; these measurements do not profile individual UI functions. The default hidden case meets the design's 0.5-percentage-point review target by a wide margin, on this simulated scenario only.

Enabled-case observations across the full approximately 65-second session:

| Nominal interval | Main window | Samples/channel | Mean / maximum sample period | Managed file bytes | Working set at measurement end |
| --- | --- | ---: | --- | ---: | ---: |
| 2 s | Shown | 32 | 2.0497 / 2.056 s | 15,550 | 21.04 MiB |
| 2 s | Hidden | 32 | 2.0468 / 2.068 s | 15,550 | 17.07 MiB |
| 200 ms | Shown | 302 | 0.21515 / 0.221 s | 120,458 | 19.50 MiB |
| 200 ms | Hidden | 302 | 0.21520 / 0.219 s | 120,458 | 17.20 MiB |

All four enabled cases reported **zero dropped records** and a session-end summary. All eight cases returned from the actual GUI after requesting normal exit in **107-136 ms**. Disabled cases created no history directory. Enabled-case working-set change during the measured window ranged from -0.055 to +0.102 MiB, with no sustained growth established in this short run.

Sample periods come from enabled-case history; there is no recorded off-case timing baseline, so this matrix does not establish an interval-by-interval timing delta. Nominal intervals remain subject to the existing worker polling and Windows scheduling. Synthetic operation-count and stalled-storage tests establish that recording adds no hardware calls or storage backpressure. The 200-ms file counts exceed the 60-point live chart limit, also confirming independent file retention.

The diagnostic queue remains bounded regardless of measured speed; a storage stall causes counted record loss instead of control backpressure. These are single short observations per case under ordinary concurrent machine activity, not guaranteed CPU/memory budgets or a statistical estimate of worst-case overhead. Capacity retention can shorten the history window before its seven-day age limit, particularly at faster sampling rates. Real takeover activity, changing presets and vendor-call overhead were not benchmarked here.

## User-run real-session review

On 2026-10-16, the user reported several hours of operation and requested an archive-readiness review. A read-only snapshot covered **08:01:25.970-12:28:16.981 UTC** (16:01-20:28 local time), or **4 h 26 min 51 s**. The application remained running; the reviewer did not issue hardware commands, stop the controller or change its configuration.

The records identify real rather than simulated operation on `NH5xAx`, using `InsydeDCHU.dll` with SHA256 `38aea56871649d1aec8d732efcfadb52142eae337f360e010d516b8457b763a6`. The recorded active curves match the original configuration copied into the independent Release directory after the delivery-path issue. BIOS/EC versions were not captured by this history session, so this observation does not extend the existing hardware compatibility baseline.

- 7,766 sample observations, each with both channels: 15,532 CSV rows. CSV and JSONL sample sequences match, all event sequences are contiguous, and all records parse successfully.
- Recorded drop counters remain zero. All readings are accepted; no sample-error, fault or protection events appear. All 29 control-write starts have successful result events. These are recorded outcomes rather than an independent measurement of EC behavior.
- Mean sample interval: 2.06169 s; median: 2.061 s; 99th percentile: 2.081 s; maximum: 2.097 s. No interval exceeds three seconds.
- CPU-channel temperatures span 51-72 degrees C and GPU-channel temperatures 40-50 degrees C. Combined files contain 3,340,678 bytes (3.19 MiB).
- The last persisted observation was approximately 1.6 seconds old when read. There is no session-end summary because the session is still active. Normal exit/restoration, sleep/resume and retention expiry were not exercised in this review. No matched CPU or memory measurement was collected during this user run.

Frozen CSV/JSONL copies and the parsed `review.json` are stored locally under `target/verification/gui-history-user-soak-20261016-122818/`. They are ignored local evidence, not repository-stored artifacts; this snapshot preserves the reviewed data independently of automatic history retention.

Archive-readiness conclusion: no blocking issue was found in the reviewed session. Together with the completed 28 implementation tasks and prior automated/demo acceptance, the evidence supports archiving this feature while retaining the boundaries below. The review itself does not archive the change or synchronize its delta specification.

Following user confirmation, the change was archived as [2026-10-16-add-gui-history-recording](../openspec/changes/archive/2026-10-16-add-gui-history-recording/proposal.md), with all 10 requirements synchronized into the [GUI history recording specification](../openspec/specs/gui-history-recording/spec.md). The independent build's configuration handoff is documented in the README configuration section.

## Outstanding acceptance boundaries

- Real sleep/resume, shutdown/logoff, vendor-call timing and real recovery scenarios.
- Multi-day GUI operation and long-term working-set behavior; the four-hour session supplies continuity evidence but no memory trend or proof of absence of leaks.
- Different models, BIOS/EC, vendor components, DLL hashes, storage devices, DPI/font settings and Windows versions.
- Filesystem or driver calls that never return: recorder producers remain nonblocking, but crash-safe final flushing and restoration are not guaranteed.

Historical hardware evidence remains separately scoped in [NH5xAx validation](NH5XAX.md) and [ownership research](../CONTROL_OWNERSHIP_RESEARCH.md); its original dates and baselines are unchanged.
