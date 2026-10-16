# Spec Delta

## Purpose

Preserve GUI fan telemetry and active control context in optional local history files so users can investigate behavior after the live chart expires or the application exits, while keeping recording failures separate from fan control.

## ADDED Requirements

### Requirement: Recording is opt-in and remembered

The GUI SHALL default recording to disabled when the preference is absent. A successful manual toggle SHALL persist the preference and activate or deactivate recording without a separate Save and Apply action. Subsequent launches SHALL honor the saved preference.

#### Scenario: Existing configuration has no recording preference
- **WHEN** a valid older configuration without a recording preference is loaded
- **THEN** the GUI starts with recording disabled and creates no history files or recorder activity

#### Scenario: Enable and restart
- **WHEN** the user enables recording and the preference save succeeds
- **THEN** recording starts asynchronously without Save and Apply and a subsequent normal GUI launch starts a new recording session automatically

#### Scenario: Disable and restart
- **WHEN** the user disables recording and the preference save succeeds
- **THEN** new observations stop being accepted, accepted observations are finalized asynchronously, and the next GUI launch leaves recording disabled

### Requirement: Recording preferences preserve configuration boundaries

A recording toggle SHALL save only its preference against the latest successfully saved configuration. It SHALL NOT submit curve drafts, persist other pending edits, change the worker's active control settings, or trigger auto-start registration. Configuration writes SHALL preserve newer successful changes.

#### Scenario: Toggle with unrelated pending edits
- **WHEN** the user has unsaved curve, appearance, preset, or takeover changes and toggles recording
- **THEN** those edits retain their previous draft or runtime status and only the recording preference changes on disk

#### Scenario: Serialized saves
- **WHEN** a full settings save and recording toggle are requested close together
- **THEN** their successful results preserve both intended updates without stale overwrites or implicit draft submission

#### Scenario: Preference persistence fails
- **WHEN** saving the recording preference fails
- **THEN** the original configuration remains intact, enabling remains inactive, and disabling still stops this session while a visible warning explains that the saved preference can enable recording next launch

### Requirement: Existing telemetry is recorded without extra sampling

Enabled recording SHALL preserve each existing GUI sampling attempt once, independently of chart capacity. Successful readings SHALL include all detected channels, their temperature, RPM and actual duty, plus associated curve demand, worker request and active control context. Recording SHALL NOT add hardware reads or workload queries.

#### Scenario: Record a successful observation
- **WHEN** a scheduled sample succeeds while recording is active
- **THEN** each detected channel has one row containing the observed telemetry, applicable control values, preset and mode, with no duplicate rows from display refreshes

#### Scenario: A worker request differs from curve demand
- **WHEN** smoothing or protection produces a request different from curve demand
- **THEN** the file preserves both values separately and identifies the relevant control state

#### Scenario: Firmware controls the fans
- **WHEN** recording is active without software takeover
- **THEN** observed telemetry is recorded and software requests that are not applicable remain empty

#### Scenario: Missing data
- **WHEN** RPM is unavailable or a sampling attempt fails
- **THEN** unavailable values are not replaced with zero, and a failed attempt is represented by a timestamped failure event rather than a fabricated successful reading

#### Scenario: A temperature jump triggers the existing verification read
- **WHEN** a successful reading triggers the worker's existing retry
- **THEN** the first reading is preserved as requiring verification and linked to the retry result without increasing hardware call counts

### Requirement: Files have an interpretable and coherent format

History SHALL use versioned UTF-8 CSV telemetry and JSONL events with session identifiers, increasing observation sequences, wall-clock timestamps and monotonic elapsed times. Observations SHALL distinguish sampled readback from subsequent control requests and operation outcomes. Simulated data SHALL be explicitly identified.

#### Scenario: Associate a sample and control outcome
- **WHEN** a sampling observation is followed by a control write or restoration attempt
- **THEN** its telemetry is submitted before that hardware call and later start/result events identify the associated observation, with an absent result remaining unknown

#### Scenario: Rotation or dropped transitions remove earlier context
- **WHEN** earlier segments are deleted or state-change events are dropped
- **THEN** surviving segments begin with full effective-state checkpoints and subsequent accepted observations restore current state without depending on missing events

#### Scenario: Special characters and absent fields
- **WHEN** a preset or error contains commas, quotes or newlines, or a measurement is unavailable
- **THEN** standard CSV or JSON parsing recovers the original text and distinguishes absent measurements from measured zero

#### Scenario: Clock changes or demo operation
- **WHEN** the wall clock changes or the GUI uses simulated hardware
- **THEN** sequences and monotonic elapsed times preserve observation ordering, and simulated sessions are visibly marked in the files and stored separately from real sessions

### Requirement: Lifecycle and control transitions remain visible

