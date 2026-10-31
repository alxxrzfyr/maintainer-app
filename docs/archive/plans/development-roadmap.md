# Maintainer: fast, lightweight native GUI roadmap

Date: 2026-10-30. Source baseline: `07f7b9a096c3a84b57b329fca5e4d4d2d93289c1` (1.2.0); reconciled with `main` at `79b42ac`.
Status: implementation proposal; no runtime changes or Linux validation in this review.
Read with the [evidence register](../research/2026-10-30-engineering-review.md), [safety decisions A1-A3](safety-design-decisions.md) and [memory experiments M0-M6](../research/2026-10-30-memory-under-100mb.md). [Documentation index](../../README.md). D0-D13 are stable references, not execution order; new types/files are proposals.

## 1. Outcome and constraints

Deliver a responsive, lightweight native GTK4 maintenance app for Ubuntu 22.04 and newer Ubuntu-based desktops. Connect observed problems to reviewed actions and verified outcomes. This integrated workflow is a product hypothesis, not a proven exclusive advantage. Host-speedup claims require a named workload and controlled measurements.

**Memory milestone: combined idle PSS strictly below 100,000,000 bytes** (100 decimal MB, about 95.37 MiB), including every Maintainer-owned process. Feasibility is unverified. The former 150 MB target/220 MB ceiling does not satisfy this milestone; <75/<50 MB are exploratory checkpoints.

| Value | Acceptance / owner |
|---|---|
| Fast | Usable navigation within 2 s; responsive during slow I/O. Measure window mapping, usable shell and first readings separately; D4/D9/D10. |
| Lightweight | Meet the memory milestone; idle CPU target <=0.5%, ceilings 1% hardware / 2% software rendering. Bound work and retained data; no permanent Maintainer daemon; D0/D3/D9/D10/D12. |
| Native GUI | Rust 1.85, GTK 4.6 floor, semantic GTK CSS, Wayland/X11, keyboard and screen-reader access. Everyday workflows remain graphical; D4/D9/D11. |
| Stable | Denied permissions, missing integrations, cancellation, DB failure and partial results produce usable states; no fixture crash/hang or silent replay. |
| Safe/private | Unprivileged GUI, narrow elevation, reviewed destructive actions, private local settings/history, default report redaction, no telemetry/upload; D2/D5-D8/D13. |
| Understandable | Show measurement/source, age, scope, risk and actual result. Incomplete inspection cannot mean "healthy" or "no duplicates"; D4/D9/D12/D13. |

Keep 2 s default monitoring and 10 s hidden sampling. Render on demand; 60 FPS is an interaction aspiration. Preserve accessibility and safety during optimization. Use existing dependencies first. Exclude WebView/Electron/libadwaita, automatic duplicate deletion, RAM cleaning, arbitrary commands, unsolicited scans, new cleaner catalogs and a snapshot/rollback engine. Defer other feature expansion.

**Shared rules:** helper API/Polkit changes require explicit approval, [threat review](../../threat-model.md) and authorization tests; this plan grants none. Keep other maintenance operations sandboxed. Preserve shipped migrations and Cargo-owned lockfiles; no signing credentials needed. Missing evidence is `unverified`. Interrupted mutations may be `partial`, `recovery required` or `outcome unknown`; never automatically replay them. One terminal notification does not guarantee exactly-once effects across crashes.

## 2. Delivery order and accountability

Assign a named developer and reviewer to each row. P0 addresses unsafe/broken behavior; P1 addresses efficiency/verification. Both are required. Dependencies specify completion order, not estimates. Evidence IDs refer to the linked register.

