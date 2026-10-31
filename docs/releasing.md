# Release guide

Pushing a stable version tag such as `v1.3.0` runs CI and publishes its GitHub
Release after every build and installed-package check passes. Publishing a tag
is therefore an instruction to release that exact commit.

The release contains four uploaded files: amd64 and arm64 Debian packages, plus
one checksum file per architecture. GitHub adds ZIP and TAR.GZ source archives,
giving six downloads. Packages are reused from the verified workflow artifacts;
the publishing job never rebuilds them.

Current scope and open checks live in [release status](release-status.md).
The commands below use `v1.3.0` as the planned stable release. Run Linux build
and artifact commands in a Linux environment; Git tagging commands also work
in PowerShell.

## Prepare the release commit

Complete the acceptance work recorded in release status, including performance
measurements and manual system checks. Green CI covers its automated cases.
Preserve historical failed measurements and record new evidence separately.

Align these files before the final candidate build:

| File | Stable 1.3.0 value |
|---|---|
| `crates/maintainer-{app,core,helper}/Cargo.toml` | `version = "1.3.0"` |
| `Cargo.lock` | The three workspace packages remain `1.3.0` |
| `packaging/debian/changelog` | `1.3.0-1`, distribution `jammy`, actual release date |
| `data/io.github.maintainerapp.Maintainer.metainfo.xml` | `1.3.0`, `type="stable"`, actual release date |
| `CHANGELOG.md` | Confirm the `1.3.0` date and version links |
| `docs/releases/1.3.0.md` | Final release announcement; remove the draft notice |
| `README.md` and `docs/release-status.md` | Accurate status, evidence, and any remaining limitations |

Do not edit `Cargo.lock` by hand. Current crate versions already match; no
dependency update or lockfile regeneration is needed for release preparation.
Keep Debian `UNRELEASED`, AppStream `development`, and draft notices until the
stable acceptance decision is supported.

Use the [candidate commands](../README.md#build-a-candidate) after metadata is
final. Documentation and package metadata changes invalidate earlier source-bound
candidate manifests. Build a fresh candidate outside the checkout.

Commit the completed preparation, merge the release PR into `main`, and wait
for the merged commit's CI and security audit to pass. A PR run may test a merge
ref; confirm the final `main` push run and its exact SHA.

## Create and push an annotated tag

Run from a clean working tree after the verified release commit reaches `main`:

```bash
git switch main
git pull --ff-only origin main
git status --short
git log -1 --oneline
git tag -a v1.3.0 -m "Maintainer 1.3.0"
git show --no-patch v1.3.0
git push origin v1.3.0
```

The tag points to the checked-out commit. Confirm it is the same SHA verified
on `main`, and push only this tag. If `v1.3.0` already exists, stop and inspect
it; published tags must not be moved or force-pushed.

The tag triggers `.github/workflows/ci.yml`. Its publishing job waits for the
existing checks and both architectures' installed-package tests. It verifies
the remote tag, source commit, workflow/run attempt, package versions,
architectures, lockfile bytes, and checksums before uploading.

The job creates a draft first, uploads and verifies all four files, then
publishes it as the latest release. Repeating publication accepts identical
files and fills missing uploads; different or unexpected assets fail validation
without being replaced. Upgrade tests select the newest earlier stable release,
so reruns still exercise an upgrade after this version is published.

Only the publishing job has release-write permission. Branch and pull-request
runs cannot publish. Tags must use `vMAJOR.MINOR.PATCH`, and the tagged commit
must belong to `main`. Automated publication does not mark unmeasured manual or
performance acceptance as passed.

## Publish an existing tag

A tag created before this publishing job was added still uses its original
workflow. Keep that tag unchanged and run the recovery option from `main`:

1. Open **Actions → CI → Run workflow** and select branch `main`.
2. Set `release_tag` to the existing tag, such as `v1.3.0`.
3. Set `candidate_run_id` to a successful **main push** CI run for the exact
   tagged commit. For the original 1.3.0 merge, this is `41240057429`.
4. Run the workflow. Recovery validates that run's completed tests, source, and
   installed-tested artifacts, then publishes those exact files.

Recovery skips rebuilding because it consumes an already successful run. A
failed, foreign, PR-only, or mismatched run is rejected. If the artifacts have
expired, obtain a fresh complete verification run rather than replacing the
tag or using unrelated package bytes.

## Inspect or download packages manually

From the successful final `main` push CI run, download both
`maintainer-validated-amd64` and `maintainer-validated-arm64`. Use the artifacts
that passed installed lifecycle tests, rather than rebuilding release binaries.
With GitHub CLI installed, replace `MAIN_CI_RUN_ID` with that run's numeric ID:

```bash
gh run download MAIN_CI_RUN_ID --repo alxxrzfyr/maintainer-app --name maintainer-validated-amd64 --dir /tmp/maintainer-release-1.3.0/amd64
gh run download MAIN_CI_RUN_ID --repo alxxrzfyr/maintainer-app --name maintainer-validated-arm64 --dir /tmp/maintainer-release-1.3.0/arm64
```

Choose a new output directory outside the checkout. Verify each architecture's
checksum file from inside its directory:

```bash
(cd /tmp/maintainer-release-1.3.0/amd64 && sha256sum --check SHA256SUMS-amd64)
(cd /tmp/maintainer-release-1.3.0/arm64 && sha256sum --check SHA256SUMS-arm64)
```

Confirm both `SOURCE-*.json` manifests name the tagged commit, the intended CI
run, matching lockfile hashes, and exact package hashes. The annotated tag can
be resolved to its commit with `git rev-parse "v1.3.0^{commit}"`.
`scripts/validate-candidate.py` provides source/tag/artifact binding checks.
Recovery requires a successful `main` push run; automatic publishing validates
the active tag run after its prerequisite jobs finish. Preserve the
manifests and lockfile with the reviewed validation evidence.

## Manual publication fallback

Use the existing tag, the finalized versioned release notes, and the exact
validated packages and checksum files:

```bash
gh release create v1.3.0 /tmp/maintainer-release-1.3.0/amd64/maintainer_1.3.0-1_amd64.deb /tmp/maintainer-release-1.3.0/arm64/maintainer_1.3.0-1_arm64.deb /tmp/maintainer-release-1.3.0/amd64/SHA256SUMS-amd64 /tmp/maintainer-release-1.3.0/arm64/SHA256SUMS-arm64 --repo alxxrzfyr/maintainer-app --verify-tag --title "Maintainer 1.3.0" --notes-file docs/releases/1.3.0.md
```

`--verify-tag` requires the tag to exist remotely. This command publishes the
release; use `--draft` when preparing an unpublished GitHub preview. A Git tag
and a GitHub Release are separate objects.

After automatic or manual publication, verify both downloads and checksums, record the release URL,
and update release status/README with the actual publication result. APT repository
signing and publication are separate operations; these steps do not access keys
or publish an APT repository.
