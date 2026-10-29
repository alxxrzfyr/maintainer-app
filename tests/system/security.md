# Security validation procedure

Status: reusable procedure; no tests are marked passed. Revised 2026-10-30 for the [current threat model](../../docs/threat-model.md) and [roadmap](../../docs/plans/development-roadmap.md).

Run privileged/destructive cases only in a disposable VM with a recoverable snapshot, a nonroot desktop account and a real system bus/Polkit agent. Install the exact candidate .deb. Use owned test files/processes. Ordinary unit tests must use injected adapters and must not contact the developer host's live helper.

## Record a result

Create a dated result under `docs/validation/` only after testing. Record package SHA-256, commit, dirty state, distro/kernel/GTK, display/renderer, Polkit version and effective rules, account/session class, operator and date. Per case record command/action, expected result, actual result and sanitized evidence. Use `pass`, `fail`, `unverified`, or `not applicable` with a reason. Missing services or tools do not count as a pass.

## Identity, authorization and packaging

| Case | Required observation |
|---|---|
| Root GUI refusal | In the disposable VM, invoke without a display; app exits before window, DB or helper access with an explanation. Missing guard is a failure, not a skipped check. |
| Service ownership | Installed helper, unit, bus policy and Polkit action files are root-owned and not user-writable; a normal user cannot acquire the helper service name. |
| Sender identity | No request payload can impersonate another bus caller. Disconnected/unresolvable callers are refused before mutation. A different valid UID is judged by policy, not assumed automatically unauthorized. |
| Each protected action | Separately exercise terminate, vacuum, kernel removal and install-updates: allow, deny, dismissed prompt, missing agent/authority and repeated invocation. Record host policy overrides. |
| Active-session exceptions | Cache clean/SMART use their shipped active-session policy; no password prompt may be expected there. Test inactive/other-session behavior separately. |
| Installed upgrade | Matching unit/bus/Polkit files load after package upgrade; exact helper path/action mapping is checked. Exercise both D-Bus maintenance and pkexec update entrypoints. |

## Parameters, lifetime and data integrity

| Case | Required observation |
|---|---|
| Malformed wire/CLI requests | Invalid signatures/methods, empty/oversized strings, package-option/path injection and numeric boundaries cause no mutation. Measure memory/timeout behavior; do not infer safety from a transport-size default. |
| Process identity | Test the installed helper flow and any retained core API: PID 0/1, out-of-range PID, absent PID, zero/stale start time, exit/reuse between stages, read/syscall denial and exhausted FDs affect no unrelated process. Only SIGTERM is available; delivery is not labelled confirmed exit. |
| Kernel protection | Reject running kernel and resolver removals of its components. Remove a reviewed old kernel only in the VM; check dpkg/boot state and reboot the retained kernel. A sandbox refusal is a failed capability test. |
| Review binding | Selected/full intent, changed versions/dependencies/removals and held locks are handled without unreviewed changes. Do not delete APT locks or kill their owners. |
| Cancellation/disconnect | Test before elevation, before mutation, during configuration and after mutation. Add logout, owner restart, duplicate start request, shutdown inhibition/override and VM power interruption. UI says cancelled only when the owner confirms it; unknown outcomes reconcile before retry. |
| Partial results | Inject failed package configuration, child failure, disk full and history-write failure. Preserve successful item outcomes; identify recovery-required/unknown states and prevent blind replay. |
| Cleanup substitution | Swap file/ancestor/symlink between review and action; add/remove directory children. No out-of-scope target changes. Reject duplicate/overlapping selections. |
| Trash | Test hostile symlink/owner/permissions, custom XDG path, both mount layouts, simultaneous same-name moves, EXDEV and ENOSPC. Never overwrite an existing entry or permanently delete without stronger confirmation. |
| Report saving | Precreate the temporary name as a symlink and as a permissive file; test simultaneous saves and failed rename. Unrelated files remain unchanged and new reports remain private. |
| Flood/stuck operations | Saturate helper and app queues, generate verbose output and hold pipes open. Observe bounded resources, denial/busy states and capacity recovery; test actual installed sandbox behavior. |
| Retained results/scans | Hold completed receivers without consuming them, then submit more work. Include speed tests; verify the global limits. Deep/wide trees, long names and many errors must stop with visible incomplete coverage within the approved scan budget. |
| Duplicate evidence | Known equal files are found through app dispatch, including below the top-200 large-file ranking; hard links are not counted as extra recoverable copies. Changed files and partial scans cannot authorize a stale removal plan. |

## Privacy and evidence

| Case | Required observation |
|---|---|
| Default report | Seed username, hostname, IPv4/IPv6, MAC, serial, custom-home/sensitive paths and arguments without ` -- ` in typed fields, service descriptions, event templates and capability reasons. Preview, clipboard and file contain no seeded sensitive value and use the same preview revision. |
| Errors/audit | Repeat seeded failures through D-Bus, CLI, GUI, history and report export. Record leakage as a failure; do not assume utility output is sanitized. Check the approved audit schema without storing credentials or process arguments. |
| Passive network | Capture startup and monitoring separately with no user action. No application-triggered external service request; distinguish unrelated OS traffic. |
| Explicit network actions | Deny speed-test confirmation: no request. Accept: only the fixed service/test payload is used; provider/data use is disclosed and results are not persisted. Updates may use configured repositories; no system report is uploaded. |
| Persistence | Settings/history only; no live-metric persistence. Test unset/empty/relative/custom XDG paths, substituted DB symlink, existing permissive directory, DB/journal/WAL/SHM modes, disk full, corruption/newer schema, interrupted migration and backup restore. Preserve user data on failure; logical deletion is not secure erasure. |
| Dependency review | Record the approved advisory tool/database date and lockfile reviewed; triage vulnerable/unmaintained entries with an owner. If tooling is absent, mark unverified; do not install it or regenerate Cargo.lock silently. |

## Release sign-off

The reviewer records passed/failed/unverified case names, related T IDs, open defects, disabled capabilities and any explicit scope approval. Link evidence to D2/D5-D8/D11-D13 and the relevant A1-A3 decision. Blank procedures, mock tests and policy-string assertions cannot substitute for installed tests. No unresolved release blocker may be labelled passed.
