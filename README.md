<div align="center">

<br>

<img src="data/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg" alt="Maintainer logo" width="96" height="96">

# Maintainer

**Monitor your system. Review maintenance before it runs.**

A native GTK4 app for Ubuntu-based Linux.<br>
Inspect storage, manage updates, and understand system health in one place.

[Features](#features) · [Install](#install) · [Development](#development) · [Documentation](docs/README.md) · [Release status](docs/release-status.md)

<br>

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%2B-E95420?logo=ubuntu&logoColor=white)](https://ubuntu.com)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

<br>

</div>

Maintainer brings everyday system checks and maintenance into a single desktop
interface built with Rust. Use it to see what is using your resources, inspect
files taking up space, review package changes, and collect diagnostic information.

System data stays on your computer. There is no telemetry, cloud account, or
permanent background daemon, and maintenance requires explicit user action.
Live metric history stays in memory; SQLite stores settings and application history.

**Current status:** 1.3.0 is an unpublished development candidate. CPU acceptance
and manual system validation remain open. See [release status](docs/release-status.md)
for the latest evidence and remaining work.

## Features

- **Live monitoring.** View CPU, memory, disks, sensors, network traffic, and
  processes with in-memory history. Monitoring refreshes every two seconds by
  default, slows while hidden, and updates affected widgets incrementally.
- **Storage inspection.** Start large-file and duplicate scans when you need
  them. Scans report progress, support cancellation, and identify incomplete
  coverage so you can distinguish inspected files from skipped or unreadable paths.
  Duplicate files are never deleted automatically.
- **Reviewed cleanup.** Inspect selected regular files before choosing Trash or
  explicitly confirming permanent deletion. Protected quarantine and recovery
  preserve raced replacements and existing destination files. Folder removal
  and unsupported filesystem operations remain unavailable.
- **Package updates.** Review the exact native APT plan before applying changes.
  Transactions have durable receipts, cancellation boundaries, Resume, and
  shutdown/sleep inhibition during mutation and verification. Held, essential,
  active-kernel, and kernel-tracking packages are protected; self-updates use
  the normal system package manager.
- **Diagnostics and hardware.** Inspect failed services, journal findings,
  hardware inventory, and optional SMART health. Missing information is explained;
  unprobed graphics acceleration is shown as unverified.
- **Reports you can review.** Preview redacted diagnostic reports, copy them,
  or save a new private file without overwriting an existing name. Review the
  preview before sharing; automated redaction is not a substitute for checking it.

<br>

## Install

Download the `amd64` or `arm64` Debian package from
[Releases](https://github.com/alxxrzfyr/maintainer-app/releases). Replace `VERSION`
and `ARCH` below with the downloaded filename's values. APT installs the required
package dependencies:

```bash
sudo apt install ./maintainer_VERSION-1_ARCH.deb
maintainer
```

Launch Maintainer from your application menu or the command above. Run the GUI
as your normal user; installed D-Bus/Polkit services handle privileged work and
request authorization when required.

To remove the application:

```bash
sudo apt remove maintainer
# Purge system configuration; user-home data remains.
sudo apt purge maintainer
```

## Requirements

- Ubuntu 22.04 or newer; Ubuntu-based derivatives are target platforms.
- GTK 4.6+, GLib 2.72+, system SQLite, D-Bus, Polkit, systemd, and APT/dpkg.
- Native X11 or Wayland session. Tested environments are listed in release status;
  target-platform wording is not a completed compatibility matrix.
- UDisks2 for SMART; `flatpak`/`fwupdmgr` for optional update inspection,
  `curl` for the confirmed speed test, and `iw` for detailed Wi-Fi information.
  Optional `pciutils`/`usbutils` tools add hardware names; sysfs supplies native
  device evidence. Acceleration stays unverified until its API is actually probed.

Missing optional integrations produce an explained unavailable/permission state.
The speed test resolves `curl` in its worker after confirmation and displays a
failure reason if it is unavailable. Opening Network does not probe for `curl`.

<br>

## Architecture and safety

Maintainer separates presentation, system services, and privileged operations:

- `maintainer-app` owns GTK widgets, presentation state, and background job dispatch.
- `maintainer-core` provides collectors, typed services, cancellable jobs, and persistence.
- `maintainer-helper` provides fixed privileged maintenance operations and the
  separately scoped package owner through D-Bus/Polkit.

Blocking system work runs outside the GTK main thread. Privileged operations
validate callers and parameters; they do not accept arbitrary shell commands.
File and process reviews retain identities through execution rather than trusting
paths or process IDs alone. The [threat model](docs/threat-model.md) documents
these controls, supported cases, and remaining risks.

<br>

## Development

Install [rustup](https://rustup.rs/) and the build/verification prerequisites.
Rustup selects the toolchain pinned in `rust-toolchain.toml`:

```bash
sudo apt update
sudo apt install -y build-essential libgtk-4-dev libsqlite3-dev \
  libpolkit-gobject-1-dev pkg-config python3 python3-apt xvfb xauth dbus-x11 \
  desktop-file-utils appstream fakeroot dpkg-dev
```

Run the app, build the workspace, or prepare release binaries:

```bash
cargo run --locked -p maintainer-app
cargo build --locked --workspace
python3 scripts/build-release.py
```

The release wrapper remaps local build paths and checks the resulting binaries.
Cargo owns Rust dependencies and the lockfile. Backup/recovery uses system SQLite,
including Ubuntu 22.04's SQLite 3.37; no bundled SQLite is required.
Privileged workflows require the installed services; do not launch the GUI or a
bare development helper with sudo.

### Verify changes

```bash
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
bash scripts/test-native.sh
/usr/bin/python3 -W error::ResourceWarning -m unittest discover -s tests/performance -p 'test_*.py'
/usr/bin/python3 -W error::ResourceWarning -m unittest discover -s tests/release -p 'test_*.py'
/usr/bin/python3 -W error::ResourceWarning -m unittest discover -s tests/package_transaction -p 'test_*.py'
```

`test-native.sh` includes ignored display fixtures and app integration targets.
Each GTK case runs in a separate process with Xvfb and a private session bus.
Do not substitute a parallel whole-workspace Cargo test run for this script.
For a focused core check, use `cargo test --locked -p maintainer-core persistence::`.

### Build a candidate

```bash
python3 scripts/check-release-local.py build /tmp/maintainer-candidate-new
python3 scripts/check-release-local.py verify /tmp/maintainer-candidate-new
```

Choose a new directory outside the checkout. The gate runs three full test
passes, formatting, linting, both builds, metadata, and exact package inspection.
Its manifest binds the source, lockfile, package, and runtime payloads. Later edits
invalidate that proof. It does not install packages on your host.

To package binaries already built by the release wrapper:

```bash
EXPECTED_DEB_ARCH=$(dpkg --print-architecture) bash scripts/build-package.sh /tmp/maintainer-packages
```

Without an output argument, the builder creates a private `/tmp/maintainer-artifacts.*`
directory. Source-tree output, including symlinked destinations, is refused.
Keep raw local output in ignored [logs](logs/README.md).

CI runs the configured native, security, packaging, and disposable installed-upgrade
checks for the exact pushed commit. Local tests, hosted checks, hardware measurements,
and interactive manual validation are separate evidence.

### Measure performance

```bash
PERF_SKIP_XVFB=1 scripts/measure-performance.sh target/release/maintainer-app --matrix
```

Use a real unprivileged desktop session without concurrent builds/tests.
The [performance rules](AGENTS.md#performance-rules) define acceptance; current
results and remaining work belong in [release status](docs/release-status.md).
A short diagnostic or one passing page does not certify the full protocol.

<br>

## Troubleshooting

- **Authorization:** Check the installed helper/package-owner services, a running
  desktop Polkit agent, and `systemctl is-active dbus`. Missing services do not
  trigger an unbound fallback command.
- **Graphics:** `GSK_RENDERER=cairo maintainer` tests software rendering;
  `GDK_BACKEND=x11 maintainer` tests X11. Record overrides in measurements.
- **Unavailable hardware:** SMART needs UDisks2 and compatible devices. Sensors
  require Linux hwmon support. Missing information must not be presented as zero
  or verified acceleration.

<br>

## Documentation

- [Documentation index](docs/README.md): guides, validation records, and manual checks.
- [Contributor instructions](AGENTS.md): coding rules, architecture boundaries, and performance budgets.
- [Release status](docs/release-status.md): current scope, verification evidence, and remaining acceptance work.
- [Threat model](docs/threat-model.md): security controls and their limitations.
- [Changelog](CHANGELOG.md): changes across releases.

Report problems through [GitHub Issues](https://github.com/alxxrzfyr/maintainer-app/issues),
including your distribution, reproduction steps, and any relevant redacted output.

## License

Maintainer is developed for Linux users who want to understand, monitor, and
maintain their systems. The project is available under the [MIT License](LICENSE).
