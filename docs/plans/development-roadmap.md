# Maintainer: fast, lightweight native GUI roadmap

Date: 2026-10-30. Baseline: `07f7b9a096c3a84b57b329fca5e4d4d2d93289c1` (1.2.0).
Status: implementation proposal; this review changes documentation, not runtime behavior. No Linux validation performed.
Start at the [documentation index](../README.md). Read with [research and evidence](../research/2026-10-30-engineering-review.md) and [safety design decisions](safety-design-decisions.md). Existing D0-D11 IDs remain stable; D12-D13 cover gaps found in the deeper review. IDs are references, not execution order. New types and files are proposals unless already present in source.

## 1. Outcome and scope

Deliver a maintenance app that consumes little background resource, remains responsive, changes only reviewed targets, and reports what actually happened. Make those properties testable on Ubuntu 22.04 and newer Ubuntu-based desktops.

**Product commitment: fast, lightweight, native GTK4 GUI.** The user's new memory goal is **strictly below 100 MB total idle PSS**, with lower usage explored after that. For this milestone, MB means 1,000,000 bytes: the threshold is `<100,000,000 bytes` (about 95.37 MiB). Count the GUI and all other Maintainer-owned processes. This is a new development goal, not an achieved measurement. The [under-100-MB investigation](../research/2026-10-30-memory-under-100mb.md) defines feasibility tests and ordered experiments. Existing 150 MB targets/220 MB ceilings are historical policy context; satisfying them alone does not complete this goal.

This roadmap improves the app and the reliability of its maintenance workflows. It does not promise that cleaning caches makes Linux faster. Any future host-performance claim needs a named workload and controlled before/after measurements.

Keep Rust 1.85, GTK 4.6 compatibility, existing semantic CSS, local-only settings/history, and the unprivileged GUI. Use existing dependencies first. No automatic duplicate deletion, RAM cleaning, arbitrary commands, new permanent service, or broad feature expansion in this cycle.

Repository rules remain authoritative. Helper API/Polkit changes need explicit approval, a threat review, and authorization tests before implementation. Planning them here is not that approval. Do not widen the maintenance service sandbox as a shortcut. Never edit shipped migrations or Cargo.lock by hand. No signing credentials are needed for this work.

| Core value | Concrete acceptance / owner |
|---|---|
| Fast | Usable shell/navigation within 2 s of launch, responsive navigation during slow I/O, and measured main-loop delay; D4/D9/D10. Record window mapping and first live readings separately; mapping an empty shell is insufficient. |
| Lightweight | Under 100 MB combined idle PSS on the declared hardware profiles; retain CPU target <=0.5%, hardware ceiling 1%, software ceiling 2%, bounded active work and no permanent Maintainer daemon; D0/D3/D9/D10/D12. |
| Native GUI | Rust + GTK4 widgets, semantic GTK CSS, keyboard/screen-reader access, native Wayland/X11 and GTK 4.6 floor. Supported everyday workflows stay usable through the GUI; no WebView/Electron/libadwaita or required terminal workaround; D4/D9/D11. |
| Stable | Missing integrations, denied permissions, cancelled jobs, DB failures and partial results remain usable explained states; no crash/hang or silent replay in required fixtures; D1-D13 as applicable. |
| Safe and private | Normal-user GUI, approved narrow elevation, explicit destructive review, private local history, no telemetry/upload and tested default report redaction; D2/D5-D8/D13. |
| Understandable | Show the measurement/source, age, scope, risk and actual result. Partial inspection is not "healthy" or "no duplicates"; D4/D9/D12/D13. |

Treat 60 FPS as an interaction/rendering aspiration, not a continuous repaint requirement. Keep default monitoring at 2 s and hidden sampling at 10 s. Optimize widget/data lifetime and demand-driven rendering while preserving readable text, accessibility and operation safety. A performance change must pass the other values too.

## 2. Product direction

Monitoring, cleanup previews, privacy, and kernel management already exist elsewhere. See the primary-source comparison in the research note. Proposed position: **one local workflow that connects observed problems to reviewed actions and verifiable results, with published resource costs.** This is a product hypothesis, not a demonstrated exclusive advantage.

| Candidate | Decision this cycle | Evidence required |
|---|---|---|
| Evidence -> explanation -> reviewed action -> verified result | Build on existing capabilities | Each warning cites its measurement/source; each action states scope, reversibility, permission, and outcome |
| Published resource limits on modest hardware | Prioritize | Reproducible release report on a named 4 GB reference device |
| New cleaner catalog, service/repository editor, universal optimizer | Defer | Separate demand and safety case; these increase scope before current guarantees are proven |
| New snapshot/rollback engine | Defer | Explain existing recovery tools; never label APT changes reversible without proof |

## 3. Delivery order and accountability

Assign one named developer and one reviewer to each row before starting. Roles identify expertise, not extra staffing. P0 covers unsafe or broken behavior; P1 covers efficiency and verification. Both are required for this hardening release. Dependencies are completion requirements, not time estimates.

