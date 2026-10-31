# Release audit: 2026-11-01

Historical baseline audit on `release/1.3.0-readiness`; later code changes and
test results are recorded in the [review follow-up](review-followup.md).
Version 1.3.0 remains unpublished and is not release-ready: the clean hardware
sample fails CPU acceptance. See
[release status](../../release-status.md) for remaining manual checks.

## Changes

| Component | Scope |
|---|---|
| Core | Bounded jobs/streams/scans, retained collectors, private persistence and duplicate identities |
| Security boundaries | Reviewed pidfds, exact cleanup grants, quarantine/recovery and private exclusive report exports |
| Package transactions | Retained native APT plans/locks, exact approval, receipts, cancellation, inhibition and resume |
| GTK | Incremental widgets, lazy pages, background settings and authoritative confirmation |
| Packaging | Fresh exact payloads, external artifact output, corrected Debian metadata and duplicate-rule removal |
| CI | Release triggers, system Python APT tests, three full passes, pinned Actions/tools and security scans |

The audit preserved the implementation already in the working tree. No compiler,
lint, formatter or numerical budget was weakened. Shipped SQL is unchanged.

## Verification

| Evidence | Result and scope |
|---|---|
| Local format, strict workspace Clippy, debug/release builds | Passed; final binaries contain no local home paths |
| Predecessor local source-bound gate | 955 tests passed three times; exact package/source binding verified |
| SQLite follow-up | 36 persistence tests passed; resolved-feature regression failed before the fix and passed afterward |
| Multiarch follow-up | Real amd64/i386 APT/dpkg installation and final-state assertion passed locally |
| External package-output regression | Reproduced root-output failure; fixed archive/metadata suites passed all 23 tests |
| Hosted follow-up at `d3fa23a` | Full three-pass check, native candidate gate and amd64/arm64 package builds passed; later edits require their own commit checks |
| Metadata and workflow validation | Desktop/schema/AppStream and Actionlint/ShellCheck passed; AppStream has informational missing developer metadata |

Use [GitHub Actions](https://github.com/alxxrzfyr/maintainer-app/actions) for the
exact final commit's jobs and artifacts. Source-bound manifests identify the
source, lockfile and tested package/runtime bytes. Green builds do not certify
interactive authorization or hardware performance.

Two hosted failures were fixed without skipping tests. Rusqlite serialization
enabled `sqlite3_error_offset`, unavailable in Jammy's SQLite 3.37; the bounded,
padded read-only snapshot wrapper now uses the stable C ABI and system library.
Independent review found no concrete ABI/ownership defect. Multiarch setup now
writes only the disposable fixture's architecture file before exercising the
real transaction. No dependency was added and Cargo.lock was not regenerated.

## Hardware performance

[Raw measurement](hardware.json): X11/XFCE, GTK 4.14.5, confirmed hardware
rendering, normal Overview window, 30-second settling and 300.15-second sample.
No concurrent local builds or tests ran. Runtime SHA256:
`a1517af496dd38b5db8f095208faad072642417572f36b256f711ed8b9c6b277`.
This identifies the pre-review runtime. Later hardware/UI code changes require
fresh measurements; this sample does not certify them.

| Metric | Result | Assessment |
|---|---|---|
| Average CPU | 0.6296772984% | Fails 0.35% preferred target and strictly below 0.5% fallback |
| Usable startup | 1,870.11 ms | Within 2,000 ms; fresh process, not cold boot |
| Peak PSS | 74,650,624 bytes | Below 100,000,000-byte milestone |
| Peak private dirty | 33,456,128 bytes | Below 130,000,000-byte limit |
| Swap/SwapPss | 0 bytes | No observed swapping |

An earlier candidate failed with 0.69960% CPU and 3,331 ms startup. A later
concurrent-probe diagnostic recorded 0.45644% CPU; it does not replace the clean
failed result. The interrupted page matrix is incomplete, not an application
crash or pass. Cold boot, repeated/full-page/hidden/scale/after-job/soak checks
remain unverified. [October results](../2026-10-31/report.md) retain earlier failures.

## Public repository security

Gitleaks and independent tree/history scans found no tested credential/private-key
patterns. New commits use GitHub noreply addresses. Release-path remapping fixes
the observed embedded-home-path disclosure. Raw [local logs](../../../logs/README.md)
are ignored; packages stay outside the checkout. No signing keys, private
environment files or global skills were inspected. No host package was installed.

Cargo-audit 0.22.2 reviewed 174 dependencies against the 2026-10-30 RustSec database
and found zero known vulnerabilities, with three unmaintained-crate notices:
[zvariant](https://rustsec.org/advisories/RUSTSEC-2026-0370.html),
[zbus_names](https://rustsec.org/advisories/RUSTSEC-2026-0371.html),
[zvariant_derive](https://rustsec.org/advisories/RUSTSEC-2026-0372.html).
The static audit tool supports Jammy; tool downloads are checksum-pinned.

Review covered caller-bound authorization, descriptor identity, native plan
binding, authenticated archives, private exports and database recovery. Fixtures
cover denial, races, disconnects, cancellation and partial failure. Installed
interactive Polkit, effective sandbox, accessibility and full privacy acceptance
remain open. This is not formal independent security certification.

## Cleanup

Removed duplicate `packaging/debian/compat` and `install` declarations and the
unused GResource manifest. Consolidated 15 draft/profile files totaling 157,801
bytes; representative failed measurements remain. Four plan/research documents
moved into the unchanged-content historical archive. Current evidence is grouped
by date; raw logs stay private. No release tag, signing or APT publication occurred.
