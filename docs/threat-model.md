# Maintainer threat model

Reviewed: 2026-10-30 against `07f7b9a096c3a84b57b329fca5e4d4d2d93289c1` (1.2.0).
Status: source review, not a completed security audit or installed-system test.
See the [roadmap](plans/development-roadmap.md), [safety decisions](plans/safety-design-decisions.md), [research](research/2026-10-30-engineering-review.md), and [security procedure](../tests/system/security.md).

## Scope and trust

Protect system integrity, user files, diagnostic privacy and desktop responsiveness. Treat callers, request parameters, filesystem contents, external-program output and exported text as untrusted inputs. Distribution tools, D-Bus, Polkit, the kernel and correctly installed root-owned files form the trusted computing base; their presence is not proof that Maintainer uses them safely.

The desktop app is intended to run as the logged-in user. Its entrypoint currently lacks a root-refusal guard. Cleanup and report saving run with the app's permissions. UI confirmation limits accidental actions through this app; it does not authenticate arbitrary D-Bus callers or protect against an attacker controlling the app's own memory.

The service accepts authorized callers, not one hardcoded UID or verified GUI executable. UID 1000 is an example, not an access rule. Existing root compromise and a malicious administrator changing policy are outside this model. Untrusted package maintainer scripts and compromised repositories require distribution-level controls; fixed argv does not sandbox those scripts.

## Current operation boundary

| Entry / operation | Control visible in source | Limit or unresolved evidence |
|---|---|---|
| D-Bus: terminate process | Bus sender resolution, operation-specific Polkit check, PID/start-time checks, SIGTERM only | Numeric kill follows validation; final PID race remains |
| Exported core termination function | Rechecks PID/start time; returns permission-required on signal denial | Same numeric-PID race; read failures collapse to absence. No app callsite; the GUI uses the helper |
| D-Bus: clean package cache | Fixed `/usr/bin/apt-get clean`; bounded request ID | Installed sandbox behavior unverified; no arbitrary path argument |
| D-Bus: vacuum journal | Fixed journalctl; size 100..4096 MiB, age 1..365 days | Destructive; bounds do not preserve a guaranteed amount of diagnostic history |
| D-Bus: refresh SMART | Validated drive ID; fixed UDisks2 object/interface/method | Separate service is trusted; permission/device failures remain possible |
| D-Bus: remove kernel | Kernel-name validation and independent running-release check; fixed purge invocation | Service sandbox conflicts with dpkg/boot writes; dependency removals need execution review |
| pkexec: install updates | Exact helper executable annotation; CLI permits full/selected validated requests; fixed APT executable and cleared environment | Runs outside maintenance service sandbox; review binding and privileged cancellation unresolved |
| GUI/core: cleanup and reports | Review/confirmation, inode checks, Trash preference, report redaction/save logic | Path races, export temporary-file safety and audit outcomes need further work |

System bus policy permits delivery to the helper interface and standard introspection/property/peer interfaces; only root is allowed to own its service name. Delivery permission is distinct from operation authorization. Do not change bus/Polkit policy without the required review.