| ID | Priority / owner expertise | Work / evidence | Depends on |
|---|---|---|---|
| D0 | P0 / release | Baseline and acceptance contract; E13 | None |
| D1 | P0 / core | Bounded child execution; E3 | D0 |
| D2 | P0 / app | Root-launch refusal; E1/E12 | D0 |
| D3 | P1 / core | Global job admission; E4/E18/E21 | D0 |
| D4 | P0 / app + core | Remove GTK-thread I/O; E5/E20 | D0; integrate D1/D3 |
| D5 | P0 / security + packaging | Privileged package boundary; E2/E15 | D0; approved A1 candidate design |
| D6 | P0 / updates | Reviewed transactions and cancellation; E9/E10 | D1, D5 |
| D7 | P0 / filesystem | Cleanup, Trash and export; E8/E11/E16 | D0; approved A2 candidate design; D12 traversal |
| D8 | P0 / helper | Stable process identity; E8 | D0; A3; API approval if changed |
| D9 | P1 / GTK | Widget reuse and monitoring identity; E6/E14/E22 | D3, D4 |
| D10 | P1 / performance | Measurement harness and final evidence; E7/E13/E22 | D0; final run after implementation |
| D11 | P0 / release + QA | Installed-artifact release gates; E7/E12/E15 | D1-D10, D12-D13 |
| D12 | P0 / storage | Correct, bounded duplicate discovery; E17/E18 | D0; integrate D3 |
| D13 | P0 / privacy + persistence | Redaction and private storage; E11/E19/E20 | D0; integrate D4/D7 |

Start D10's harness alongside D1-D4/D12, D13 redaction and safety design work. D4 need not wait for D3. Missing Linux baseline access does not block independent fixes. Use focused PRs; finish approved boundary implementations before dependent workflows and final D10/D11 validation.

An unresolved required item blocks this milestone. Defer release or obtain explicit approval for a narrower scope/disabled capability, with matching UI and release notes. An interim safety release cannot claim completion of the memory goal.

## 3. Work packages

File shorthand: app/core/helper means the corresponding `crates/maintainer-*/src/` directory.

### D0 — Establish the baseline and acceptance contract

**Files:** `README.md`, `AGENTS.md` and `.github/workflows/ci.yml` (read), `CHANGELOG.md`; dated results under `docs/validation/`.

- Record commit/dirty state, manifest and lock hashes, toolchain, fixtures, distro/kernel/GTK, display server/renderer, CPU/RAM and exact commands. Remove personal identifiers/paths. Current main includes libc 0.2.191 and Tokio 1.54.1; these dependency updates are committed. Use pinned Rust and locked resolution; record dependency changes separately from the source baseline.
- Attempt baseline Linux checks; record actual executed/skipped counts, failures or unavailable environment. Historical reports are not current results.
- Preserve <=130 MB Private_Dirty, <=40 MB matched-GTK delta target and <=2 s startup. Resolve legacy MB/MiB ambiguity before enforcement; publish bytes and both units meanwhile. The new 100 MB threshold is already decimal. Classify pass, target miss, ceiling failure, environment error and unverified separately.
- Compare minimal GTK, app shell and full app on floor/current profiles. The historical ~135 MB GTK estimate is not a universal floor. If the matched fixture is >=100 MB, attribute renderer/library costs before promising app-only savings.

**Done:** dated baseline, missing-evidence owners and unit decisions. Ratify resource defaults, D1's cleanup bound, D4's metadata cadence and D10's responsiveness criterion. Independent safety fixes may proceed while runtime measurements remain pending.

**Proposed resource defaults:** D0 ratifies or replaces these from measurements. These cap application data, not PSS. Test limit-1/limit/limit+1 and peak process memory.

