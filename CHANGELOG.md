# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [1.1.0] - 2026-08-22

### Added

- The full interface now ships: the Overview, System, Storage, Cleanup,
  Updates, Diagnostics, Reports, and Settings pages, a device specification
  panel, a hardware inventory view covering GPU, USB, PCI, and BIOS, expanded
  diagnostics and cleanup flows, storage sub-pages, and the complete light and
  dark stylesheets.

### Fixed

- The 1.0.0 package was built before the interface work had been committed, so
  it installed the earlier scaffold interface. Its changelog entry described
  features the artifact did not contain. Nothing was wrong with the packaging
  itself; the code simply was not in the repository at that point.
- Four helper client tests asserted that an unreachable helper reports
  `Unavailable`. That holds only where no system bus exists; on a desktop with
  a bus and Polkit the authorization is refused first and the result is
  `PermissionRequired`. Both are correct refusals, so either is now accepted.

### Performance

- Only the page on screen is refreshed. Both page update paths rebuild widget
  subtrees, so refreshing a hidden page forced repaints that changed nothing
  visible. Measured idle CPU fell from 2.79% to 1.39% under a software
  rasterizer. Switching pages repaints the incoming page immediately, so no
  stale content is shown.
- hwmon topology and sensor labels are discovered once, so each sample reads
  only the temperature value files.

[1.1.0]: https://github.com/alxxrzfyr/maintainer-app/releases/tag/v1.1.0

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

### Fixed

- Package dependencies now match what the binaries actually use.
  `libpackagekit-glib2-18` was declared but never linked, since PackageKit is
  reached over D-Bus rather than through its C library. The `packagekit` and
  `udisks2` services, which are needed at runtime for update execution and
  SMART reporting, were not declared at all and are now Recommends. Polkit is
  requested as `polkitd | policykit-1` so the package resolves on Ubuntu 22.04
  and on releases where `policykit-1` became transitional.

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