Enabled recording SHALL capture session start and end, active preset and control-mode changes, protection transitions, sampling/control faults, suspension/resumption, and restoration outcomes. Repeated unchanged status refreshes SHALL NOT produce duplicate observations or events.

#### Scenario: Observe a fault and restoration
- **WHEN** a hardware fault causes the worker to attempt restoration
- **THEN** events distinguish the fault, restoration attempt and actual success or failure without changing the worker's existing recovery policy

#### Scenario: Suspend and resume
- **WHEN** the GUI receives suspend and resume notifications
- **THEN** recording represents the pause and observed restoration outcome, writes no synthetic samples for the gap, and continues with subsequent scheduled samples after resume

### Requirement: Background recording follows the control session

Recording SHALL continue while the main window is hidden in the tray and SHALL remain independent of the number of live chart points. Normal exit SHALL attempt fan restoration before waiting for recorder finalization, and recorder shutdown SHALL have a finite wait budget.

#### Scenario: Hidden window and chart truncation
- **WHEN** the window is hidden or old chart points are evicted
- **THEN** subsequent observations continue reaching the history files under the same recording preference

#### Scenario: Normal exit with a stalled writer
- **WHEN** the user exits while storage is stalled
- **THEN** the existing fan restoration path proceeds before waiting on storage, and the GUI stops waiting for recorder completion after at most two seconds

#### Scenario: Interrupted process
- **WHEN** the process is forcibly terminated or a driver or filesystem call hangs
- **THEN** previously completed records remain usable, an incomplete final record or missing session-end event is treated as interruption, and the application does not claim guaranteed final flushing or restoration

### Requirement: Recording cannot backpressure fan control

Recording SHALL use bounded pending storage and SHALL NOT make control sampling, hardware writes, restoration, or UI interaction wait for history file I/O. Saturation SHALL discard new recording observations while keeping control active, with loss counts visible in the GUI and recoverable files.

#### Scenario: Slow storage fills pending capacity
- **WHEN** the recording queue reaches capacity
- **THEN** producers return without waiting, new observations are counted as dropped, and the GUI shows degraded recording status

#### Scenario: Recording catches up or stops
- **WHEN** storage recovers or a session finishes normally after queue saturation
- **THEN** the next writable diagnostic or final summary reports the cumulative number of lost observations, including losses at the tail of the session

### Requirement: Recorder failures are isolated and actionable

The GUI SHALL show desired preference separately from actual recording status, output location, dropped-observation count and error state. Recorder I/O failure SHALL stop that recording attempt and leave fan control running. The saved preference SHALL remain unchanged until the user changes it.

#### Scenario: Output cannot be opened or written
- **WHEN** opening, writing, flushing, rotating or required retention cleanup fails
- **THEN** the GUI shows a recording error and location, stops claiming to record, and does not stop control or rewrite the saved preference

#### Scenario: Retry after an error
- **WHEN** the user switches recording off and on after addressing the error
- **THEN** the application attempts a new uniquely identified session without overwriting earlier files

### Requirement: History storage is local and bounded

History SHALL be stored under a dedicated directory beside the executable, with distinct files for sessions and UTC days. Automatic retention SHALL keep at most seven UTC date buckets and 128 MiB of managed history files after successful maintenance. Cleanup SHALL affect only recognized closed history files and SHALL NOT follow links or delete unrelated files.

#### Scenario: New session or day
- **WHEN** recording starts, a UTC date changes, or a segment reaches its size limit
- **THEN** new uniquely named segments are created and no existing history is overwritten

#### Scenario: Expired or oversized history
- **WHEN** managed history exceeds its age or size budget
- **THEN** oldest eligible closed segments are removed before accepting further file growth, while current open files and unrelated entries are preserved

#### Scenario: Cleanup cannot safely meet the budget
- **WHEN** eligible files cannot be removed safely or access is denied
- **THEN** recording reports a storage error and stops growing history rather than deleting outside its managed scope

### Requirement: Configuration compatibility remains strict

The new preference SHALL accept only a boolean and SHALL default to false for omitted fields and standalone legacy imports. Invalid configuration SHALL NOT be silently overwritten. GUI legacy import SHALL preserve the current independent recording preference. CLI sessions SHALL NOT start the GUI recorder.

#### Scenario: Invalid preference
- **WHEN** a configuration contains a non-boolean recording preference
- **THEN** loading or importing it reports validation failure and preserves the original file bytes

#### Scenario: Import legacy curves while recording
- **WHEN** legacy curves are imported into a running GUI with recording enabled
- **THEN** recording continues under its existing preference while imported curves remain drafts until the existing apply action

#### Scenario: CLI loads a recording-enabled configuration
- **WHEN** CLI monitor or tune loads a valid configuration with the preference enabled
- **THEN** it retains its existing session outputs and behavior without starting a GUI history recorder