| Resource / owner | Initial rule |
|---|---|
| Command output / D1 | Existing 10 MiB per stream; 64 KiB frame/line; 4 KiB displayed progress/error. Structured overflow fails; diagnostic truncation is explicit. |
| Jobs/results / D3 | Four active, 16 queued, 24 accepted-but-unreleased including completed jobs. Release an accepted-job permit only after work stops and its result is consumed/dropped. Reserve result slots; bound each type and aggregate retained payloads to 16 MiB initially, reduced to measured idle headroom. Oversized results receive bounded terminal errors. |
| Worker lanes / D3-D4 | One metric worker and one serialized DB worker, separately counted. Network metadata and speed tests use normal admission. One expensive filesystem scan; never occupy a worker waiting for that slot. |
| Scans / D12 | Stop at the first active-work limit: 64 MiB, 100,000 candidates, 1,000 issue samples or depth 256. Count paths, frontier, indexes, hard links and UI copies. Completed results must fit idle headroom. Report incomplete coverage and omitted-issue counts. |
| Status / D3-D4 | One replaceable progress value per job and one latest metric snapshot, with generations/timestamps. Reserved terminal outcomes cannot be overwritten by progress. |
| DB/reports / D4/D13 | 32 outstanding nonblocking DB requests; coalesce superseded settings. One safely rendered preview, <=4 MiB; identify omitted sections. Bound diagnostic strings. |

### D1 — Unify bounded command execution

**Files:** core `command.rs` and callers; compiled test-child fixtures.

- Drain stdout/stderr concurrently through one ordinary/streaming implementation. Continue draining after capture caps, discarding excess. Bound lines and pending progress; reject truncated protocol records.
- Enforce deadlines/cancellation through process exit, including after stdout EOF. Interruptible reads must prevent inherited-pipe joins from hanging. Give ordinary owned children a dedicated process-group/descendant policy; never signal the GUI's inherited group. Surface kill/read/join failures. D5/D6 own elevated transactions.
- Inventory direct process calls, including connection/helper adapters. Route ordinary calls through the runner; document approved exceptions. Preserve direct argv, existing timeouts, parser locale and typed errors. Use trusted executable resolution, validated arguments, narrow environment overrides and null stdin unless required; no shell.

**Tests/done:** both pipes exceed capacity; 32 MiB without newline; stdout closes before exit; descendant retains pipe; invalid UTF-8; nonzero exit; missing executable/spawn failure; timeout/cancel. Fixtures finish within deadline plus proposed 2 s cleanup, obey caps and leave no owned child/reader. Report uninterruptible/denied cleanup honestly. Verify one terminal outcome, boundary sizes and peak memory. No live APT fixture.

### D2 — Refuse root before GTK or persistence

**Files:** app `main.rs`, `application.rs`; core identity adapter; package/smoke scripts.

- Check effective UID before GTK, DB or helper initialization. UID 0 exits nonzero with normal-user launch instructions; do not drop privileges inside the GUI.
- Keep identity access outside widgets using existing libc. Install packages as root; launch smoke tests as a dedicated unprivileged user. Early launch failure must fail the test.

**Tests/done:** injected root/nonroot cases; disposable-VM root invocation exits without display/window, DB creation or helper call; normal-user installed launch succeeds.

### D3 — Enforce application-wide job limits

**Files:** core `jobs.rs`; app `runtime.rs`, `dispatch.rs`; network speed-test caller.

- Replace per-`execute()` pools with shared scheduler state and D0 limits. GTK submission immediately returns accepted/busy. Reserve batch job/result slots atomically or reject the entire batch. Bound completed receivers too; no dispatcher thread per submission or worker waiting for its consumer.
- Queued cancellation skips execution; running work cooperates. Reserve terminal result capacity. Shutdown rejects new jobs, cancels pending work and never blocks GTK on uncontrolled tasks. Recover permits after panics; dropping a receiver requests cancellation but never releases an active-worker permit before execution exits. Results transferred to UI models remain within D0's retention budgets. Package shutdown follows A1, not generic killing.

**Tests/done:** with four active and 16 queued, reject job 21 immediately. Drop all four blocked running receivers: a fifth job must not start until a worker exits. Separately retain 24 completed receivers; reject job 25 until consumption/drop. Cover independent submissions, atomic batches, panic and queued cancellation. Accepted subscribed jobs deliver one outcome; rejected batches do nothing. Include speed tests and storage in saturation tests.

### D4 — Remove system work from GTK callbacks

**Files:** app `monitoring.rs`, `runtime.rs`, `ui/shell.rs`; core `metrics/connection.rs`, `hardware.rs`; proposed monitoring service.

