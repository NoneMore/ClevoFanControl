# Control Center Fan Ownership Research

Research date: 2026-09-21. Scope: the current machine and branch `codex/fltk-rs-rewrite`; code baseline `57a12f768954c9b24198f568bc4dc90e87b8e4d8`.

Historical references: the old MFC project and vendor binaries have been removed from the current source tree. Legacy paths and line numbers in this document refer to the baseline above and can be inspected with `git show 57a12f7:<path>`.

## Conclusions and Recommendations

The original application has no higher control priority than Control Center (CC), nor does it lock CC out of the controller. It calls vendor DLLs to write target duty. Some control paths write again when later sampling detects a discrepancy, but that does not prevent other programs from writing.

Switching to x64, running as administrator, and directly accessing the EC address architecture, access permissions, and communication paths respectively. None automatically provides exclusive ownership. Strictly preventing CC overwrites requires removing the writer, restricting its write permissions, or filtering its commands before they reach firmware.

Choose according to requirements:

1. **If replacing the CC background components is acceptable: retain the vendor ACPI driver, disable the CC components that actually control hardware, and make the Rust backend the sole routine controller.** This is the least complex product route. Identify and replace any hotkey, lighting, or performance profile functions the user still needs.
2. **If losing some CC hardware functionality is acceptable: first validate the device instance `Exclusive` property or device ACLs, with a Rust backend that holds a device handle persistently.** This may avoid a custom kernel driver, but isolates the entire Bridge rather than inherently isolating only fans.
3. **If CC must keep running with non-fan functions intact: consider a device-specific KMDF upper filter as the formal solution.** Parse and filter fan-related requests before `AcpiBridge.sys`; user-mode DLL interception can be an initial experiment. Cover bulk commands and bypasses actually used.
4. **If those approaches do not meet requirements: continue evaluating a replacement function driver, ACPI method rewriting, EC firmware changes, or independent hardware control.** These can change control boundaries, but cost substantially more than a routine software rewrite. Model adaptation and runtime validation were not completed in this study.

These conclusions come from source review, static binary analysis, and read-only system inspection. This study did not perform fan writes, stop services, change device properties, install filter drivers, flash firmware, or test contention with CC. Do not turn “Windows supports this mechanism” into “takeover was validated on this machine.”

## 1. How the Original Application Takes Over

Entry points are in `ClevoFanControl/Core.cpp`:

| Location | Actual behavior |
| --- | --- |
| `Init()`, line 174 | Dynamically loads `ClevoEcInfo.dll`, resolving `InitIo`, `SetFanDuty`, `SetFanDutyAuto`, and telemetry functions |
| `TimerCallback()`, line 441 | Triggers the work loop; sampling interval comes from configuration |
| `Work()`, line 685 | Runs `Update()` first; normal takeover calls `Control(config)`, while cancellation calls `ResetFan()` |
| `Update()`, line 761 | Reads temperature, current duty, and optional RPM |
| `Control(config)`, line 843 | Calculates targets from temperature, curves, and hysteresis; writes directly or delegates to the smoothing thread |
| `ResetFan()`, line 955 | Calls `SetFanDutyAuto(fanId)` for physical fans to restore firmware automatic mode |
| `SetFanDuty()`, line 984 | When current and target values differ, converts percent to 0–255 and calls the DLL; the third fan follows the GPU target |
| `SoftControlDuty()`, line 1024 | Gradually changes the software's smoothed duty; writes only when that internal value changes |

The claim that the original application continuously fights for control needs qualification:

- The ordinary, non-smoothed path attempts correction when later sampling detects a different duty, but CC writes can take effect in the meantime.
- After the smoothing thread reaches its target, `changed` is false; an external overwrite alone does not necessarily trigger another write.
- Forced cooling also conditions writes on internal target values and cannot be treated as an external overwrite detector.
- `m_bTakeOverStatus` is process-local state, `m_csEcApi` serializes only this program, and the single-instance Mutex only prevents duplicate instances of this application. None is an ownership protocol recognized by CC.
- `THREAD_PRIORITY_ABOVE_NORMAL` affects CPU scheduling only; it does not block CC device I/O.

Static disassembly of the original `ClevoEcInfo.dll` shows:

