# Hardware and UI review follow-up

Initial scope: `5d33175..b2e62d8` on `release/1.3.0-readiness`.
The subsequent cleanup section records verification of `3b61032`.
This record covers local verification; the committed source is bound by its
subsequent CI manifests. It does not replace the earlier hardware measurements.

## Confirmed fixes

- Allow `lspci`/`lsusb`, use the matching tagged PCI output, and expose failures
  alongside native sysfs fallback data. Test actual parser inputs with fixtures.
  Tracked fixture names stay portable; Linux PCI slot names are created in temporary roots.
- Report unprobed acceleration as unverified. Read USB speed/descriptor maximum
  current and DMI fields instead of inventing values when data is unavailable.
- Keep Diagnostics defaults and construction pure. Mount once, collect in the
  bounded worker pool, and render loading, partial availability, and failure.
- Separate shell settings/storage/cleanup/updates/diagnostics wiring into private
  modules. Shell size changed from 2,097 to 867 lines in this revision.
- Execute theme-control and installed-policy-verifier behavior in tests. Remove
  redundant widget handles, checked-downcast panics, and repeated kernel parsing.
- Reconcile contributor rules, commands, policy ownership, and verification scope.
  CSS header corrections change comments only; layouts/tokens are unchanged.

## Local verification

Passed `cargo fmt --all -- --check`, strict workspace Clippy, the complete native
runner, all Python suites, debug build, and the private-path release wrapper.
The native runner passed 891 Rust tests; Python suites passed 24 performance,
23 release, and 38 package tests, totaling 976 tests in this local pass.
The focused hardware suite passed 12 fixtures/regressions. Its allowlist test
failed before the fix and reached the injected runner afterward.

The blocked-worker GTK fixture mounted Diagnostics in 9.14 ms and processed an
idle event in 0.85 ms while collection remained blocked. Revisiting the page
before/after completion kept the collection count at one. These are fixture
responsiveness measurements, not idle CPU/memory or cold-boot acceptance.

No dependency/lockfile, helper API/Polkit policy, shipped migration, or CI/lint
policy changed. No privileged installer ran on the personal host; verifier tests
execute controlled policy responses only. Raw output remains in ignored
`logs/2026-11-01/review-followup/`. Numerical release acceptance remains open in
[release status](../../release-status.md).

## Subsequent cleanup after `b2e62d8`

Verified source: `3b61032`. Hosted evidence is linked from
[release status](../../release-status.md); the measurements above remain tied
to the initial review.

- Validate duplicate-row widget structure and current bindings. Disable and clear
  unbound rows, ignore detached checkboxes, and release callback borrows before
  invocation so a callback can replace itself safely. Default selection stays empty.
- Replace Rust-source assertions with GTK widget, allocation, animation, and
  staged-worker tests. Keep CSS asset contract checks. Resolve speed-test `curl`
  availability only in the confirmed worker, with visible failure/cancellation.
- Split state into navigation, storage, cleanup, settings, and history modules.
  Split dispatch into storage, cleanup, diagnostics, updates, actions, and reports.
  Preserve public import paths and shared bounded transport/result accounting.

Five duplicate lifecycle regressions pass; malformed-row and callback-replacement
cases reproduced aborts before the fix. Seven focused presentation/network tests
pass. The shell allocation test checks that a requested size does not change
navigation before the surface is allocated, then verifies compact/wide transitions.
Raw logs remain ignored under `logs/2026-11-01/final-cleanup/` and the focused
`duplicates-hardening/` and `behavioral-regressions/` directories. These checks do
not close idle CPU acceptance or the remaining manual release checks.

The final cleanup pass passed 897 native Rust tests and all 85 Python tests,
plus formatting, strict workspace Clippy, debug/release builds, workflow lint,
and secret/dependency scans. Dependency audit reports the same three known
unmaintained D-Bus support crates. An initial animation test failed because its
observation window included first-frame setup under build load; the corrected
test waits for allocation before observing later movement. The full suite then
passed. No dependency, helper API, migration, or CI/lint policy changed.
