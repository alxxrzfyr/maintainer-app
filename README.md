# Maintainer

Maintainer is a modern, lightweight native desktop application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux computers.

**Modern interface. Native performance. Local-first system maintenance.**

## Status

Maintainer is in the specification stage. Product scope and design are approved; application source and build tooling have not been scaffolded yet.

## Product direction

Maintainer provides one calm place to:

- understand CPU, memory, storage, temperature, network, and process activity;
- find large and duplicate files;
- review and perform safe cleanup;
- inspect available system, Flatpak, and firmware updates where supported;
- diagnose failed services and recurring journal events;
- inspect SMART storage health where supported; and
- generate a useful, privacy-conscious local system report.

It does not promise fake optimization, automatic duplicate deletion, cloud monitoring, or arbitrary system tweaking.

## v1 scope

Version 1 includes:

- Overview and live system monitoring
- Storage analysis, large-file scanning, and duplicate detection
- Process inspection and confirmed termination
- Previewed cleanup with operation history
- APT update review and execution, plus optional Flatpak and firmware adapters
- Failed-service and grouped-log diagnostics
- SMART storage health
- Redacted local reports
- Simple settings and capability explanations

Foreground application and screen-time tracking are deferred until reliable, privacy-preserving behavior can be provided across supported desktop environments.

## Technology

- Rust
- GTK4 without libadwaita or embedded web technology
- SQLite for settings and persistent application history
- Linux interfaces including `/proc`, `/sys`, D-Bus, systemd journal, APT, and SMART capabilities
- A narrowly scoped D-Bus/Polkit helper for privileged operations
- Native `.deb` packages distributed through an APT repository

The GUI always runs unprivileged. Maintainer has no permanent daemon and uploads no system data.

## Supported systems

The product targets current Ubuntu-based distributions with Ubuntu 22.04 as the compatibility floor. Features are capability-driven: unavailable integrations must be explained clearly instead of failing or showing misleading data.

## Performance contract

On the defined low-end reference system:

- Target idle RSS: 70 MB or less where feasible
- Hard idle RSS ceiling: 100 MB
- Average idle CPU: 0.5% or less
- Cold start: 2 seconds or less
- Default monitoring refresh: 2 seconds
- Expensive scans: on demand, asynchronous, progress-reporting, and cancellable

## Architecture

```text
GTK4 view
  |
Application service
  |
Capability adapter or cancellable job
  |
Linux API / D-Bus / validated privileged helper
  |
Typed result and incremental view update
```

Live measurements are not stored unnecessarily. SQLite is reserved for settings, maintenance history, scan metadata, diagnostic history, and other durable application state.

## Project documents

- [`PRD.md`](PRD.md): product scope, requirements, stories, delivery phases, and success criteria
- [`DESIGN.md`](DESIGN.md): visual system, layouts, interaction rules, and accessibility requirements
- [`AGENTS.md`](AGENTS.md): architecture boundaries and repository guidance for coding agents
- [`CLAUDE.md`](CLAUDE.md): Claude Code-specific guidance
- [`GEMINI.md`](GEMINI.md): Antigravity CLI-specific guidance

## Development

### Prerequisites

- `libgtk-4-dev`, `libsqlite3-dev`, `libpolkit-gobject-1-dev`, `pkg-config`
- The pinned Rust toolchain (1.85.0) is installed automatically by rustup from `rust-toolchain.toml`.

### Commands

```bash
# Build all workspace members (debug)
cargo build --workspace

# Build all workspace members (release)
cargo build --workspace --release

# Run the GUI application (dev mode)
cargo run -p maintainer-app

# Run the privileged helper (dev mode)
cargo run -p maintainer-helper

# Format check (must pass before commit)
cargo fmt --all -- --check

# Lint (must pass before commit)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Test all workspace members
cargo test --workspace --all-features

# Test a single target
cargo test -p maintainer-core module::tests::test_name -- --exact
```