```text
Application SetFanDuty(fanId, duty)
  -> ClevoEcInfo.dll
  -> Port access provided by NTPort
  -> Wait for the input buffer at status port 0x66 to become free
  -> Send 0x99 to 0x66
  -> Send fanId, then duty, to 0x62

Automatic restoration: 0x99 -> 0xFF -> fanId
```

There is no visible owner identity, persistent lease, or logic to reject the next writer. Internal EC mode priorities still require the model's firmware or runtime experiments to establish.

## 2. Evidence from This Machine

### 2.1 Machine and Active Components

- Model/board: `Notebook NP5x_6x_7x_SNx`.
- BIOS: `1.07.27RTR6`.
- Running: `FnKey.exe` from `CLEVOCO.FnhotkeysandOSD_7.88.1.0_x64__6h6z29zh29qx0`.
- The process module list confirms it actually loads `InsydeDCHU.dll` from its own directory; this was not merely a same-named file found on disk.
- The package manifest declares FnKey as `Windows.FullTrustApplication` and requests `runFullTrust`. Its location under WindowsApps does not imply ordinary UWP sandbox restrictions.
- `CCDCHUService` is running, starts automatically, and uses LocalSystem. Its actual program is under the DriverStore directory `acpibridge1.inf_amd64_0a24f05be94f40ec`.
- `HKClipSvc` is also running; its name alone does not establish it as a fan writer.
- Actual driver: `C:\Windows\System32\drivers\AcpiBridge.sys`, version `1.0.0.9`, device `ACPI\CLV0001\1`, installed INF `oem77.inf`.
- `ACPI\CLV0002\1` also exists. DCHP in this machine's DSDT contains only identity and status methods, so it must not be called a confirmed second fan command bridge.

`CCDCHUService.OnStart()` launches FnKey and creates the `DCHUSERVICE` pipe. The service assembly also contains WMI/DCHU calls. It is not a confirmed, independently removable fan-only service. Stopping the service does not establish that an existing FnKey process also exits.

### 2.2 DLL and Driver in Use

The FnKey DLL is x64 and exports:

```text
GetDCHU_Data_Integer
GetDCHU_Data_Buffer
SetDCHU_Data
SetDCHU_DataEx
ReadAppSettings
WriteAppSettings
```

Observed device-opening code uses `GENERIC_READ | GENERIC_WRITE` and `FILE_SHARE_READ | FILE_SHARE_WRITE`. Ordinary commands use `DeviceIoControl(0x322400)`; AppSettings uses `0x32240C`.

`SetDCHU_DataEx` also uses `0x322400`, so hooking only `SetDCHU_Data` misses an entry point. Reads and writes share an IOCTL. Filtering must inspect commands and payloads rather than classifying only by IOCTL number, function name, or handle read/write access.

The inspected device creation path in the actual driver includes `WdfDeviceCreate` and interface registration, but no `WdfDeviceInitSetExclusive`. The installation INF does not declare `Exclusive`; device property queries returned no explicit exclusive value. These are static/configuration observations. Actual exclusivity has not been validated with a two-handle experiment.

The device interface GUID is `86994c74-ad43-4812-b7e7-0c420b5c5fd7`. The driver translates requests into ACPI method evaluations. It is a communication bridge, not a fan arbiter that automatically has the highest priority.

### 2.3 FnKey Contains Actual Fan Write Code

Static IL analysis of the installed FnKey can track obfuscated methods by metadata token:

- `06000806` writes the subcommand to the highest byte of a four-byte payload, then enters the generic call function.
- `06000819` dispatches to `SetDCHU_Data` or `SetDCHU_DataEx`.
- Methods including `0600000A`, `0600000D`, and `0600000E` contain fan mode writes using `121 / subcommand 1`.
- Additional paths use `121 / 14` for fan offset and `121 / 34` for restoring defaults.

This establishes real call paths, not a measured periodic write interval or execution of every branch on this machine.

## 3. Local ACPI Commands and Filtering Scope

The DSDT was exported read-only with Windows `GetSystemFirmwareTable`, then decompiled with ACPICA iASL `20260408`. These ACPI methods were not invoked.

In this machine's DSDT:

```text
InsydeDCHU.dll -> AcpiBridge.sys -> DCHU._DSM -> SCMD / CC30 / CPKG
WMI method entry              -> WMI.WMBB -> SCMD / CC20 / CPKG
                                                    |
                                          EC.ECMD or EC field writes
```