- Move `read_connection_details()` out of `apply_network()`: its `ip`/`iw` calls currently block GTK. Return typed worker results; use D1 deadlines.
- Move device discovery and DB open/migration/settings/history off GTK. Use D0's separate serialized DB lane, loading states and incremental updates. Keep monitoring usable on DB failure. Serialize settings with revision IDs; stale completions cannot replace newer edits, and save failures stay visible.
- Keep GTK objects/borrows on the main thread; copy requests before dispatch and drop borrows before callbacks. Consolidate touched collection paths in `Runtime::collect_snapshot()` and the shell worker behind one core service. D13 owns storage privacy/recovery.
- Apply saved intervals and 2 s/10 s defaults. Proposed metadata refresh: Network entry, explicit refresh, or at most once per 10 s while visible. Allow at most one queued or running metadata request; coalesce further refreshes without blocking metric rates. Coalesce snapshots using monotonic timestamps/generations; discard stale results and distinguish busy/unavailable/permission/failure/cancel states.

**Tests/done:** slow connection/DB fixtures preserve GTK heartbeat/navigation; reducers require no system access; saved intervals change scheduling; worker-start failure reports unavailable instead of panicking. Measure main-loop delay.

### D5 — Implement an approved privileged package boundary

**Files:** helper `server.rs`, `main.rs`, `validate.rs`, `operations/{kernel,updates}.rs`; service unit, Polkit policy, app client, packaging, threat model.

- Test the predicted kernel-removal failure in an installed VM; current writable paths exclude dpkg/boot. Test cache cleaning independently.
- Compare A1's installed PackageKit, short-lived owner and discovery-only options. Obtain required approval of the candidate design and prototype scope before boundary implementation; close A1 after its VM proof. Prove both lifecycle ownership and review-to-execution binding. Keep cache/journal/SMART sandboxed; no broad writable-root exception. Specify caller authentication, bounded schema, operation authorization, cancellation/reconnect, API compatibility and receipt retention.
- Serialize package mutations while respecting native APT/dpkg locks. Never delete locks or kill their owner. Bound lock/auth waits; preserve active configuration across GUI disconnect. Handle logout, service restart and shutdown using supported inhibition where available; record override/power-loss limits and reconcile private receipts without automatic replay. Test appropriate resource limits instead of copying the helper's 128M/25% limits onto APT.
- Return bounded safe errors/action categories, never raw utility output or sensitive arguments. Bound wire decoding allocations; test malformed/unknown fields, panic recovery and request/idle deadlines.
- At execution, rederive the running kernel and validate the complete reviewed removal plan, dependencies and protected metapackages. Reject running-kernel removal. Define evidence for retaining a bootable fallback; installed does not mean known bootable. Keep removal unavailable until approved installed tests pass.

**Tests/done:** old-kernel removal, running-kernel denial, unauthorized/dismissed authorization, stale plan, malformed package, locks and partial package failure. Check dpkg/boot files and reboot into the retained kernel. Run A1's disconnect/restart/recovery proof; all other helper actions still pass their installed sandbox tests.

### D6 — Preserve update intent and report actual outcomes

**Files:** core `updates/{model,apt}.rs`, `command.rs`; app dispatch/update state/page; approved helper package entrypoint.

- Store full-versus-selected intent explicitly; candidate counts cannot determine it. Revalidate selected plans with the same selection resolver. Reuse `UpdatePlan` for canonical installations, removals, architectures and versions.
- Implement A1's authoritative review binding. APT simulation is unlocked; nonroot configuration can differ. Repeating simulation or pinning requested packages does not freeze dependencies. `--no-remove` is only an additional guard. Refuse unbound mutations; never externally seize locks in a way that deadlocks APT.
- Verify installed versions and configured states by name + architecture; distinguish remove/purge. Batch queries; truncated output or unresolved postconditions cannot mean success. Preserve repository authentication, holds and downgrade protection; no bypass flags.
- Model authorization, download, configuration, verification and recovery. Cancel only at backend-confirmed safe boundaries; preserve existing UI restrictions. After mutation begins, timeout/GUI close cannot imply cancelled, rolled back or successful. Do not SIGKILL dpkg as recovery; require owner-confirmed completion.

