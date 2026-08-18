# Maintainer — Product Requirements Document

Status: Active specification · Last reviewed: 2026-08-18

## Product summary

Maintainer is a modern, lightweight native desktop application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux computers. It combines common system-management workflows without becoming a fake optimizer, full administration suite, or cloud service.

Positioning: **Your Linux computer's personal maintenance center.**

## Problem

Linux system information and maintenance are fragmented across monitors, disk analyzers, package tools, service managers, logs, SMART utilities, file managers, and terminal commands. Less technical users struggle to decide what matters and what is safe; experienced users lose time moving between tools and assembling reports. Existing tools often expose raw data without connecting evidence to a clear, controlled action.

## Product principles

- **Lightweight:** Maintainer must not become the performance problem it measures.
- **Native:** GTK4 and Rust only; no browser runtime.
- **Local-first:** No account, telemetry, cloud backend, or system-data upload.
- **Safe:** Review, confirmation, least privilege, and useful operation history.
- **Transparent:** Explain evidence, limitations, requested privileges, and exact effects.
- **Evidence-based:** Separate detected facts, possible causes, and recommendations.
- **Polished:** Modern information hierarchy without costly visual effects.

## Goals

| Goal | How we know it worked |
|---|---|
| Give users one useful health overview | Users can identify resource pressure, low storage, updates, failed services, and storage warnings from Overview |
| Explain resource usage | CPU, memory, disk, network, temperature, and process data use correct Linux semantics and explicit unavailable states |
| Make storage understandable | Users can drill into usage and complete cancellable large-file and duplicate scans |
| Provide safe maintenance | Every destructive operation is previewed, confirmed, validated, and recorded with per-item results |
| Centralize updates | Supported APT updates can be reviewed, authorized through Polkit, executed, and reported without storing credentials |
| Improve troubleshooting | Failed services and repeated journal events link evidence to cautious explanations and recommendations |
| Produce useful reports | Users can copy or save a local redacted system snapshot |
| Stay lightweight | Meet the defined memory, CPU, startup, refresh, and responsiveness budgets |
| Support Ubuntu-based desktops | Operate capability-first on current Ubuntu-based distributions with Ubuntu 22.04 as the floor |

## Non-goals

- Antivirus, malware removal, registry cleaning, RAM boosting, or generic “optimization.”
- Full file-manager, package-manager, terminal, disk-recovery, remote-administration, or cloud-monitoring replacement.
- Arbitrary system tweaks or arbitrary privileged command execution.
- Automatic duplicate deletion or a dangerous “clean everything” action.
- Permanent background daemon or continuous filesystem scanning.
- Guaranteed availability of sensors, SMART, Flatpak, firmware, journal, or desktop-specific capabilities when the host does not expose them.
- Foreground application and screen-time tracking in v1; revisit only after reliable privacy-preserving support exists across relevant X11 and Wayland desktops.
- First-class support for non-Ubuntu distribution families in v1.

## Users

| User | What they need |
|---|---|
| Ubuntu-based desktop user | A clear answer to what needs attention and a safe path to act |
| User on older or storage-limited hardware | Useful monitoring and cleanup that consume few resources |
| Developer or experienced Linux user | Accurate system data, transparent evidence, capability details, and concise reports |
| User seeking support | A redacted snapshot they can inspect before sharing |

## v1 scope

### Overview

- CPU, memory, primary storage, temperature, and battery cards where applicable.
- Attention items for actual detected conditions: updates, reclaimable storage, failed services, and critical SMART indicators.
- Severity, evidence, and destination for every attention item; no opaque health score.
- Quick actions for storage analysis, updates, diagnostics, cleanup, and reports.
- Recent maintenance and diagnostic events. Greeting is optional.

### System

- Overall and per-core CPU usage, frequency where available, load average, model, and temperature.
- Total, used, available, cached, and swap memory using correct Linux memory semantics.
- Mounted filesystem capacity, activity rates, mount point, and filesystem type.
- Interface status and upload/download rates without packet inspection.
- Sortable process list with name, PID, user, CPU, and memory.
- Process termination only after confirmation; privilege requested only when needed.

