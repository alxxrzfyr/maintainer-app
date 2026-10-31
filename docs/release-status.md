# Release status

Version 1.3.0 remains an unpublished development candidate. Release acceptance
is incomplete: the last clean hardware sample fails CPU acceptance, and the
manual checks below remain open. Builds and CI do not publish a release.

## Current scope

Code implements bounded jobs/commands/scans, incremental GTK updates, private
persistence and report export, retained-pidfd process review, protected regular-file
cleanup/recovery, and retained native APT plans with exact approval and receipts.
See the [threat model](threat-model.md) for security controls and limits.

The review follow-up addresses hardware command validation, evidence-based
hardware values, background Diagnostics collection, feature-specific shell wiring,
and behavioral tests. Subsequent cleanup checks duplicate-row lifecycles, defers
speed-test capability probes, and groups state/dispatch by feature. The
[review follow-up](validation/2026-11-01/review-followup.md) records local results.
Commands and layout are described in [README](../README.md);
contributor rules and numerical acceptance live in [AGENTS.md](../AGENTS.md).

## Verification evidence

The latest recorded implementation revision, `3b61032`, passed all seven
push-triggered hosted checks:

- [CI](https://github.com/alxxrzfyr/maintainer-app/actions/runs/41236056206):
  three complete test passes, release build, native amd64/arm64 packages, and
  disposable installed upgrade/readiness/root-refusal/removal/purge.
- [Native candidate gate](https://github.com/alxxrzfyr/maintainer-app/actions/runs/41236056201):
  three full passes, debug/release builds, metadata, and exact source/package binding.
- [Security audit](https://github.com/alxxrzfyr/maintainer-app/actions/runs/41236056207):
  secret scanning, dependency advisories, and workflow/shell validation.

Its local cleanup pass also passed 897 Rust and 85 Python tests.
Those runs cover that revision, including its documentation and payloads.
Subsequent edits require fresh checks. Read the exact current commit's
results on [Actions](https://github.com/alxxrzfyr/maintainer-app/actions).
The [dated audit](validation/2026-11-01/report.md) records the baseline's scope;
[October evidence](validation/2026-10-31/report.md) preserves earlier experiments.
Raw local logs are private and [ignored by Git](../logs/README.md).

## Last clean hardware sample

The [recorded measurement](validation/2026-11-01/hardware.json) used runtime
`a1517af496dd38b5db8f095208faad072642417572f36b256f711ed8b9c6b277`, hardware
rendering, 30-second settling, and a 300-second Overview sample:

- Average CPU: 0.62968%, failing the accepted fallback.
- Usable startup: 1,870 ms.
- Peak PSS: 74,650,624 bytes. Peak private dirty: 33,456,128 bytes.
- Swap/SwapPss: zero. Startup and memory passed their limits in this sample.

This is evidence for that runtime, not a fresh measurement of later code.
Acceptance limits are defined once in [performance rules](../AGENTS.md#performance-rules).

## Remaining acceptance

- Meet CPU acceptance and repeat valid cold-boot, full-page, hidden/scale,
  after-job, and extended-soak measurements for the final runtime.
- Complete installed interactive Polkit allow/deny/dismiss, missing-agent,
  effective sandbox, kernel/reboot, and interruption/recovery checks on a
  disposable system. Automated package lifecycle tests cover only their cases.
- Complete keyboard, screen-reader, theme/contrast, scaling, and full privacy
  validation. Fixture assertions are not manual certification.
- Review migration from the unmaintained D-Bus support crates reported by RustSec.

Use the [candidate commands](../README.md#build-a-candidate) after all release
inputs are final. Their external manifest binds source, lockfile, three full test
passes, and package/runtime bytes; it does not install on the personal host.

[Archived plans](archive/README.md) are historical records. Their status labels
neither complete nor waive unfinished work. Green CI does not waive performance
or manual acceptance.
