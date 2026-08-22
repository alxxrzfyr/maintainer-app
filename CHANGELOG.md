# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.0.0] - 2026-08-22

First release.

### Added

- System monitoring for CPU, memory, storage, temperature, network, and
  processes, with sparkline history and a 2 second refresh that backs off to
  10 seconds while the window is hidden.
- Storage analysis: cancellable scans that locate large files and verify
  duplicate groups by content hash, without ever deleting anything on its own.
- Cleanup with mandatory preview and confirmation, recorded operation history,
  Trash preferred over permanent deletion, and a stronger confirmation before
  any permanent removal.
- Update review for APT, plus Flatpak and firmware inspection where those tools
  are installed. Privileged execution is authorized through Polkit and never
  stores credentials.
- Diagnostics for failed systemd services and recurring journal events, keeping
  observed evidence separate from interpretation.
- Storage health reporting from SMART attributes via UDisks2, including a clear
  unavailable state for drives and enclosures that do not expose them.
- Local system reports that redact usernames, hostnames, IP addresses, serial
  numbers, sensitive paths, and process arguments by default.
- Native `.deb` packaging for `amd64` and `arm64`, with desktop entry, icons,
  GSettings schema, D-Bus service, and Polkit policy, plus a Polkit-gated
  helper binary separate from the unprivileged GUI. Each architecture is built
  natively, because linking requires the GTK4, GLib, SQLite, and Polkit
  development libraries for the target architecture.

### Security

- The GUI never runs as root. All privileged work goes through a D-Bus helper
  with fixed operation types and allowlisted parameters; arbitrary command
  execution is not possible by construction.
- External programs are invoked with explicit argument arrays, never through a
  shell, and their output is parsed by isolated, fixture-tested adapters.
- No telemetry, no account, and no system-data upload.

### Performance

- Per-process `/proc` enumeration is skipped unless the System page is visible,
  and page widgets are only refreshed for the page currently on screen.
- hwmon topology and sensor labels are discovered once and cached, so a sample
  reads only the temperature value files.
- Idle CPU measured at roughly 0.9% under a software rasterizer, down from
  about 10% before these changes.

[1.0.0]: https://github.com/alxxrzfyr/maintainer-app/releases/tag/v1.0.0