| ID | Priority / accountable role | Work | Depends on |
|---|---|---|---|
| D0 | P0 / release lead | Establish facts and acceptance contract | None |
| D1 | P0 / core developer | Drain and bound child-process output | D0 |
| D2 | P0 / app developer | Refuse root GUI launches | D0 |
| D3 | P1 / core developer | Enforce one application-wide job limit | D0 |
| D4 | P0 / app + core developer | Remove system I/O from GTK callbacks | D0; use D1/D3 as they land |
| D5 | P0 / security + packaging reviewer | Make kernel/package operations safe and executable | D0; approved boundary design |
| D6 | P0 / updates developer | Preserve reviewed transaction semantics and honest cancellation | D1, D5 |
| D7 | P0 / filesystem developer | Protect cleanup, Trash and report saving | D0; approved filesystem design; D12 traversal integration |
| D8 | P0 / helper developer | Bind process termination to a stable process identity | D0; helper approval if its API changes |
| D9 | P1 / GTK developer | Reuse widgets and preserve monitoring identity/state | D3, D4 |
| D10 | P1 / performance developer | Build measurement harness; verify final release | D0; final run after implementation work |
| D11 | P0 / release lead + QA | Test installed artifacts and enforce release gates | D1-D10, D12-D13 |
| D12 | P0 / storage developer | Repair duplicate discovery and bound scan work | D0; integrate D3 admission |
| D13 | P0 / privacy + persistence developer | Close report disclosure and private-storage gaps | D0; integrate D4 DB worker and D7 export |

Execution order: D0; then independent D1-D4/D12 fixes, D13 redaction work and the D5/D7/D8 safety designs; then approved boundary implementations, D6 and D9; finally D10/D11 evidence against the completed artifact. D10's harness starts early. D4's removal of GTK-thread I/O must not wait for the scheduler rewrite. Split broad work packages into the focused PRs described below; do not bundle the whole roadmap into one merge.

If a required item cannot close, defer release or obtain explicit approval to disable/narrow the affected capability and document that change. The owner cannot waive an unsupported safety claim simply by recording it as a known issue. D0 records missing Linux evidence without blocking independent safety fixes; runtime evidence is required before release.

## 4. Work packages

### D0 — Freeze facts and define the measurement contract

**Files:** `README.md`, `AGENTS.md` (read), `CHANGELOG.md`, `.github/workflows/ci.yml` (read), new dated result files under `docs/validation/`.

- Record commit, clean/dirty state, toolchain, fixture versions, distro, kernel, GTK, display server, renderer, CPU/RAM, and exact commands for each run. Keep machine identifiers and personal paths out of shared results.
- Record manifest/lockfile hashes and any dependency changes separately from the release baseline. This workspace already contains libc/Tokio manifest and lockfile edits; their compatibility/advisory status is unverified. Use locked Cargo resolution and the pinned compiler; do not silently update dependencies or claim the baseline commit describes a dirty build.
- Attempt the existing CI-equivalent checks on Linux before changing code. If unavailable, record why and proceed with independent fixes; validate the baseline when Linux becomes available. Record failures and actual executed/skipped test counts. Historical figures and test annotations are not a fresh baseline.
- Preserve AGENTS limits: private/dirty memory <=130 MB and startup <=2 s. This documentation review aligned README's former 150 MB/2.5 s ceilings with those limits; no runtime result is implied. Confirm MB versus MiB before enforcing thresholds; publish raw bytes and both units until resolved. Do not silently choose a more permissive interpretation.
- Separate release blockers, target misses, unsupported measurements, and environment failures. Missing evidence is `unverified`, never `pass`.
- Establish the new memory goal's headroom on the floor and current supported GTK profiles: minimal GTK fixture, app shell, then full app. Do not assume the historical ~135 MB GTK figure is a universal floor. If a matched minimal fixture is already >=100 MB, identify renderer/library costs before promising application-only savings. Record feasibility per environment; no silent toolkit switch or budget waiver.

**Done:** a dated baseline record identifies observed results, missing measurements, and accountable owners. Record the owner's decision for ambiguous units and new proposed criteria before enforcing them. Existing stricter limits remain in force. An unavailable benchmark is not a reason to delay a root guard or another independently testable fix.

**Proposed resource defaults:** ratify or replace these with measured values in D0. They are limits on retained application data, not measured PSS or existing product promises. Test limit-1/limit/limit+1 and record peak process memory.

| Resource / owner | Initial implementation rule |
|---|---|
| Command capture / D1 | Preserve the existing 10 MiB per-stream default; enforce while reading. Frame/line limit 64 KiB; displayed progress/error text 4 KiB. Overflow invalidates structured output; diagnostic capture may truncate explicitly. |
| Admission / D3 | Four active, 16 queued, 24 total accepted-but-unreleased jobs including completed results. Release admission after consumption/drop. Reserve result slots and enforce per-type payload limits; initial aggregate retained-result cap 16 MiB, reduced to fit measured idle headroom. Oversized data gets an explicit bounded failure, never a lost terminal state. |
| Background lanes / D3-D4 | One metric worker and one serialized DB worker, separately accounted. Network metadata uses ordinary admission; move the speed-test raw thread through it. One expensive filesystem scan at a time; never wait for its slot while occupying a worker. |
| Scan retention / D12 | Initial active-work cap: 64 MiB, 100,000 candidates, 1,000 issue samples and depth 256; stop at the first limit. Count paths, indexes, frontier, hard-link data and UI copies. Retained results after completion must also fit measured idle headroom; reduce limits when they do not. Show incomplete coverage and aggregate omitted issues. |
| Status / D3-D4 | One replaceable progress value per job; one latest metric snapshot. Generations and sample times are mandatory. Terminal outcomes use reserved slots and cannot be overwritten by progress. |
| DB/report retention / D4/D13 | 32 outstanding DB requests with nonblocking admission; coalesce superseded settings. One retained preview, maximum 4 MiB after safe rendering; over-limit reports identify omitted sections. No unbounded queue of full reports or raw diagnostic strings. |

