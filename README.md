<div align="center">

<img src="data/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg" alt="Maintainer Logo" width="96" height="96">

# Maintainer

**Fast, lightweight, native GTK4 GUI — the product goals for Maintainer.**

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%2B-E95420?logo=ubuntu&logoColor=white)](https://ubuntu.com)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

</div>

Maintainer is a native GTK4 application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux systems. Built in Rust, it brings local monitoring and reviewed maintenance workflows into one desktop interface. Resource budgets and remaining safety work are documented below.

Maintainer starts no permanent background daemon, collects no telemetry, and performs maintenance through explicit user actions.

---

## Installation

### Debian Package (.deb)

Maintainer provides official `.deb` packages for `amd64` and `arm64` architectures on the [Releases](https://github.com/alxxrzfyr/maintainer-app/releases) page.

```bash
# Install the package and all required dependencies
sudo apt install ./maintainer_1.2.0-1_amd64.deb
```

Once installed, launch Maintainer from your application launcher or run:

```bash
maintainer
```

To remove the package:

```bash
# Remove package binaries and preserve user configuration
sudo apt remove maintainer

# Remove package and system configuration; user-home data remains
sudo apt purge maintainer
```

---

## Highlights

| Capability | Description |
|---|---|
| **Live Monitoring** | CPU, memory, storage, sensors, network I/O and processes with in-memory sparklines. Default 2s visible and 10s hidden sampling. |
| **Storage Inspection** | On-demand large-file scans. Duplicate discovery has known app-to-scanner defects tracked in roadmap D12; current empty results are not reliable proof of no duplicates. |
| **Cleanup** | Operation preview, local history and FreeDesktop Trash support. Filesystem identity, partial-result and reporting gaps are tracked in D7. |
| **Update Management** | Unified inspection and execution of APT upgrades, with optional review for Flatpak packages and fwupd device firmware. |
| **Diagnostics & Health** | Analysis of failed systemd units, aggregated journal logs, and SMART disk telemetry via UDisks2. |
| **Diagnostic Reports** | Redacted preview, clipboard and file export. Free-text redaction and private-storage gaps are tracked in D13; preview remains necessary before sharing. |

---

## Architecture & Security

Maintainer separates desktop presentation, core services, and privileged
operations. The intended boundary and its current limitations are:

| Component | Current behavior | Work still required |
|---|---|---|
| Desktop app | Intended to run as the normal user; performs user-owned cleanup and report saving | Enforce root refusal and remove remaining blocking system/database work from GTK callbacks |
| Core | Typed collectors, adapters, jobs, settings/history and redaction | Enforce a shared job limit, bounded command output and safe reviewed filesystem operations |
| D-Bus maintenance helper | Validates fixed operation parameters and checks each operation through Polkit | Close process-identity races, test installed sandbox behavior and correct kernel-removal restrictions |
| Authenticated update CLI | pkexec launches the fixed helper, which validates arguments and replaces itself with APT | Bind execution to the reviewed plan and verify privileged operation ownership, cancellation and recovery |

The shipped policy avoids retained authorization for irreversible operations.
Cache cleaning and SMART refresh are allowed for an active local session;
other action/session combinations require administrator authorization by default.
Host rules and the desktop authentication agent affect actual prompts.

Helper operations use fixed executables and explicit argument arrays. This
reduces command-injection risk; it does not establish atomic transactions,
race-free file operations or a completed security audit. The update CLI runs
outside the maintenance helper's systemd sandbox.

See the [threat model](docs/threat-model.md) for implemented controls, open risks
and evidence requirements. The [documentation index](docs/README.md) links the
development roadmap and supporting research. Planned protections are not
shipped guarantees.

---

## Platform Requirements

- **Target Operating System:** Ubuntu 22.04 LTS (Jammy Jellyfish) or newer, and compatible Ubuntu-based distributions (Linux Mint, Pop!_OS).
- **Display Server:** Native Wayland and X11 support via GTK4.
- **Runtime Libraries and Services:** GTK4 (>= 4.6), GLib (>= 2.72), SQLite3, D-Bus, PolicyKit, systemd; APT/dpkg for Ubuntu package operations.
- **Recommended Services:** PackageKit and UDisks2. Missing integrations must produce an explained unavailable state.
- **Optional Tools:** `flatpak` and `fwupd` for update inspection, `curl` for the confirmed speed test, and `iw` for detailed Wi-Fi information.

---

## Performance Budgets

These are development goals and existing guardrails, not benchmark results.
The next milestone is below 100 MB of combined idle PSS for all Maintainer-owned
processes, preserving the native GUI, startup, CPU, safety and accessibility
requirements. Current compliance is unverified. Passing the legacy 220 MB
ceiling alone does not complete the new milestone.

| Metric | Target | Hard Ceiling | Notes |
|---|---|---|---|
| **Idle Memory (PSS)** | < 100 MB combined | 220 MB legacy ceiling | New milestone requires <100,000,000 bytes; replaces the former 150 MB goal |
| **Private Dirty Memory** | <= 130 MB | 130 MB | Separate from RSS and PSS |
| **Idle CPU Utilization** | <= 0.5% | 1.0% | Hardware rendering; software-rendered ceiling is 2.0% |
| **Cold Start Time** | <= 2.0s | 2.0s | Measure to a usable shell; record map and first live data separately |
| **UI Responsiveness** | Smooth interaction, 60 FPS aspiration | Non-blocking | No continuous idle repaint requirement; measure main-loop delay |

Maintainer's PSS increment over a matched minimal GTK baseline has a 40 MB
target; that comparison is a diagnostic delta, not exact memory attribution.
Software-rendered memory remains separately reported under the existing policy;
it cannot certify a universal under-100-MB claim. The new goal uses decimal MB;
retain raw bytes/MiB and resolve older budget-unit ambiguity in D0.
See the [roadmap](docs/plans/development-roadmap.md) and
[memory investigation](docs/research/2026-10-30-memory-under-100mb.md) for the
feasibility tests, whole-app accounting and lower exploratory checkpoints.

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