The helper checks D-Bus caller PID/UID using the bus daemon and asks Polkit with a `system-bus-name` subject. The shipped defaults use `auth_admin` for process termination, journal vacuum, kernel removal and updates; cache cleaning and SMART refresh allow an active local session. All use `auth_admin` for other/inactive sessions. The policy avoids `_keep`, but effective authorization and visible prompts depend on host rules/session/agent. See [Polkit policy semantics](https://www.freedesktop.org/software/polkit/docs/latest/polkit.8.html).

For updates, pkexec supplies elevation. `PKEXEC_UID` selects the helper CLI mode; the variable alone is not proof of privilege or authentication. The wrapper validates arguments because [pkexec does not](https://www.freedesktop.org/software/polkit/docs/latest/pkexec.1.html).

## Threat register

Present means a control exists in source, not that installed behavior passed. Partial means a control leaves a material gap. Open means evidence or protection is missing. D IDs refer to proposed work, not completed mitigations.

| ID / threat | Current control and residual risk | Status / closure |
|---|---|---|
| T1: caller spoofing | Sender comes from the D-Bus header; credentials resolve through the daemon. Mock authorization tests do not prove live bus identity binding. | Present; installed allow/deny/vanished-sender cases: D11 |
| T2: PID reuse | The wired helper and exported core implementation check start time, then use numeric kill. Validation/signalling are separate; signal success does not prove exit. | Partial; every retained implementation, stable references and race fixtures: D8/A3 |
| T3: signal injection | Request has no signal selector; implementation uses SIGTERM. Numeric/type boundaries still need validation tests. | Present; retain authorization and target restrictions: D8/D11 |
| T4: filesystem substitution | Fixed helper paths reduce caller-controlled scope. User cleanup checks inode/device before later path operations. Trash/save destinations are also mutable; directory manifests do not freeze writers. | Partial; complete concurrency argument and adversarial identity assertions: D7/A2 |
| T5: command/argument injection | Helper uses fixed executables, explicit argv, cleared environments and validated package strings; selected updates have an option terminator. This does not freeze APT's dependency resolution or make every subprocess safe. | Present control; preserve it while fixing output/resource handling: D1/D5/D6 |
| T6: stale authorization | Each D-Bus action requests Polkit authorization; shipped defaults avoid retained authorization. Host overrides can change behavior. | Present; test each action/session and prompt dismissal: D11 |
| T7: wrong action or reviewed scope | Distinct action IDs exist. The update code infers execution intent from package count and revalidates with full-upgrade discovery. A simulation hash or supervisor alone does not bind execution. | Partial; explicit intent and approved backend proof: D5/D6/A1 |
| T8: resource exhaustion | Service has a four-request semaphore and configured cgroup limits; application job limits are per batch and speed tests bypass them. Output, retained results, scan paths/issues and blocking calls need aggregate limits. The CLI is outside service resource limits. | Partial; D1/D3/D5/D11/D12 |
| T9: malformed/oversized input | Typed method signatures and field-specific length/character/range checks exist. DTOs do not declare Serde unknown-field rejection. No app-owned transport allocation budget is demonstrated. | Partial; malformed wire calls, bounded allocations and field-boundary tests: D5/D11 |
| T10: journal destruction | Confirmation in the GUI, operation authorization and numeric bounds limit use. They do not make deletion reversible or protect logs from a caller legitimately authorized to vacuum. | Present limits; denial/partial-failure tests: D5/D11 |
| T11: interruption and false success | APT replaces the authenticated wrapper with exec; app checks versions after success. Safe cancellation, configured state, disconnect/logout/restart and power-loss recovery are unproved. Cleanup may mutate files before history recording fails. | Partial; D5-D7/A1; reconcile unknown outcomes before retry |
| T12: disclosure through output/export | Helper errors can contain raw utility output. Reports copy free-text descriptions/reasons/templates that structural redaction and residual patterns do not fully sanitize. Complete authorization audit coverage is unproved. | Open; safe errors/audit: D5; secure export: D7; end-to-end default redaction: D13 |
| T13: elevated GUI / substituted installed files | Root refusal is absent. Correct root ownership and permissions of binaries, policy and units must be verified in installed packages. | Open; D2/D11 |
| T14: unexpected network activity | Passive monitoring is intended to stay local. APT and the user-confirmed Cloudflare speed test can contact external services. Direct network restrictions cover the maintenance unit, not every process or delegated service. | Source behavior; capture startup/passive/confirmed-action traffic: D11 |
| T15: substituted/private persistent data | SQLite opens by path before chmod; application-directory ownership/permissions and empty/relative XDG values are not validated. Migration transactions exist, but a newer schema is not explicitly refused. | Partial; private storage, no accidental reset/overwrite and tested recovery: D13 |
| T16: misleading inspection evidence | Duplicate dispatch fabricates equal identities and searches a top-200 list; incomplete/failed traversal can be mistaken for empty or zero. Misleading evidence can drive a wrong maintenance decision. | Open; truthful app-path discovery, bounded coverage and typed partial states: D12 |

The PID race is why [stable process references](https://manpages.ubuntu.com/manpages/jammy/man2/pidfd_send_signal.2.html) are proposed. They still need review-identity and permission checks. No planned pidfd, supervisor, hardened cleanup or safe-error feature should be described as shipped until its PR and tests close.

## Configured service sandbox

`data/systemd/maintainer-helper.service` is activated on demand and has no boot-enablement section. The server checks for zero in-flight requests on a 60 s timer; this is periodic idle checking, not a precisely reset inactivity deadline. Blocking operations may delay it.

- Filesystem: `ProtectSystem=strict`, writable exceptions `/var/cache/apt` and `/var/log/journal`, `ProtectHome=yes`, `PrivateTmp=yes`.
- Privilege/network: `NoNewPrivileges=yes`, `IPAddressDeny=any`, `RestrictAddressFamilies=AF_UNIX`.
- Kernel/process controls: protection of modules, tunables, logs, cgroups and clock; restricted namespaces, realtime and SUID/SGID; locked personality.
- Resource settings: `MemoryMax=128M`, `CPUQuota=25%`, `TasksMax=32`.

These are configured controls; effective enforcement must be checked on each supported system. The [systemd execution manual](https://manpages.ubuntu.com/manpages/jammy/man5/systemd.exec.5.html) explains the read-only filesystem and writable exceptions. The kernel-removal path does not fit those exceptions. Do not widen them for all helper actions as a repair shortcut.

`ProtectProc` is not set; process validation needs access to target metadata. `CapabilityBoundingSet` and `SystemCallFilter` are not configured. The update CLI inherits none of this unit's sandbox merely because it uses the same executable. APT's own locks/recovery are necessary but are not Maintainer transaction rollback.

## Evidence and release decision

Existing test locations are [authorization mocks](../crates/maintainer-helper/src/authorize.rs), [validation](../crates/maintainer-helper/src/validate.rs), [server routing](../crates/maintainer-helper/src/server.rs), [policy consistency](../crates/maintainer-helper/tests/policy_consistency.rs), [cleanup](../crates/maintainer-core/src/cleanup/execute.rs), and [redaction](../crates/maintainer-core/src/reports/redact.rs). Actual test functions include `authorize_fails_when_caller_unknown`, `authorize_fails_when_polkit_denies`, `interface_rejects_when_concurrency_full`, `remove_kernel_rejects_running_kernel`, and `execute_detects_inode_change`. Their presence is not a pass result or proof of race closure.

No installed authorization, adversarial filesystem, message-flood or end-to-end privacy sign-off is recorded by this review. Every open/partial item needs its linked work and reproducible evidence, or an explicitly approved disabled/narrowed capability before release. New helper API/Polkit changes require approval and authorization tests. This documentation update changes no executable security boundary.
