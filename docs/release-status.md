# Release status

Version 1.3.0 is an unpublished development candidate on
`release/1.3.0-readiness`. It is not release-ready while performance acceptance
and the remaining manual checks are unresolved. No release tag or APT
publication is authorized by this branch preparation.

## Candidate scope

The candidate includes bounded workers, commands and scans; incremental GTK
updates; private persistence and report export; retained-pidfd process review;
identity-checked regular-file cleanup with quarantine/recovery; and native APT
transactions with exact plan approval, private receipts, cancellation boundaries,
shutdown inhibition and reconnection. Duplicate files are never deleted
automatically. Helper operations retain fixed types and arguments.

Cleanup is limited to caller-owned single-link regular files on the same supported
filesystem. Raced replacements are restored or retained, never deleted. No content
snapshot or physical-space guarantee is made. Two retained kernel images protect
against removing every fallback; they are not proof of bootability.

Release builds now use `python3 scripts/build-release.py` to preserve compiler
settings while remapping local build paths. The build rejects a resulting GUI or
helper containing the checkout or Cargo-home prefix. CI uses locked dependencies,
three complete native/Python passes, native amd64/arm64 packaging and disposable
installed-upgrade checks. Separate jobs audit public history/dependencies/workflows
and bind a native candidate to source and exact package bytes.

## Evidence and acceptance

The [November audit](validation/2026-11-01-release-audit.md) records concrete
commands, observations, cleanup and review limitations. Earlier failed experiments
are consolidated in the [October summary](validation/2026-10-31-summary.md), with
representative raw records retained. Historical counts and hashes do not prove
that a later revision passes.

The preferred average idle CPU target is at most 0.35%. The approved fallback is
strictly below 0.5%; exactly 0.5% fails. Usable startup must be at most 2 seconds.
The README's next memory milestone is below 100,000,000 bytes combined idle PSS;
older GTK baseline and ceiling comparisons remain diagnostic context. No budget
was relaxed to obtain green CI.

The source-bound local gate must run after all source/documentation edits:

```bash
python3 scripts/check-release-local.py build /tmp/maintainer-candidate-new
python3 scripts/check-release-local.py verify /tmp/maintainer-candidate-new
```

Its external `LOCAL-CANDIDATE.json` binds source, lockfile, three full test passes
and exact package/runtime bytes. It performs static archive inspection without
installing on the personal host. CI/check results must be read for the exact
pushed commit; [the Actions page](https://github.com/alxxrzfyr/maintainer-app/actions)
contains those independent results. Green hosted tests do not certify hardware
performance or interactive authorization.

## Latest local hardware result

The [November hardware sample](validation/2026-11-01-hardware.json) recorded
0.62968% CPU over 300 seconds, failing the 0.35% preferred target and the strictly
below 0.5% fallback. Usable startup was 1,870 ms, peak PSS 74,650,624 bytes and
private dirty 33,456,128 bytes. Those limits passed in this one hardware sample;
cold boot, other pages and the wider protocol remain unverified.

## Remaining release work

- Meet CPU/startup acceptance in valid measurements; repeat cold-boot, full-page,
  hidden/scale, after-job and extended-soak checks. A single hardware page sample
  cannot certify the complete performance protocol.
- Complete installed interactive Polkit allow/deny/dismiss, missing-agent,
  effective sandbox, reboot/kernel and interruption/recovery checks on a
  disposable system. Automated installed upgrade results cover their recorded
  cases only.
- Complete keyboard, screen-reader, theme/contrast, scaling and comprehensive
  privacy validation. Fixture assertions are not manual certification.
- Review migration away from the unmaintained D-Bus support crates identified by
  RustSec. No speculative dependency/toolchain upgrade is included in this audit.

The earlier user waiver of additional machines and third-party verification does
not turn unmeasured behavior into a pass. The [archive](archive/README.md) preserves
historical plans and research. Current behavior comes from code, repository policy
from [AGENTS.md](../AGENTS.md), and product/release contracts from
[README.md](../README.md) and [CHANGELOG.md](../CHANGELOG.md).