**Tests/done:** selected subset/dependencies; same counts with different intent; changed versions/removals; lock/offline/auth failures; partial configuration; cancel before/after elevation; close during install. Use disposable VMs for mutations.

### D7 — Bind cleanup to review and protect Trash/export

**Files:** core `cleanup/{plan,execute,trash}.rs`, history and proposed secure export adapter; app cleanup results, `ui/reports.rs`.

- Implement A2's approved candidate protocol; require its proof before production acceptance. Anchored resolution, inode checks and quarantine alone do not make later mutation atomic. Specify supported writers/filesystems/syscalls; refuse cases without a scope/identity proof. The threat is concurrent filesystem mutation, not control of GUI memory.
- Require approved roots; reject overlapping/duplicate targets and symlink/mount escapes. Revalidate type/identity/content metadata. Recursive deletion needs a bounded reviewed descendant manifest or equivalent proof; new descendants are unapproved. Cancel during traversal; integrate D12's bounded walker.
- Honor XDG Trash and both mount layouts. Validate ownership/permissions/symlinks; create metadata exclusively and move without replacing existing entries. Refuse cross-filesystem moves. Failed-move cleanup must preserve preexisting entries. A library is acceptable only with equivalent verified behavior.
- Separate bytes moved to Trash from estimated bytes unlinked; neither is measured free-space gain. Account for hard links/open files/snapshots. Preserve old records; add forward migrations if needed. Return per-item outcomes even when history recording fails; flag missing audit history and reconcile before retry.
- Save reports off GTK using exclusive private temporary files and approved final replacement. Never reuse a predictable temporary path/symlink. Handle concurrent saves and failed rename without overwriting unrelated files.

**Tests/done:** deterministic ancestor/leaf swaps, hostile Trash/export symlinks, destination collisions, ENOSPC/EXDEV, metadata failure, mid-tree cancellation, overlap, history failure, concurrent save and failed rename. Assert actual affected identities and private report permissions; no out-of-scope changes. Permanent fallback needs stronger confirmation. Use unprivileged temporary-home/mount fixtures plus A2's race proof.

### D8 — Signal only the reviewed process

**Files:** core `metrics/terminate.rs`; helper `operations/process.rs`, validation; affected app client/dispatch and threat model.

- Implement A3's handle-acquisition, metadata/liveness and authorization ordering. Bind reviewed PID/start time to a live pidfd before signalling. Pin `/proc` access or verify reference liveness; a recycled numeric PID must not authorize another process. Reject mismatches/exits; never retry through numeric `kill()`.
- The GUI uses the helper; the exported core function has no app callsite. Consolidate retained routes behind one adapter or remove unused functionality after consumer review. Do not incidentally add another GUI route.
- Use floor-supported flags zero and SIGTERM only. Keep denied, read-failed, unavailable and exited states distinct; document unsafe invariants and FD ownership. Signal acceptance means "termination requested", not observed exit.

**Tests/done:** each retained route: stale identity, exit/reuse at every stage, authorized owned child, denied caller, PID 0/1/out-of-range, syscall/read denial, exhausted FDs and malformed stat names. No unrelated process is signalled; every FD closes. API changes require the shared approval rule.

### D9 — Reuse widgets and preserve monitoring identity

**Files:** app `ui/{processes,system,network}.rs`, `monitoring.rs`, shell/page ownership; core sensor cache.

