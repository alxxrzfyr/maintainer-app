<div align="center">

<img src="data/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg" alt="Maintainer Logo" width="96" height="96">

# Maintainer

**Fast, lightweight, native system monitor and maintenance center for Linux.**

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%2B-E95420?logo=ubuntu&logoColor=white)](https://ubuntu.com)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

</div>

Maintainer is a native GTK4 application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux systems. Built in Rust, Maintainer pairs low resource usage with strict privilege separation to provide deep system visibility and safe maintenance workflows.

Maintainer contains no background daemons, collects no telemetry, and executes zero opaque optimizations.

---

## Installation

### Debian Package (.deb)

Maintainer provides official `.deb` packages for `amd64` and `arm64` architectures on the [Releases](https://github.com/alxxrzfyr/maintainer-app/releases) page.

```bash
# Install the package and all required dependencies
sudo apt install ./maintainer_1.0.0-1_amd64.deb
```

Once installed, launch Maintainer from your application launcher or run:

```bash
maintainer
```

To remove the package:

```bash
# Remove package binaries and preserve user configuration
sudo apt remove maintainer

# Remove package and purge all associated configuration
sudo apt purge maintainer
```

---

## Highlights

| Capability | Description |
|---|---|
| **System Telemetry** | Real-time tracking of CPU, memory, storage, sensors, network I/O, and processes with historical sparklines. Adaptive 2s active and 10s idle refresh rates. |
| **Storage Inspection** | Asynchronous scanning for large files and content-hash verified duplicate groups with zero unsolicited scans. |
| **Safe Cleanup** | Mandatory operation preview, structured SQLite audit history, and FreeDesktop Trash integration before any permanent deletion. |
| **Update Management** | Unified inspection and execution of APT upgrades, with optional review for Flatpak packages and fwupd device firmware. |
| **Diagnostics & Health** | Analysis of failed systemd units, aggregated journal logs, and SMART disk telemetry via UDisks2. |
| **Privacy-First Reports** | One-click diagnostic reports with automatic redaction of usernames, hostnames, IP addresses, serial numbers, and process arguments. |

---

## Architecture & Security

Maintainer separates presentation from privileged system management. The
boundary is structural: the interface has no code path to root, so a defect in
it cannot escalate.

```text
┌────────────────────────────────────────────────────────────────────┐
│                           maintainer-app                           │
├────────────────────────────────────────────────────────────────────┤
│  GTK4 windows, widgets and presentation state.                     │
│  Runs as the logged-in user. Holds no privilege, reads no          │
│  system interface directly, executes no commands.                  │
└────────────────────────────────────────────────────────────────────┘
                                  │                                   
                         typed service calls                          
                                  ▼                                   
┌────────────────────────────────────────────────────────────────────┐
│                          maintainer-core                           │
├────────────────────────────────────────────────────────────────────┤
│  Collectors for /proc, /sys and D-Bus. Capability adapters,        │
│  cancellable jobs, SQLite settings and operation history.          │
│  Reports each capability as available, unavailable with a          │
│  reason, or permission required.                                   │
└────────────────────────────────────────────────────────────────────┘
                                  │                                   
                     D-Bus, authorized by Polkit                      
                                  ▼                                   
┌────────────────────────────────────────────────────────────────────┐
│                         maintainer-helper                          │
├────────────────────────────────────────────────────────────────────┤
│  The only component that runs as root. D-Bus activated.            │
│  Fixed operation types with allowlisted, validated                 │
│  parameters. Arbitrary command execution is impossible by          │
│  construction, not by policy.                                      │
└────────────────────────────────────────────────────────────────────┘
```

| Boundary | Guarantee |
|---|---|
| Unprivileged interface | The GUI process never runs as root, never stores administrator credentials, and cannot modify system files |
| Audited helper | Privileged work is delegated to `maintainer-helper`, which accepts fixed operation types with validated parameters and is gated by PolicyKit authentication |
| No shell interpretation | External programs are invoked with explicit argument arrays, never assembled into shell strings, so command injection has no surface |
| Honest capabilities | A missing service or unsupported device yields an explicit unavailable state with a reason, never a fabricated value |

Threat analysis and the full helper API surface are documented in
[`docs/threat-model.md`](docs/threat-model.md).
---

## Platform Requirements

- **Target Operating System:** Ubuntu 22.04 LTS (Jammy Jellyfish) or newer, and compatible Ubuntu-based distributions (Linux Mint, Pop!_OS).
- **Display Server:** Native Wayland and X11 support via GTK4.
- **System Libraries:** GTK4 (>= 4.6), GLib (>= 2.72), SQLite3, PolicyKit, PackageKit, UDisks2, systemd.
- **Optional Integrations:** `flatpak` (Flatpak application updates), `fwupd` (firmware updates).

---

## Performance Budgets

Engineered for resource-constrained hardware and responsive desktop performance:

| Metric | Target | Hard Ceiling | Notes |
|---|---|---|---|
| **Idle Memory (PSS)** | <= 150 MB | 220 MB | Baseline dominated by GTK4 runtime |
| **Private Dirty Memory** | <= 130 MB | 150 MB | Under 4% of total RAM on a 4 GB system |
| **Idle CPU Utilization** | <= 0.5% | 1.0% | Measured on hardware-accelerated sessions |
| **Cold Start Time** | <= 2.0s | 2.5s | Measured from process exec to initial window map |
| **UI Responsiveness** | 60 FPS | Non-blocking | Heavy scans execute in background workers |

---

## Building from Source

### Prerequisites

Install the required build dependencies and development headers:

```bash
sudo apt update
sudo apt install -y libgtk-4-dev libsqlite3-dev libpolkit-gobject-1-dev pkg-config
```

Maintainer requires Rust 1.85.0 or newer. When using `rustup`, the pinned toolchain from `rust-toolchain.toml` is used automatically.

### Compilation

```bash
# Build optimized release binaries for the workspace
cargo build --workspace --release

# Run the GUI application in development mode
cargo run -p maintainer-app

# Run the privileged helper in development mode
cargo run -p maintainer-helper
```

### Verification & Quality Assurance

All commits must satisfy formatting, linting, and test suites:

```bash
# Run unit and integration tests
cargo test --workspace --all-features

# Verify formatting
cargo fmt --all -- --check

# Static analysis and lints
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Validate against performance budgets
scripts/measure-performance.sh target/release/maintainer-app
```

---

## Troubleshooting

### PolicyKit Authentication and D-Bus Activation

Privileged actions (such as package updates and process termination) require an active PolicyKit authentication agent in your desktop session.

If privileged actions fail with authorization errors:
1. Verify that a Polkit agent is running in your session (standard in GNOME, KDE, and XFCE; standalone window managers may require launching `lxqt-policykit` or `polkit-gnome-authentication-agent-1`).
2. Verify that the D-Bus system bus is active:
   ```bash
   systemctl is-active dbus
   ```

### Graphics and Rendering Fallbacks

Maintainer leverages GTK4 GPU acceleration by default. If running inside a virtual machine or on systems with proprietary driver rendering issues:

- Force the Cairo software renderer:
  ```bash
  GSK_RENDERER=cairo maintainer
  ```
- Force the X11 backend on Wayland composite sessions:
  ```bash
  GDK_BACKEND=x11 maintainer
  ```

### Storage SMART Telemetry or Hardware Sensors Unavailable

Maintainer presents explicit capability states rather than default or zero values when system metrics cannot be retrieved:

- **SMART Health:** Requires `udisks2` to be running (`systemctl status udisks2`). Virtual disks, RAID controllers, and certain USB-to-SATA bridge enclosures do not expose standard SMART passthrough attributes.
- **Thermal Sensors:** Hardware temperature monitoring reads from the Linux `hwmon` subsystem (`/sys/class/hwmon`). Virtualized environments lacking virtual temperature sensors will report sensors as unavailable.

### GSettings Schema Missing During Local Development

When executing `cargo run -p maintainer-app` from source without installing the system package, GLib may require access to the local GSettings schema:

```bash
glib-compile-schemas data/
GSETTINGS_SCHEMA_DIR=data cargo run -p maintainer-app
```

---

## License

Maintainer is open-source software licensed under the [MIT License](LICENSE).
