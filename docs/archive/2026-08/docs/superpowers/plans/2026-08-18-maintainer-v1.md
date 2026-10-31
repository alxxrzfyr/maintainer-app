# Maintainer v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build and package Maintainer v1 as a lightweight, native GTK4 application that monitors, maintains, diagnoses, and reports on Ubuntu-based Linux systems without a daemon or cloud service.

**Architecture:** Use a Cargo workspace with an unprivileged GTK4 application, a UI-independent core library, and a narrowly scoped D-Bus/Polkit helper. Core features are capability adapters and cancellable jobs returning typed snapshots, progress, and per-item results; GTK views consume presentation models and never access Linux interfaces, SQLite, or privileged operations directly.

**Tech Stack:** Rust 1.85.0, Cargo, GTK4/GLib/GIO with GTK 4.6 API floor, SQLite through `rusqlite`, D-Bus through `zbus`, BLAKE3 duplicate hashing, native Linux interfaces, systemd/PackageKit/UDisks2 D-Bus, selected Linux utilities through explicit argument arrays, Debian packaging, Polkit, and GitHub Actions.

## Global Constraints

- Product target: current Ubuntu-based distributions with Ubuntu 22.04 as compatibility floor.
- UI: GTK4 only; no libadwaita, WebView, Electron, browser runtime, JavaScript toolchain, ESLint, or Prettier.
- Distribution: native `.deb` package and APT repository metadata; public repository signing is outside source implementation and secrets never enter the repository.
- Privilege: GUI always unprivileged; helper exposes fixed typed methods, validates callers and parameters, and never accepts arbitrary commands.
- Privacy: local only, no telemetry, no account, no report upload, and no persistence of high-frequency live metrics.
- Activity tracking: foreground application and screen-time tracking are not part of v1.
- Performance: target idle RSS is 70 MB or less where feasible; hard ceiling is 100 MB; average idle CPU is 0.5% or less; cold start is 2 seconds or less; default refresh is 2 seconds.
- Background behavior: no permanent daemon, unsolicited deep scan, or continuous animation; monitoring slows while hidden.
- Safety: destructive operations show exact targets, require confirmation, prefer Trash, revalidate identity/path at execution, and return per-item results.
- Reliability: optional capabilities return available, unavailable with reason, or permission required; unsupported capability is not an exceptional crash.
- Quality: TDD for each behavior, `rustfmt`, Clippy with warnings denied, targeted tests during tasks, full workspace checks at phase gates, and no ignored production `Result`.
- Dependency policy: add dependencies only in the task that needs them, pin exact direct versions, explain each addition, and let Cargo generate `Cargo.lock`; never edit the lockfile manually.
- Commit policy: perform commit steps only when the user has explicitly authorized commits. Otherwise stop at the stated review checkpoint with files unstaged.

---

## Locked Workspace and File Structure

```text
.
├── Cargo.toml                         # workspace members, shared metadata, exact dependency versions
├── Cargo.lock                         # Cargo-generated dependency lock
├── rust-toolchain.toml                # pinned Rust 1.85.0 toolchain and components
├── rustfmt.toml                       # formatting policy
├── clippy.toml                        # lint configuration that does not weaken warnings
├── crates/
│   ├── maintainer-core/
│   │   ├── Cargo.toml
│   │   └── src/
│   │       ├── lib.rs
│   │       ├── error.rs               # typed subsystem and operation errors
│   │       ├── capability.rs          # availability and permission states
│   │       ├── jobs.rs                # cancellation, progress, bounded job execution
│   │       ├── command.rs             # direct-process runner and parser boundary
│   │       ├── metrics/               # CPU, memory, disk, network, temperature, process models/adapters
│   │       ├── storage/               # traversal, large files, duplicates, scan policy
│   │       ├── cleanup/               # analyzers, plans, execution contracts, Trash behavior
│   │       ├── diagnostics/           # service, journal, and SMART adapters
│   │       ├── updates/               # update models and PackageKit/APT adapters
│   │       ├── reports/               # report assembly, redaction, and text rendering
│   │       ├── persistence/           # SQLite connection, migrations, repositories
│   │       └── settings.rs            # validated durable preferences
│   ├── maintainer-app/
│   │   ├── Cargo.toml
│   │   ├── src/
│   │   │   ├── main.rs
│   │   │   ├── application.rs         # GTK application lifecycle and dependency assembly
│   │   │   ├── state.rs               # presentation state and typed intents
│   │   │   └── ui/                    # shell, views, reusable native widgets, dialogs
│   │   └── tests/                     # GTK state and accessibility smoke tests
│   └── maintainer-helper/
│       ├── Cargo.toml
│       ├── src/
│       │   ├── main.rs                # D-Bus service lifecycle
│       │   ├── api.rs                 # fixed public D-Bus methods and DTOs
│       │   ├── authorize.rs           # Polkit checks and caller identity
│       │   ├── validate.rs            # operation-specific parameter validation
│       │   └── operations/            # fixed process, cleanup, log, SMART fallback operations
│       └── tests/                     # authorization, denial, validation, and PID/path-race tests
├── data/
│   ├── io.github.maintainerapp.Maintainer.desktop
│   ├── io.github.maintainerapp.Maintainer.metainfo.xml
│   ├── io.github.maintainerapp.Maintainer.gschema.xml
│   ├── icons/
│   ├── resources.gresource.xml
│   ├── styles/light.css
│   ├── styles/dark.css
│   ├── dbus/io.github.maintainerapp.Helper.service
│   └── polkit/io.github.maintainerapp.policy
├── migrations/001_initial.sql
├── fixtures/                          # sanitized proc, sys, command, journal, SMART, and package fixtures
├── tests/system/                      # opt-in host and VM smoke tests
├── packaging/debian/                  # reproducible Debian source-package files
├── scripts/measure-performance.sh
├── scripts/test-package.sh
└── .github/workflows/ci.yml
```

`io.github.maintainerapp.Maintainer` is the development application ID. Before public packaging, complete the PRD name/identifier ownership check and change every desktop, schema, D-Bus, Polkit, metainfo, and packaging reference atomically if ownership differs.

## Core Interfaces Shared Across Tasks

```rust
pub enum Capability<T> {
    Available(T),
    Unavailable(UnavailableReason),
    PermissionRequired(PermissionRequest),
}

pub struct UnavailableReason {
    pub code: &'static str,
    pub message: String,
    pub remediation: Option<String>,
}

pub trait Cancellation: Send + Sync {
    fn is_cancelled(&self) -> bool;
}

pub struct JobProgress {
    pub completed_units: u64,
    pub total_units: Option<u64>,
    pub current_item: Option<String>,
}

pub trait ProgressSink: Send + Sync {
    fn report(&self, progress: JobProgress);
}

pub struct ItemResult<T> {
    pub item: T,
    pub outcome: OperationOutcome,
}

pub enum OperationOutcome {
    Succeeded,
    Skipped { reason: String },
    Failed { message: String },
    Cancelled,
}
```

Every task uses these contracts rather than inventing page-specific availability, cancellation, or error shapes.

---

### Task 1: Scaffold the Reproducible Cargo Workspace and Quality Gates

**Files:**
- Create: `Cargo.toml`
- Create: `Cargo.lock` through Cargo
- Create: `rust-toolchain.toml`
- Create: `rustfmt.toml`
- Create: `clippy.toml`
- Create: `crates/maintainer-core/Cargo.toml`
- Create: `crates/maintainer-core/src/lib.rs`
- Create: `crates/maintainer-app/Cargo.toml`
- Create: `crates/maintainer-app/src/main.rs`
- Create: `crates/maintainer-helper/Cargo.toml`
- Create: `crates/maintainer-helper/src/main.rs`
- Create: `.github/workflows/ci.yml`
- Modify: `AGENTS.md`
- Modify: `README.md`