- Replace full process-row rebuilds with PID/start-time keyed updates or GTK 4.6 ListView recycling. Preserve sorting, focus, selection, scroll and confirmation target. Reuse disk/sensor widgets; keep existing CPU-bar reuse.
- Measure eager page construction; create secondary pages on demand. Keep typed state independent of widgets; bound inactive caches and release expensive views where profiling supports it. Disconnect page-owned signals, stop timers and break strong-reference cycles. Preserve navigation/job state; measure after visiting every page.
- Key network history by interface identity; remove stale interfaces. Rediscover sensors on refresh or bounded disappearance recovery. Show sample age and unavailable/permission states; failed CPU/memory readings cannot leave old values labelled live. Stop hidden repaints while retaining bounded sampling/history.

**Tests/done:** 1,000 synthetic processes, reorder/PID reuse, interface hotplug/reorder, sensor disappearance and availability recovery. Unchanged samples preserve row objects and action identity. Compare CPU, memory, allocations/row counts and main-loop delay.

### D10 — Measure the contract on the release artifact

**Files:** `scripts/measure-performance.sh`; proposed minimal GTK fixture/report format and opt-in startup instrumentation.

Run M0-M6 from the linked memory research; profile causes separately from acceptance runs. Measure the combined candidate because individual savings overlap. Include all-pages-visited and completed/dismissed scan/report states, normal 1100x700 size and 200% scale as that protocol requires.

- Read PSS/Private_Dirty from `smaps_rollup`, with documented `smaps` fallback; kernel kB means 1,024 bytes. Report RSS, Swap/SwapPss and available GPU statistics separately. Include all owned workers/helpers using stable PID/start-time identity; unavailable accounting is unverified. No swapping/preloading/omitted-process shortcut. Matched GTK subtraction is diagnostic, not exact attribution.
- Start a monotonic clock immediately before child spawn; report spawn overhead and correlate inherited-FD markers. Record mapping, usable shell and first data separately. Usable requires visible navigation/loading states and a processed navigation canary. GTK mapping does not prove compositor presentation; record that separately. Reject preexisting-instance launches; RSS is not readiness.
- Fix session, actual renderer, window size/scale/theme, page cardinality and power profile; record deviations. Unknown renderer is unverified. Run unprivileged without dropping caches. Emit machine-readable result categories and sanitized raw samples; fixture-test parsing, startup, exit and errors. Instrumentation is test-only.

**Required protocol:**

| Measurement | Procedure |
|---|---|
| Idle | Release build; 30 s settle, 300 s sampling, three runs per Overview/System/Processes/Network page. Add hidden/minimized runs from Overview and Processes. Memory every 5 s; median and maximum. No active scan/speed test/update. |
| CPU | Combined owned-process CPU time / wall time; 100% = one core. Separate GUI/helper/children; include short-lived children without double counting. Parse parenthesized `/proc` names safely; disclose observer overhead and shared-service costs. |
| Sustained use | 30-minute soak and ten navigation cycles; report trend, retained results and post-job idle state. |
| Startup | Ten fresh-process warm launches and at least three separate post-reboot cold launches. Pair baseline/candidate on the same device; alternate warm-run order. |
| Invalid runs | Predeclare environmental exclusions for both builds; retain raw results/reasons. Crashes, timeouts and budget failures remain failures. |

**Gates:** every required valid sampled maximum is `<100,000,000` combined idle PSS bytes on declared hardware profiles. Retain D0's <=40 MB delta target and <=130 MB Private_Dirty limit. Each 300 s CPU average targets <=0.5%, ceiling 1%; every valid usable-shell launch <=2 s. Report measurement margin/repeatability; missing or uncertain data cannot pass. Software rendering retains its 2% CPU ceiling and reported-memory exception, but cannot certify a universal under-100-MB claim.

**Proposed responsiveness gate:** schedule a 100 ms heartbeat for 60 s per deterministic slow-adapter/scan fixture on the reference device. Measure nonnegative lateness against the oldest unserved monotonic deadline, then advance to the next future deadline and count skipped ticks. A stalled run fails; missed callbacks cannot be discarded to improve results. Require p95 <=50 ms and maximum <=200 ms. D0 must ratify cadence, duration and thresholds.

### D11 — Gate releases on installed packages