For work interrupted after effects may have occurred, distinguish `partial`, `recovery required` and `outcome unknown` from `cancelled`. One terminal notification within a running process does not guarantee exactly-once execution across crashes. Never automatically replay destructive work after a missing response.

### D1 — One bounded command-running implementation

**Files:** `crates/maintainer-core/src/command.rs`, adapters using it; new test-only child executable/fixtures.

- Consolidate ordinary and streaming execution around concurrent stdout/stderr draining. Continue draining after a capture cap; discard excess bytes and mark truncation.
- Bound retained bytes per stream, progress message length, and pending progress. A giant line without a newline must not allocate unbounded memory. Treat progress as replaceable status; preserve required protocol records or fail explicitly on overflow.
- Check deadline/cancellation until process exit, including after stdout closes. Use interruptible reads/polling so a reader join cannot wait forever on an inherited pipe. For ordinary owned children, define a dedicated process-group/descendant policy; never signal the GUI's inherited process group. Surface kill/read/join failures. Privileged-process ownership belongs to D5-D6.
- Keep direct argv invocation, fixed locale for parsers, no shell, existing timeout defaults, and contextual typed errors. Parsers must reject incomplete/truncated transaction plans.
- Inventory every direct process call, including helper adapters and connection utilities. Route ordinary children through the runner; document exceptions for the approved package owner. Resolve production executable paths without trusting a user-writable PATH, use null stdin unless a protocol requires it, and keep parser/environment overrides narrow. An absolute path alone does not validate arguments.

**Regression tests:** child writes >pipe capacity to both streams; 32 MiB without newline; closes stdout then sleeps; descendant holds a pipe; invalid UTF-8; nonzero exit; missing executable; spawn failure; timeout/cancel. Use compiled fixtures, never live APT or shell strings.

**Done:** controlled fixtures terminate within the configured deadline plus a proposed 2 s cleanup bound; retained data obeys caps; one terminal outcome is delivered. No owned fixture child/reader remains. An OS-level uninterruptible process or denied signal is reported as incomplete cleanup, never successful cancellation. Record peak memory for the large-output fixture and test cap-1/cap/cap+1 boundaries.

### D2 — Refuse root before initializing GTK or persistence

**Files:** app `src/main.rs`, `application.rs`; a narrow core identity adapter using the existing libc dependency; package/smoke scripts.

- Check effective UID before GTK application creation, DB access, helper setup, or window creation. UID 0 exits nonzero with a plain explanation to run as the normal desktop user. Do not attempt to drop privileges inside the GUI.
- Keep identity access outside widgets. Do not add a dependency solely for this check.
- Separate package installation as root from GUI launch as a dedicated unprivileged test user. The current package script launches as root and tolerates an early exit; fix both assumptions.

**Tests/done:** injected root/nonroot identity cases plus a disposable-VM root invocation that exits without a display, window, DB creation, or helper call; normal-user installed launch succeeds. The privileged invocation is only a refusal test in that VM, never a developer-host GUI launch.

### D3 — One bounded scheduler for the application

**Files:** core `src/jobs.rs`; app `runtime.rs`, `dispatch.rs`.

- Replace per-`execute()` token pools with shared scheduler state: at most four active on-demand jobs across all callers. Keep the separate metric worker explicit in resource accounting.
- Apply D0's queue and total admission limits, configurable in tests. Submission from GTK returns immediately with accepted/busy status. A bounded queue alone is insufficient: abandoned, completed receivers must not accumulate unlimited payloads. Batch submission reserves all job/result slots or rejects the batch. Do not create a dispatcher thread per submission or hold a worker waiting for its result consumer.
- Cancellation removes/skips queued work; running work cooperates. Reserve a result slot for each accepted job so its terminal outcome cannot be lost when the queue is full. Shutdown rejects new submissions, cancels pending work, and does not block GTK waiting on an uncontrolled task. Package transactions follow D5's supervisor policy, not generic kill-on-shutdown behavior. Catch worker panics without losing capacity.

**Tests/done:** hold four jobs behind a barrier, enqueue 16, then reject job 21 without blocking GTK. Separately retain 24 completed receivers and reject further admission until one is consumed/dropped. Use independent submissions and batches. Queued cancellation runs no work; panic/receiver drop cannot leak capacity; accepted jobs produce one terminal outcome while subscribed; rejected batches execute nothing. Include speed tests and storage jobs in the same saturation test.

### D4 — Make presentation updates pure and startup asynchronous

**Files:** app `monitoring.rs`, `runtime.rs`, `ui/shell.rs`; core `metrics/connection.rs`, `hardware.rs`; proposed core monitoring service module.

