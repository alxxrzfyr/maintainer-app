# 1.3.0 release audit

Audit date: 2026-11-01 in the desktop session's local timezone.
Branch: `release/1.3.0-readiness`, based on `5e2c0ca` and the existing uncommitted
implementation. Version 1.3.0 remains unpublished. This record distinguishes
local observations, hosted automation and remaining manual acceptance.

## Changes under review

| Component | Cohesive changes |
|---|---|
| Core services | Bounded jobs/streams/scans, retained collectors, private persistence/recovery, duplicate identities and redacted exclusive exports |
| Cleanup and helper | Exact descriptor-based reviews, protected quarantine, connection-bound batch authorization and collision-preserving recovery |
| Package transactions | Native retained APT plans/locks, exact approval, private receipts, inhibitor lifetime, cancellation and resume/reconciliation |
| GTK application | Lazy pages, stable widget identities, settings workers, retained process review and authoritative package confirmation |
| Build and packaging | Locked private-path release builds, fresh exact-payload staging, corrected Debian prerequisites and removal of duplicate declarations |
| Automation | Release-branch triggers, system Python APT tests, three full passes, pinned Actions, secret/advisory/workflow checks and source-bound artifacts |

The implementation already existed in the working tree when this audit began.
The release work preserves it and adds the demonstrated build/privacy/CI fixes
and cleanup. Compiler/toolchain, formatting/lint rules, numerical budgets and the
shipped SQL migration are not weakened or rewritten.

## Local evidence

| Check | Observed result |
|---|---|
| `cargo fmt --all -- --check` | Passed |
| `cargo clippy --locked --workspace --all-targets --all-features -- -D warnings` | Passed at baseline; final source gate repeats it |
| `bash scripts/test-native.sh` | 871 Rust tests passed, including separately executed GTK fixtures and app integration targets; no ignored cases left unexecuted |
| Python suites with `-W error::ResourceWarning` | Initial performance/release/package suites passed 24/16/38 tests; expanded release suite passed 22 tests after CI/build regressions |
| Debug and remapped release builds | Passed with the pinned Rust 1.85.0 toolchain and locked dependencies; both release binaries contain zero local home-path occurrences |
| Desktop entry, GSettings schema, AppStream | Passed; AppStream reports informational missing developer metadata |
| Actionlint 1.7.12 with ShellCheck | Passed |
| Gitleaks 8.30.1 | Zero findings in reachable history and exported candidate source; matched values are redacted |
| Source security review | No concrete high-confidence privilege-escalation, unauthorized deletion or arbitrary-command defect found in reviewed boundaries |

The release-trigger regression failed before the workflow fix and passed after
it. Release-path tests exercise rustc itself, compiler-flag preservation and
rejection of binaries containing private build prefixes. Final candidate builds
also inspect the actual GUI/helper bytes, not only mocked compiler settings.

Run `scripts/check-release-local.py build` and `verify` after all edits. The
external manifest records the exact final source/package/runtime binding and
three full test passes. A preceding test run cannot certify later edits. Local
logs and temporary profiler/build products remain outside the repository.

## Performance

The pre-remapping candidate's normal Overview window was measured for 300.17
seconds after 30 seconds of settling on the existing X11/XFCE desktop, GTK
4.14.5 and confirmed hardware rendering. No concurrent build or test workload
ran during the sample. Its GUI SHA256 was
`8074df0fad2f10c0459dab1f8ccd0759e4b311e265f1f87358e61a51a1bc3785`.

| Metric | Measurement | Acceptance |
|---|---|---|
| Average process-family CPU | 0.6995969012% | Failed both 0.35% target and strictly below 0.5% fallback |
| Usable window | 3,331.33 ms | Failed 2,000 ms requirement; fresh process, not cold boot |
| Peak combined PSS | 95,110,144 bytes | Below 100,000,000-byte milestone in this sample |
| Peak private dirty | 52,641,792 bytes | Below 130,000,000-byte limit |
| Swap/SwapPss | 0 bytes | No swapping observed |

Build-path remapping changes artifact hashes. This earlier measurement is not a
claim about later binaries. Any final measurements must bind the remapped runtime
hash and retain failures. Full page repetitions, hidden/scale behavior, cold boot,
main-loop latency, after-job idle and extended-soak acceptance remain open unless
separate evidence explicitly records them. Green hosted jobs cannot waive these
hardware requirements.

