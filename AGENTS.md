# Maintainer contributor instructions

Maintainer is a native GTK4 monitor and maintenance application for Ubuntu-based
Linux. Keep changes focused, safe, responsive, and verifiable.

## Sources of truth

- Code, Cargo manifests, scripts, workflows, and `data/styles/` define current
  behavior, commands, dependencies, and semantic CSS tokens.
- This file defines contributor rules and performance acceptance.
- [README](README.md) owns setup and day-to-day commands. Do not copy that guide
  into other documents.
- [Release status](docs/release-status.md) owns remaining work and evidence links.
  Dated reports record observations for a named source or artifact, not later code.
- [Threat model](docs/threat-model.md) explains trust boundaries and limitations.
  If documentation disagrees with code, verify the code and update the relevant
  document. Do not treat a plan or passing historical run as shipped behavior.

## Stack and boundaries

Use the pinned Rust toolchain, Cargo, GTK4, semantic GTK CSS, and SQLite for
settings/history. Target Ubuntu 22.04 or newer. No libadwaita, WebView, Electron,
embedded browser, telemetry, cloud account, or live-metric persistence.

- `ui`: widgets and presentation only. No Linux reads, command execution, or SQL.
- `core`: typed services, collectors, capability adapters, bounded cancellable
  jobs, and persistence. Return typed results for incremental UI updates.
- `helper`: fixed privileged operations over D-Bus/Polkit. The maintenance helper
  and dedicated package owner have separate service scopes. Never expose arbitrary
  commands, shell text, or unvalidated operation parameters.

Prefer native Rust APIs, `/proc`, `/sys`, D-Bus, filesystem APIs, then established
utilities. Run utilities with explicit argument arrays and fixture-tested parsers.
Optional integrations report available, unavailable with a reason, or permission
required. Missing data is not zero, success, or an inferred capability.

## Development and verification

Use the [README commands](README.md#development) and repository scripts:

- `scripts/test-native.sh` runs native tests with each GTK case in its own process.
  Plain parallel `cargo test` is not the full GTK verification entry point.
- `scripts/build-release.py` builds locked release binaries and rejects embedded
  local build paths. Do not package a direct Cargo release build that bypasses it.
- `scripts/check-release-local.py` runs the complete three-pass candidate gate.
  Its manifest is invalid after source, documentation, lockfile, or payload edits.
- CI performs its configured checks for the exact pushed commit. Check their
  actual results; a local pass, successful push, or package build is not CI proof.
- State why before changing dependencies or regenerating `Cargo.lock`. Use Cargo
  for deliberate dependency operations; never edit the lockfile by hand.

## Code style

- Let rustfmt own formatting. Clippy warnings are defects; narrowly scoped allows
  require a local rationale. Prefer behavioral tests over source-spelling checks.
- Follow Rust naming conventions. Use explicit `crate::` paths across top-level
  modules and `super::` only for close parent relationships.
- Keep GTK objects on the main thread and blocking work in bounded workers.
  UI defaults and constructors must not collect hardware or perform system I/O.
- Avoid production `unwrap`, `expect`, and ignored `Result` values. Return
  contextual typed errors at subsystem boundaries.
- Make units explicit: bytes, percentages, Celsius, durations, rates, and PIDs.
- Keep modules cohesive and interfaces narrow. UI modules are not service locators.
  Do not refactor unrelated code or add dependencies without need.
- `unsafe` requires necessity, documented invariants, and focused tests. Comments
  explain safety, policy, or non-obvious Linux semantics, not code narration.

## Safety and privacy

- Refuse root GUI launch; never store administrator credentials.
- Never execute arbitrary user commands. Preserve fixed operation allowlists.
- Retain and validate filesystem identities across review/execution; canonicalized
  paths alone do not prevent substitution races.
- Destructive actions require review and confirmation. Never automatically delete
  duplicates. Prefer Trash; permanent deletion requires stronger confirmation.
- Redact usernames, hostnames, IPs, serials, sensitive paths, and process arguments
  from reports by default. Preview remains necessary before sharing.
- Never read, print, commit, or upload signing keys, credentials, or private data.

## Performance rules

The active release milestone is strictly below **100,000,000 bytes combined idle
PSS** for Maintainer-owned processes on hardware rendering. The former 150 MB
PSS target and 220 MB ceiling are legacy context, not alternate release acceptance.
The matched minimal-GTK comparison retains a 40 MB application-increment target;
its delta is diagnostic, not exact attribution. Older GTK baseline figures are
observations tied to their renderer/environment, not universal lower bounds.

- Private dirty idle memory: at most 130,000,000 bytes.
- Hardware-rendered idle CPU: at most 0.35% preferred; the approved fallback is
  strictly below 0.5%. Exactly 0.5% fails. Older 1% hardware and 2% software ceilings are
  diagnostic only. Memory ceilings are not enforced under software rendering;
  report it separately rather than certifying hardware acceptance.
- Usable startup: at most 2 seconds. Record map/first-data milestones separately;
  a warm process launch is not cold-boot proof.
- Default sampling is 2 seconds visible and slower while hidden. Update affected
  widgets; never rebuild the dashboard for one metric.
- No permanent daemon or unsolicited scans. Deep storage, duplicates, SMART, and
  journal work run on request with progress and cancellation.
- Measure performance-sensitive changes against these limits. Preserve failures
  and distinguish individual diagnostics from complete release acceptance.

## Protected changes

- Helper API/Polkit changes require explicit authorization, threat review, and
  authorization tests.
- Shipped SQL migrations are immutable; add a forward migration.
- Packaging signing/APT credentials must never be read, printed, or committed.
- Explain and obtain approval before weakening CI, compiler, formatter, or lint
  policy. Do not skip failing checks to obtain a pass.

## Documentation and outputs

Keep setup in README, rules here, current validation in release status, and
release history in CHANGELOG. Contract changes update README and CHANGELOG in
that change. Link shared guidance instead of repeating thresholds or commands.

Use `docs/validation/YYYY-MM-DD/report.md` for reviewed results and place supporting
measurements beside it. Keep raw output in ignored `logs/YYYY-MM-DD/<run>/`.
Never force-add raw logs. Packages/candidate bundles stay outside the checkout;
`target/` is Cargo-owned ignored build output.

`docs/archive/` is read-only history unless a historical update is explicitly
requested. It is not current policy or a task queue. Archiving does not complete
or waive unfinished work.

## Definition of done

1. Relevant behavioral tests pass; changed behavior has a regression that fails
   without the fix. Privileged/destructive cases cover authorization, validation,
   denial, cancellation, and partial failure.
2. Formatting, lint, affected builds, and required repository checks pass without
   new warnings, leaked private data, GTK-thread blocking, or unexplained failures.
3. Applicable loading, unavailable, permission, empty, success, partial-failure,
   failure, and cancelled states are clear. Performance-sensitive work is measured.
4. Relevant documentation matches the change and records unverified scope honestly.