**Interfaces:**
- Consumes: approved repository documents only.
- Produces: buildable workspace members `maintainer-core`, `maintainer-app`, and `maintainer-helper`; verified development commands documented in `AGENTS.md`.

- [ ] **Step 1: Verify the Ubuntu 22.04 native-library floor before choosing crate versions**

Run in an Ubuntu 22.04 development environment:

```bash
pkg-config --modversion gtk4
rustc --version
cargo --version
```

Expected: GTK reports at least `4.6`; Rust is installable through rustup. If GTK is below 4.6, install Ubuntu's `libgtk-4-dev` rather than raising the product floor.

- [ ] **Step 2: Add the pinned toolchain and empty workspace manifests**

Use Rust `1.85.0`, edition `2024`, resolver `2`, and exact direct dependency requirements. Initial GTK-family pins:

```toml
[workspace]
resolver = "2"
members = [
  "crates/maintainer-core",
  "crates/maintainer-app",
  "crates/maintainer-helper",
]

[workspace.package]
edition = "2024"
rust-version = "1.85"
license = "GPL-3.0-or-later"

[workspace.dependencies]
glib = "=0.20.12"
gio = "=0.20.12"
gtk4 = { version = "=0.9.7", features = ["v4_6"] }
thiserror = "=2.0.12"
```

Before writing these pins, verify they resolve together with `cargo info`; if a listed exact release has been yanked, select the newest non-yanked patch in the same compatible release line and record the reason in the commit/review note.

- [ ] **Step 3: Write a workspace smoke test before exporting any core API**

```rust
#[cfg(test)]
mod tests {
    #[test]
    fn core_crate_is_linked() {
        assert_eq!(maintainer_core::PRODUCT_NAME, "Maintainer");
    }
}
```

Place this in `crates/maintainer-app/src/main.rs` beneath an empty `main` function.

- [ ] **Step 4: Run the test and verify the expected failure**

Run:

```bash
cargo test -p maintainer-app core_crate_is_linked
```

Expected: compilation fails because `maintainer_core::PRODUCT_NAME` is not defined.

- [ ] **Step 5: Export the minimal core constant and build all binaries**

```rust
pub const PRODUCT_NAME: &str = "Maintainer";
```

Keep both binaries side-effect free: app exits successfully until GTK lifecycle arrives; helper exits successfully until its D-Bus lifecycle arrives.

- [ ] **Step 6: Add CI and run local quality gates**

CI installs `libgtk-4-dev`, `libsqlite3-dev`, `libpolkit-gobject-1-dev`, `pkg-config`, and `xvfb` on Ubuntu 22.04, then runs:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
```

Expected: all commands exit `0`; no ESLint, Prettier, Node, npm, or pnpm step exists.

- [ ] **Step 7: Document only commands that passed**

Replace the “no commands available” section in `AGENTS.md` and `README.md` with the exact successful commands. Include single-test syntax:

```bash
cargo test -p maintainer-core module::tests::test_name -- --exact
```

- [ ] **Step 8: Review checkpoint and optional commit**

Confirm the lockfile was generated by Cargo and Ubuntu 22.04 CI does not request GTK APIs newer than 4.6. If commits are authorized:

```bash
git add Cargo.toml Cargo.lock rust-toolchain.toml rustfmt.toml clippy.toml crates .github/workflows/ci.yml AGENTS.md README.md
git commit -m "build: scaffold Maintainer Rust workspace"
```

---

### Task 2: Define Errors, Capabilities, Commands, and Cancellable Jobs

**Files:**
- Create: `crates/maintainer-core/src/error.rs`
- Create: `crates/maintainer-core/src/capability.rs`
- Create: `crates/maintainer-core/src/jobs.rs`
- Create: `crates/maintainer-core/src/command.rs`
- Modify: `crates/maintainer-core/src/lib.rs`
- Test: unit tests beside each module

**Interfaces:**
- Consumes: Rust standard library and `thiserror`.
- Produces: `Capability<T>`, `UnavailableReason`, `PermissionRequest`, `CancellationToken`, `ProgressSink`, `JobProgress`, `ItemResult<T>`, `OperationOutcome`, `CommandSpec`, and `CommandRunner`.

- [ ] **Step 1: Write failing capability-state tests**

```rust
#[test]
fn unavailable_preserves_reason_and_remediation() {
    let value = Capability::<()>::Unavailable(UnavailableReason {
        code: "sensor-missing",
        message: "No readable temperature sensor was found".into(),
        remediation: Some("Install a supported hwmon driver".into()),
    });
    assert_eq!(value.reason().unwrap().code, "sensor-missing");
}
```

Run `cargo test -p maintainer-core capability` and expect unresolved types/methods.

- [ ] **Step 2: Implement the shared capability and error models**

Implement the interfaces in “Core Interfaces Shared Across Tasks,” plus:

```rust
pub struct PermissionRequest {
    pub action_id: String,
    pub explanation: String,
}

