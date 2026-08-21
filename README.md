# Maintainer

<div align="center">

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![License](https://img.shields.io/github/license/alxxrzfyr/maintainer-app)](LICENSE)

</div>

**Your Linux computer's personal maintenance center.**

Maintainer is a modern, lightweight native desktop application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux computers. It brings system monitoring, storage analysis, safe cleanup, update review, and diagnostics into one calm place — without becoming a fake optimizer, a full administration suite, or a cloud service.

## Status

Approaching v1 release. All three crates (`maintainer-app`, `maintainer-core`, `maintainer-helper`) build, pass tests, and produce release binaries. CI, packaging, and system tests are in place. Phase 6 (v1 hardening) is in progress: accessibility review, compatibility testing, performance measurement, and security verification.

Foreground application and screen-time tracking are explicitly deferred from v1.

## What Maintainer does

- Understand CPU, memory, storage, temperature, network, and process activity at a glance
- Find large and duplicate files
- Review and perform safe, previewed cleanup with operation history
- Inspect and execute available APT updates, plus optional Flatpak and firmware updates where supported
- Diagnose failed services and recurring journal events
- Inspect SMART storage health where supported
- Generate a redacted, privacy-conscious local system report

Maintainer does not promise fake optimization, automatic duplicate deletion, cloud monitoring, or arbitrary system tweaking. It never runs as root, has no permanent background daemon, and uploads no system data.

## Tech stack

| Layer | Choice |
|---|---|
| Language | Rust (stable toolchain, pinned via `rust-toolchain.toml`) |
| UI | GTK4 only — no libadwaita, WebView, Electron, or embedded browser |
| Styling | GTK CSS with semantic tokens (see `DESIGN.md`) |
| Persistent data | SQLite — settings and application history only |
| System interfaces | `/proc`, `/sys`, D-Bus, systemd journal, APT, SMART/UDisks2 |
| Privileged operations | A narrowly scoped D-Bus/Polkit helper (`maintainer-helper`); the GUI always runs unprivileged |
| Build tooling | Cargo (workspace with `maintainer-app`, `maintainer-core`, `maintainer-helper`) |
| Packaging | Native `.deb` package distributed through an APT repository |

## Compatible systems

Maintainer targets **current Ubuntu-based distributions**, with **Ubuntu 22.04 LTS as the compatibility floor**. Support is capability-driven: if an integration (SMART, Flatpak, firmware, sensors, journal, etc.) is unavailable on a given system, Maintainer explains why instead of failing or showing misleading data.

Runtime dependencies (installed automatically by the `.deb`):
- GTK4 ≥ 4.6, GLib ≥ 2.72, SQLite3, PolicyKit, PackageKit, UDisks2, systemd

Optional, suggested:
- `flatpak` — enables Flatpak update inspection
- `fwupd` — enables firmware update inspection

Non-Ubuntu distribution families are not first-class targets in v1.

## Performance contract

On the defined low-end reference system:

- Target idle RSS: 70 MB or less where feasible (100 MB hard ceiling)
- Average idle CPU: 0.5% or less
- Cold start: 2 seconds or less
- Default monitoring refresh: 2 seconds (visible), backs off to 10 seconds when the window is hidden
- Expensive scans (storage, duplicates, SMART, journal): on demand, asynchronous, progress-reporting, and cancellable

## Setup

### Prerequisites

- `libgtk-4-dev`, `libsqlite3-dev`, `libpolkit-gobject-1-dev`, `pkg-config`
- Rust toolchain 1.85.0 — installed automatically by `rustup` from `rust-toolchain.toml`

```bash
sudo apt install libgtk-4-dev libsqlite3-dev libpolkit-gobject-1-dev pkg-config
```

### Build from source

```bash
# Build all workspace members (debug)
cargo build --workspace

# Build all workspace members (release)
cargo build --workspace --release

# Run the GUI application (dev mode)
cargo run -p maintainer-app

# Run the privileged helper (dev mode)
cargo run -p maintainer-helper
```

### Verify

```bash
# Format check
cargo fmt --all -- --check

# Lint
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Test all workspace members
cargo test --workspace --all-features

# Test a single target
cargo test -p maintainer-core module::tests::test_name -- --exact
```

### Install from .deb package

Download the latest `.deb` from the [releases page](https://github.com/maintainerapp/maintainer/releases):

```bash
sudo apt install ./maintainer_1.0.0-1_amd64.deb
```

This installs:
- `/usr/bin/maintainer` — the GUI application (runs unprivileged)
- `/usr/libexec/maintainer-helper` — the Polkit-gated privileged helper
- Desktop entry, icons, GSettings schema, D-Bus service, and Polkit policy

### Uninstall

```bash
# Remove binaries and data; user settings preserved
sudo apt remove maintainer

# Remove everything including user databases
sudo apt purge maintainer
```

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

Three boundaries are maintained strictly: `ui` (GTK widgets, never touches Linux interfaces directly), `core` (typed services, capability adapters, collectors, jobs, persistence), and `helper` (narrowly scoped privileged operations with validated callers and parameters — never arbitrary command execution).

## License

Maintainer is licensed under the [MIT License](LICENSE).
