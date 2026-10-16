# Repository Agent Instructions

## Language

- Write all repository documentation, agent instructions, and OpenSpec planning artifacts in English. Use Chinese only for final user-facing deliverables and handoff summaries. Preserve code identifiers, commands, and required OpenSpec structure markers.

## Change Workflow

- Complete the relevant OpenSpec planning before adding or changing externally visible behavior, hardware protocols, configuration formats, or architecture. Continue an existing change when applicable. Small fixes that restore established behavior and documentation changes may proceed directly.
- Follow [openspec/config.yaml](openspec/config.yaml) when using OpenSpec.
- Follow [CONTRIBUTING.md](CONTRIBUTING.md) for development and checks; consult [README.md](README.md) for user behavior and [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for module responsibilities and state boundaries. Maintain details in those documents rather than duplicating toolchain versions, dependency lists, or directory inventories here.

## Boundaries to Preserve

- Assess both GUI and CLI when changing shared control algorithms or hardware interfaces. They organize control sessions separately, so review scheduling, fault recovery, exit, and sleep/resume paths in each entry point.
- Distinguish the editing draft, saved configuration, and worker's active configuration when changing GUI configuration flow. Takeover toggles and automatic preset matching must not implicitly submit the draft. Explain when each affected action submits, saves, and takes effect.
- Check legacy defaults, strict validation, migration, and preservation of original files on failure when changing configuration persistence or import. CLI candidate configurations do not directly overwrite user configurations.
- Check the return of control after faults and takeover cancellation, including the restoration result. Do not promise automatic restoration after a crash, forced termination, or a hung DLL/driver.

## Verification and Completion

- For Rust changes, run `pwsh -NoProfile -File ./scripts/check.ps1`; add `-Release` for build, resource, script, or release workflow changes. Maintain shared checks in that script; see the development guide for details. Documentation-only changes require content, link, and diff checks.
- Prefer simulated hardware for routine verification. GUI checks, real sampling, and real writes are separate acceptance levels. `--hardware-check` and real CLI `tune` write fan settings; run them only within the machine and operation scope already authorized by the user, with explicit restoration steps.
- The demo GUI can save runtime configuration and shares the single-instance lock with real control programs. Follow the development guide to use a separate build directory or back up configuration, and exit existing programs normally.
- Limit hardware conclusions to the validated model, BIOS/EC, vendor components, DLL hash, and scenario. Preserve dates, baselines, and outstanding validation items when consulting [CONTROL_OWNERSHIP_RESEARCH.md](CONTROL_OWNERSHIP_RESEARCH.md) and [docs/NH5XAX.md](docs/NH5XAX.md). Do not present simulated results as hardware evidence.
- Report the resulting behavior, checks actually performed and their results, and unverified scenarios. Link new documentation from an appropriate entry point. Use a separate session directory for raw tuning records; preserve conclusions in `docs/` with evidence locations. A local archive does not mean the evidence is stored in the repository.

## Distribution and File Conventions

- Do not commit vendor DLLs, drivers, or personal runtime configurations with source or release packages. When dependencies or the toolchain change, review the lockfile, README technology stack, [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md), and `licenses/`.
- Use UTF-8 and LF for new files and follow `.editorconfig`. Use `cargo fmt` for Rust formatting; avoid bulk rewrites of existing files solely for cleanup.