- Move `read_connection_details()` out of `MonitoringState::apply_network()`: the reducer currently runs `ip`/`iw` on GTK through that call. Return connection details as typed worker messages. Route fallback utilities through D1 with deadlines.
- Move `collect_device_info()` and DB open/migration/settings/history calls out of GTK startup/callbacks. Render loading values and update affected widgets from worker results. Database failure must preserve monitoring access. Serialize setting writes with revision IDs so an older completion cannot overwrite a newer edit; surface save failures.
- Give DB work its own bounded serialized lane so four long scans cannot starve settings/startup. Keep GTK objects and `RefCell` borrows on GTK; copy typed requests before dispatch, and drop borrows before callbacks. D13 owns file privacy and corruption recovery; D4 owns scheduling and presentation.
- Consolidate duplicated collection in `Runtime::collect_snapshot()` and the shell worker into one core service; widgets receive results, not collector dependencies. Do this only for touched execution paths.
- Keep default 2 s/10 s sampling; apply saved intervals. Proposed connection cadence: on Network entry, explicit refresh, and no more than once per 10 s while visible. One metadata request at a time must not block metric sampling. Keep rates on normal sampling. Use a bounded latest-snapshot mailbox, monotonic timestamps and job generations; coalesce queued requests and reject late/stale results. Cancelled, failed, busy, unavailable and permission-required outcomes remain distinct in presentation.

**Tests/done:** a deliberately slow connection adapter/DB does not stop a GTK heartbeat or navigation; reducers accept fixture messages without system access; saved refresh intervals affect scheduling; worker-start failure produces an unavailable state, not `expect()` termination. Record main-loop delay under the slow-adapter test.

### D5 — Review the privileged package boundary before repairing it

**Files:** helper `server.rs`, `main.rs`, `validate.rs`, `operations/kernel.rs`, `operations/updates.rs`; `data/systemd/maintainer-helper.service`; Polkit policy, app helper client, packaging, threat model.

- First reproduce kernel-removal failure in an installed-package VM. The current sandbox permits cache/journal writes, not dpkg/kernel writes. Also test cache cleaning independently; do not assume every existing action fits its sandbox.
- Complete decision A1 in [safety design decisions](safety-design-decisions.md): compare the installed PackageKit backend, a dedicated package owner, and discovery-only fallback. Keep cache/journal/SMART operations sandboxed and separate package mutations. Do not make `/`, `/usr`, or `/var` writable for all maintenance actions. A supervisor solves lifecycle ownership, not exact-plan binding by itself.
- Approval must settle caller binding, fixed operation schema, authorization per operation, execution ownership, phase-aware cancellation, disconnect behavior, and bounded logs. Default proposal: a short-lived authenticated supervisor owns each package transaction through completion; GUI disconnect does not terminate dpkg configuration or report success. It exits when work/status retention ends. Keep existing API compatibility or version it explicitly. This proposal is not authorization to implement a new helper method.
- Package mutations share an exclusive operation slot; APT/dpkg locks remain authoritative. Never delete lock files or force-kill the process holding them. Resolve denial, timeout and recovery-required states explicitly.
- Define behavior for logout, service restart and system shutdown, not only window close. Before mutation, use the supported logind inhibition mechanism where available and record its limits; privileged override/power loss remain possible. Preserve a minimal private operation receipt for reconciliation, never an automatic retry. Do not copy the maintenance unit's 128M/25% limits onto APT without testing: OOM/timeouts during configuration can damage package state.
- Define bounded safe error responses and audit records before changing helper output. Do not return raw utility stderr/stdout or persist sensitive arguments, paths or credentials. Record action/result categories without claiming a complete audit trail already exists. Test malformed wire calls and pre-validation allocation limits; typed DTOs alone do not prove unknown-field rejection or bounded memory. Recover capacity after failures/panics, and keep blocking calls from defeating request/idle deadlines.
- For kernel removal, rederive the running release at execution; require a fresh reviewed removal plan including dependencies and protected metapackages. Reject running-kernel components and any resolver result that removes them. Do not label an arbitrary installed kernel "known bootable"; retaining a fallback needs an evidence-based policy. Disable removal with a reason until the approved implementation passes installed-VM tests.

**Tests/done:** disposable VM: valid old-kernel removal, running-kernel denial, unauthorized caller, dismissed prompt, stale plan, malformed package, lock contention, and partial package failure. Verify boot files/dpkg state and reboot into the retained kernel. Every other helper action still passes installed sandbox tests. No API/policy implementation before recorded approval.

### D6 — Preserve update intent and truthful transaction states

**Files:** core `updates/{model,apt}.rs`, `command.rs`; app `dispatch.rs`, update state/page; helper package entrypoint after D5.

