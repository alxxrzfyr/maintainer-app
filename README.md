<div align="center">

<br>

<img src="data/icons/hicolor/scalable/apps/io.github.maintainerapp.Maintainer.svg" alt="Maintainer logo" width="96" height="96">

# Maintainer

**Monitor your system. Review maintenance before it runs.**

A native GTK4 app for Ubuntu-based Linux.<br>
Inspect storage, manage updates, and understand system health in one place.

[Features](#features) · [Installation](#installation) · [Development Setup](#development-setup) · [Documentation](docs/README.md) · [Releases](https://github.com/alxxrzfyr/maintainer-app/releases)

<br>

[![Ubuntu](https://img.shields.io/badge/Ubuntu-22.04%2B-E95420?logo=ubuntu&logoColor=white)](https://ubuntu.com)
[![Linux Mint](https://img.shields.io/badge/Linux_Mint-87CF3E?logo=linuxmint&logoColor=white)](#requirements)
[![Pop!_OS](https://img.shields.io/badge/Pop!__OS-48B9C7?logo=popos&logoColor=white)](#requirements)
[![Rust](https://img.shields.io/badge/Rust-1.85%2B-000000?logo=rust&logoColor=white)](https://www.rust-lang.org)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE)

<br>

</div>

Maintainer brings everyday system checks and maintenance into a single desktop
interface built with Rust. Use it to see what is using your resources, inspect
files taking up space, review package changes, and collect diagnostic information.

System data stays on your computer. There is no telemetry, cloud account, or
permanent background daemon, and maintenance requires explicit user action.
Live metric history stays in memory; SQLite stores settings and application history.

Feel free to check the [release status](docs/release-status.md) for development
progress, validation results, and remaining work before the next release.

<br>

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

<a id="install"></a>

## Installation

Download the `amd64` or `arm64` Debian package from
[Releases](https://github.com/alxxrzfyr/maintainer-app/releases). Choose `amd64`
for 64-bit Intel/AMD systems or `arm64` for 64-bit ARM systems. Replace `VERSION`
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

<br>

## Requirements

APT installs the declared runtime dependencies when you follow the
[installation steps](#installation). Rust and development packages are needed
only when [building from source](#development-setup).

| Component | Requirement |
| --- | --- |
| Operating system | Ubuntu 22.04 or newer. Ubuntu-based derivatives are also target platforms. |
| Package architecture | `amd64` (64-bit Intel/AMD) or `arm64` (64-bit ARM). |
| Desktop session | Native X11 or Wayland, running as a regular user. |
| Desktop libraries | GTK 4.6 or newer and GLib 2.72 or newer. |
| Local storage | System SQLite for settings and application history. |
| Package management | APT/dpkg, Python 3, and `python3-apt` for native package operations. |
| System services | D-Bus for service communication, Polkit for authorization, and systemd for service management. |

See [release status](docs/release-status.md) for tested environments and remaining
validation. The target platforms above do not imply that every configuration has
been tested.

<br>

### Optional features

These packages support additional features. Install them as needed:

| Feature | Ubuntu package | What it provides |
| --- | --- | --- |
| Drive health | `udisks2` | S.M.A.R.T. health information for compatible drives; authorization may be required. |
| Flatpak updates | `flatpak` | Inspection of available Flatpak updates. |
| Firmware updates | `fwupd` | Inspection of available firmware updates through `fwupdmgr`. |
| Network speed test | `curl` | Runs a speed test after you confirm it. |
| Wi-Fi details | `iw` | Additional information about the wireless connection. |
| PCI device names | `pciutils` | Adds readable device names through `lspci`. |
| USB device names | `usbutils` | Adds readable device names through `lsusb`. |

When an integration is missing or needs permission, Maintainer explains the
affected feature's status. The speed test checks for `curl` only after confirmation
and reports if it is missing. Opening Network does not perform that check.
Hardware information also comes from Linux sysfs; graphics acceleration remains
unverified until its API is probed.


## Architecture and Security

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

<a id="development"></a>

## Development Setup

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

<br>

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


### Dependency maintenance

[Dependabot](.github/dependabot.yml) checks Cargo dependencies and GitHub Actions
every Monday at **09:00 Singapore time**, with up to five open version-update
pull requests each for Cargo and Actions:

- **GTK Rust bindings:** related crates share one pull request for compatibility review.
- **Other dependencies:** minor and patch updates are grouped separately for Cargo
  and Actions; major updates get individual pull requests.

Security updates follow the repository's GitHub security settings and are handled
separately from these version-update groups.

Before merging, review the changes and CI results. Keep exact Cargo version pins
and full commit hashes for Actions, and check compatibility with the pinned Rust
toolchain and supported Ubuntu libraries. Update the Rust toolchain and any tools
downloaded by scripts or workflow shell steps manually.

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

Pushing a stable version tag such as `v1.3.0` also publishes a GitHub Release
after its automated checks pass, using the exact installed-tested packages and
checksums. See the [release guide](docs/releasing.md) for tagging and recovery.

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
- [Release guide](docs/releasing.md): version preparation, tags, verified artifacts, and publication.
- [Threat model](docs/threat-model.md): security controls and their limitations.
- [Changelog](CHANGELOG.md): changes across releases.

Report problems through [GitHub Issues](https://github.com/alxxrzfyr/maintainer-app/issues),
including your distribution, reproduction steps, and any relevant redacted output.

<br>

## License

Maintainer is developed for Linux users who want to understand, monitor, and
maintain their systems. The project is available under the [MIT License](LICENSE).
