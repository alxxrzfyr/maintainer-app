# Safety decisions required before implementation

Status: proposed, 2026-10-30. Companion to the [roadmap](development-roadmap.md); this is the decision record for D5-D8, not a second work queue. No helper API, Polkit, sandbox or product-scope change is approved by this document.

Each decision closes with a named owner/reviewer, selected design, supported environments, failing adversarial case, passing proof, residual risks and approval reference where required. A source citation is evidence about an API; it is not proof that our integration is correct.

## A1. Package ownership and reviewed execution — D5/D6

Two separate problems must close: **who owns work after the GUI disconnects**, and **what prevents execution from changing the reviewed package set**. Solving one does not solve the other.

| Option | What it offers | What still needs proof / disposition |
|---|---|---|
| Existing pkexec -> exec APT | Small wrapper, fixed argv, existing elevation | No application-owned status/reconnection protocol; separate simulation and execution. Not sufficient for the proposed contract. |
| Installed PackageKit backend | Transaction identity, progress, authorization and backend-reported cancellation | Check the actual Ubuntu 22.04 backend and derivative versions: exact plans, removals, holds, disconnect, restart and cancellation semantics. Latest documentation is not floor-version evidence. Evaluate first; do not adopt solely because it has a transaction API. |
| Dedicated short-lived package owner | Explicit lifetime, bounded status, reconnect/recovery policy; leaves other maintenance operations sandboxed | Must use a reviewed mechanism that binds resolution to execution. Wrapping apt-get and re-running simulation still leaves the gap. Additional IPC, state and privileged code increase review cost. |
| Discovery-only capability | Keeps inspection useful while preventing unproved mutations | Recommended fallback if neither executable design passes. Disabling an advertised capability requires explicit scope approval and release-note/UI changes; no silent fallback to unsafe execution. |