### Storage and files

- Capacity overview for relevant partitions and mounts.
- Cancellable directory analysis with drill-down and exclusions.
- Cancellable large-file scan showing name, full path, size, and modification time.
- Duplicate pipeline: size grouping, candidate comparison, partial hashing, then full hashing.
- Duplicate results show all copies, paths, dates, size, and digest. User selects every removal; no automatic deletion.

### Cleanup and history

- Analyze package cache, user cache, thumbnail cache, Trash, and logs maintained through appropriate system mechanisms.
- Flow: scan, analyze, review, confirm, execute, result.
- Exact targets and estimated recovery shown before execution.
- Prefer Trash where supported; permanent deletion receives stronger confirmation.
- Record operation, time, result, recovered bytes, errors, cancellation, and partial success.

### Updates

- APT package updates, including package, installed version, available version, security classification, and download size when available.
- Flatpak application and firmware updates through optional capability adapters.
- Explicit initiation, review, Polkit authorization, progress, cancellation only where backend-safe, and final result.
- Never store administrator credentials. Never treat update failure as proof of system damage.

### Diagnostics and storage health

- Failed service status and last known failure evidence.
- Journal errors and warnings grouped by stable event identity and occurrence period rather than raw floods.
- Diagnostic presentation explicitly separates `Detected`, `Possible cause`, and `Recommendation`.
- SMART health, temperature, power-on hours, model, firmware, and critical attributes where available.
- Missing hardware interfaces or permissions produce clear availability guidance.

### Reports

- OS, kernel, desktop, CPU, RAM, GPU, storage, battery, health, update, diagnostics, and recent maintenance sections where available.
- Copy to clipboard and save as local text.
- Default redaction of usernames, hostnames, IP addresses, serial numbers, sensitive paths, and process arguments.
- Show report contents before users share them; Maintainer never uploads reports.

### Settings

- Theme, language, startup preference, and window behavior.
- Monitoring intervals with a 2-second default.
- Scan roots, exclusions, large-file threshold, and duplicate options.
- Update, storage, and maintenance notifications.
- Clear local data controls and an explicit privacy summary.

## User stories

Reference these IDs from issues, commits, and pull requests.

| ID | As a | I want | So that | Done when |
|---|---|---|---|---|
| US-01 | Desktop user | See a concise overview | I know what deserves attention | Current metrics and evidence-backed attention items load without blocking the UI |
| US-02 | Desktop user | Inspect live system and process use | I can identify resource pressure | Metrics use correct units/semantics and the process list sorts correctly |
| US-03 | Storage-limited user | Analyze directories and large files | I can find what fills storage | Scan supports progress, exclusion, cancellation, drill-down, and full paths |
| US-04 | Storage-limited user | Find confirmed duplicate files | I can recover space safely | Results require full-content confirmation and never preselect automatic deletion |
| US-05 | Desktop user | Preview and run cleanup | I understand and control removal | Exact targets, estimated recovery, confirmation, per-item results, and history exist |
| US-06 | Desktop user | Review and install updates | I can keep the system current | Updates are classified, explicitly approved, Polkit-authorized, and reported |
| US-07 | Troubleshooting user | Inspect failed services and repeated errors | I can investigate evidence | Events are grouped and facts remain distinct from hypotheses |
| US-08 | Hardware-conscious user | Inspect drive health | I can recognize reported storage risk | SMART data or a precise unavailable state is shown without invented certainty |
| US-09 | Support-seeking user | Generate a redacted report | I can share useful context safely | Local preview, default redaction, copy, and text export work without upload |
| US-10 | Experienced user | Understand unavailable capabilities | I can act without guessing | Every optional feature explains missing backend, hardware, or permission |
| US-11 | Desktop user | Terminate a selected process | I can respond to a runaway process | Identity is revalidated, confirmation is required, and result is explicit |
| US-12 | Privacy-conscious user | Delete Maintainer history | I control local application data | Selected or all persistent history can be removed and no cloud copy exists |

