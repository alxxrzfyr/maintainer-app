# Maintainer

Maintainer is a lightweight native GTK4 application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux computers.

## Project status

Product requirements and UI design are approved. Application source, `Cargo.toml`, lockfile, CI, packaging, and executable commands have not been created. Do not claim a command was run or passed until the relevant project files exist and the command has been executed.

## Stack

| Layer | Choice |
|---|---|
| Language | Rust, stable toolchain pinned when the project is scaffolded |
| UI | GTK4 only; no libadwaita, WebView, Electron, or embedded browser |
| Styling | GTK CSS with semantic tokens defined in `DESIGN.md` until code becomes the source of truth |
| Persistent data | SQLite for settings and application history only |
| Build and dependencies | Cargo |
| Packaging | Native `.deb` package and APT repository |
| Platform | Current Ubuntu-based distributions; Ubuntu 22.04 compatibility floor |
| Privilege boundary | Unprivileged GUI plus a small audited D-Bus/Polkit helper |

## Architecture

Maintain three boundaries:

1. `ui`: GTK widgets and presentation state. Never read Linux interfaces, run commands, or access SQLite directly.
2. `core`: typed application services, capability adapters, collectors, cancellable jobs, and persistence interfaces.
3. `helper`: narrowly scoped privileged operations with validated callers and parameters. Never expose arbitrary command or shell execution.

Expected data flow:

```text
GTK view
  |
Application service
  |
Capability adapter or cancellable job
  |
Linux API / D-Bus / validated helper
  |
Typed result
  |
Incremental GTK update
```

Use native Rust APIs, `/proc`, `/sys`, D-Bus, filesystem APIs, then established Linux utilities in that preference order. External programs must be invoked directly with explicit argument arrays, never through a shell string. Parse their output through isolated, fixture-tested adapters.

Every optional integration reports one of: available, unavailable with reason, or permission required. Unsupported capabilities are normal states, not exceptional crashes.

## Setup and commands

No setup or commands are available yet because the repository has no Rust project. During scaffolding, define and verify commands for running, building, formatting, linting, testing one target, testing all targets, security-sensitive helper tests, and packaging. Update this section with only commands that were successfully executed.

After scaffolding, Cargo is the only Rust package manager. Never install a dependency or regenerate `Cargo.lock` without stating why.

## Code style

- `rustfmt` owns Rust formatting; do not hand-format around it.
- `clippy` warnings are defects unless a narrowly scoped allow has a written rationale.
- Use explicit `crate::` paths across top-level modules and `super::` only for close parent relationships.
- Keep GTK objects on the main thread. Perform scans and blocking system work in bounded workers, returning typed messages to the UI.
- Avoid `unwrap`, `expect`, and ignored `Result` values in production paths. Convert failures into contextual typed errors at subsystem boundaries.
- Model units in names or types: bytes, percentages, Celsius, durations, rates, and process IDs must not be ambiguous.
- Files and modules use `snake_case`; types and traits use `PascalCase`; functions and variables use `snake_case`; constants use `SCREAMING_SNAKE_CASE`.
- Prefer small modules with one responsibility and narrow interfaces. UI modules must not become service locators.
- `unsafe` requires necessity, a documented invariant, and focused tests.
- Comments explain safety, policy, or non-obvious Linux semantics—not code narration.
- Do not add dependencies when the standard library or an existing dependency is sufficient.
- Do not refactor unrelated code while implementing a focused change.

## Safety and privacy rules

- The main application must never run as root or store administrator credentials.
- Never execute arbitrary user-provided commands.
- Validate and canonicalize filesystem targets without following unsafe path substitutions between review and execution.
- Destructive actions require review and confirmation. Duplicate files are never deleted automatically.
- Prefer Trash where supported. Permanent deletion requires explicit stronger confirmation.
- Privileged helper methods must use fixed operation types and allowlisted parameters.
- Reports redact usernames, hostnames, IP addresses, serial numbers, sensitive paths, and process arguments by default.
- No telemetry, cloud account, or system-data upload.
- Do not persist live metrics unless an approved feature explicitly requires history.

## Performance rules

- Target idle RSS is 70 MB or less where feasible; 100 MB is the hard ceiling.
- Average idle CPU must remain at or below 0.5% on the reference system.
- Cold start must remain at or below 2 seconds.
- Default monitoring refresh is 2 seconds and slows when the window is hidden.
- No permanent daemon and no unsolicited filesystem scans.
- Deep storage, duplicate, SMART, and journal work runs only on request, reports progress, and supports cancellation.
- Update only affected widgets; never rebuild the whole dashboard for one metric.

## Do not touch without explicit approval

| Path or concern | Why |
|---|---|
| Privileged helper API and Polkit policy | Security boundary; changes require threat review and authorization tests |
| Shipped SQLite migrations | Immutable application history; add a forward migration |
| `Cargo.lock` by hand | Cargo-owned; change only through deliberate dependency operations |
| Packaging signing and APT repository credentials | Release secrets; never read, print, or commit them |
| CI, compiler, formatter, or lint policy | Shared project rules; explain and obtain approval before weakening them |

## Documentation

| File | Read when |
|---|---|
| `PRD.md` | Deciding scope, behavior, acceptance, priorities, or delivery phase |
| `DESIGN.md` | Changing any UI, interaction, copy, responsive behavior, theme, or accessibility behavior |
| `README.md` | Understanding product positioning, platform contract, and project status |

If implementation changes a product, design, security, performance, or platform contract, update the corresponding document in the same change.

## Definition of done

1. The smallest relevant tests pass and new behavior has a regression test that fails without the change.
2. Formatting, linting, type checking, and affected builds pass using verified repository commands.
3. Privileged or destructive behavior has authorization, validation, denial, cancellation, and partial-failure tests.
4. No new warnings, leaked sensitive data, blocking GTK-main-thread work, or unexplained capability failures appear.
5. Performance-sensitive work includes a measurement against the applicable budget.
6. User-visible behavior includes loading, unavailable, permission-required, empty, success, partial-failure, failure, and cancelled states where applicable.
7. Relevant documentation matches the shipped behavior.
