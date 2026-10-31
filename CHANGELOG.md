# Changelog

All notable changes to this project are documented in this file.

The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and
this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Changed

- Run Diagnostics hardware collection once in a bounded worker instead of GTK
  constructors. Show loading, collection failures, and evidence-based hardware
  values; report unprobed acceleration and missing USB/DMI fields honestly.
- Allow hardware inspection tools through the command boundary and parse their
  machine-readable output with fixtures. Extract shell feature wiring, simplify
  kernel grouping, and replace theme/installer source checks with behavior tests.

- Target 0.35% average idle CPU, with the user-approved fallback strictly below
  0.5%. Enforce full precision; exactly 0.5% is a failure.
- Skip redundant network labels and storage style changes. Preallocate growing
  proc-text buffers while preserving complete input and contextual errors.

- Bound shared background work, retained results and child output; report
  saturation, cancellation and unavailable integrations explicitly.
- Load secondary pages on demand, reuse process/disk/sensor widgets, coalesce
  metrics, and save settings off the GTK thread with close-time draining.
- Discover duplicates using real file identity with bounded traversal and clear
  partial-coverage warnings. Store settings/history privately and support bounded
  backup-to-new-file recovery without overwriting existing databases.
- Bind process confirmation and Polkit authorization to a retained pidfd. Send
  SIGTERM through that handle; report delivery acceptance separately from exit.
- Export previews through anonymous private files and no-replace publication.
  Existing files and symlinks are preserved; choose a new filename to save again.
- Retain the native APT resolver cache and locks through authoritative review and
  commit. A separate short-lived owner provides scoped authorization, durable
  receipts, Resume, safe cancellation boundaries and read-only recovery audit.
  Protect held/essential/running-kernel/tracking packages and prevent self-update
  deadlocks. Same-name package selections remain distinct by architecture.
  Hold a verified shutdown/sleep inhibitor through package mutation and
  postcondition checks; refuse mutation if it cannot be acquired.
- Clean reviewed regular files through protected quarantine, with one approval
  bound to each batch. Raced replacements are restored or retained, never deleted.
  Trash and recovery never overwrite; explicit Restore elsewhere handles missing
  original folders. Postcommit durability failures do not offer unsafe retries.
  Folders, unsupported filesystems and cross-filesystem operations stay explicit;
  physical reclaimed bytes are conservatively not claimed. Trash follows custom
  XDG data-home settings, and directory review responds to cancellation.
- Make the same-machine release gate authoritative: three complete native/Python
  test passes, native builds and exact package/source/lock/runtime hashes. No
  host package installation is performed. Hosted CI independently checks the
  pushed source and disposable amd64/arm64 package lifecycle. Manual acceptance
  remains separate. No release has been published.

### Build and verification

- Keep generated Debian packages and validation artifacts outside the checkout;
  reject direct or symlinked output paths inside the source tree.

- Keep multiarch transaction fixtures independent of host dpkg administration
  defaults while still executing and verifying both native package installs.

- Preserve Ubuntu 22.04 system-SQLite compatibility for private backup/recovery.
  Use the stable snapshot ABI without enabling newer SQL error-offset symbols.

- Build release artifacts through a locked Cargo wrapper that remaps local source,
  Cargo and home paths while preserving existing compiler flags.
- Run CI on release branches, use the system Python APT binding, and pin Actions
  by commit. Add redacted history scans, current dependency advisories and workflow
  validation with checksum-pinned audit tools.
- Remove duplicate Debian install/compatibility declarations, correct toolchain
  prerequisites and repository links, and verify installed root-launch refusal.

### Privacy

- Default diagnostic reports omit unrestricted service descriptions, journal
  text, and capability failure details that may contain private system data.
  Failed previews show a generic message and cannot be copied or saved.
- Default SQLite storage resolves empty XDG values through HOME, rejects
  relative paths and symlinked components, and verifies the owned private
  directory and database inode before migrations. Newer schemas are preserved
  and rejected rather than silently treated as current.

### Documentation

- Assign setup/commands to README, contributor rules and acceptance to AGENTS,
  and current evidence to release status. Reconcile installed lifecycle results,
  verification entry points, prerequisites, and stale CSS design references.

