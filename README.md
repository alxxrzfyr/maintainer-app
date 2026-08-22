# Maintainer

<div align="center">

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![License](https://img.shields.io/github/license/alxxrzfyr/maintainer-app)](LICENSE)

</div>

Maintainer is a lightweight native desktop application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux systems. Built with Rust and GTK4, it provides system observability, storage analysis, safe previewed cleanup, update management, and diagnostics without background daemons, telemetry, or opaque optimizations.

## Contents

- [Installation](#installation)
  - [Debian Package (.deb)](#debian-package-deb)
  - [Building from Source](#building-from-source)
- [Key Features](#key-features)
- [Architecture and Security](#architecture-and-security)
- [Platform Requirements](#platform-requirements)
- [Performance Specifications](#performance-specifications)
- [Project Documentation](#project-documentation)
- [License](#license)

---

## Installation

### Debian Package (.deb)

Pre-built packages are available on the [Releases](https://github.com/alxxrzfyr/maintainer-app/releases) page for `amd64` and `arm64` architectures.

```bash
# Install the package and its runtime dependencies
sudo apt install ./maintainer_1.0.0-1_amd64.deb
```

Installed components:
- `/usr/bin/maintainer`: Unprivileged GTK4 graphical interface
- `/usr/libexec/maintainer-helper`: PolicyKit-authenticated system helper
- Desktop launcher, icons, GSettings schemas, D-Bus service, and PolicyKit policy definitions

To remove:

```bash
# Remove binaries while preserving user configuration
sudo apt remove maintainer

# Remove all files and configurations
sudo apt purge maintainer
```

### Building from Source

#### Prerequisites

Install required development headers:

```bash
sudo apt update
sudo apt install -y libgtk-4-dev libsqlite3-dev libpolkit-gobject-1-dev pkg-config
```

The Rust toolchain (1.85.0) is pinned in `rust-toolchain.toml` and will be installed automatically by `rustup`.

#### Build and Run

```bash
# Compile release binaries
cargo build --workspace --release

# Run the GTK4 application (development mode)
cargo run -p maintainer-app

# Run the privileged helper standalone (development testing)
cargo run -p maintainer-helper
```

#### Verification

```bash
# Run unit and integration tests
cargo test --workspace --all-features

# Check formatting and lints
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Execute performance benchmark
scripts/measure-performance.sh target/release/maintainer-app
```

---

## Key Features

- **System Monitoring:** Real-time tracking of CPU, memory, storage, temperatures, network, and processes with historical sparklines. Adaptive polling (2s active, 10s when hidden).
- **Storage Analysis and Cleanup:** Asynchronous scanning for large files and content-hash verified duplicate groups. Mandatory preview, local audit history, and Trash integration.
- **Update Management:** Review and apply APT package updates, with optional inspection of Flatpak runtimes and fwupd device firmware.
- **Diagnostics and Health:** Inspect failed systemd services, recurring journal logs, and SMART disk telemetry via UDisks2.
- **Redacted Reports:** Generate local diagnostic reports with automatic redaction of hostnames, usernames, IP addresses, serial numbers, and process arguments.

---

## Architecture and Security

Maintainer enforces strict privilege separation between the UI and system interactions:

```text
GTK4 User Interface (maintainer-app)
        │
Application Services and State Management
        │
Capability Adapters and Cancellable Jobs (maintainer-core)
        │
System Interfaces: /proc, /sys, D-Bus, Polkit Helper
        │
Privileged Operations Daemon (maintainer-helper)
```

- **`maintainer-app`:** Unprivileged GTK4 presentation layer. Contains no direct system or root access.
- **`maintainer-core`:** Typed domain models, persistence (SQLite), collectors, and system adapters.
- **`maintainer-helper`:** PolicyKit-activated D-Bus helper executing only fixed operations with allowlisted parameters. Arbitrary shell or command execution is prohibited.

---

## Platform Requirements

- **Supported Distributions:** Ubuntu 22.04 LTS (Jammy Jellyfish) and newer (including Ubuntu derivatives).
- **Display Servers:** Wayland and X11 native support via GTK4.
- **Runtime Dependencies:** GTK4 (>= 4.6), GLib (>= 2.72), SQLite3, PolicyKit, PackageKit, UDisks2, systemd.
- **Optional Packages:** `flatpak` (Flatpak package review), `fwupd` (firmware update review).

---

## Performance Specifications

| Metric | Target | Hard Ceiling |
|---|---|---|
| Idle Memory (PSS) | <= 150 MB | 220 MB |
| Private / Dirty Idle Memory | <= 130 MB | 150 MB |
| Idle CPU (hardware accelerated) | <= 0.5% | 1.0% |
| Cold Start Time | <= 2.0s | 2.5s |
| UI Responsiveness | Sub-frame event handling | Zero main-thread blocking operations |

---

## Project Documentation

- [CHANGELOG.md](CHANGELOG.md): Release history and security fixes.
- [AGENTS.md](AGENTS.md): Architecture rules and development guidelines.
- [docs/threat-model.md](docs/threat-model.md): Threat analysis and helper security model.
- [tests/system/](tests/system/): Manual accessibility, compatibility, and security test plans.

---

## License

Maintainer is distributed under the terms of the [MIT License](LICENSE).