`DCHU._DSM` uses UUID `93f224e4-fbdc-4bbf-add6-db71bdc0afad`. WMI `_WDG` includes method GUID `abbc0f6d-8ea1-11d1-00a0-c90629100000`.

This study found no `CLEVO_GET` class in `root\WMI`. Therefore, **firmware has a WMI entry point** and **the class is callable in the current Windows environment** must be stated separately. Current CC use of a WMI bypass was not established.

| Command / path | Local static behavior | Isolation implications |
| --- | --- | --- |
| `0x68 / 104` | Sends four payload bytes sequentially to four fan channels through EC `0xC1` | Do not wrap this as a single-fan operation with zeroes for the others |
| `0x69 / 105` | Bits 0–3 select channels, sending EC `0xC1, 0xFF, channel` | Restores selected channels to automatic mode; not a proven exclusive-lock acquisition |
| `0x79 / 121, sub=1` | Sends EC `0xD7` subcommands according to the mode value | Include it in fan mode protection |
| `0x79, sub=0x0E` | Writes EC `GFOF` | Fan offset path |
| `0x79, sub=0x22` | Sends EC `0xD9` when parameter bit0 is 1 | Corresponds to the application's restore-defaults path |
| `0x0E / 14` | `PK0E` writes curve fields such as `F1T2/F1D2` and speed-related fields | Blocking duty writes alone misses curve rewriting |
| `0x02 / CPKG` | Bulk packages contain four-channel duty and automatic restoration branches | Parse package selection and write bits, or explicitly reject relevant mixed packages |
| `0x04 / PK04` | Includes branches assembling EC commands from input fields | Do not broadly treat it as a read package without side effects |
| `0x6B + 0x75` | Sets an index, then writes an EC-mapped region; writing command fields also submits a command | Constrain generic writes separately |
| `0x6A / 106` | Passes two parameter bytes to EC `0xBA`; the service calls it `SetECResumerTimer` | Internal timing, units, and failure behavior remain unclear; the name does not establish a lock or safety watchdog |
| Other `0x79` subcommands | Also affect performance, wireless, and other features | Blocking all of `121` harms non-fan functionality |

The `CPKG` duty branch checks bit `0x27` of `WQW2/WRW2` and takes the DWORD at package offset `0xB6`; automatic restoration checks bit `0x28` and takes `0xBA`. This only locates static branches. Package type and encoding must be checked before execution; this is not a validated transmission format.

What appears here is transaction serialization: `_DSM` and some methods are Serialized, and `ECMD` uses the `PATM` Mutex. **A transaction lock prevents concurrent execution; it does not prevent another program from changing state after the transaction completes.** Holding a Mutex indefinitely or faking a firmware-flash busy state interferes with other EC transactions and is unsuitable as a fan ownership mechanism.

This iASL run warned about external method resolution. These conclusions are limited to inspected local method bodies. EC internal firmware was not fully reverse engineered; firmware protection, timeout, or mode priorities cannot be ruled out.

## 4. Approach Matrix

Enforced prevention below refers to covered ordinary CC write paths. It does not cover an administrator actively changing system configuration or imply blocking firmware protection.

