# Maintainer

<div align="center">

[![CI](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml/badge.svg)](https://github.com/alxxrzfyr/maintainer-app/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/alxxrzfyr/maintainer-app)](https://github.com/alxxrzfyr/maintainer-app/releases)
[![License](https://img.shields.io/github/license/alxxrzfyr/maintainer-app)](LICENSE)

**Your Linux computer's personal maintenance center.**

</div>

Checking on your own computer usually means five different tools and a browser tab of half-remembered terminal commands. Maintainer puts the useful parts in one window: what your machine is doing right now, what is eating your disk, what needs updating, and what is actually broken.

It runs on your machine, for you. No account, no telemetry, nothing uploaded.

## What you get

| | |
|---|---|
| **Live monitoring** | CPU, memory, disk, network, temperature, and processes, with small history graphs |
| **Disk detective** | Find the biggest files, and duplicates confirmed by content rather than by name |
| **Cleanup you approve** | See exactly what would be removed before anything happens. Files go to Trash by default |
| **Updates** | Review pending APT updates and apply them. Flatpak and firmware too, if you have them |
| **Diagnostics** | Which services failed, which errors keep repeating, and what that probably means |
| **Disk health** | SMART data and a plain verdict, for drives that report it |
| **Shareable report** | A snapshot of your system with personal details stripped out, so you can paste it somewhere safe |

If your hardware or system does not support something, Maintainer says so and explains why. It never shows a fake zero.

## What it refuses to do

This matters as much as the feature list:

- The window never runs as root, and never asks for your password to keep
- It will not run commands you type into it
- It will not delete duplicates for you, and there is no "clean everything" button
- It does not sit in the background. Nothing scans your disk unless you ask
- No telemetry, no account, no data leaving your machine

Anything needing admin rights goes through a small, separate helper with a fixed list of allowed operations, gated by your system's normal password prompt. Details in [`docs/threat-model.md`](docs/threat-model.md).

## Install

Grab the `.deb` for your architecture from the [releases page](https://github.com/alxxrzfyr/maintainer-app/releases):

```bash
dpkg --print-architecture              # amd64 or arm64?

sudo apt install ./maintainer_1.0.0-1_amd64.deb
```

Anything it needs that you do not already have, apt pulls in automatically, so that one command is the whole install. To remove it later:

```bash
sudo apt remove maintainer   # keeps your settings
sudo apt purge  maintainer   # removes them too
```

**Works on:** Ubuntu 22.04 or newer, and Ubuntu-based systems like Mint and Pop!_OS. Other Linux families are not supported yet. There is no Windows or macOS version — Maintainer reads Linux-specific system interfaces directly, so there is nothing to port.

## How it's put together

Three parts, and the split is the point:

```text
maintainer-app      the window: buttons, graphs, layout
      |             never touches the system directly
      v
maintainer-core     the brain: reads /proc and /sys, talks to D-Bus,
      |             runs scans in the background, stores your settings
      v
maintainer-helper   the only part allowed to act as root, and only for a
                    fixed list of operations with checked arguments
```

Why bother: the window can be sloppy and it still cannot hurt your system, because it has no power to. Anything dangerous has to pass through the helper, which is small enough to read in one sitting and refuses anything not on its list.

When Maintainer needs to run an outside program, it calls it directly with an explicit list of arguments — never by building a shell command out of text. Output is parsed by small pieces of code tested against recorded real-world samples.

## Speed and memory

Maintainer is meant to be usable on an old laptop with 4 GB of RAM:

- Starts in under 2 seconds
- Sits under 1% CPU when you leave it open
- Refreshes every 2 seconds, and slows down to every 10 when the window is hidden
- Big jobs — disk scans, duplicate hunting, SMART, log searches — only run when you ask, show progress, and can be cancelled

On memory, honesty is better than a small number: most of what Maintainer uses is GTK4 itself. An empty GTK4 window already costs about 135 MB. Maintainer adds roughly 35 MB on top of that, and around 100 MB of the total is genuinely private to it — about 2.5% of a 4 GB machine, which will not push it into swapping.

## Build it yourself

```bash
sudo apt install libgtk-4-dev libsqlite3-dev libpolkit-gobject-1-dev pkg-config

cargo build --workspace --release
cargo run -p maintainer-app
```

Rust installs its own pinned toolchain from `rust-toolchain.toml`, so you do not need to pick a version.

Before sending a change:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
scripts/measure-performance.sh target/release/maintainer-app
```

Architecture rules, conventions, and what counts as finished live in [`AGENTS.md`](AGENTS.md).

## License

[MIT](LICENSE) — do what you like with it.