#[derive(Debug, thiserror::Error)]
pub enum MaintainerError {
    #[error("{context}: {source}")]
    Io { context: String, source: std::io::Error },
    #[error("invalid data from {source_name}: {message}")]
    InvalidData { source_name: &'static str, message: String },
    #[error("operation cancelled")]
    Cancelled,
}
```

Do not expose raw command output in user-facing error messages.

- [ ] **Step 3: Write cancellation and bounded-progress tests**

Test cancellation before work starts, during iteration, duplicate terminal progress, and progress where total size is unknown. Assert cancelled jobs return `MaintainerError::Cancelled`, not a generic I/O failure.

- [ ] **Step 4: Implement cooperative cancellation and a bounded worker pool**

`CancellationToken` uses `Arc<AtomicBool>`. `JobExecutor::new(max_workers)` rejects zero, never starts more than `max_workers`, catches worker panics at the boundary, and sends typed completion through a bounded channel.

- [ ] **Step 5: Write command-runner denial tests**

```rust
#[test]
fn command_spec_rejects_shell_execution() {
    let result = CommandSpec::new("sh", ["-c", "rm -rf /tmp/example"]);
    assert!(matches!(result, Err(CommandSpecError::ForbiddenProgram(_))));
}
```

Also test absolute/allowlisted program resolution, explicit argument preservation, `LC_ALL=C`, output-size cap, non-zero exit, timeout, and cancellation.

- [ ] **Step 6: Implement `CommandRunner` without a shell**

```rust
pub trait CommandRunner: Send + Sync {
    fn run(
        &self,
        spec: &CommandSpec,
        cancel: &dyn Cancellation,
    ) -> Result<CommandOutput, MaintainerError>;
}
```

Use `std::process::Command`; close stdin unless explicitly required; cap captured stdout/stderr; kill and reap on cancellation/timeout; preserve exit status separately from sanitized diagnostics.

- [ ] **Step 7: Run quality gates and optional commit**

Run:

```bash
cargo test -p maintainer-core capability jobs command
cargo clippy -p maintainer-core --all-targets -- -D warnings
```

If commits are authorized:

```bash
git add crates/maintainer-core
git commit -m "feat(core): add capability and job contracts"
```

---

### Task 3: Build the Native GTK4 Shell and Design Tokens

**Files:**
- Create: `crates/maintainer-app/src/application.rs`
- Create: `crates/maintainer-app/src/state.rs`
- Create: `crates/maintainer-app/src/ui/mod.rs`
- Create: `crates/maintainer-app/src/ui/shell.rs`
- Create: `crates/maintainer-app/src/ui/navigation.rs`
- Create: `crates/maintainer-app/src/ui/widgets/status_view.rs`
- Create: `data/resources.gresource.xml`
- Create: `data/styles/light.css`
- Create: `data/styles/dark.css`
- Modify: `crates/maintainer-app/src/main.rs`
- Test: `crates/maintainer-app/tests/shell_state.rs`

**Interfaces:**
- Consumes: GTK4 4.6 API, GIO resources, core capability states.
- Produces: `AppIntent`, `AppState`, `Page`, `MaintainerApplication`, and `StatusView`; later views mount through `Shell::set_page_content`.

- [ ] **Step 1: Write failing pure state tests**

```rust
#[test]
fn navigation_selects_requested_page() {
    let mut state = AppState::default();
    state.reduce(AppIntent::Navigate(Page::Storage));
    assert_eq!(state.active_page, Page::Storage);
}
```

Test all v1 pages and verify no `Activity` page variant exists.

- [ ] **Step 2: Implement page and intent state without GTK types**

```rust
pub enum Page { Overview, System, Storage, Cleanup, Updates, Diagnostics, Reports, Settings }
pub enum AppIntent { Navigate(Page), WindowVisibilityChanged(bool), RetryCapability(&'static str) }
```

`AppState` owns presentation state only. System adapters remain injected services, not globals.

- [ ] **Step 3: Add GTK resource compilation and semantic CSS**

Encode every light/dark token from `DESIGN.md` as GTK named colors. Add classes for page backgrounds, surfaces, navigation selection, metric values, status rows, focus, and destructive buttons. Do not add shadows, gradients, blur, or web CSS syntax unsupported by GTK.

- [ ] **Step 4: Implement application lifecycle and shell**

Create one `gtk4::Application`, one primary `ApplicationWindow`, persistent navigation at 900 px and above, compact navigation at 640–899 px, and drawer/page switcher below 640 px. Use GTK 4.6-compatible widgets only.

- [ ] **Step 5: Add reusable availability rendering**

`StatusView::render(CapabilityViewModel)` has explicit loading, unavailable, permission-required, empty, failure, and cancelled modes. Accessible names include the status and remediation; retry appears only when retry can change state.

- [ ] **Step 6: Run headless shell tests**

Run:

```bash
xvfb-run -a cargo test -p maintainer-app --test shell_state
cargo clippy -p maintainer-app --all-targets -- -D warnings
```

Expected: navigation state passes, application constructs under Xvfb, and no GTK critical warning is emitted.

- [ ] **Step 7: Review checkpoint and optional commit**

Compare rendered spacing, colors, focus, 640/900 px adaptation, and navigation labels against `DESIGN.md`. If authorized:

```bash
git add crates/maintainer-app data/styles data/resources.gresource.xml
git commit -m "feat(ui): add GTK application shell"
```

---

### Task 4: Implement Accurate Live Metric Collectors

**Files:**
- Create: `crates/maintainer-core/src/metrics/mod.rs`
- Create: `crates/maintainer-core/src/metrics/cpu.rs`
- Create: `crates/maintainer-core/src/metrics/memory.rs`
- Create: `crates/maintainer-core/src/metrics/disk.rs`
- Create: `crates/maintainer-core/src/metrics/network.rs`
- Create: `crates/maintainer-core/src/metrics/temperature.rs`
- Create: `crates/maintainer-core/src/metrics/processes.rs`
- Create: `fixtures/proc/`
- Create: `fixtures/sys/`
- Test: unit tests beside each parser

**Interfaces:**
- Consumes: `Capability<T>`, `Cancellation`, filesystem readers, monotonic sample times.
- Produces: `MetricSource` and typed `SystemSnapshot` values with explicit units.

```rust
pub trait MetricSource: Send + Sync {
    fn sample(&mut self, now: Instant) -> Result<SystemSnapshot, MaintainerError>;
}

pub struct SystemSnapshot {
    pub cpu: Capability<CpuSnapshot>,
    pub memory: Capability<MemorySnapshot>,
    pub disks: Capability<Vec<DiskSnapshot>>,
    pub network: Capability<Vec<NetworkSnapshot>>,
    pub temperatures: Capability<Vec<TemperatureReading>>,
    pub processes: Capability<Vec<ProcessSnapshot>>,
}
```

- [ ] **Step 1: Add sanitized fixtures and failing parser tests**

Cover first CPU sample with no rate, multi-core deltas, counter reset, zero elapsed time, missing meminfo keys, `MemAvailable`, swap, mount names containing spaces, network counter wrap/reset, absent hwmon labels, process names with parentheses, vanished PIDs, and permission-denied process fields.

- [ ] **Step 2: Implement CPU and memory parsing**

CPU percentage uses deltas between monotonic samples and never divides by zero. Memory exposes total, available, used, cache, and swap separately; user-facing “used” follows Linux available-memory semantics rather than counting reclaimable cache as waste.

- [ ] **Step 3: Implement mount and disk-rate collection**

Read mount metadata without treating pseudo filesystems as cleanup targets. Compute read/write bytes per second from sector deltas and elapsed duration; mark reset devices as a fresh baseline.

- [ ] **Step 4: Implement network and temperature capability adapters**

Exclude loopback from default summary but retain it in details. Enumerate `/sys/class/hwmon`; invalid/unreadable sensors become unavailable entries rather than zero Celsius.

- [ ] **Step 5: Implement process snapshots with PID identity**

```rust
pub struct ProcessIdentity {
    pub pid: u32,
    pub start_time_ticks: u64,
}
```

Store start time to prevent PID-reuse termination. Parse UID, resident bytes, command name, and CPU deltas. Do not persist command lines or show arguments by default.

- [ ] **Step 6: Verify fixture and host smoke behavior**

Run:

```bash
cargo test -p maintainer-core metrics
cargo test -p maintainer-core --test host_metrics -- --ignored
```

Host smoke test asserts no panic and plausible ranges; fixture tests assert exact values.

- [ ] **Step 7: Optional commit**

```bash
git add crates/maintainer-core/src/metrics fixtures/proc fixtures/sys
git commit -m "feat(metrics): collect Linux system snapshots"
```

---

### Task 5: Deliver Overview, System Monitor, and Safe Process Termination

**Files:**
- Create: `crates/maintainer-app/src/ui/overview.rs`
- Create: `crates/maintainer-app/src/ui/system.rs`
- Create: `crates/maintainer-app/src/ui/widgets/metric_card.rs`
- Create: `crates/maintainer-app/src/ui/widgets/sparkline.rs`
- Create: `crates/maintainer-core/src/metrics/terminate.rs`
- Modify: `crates/maintainer-app/src/application.rs`
- Modify: `crates/maintainer-app/src/state.rs`
- Test: `crates/maintainer-app/tests/monitor_state.rs`

**Interfaces:**
- Consumes: `MetricSource`, `SystemSnapshot`, `ProcessIdentity`, window visibility intents.
- Produces: `MonitoringService`, `OverviewViewModel`, `SystemViewModel`, `ProcessSort`, and `ProcessTerminator`.

- [ ] **Step 1: Write reducer tests for incremental updates and visibility**

Assert a CPU update changes only CPU presentation state, process sorting is stable by CPU/memory/name, hidden-window state changes sampling from 2 seconds to 10 seconds, and returning visible triggers an immediate sample.

- [ ] **Step 2: Implement `MonitoringService`**

Run one bounded collector job per tick; coalesce delayed ticks instead of creating a backlog. Send typed messages to GLib main context. Never construct or mutate GTK objects in workers.

- [ ] **Step 3: Implement metric cards and bounded sparklines**

Store at most 60 samples per metric. Sparklines draw only on new data and expose current value/trend as accessible text. Missing data renders reason text, not zero.

- [ ] **Step 4: Implement Overview attention derivation**

Attention rules use explicit evidence: filesystem threshold, update count, failed services, and SMART critical state. Do not infer general health from transient CPU load. Each item includes severity, evidence, and destination page.

- [ ] **Step 5: Write process-termination race tests**

Use a fake process table to test same PID/same start time, PID reused with different start time, already exited, current-user denial from OS, and cancellation before signal.

- [ ] **Step 6: Implement unprivileged process termination**

Re-read `/proc/<pid>/stat`, compare `start_time_ticks`, then send SIGTERM. Do not escalate automatically. Return `PermissionRequired` for foreign processes; helper support is connected in Task 14. Confirmation dialog includes name, PID, user, and signal behavior.

- [ ] **Step 7: Run monitor tests and measure early idle baseline**

```bash
xvfb-run -a cargo test -p maintainer-app monitor
cargo test -p maintainer-core terminate
/usr/bin/time -v target/release/maintainer-app
```

Record startup elapsed time and maximum RSS as an early baseline; do not claim the final contract from a single short run.

- [ ] **Step 8: Optional commit**

```bash
git add crates/maintainer-core/src/metrics crates/maintainer-app/src
git commit -m "feat: add live system monitoring"
```

---

### Task 6: Add SQLite Settings and Durable Operation History

**Files:**
- Create: `migrations/001_initial.sql`
- Create: `crates/maintainer-core/src/persistence/mod.rs`
- Create: `crates/maintainer-core/src/persistence/database.rs`
- Create: `crates/maintainer-core/src/persistence/migrations.rs`
- Create: `crates/maintainer-core/src/persistence/history.rs`
- Create: `crates/maintainer-core/src/settings.rs`
- Test: unit tests using temporary SQLite databases

**Interfaces:**
- Consumes: local XDG data/config paths, typed operation results.
- Produces: `Database`, `SettingsRepository`, `HistoryRepository`, `Settings`, `HistoryEntry`, and schema version `1`.

- [ ] **Step 1: Add exact SQLite dependency and failing migration tests**

Pin `rusqlite = "=0.32.1"` without the bundled feature so `.deb` uses Ubuntu's supported SQLite library. Test fresh creation, reopening, transaction rollback, unsupported future schema, and migration idempotence.

- [ ] **Step 2: Define the initial schema**

Create tables for schema migrations, settings, operation history, scan metadata, and diagnostic history. Store byte counts as checked non-negative integers, timestamps as UTC Unix seconds, operation status as constrained text, and structured non-sensitive details as versioned JSON. Do not create live-metric or application-usage tables.

- [ ] **Step 3: Implement database opening and migration transactions**

Use XDG data directories with user-only permissions. Enable foreign keys and busy timeout. Each migration runs once in a transaction; a failed migration leaves the prior version intact.

- [ ] **Step 4: Write repository behavior tests**

Test default settings, validated refresh interval, exclusions round trip, history insertion ordering, partial failure details, delete one history entry, delete all application data, and malformed stored value fallback with an explicit warning.

- [ ] **Step 5: Implement typed repositories**

```rust
pub trait HistoryRepository: Send + Sync {
    fn record(&self, entry: &HistoryEntry) -> Result<(), MaintainerError>;
    fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, MaintainerError>;
    fn delete_all(&self) -> Result<u64, MaintainerError>;
}
```

Clamp monitoring intervals to safe bounds; default remains 2 seconds.

- [ ] **Step 6: Run migration/repository tests and optional commit**

```bash
cargo test -p maintainer-core persistence settings
```

```bash
git add migrations crates/maintainer-core/src/persistence crates/maintainer-core/src/settings.rs Cargo.toml Cargo.lock
git commit -m "feat(data): add settings and operation history"
```

---

### Task 7: Implement Cancellable Storage and Large-File Scanning

**Files:**
- Create: `crates/maintainer-core/src/storage/mod.rs`
- Create: `crates/maintainer-core/src/storage/policy.rs`
- Create: `crates/maintainer-core/src/storage/traverse.rs`
- Create: `crates/maintainer-core/src/storage/large_files.rs`
- Create: `crates/maintainer-core/src/storage/mounts.rs`
- Test: temporary-tree and fixture tests

**Interfaces:**
- Consumes: `Cancellation`, `ProgressSink`, selected roots, exclusions, mount policy.
- Produces: `ScanPolicy`, `StorageScanner`, `DirectoryUsage`, `LargeFile`, `ScanSummary`, and `ScanIssue`.

- [ ] **Step 1: Write temporary-tree safety tests**

Cover nested directories, unreadable paths, excluded path, symlink to outside root, symlink cycle, hard link, sparse file, crossing mount boundary through a fake metadata source, file disappearing during scan, cancellation, and non-UTF-8 filename.

- [ ] **Step 2: Implement explicit scan policy**

```rust
pub struct ScanPolicy {
    pub roots: Vec<PathBuf>,
    pub exclusions: Vec<PathBuf>,
    pub cross_filesystems: bool,
    pub follow_symlinks: bool,
    pub large_file_threshold_bytes: u64,
}
```

Defaults: do not follow symlinks, do not cross filesystems, threshold 1 GiB, and never silently add system roots.

- [ ] **Step 3: Implement iterative traversal**

Use an explicit stack rather than recursion. Compare device/inode identity, report recoverable permission/disappearance issues, check cancellation at least once per directory and every 256 entries, and emit throttled progress no more than 10 times per second.

- [ ] **Step 4: Compute directory and file sizes safely**

Report apparent bytes and allocated bytes separately where metadata supports both. Count hard-linked file content once per scan summary while still listing each path for duplicate-awareness.

- [ ] **Step 5: Implement large-file ranking**

Use a bounded min-heap for top results rather than retaining every file. Preserve raw `OsString` paths internally and provide lossy display text only to UI.

- [ ] **Step 6: Run tests and optional commit**

```bash
cargo test -p maintainer-core storage::
```

```bash
git add crates/maintainer-core/src/storage
git commit -m "feat(storage): add cancellable usage scanning"
```

---

### Task 8: Implement Confirmed Duplicate Detection

**Files:**
- Create: `crates/maintainer-core/src/storage/duplicates.rs`
- Create: `crates/maintainer-core/src/storage/hash.rs`
- Test: temporary-tree tests and deterministic fixtures

**Interfaces:**
- Consumes: traversal candidates, `Cancellation`, `ProgressSink`, file identity.
- Produces: `DuplicateScanner`, `DuplicateGroup`, `DuplicateFile`, `ContentDigest`, and `DuplicateIssue`.

- [ ] **Step 1: Pin BLAKE3 and write failing pipeline tests**

Pin `blake3 = "=1.8.2"`. Tests prove: different sizes never hash; same size/different prefix stops before full hash; same partial hash/different content is rejected by full hash; identical content groups; same inode is not presented as reclaimable duplicate; file changed while hashing is rejected; cancellation closes files promptly.

- [ ] **Step 2: Implement staged grouping**

Pipeline order is metadata and size, prefix/suffix partial BLAKE3, then full-file BLAKE3. Re-stat before and after full hashing and require unchanged device, inode, size, and modification time.

- [ ] **Step 3: Bound memory and file descriptors**

Process one size group at a time, limit concurrent hash workers to the job executor bound, and open no more than one file per worker. Progress distinguishes discovery bytes from hashing bytes.

- [ ] **Step 4: Model groups without deletion policy**

```rust
pub struct DuplicateGroup {
    pub digest: ContentDigest,
    pub size_bytes: u64,
    pub files: Vec<DuplicateFile>,
}
```

Core scanner never chooses a file to remove. UI may recommend a keep candidate using path/date criteria, but selection defaults empty.

- [ ] **Step 5: Run exactness and cancellation tests**

```bash
cargo test -p maintainer-core storage::duplicates
cargo test -p maintainer-core storage::hash
```

- [ ] **Step 6: Optional commit**

```bash
git add Cargo.toml Cargo.lock crates/maintainer-core/src/storage
git commit -m "feat(storage): detect confirmed duplicate files"
```

---

### Task 9: Deliver the Storage, Large Files, and Duplicate UI

**Files:**
- Create: `crates/maintainer-app/src/ui/storage/mod.rs`
- Create: `crates/maintainer-app/src/ui/storage/overview.rs`
- Create: `crates/maintainer-app/src/ui/storage/browser.rs`
- Create: `crates/maintainer-app/src/ui/storage/large_files.rs`
- Create: `crates/maintainer-app/src/ui/storage/duplicates.rs`
- Create: `crates/maintainer-app/src/ui/widgets/job_progress.rs`
- Modify: `crates/maintainer-app/src/state.rs`
- Test: `crates/maintainer-app/tests/storage_state.rs`

**Interfaces:**
- Consumes: `StorageScanner`, `DuplicateScanner`, scan settings, progress/results.
- Produces: `StorageIntent`, `StorageState`, virtualized result models, and selection review state.

- [ ] **Step 1: Write reducer tests**

Test root selection, empty selection denial, start/progress/cancel/result state, recoverable issue count, stale result rejection by job ID, directory drill-down, sort stability, duplicate selections defaulting empty, and full-path review before removal.

- [ ] **Step 2: Implement scan orchestration**

Each scan gets an immutable ID and policy snapshot. New scans require cancellation/confirmation of the active scan. A late message from an old job cannot overwrite current state.

- [ ] **Step 3: Build adaptive native lists**

Use `gio::ListStore`, `gtk4::ListView`/`ColumnView`, and factories so large result sets do not create one permanent widget tree per row. Narrow layout retains filename/size and opens details for path/date/hash.

- [ ] **Step 4: Implement review-only removal handoff**

Selected paths become a `RemovalPlanRequest` consumed by cleanup in Task 10. Storage UI cannot call filesystem deletion. Show target count, total apparent bytes, every path, Trash eligibility, and changed-file warnings.

- [ ] **Step 5: Run headless state and large-list smoke tests**

```bash
xvfb-run -a cargo test -p maintainer-app storage_state
```

Create 10,000 in-memory rows and assert construction completes without 10,000 realized row widgets.

- [ ] **Step 6: Optional commit**

```bash
git add crates/maintainer-app/src/ui/storage crates/maintainer-app/src/ui/widgets/job_progress.rs crates/maintainer-app/src/state.rs
git commit -m "feat(ui): add storage analysis workflows"
```

---

### Task 10: Implement Previewed Cleanup, Trash, and History

**Files:**
- Create: `crates/maintainer-core/src/cleanup/mod.rs`
- Create: `crates/maintainer-core/src/cleanup/analyze.rs`
- Create: `crates/maintainer-core/src/cleanup/plan.rs`
- Create: `crates/maintainer-core/src/cleanup/execute.rs`
- Create: `crates/maintainer-core/src/cleanup/trash.rs`
- Create: `crates/maintainer-app/src/ui/cleanup.rs`
- Create: `crates/maintainer-app/src/ui/dialogs/confirm_cleanup.rs`
- Test: temporary XDG directory tests and UI reducer tests

**Interfaces:**
- Consumes: selected category/path requests, settings, `HistoryRepository`, helper client capability.
- Produces: `CleanupAnalyzer`, immutable `CleanupPlan`, `CleanupExecutor`, `CleanupResult`, and `RemovalPlanRequest`.

- [ ] **Step 1: Write allowlist and path-race tests**

Test package cache, thumbnail cache, Trash, explicit user-cache root, unknown category denial, `..`, symlink substitution, root filesystem target, home-directory target, changed inode between plan/execute, cross-filesystem Trash behavior, cancellation, and partial failure.

- [ ] **Step 2: Implement analyzers that never delete**

Each analyzer returns stable target identity, category, display path, apparent/allocated bytes, privilege requirement, reversibility, and evidence timestamp. User cache includes only explicitly supported cache roots; it never treats all of `~/.cache` as safe by default.

- [ ] **Step 3: Implement immutable cleanup plans**

```rust
pub struct CleanupPlan {
    pub id: Uuid,
    pub created_at: SystemTime,
    pub items: Vec<CleanupItem>,
    pub estimated_reclaimable_bytes: u64,
}
```

Pin exact `uuid` dependency. Execution receives the reviewed plan ID and selected item IDs, then revalidates each target.

- [ ] **Step 4: Implement FreeDesktop Trash behavior**

For supported same-filesystem locations, move to `.Trash-$uid` or user data Trash and write matching `.trashinfo` atomically. If Trash is unavailable, return a permanent-deletion requirement; do not silently unlink.

- [ ] **Step 5: Implement execution and durable history**

Execute independent items, check cancellation between items, collect every outcome, recompute actual recovered bytes from successful targets, and record one operation plus per-item sanitized details. Never record secret path segments in logs; history UI may show local paths from its protected database.

- [ ] **Step 6: Implement cleanup UI flow**

Enforce scan, review, confirm, execute, result. Permanent deletion uses stronger copy and no preselected confirmation. No “Clean everything” action exists.

- [ ] **Step 7: Run tests and optional commit**

```bash
cargo test -p maintainer-core cleanup
xvfb-run -a cargo test -p maintainer-app cleanup
```

```bash
git add Cargo.toml Cargo.lock crates/maintainer-core/src/cleanup crates/maintainer-app/src/ui/cleanup.rs crates/maintainer-app/src/ui/dialogs migrations
git commit -m "feat(cleanup): add previewed maintenance flow"
```

---

### Task 11: Implement Failed-Service and Grouped-Journal Diagnostics

**Files:**
- Create: `crates/maintainer-core/src/diagnostics/mod.rs`
- Create: `crates/maintainer-core/src/diagnostics/services.rs`
- Create: `crates/maintainer-core/src/diagnostics/journal.rs`
- Create: `crates/maintainer-core/src/diagnostics/group.rs`
- Create: `crates/maintainer-core/src/diagnostics/interpret.rs`
- Create: `crates/maintainer-app/src/ui/diagnostics.rs`
- Create: `fixtures/systemd/`
- Create: `fixtures/journal/`
- Test: parser, grouping, interpretation, and reducer tests

**Interfaces:**
- Consumes: systemd D-Bus client, `journalctl --output=json` through `CommandRunner`, cancellation.
- Produces: `ServiceDiagnostic`, `JournalEventGroup`, `DiagnosticFinding`, `Evidence`, and cautious interpretation rules.

- [ ] **Step 1: Write service and journal fixture tests**

Cover failed/inactive/activating services, user versus system units, missing systemd, malformed JSON line, binary journal field encoding, duplicate messages with variable PIDs/numbers, rate-limited events, permission denial, cancellation, and output cap.

- [ ] **Step 2: Implement systemd service adapter**

Use systemd D-Bus properties for unit state and failure timestamp. Return unavailable when systemd is absent and permission-required only when the requested detail truly needs it.

- [ ] **Step 3: Implement bounded journal collection**

Invoke `journalctl` directly with explicit arguments, `LC_ALL=C`, JSON output, priority/time filters, line and byte caps. Do not invoke a shell or load the entire journal.

- [ ] **Step 4: Implement stable grouping**

Normalize only documented volatile fields such as PID and standalone timestamps; preserve unit, identifier, priority, message template, first/last occurrence, count, and bounded sample evidence. Unit tests prove unrelated messages never merge due to aggressive digit stripping.

- [ ] **Step 5: Implement evidence-first interpretation**

```rust
pub struct DiagnosticFinding {
    pub detected: String,
    pub evidence: Vec<Evidence>,
    pub possible_causes: Vec<String>,
    pub recommendation: Option<String>,
}
```

Interpretation rules are allowlisted and fixture-tested. Unknown events receive no invented cause.

- [ ] **Step 6: Build diagnostics UI and run tests**

Render Detected, Evidence, Possible cause, Recommendation in that order. Group rows expand to bounded examples.

```bash
cargo test -p maintainer-core diagnostics
xvfb-run -a cargo test -p maintainer-app diagnostics
```

- [ ] **Step 7: Optional commit**

```bash
git add crates/maintainer-core/src/diagnostics crates/maintainer-app/src/ui/diagnostics.rs fixtures/systemd fixtures/journal
git commit -m "feat(diagnostics): group service and journal evidence"
```

---

### Task 12: Add SMART Storage Health Through UDisks2

**Files:**
- Create: `crates/maintainer-core/src/diagnostics/smart.rs`
- Create: `crates/maintainer-core/src/diagnostics/udisks.rs`
- Modify: `crates/maintainer-app/src/ui/diagnostics.rs`
- Create: `fixtures/udisks/`
- Test: adapter and health-classification tests

**Interfaces:**
- Consumes: UDisks2 D-Bus properties; optional helper fallback added in Task 14.
- Produces: `SmartDrive`, `SmartAttribute`, `SmartAssessment`, and `Capability<Vec<SmartDrive>>`.

- [ ] **Step 1: Pin `zbus` and write D-Bus mapping tests**

Pin `zbus = "=5.5.0"`. Fixtures cover ATA SMART available/unavailable, NVMe without ATA interface, temperature Kelvin conversion, failing overall assessment, critical attribute, stale sample, permission-required refresh, and removable drive.

- [ ] **Step 2: Implement UDisks2 discovery and mapping**

Map object paths to stable drive identity, model, firmware, serial only in protected internal model, temperature, power-on hours, assessment, and selected critical attributes. Serial is never shown in report output unless user disables redaction.

- [ ] **Step 3: Keep health wording bounded by source evidence**

Use `Passed`, `Warning reported by drive`, `Failure reported by drive`, `Data stale`, and `Unavailable`. Never predict remaining drive life or label missing SMART as healthy.

- [ ] **Step 4: Add refresh and UI states**

Refresh is on demand because it may spin up disks or require permission. Show source, sample time, and precise unavailable reason.

- [ ] **Step 5: Run tests and optional commit**

```bash
cargo test -p maintainer-core diagnostics::smart diagnostics::udisks
```

```bash
git add Cargo.toml Cargo.lock crates/maintainer-core/src/diagnostics crates/maintainer-app/src/ui/diagnostics.rs fixtures/udisks
git commit -m "feat(diagnostics): add SMART health adapter"
```

---

### Task 13: Generate Redacted Local Reports

**Files:**
- Create: `crates/maintainer-core/src/reports/mod.rs`
- Create: `crates/maintainer-core/src/reports/model.rs`
- Create: `crates/maintainer-core/src/reports/redact.rs`
- Create: `crates/maintainer-core/src/reports/render_text.rs`
- Create: `crates/maintainer-app/src/ui/reports.rs`
- Test: golden report fixtures and redaction property tests

**Interfaces:**
- Consumes: latest metric snapshot, updates, diagnostics, SMART, history, OS metadata.
- Produces: `SystemReport`, `RedactionPolicy`, `RedactedReport`, and `TextReportRenderer`.

- [ ] **Step 1: Write failing redaction tests**

Fixtures include username, hostname, IPv4/IPv6, MAC, serial, home path, mounted path, process arguments, email-like text, and benign version numbers. Assert sensitive values are absent and structure remains useful.

- [ ] **Step 2: Implement structured redaction before rendering**

Do not rely only on regex over final text. Sensitive fields are typed and removed/replaced before rendering; a final conservative scan catches accidental embedded values. Default policy redacts every category named in the PRD.

- [ ] **Step 3: Assemble capability-aware report sections**

Unavailable sections state why. Report uses collected snapshots and does not trigger privileged or expensive operations without user initiation.

- [ ] **Step 4: Implement deterministic plain-text rendering**

Stable section order: OS, hardware, health, software updates, diagnostics, maintenance, capability notes, redaction notice. Golden tests fix line endings, units, and omission rules.

- [ ] **Step 5: Implement preview, copy, and atomic save**

Preview is mandatory. Save through GTK file chooser, write a temporary sibling file with mode `0600`, flush, then atomically rename. Maintainer provides no upload/share endpoint.

- [ ] **Step 6: Run report tests and optional commit**

```bash
cargo test -p maintainer-core reports
xvfb-run -a cargo test -p maintainer-app reports
```

```bash
git add crates/maintainer-core/src/reports crates/maintainer-app/src/ui/reports.rs fixtures
git commit -m "feat(reports): export redacted system snapshots"
```

---

### Task 14: Implement and Threat-Test the D-Bus/Polkit Helper

**Files:**
- Create: `crates/maintainer-helper/src/api.rs`
- Create: `crates/maintainer-helper/src/authorize.rs`
- Create: `crates/maintainer-helper/src/validate.rs`
- Create: `crates/maintainer-helper/src/operations/mod.rs`
- Create: `crates/maintainer-helper/src/operations/process.rs`
- Create: `crates/maintainer-helper/src/operations/package_cache.rs`
- Create: `crates/maintainer-helper/src/operations/journal.rs`
- Create: `crates/maintainer-helper/src/operations/smart.rs`
- Create: `data/dbus/io.github.maintainerapp.Helper.service`
- Create: `data/polkit/io.github.maintainerapp.policy`
- Create: `crates/maintainer-helper/tests/security.rs`
- Create: `docs/threat-model.md`

**Interfaces:**
- Consumes: D-Bus caller unique name, process credentials, Polkit authority, fixed operation DTOs.
- Produces: versioned `Helper1` D-Bus interface and unprivileged `HelperClient` capability adapter.

```rust
pub trait HelperApi {
    fn terminate_process(&self, request: TerminateProcessRequest) -> HelperResult;
    fn clean_package_cache(&self, request: PackageCacheRequest) -> HelperResult;
    fn vacuum_journal(&self, request: JournalVacuumRequest) -> HelperResult;
    fn refresh_smart(&self, request: SmartRefreshRequest) -> HelperResult;
}
```

- [ ] **Step 1: Write the threat model before implementation**

Document assets, trust boundaries, caller spoofing, PID reuse, path substitution, symlink races, argument injection, D-Bus message limits, authorization replay, confused deputy, partial execution, denial of service, and audit-data leakage. Map each threat to a test or explicit non-goal.

- [ ] **Step 2: Write failing authorization and validation tests**

Test unknown caller, caller disconnect, mismatched UID, denied/cancelled Polkit prompt, wrong action ID, stale authorization, PID reuse, forbidden signal, package-cache path injection, journal values outside bounds, unknown operation, oversized message, malformed DTO, and concurrent request limit.

- [ ] **Step 3: Define fixed D-Bus DTOs and versioning**

No method accepts program name, argument vector, shell text, arbitrary environment, arbitrary filesystem root, or arbitrary signal. DTOs include request ID and fields needed for revalidation only.

- [ ] **Step 4: Implement caller identity and Polkit authorization**

Resolve Unix process ID and UID from the D-Bus daemon for the unique sender, bind authorization to sender/action/request, and reject after disconnect. Use separate Polkit actions for process termination, package cache, journal maintenance, and SMART refresh.

- [ ] **Step 5: Implement fixed operations**

Revalidate process start time immediately before SIGTERM. Package cache uses fixed APT cache operation. Journal vacuum accepts only bounded size/age values. SMART refresh targets a validated UDisks object/device identity. Invoke any utility directly with fixed executable and explicit arguments.

- [ ] **Step 6: Add service hardening**

D-Bus activation launches the helper only on demand. Apply systemd sandboxing where compatible: no network, private temporary directory, restrictive filesystem access, capability bounding, no new privileges except required operation path, memory/CPU limits, and request concurrency cap. The helper exits after an idle timeout and is not a permanent daemon.

- [ ] **Step 7: Run security tests in denial and authorized test harnesses**

```bash
cargo test -p maintainer-helper security
cargo clippy -p maintainer-helper --all-targets -- -D warnings
```

Use a mock Polkit authority for unit tests and an isolated VM for real authorization smoke tests. Never run destructive helper tests against the developer's host data.

- [ ] **Step 8: Mandatory human review checkpoint**

Review `docs/threat-model.md`, D-Bus API, Polkit actions, systemd hardening, and every subprocess argument before connecting the helper to UI. This security boundary must not proceed on agent self-approval alone.

- [ ] **Step 9: Optional commit after review approval**

```bash
git add crates/maintainer-helper data/dbus data/polkit docs/threat-model.md Cargo.toml Cargo.lock
git commit -m "feat(helper): add constrained Polkit operations"
```

---

### Task 15: Implement APT Update Review and PackageKit Execution

**Files:**
- Create: `crates/maintainer-core/src/updates/mod.rs`
- Create: `crates/maintainer-core/src/updates/model.rs`
- Create: `crates/maintainer-core/src/updates/apt.rs`
- Create: `crates/maintainer-core/src/updates/packagekit.rs`
- Create: `crates/maintainer-core/src/updates/flatpak.rs`
- Create: `crates/maintainer-core/src/updates/fwupd.rs`
- Create: `crates/maintainer-app/src/ui/updates.rs`
- Create: `fixtures/apt/`
- Create: `fixtures/packagekit/`
- Test: parser, D-Bus adapter, transaction, and reducer tests

**Interfaces:**
- Consumes: `CommandRunner` for read-only APT simulation, PackageKit D-Bus for authorized transactions, optional Flatpak/fwupd D-Bus adapters.
- Produces: `UpdateProvider`, `UpdateCandidate`, `UpdateClassification`, `UpdatePlan`, `UpdateProgress`, and `UpdateResult`.

- [ ] **Step 1: Write update model and APT fixture tests**

Cover no updates, security updates, phased updates, held packages, removals, new dependencies, download size unavailable, localization forced to C, malformed output, dpkg lock, offline repository, cancellation before transaction, and partial PackageKit failure.

- [ ] **Step 2: Implement read-only APT discovery**

Use fixed `apt-get` simulation arguments with `LC_ALL=C` and no shell. Discovery must not mutate package state. Parse into typed candidates and preserve backend evidence when security classification is unavailable.

- [ ] **Step 3: Implement immutable update review plan**

Plan lists package, installed/available version, classification, download bytes, held/phased state, required removals, and backend. Any package-set change invalidates the reviewed plan and requires fresh review.

- [ ] **Step 4: Implement PackageKit transaction adapter**

PackageKit owns package authorization and transaction lifecycle where available. Map D-Bus status, percentage, package progress, restart requirement, cancellation support, errors, and final state. Never request or store passwords.

- [ ] **Step 5: Implement optional Flatpak and fwupd adapters**

Detect D-Bus/CLI capability independently. If unavailable, return reason/remediation. Do not make either backend a required dependency for core APT updates.

- [ ] **Step 6: Build update review and execution UI**

Explicit sequence: Check, Review, Confirm, Authenticate, Execute, Result. Security labels appear only when backend evidence supports them. Disable cancellation when backend reports cancellation unsafe.

- [ ] **Step 7: Test against mocks and disposable VMs**

```bash
cargo test -p maintainer-core updates
xvfb-run -a cargo test -p maintainer-app updates
```

VM matrix tests APT success, user denial, interrupted network, dpkg lock, restart-required result, and recovery by native package tools. Do not test package mutation on the host.

- [ ] **Step 8: Optional commit**

```bash
git add crates/maintainer-core/src/updates crates/maintainer-app/src/ui/updates.rs fixtures/apt fixtures/packagekit Cargo.toml Cargo.lock
git commit -m "feat(updates): review and install system updates"
```

---

### Task 16: Complete Settings, Privacy Controls, and History Views

**Files:**
- Create: `crates/maintainer-app/src/ui/settings.rs`
- Create: `crates/maintainer-app/src/ui/history.rs`
- Modify: `crates/maintainer-app/src/application.rs`
- Modify: `crates/maintainer-app/src/state.rs`
- Modify: `data/io.github.maintainerapp.Maintainer.gschema.xml`
- Test: settings reducer, persistence, privacy deletion, and restart tests

**Interfaces:**
- Consumes: `SettingsRepository`, `HistoryRepository`, monitoring/storage services.
- Produces: validated settings UI and user-controlled local-data deletion flow.

- [ ] **Step 1: Write settings validation tests**

Test 2-second default, accepted refresh range 1–60 seconds, hidden 10-second minimum, canonical scan roots, exclusion within selected roots, large-file threshold minimum, theme/system selection, notifications, invalid persisted value, and restart round trip.

- [ ] **Step 2: Implement grouped settings UI**

Groups: General, Monitoring, Storage, Maintenance, Privacy. Activity controls do not appear. Each setting has concise impact text and accessible label; invalid input cannot be saved.

- [ ] **Step 3: Connect settings without global mutable state**

Application service publishes immutable settings snapshots. Monitoring applies refresh changes at the next scheduling boundary; active scans retain their original policy snapshot.

- [ ] **Step 4: Implement history and clear-data flow**

Show operation, time, result, recovered bytes, and bounded error summary. Delete one or all history requires confirmation. “Delete all Maintainer data” closes the database, removes database and generated reports only when explicitly selected, recreates defaults, and never touches maintenance targets.

- [ ] **Step 5: Run tests and optional commit**

```bash
cargo test -p maintainer-core settings persistence
xvfb-run -a cargo test -p maintainer-app settings history
```

```bash
git add crates/maintainer-app/src data/io.github.maintainerapp.Maintainer.gschema.xml
git commit -m "feat(settings): add privacy and history controls"
```

---

### Task 17: Package Native `.deb` Artifacts and Desktop Integration

**Files:**
- Create: `data/io.github.maintainerapp.Maintainer.desktop`
- Create: `data/io.github.maintainerapp.Maintainer.metainfo.xml`
- Create: `data/io.github.maintainerapp.Maintainer.gschema.xml`
- Create: `data/icons/`
- Create: `packaging/debian/control`
- Create: `packaging/debian/rules`
- Create: `packaging/debian/changelog`
- Create: `packaging/debian/copyright`
- Create: `packaging/debian/install`
- Create: `packaging/debian/postinst`
- Create: `packaging/debian/postrm`
- Create: `scripts/test-package.sh`
- Modify: `.github/workflows/ci.yml`
- Modify: `README.md`

**Interfaces:**
- Consumes: release binaries, GResources, schema, icon, D-Bus service, Polkit policy.
- Produces: installable/removable `.deb` and repository-ready unsigned artifacts.

- [ ] **Step 1: Resolve public identity before packaging**

Check package name, desktop application ID, GitHub organization, and trademark availability. If development ID changes, update every reference in one reviewed change and add a test that searches installed metadata for stale IDs.

- [ ] **Step 2: Write package smoke test before package rules**

`scripts/test-package.sh` installs into a disposable Ubuntu container/VM, asserts binaries and metadata paths, validates desktop/metainfo/schema files, activates helper on demand, launches app under Xvfb, removes package, and asserts no binary/policy/service remains. User database and settings remain on ordinary uninstall.

- [ ] **Step 3: Define runtime dependencies and installation paths**

Depend on Ubuntu 22.04-compatible GTK4, GLib, SQLite, Polkit, PackageKit, UDisks2, systemd, and required runtime libraries. Optional Flatpak/fwupd capabilities remain suggested rather than hard dependencies unless required for safe adapter behavior.

- [ ] **Step 4: Add maintainer scripts with minimal side effects**

Compile schemas/update desktop caches only through standard debhelper mechanisms. Do not start a daemon, modify user files, add repositories, or fetch network content during install.

- [ ] **Step 5: Build and test on representative matrix**

At minimum: Ubuntu 22.04, Ubuntu 24.04, current Ubuntu release, current Linux Mint, and one additional current Ubuntu derivative. Test install, launch, basic metrics, unavailable states, report export, Polkit denial, and uninstall.

- [ ] **Step 6: Keep APT publishing separate from build**

CI may produce checksums, source package, `.deb`, and unsigned repository metadata. Signing keys and publication credentials stay in release infrastructure and are never read by ordinary development agents.

- [ ] **Step 7: Optional commit**

```bash
git add data packaging scripts/test-package.sh .github/workflows/ci.yml README.md
git commit -m "build: package Maintainer for Ubuntu-based systems"
```

---

### Task 18: Accessibility, Security, Performance, and v1 Release Hardening

**Files:**
- Create: `scripts/measure-performance.sh`
- Create: `tests/system/accessibility.md`
- Create: `tests/system/compatibility.md`
- Create: `tests/system/security.md`
- Create: `tests/system/release-smoke.sh`
- Modify: `PRD.md`
- Modify: `README.md`
- Modify: `AGENTS.md`
- Modify: `.github/workflows/ci.yml`

**Interfaces:**
- Consumes: complete v1 application and packaging.
- Produces: reproducible release evidence for every PRD release criterion.

- [ ] **Step 1: Define the exact reference system and measurement protocol**

Record CPU, RAM, storage, GPU/session, Ubuntu-based distribution, desktop, display protocol, kernel, build profile, enabled page, sample duration, and tool versions. Use release build, cold filesystem-cache procedure for cold start, 5-minute idle CPU/RSS samples, visible and hidden-window samples, and three repetitions.

- [ ] **Step 2: Automate performance measurements**

`scripts/measure-performance.sh` records startup wall time, steady RSS, 5-minute average CPU, wakeups when available, visible 2-second refresh, hidden refresh, and scan UI responsiveness. It exits non-zero for RSS above 100 MB, CPU above 0.5%, or cold start above 2 seconds; it reports the 70 MB RSS target separately.

- [ ] **Step 3: Profile and fix measured budget failures one cause at a time**

Check unbounded samples, excess widget realization, polling while hidden, duplicate collectors, retained scan results, logging, thread count, and SQLite connections. Add a regression benchmark or bounded-data test for each fixed cause. Never weaken a budget to hide a regression without product approval.

- [ ] **Step 4: Complete keyboard and accessibility review**

Test every page without pointer: navigation, sort, selection, scan start/cancel, dialogs, update review, report preview/save, settings, and history deletion. Verify visible focus, accessible names/descriptions, 200% text, light/dark contrast, status not color-only, and textual chart equivalents.

- [ ] **Step 5: Complete security review**

Re-run helper authorization/validation/denial tests, path/PID race tests, report redaction properties, command allowlist tests, package VM tests, dependency audit, and secret scan. Confirm GUI refuses root execution and no telemetry/network endpoint exists beyond explicit update backends.

- [ ] **Step 6: Complete failure-state matrix**

For each feature verify loading, unavailable, permission-required, empty, success, partial-failure, failure, and cancelled states where applicable. Include missing systemd, no sensors, no SMART, no PackageKit, offline network, locked dpkg, unreadable paths, removed devices, and database corruption recovery.

- [ ] **Step 7: Run final release commands**

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
cargo build --workspace --release
scripts/measure-performance.sh target/release/maintainer-app
scripts/test-package.sh
```

Expected: all commands exit `0`; performance output meets hard budgets; package matrix evidence is linked from `tests/system/compatibility.md`.

- [ ] **Step 8: Reconcile documents with shipped behavior**

Mark only implemented PRD stories complete, replace specification-stage README instructions with verified setup/package commands, and keep deferred activity tracking explicitly out of v1. Document any optional capability limitations found in the compatibility matrix.

- [ ] **Step 9: Final review checkpoint and optional commit**

Do not call v1 complete until every PRD release criterion has concrete evidence. If commits are authorized:

```bash
git add scripts tests/system PRD.md README.md AGENTS.md .github/workflows/ci.yml
git commit -m "chore: harden Maintainer v1 release"
```

---

## Phase Gates and Execution Order

1. Tasks 1–5 produce a working Overview/System vertical slice and early performance baseline.
2. Task 6 provides durable settings/history required by later maintenance flows.
3. Tasks 7–9 produce the complete storage and duplicate vertical slice.
4. Task 10 produces safe cleanup and operation history.
5. Tasks 11–13 produce diagnostics, SMART, and redacted reports.
6. Task 14 requires a human security-boundary review before Task 15 connects privileged update/maintenance behavior.
7. Tasks 15–16 complete update execution, settings, privacy, and history UX.
8. Tasks 17–18 package, measure, audit, and validate v1 across supported environments.

Parallel work is allowed only after interfaces are merged and stable:

- Task 6 may proceed alongside Tasks 4–5 after Task 2.
- Task 8 may proceed alongside Task 9 UI state scaffolding after Task 7 models are fixed.
- Tasks 11 and 12 may proceed in parallel after Task 2.
- Task 13 may begin once report input models from Tasks 4, 6, 11, and 12 are stable.
- Task 14 may proceed alongside unprivileged feature work, but integration waits for human review.

## Traceability to PRD Stories

| PRD story | Implemented by | Release evidence |
|---|---|---|
| US-01 Overview | Tasks 3–5 | monitor state tests, accessibility review, performance run |
| US-02 System/processes | Tasks 4–5, 14 | metric fixtures, PID-race tests, process UI tests |
| US-03 Storage/large files | Tasks 7, 9 | temporary-tree tests, cancellation tests, large-list smoke |
| US-04 Duplicates | Tasks 8–10 | staged-hash tests, empty default selection, reviewed removal |
| US-05 Cleanup | Tasks 10, 14 | allowlist/race tests, Polkit denial, per-item results/history |
| US-06 Updates | Tasks 14–15 | parser/mocks, Polkit denial, disposable-VM transaction tests |
| US-07 Services/journal | Task 11 | D-Bus/JSON fixtures, grouping and interpretation tests |
| US-08 SMART | Tasks 12, 14 | UDisks fixtures, unavailable/permission states |
| US-09 Reports | Task 13 | golden output, redaction properties, local save test |
| US-10 Capability explanations | Tasks 2–5 and each adapter task | state matrix in Task 18 |
| US-11 Process termination | Tasks 5, 14 | identity revalidation and authorization tests |
| US-12 Local-data control | Tasks 6, 16 | repository deletion and restart tests |

## Plan Self-Review Checklist

Before execution begins, confirm:

- Every v1 page and PRD story maps to at least one task and test artifact.
- Activity/screen-time tracking appears only as deferred scope.
- No task adds libadwaita, browser technology, Node, ESLint, or Prettier.
- UI never reads `/proc`, `/sys`, SQLite, D-Bus backends, or commands directly.
- Helper methods accept no command string, executable, argument vector, arbitrary path root, or unrestricted signal.
- Every scan/update/cleanup flow defines cancellation, denial, partial failure, and stale-result handling.
- Type names in consuming tasks match the producing task interfaces.
- Ubuntu 22.04 and GTK 4.6 remain the compatibility floor.
- Final measurements distinguish the 70 MB target from the 100 MB hard ceiling.
- No repository commit occurs without explicit user authorization.