## Architecture and security requirements

Maintainer has three security boundaries:

1. Unprivileged GTK4 application for views and user interaction.
2. Core Rust services for typed collectors, capability adapters, jobs, and persistence.
3. Small audited D-Bus/Polkit helper exposing fixed validated operations only.

Requirements:

- UI code never executes system operations directly.
- No shell command construction and no arbitrary command API.
- External utilities receive explicit argument arrays and have isolated parsers.
- Filesystem targets are validated against operation policy and rechecked at execution.
- Expensive work runs outside the GTK main thread and returns typed progress/results.
- Jobs are bounded and cancellable; partial failure remains visible.
- Optional capabilities report available, unavailable with reason, or permission required.
- SQLite stores settings and durable application history, not high-frequency live metrics.
- No permanent daemon.

## Performance requirements

Measured on a documented low-end reference machine using a release build:

| Measure | Requirement |
|---|---|
| Idle resident memory | Target 70 MB or less where feasible; hard ceiling 100 MB |
| Average idle CPU | 0.5% or less |
| Cold start | 2 seconds or less |
| Default metric refresh | 2 seconds |
| Hidden-window behavior | Monitoring frequency reduced without making displayed data misleading |
| Scan responsiveness | UI remains interactive; progress and cancellation are available |
| Background work | No unsolicited deep scan or permanent monitoring service |

Performance measurements must state hardware, distribution, desktop session, build profile, sample duration, and enabled page.

## Compatibility requirements

- Product target: current Ubuntu-based distributions with Ubuntu 22.04 as compatibility floor.
- Distribution: native `.deb` packages through an APT repository.
- UI: GTK4 without libadwaita.
- Capabilities vary by distribution, desktop, hardware, and installed services; unsupported optional features degrade honestly.
- Release validation uses representative Ubuntu, Linux Mint, and at least one additional Ubuntu-derived environment at the compatibility floor and current base.

## Delivery phases

Vertical slices ship internally in this order. Shared foundations are added only when a slice needs them.

| Phase | Includes | Exit criterion |
|---|---|---|
| 1 — Overview and monitoring | US-01, US-02, capability states, metric collectors, basic shell | Accurate live flow meets idle/startup budgets on reference hardware |
| 2 — Storage and files | US-03, US-04, cancellable job framework | Large-file and duplicate scans are accurate, bounded, cancellable, and safe on fixtures and real test trees |
| 3 — Cleanup and history | US-05, US-12, SQLite persistence | Preview-confirm-execute-result flow handles success, cancellation, denial, and partial failure |
| 4 — Diagnostics and reports | US-07, US-08, US-09, US-10 | Services, journal, SMART, redaction, copy, and export pass fixture and capability tests |
| 5 — Updates and privilege helper | US-06, US-11, D-Bus/Polkit helper | Threat review and authorization/validation/denial tests pass; APT flow succeeds in isolated test systems |
| 6 — v1 hardening | Compatibility, accessibility, performance, packaging | Supported-system matrix, security checks, keyboard review, budgets, and `.deb` installation pass |
| Later | Foreground application and screen-time tracking; non-Ubuntu distribution families | Separate approved specification required |

## v1 release criteria

- Every user story above meets its observable completion condition.
- No known path permits arbitrary privileged execution or unconfirmed destructive action.
- Monitoring and scan work never block the GTK main thread.
- Performance contract passes on the documented reference system.
- Keyboard operation, visible focus, accessible names, contrast, and non-color status cues pass review.
- Representative supported environments complete install, launch, monitoring, cleanup, diagnostics, report, and update smoke tests.
- No telemetry or hidden network request exists; repository access occurs only for explicit update features.
- Known unsupported capabilities show useful explanations.

## Decisions required before public release

| Decision | Needed by |
|---|---|
| Exact low-end benchmark hardware and measurement procedure | Phase 1 exit |
| Initial published package architectures | Packaging work in Phase 6 |
| Public name, package identifier, application ID, repository, and trademark availability | Before public packaging |
