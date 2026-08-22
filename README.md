# Maintainer

<div align="center">

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![License](https://img.shields.io/github/license/alxxrzfyr/maintainer-app)](LICENSE)

**Your Linux computer's personal maintenance center.**

</div>

Maintainer is a native GTK4 desktop application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux systems. It brings system monitoring, storage analysis, previewed cleanup, update review, and diagnostics into one place — without becoming a fake optimizer, an administration suite, or a cloud service.

## Features

- **System monitoring** — CPU, memory, storage, temperature, network, and process activity, with sparkline history
- **Storage analysis** — locate large files and verified duplicate groups through cancellable scans
- **Safe cleanup** — every removal is previewed, confirmed, and recorded; Trash is preferred over permanent deletion
- **Update review** — inspect and apply available APT updates, plus Flatpak and firmware where those are installed
- **Diagnostics** — failed systemd services and recurring journal events, with evidence separated from interpretation
- **Storage health** — SMART attributes and overall assessment via UDisks2 where the drive exposes them
- **Local reports** — generate a redacted system snapshot you can inspect before sharing

Every optional integration reports one of three states: available, unavailable with a reason, or permission required. An unsupported capability is a normal state, never a crash or a misleading zero.

## Design constraints

Maintainer deliberately does not:

- run the GUI as root, or store administrator credentials
- execute arbitrary user-supplied commands
- delete duplicates automatically, or offer a "clean everything" action
- run a permanent background daemon, or scan the filesystem unsolicited
- collect telemetry, require an account, or upload system data

Privileged work is delegated to a narrowly scoped D-Bus helper with fixed operation types, allowlisted parameters, and Polkit authorization. See [`docs/threat-model.md`](docs/threat-model.md).

## Requirements

Maintainer targets current Ubuntu-based distributions, with **Ubuntu 22.04 LTS as the compatibility floor**. Non-Ubuntu distribution families are not first-class targets.

Runtime dependencies are resolved automatically by the `.deb`:

`libgtk-4-1` (≥ 4.6), `libglib2.0-0` (≥ 2.72), `libsqlite3-0`, `policykit-1`, `packagekit`, `udisks2`, `systemd`

Optional, enabling extra integrations when present: `flatpak`, `fwupd`.

## Installation

Download the latest `.deb` from the [releases page](https://github.com/alxxrzfyr/maintainer-app/releases):

```bash
sudo apt install ./maintainer_1.0.0-1_amd64.deb
```

This installs the unprivileged GUI at `/usr/bin/maintainer`, the Polkit-gated helper at `/usr/libexec/maintainer-helper`, and the desktop entry, icons, GSettings schema, D-Bus service, and Polkit policy.

Removal keeps user settings; purge removes them:

```bash
sudo apt remove maintainer   # binaries and data
sudo apt purge  maintainer   # also user databases
```

## Performance budget

Measured on a low-end reference system with a release build. Absolute memory figures are dominated by GTK4 itself — a GTK4 window containing a single label already costs roughly 167 MB RSS / 135 MB PSS — so Maintainer's own share above that baseline is budgeted separately, and is the figure application changes can actually move.

| Measure | Budget |
|---|---|
| Idle memory | 150 MB PSS target, 220 MB ceiling |
| Idle memory attributable to Maintainer | 40 MB PSS target above a minimal GTK4 baseline |
| Private/dirty idle memory | 130 MB or less, keeping a 4 GB system near 4% of total RAM without swapping |
| Idle CPU | 0.5% target; 1.0% ceiling with hardware acceleration, 2.0% under a software rasterizer |
| Cold start | 2 seconds or less |
| Monitoring refresh | 2 seconds visible, 10 seconds when hidden |

Storage, duplicate, SMART, and journal work runs only on request, reports progress, and can be cancelled.

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

Three boundaries are enforced:

| Crate | Responsibility |
|---|---|
| `maintainer-app` | GTK4 widgets and presentation state; never reads Linux interfaces or SQLite directly |
| `maintainer-core` | Typed services, capability adapters, collectors, cancellable jobs, persistence |
| `maintainer-helper` | Narrowly scoped privileged operations with validated callers and parameters |

External programs are invoked with explicit argument arrays, never through a shell, and their output is parsed by isolated, fixture-tested adapters.

## Building from source

Requires `libgtk-4-dev`, `libsqlite3-dev`, `libpolkit-gobject-1-dev`, and `pkg-config`. The Rust toolchain is installed automatically by rustup from `rust-toolchain.toml`.

```bash
sudo apt install libgtk-4-dev libsqlite3-dev libpolkit-gobject-1-dev pkg-config

cargo build --workspace --release
cargo run -p maintainer-app
```

Verification:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
scripts/measure-performance.sh target/release/maintainer-app
```

Contributor guidance, architecture rules, and the definition of done live in [`AGENTS.md`](AGENTS.md).

## License

Released under the [MIT License](LICENSE).
