# Maintainer

Maintainer is a lightweight native GTK4 application for monitoring, maintaining, diagnosing, and understanding Ubuntu-based Linux computers.

## Stack

| Layer | Choice |
|---|---|
| Language | Rust, stable toolchain pinned in `rust-toolchain.toml` |
| UI | GTK4 only; no libadwaita, WebView, Electron, or embedded browser |
| Styling | GTK CSS with semantic tokens defined in `data/styles/` |
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

Prerequisites: `libgtk-4-dev`, `libsqlite3-dev`, `libpolkit-gobject-1-dev`, `pkg-config`. The pinned Rust toolchain is installed automatically by rustup from `rust-toolchain.toml`.

```bash
# Run the GUI application (dev mode)
cargo run -p maintainer-app

# Run the privileged helper (dev mode)
cargo run -p maintainer-helper

# Build all workspace members (debug)
cargo build --workspace

# Build all workspace members (release)
cargo build --workspace --release

# Format check (must pass before commit)
cargo fmt --all -- --check

# Lint (must pass before commit)
cargo clippy --workspace --all-targets --all-features -- -D warnings

# Test all workspace members
cargo test --workspace --all-features

# Test a single target
cargo test -p maintainer-core module::tests::test_name -- --exact

# Test the app crate only
cargo test -p maintainer-app

# Test the helper crate only
cargo test -p maintainer-helper
```

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

- Idle memory target is 150 MB PSS or less; 220 MB PSS is the hard ceiling. Absolute figures are dominated by GTK4 itself (a window with one label already costs ~167 MB RSS / ~135 MB PSS), so Maintainer's own share above that baseline is budgeted separately at 40 MB PSS or less, and that is the figure application changes can move.
- Private/dirty idle memory stays at or below 130 MB so a 4 GB system stays near 4% of total RAM without swapping.
- Average idle CPU stays at or below 0.5% as a target, with a 1.0% hard ceiling on hardware-accelerated sessions and 2.0% under a software rasterizer, where sparkline repaints cannot be offloaded to a GPU. Memory ceilings are not enforced under software rendering because no GTK4 program can meet them there.
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
| `README.md` | Understanding product positioning, platform contract, and performance budgets |
| `CHANGELOG.md` | Recording a user-visible change |
| `docs/threat-model.md` | Changing the privilege boundary, helper API, or Polkit policy |
| `tests/system/*.md` | Running the manual accessibility, compatibility, or security passes before a release |

Code is the source of truth for UI structure and styling; the CSS in
`data/styles/` defines the semantic tokens. If a change alters a product,
security, performance, or platform contract, update `README.md` and
`CHANGELOG.md` in the same change.

## Definition of done

1. The smallest relevant tests pass and new behavior has a regression test that fails without the change.
2. Formatting, linting, type checking, and affected builds pass using verified repository commands.
3. Privileged or destructive behavior has authorization, validation, denial, cancellation, and partial-failure tests.
4. No new warnings, leaked sensitive data, blocking GTK-main-thread work, or unexplained capability failures appear.
5. Performance-sensitive work includes a measurement against the applicable budget.
6. User-visible behavior includes loading, unavailable, permission-required, empty, success, partial-failure, failure, and cancelled states where applicable.
7. Relevant documentation matches the shipped behavior.