- Group validation evidence by date and keep a short index. Store raw local logs
  in an ignored `logs/` directory; publish only reviewed reports and measurements.

- Consolidate 15 superseded validation notes and profiler outputs into one dated
  summary. Preserve failed measurements and the historical archive; repair links
  in reusable system validation procedures.

- Archive superseded PRDs, implementation plans, designs and research with Git
  provenance. Repository instructions treat them as historical records; current
  release status retains unfinished work and validation limits.
- Correct the threat model and security, compatibility, and accessibility
  procedures to distinguish current controls, open gaps, and unverified results.
- Consolidate development plans and research; align README performance ceilings
  with repository instructions and remove unsupported security guarantees.
- Add concrete safety decisions, resource limits proposed for validation, and
  work packages for duplicate-scan integration, report privacy and persistence.
  Record dependency-overlay uncertainty and preserve installed-test evidence gates.
- Retain fast, lightweight, native GTK4 GUI as explicit product values; set the
  next memory milestone below 100 MB combined idle PSS, with profiling experiments
  and lower exploratory targets. Attainment remains unverified.
  Runtime privacy and persistence hardening has started; D13 remains incomplete.

## [1.2.0] - 2026-08-23

### Added

- A dedicated Network page with live download and upload activity, connection
  details, Wi-Fi information, and an optional internet speed test.
- A dedicated Processes page for viewing, sorting, and safely ending processes.

### Improved

- The System page is easier to scan, with clearer CPU, memory, disk, network,
  and temperature information.
- Temperature readings are grouped by hardware source, duplicate and invalid
  readings are hidden, and sensor names are easier to understand.
- Navigation icons, spacing, colors, cards, buttons, and status messages are
  more consistent across the light and dark themes.
- System updates now show active progress and clearer transaction states.

### Fixed

- Update completion is no longer reported until Maintainer confirms that every
  reviewed package reached its expected version.
- Administrator authentication now appears reliably for critical actions and
  is requested again for each irreversible operation.
- The installer now detects incomplete authorization setup instead of leaving
  privileged features partially configured.
- Internet speed-test uploads, errors, and cancellation now behave reliably.
- Storage navigation no longer closes the application during certain page
  transitions.
- Missing icon-theme entries no longer produce blank placeholder icons.
- Destructive buttons remain readable in every interaction state.

### Privacy

- Passive network monitoring never contacts an external service.
- The internet speed test runs only after confirmation, explains that it uses
  Cloudflare and approximately 10–25 MB of data, and does not save its results.

## [1.1.3] - 2026-08-23

### Added

- Kernel management:
  - **Status enumeration** — installed kernels are listed as Running,
    Installed, or Removable. Removable is determined solely by what
    `apt-get --simulate autoremove` reports; no end-of-life or
    "unsupported" status is fabricated, since no reliable local source
    for per-build kernel EOL exists on Ubuntu or Mint.
  - **Triple-layered protection against removing the running kernel** —
    the core enumeration makes it structurally impossible for the running
    kernel to be marked removable, the helper independently re-reads
    `/proc/version` before executing a removal and rejects any match, and
    the UI disables the delete button for the running kernel even inside
    the already-removable code path.
  - **Safe removal** — a new privileged helper operation purges the
    kernel image, headers, modules, and modules-extra packages via
    `apt-get purge` with an explicit argument array, gated by a new
    Polkit action requiring a password on every attempt (not retained,
    since removal is irreversible). A strong confirmation dialog
    ("Remove Permanently") is required before the helper is called.
- Overview and System page redesign:
  - **Storage** — a real capacity bar with color thresholds at 80% and
    90% usage, replacing a bare percentage value.
  - **Network** — separate download/upload rate indicators.
  - **System page detail** — per-core CPU utilization, memory
    composition breakdown, per-disk capacity, and per-interface network
    rates, so the System page is a genuine drill-down rather than a
    near-duplicate of the Overview summary.
  - Existing icon set and typography classes are unchanged; this is a
    structural change to layout and information density, not a new
    visual language.

### Fixed