## Public repository review

The RustSec database contained 1,395 advisories, at commit
`2295466c4e9df26647fd67d8fa1c705ac2fc7f81`, updated 2026-10-30. Cargo-audit
0.22.2 reviewed 174 lockfile dependencies and found zero known vulnerabilities.
It reported unmaintained `zvariant` 4.2.0,
[`RUSTSEC-2026-0370`](https://rustsec.org/advisories/RUSTSEC-2026-0370.html),
`zbus_names` 3.0.0,
[`RUSTSEC-2026-0371`](https://rustsec.org/advisories/RUSTSEC-2026-0371.html), and
`zvariant_derive` 4.2.0,
[`RUSTSEC-2026-0372`](https://rustsec.org/advisories/RUSTSEC-2026-0372.html).
These are maintenance notices, not a zero-warning audit. Migration needs a
separate compatibility review; this audit adds no application dependency and does
not regenerate the lockfile.

The independent pattern scan covered 563 reachable historical blobs across 60
commits and the candidate tree. Tested credential formats, private-key headers and
credential-bearing database URLs had no matches. Documentation contained no
personal home paths. Release binaries initially did contain local build paths;
release remapping and actual-binary validation address that observed disclosure.
New commits use the repository owner's GitHub noreply address. Workflow tokens
come from GitHub's runtime store; there is no published credential or signing key.
Audit tools and Actions are pinned, tool archives are checksum-verified, workflow
permissions are read-only, and secret-scanner matches are redacted. The audit tool uses its static musl build because the GNU artifact requires glibc 2.39, newer than the Ubuntu 22.04 floor. No global
skills, signing credentials or private environment files were inspected.

The review traced system-bus caller authorization, single-use cleanup grants,
quarantine identity checks, retained native package plans and authenticated archive
checks, report publication and private database recovery. Native and Python
fixtures exercise denial, races, disconnects, cancellation, partial failure and
recovery in disposable directories. These observations are not formal independent
security certification. Interactive installed Polkit and effective sandbox/manual
privacy checks remain open.

## Cleanup

| Removed or consolidated | Reason and retained evidence |
|---|---|
| `packaging/debian/compat` | Duplicate of `debhelper-compat (= 13)` |
| `packaging/debian/install` | Duplicated explicit installation rules and an inconsistent GUI binary destination |
| `data/resources.gresource.xml` | Unreferenced resource manifest; the application loads native CSS files directly |
| 15 untracked validation drafts/JSON profiles, 157,801 bytes | Consolidated into the [October summary](2026-10-31-summary.md); representative complete/failed measurements remain |
| Four old plan/research locations | Moved to the historical archive with provenance; historical content remains available |
| Current release status | Replaced repeated experiment narratives with one status page and evidence links |

Active parsers/fixtures, system procedures, licenses, source, policies, toolchains
and historical archive content are preserved. No host package was installed or
removed. The local root-launch smoke could not run because passwordless sudo is
unavailable; the installed disposable-runner test checks root refusal explicitly.

## Hosted checks and release decision

The branch's [Actions runs](https://github.com/alxxrzfyr/maintainer-app/actions)
are the source for exact pushed-commit results: CI check, native amd64/arm64
packages, installed upgrade/readiness/removal/purge, security/workflow audit and
the native candidate gate. Artifact manifests and checksums bind tested bytes.
No release tag, published release, signing key or APT deployment is part of this
branch preparation. Passing source/build/hosted tests alone is insufficient for
release approval while performance and manual acceptance remain unresolved.

## Hosted compatibility follow-up

The first pushed candidate failed to link on Ubuntu 22.04 because rusqlite's
`serialize` feature enabled `sqlite3_error_offset`, introduced after Jammy's
SQLite 3.37. Removing that feature and isolating the stable snapshot C ABI
preserves system SQLite, padded read-only recovery and exclusive private export.
The resolved-feature regression failed before the fix; 36 focused persistence
tests passed afterward. Independent review found no concrete ABI, ownership or
buffer-safety defect. Cargo.lock was not regenerated. Hosted results for the
follow-up commit determine whether the compatibility failure is resolved.
The interrupted performance matrix belongs to the preceding binary and is not
a complete measurement or an application crash.
