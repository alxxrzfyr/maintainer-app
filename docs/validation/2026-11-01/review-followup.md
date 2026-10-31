# Hardware and UI review follow-up

Scope: the review changes after `5d33175` on `release/1.3.0-readiness`.
This record covers local verification; the committed source is bound by its
subsequent CI manifests. It does not replace the earlier hardware measurements.

## Confirmed fixes

- Allow `lspci`/`lsusb`, use the matching tagged PCI output, and expose failures
  alongside native sysfs fallback data. Test actual parser inputs with fixtures.
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