- Store transaction intent explicitly (`full` versus selected exact package versions); never infer it from candidate counts. Revalidate a selected plan using the same selection resolver, not the full-upgrade resolver. Canonical plan comparison includes installations, versions, architectures, and removals. The current model already includes removals: extend its use, do not build a second competing model.
- APT simulation has no transaction lock and may see different configuration as a normal user. D5 must define how resolved operations are bound to review at the privileged execution boundary. Repeating simulation or pinning only requested packages is insufficient to freeze dependencies. Until a reviewed backend/locking strategy guarantees that binding, refuse transactions that cannot meet it; expose discovery and a clear unavailable reason. APT `--no-remove` is an additional guard for removal-free plans, not a complete solution. Never seize APT locks externally in a way that deadlocks APT itself.
- Verify installed version **and configured installation state**, keyed by package name + architecture; distinguish remove from purge when checking postconditions. Batch dpkg queries where practical. Incomplete/truncated output, half-configured packages, or unresolved postconditions cannot produce success. Preserve configured repository authentication, package holds and downgrade protections; never add unauthenticated/force flags to bypass failures.
- Model authorization, download, installation/configuration, verification, failure, and recovery-required states. Cancel before mutation only when the backend confirms it can stop; after mutation begins, retain progress and explain why stopping is unavailable unless a reviewed backend proves a safe boundary. The UI already restricts controls: preserve that behavior. SIGKILL on timeout is not a package recovery strategy. Test GUI close/disconnect without orphaning work or falsely claiming rollback; only supervisor-confirmed completion can produce a terminal success.

**Tests/done:** selected subset with dependencies; same candidate count/different intent; changed versions/removals; lock held; offline; denied/dismissed auth; partial dpkg configuration; cancellation before/after elevation; close during install. All privileged cases run in disposable VMs. No system update on the developer host.

### D7 — Bind cleanup to reviewed targets; make Trash collision-safe

**Files:** core `cleanup/{plan,execute,trash}.rs`, history interfaces and proposed secure export adapter; app cleanup results and `ui/reports.rs`.

- Document same-user concurrent mutation as the threat. Current inode checks precede later path-based operations; they do not make the whole operation atomic. Define an anchored-directory traversal/move/delete strategy, reject symlink/mount escapes, and test substitution at each boundary. `openat2()` can constrain path resolution; it alone does not solve later unlink/rename races.
- Complete decision A2 before implementing a new deletion protocol. A private quarantine followed by validation can already move an unreviewed replacement; a manifest alone cannot freeze a writer. State the supported concurrency assumptions and prove them, or keep that mutation unavailable. Do not describe check-then-unlink as race-free.
- Define concurrency as filesystem mutation by another writer; do not claim protection against an attacker controlling the GUI's own memory. Require approved roots; reject overlapping/duplicate targets. Recheck type, identity and relevant content metadata. Recursive permanent deletion needs a bounded reviewed descendant manifest or an equivalently reviewed strategy; newly substituted/added descendants are not automatically approved. Check cancellation during traversal. Refuse cases whose identity/scope cannot be preserved, including unsupported filesystems/syscalls.
- Honor `$XDG_DATA_HOME`; validate Trash ownership, permissions and symlinks. Use atomic exclusive metadata creation and a no-replace move strategy; an `exists()` check plus ordinary rename is insufficient. Handle both mount Trash layouts and cross-filesystem refusal. Test cleanup after a failed move so a valid existing Trash entry cannot be removed. An established adapter is acceptable only after equivalent safety is verified; no dependency addition by default.
- Separate bytes moved to Trash from estimated bytes unlinked; do not label either as measured free-space gain. Hard links, snapshots and open files affect physical recovery. Preserve historical records; do not reinterpret old totals as verified values. Use a forward migration if needed. After mutation, return the per-item result even if audit recording fails, flag incomplete audit history, and prevent a blind retry. Reconcile partial/unknown outcomes instead of replaying the plan.
- Apply the same filesystem review to report saving: replace predictable create/truncate temporary paths with exclusive private-file creation and reviewed destination handling. A preexisting temporary symlink or permissive file must never be followed/reused. Keep final replacement explicitly user-approved and handle failed rename/concurrent save without overwriting unrelated data. Perform blocking save work outside GTK through the core adapter.

**Tests/done:** deterministic file/ancestor swap, hostile Trash symlink, name collision, ENOSPC, EXDEV, metadata failure, cancel mid-directory, overlapping selection, history-write failure, hostile export temporary file, concurrent save and rename failure. No out-of-scope changes; permanent fallback requires stronger confirmation and new reports remain private. Use a temporary unprivileged home/mount fixture.

### D8 — Close the process-identity race

**Files:** core `metrics/terminate.rs`; helper `operations/process.rs`, validation/threat model; app termination dispatch/client as affected.

- The current start-time check followed by numeric `kill()` leaves a final race. After authorization, acquire a stable process reference, verify reviewed PID/start time, then send only SIGTERM through `pidfd_send_signal()`. Pin `/proc` access or validate reference liveness so metadata from a recycled numeric PID cannot authorize another target. A pidfd acquired after review still needs that review-identity check. Reject mismatches/exits; never retry by numeric PID.
- Audit both implementations: the shipped GUI calls the helper; the core normal-user function is exported but has no app callsite. Consolidate retained functionality behind one identity/FD adapter, or remove an unused unsafe route after checking consumers. Do not introduce a new GUI termination route incidentally. Write down acquisition/metadata/liveness ordering; helper API changes need approval. Preserve denied/read-failed versus exited states. Signal success means "termination requested", not proof of exit.
- Use APIs available at the Ubuntu 22.04 floor; avoid newer pidfd flags. Unsupported/denied syscall support is a capability state, not an unsafe fallback. Document each necessary unsafe syscall invariant and FD ownership.