| Approach | Can prevent overwrites? | Other CC functions | Assessment |
| --- | --- | --- | --- |
| Vendor manual mode / fixed duty | No exclusivity guarantee found | Largely retained, with possible contention | Validate coexistence first; do not treat a mode as a lock |
| Disable/remove actual writers, retain driver | Yes, if writers really stop running | Mixed background functions may need replacement | Simplest default product route |
| Official CC setting disabling automatic fan management | Viable if it exists and covers all events | Best preservation | Static inspection did not confirm such a complete switch here |
| Block the fan module or selectively modify CC logic | Viable with complete call-site coverage | Potentially retained | Mixed FnKey responsibilities, updates, and recovery increase maintenance cost |
| Device instance Exclusive + persistent handle | Can block other opens if the stack enforces exclusivity correctly | All Bridge-related functions affected | Official Windows mechanism; worth a low-cost prototype |
| Device ACL + separate service identity | Can block unauthorized device opens | Also whole-device scope | Clearer than racing startup order; does not naturally distinguish EXE names |
| DLL proxy / Detours interception | Viable for covered calls in covered processes | Semantic preservation possible | Suitable for experiments; account for multiple DLLs, Ex exports, updates, loading, and package restrictions |
| AcpiBridge device upper filter | Can reject requests before they enter the original driver | Semantic preservation possible | Main formal candidate for strict coexistence; requires kernel driver development and signing |
| Additional filtering of actual WMI paths | Can cover entries outside the main Bridge | Depends on filtering granularity | Add only after proving use; do not break WMI globally |
| Replacement AcpiBridge function driver | Can integrate permissions and command arbitration | Must preserve events, caches, and protocol compatibility | More work than a device filter; fallback route |
| Have CC use configurations/curves generated by this application | Reduces policy conflicts, without enforced prevention | Good preservation | Possible requirements compromise; user mode changes in CC can still conflict |
| PawnIO / custom x64 port driver direct EC access | Not by itself | Still contends | Access-layer replacement, not arbitration |
| Disable CC's Bridge and use a separate EC channel | Cuts CC paths through the disabled device | Related vendor control functions fail | Can combine with PawnIO; check WMI/other-driver bypasses and do not still depend on the disabled Bridge |
| Run CC in a restricted environment with a custom hardware proxy | Viable if all its hardware access is covered | Proxy must preserve functions | Rebuilds an intermediary protocol; usually more complex than direct filtering |
| ACPI method rewriting / table override | Can block legacy writes at covered AML entry points | Preserve method by method | Development experiment; full local table recompilation and boot compatibility unvalidated |
| Modify EC firmware / adapt an open-source EC | Can define priorities inside the controller | All board responsibilities need adaptation | Changes final execution rules, but is not a generic DLL replacement |
| Hypervisor I/O/MMIO interception | Theoretically viable on covered paths | High complexity | Must handle access types, VBS/Hyper-V/SMM boundaries; unsuitable for the current product |
| Independent fan control hardware | Can bypass the original EC's fan-output authority | Hardware adaptation required | Ultimate alternative beyond a software-only rewrite |
| Frequent overwrites, higher process priority, or early startup | No guarantee; contention windows remain | Superficially retained | Does not meet overwrite-prevention acceptance |
| Suspend the entire CC process | That process does not execute while suspended | Its background functions pause | Diagnostic use; not maintainable selective isolation |
| Read-only registry or blocked configuration saves | Cannot stop commands already issued to hardware | May cause inconsistent configuration | Not an ownership boundary |

### 4.1 Exclusive Implementation Boundaries

An administrator installer can set Windows `DEVPKEY_Device_Exclusive / SPDRP_EXCLUSIVE`. This differs from a single `CreateFile` call's sharing flags. Actual exclusivity must be enforced at the device stack's named device object.

Candidate implementation: configure the device instance's exclusive property → rebuild the device stack to activate it → have the custom backend obtain and persistently hold the sole handle → submit all UI actions through that backend. Test whether a device/system restart is required, whether internal opens by the vendor driver are affected, and how CC behaves when opening fails.

Existing Insyde DLL calls manage their own opens and closes; do not assume they reuse the handle held by the backend. This route may require implementing the verified Bridge IOCTL wrapper directly in Rust rather than calling the DLL unchanged.

The actual FnKey DLL's `SetDCHU_Data` provides static evidence for reimplementing communication: request size `0x420`, output buffer `0x40C`, and command IOCTL `0x322400`. Request offsets are UUID at `+0x00`, revision at `+0x10`, `_DSM` at `+0x14`, and command number at `+0x18`. The package-length field at `+0x1C` is `0x104`, followed by argument encoding containing a 256-byte buffer. A later prototype should reproduce confirmed reads first, then validate writes and return-value semantics. These fields are not a stable public ABI shared by arbitrary DLL versions.

Opening first alone creates a startup-order race. Identifying the principal requires a separate service identity and suitable ACLs. If an ordinary user and CC share an account, user-based authorization cannot distinguish the applications; blanket LocalSystem access may also admit vendor services.

Even successful exclusive access to CLV0001 covers only that device entry. It does not automatically block WMI, other kernel drivers, or the EC's own behavior.

### 4.2 Minimal Design for Precise Filtering

```text
fltk-rs UI -> Rust control backend -> restricted control entry / authorized handle
                                            |
CC processes -> vendor DLL -> device filter -> vendor AcpiBridge -> ACPI -> EC
                                    |
                                    +-- Reject non-owner fan rewrite requests
                                    +-- Forward confirmed non-fan operations
```

