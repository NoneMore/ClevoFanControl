# Clevo NH5xAx validation

This branch adds hardware support for the Clevo `NH5xAx` barebone using the vendor `InsydeDCHU.dll` + `AcpiBridge.sys` path already used by the main program.

## Validated machine

- System product name: `NH5xAx`
- CPU: AMD Ryzen 7 3700X
- BIOS: Insyde `1.07.07`
- ACPI device: `ACPI\CLV0001`
- Acpi Bridge: Insyde `1.0.0.8`
- DLL path: `C:\Program Files (x86)\ControlCenter\DCHU\InsydeDCHU.dll`
- DLL version: `1.0.0.1`
- DLL SHA256: `38aea56871649d1aec8d732efcfadb52142eae337f360e010d516b8457b763a6`
- Physical fans: 2 (CPU + GPU)

The DLL above is Microsoft WHCP-signed on the tested machine.

## Hardware validation performed

Read-only package validation succeeded:

- package `0x0D` reports 2 fans
- package `0x0C` reports plausible CPU/GPU temperature, duty and tachometer values
- RPM conversion matches the existing fan utility closely

Per-channel write validation also succeeded using the same command sequence as the main hardware backend:

- CPU channel 1: fixed 70% duty was read back as 70%; GPU duty remained unchanged
- GPU channel 2: fixed 70% duty was read back as 70%; CPU duty remained unchanged
- after each test, `SetDCHU_Data(0x69, mask)` returned both channels to firmware automatic control

Observed CPU test:

```text
Before:  CPU 34% / 3134 RPM, GPU 48% / 2531 RPM
70%:     CPU 70% / 4339 RPM, GPU 48% / 2531 RPM
Auto:    CPU 31%, GPU 44%
```

Observed GPU test:

```text
Before:  CPU 34% / 3098 RPM, GPU 48% / 2531 RPM
70%:     CPU 34% / 3111 RPM, GPU 70% / 3297 RPM
Auto:    CPU 34%, GPU 41%
```

## Implementation notes

`NH5xAx` is added to the default model whitelist. The tested DLL SHA256 is added to the hardware allowlist.

Because the tested system has a staged FnKey AppX package rather than a package registered for the current user, automatic AppX discovery may return nothing. For `NH5xAx`, the hardware resolver therefore checks the validated legacy Control Center path first:

```text
C:\Program Files (x86)\ControlCenter\DCHU\InsydeDCHU.dll
```

If that file is absent, normal DLL discovery is used.

## Four-hour ordinary-workload CLI session

On 2026-10-15, the CLI completed a four-hour session on this machine:

- 10 minutes of firmware automatic baseline, followed by about 230 minutes of adaptive control
- 2-second fan sampling: 7,198 samples per channel, with a maximum interval of 2.014 seconds
- 13 curve adjustments; the final candidate remained in use for about 68 minutes
- normal timed exit, successful firmware automatic restoration, and no hardware errors reported
- CPU/GPU utilization and GPU power were recorded every 5 seconds after the first 172 seconds; no per-process attribution was collected

| Measurement | CPU baseline | CPU adaptive | GPU baseline | GPU adaptive |
| --- | ---: | ---: | ---: | ---: |
| Mean temperature, °C | 59.8 | 58.5 | 45.5 | 45.6 |
| 95th percentile temperature, °C | 66 | 65 | 47 | 52 |
| Maximum sampled temperature, °C | 77 | 72 | 47 | 53 |
| Mean duty, % | 33.0 | 35.2 | 44.0 | 44.0 |
| Mean RPM | 3,061 | 3,243 | 2,372 | 2,374 |
| Actual duty changes per hour | 888.0 | 4.4 | 24.0 | 3.7 |

During adaptive control, CPU temperature was at or below 65°C in 96.6% of samples, and GPU temperature was at or below 50°C in 77.7% of samples. Actual duty changed 17 times for CPU and 14 times for GPU; individual changes were at most two percentage points. The last 68 minutes held GPU duty at 42%.

The session used `stableResponse`: a 15-second median and 30-second EMA, a two-point duty deadband, a normal change interval of at least 30 seconds, and a 120-second wait before decreasing duty. Raw high temperatures bypass these delays. This response policy is also supported by the GUI; it must be preserved when applying the candidate configuration.

Baseline and adaptive workloads differed, and the baseline also included development activity. These measurements demonstrate stable control under the observed ordinary workload, not a controlled cooling/noise comparison. CPU mean RPM increased, acoustic noise was not measured, and the unvisited high-temperature curve region remains unvalidated. The four-hour run used the CLI, not a four-hour GUI soak test.

## GUI recovery and CPU usage follow-up

After applying the candidate, a GUI report showed CPU fan duty at 100% while the curve target was about 36%. Toggling software takeover restored normal control. There was no raw GUI temperature log to establish the original trigger. Code inspection found that a high-temperature safety boost could otherwise decrease by only one percentage point every 30 seconds after the normal decrease delay.

Safety boost recovery now requires 30 continuous seconds below the trigger minus 5°C, then restores the higher of pre-boost duty and current curve demand. Synthetic-temperature regression tests cover recovery, interrupted cooling and retained curve demand; this change was not tested by deliberately heating the machine. The GUI now displays both curve demand and requested duty, plus an explicit protection status.

Repeated snapshot rendering and tray updates were suppressed, and the GUI polling loop was bounded while hidden. Short on-machine checks measured about 3.0–3.4% total CPU usage before the change, 0.10% with the updated window shown and 0.00% rounded while hidden, over six-second measurement windows. Closing hid the application without terminating control, reopening succeeded, and the saved curve and tray settings were preserved. These short checks do not replace a long GUI soak test.

## Still to validate

The core read/write/restore protocol is validated, but the following should still be tested before treating the machine as fully production-proven:

- long-running GUI curve control
- sleep/resume
- shutdown/logoff behavior
- login auto-start
- recovery after forced process termination
- Windows 11 after upgrade

For first GUI testing, leave software takeover disabled until telemetry looks correct, then use conservative curves and verify both RPM channels after enabling takeover.