**Files:** CI, `scripts/test-package.sh`, `tests/system/*`, release metadata, README/changelog/threat model.

- Keep format/lint/tests/build checks. Test package install/upgrade/remove and unprivileged GUI launch with real system bus/Polkit. Missing tools or failed launch must fail; surviving three seconds is insufficient.
- Use `check -> build/package -> artifact validation -> publish`. Build amd64/arm64 packages once; validate/publish identical checksums. Require deterministic harness checks; numeric gates use controlled hardware or reviewer-attested evidence for the same artifact. Preserve existing CI policy; no signing-key access needed.
- Execute the matrix below with operator/date, commit/checksum, steps, results and defects. Audit installed privileged ownership/permissions, matching schema/service/policy versions and dependency impact against Rust 1.85/floor libraries plus a dated advisory snapshot. Release from a clean recorded commit/lockfile.
- Match claims/release notes to evidence. Define DB backup/compatibility before upgrade. Permit downgrade only after verifying the older binary with the schema or reviewing a compatible backup restore; otherwise mark unsupported. Downgrade cannot reverse cleanup/APT changes.

**Done:** all required work/evidence closes, the memory milestone is demonstrated, required checks pass and reviewers attest installed behavior. Track secondary target misses; explicitly narrow unsupported environment claims. Apply the shared release-blocking rule, not an owner waiver.

### D12 — Repair duplicate discovery and bound scans

**Files:** core `storage/{traverse,duplicates,hash,policy}.rs`, `cleanup/analyze.rs`; app dispatch/storage state/views.

- PR 1: replace fabricated `(device=0, inode=0)` and current-time metadata with real file identity. They currently cause `distinct_inode_count()` to skip every dispatched group. Test through the app/service boundary.
- PR 2: separate duplicate discovery from the top-200 large-file ranking. The UI threshold is already 1 byte. Collect eligible regular-file metadata within declared scope/D0 limits; incomplete coverage cannot report "no duplicates". No persistent scan index.
- Bound frontier/path bytes, issues, hard-link indexes, candidates/results and UI copies. Share cancellable traversal with cleanup size calculation; errors cannot become zero bytes. Report inspected/skipped counts and stopping reason; avoid stack exhaustion.
- Hash validated regular-file descriptors; compare descriptor metadata before/after reads. Preserve staged hashing, fixed buffers, hard-link deduplication and between-read cancellation. Handle growing/changing files; kernel-blocked I/O cannot promise immediate cancellation or release its occupied worker slot.
- Keep duplicate selections empty until chosen. D7 revalidates deletion; a digest is not authorization and oldest does not mean expendable.

**Tests/done:** equal files appear via the app; same-size unequal files do not; hard links are not extra recoverable copies. Find a pair below 200 larger unique files. Cover limits, long/non-UTF-8 paths, unreadable/overlapping roots, directory/symlink swaps, same-size edits, deep/wide trees, cancellation and resource release. Measure peak memory/cancel latency locally; partial coverage survives navigation.

### D13 — Protect reports and persistent data

**Files:** core `reports/{model,redact,render_text}.rs`, `persistence/{database,history}.rs`; app report/DB handling; D7 export adapter.

- Inventory preview, clipboard, saved reports, errors and history. Current redaction copies unsafe service/event/capability text. Prefer typed safe summaries; omit unrestricted diagnostic text by default. Regex cannot recognize every secret.
- Use one immutable redacted preview revision for copy/save. Policy/regeneration changes invalidate controls; reject stale async exports. Persist structured error categories with correct JSON escaping; cap report/error data and never log unsanitized inputs.
- Resolve unset/empty/relative XDG values correctly. Secure the owned directory before SQLite open; reject ownership/symlink substitutions and verify DB/journal/WAL/SHM permissions. Never chmod unrelated/substituted targets. Preserve DB and monitoring on denied/locked/corrupt/newer-schema failures; no silent reset.
- Keep shipped migrations immutable. Test atomic forward migration, interruption/disk-full failure, ordered settings and retention. Use SQLite backup or verified quiescent copying; test restore/older-schema compatibility. Logical history deletion is not secure erasure from backups/snapshots.