- Begin in observation mode to establish actual commands, originating processes, trigger events, and bulk payloads. Static presence alone does not prove ongoing execution.
- Serialize owner writes and CC requests in one place. Handle in-flight requests before changing takeover state so old requests allowed earlier cannot arrive later and overwrite new targets.
- Apply policy by command semantics, subcommands, and package fields. Passing all writable packages that are not understood is incompatible with claiming strict fan overwrite prevention.
- Reject an entire bulk package, or remove fan-write portions only after fully understanding the protocol. The latter must preserve return semantics and remaining fields.
- Bind caller identity to the authorized service/file object, rather than comparing only process names or depending on the thread identity at a queued callback.
- Establish whether rejection causes CC to show failure, retry, or exit. Faking success creates disagreement between its UI and hardware state; do not adopt it without validation.
- Define release and automatic-restoration behavior for application exit, backend crash, handle cleanup, sleep/resume, and driver removal. Do not rely solely on restoration during normal UI exit.
- This is a device-specific filter. Do not hook the system kernel, modify `Acpi.sys`, or indiscriminately block the entire ACPI/WMI class for this task.

## 5. Additional Approaches Explored

### Preserve the Legacy Protocol with PawnIO

The public `LpcACPIEC.p` provides reads/writes to `0x62/0x66` and requires callers to coordinate with the `Access_EC` Mutex. In theory, it could support an x64 rewrite of the legacy DLL's port transmission logic, removing x86 `ClevoEcInfo.dll` and NTPort dependencies.

This is only a protocol migration candidate. Applicability of the legacy EC `0x99` command to this machine, coordination with ACPI port transactions, and module/driver deployment all require validation. The Mutex constrains participants only. There is no evidence that CC or the Windows ACPI executor uses it, so it cannot be treated as a system-wide lock.

### Share Policy with CC Instead of Competing

The vendor FanSpeedSetting `SetFanMode` writes firmware commands, saves AppSettings, and notifies other components. If this application writes only hardware without maintaining the state CC reads, later configuration restoration is more likely to create policy divergence.

If a limited set of firmware-supported curve points is acceptable, investigate sharing one configuration between CC and this application. It can reduce unexpected restoration, but cannot prevent a user from changing modes in CC, nor provide all current arbitrary-curve/process-preset functionality.

### ACPI and EC Firmware

ACPI table rewriting could make existing vendor entry points ignore protected fan writes while retaining a separate path for the custom backend. AML itself does not know the Windows caller's PID, so adding a condition to identify the EXE is insufficient. WMI, bulk packages, and generic write paths also need coverage.

Windows provides an official ACPI table-load mechanism for development, but it is not a stable deployment interface for ordinary applications. EC firmware changes go deeper. System76's public EC source can inform control architecture research, but this study did not establish direct support for this board. Do not flash it across models.

Independent hardware control also requires establishing PWM versus voltage control, power delivery, and tachometer feedback. Adding a PWM wire alone cannot be assumed to provide takeover.

## 6. Next Validation Sequence

This section is a future plan, not work performed during this study.

1. **Identify the overwrite source first.** Check targets, modes, duty, and RPM with CC writers stopped, running normally, switching modes, connecting/disconnecting power, and resuming from sleep. Distinguish CC commands, EC automatic restoration, and normal RPM variation; RPM changes alone do not prove register overwrites.
2. **Validate a single controller.** Retain the vendor driver and have the x64 backend read, set, and restore automatic control. Establish hardware behavior after backend crashes instead of treating the unknown `SetECResumerTimer` as validated recovery.
3. **Decide how much CC to retain.** If its backend is unnecessary, stop at replacement. If whole-Bridge isolation is acceptable, validate Exclusive/ACL. If other control functions must remain, proceed to command filtering.
4. **Build a list of commands actually used for filtering.** Alongside individual commands, establish actual CPKG, PK04, generic-write, and configuration-restoration use. Check whether currently unexposed WMI classes change after driver/CC updates.
5. **Validate contention and recovery.** Block CC fan-changing requests before hardware execution, retain non-fan functions, restore automatic control on release, and avoid incorrect residual control state after CC updates, service restarts, sleep, or backend exit.