- Update discovery under-reported updates relative to the system's own
  update manager:
  - The cause was `apt-get --simulate upgrade`, apt's deliberately
    conservative mode, which refuses any update requiring a new package
    or a dependency change. On a real Ubuntu 24.04 / Mint machine this
    silently hid a kernel update entirely, because it is tied to an HWE
    metapackage chain that pulls in new headers, tools, and modules
    packages.
  - Switched to `apt-get --simulate dist-upgrade`, the same class of
    resolver mainstream update managers use, at all three call sites
    that previously used the conservative mode: the Updates page
    discovery, the pre-install consistency recheck, and the Diagnostics
    page's pending-updates count.
  - Verified directly against `apt list --upgradable` on a real machine:
    the package set now reported is an exact match, plus the underlying
    versioned kernel packages the metapackage view collapses.


### Added

- Updates page redesign:
  - **Selection modes** — a Security Only / Everything toggle above the
    package list, backed by a top action bar showing the summary and the
    Install button. The toggle is not cosmetic: it changes which candidates
    are actually submitted to PackageKit, not just which are displayed.
  - **Per-package selection** — checking or unchecking an individual
    package switches to a Custom selection mode. Custom selections never
    feed the installer directly; a Review Selection action re-invokes
    apt's own dependency resolver (`apt-get --simulate install
    name=version`, pinned to the reviewed version) and shows whatever apt
    itself discloses, including dependencies the user did not select,
    before Install becomes available again.
  - **Status icons per package** — security, kernel, held/phased (with
    distinguishable tooltips), and a backend badge for Flatpak and fwupd
    entries. Security and kernel are shown independently, since a kernel
    update can also be a security update.
  - **Reboot notice** — a page-level notice appears when the plan requires
    a restart and includes a kernel update, read from the same
    `/var/run/reboot-required` file apt itself uses.
  - **Custom selection indicator** — a visible label next to the mode
    toggle whenever a per-package checkbox has overridden it, so the mode
    change is not communicated only by the toggle buttons going blank.

### Fixed

- Update classification:
  - `classify_origin` was case-sensitive, so a literal `Debian-Security`
    origin fell through to Standard instead of Security. The check is now
    case-insensitive.
  - Linux Mint's `linuxmint` origin had no explicit rule, so every
    Mint-native package (Cinnamon, xapps, mintupdate) classified as
    Unknown rather than Standard. Now classified explicitly.
- Test suite safety:
  - Running the test suite was executing a live `apt-get clean` against
    the development machine, because the bus and Polkit policies installed
    for the 1.1.1 fix made the helper genuinely reachable, and
    clean-package-cache is deliberately passwordless for an active
    session. The affected tests no longer assert unconditional refusal for
    the two actions designed to succeed without a password.
- Visual styling:
  - Five new CSS classes from the Updates redesign (`action-bar-top`,
    `security-icon`, `kernel-icon`, `held-icon`, `backend-icon`) had no
    rules in either theme and rendered with no visual distinction. Added
    styling for both dark and light themes using the existing semantic
    color tokens.

## [1.1.1] - 2026-08-22

### Fixed

- Privileged actions failed with "Administrator authentication is required for
  this action" instead of asking for a password. No system bus policy was
  installed, and the system bus denies by default, so the call was rejected
  before the helper started and Polkit was never consulted. A bus policy is now
  shipped to `/usr/share/dbus-1/system.d/`, and a bus-level rejection is
  reported as the configuration fault it is rather than as a missing password.
- The D-Bus service file delegated activation to `maintainer-helper.service`,
  which was never shipped, so activation would have failed even once the bus
  permitted the call. The unit is now installed and carries the sandbox the
  threat model describes.

### Security

- Irreversible actions now require the password on every attempt. Terminating a
  process and vacuuming the journal use `auth_admin` rather than
  `auth_admin_keep`, so an authorization cannot be reused for a later action.
- Reversible and read-only actions no longer prompt for an active local session.
  Cleaning the APT cache only discards files APT can fetch again, and refreshing
  SMART data changes nothing, matching how udisks2 treats the same read.
- The threat model previously listed systemd hardening for a unit that did not
  exist. The unit now ships with those protections, and the two directives that
  cannot be asserted without a live privileged test are named as outstanding
  rather than claimed.

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