**Tests/done:** seed sensitive values in every typed/free-text field, including custom homes, Unicode and arguments without ` -- `. No seeds reach default preview/clipboard/file/persisted errors, including failure paths. Test XDG variants, hostile DB symlink, permissive directory, read-only/full/locked/corrupt/newer DB, interrupted migration and backup restore through actual entrypoints.

## 4. Validation and handoff

| Evidence / owner | Required record |
|---|---|
| Build / D0, D11 | Commit, lock hash, Rust/GTK versions, package SHA-256, executed/skipped tests and failures |
| Safety / D5-D8 | Approved A1-A3 as applicable; installed allow/deny, stale-target, partial-failure and recovery proofs linked to threat IDs |
| Workflows / D4, D9, D12-D13 | App-path duplicate, availability, ordered settings, preview/export and DB-error regressions |
| Resources / D1, D3, D9, D10, D12 | Cap/overflow/cancel fixtures, combined idle PSS, CPU/startup/GUI evidence, failures/exclusions |
| Support / D11 | Tested artifacts/environments, operator/reviewer, unresolved defects and approved scope changes |

| Environment | Minimum evidence |
|---|---|
| Ubuntu 22.04 amd64, GTK 4.6, Wayland + X11 | Install/upgrade/remove, navigation, missing capabilities, Polkit allow/deny/cancel, cleanup/update/kernel VM cases |
| Ubuntu 24.04 and latest supported Ubuntu LTS at execution | Same critical flows; exact versions |
| Supported Linux Mint Cinnamon + Pop!_OS releases | Installed smoke, auth agent, absent SMART/sensors, update discovery; exact versions/sessions |
| Ubuntu arm64 | Native package/build and installed GUI/auth smoke; hardware performance needs a measured device |
| Named 4 GB device with working GPU | Full D10 protocol and scan/navigation responsiveness |
| Xvfb/software CI | Deterministic regression/smoke/CPU; no hardware-memory certification |

**PR handoff:** work/evidence IDs, affected paths, failing-before/passing-after regression, relevant measurements, approval reference, limitations and recovery/disable behavior. Provide loading, empty, unavailable, permission-required, success, partial-failure, failure and cancelled states where applicable. Deliver terminal results once; late results cannot revive replaced/cancelled jobs.

Run targeted tests first, then the required Linux suite using pinned Rust. `--locked` may download locked dependencies but must not select versions or rewrite the lockfile. These commands have not run in this review.

```bash
set -eu
cargo fmt --all -- --check
cargo metadata --locked --format-version 1
cargo clippy --locked --workspace --all-targets --all-features -- -D warnings
cargo test --locked --workspace --all-features -- \
  --skip regression_tests::storage_apply_state_does_not_reenter_borrowed_state \
  --skip regression_tests::sidebar_is_fixed_pinned_and_updates_active_destination \
  --skip regression_tests::update_execution_buttons_follow_transaction_safety
xvfb-run -a cargo test --locked -p maintainer-app --lib --all-features \
  regression_tests::storage_apply_state_does_not_reenter_borrowed_state -- --exact
xvfb-run -a cargo test --locked -p maintainer-app --lib --all-features \
  regression_tests::sidebar_is_fixed_pinned_and_updates_active_destination -- --exact
xvfb-run -a cargo test --locked -p maintainer-app --lib --all-features \
  regression_tests::update_execution_buttons_follow_transaction_safety -- --exact
cargo build --locked --workspace
cargo build --locked --workspace --release
```

Each skipped GTK case is rerun in its own displayed process, matching current CI isolation. Stop on any failure. Inventory added/renamed GTK cases with `cargo test --locked ... -- --list`; skipped is not passed. Privileged/destructive integration tests require disposable VM snapshots; ordinary tests must never call a live host helper.