**Tests/done:** every retained route: stale identity, target exits/reuses PID between stages, normal authorized child, denied caller, PID 0/1, out-of-range PID, syscall/read denial, exhausted FDs and malformed stat names. No unrelated process is signalled; handles close on every exit path. Integration fixtures are owned test children, never unrelated host processes.

### D9 — Reuse GTK rows; key monitoring data by identity

**Files:** app `ui/{processes,system,network}.rs`, `monitoring.rs`; core temperature cache as needed.

- Replace full process-row rebuilds with identity-keyed updates or GTK 4.6-compatible ListView/factory recycling. Identity is PID + start time, not row index. Preserve sorting, focus, selection, scroll position, and confirmation target as processes change.
- Reuse disk/sensor widgets by stable identity. CPU rows already update in place when core count matches; preserve that working optimization. Do not claim every System widget currently rebuilds.
- Measure eager page construction in `Shell::new()`. Build secondary pages on first use; retain typed state independently of widgets. Test releasing inactive heavy views/results where profiling proves value, with bounded caches and explicit ownership of signal/timer handles. Preserve focus, navigation state and active-job progress. Measure after visiting every page; startup-only savings do not prove sustained low memory.
- Key network history by interface identity rather than vector position; remove stale interfaces. Refresh sensor topology on explicit refresh or bounded rediscovery after disappearance. Keep unavailable/permission-required values and sample ages explicit; an old CPU/memory value must not remain labelled live after failure. Stop hidden-window widget repaints while retaining bounded sampling/history.

**Tests/done:** 1,000 synthetic processes with reorder/reuse; interface reorder/hotplug; sensor removal; available->unavailable->available. Unchanged samples preserve row objects; actions retain correct identity. Record CPU, allocation/row creation counts, and main-loop delay before/after; do not accept a speedup that exceeds memory budgets.

### D10 — Measure the actual contract

**Files:** `scripts/measure-performance.sh`; proposed minimal GTK baseline fixture and validation report format; startup instrumentation outside widgets.

Execute M0-M6 in the [memory investigation](../research/2026-10-30-memory-under-100mb.md), in order of measured contribution. Do not add their individual savings together as if independent; measure the combined candidate. Lower exploratory checkpoints are <75 MB and <50 MB, not promises or reasons to weaken the native GUI/safety contract.

- Read PSS and Private_Dirty from `/proc/<pid>/smaps_rollup`, with documented smaps fallback. Parse kernel kB fields as 1,024 bytes; report RSS and Swap/SwapPss separately. Count every Maintainer-owned process, including separately activated helpers, with stable PID/start-time identity. Report unavailable GPU/driver accounting explicitly; PSS alone is not all GPU/system memory. Do not improve the number by swapping pages out, preloading shared libraries in another process or omitting workers. The matched GTK delta is diagnostic, not exact attribution or a universal minimum.
- Define the startup clock and endpoint in the harness. Start immediately before the parent spawns the child (report spawn overhead); correlate inherited-FD markers with monotonic time. Record map, usable shell and first live data separately. The usable-shell marker requires visible navigation/loading states and a successfully processed navigation canary; mapping alone is insufficient. Record frame/presentation evidence separately: GTK map does not prove compositor presentation. Reject preexisting single-instance launches; never infer readiness from RSS. Keep instrumentation off unless explicitly enabled for testing.
- Detect/report rendering mode from the actual session. Unknown mode is unverified. Never turn a missing measurement into zero. Benchmark as a normal user; do not drop system caches or run the GUI as root. Fix window size, scale, theme, page data/cardinality, power profile and session services; record deviations.
- Add machine-readable pass/target-miss/ceiling-fail/environment-error/unverified output and retain sanitized raw samples. Fixture-test parsers and startup/exit/error handling. The harness is explicitly requested validation, not production telemetry/history.

**Required protocol:** release build; fixed normal-user session; 30 s settle; 300 s sampling; three runs per page (Overview/System/Processes/Network), plus hidden/minimized cases originating from Overview and Processes. Capture memory every 5 s; report median and maximum. Report combined owned-process CPU time/wall time (100% = one core), with GUI/helper/child breakdown and no double counting. Account for short-lived children through the owner; parse `/proc/<pid>/stat` safely around parenthesized names. State instrumentation overhead and separately observed shared-service costs. No scan/speed test/update runs during idle measurements. Run a 30-minute soak plus ten navigation cycles; publish memory trend, not a universal leak-free claim.

Measure ten fresh-process launches after cache warmup and at least three separately recorded post-reboot cold launches. Pair baseline/candidate measurements on the same device; alternate order for warm runs. Keep crashes, timeouts and budget failures as failures. Exclude environmental runs only by predeclared criteria applied to both versions; retain the raw run and reason. Do not relabel slow app behavior as background noise or call repeated warm-cache launches cold.

**Gates for the new milestone:** combined idle PSS `<100,000,000 bytes` on declared hardware profiles, for each sampled maximum; retain the <=40 MB matched-baseline delta target and <=130 MB Private_Dirty ceiling as additional diagnostics/limits. Average idle CPU target <=0.5%, ceiling 1%; startup-to-usable-shell <=2 s using the declared clock. Apply CPU ceilings to each valid 300 s average and startup ceiling to every valid measured launch. Missing data or >=100 MB means this milestone is incomplete even if the old 220 MB ceiling passes. Keep lower-confidence hardware results unverified.

