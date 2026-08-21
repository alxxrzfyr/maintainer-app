# Security Review Checklist

Status: Template — complete before v1 release.

## Test environment

- Distribution: ___
- Kernel: ___
- PolicyKit version: ___
- D-Bus version: ___
- Reviewer: ___
- Date: ___

## Privileged helper authorization tests

| Test case | Pass | Notes |
|---|:---:|---|
| Helper refuses operation without Polkit authentication | [ ] | |
| Helper refuses operation from unauthorized caller (wrong bus name) | [ ] | |
| Helper allows operation after valid Polkit authentication | [ ] | |
| Helper denies operation when user clicks "Cancel" in Polkit dialog | [ ] | |
| Helper refuses unrecognized operation type | [ ] | |
| Helper logs authorization attempts | [ ] | |
| Helper times out idle connections | [ ] | |
| Each method has a separate Polkit action ID | [ ] | |

## Path and PID validation

| Test case | Pass | Notes |
|---|:---:|---|
| Helper rejects path traversal (../) in cleanup targets | [ ] | |
| Helper rejects symlinks pointing outside allowed trees | [ ] | |
| Helper rejects paths containing null bytes | [ ] | |
| Helper canonicalizes paths before operation | [ ] | |
| Path is re-validated between user review and execution | [ ] | |
| Process termination re-checks PID identity before signal | [ ] | |
| Terminated PID must match the confirmed process name/start-time | [ ] | |
| No TOCTOU gap between display and confirmed action | [ ] | |

## Command allowlist and execution safety

| Test case | Pass | Notes |
|---|:---:|---|
| Helper only executes fixed operation types (no arbitrary commands) | [ ] | |
| APT operations use fixed argument arrays, not shell strings | [ ] | |
| No `sh -c`, `bash -c`, or equivalent shell invocation in helper | [ ] | |
| External utilities receive explicit argv, never interpolated user input | [ ] | |
| Update helper accepts only allowlisted package operations | [ ] | |
| Cleanup helper only targets approved cache/trash directories | [ ] | |
| Helper refuses operation on paths outside its allowlist | [ ] | |

## Report redaction

| Test case | Pass | Notes |
|---|:---:|---|
| Username replaced with placeholder in default report | [ ] | |
| Hostname replaced with placeholder in default report | [ ] | |
| IP addresses (v4 and v6) replaced with placeholder | [ ] | |
| Serial numbers redacted from SMART data in report | [ ] | |
| Sensitive file paths redacted (home directory contents) | [ ] | |
| Process arguments redacted (may contain secrets) | [ ] | |
| MAC addresses redacted | [ ] | |
| Redaction is the default; user must opt-in to unredacted | [ ] | |

## Network and telemetry

| Test case | Pass | Notes |
|---|:---:|---|
| Application makes no network connections at startup | [ ] | |
| Application makes no network connections during monitoring | [ ] | |
| No DNS queries to analytics/telemetry domains | [ ] | |
| No crash reporting to external services | [ ] | |
| Only APT/PackageKit/fwupd backends make network requests | [ ] | |
| Network requests occur only when user explicitly initiates updates | [ ] | |
| No embedded tracking pixels, fonts, or CDN resources | [ ] | |

## GUI privilege boundary

| Test case | Pass | Notes |
|---|:---:|---|
| GUI refuses to launch when run as root (uid 0) | [ ] | |
| GUI displays error message explaining why root is refused | [ ] | |
| GUI never stores or caches admin credentials | [ ] | |
| GUI delegates all privileged work to helper via D-Bus | [ ] | |
| GUI cannot bypass Polkit to perform privileged operations | [ ] | |
| GTK application runs entirely in user session | [ ] | |

## Data safety

| Test case | Pass | Notes |
|---|:---:|---|
| Cleanup operations require explicit user confirmation | [ ] | |
| Duplicate deletion never auto-selects files | [ ] | |
| Process termination requires confirmation dialog | [ ] | |
| Permanent deletion requires stronger confirmation than Trash | [ ] | |
| Database corruption does not crash the application | [ ] | |
| Database stores no live metrics (settings and history only) | [ ] | |
| Application data directory has user-only permissions | [ ] | |

## Dependency audit

| Check | Pass | Notes |
|---|:---:|---|
| `cargo audit` reports no known vulnerabilities | [ ] | |
| No dependencies with unmaintained status | [ ] | |
| All dependencies pinned in Cargo.lock | [ ] | |
| No unnecessary network-capable dependencies | [ ] | |
| Build dependencies do not execute arbitrary download scripts | [ ] | |

## Sign-off

| Reviewer | Date | Critical findings | Result |
|---|---|---|---|
| ___ | ___ | ___ | ___ |
