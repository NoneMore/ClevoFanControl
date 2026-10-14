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

## Still to validate

The core read/write/restore protocol is validated, but the following should still be tested before treating the machine as fully production-proven:

- long-running GUI curve control
- sleep/resume
- shutdown/logoff behavior
- login auto-start
- recovery after forced process termination
- Windows 11 after upgrade

For first GUI testing, leave software takeover disabled until telemetry looks correct, then use conservative curves and verify both RPM channels after enabling takeover.