Software rendering retains the existing 2% CPU ceiling and memory-reporting exception; it cannot certify a universal under-100-MB claim. Attempt and report memory improvements there too. Published claims must name tested profiles. For older limits retain D0's unit decision; the new 100 MB milestone already uses explicit decimal bytes.

**Additional proposed responsiveness criterion:** main-loop heartbeat p95 delay <=50 ms and maximum <=200 ms during deterministic slow-adapter/scan fixtures on the reference device. This is a new test criterion, not an existing measured result; approve it in D0.

### D11 — Validate installed packages, not just source builds

**Files:** `.github/workflows/ci.yml`, `scripts/test-package.sh`, `tests/system/*`, release metadata, README/changelog/threat model.

- Keep format/lint/tests/build checks. Add artifact installation/upgrade/removal and unprivileged GUI smoke tests in disposable environments with real system bus/Polkit. A source process surviving three seconds is insufficient. Missing required tools or failed launch must fail, not be swallowed.
- Split CI into `check -> build/package -> artifact validation -> publish`. Build each amd64/arm64 package once, record its checksum, then install/test that artifact; publishing downloads those same tested bytes. This avoids a dependency cycle between packaging and tests. Currently packaging publishes after only `check`. Make deterministic harness/behavior checks required; run numeric performance gates on a controlled runner or retain a reviewer-attested hardware report for the identical package checksum. No cryptographic signing/key access is needed to attest results. Do not convert noisy shared-runner timings into false certification or weaken existing checks.
- Run the matrix below; record operator/date, commit/package checksum, steps, results, and defect links. Carry forward `unverified` honestly. Audit changed dependencies and installed privileged file ownership/permissions; do not claim a completed security audit from source-string tests.
- Update inaccurate architecture/security/performance claims and release notes to match tests. Add forward migrations only where needed. Before an app upgrade, define DB backup/compatibility handling. App downgrade is allowed only after testing the older binary against the current schema, or restoring a compatible pre-upgrade DB with explicit review. Otherwise document that downgrade is unsupported. It never reverses completed cleanup or APT changes.
- Review dependency changes against the pinned Rust 1.85 toolchain and floor libraries, plus an identified advisory snapshot. Record direct/transitive impact and accepted findings; a newer version or a green vulnerability scan is not proof of safety. Release from a recorded clean commit/lockfile. Artifacts require matching schema, service and policy versions, not only matching GUI binaries.

**Done:** D0-D10 and D12-D13 close, the under-100-MB milestone is demonstrated on the declared hardware profiles, no unapproved exceptions, required checks green, other target misses tracked, and reviewers attest installed behavior. An interim safety release may be separately scoped, but it cannot be presented as completion of this memory goal. If an environment is not verified, narrow the published support claim explicitly rather than marking its row green.

### D12 — Make scan results correct, complete within scope, and bounded

**Files:** core `storage/{traverse,duplicates,hash,policy}.rs`, `cleanup/analyze.rs`; app `dispatch.rs`, storage state/views.

- First PR: replace the duplicate dispatcher's fabricated `(device=0, inode=0)` and current-time metadata with real discovery identity. Today `distinct_inode_count()` then skips every group. Test through the actual dispatch/service boundary, not only `DuplicateScanner` with hand-built correct candidates.
- Second PR: separate duplicate candidates from the top-200 large-file ranking. The UI already sets a 1-byte threshold, so changing that threshold does not fix the 200-file limit. Stream real candidate metadata into a bounded collector. Scan all eligible regular files within the declared scope until a D0 limit; report stopped/partial coverage, never "no duplicates" for an incomplete search. Do not add persistent scan indexes or SQLite metrics tables.
- Bound frontier/path bytes, issue retention, hard-link indexes, candidates and results, including copies in the GTK model. Deep/wide/unreadable trees must not exhaust memory or stack. Replace cleanup's recursive size helpers with the same cancellable bounded traversal; errors must not become zero bytes. Report inspected/skipped counts and the limiting reason.
- Hash a validated regular-file descriptor and compare metadata on that descriptor before/after reading; do not separately stat a path and then follow its replacement. Preserve staged hashing, fixed buffers, hard-link deduplication and cancellation between reads. Handle growing/changing files explicitly. Cancellation cannot promise to interrupt a kernel-blocked filesystem call; do not free its worker slot while it still runs.
- Keep duplicate selections empty until the user chooses. Bind any removal review to current file evidence and D7; a scan digest does not authorize deletion. No automatic deletion or arbitrary "oldest copy is safe" rule.

**Tests/done:** two equal owned files appear through the app path; same-size unequal files do not; hard links are not extra recoverable copies. A pair ranked below 200 larger unique files is found. Test limits, long/non-UTF-8 paths, inaccessible/overlapping roots, directory/symlink substitution, same-size edits, deep/wide trees, cancellation and released resources. Record peak memory and cancellation latency on a local fixture; incomplete coverage remains visible after navigation.

### D13 — Make privacy and persistence guarantees true at every output

**Files:** core `reports/{model,redact,render_text}.rs`, `persistence/{database,history}.rs`; app report/DB error handling; D7 save adapter.