Acceptance of strict overwrite prevention cannot rely only on final readings matching targets. At minimum, prove that blocked CC requests did not reach protected write paths and also check hardware state.

## 7. Evidence Locations and Public References

Local temporary research directory: `C:\Users\Xiao Qing\AppData\Local\Temp\clevo-x64-dependency-research`. It contains static IL, disassembly, and ACPI exports; these temporary materials were not introduced as project dependencies.

| File | Location |
| --- | --- |
| `acpi/DSDT.dsl` | `WMBB` 103700; `ECMD` 104585; `DCHU` 106106; `PK0E` 107290; `CPKG` 107483; manual/automatic branches 110633/110668; `0x79` 111066 |
| `FnKey-token.il.txt` | Search the metadata tokens above to distinguish obfuscated method names |
| `ActualDCHU-0.asm.txt` | Actual FnKey DLL; `CreateFileW` call VA `0x18000237E` |
| `AcpiBridge.sys.asm.txt` | Actual v1.0.0.9; device creation path starts at VA `0x140006DF8` |
| `FanSpeedSetting-all.il.txt` | Static analysis of the official package; `FAN.SetFanMode`, `FAN.SetFanInfo`, and others |

Key SHA-256 hashes:

```text
AcpiBridge.sys 1.0.0.9:
5c01679117aab5629526dab211eb0d57b1035631cd498710d2450290dcf80f35
Actual FnKey.exe:
d9968396dda8080d112cfe011b4a73f834e3302da18feb4558a747b0387103af
Actual FnKey/InsydeDCHU.dll:
75a47020d3a9d052e94dcb4e3ad61fb69f20177dd46e20c9a20883d92b83981b
DriverStore/InsydeDCHU.dll:
22fecadff27f4bf08cb4a17fe455ae490107b409e55827210f821947d65a9d47
Local DSDT.dat:
c7062c7817f081bbd38b51910cd5236f824c9810dec41801864a6b599c8f3b58
```

Public primary references:

- [Windows device exclusivity semantics](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/specifying-exclusive-access-to-device-objects)
- [Changing device properties after installation, including SPDRP_EXCLUSIVE](https://learn.microsoft.com/en-us/windows-hardware/drivers/kernel/setting-device-object-registry-properties-after-installation)
- [DEVPKEY_Device_Exclusive](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/devpkey-device-exclusive)
- [KMDF device access permissions](https://learn.microsoft.com/en-us/windows-hardware/drivers/wdf/controlling-device-access-in-kmdf-drivers)
- [Windows device filter driver installation](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/installing-a-filter-driver)
- [WDF DeviceIoControl request handling](https://learn.microsoft.com/en-us/windows-hardware/drivers/ddi/wdfio/nc-wdfio-evt_wdf_io_queue_io_device_control)
- [Windows kernel driver signing requirements](https://learn.microsoft.com/en-us/windows-hardware/drivers/install/kernel-mode-code-signing-requirements--windows-vista-and-later-)
- [Microsoft Detours](https://github.com/microsoft/Detours)
- [Microsoft ASL Compiler / ACPI table override](https://learn.microsoft.com/en-us/windows-hardware/drivers/bringup/microsoft-asl-compiler)
- [PawnIO LpcACPIEC source](https://github.com/namazso/PawnIO.Modules/blob/master/LpcACPIEC.p)
- [PawnIO module integration and licensing](https://github.com/namazso/PawnIO.Modules/wiki/Using-PawnIO-Modules)
- [System76 open-source EC](https://github.com/system76/ec)
- [TUXEDO Clevo fan control source](https://github.com/tuxedocomputers/tuxedo-drivers/blob/main/src/tuxedo_io/tuxedo_io.c)
- [clevo-thermald NH5xAx reverse-engineering notes](https://github.com/samoylenkodmitry/clevo-thermald/blob/main/docs/REVERSE-ENGINEERING.md): reference for another model, not a substitute for validation on this machine.
- [RLECViewer](https://github.com/zuyan9/RLECViewer): its README requires uninstalling Control Center first; it is not evidence that CC coexistence priority has been solved.

Some other online projects describe `SetFanAutoDuty(1)` as acquiring manual control. This conflicts with the per-channel automatic-restoration branch in this machine's DSDT. The rewrite should follow this machine's firmware and validation results rather than copy command descriptions across models.
