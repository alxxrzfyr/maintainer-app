# Maintainer threat model

Updated 2026-10-31 for the unpublished implementation candidate. This describes
source controls and local fixtures, not installed-system certification.

## Trust and authorization

Protect system integrity, reviewed files, diagnostic privacy and responsiveness.
Callers, paths, package requests, utility output and exported text are untrusted.
The kernel, system bus, Polkit, distribution package tools and correctly installed
root-owned integration files form the trusted computing base. Root compromise,
malicious administrators and malicious repository maintainer scripts are outside
Maintainer's protection. Fixed arguments do not sandbox package scripts.

The GUI refuses effective UID 0 before GTK or persistence initialization. Blocking
work runs in bounded workers. The system bus supplies caller credentials; Polkit
authorizes a system-bus-name subject. Bus delivery permission is not operation
authorization. Host policy and desktop agents determine actual prompts.

Shipped policy uses auth_admin without retained authorization for package changes,
process termination, journal vacuum and file cleanup. Cache cleaning and SMART
refresh allow an active local session; other sessions require administration.
Helper/API and policy changes were authorized for this candidate.

## Operation boundaries

| Operation | Enforced control | Limit |
|---|---|---|
| Process termination | Reviewed pidfd retained across D-Bus and authorization; validated identity; SIGTERM only | Accepted signal does not prove process exit; no numeric-PID fallback |
| Cache, journal, SMART | Fixed operations and arguments; bounded identifiers, sizes and ages | Journal removal is irreversible; UDisks2 remains a trusted service |
| Updates and kernel removal | Dedicated PackageOwner service retains one native APT cache and native locks from resolution through approval and commit; approval binds complete operations, architectures, versions, archive hashes and fixed policy | Distribution scripts run as root. Maintainer self-update requires another package manager. Running kernels, essential packages and kernel tracking metapackages are protected |
| Transaction recovery | Private bounded atomic receipts; original connection/UID binding; stable request-ID deduplication; approved backend survives GUI/owner loss | Cancellation is refused after mutation starts. Recovery never replays mutation; reconciliation reports only observed postconditions and clean package state |
| File cleanup | Retained parent/file descriptors, bounded regular-file manifest, protected root-owned quarantine, no-replace moves; compare isolated entry with reviewed descriptor before deletion | Caller-owned, single-link regular files on the same supported filesystem only; no recursive folder deletion, content snapshot or physical-space guarantee |
| Cleanup authorization | One Polkit approval for at most 256 exact entries, bound to UID, unique connection, mode and single-use indices; bounded five-minute grants | A disconnected connection cannot resume a grant or silently authorize a new batch |
| Cleanup recovery | Raced entry is restored without overwrite or retained for recovery; explicit restore elsewhere accepts a validated caller-owned destination descriptor | User approved that an unreviewed replacement may temporarily move. It must never be deleted. Recovery collisions preserve the retained entry |
| Trash/permanent deletion | Explicit review and stronger permanent-delete confirmation; Trash has no permanent-delete fallback | Post-publication sync/receipt failures report durability uncertainty and never trigger a destructive retry |
| Reports | Default redaction, anonymous private output inode, held destination directory and exclusive publication | No overwrite; unsupported anonymous-file support is explicit. A directory-sync failure after publication means durability is uncertain |
| Persistence | Private database/sidecar validation; preserve corrupt/newer databases; bounded exclusive recovery output | Installed upgrade/downgrade behavior is not inferred from fixtures |

Retained file identity does not freeze file contents. A same-user writer can alter
bytes in the reviewed inode; the cleanup contract binds identity, not a snapshot.
Source directory replacement or collision cannot authorize deletion of a different
inode. Leftover receipts and recovery entries are bounded and never silently purged.

Native APT requests contain fixed intents and validated package specifications.
There is no arbitrary shell/command API. Production clears inherited configuration,
uses fixed executable paths and bounds requests, plans, receipts and recent history.
Read-only reconciliation obtains native locks and checks the complete configured
package map. A clean repaired state with an unfulfilled original plan reports an
unknown original outcome, not success. Terminal receipts may expire; unresolved
receipts are retained. Legacy unbound update/kernel mutation routes refuse work.

## Service scope and residual risks

The maintenance helper is D-Bus activated, has no boot-enablement section and exits
when idle. Its unit keeps ProtectSystem=strict, ProtectHome=yes, PrivateTmp=yes,
NoNewPrivileges=yes, IPAddressDeny=any and AF_UNIX-only sockets. Writable state is
limited to configured cache/journal paths and private Maintainer state. Validated
caller-owned descriptors provide the cleanup capabilities. MemoryMax=128M,
CPUQuota=25% and TasksMax=32 bound this helper. These settings describe configured
policy, not proof of effective enforcement on every host.

The separate package owner needs normal root filesystem/network scope for APT and
maintainer scripts. It does not widen the maintenance helper sandbox. Its unit
uses private state, UMask=0077 and KillMode=process to preserve an approved backend
if the owner exits. Immediately before dpkg mutation, the backend acquires a
shutdown/sleep block inhibitor through a fixed systemd-inhibit holder. A bounded
echo handshake confirms acquisition before mutation; the holder stays alive
through postconditions, terminal receipt persistence and native-lock release.
Acquisition failure refuses mutation. Administrator overrides, logind failure,
forced shutdown and power loss can still interrupt work. Upgrade scripts mask both activations, verify effective masks,
wait for ownership/locks to drain and undo only their own masks. Purge refuses an
active transaction. No application rollback of package scripts is promised.

Passive monitoring stays local. APT and the user-requested speed test can contact
external services. There is no telemetry or system-data upload. Default report
redaction and bounded safe errors reduce disclosure; fixtures cannot prove that
all possible host strings or administrator policy overrides are safe.

## Evidence and release scope

Local coverage includes private-bus authorization/denial/connection tests, actual
filesystem race and recovery syscalls in temporary directories, retained-cache
APT transactions in disposable directory roots, parser/resource boundaries and
package archive integration. Tests never install packages on the personal host.
The local gate runs available native and Python suites three times and binds the
source and exact package bytes in an external manifest. A source review found no
critical security/data-loss defect in this final scope; that is not a formal audit.

The user waived additional machines and third-party release verification. No live
installed Polkit, installed upgrade, Wayland/arm64 matrix, other-hardware or full
privacy/security certification is claimed. Numerical performance budgets still
require measurements. Original planning content is preserved in the historical archive; current
release status records unfinished validation without treating archive checklists
as current instructions.