- Inventory fields reaching preview, clipboard, saved reports, errors and history. Structured redaction currently protects named fields but copies some service descriptions, event templates and capability reasons. The residual scrubber does not remove arbitrary usernames/hostnames/serials or every sensitive path/argument. Prefer safe typed summaries and omit unrestricted diagnostic text by default; do not promise a regex can recognize every secret.
- One immutable redacted preview revision feeds copy and save. Regeneration/policy changes invalidate earlier controls; async completions cannot export an older revision by accident. Use structured error categories in persistent history; serialization must correctly escape quotes, backslashes and control characters. Cap retained report/error data and do not log the unsanitized input while diagnosing failures.
- Resolve empty/relative XDG values according to the specification. Secure the application-owned directory before opening SQLite; reject unsafe ownership/symlink substitutions and verify DB/journal/WAL/SHM permissions where present. Do not chmod a substituted target or unrelated parent directory. Keep monitoring available when persistence is denied/locked/corrupt/newer than supported; preserve the DB and show the error instead of silently resetting it.
- Keep shipped migrations immutable. Test atomic forward migration, interruption/disk-full failure, settings revision order and retention behavior. Use SQLite's backup mechanism or a verified quiescent backup procedure; copying a live DB file alone is not a recovery design. Test restore/older-schema compatibility before offering downgrade. Logical history deletion is not secure erasure from backups or filesystem snapshots.

**Tests/done:** seed distinct sensitive values in every free-text/typed field, including custom-home paths and argument formats without ` -- `; preview/clipboard/file/default persisted errors contain none of those seeds. Repeat with permission/error paths and Unicode. Test XDG unset/empty/relative/custom, hostile DB symlink, permissive existing directory, read-only/full/corrupt/newer DB, migration interruption and backup restore. Evidence must cover actual report and persistence entrypoints, not only scrubber helpers.

## 5. Validation matrix and handoff

| Release evidence | Required content / owner |
|---|---|
| Build identity / D0, D11 | Clean commit, lockfile hash, Rust/GTK versions, exact package SHA-256, executed/skipped tests and failures |
| Boundary safety / D5-D8 | Approved A1-A3 decisions as applicable; installed allow/deny, stale-target, partial-failure and recovery evidence tied to T IDs |
| Correct workflows / D4, D9, D12-D13 | App-path regressions for duplicate results, availability, ordered settings, safe preview/export and DB failure |
| Resource behavior / D1, D3, D9, D10, D12 | Cap/overflow/cancel fixtures, under-100-MB combined idle measurements, CPU/startup/GUI evidence; failures and exclusions retained |
| Release/support / D11 | Exact tested artifacts, environment matrix, operator/reviewer, unresolved defects and any explicitly approved disabled capability |

Every required row must be satisfied or have an explicitly approved narrower release scope. A secondary target miss has an owner; missing the new memory milestone, an unresolved safety issue, ceiling failure or missing required evidence blocks this milestone's release. Link existing results instead of copying checklists into every PR.

| Environment | Minimum evidence |
|---|---|
| Ubuntu 22.04 amd64, GTK 4.6, Wayland + X11 | Full install/upgrade/remove, navigation, missing capabilities, Polkit allow/deny/cancel, cleanup/update/kernel VM cases |
| Ubuntu 24.04 and latest Ubuntu LTS supported at execution | Same critical flows; record exact versions, do not copy old matrix labels |
| Linux Mint Cinnamon + Pop!_OS representative supported release | Installed smoke, auth agent, unavailable SMART/sensors, update discovery; record exact distro/version/session |
| Ubuntu arm64 | Native package/build tests plus installed GUI/auth smoke; hardware performance remains unverified without a measured device |
| Named 4 GB device with working GPU | D10 full protocol; idle/hidden behavior; scan/navigation responsiveness |
| Xvfb/software-rendered CI | Deterministic regression/smoke/CPU checks; no hardware-memory certification |

Per PR, supply: work ID; evidence IDs from the research note; failing-before/passing-after regression; affected paths; measurements when relevant; boundary approval reference; limitations; rollback/disable behavior. Implement explicit loading, empty, unavailable, permission-required, success, partial-failure, failure and cancelled states where the operation supports them. Terminal results are delivered once; late results cannot revive cancelled or replaced jobs.

Run on Linux using the pinned toolchain. `--locked` is intentional: validation must fail rather than rewrite Cargo.lock. Resolution may download already-locked dependencies; it must not select new versions. These commands are not claimed to have run in this review.

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

The three isolated GTK tests above match the current `.github/workflows/ci.yml`; each skipped workspace case is rerun explicitly with a display. Check every exit code and stop on failure. Inventory new GTK tests and isolate them similarly; a skipped display test is not a pass. Verify exact test names with `cargo test --locked ... -- --list` when implementation adds or renames them. Run targeted tests first, then the required suite once per final change. Destructive/privileged integration tests require disposable VM snapshots; ordinary tests must not reach a live host helper.

Completion means the exact release artifact proves the under-100-MB native GUI milestone and required safety/CPU/startup behavior on the declared profiles. Secondary targets and untested environments stay explicitly labelled; an absence of observed failures is not universal safety proof.