The [APT manual](https://manpages.debian.org/bookworm/apt/apt-get.8.en.html) documents unlocked simulation and dependency-changing upgrades. This is corroboration from Debian APT, not a substitute for the Ubuntu-floor test. [PackageKit's transaction interface](https://www.freedesktop.org/software/PackageKit/doc/dbus-org.freedesktop.packagekit1.Transaction.html) exposes `AllowCancel` and caller/transaction state. Neither source establishes an immutable preview-to-execution guarantee for Maintainer. Current PackageKit documentation describes version 1.99.0; do not copy new signatures into a floor implementation.

The selected design must specify:

1. **Review binding:** canonical intent and complete operations: name, architecture, old/new version, install/remove/purge/downgrade, relevant policy. Define the authoritative resolver/configuration and the exact point after which competing changes cannot alter the approved operation set. A hash detects a mismatch only when both sides actually compare the same authoritative plan.
2. **Authority:** authenticate the caller independently of UI confirmation. Validate bounded requests before expensive work. Use fixed operation types; no caller-supplied executable, environment, repository, root path or arbitrary APT options. Restrict status/cancel/reconnect to the originating identity under the approved policy; an operation ID is not authorization.
3. **Ownership:** one system package mutation at a time through this owner, plus native APT/dpkg locks against other tools. Start/query/cancel responses identify the same transaction. Deduplicate repeated start requests within the defined receipt lifetime; after lost state, reconcile before allowing another mutation. Do not claim exactly-once effects across power loss.
4. **Lifecycle:** backend-confirmed stages determine cancellation. Before mutation, a successful stop can be cancelled. After mutation begins, timeout/disconnect means work may continue or require recovery; it cannot become success/cancelled on the GUI's guess. Bound wait-for-lock and authorization states without killing an active dpkg transaction.
5. **Recovery:** retain a minimal private receipt: operation identity, origin, reviewed-plan identifier, phase, completion/postconditions. Specify storage location, schema, size/retention, expiry and restart reconciliation; no credentials/raw utility output. Handle GUI close, logout, helper restart, shutdown and interrupted configuration. [systemd 249 inhibitors](https://www.freedesktop.org/software/systemd/man/249/systemd-inhibit.html) can coordinate sleep/shutdown but privileged overrides and power loss remain possible.

**Proof before selection:** disposable VM/local package fixtures; selected and full plans; dependency/removal change between preview and execution; multiarch/holds; invalid repository signature; lock contention; denied/dismissed authorization; duplicate request; disconnect/reconnect; interrupted configuration/reboot. Compare actual changed packages and configured states with the approved plan. No unreviewed mutation is acceptable. Record backend version and the failed alternatives as well as the passing design.

## A2. File identity and concurrent writers — D7/D12

Distinguish three properties: staying inside approved roots, changing the reviewed file identity, and avoiding overwriting an existing destination. They need separate proofs.

| Mechanism | Provides | Does not establish |
|---|---|---|
| Directory descriptors plus constrained path resolution | Anchored traversal; supported symlink/mount restrictions | Identity of a name at a later unlink/rename |
| Open file descriptor plus before/after metadata | Evidence about the opened file during inspection/hashing | Frozen file content against arbitrary concurrent writers |
| Exclusive creation / no-replace rename | Destination collision protection on supported filesystems | Conditional source rename/unlink based on the reviewed inode |
| Descendant manifest | Explicit reviewed recursive scope | A lock on the tree or consent for newly added descendants |
| Move to quarantine, then check | A place to inspect what was moved | Zero out-of-scope effects: a replacement may already have been moved; restoration can collide |

[openat2](https://man7.org/linux/man-pages/man2/openat2.2.html) constrains resolution; [renameat2](https://man7.org/linux/man-pages/man2/rename.2.html) offers destination no-replace behavior with filesystem requirements. Neither documents a generic compare-inode-and-unlink operation. The current plan therefore does **not** assert that those primitives alone close an arbitrary concurrent-writer threat.

**Required design output:** enumerate supported roots/filesystems, writer assumptions and how they are enforced; give syscall ordering, descriptor ownership, source/destination checks, cancellation points and partial-failure recovery. Identify the irreversible step. Specify what changes while a confirmation is open invalidate the plan. Do not assume an advisory lock prevents a noncooperating writer or that moving a directory freezes writes through existing descriptors.

**Recommendation:** ship anchored read-only inspection independently. Enable mutation only for cases whose approved protocol meets the stated scope/identity contract. If that requires narrowing supported operations or the contract, obtain explicit approval; otherwise the mutation stays unavailable. Preserve original files after a failed Trash/export step; never delete a colliding entry to make room.

**Proof:** deterministic barriers at discovery, descriptor acquisition, validation and mutation; swap each ancestor/leaf, add/remove descendants, change file content, collide destinations and inject ENOSPC/EXDEV/permission failures. Assert the actual affected identities, not only the returned error. Include an existing open writer. A stress loop is useful secondary evidence; it cannot replace the controlled race cases.

## A3. Process identity through review and elevation — D8

Both core and helper currently implement start-time validation followed by numeric-PID signalling. The shipped GUI uses the helper; the core function is exported but has no app callsite. Consolidate retained code behind a stable-handle adapter or remove unused functionality after checking consumers. A PID/start-time pair is useful review metadata; it is not an open kernel reference.

The design must state when the pidfd is acquired, how live metadata is tied to that handle, how the review remains bound during an authorization prompt, and who closes it on cancellation/failure. Retaining a handle through elevation may require a reviewed API change. Do not claim an FD opened after a potentially stale review automatically represents the reviewed process.

Use floor-supported syscalls with flags zero, SIGTERM only and explicit permission errors. Reject invalid/out-of-range PIDs before casts. No numeric-PID fallback. Signal success means delivery accepted; separately observe exit if the UI needs to claim termination. [pidfd_send_signal](https://man7.org/linux/man-pages/man2/pidfd_send_signal.2.html) still performs permission checks.

**Proof:** inject exit/reuse at every acquisition/check/send boundary in each retained route; preserve denied/unavailable/vanished distinctions, close all FDs, and demonstrate that no unrelated process is signalled. If the proof requires a helper API change, link its authorization review before merging.
