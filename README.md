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

Published `.deb` packages for `amd64` and `arm64` are available on the [Releases](https://github.com/alxxrzfyr/maintainer-app/releases) page. Version 1.3.0 is an unpublished development candidate; see [release status](docs/release-status.md) before treating it as release-ready.

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
| **Live Monitoring** | CPU, memory, storage, sensors, network I/O and processes with in-memory sparklines. Default 2s visible and 10s hidden sampling; sensor descriptors are retained while values stay fresh. Unchanged labels and storage styles are skipped. |
| **Storage Inspection** | On-demand large-file scans. Bounded duplicate discovery uses real file identities independently of the large-file shortlist; incomplete coverage is labelled. |
| **Cleanup** | Bounded regular-file review, one administrator approval per batch, protected quarantine, Trash and explicit permanent deletion. Recovery preserves raced or colliding entries; folders stay intact. Review supports cancellation, and Trash follows XDG data-home settings. |
| **Update Management** | APT inspection and native locked-plan installation/removal with explicit final review, cancellation boundaries, durable receipts and Resume. Approved package mutation blocks shutdown/sleep through postcondition verification; administrator overrides and power loss remain possible. Held, essential, running-kernel and kernel-tracking packages are protected. Flatpak/firmware inspection remains optional. |
| **Diagnostics & Health** | Analysis of failed systemd units, aggregated journal logs, and SMART disk telemetry via UDisks2. |
| **Diagnostic Reports** | Redacted preview, clipboard and file export. Default exports omit unrestricted diagnostic descriptions, journal text and capability failure details; exports create a new private file without overwriting an existing name; unsupported filesystems report failure. Preview remains necessary before sharing. |

---

## Architecture & Security

Maintainer separates desktop presentation, core services, and privileged
operations. The intended boundary and its current limitations are:

| Component | Current behavior | Work still required |
|---|---|---|
| Desktop app | Refuses root launch, keeps blocking work off GTK, and reviews exact authoritative package changes before Apply | Local checks establish this machine's behavior; other platforms remain unverified |
| Core | Bounded jobs/commands/scans, descriptor-pinned file reviews, private SQLite backup/recovery and redaction | Cleanup reviews cap at 256 files and 4096 visited paths; partial coverage stays explicit |
| Maintenance helper | SIGTERM through retained pidfd; fixed cache/journal/SMART operations; reviewed file batches and recovery | File mutation supports caller-owned single-link regular files on the quarantine filesystem, ext4/xfs/btrfs/tmpfs; no content snapshot or physical-space guarantee |
| Package owner | Separate short-lived service, retained native APT cache/locks, authenticated scoped control and private durable receipts | No automatic replay/rollback; interrupted work is audited read-only after external repair. Maintainer self-updates use the normal system package manager |

The shipped policy avoids retained authorization for irreversible operations.
Cache cleaning and SMART refresh are allowed for an active local session;
other action/session combinations require administrator authorization by default.
Host rules and the desktop authentication agent affect actual prompts.

Helper operations use fixed executables and explicit argument arrays. This
reduces command-injection risk; it does not establish atomic transactions,
race-free file operations or a completed security audit. Legacy unbound CLI execution remains refused; package operations use the dedicated owner.

See the [threat model](docs/threat-model.md) for implemented controls, open risks
and evidence requirements. The [documentation index](docs/README.md) links the
current validation and historical research. Planned protections are not
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
| **Idle Memory (PSS)** | < 100 MB combined | < 100 MB release milestone | Strictly below 100,000,000 bytes; passing the older 220 MB ceiling is insufficient |
| **Private Dirty Memory** | <= 130 MB | 130 MB | Separate from RSS and PSS |
| **Idle CPU Utilization** | <= 0.35% preferred | Strictly < 0.5% accepted fallback | User-approved fallback; exactly 0.5% fails. Older 1% hardware / 2% software diagnostic ceilings do not satisfy acceptance |
| **Cold Start Time** | <= 2.0s | 2.0s | Measure to a usable shell; record map and first live data separately |
| **UI Responsiveness** | Smooth interaction, 60 FPS aspiration | Non-blocking | No continuous idle repaint requirement; measure main-loop delay |

Maintainer's PSS increment over a matched minimal GTK baseline has a 40 MB
target; that comparison is a diagnostic delta, not exact memory attribution.
Software-rendered memory remains separately reported under the existing policy;
it cannot certify a universal under-100-MB claim. The new goal uses decimal MB;
retain raw bytes/MiB and resolve older budget-unit ambiguity in D0.
See [current release status](docs/release-status.md) for remaining validation.
The [archived roadmap](docs/archive/plans/development-roadmap.md) and
[archived memory investigation](docs/archive/research/2026-10-30-memory-under-100mb.md)
preserve the original experiment design and proposed checkpoints.

---

## Building from Source

### Prerequisites

Install the required build dependencies and development headers:

```bash
sudo apt update
sudo apt install -y libgtk-4-dev libsqlite3-dev libpolkit-gobject-1-dev pkg-config python3-apt xvfb dbus-x11 fakeroot dpkg-dev
```

Backup and recovery use the distribution SQLite library, including Ubuntu 22.04's SQLite 3.37. No bundled SQLite is required.

Maintainer requires Rust 1.85.0 or newer. When using `rustup`, the pinned toolchain from `rust-toolchain.toml` is used automatically.

### Compilation

```bash
# Build locked release binaries with local build paths remapped
python3 scripts/build-release.py

# Run the GUI application in development mode
cargo run -p maintainer-app

# Run the privileged helper in development mode
cargo run -p maintainer-helper
```

### Verification & Quality Assurance

All commits must satisfy formatting, linting, and test suites:

```bash
# Run every unit, host and native GTK test, including display fixtures
# Requires dbus-run-session and Xvfb; native GTK tests run in separate processes.
scripts/test-native.sh

# Verify formatting
cargo fmt --all -- --check

# Static analysis and lints
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Collect hardware diagnostics in a real unprivileged desktop session
PERF_SKIP_XVFB=1 scripts/measure-performance.sh target/release/maintainer-app --matrix
# This diagnostic alone does not certify the full release protocol.
```

The authoritative release check runs on this machine and needs no hosted runner,
second computer, or third-party verification service:

```bash
python3 scripts/check-release-local.py build /tmp/maintainer-candidate-new
python3 scripts/check-release-local.py verify /tmp/maintainer-candidate-new
```

Use a new output directory outside the source tree. Package staging and output
also stay outside the checkout; direct package builds accept an external directory
or create a private directory under `/tmp`. The gate runs all native and
Python tests three times, checks formatting/lint/builds/metadata, and inspects the
exact package and helper integration. Its manifest binds source, lockfile,
package and runtime payload hashes. It does not install packages on the host.
CI is optional additional automation and is not a release prerequisite. No
untested architecture, installed-upgrade behavior or hardware certification is
claimed. This is the unpublished 1.3.0 development candidate.

---

## Troubleshooting

### PolicyKit Authentication and D-Bus Activation

Package changes, process termination and file cleanup use the desktop PolicyKit agent. A missing installed owner/helper produces an unavailable state; no unbound fallback is executed. A reviewed cleanup batch authenticates once, and its grant is tied to the same live connection and exact file manifest.

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
