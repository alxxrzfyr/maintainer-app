<div align="center">

<img src="data/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg" alt="Maintainer logo" width="96" height="96">

# Maintainer

Native GTK4 monitoring and reviewed maintenance for Ubuntu-based Linux.

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%2B-E95420?logo=ubuntu&logoColor=white)](https://ubuntu.com)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

</div>

Maintainer brings monitoring, storage inspection, maintenance, and diagnostics
into a native interface designed to stay fast and lightweight. It has no telemetry, cloud account, or
permanent background daemon. Maintenance requires explicit user action.

## Install

Download the package matching your architecture from
[Releases](https://github.com/alxxrzfyr/maintainer-app/releases). Replace `VERSION`
and `ARCH` below with the downloaded filename's version and architecture:

```bash
sudo apt install ./maintainer_VERSION-1_ARCH.deb
maintainer
```

Run the GUI as your normal user. The installed D-Bus/Polkit services handle
privileged work. To remove the package:

```bash
sudo apt remove maintainer
# Purge system configuration; user-home data remains.
sudo apt purge maintainer
```

Candidate version and validation are tracked in [release status](docs/release-status.md).
A successful development build does not publish a release.

## Features

- Live CPU, memory, storage, sensors, network, and process monitoring with in-memory
  history and incremental updates.
- On-demand large-file and duplicate inspection with bounded scans, progress,
  cancellation, and explicit partial coverage. Duplicates are never auto-deleted.
- Reviewed regular-file cleanup, Trash, stronger permanent-delete confirmation,
  protected quarantine, and recovery without overwriting existing entries.
- Native APT review and transactions with exact approval, durable receipts,
  cancellation boundaries, and resume. Held, essential, and active kernel packages
  are protected. Self-updates use the normal system package manager.
- Service/journal diagnostics, optional SMART health, hardware inspection, and
  redacted report preview, clipboard, and private file export.

The GUI, core services, maintenance helper, and package owner have separate roles.
See the [threat model](docs/threat-model.md) for controls and limitations.

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

## Development

Install build and verification prerequisites:

```bash
sudo apt update
sudo apt install -y build-essential libgtk-4-dev libsqlite3-dev \
  libpolkit-gobject-1-dev pkg-config python3 python3-apt xvfb xauth dbus-x11 \
  desktop-file-utils appstream fakeroot dpkg-dev
```

Use rustup with the toolchain pinned in `rust-toolchain.toml`. Cargo owns Rust
packages and the lockfile. Backup/recovery uses system SQLite, including Jammy's
SQLite 3.37; no bundled SQLite is required.

```bash
cargo run --locked -p maintainer-app
cargo build --locked --workspace
python3 scripts/build-release.py
```

The release wrapper remaps local build paths and checks the resulting binaries.
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

## Troubleshooting

- **Authorization:** Check the installed helper/package-owner services, a running
  desktop Polkit agent, and `systemctl is-active dbus`. Missing services do not
  trigger an unbound fallback command.
- **Graphics:** `GSK_RENDERER=cairo maintainer` tests software rendering;
  `GDK_BACKEND=x11 maintainer` tests X11. Record overrides in measurements.
- **Unavailable hardware:** SMART needs UDisks2 and compatible devices. Sensors
  require Linux hwmon support. Missing information must not be presented as zero
  or verified acceleration.

See [documentation](docs/README.md) for validation, security guidance, and history.
Maintainer is licensed under the [MIT License](LICENSE).
