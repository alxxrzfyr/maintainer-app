# Evidence supporting the development roadmap

Research date: 2026-10-30. Source baseline: `07f7b9a096c3a84b57b329fca5e4d4d2d93289c1`; reconciled with `main` at `79b42ac27c9ba7f1e591304305aee1966288a0b2`.
Companions: [implementation roadmap](../plans/development-roadmap.md), [under-100-MB research](2026-10-30-memory-under-100mb.md) and [safety decisions](../plans/safety-design-decisions.md). Navigation: [documentation index](../../README.md).

## Method and limits

Inspected source paths, tests, CI, packaging, threat model, release notes, and historical performance commits. Checked relevant upstream specifications, Rust/GTK documentation, Ubuntu 22.04 manuals, and comparable projects' own documentation. Compared positive claims against contrary evidence; no third-party performance rankings used.

This machine is Windows; Rust/cargo are absent from PATH and WSL reports no installed Linux environment. No new builds, tests, privileged operations, hardware benchmarks, independent penetration test, or dependency advisory audit were performed. Source inspection establishes control flow/configuration, not measured frequency or exploitability. No competitor was benchmarked. Online documentation is current at access time; newer API flags must not be assumed to exist on Ubuntu 22.04.

At `79b42ac`, Rust source still matches the source baseline; main now includes the dependency updates from `c0edad2`. Current pins are libc `0.2.191` and Tokio `1.54.1`. Current Cargo.lock SHA-256: `3a56d29c8936b65830ecd38ec5f29f78c170c2b17d9d8ef179c4a67bf520a238`. These are committed dependencies, not a dirty overlay. No fresh runtime/compiler validation or advisory audit was performed here; the findings describe source, not a tested build.

The focused plan recheck retraced E1/E2/E5/E17 and checked the primary documentation for Linux memory accounting, GTK threading, APT simulation, systemd sandboxing, file/process handles, PackageKit and SQLite backup. Contrary evidence remains decisive: PSS is not all system/GPU memory; simulation does not lock an APT transaction; no-replace rename protects the destination, not a previously reviewed source identity; PackageKit lifecycle support does not prove exact-plan binding. These limits remain explicit design/test requirements, not assumed fixes.

## Local findings, including corrections to the retrospective

F = directly observed in source/configuration; I = inference requiring runtime validation; H = historical report, not independently reproduced. Paths are relative to the repository root.

| ID / class | Evidence and consequence | Roadmap |
|---|---|---|
| E1 / F | App `src/main.rs` and `application.rs` initialize normally without a root identity guard. This proves missing app enforcement, not that every desktop permits a root GUI session. | D2 |
| E2 / F+I | Helper `server.rs::do_remove_kernel` calls `operations/kernel.rs` inside the maintenance service. The unit sets `ProtectSystem=strict` and explicitly allowlists `/var/cache/apt` and `/var/log/journal`; required dpkg/boot paths are not allowlisted. Kernel purge is expected to fail on those writes; reproduce with the installed unit. | D5 |
| E3 / F+I | Core `command.rs::SystemCommandRunner::run` polls exit before draining pipes. Pipe saturation can stall a verbose child until timeout. `run_streaming` improves draining but retains unbounded stderr/line allocations, and waits without deadline checks after stdout EOF. | D1 |
| E4 / F | Core `jobs.rs::execute` creates tokens and dispatcher threads per call. The tested four-worker limit is per batch, not across callers. | D3 |
| E5 / F | App `monitoring.rs::apply_network` calls core `read_connection_details()`, which executes `ip`/`iw` via blocking `.output()`. The shell invokes the reducer on GTK. `DeviceInfo::default()` also collects system data synchronously. A worker for other metrics does not remove these main-thread paths. | D4 |
| E6 / F | Process rows are recreated (maximum 100 displayed) in `ui/processes.rs::update`; disks/temperature widgets also rebuild. Contrary evidence: `ui/system.rs::update_cpu_detail` already reuses core bars when topology matches. Optimize remaining churn rather than replacing all UI code. | D9 |
| E7 / F | Script measures RSS, declares startup when RSS exceeds 1 MiB, and does not read PSS/private-dirty. Performance CI is nonblocking and packaging depends only on `check`, not smoke/performance. | D10-D11 |
| E8 / F+I | Helper `operations/process.rs` and exported core `metrics/terminate.rs` both check start time then call numeric `kill`. The actual GUI invokes the helper; no app callsite uses the core function. Core `read_stat().ok()` also collapses read failures into absence. Cleanup and Trash have separate path/name race windows. Fix retained implementations without claiming two wired GUI routes. | D7-D8 |
| E9 / F | App `dispatch.rs` revalidates using full-upgrade discovery even for selected plans; execution chooses full/selected by candidate count. Core `UpdatePlan` already models removals and hashes them: the gap is intent/execution correspondence, not absence of removal modeling. | D6 |
| E10 / F+I | Updates use pkexec->helper->exec APT. Streaming cancellation ignores kill errors then waits. Preserving PID through exec does not grant the normal user permission to signal root. UI already restricts transaction controls; cancellation guarantees need testing by phase, not an assumption that Cancel is always exposed. | D6 |
| E11 / F | Cleanup counts same-filesystem Trash moves as recovered bytes. The file still occupies that filesystem. `history.record(...)?` can return an overall error after successful mutations. Its hand-built error JSON escapes quotes but not backslashes/control characters, so some errors cannot round-trip correctly. | D7/D13 |
| E12 / F | Package script requires root and launches GUI in that context; early GUI failure is nonfatal. Security/compatibility checklists are unfilled templates. This means no recorded sign-off there, not proof nobody ever tested. | D2, D11 |
| E13 / H | CHANGELOG records ~10%->0.9% and separately 2.79%->1.39% CPU under software rendering. Commit `67a6aaf` records ~194 MB PSS/~104 MB private-dirty and acknowledges hardware CPU is provisional. Different application states and incomplete raw data prevent treating these as a current 1.2.0 benchmark. | D0, D10 |
| E14 / F | `monitoring.rs::apply_cpu/apply_memory` change Overview's availability on failure but retain System's prior snapshot; `ui/system.rs::update` still renders that snapshot as available. Preserve history if useful, but do not label it as a current reading. | D9 |
| E15 / F | Helper `server.rs::to_dbus_error` stringifies errors; operation adapters include raw utility output. DTOs have no `deny_unknown_fields`, and source logging does not establish a complete authorization audit trail. Former threat-model guarantees on these points were unsupported. | D5/D11 |
| E16 / F+I | App `ui/reports.rs::atomic_save` opens a predictable sibling `.tmp` with create/truncate and no exclusive/no-follow check. Existing symlinks/files can affect what is overwritten or which permissions survive. Controlled hostile-directory fixtures are required; atomic final rename alone does not close this gap. | D7 |
| E17 / F | App `dispatch.rs::dispatch_duplicate_scan` takes only `ScanSummary.large_files` (top 200), assigns every candidate `(device=0, inode=0)` and invents its modification time. Core `DuplicateScanner::scan` skips groups with fewer than two distinct identities: every such dispatched group is skipped. The shell already lowers the size threshold to 1 byte; changing the threshold alone cannot repair this. Module tests construct real identities, missing the broken integration. | D12 |
| E18 / F+I | Storage's top-file ranking and hash buffers are bounded, but traversal frontier, issue vectors and hard-link paths have no aggregate byte cap. `DuplicateScanner` groups all supplied candidates before processing groups. Cleanup's recursive size helpers lack inner cancellation and use zero/fallback sizes on errors. Large-tree resource use/cancellation therefore need their own controls; actual peak memory is unmeasured. | D3/D7/D12 |
| E19 / F+I | `reports/redact.rs` copies service descriptions/capability reasons and most event text. `scrub_process_args` handles only ` -- `; residual scanning covers address/email/home-path patterns, not arbitrary usernames/hostnames/serials or non-home sensitive paths. Rendering exports these free-text fields. The default all-fields privacy claim is unsupported; seed whole-report fixtures before claiming closure. | D13 |
| E20 / F+I | `persistence/database.rs` accepts empty/relative XDG_DATA_HOME, creates directories with inherited permissions, opens SQLite by path and only then chmods that path. This does not establish a private directory or substitution-safe open. The migration runner uses per-migration transactions but does not refuse a schema newer than its maximum. No live DB attack was attempted. | D4/D13 |
| E21 / F | `ui/network.rs` launches speed-test work through a raw thread, outside JobExecutor. `SharedProgress` already coalesces ordinary progress into one slot, but a shared executor fix alone would not cover every on-demand job. | D3 |
| E22 / F+I | `ui/shell.rs::Shell::new` constructs all ten pages before presenting the window. Lazy creation is a concrete experiment; its PSS/startup savings are unknown. Sparkline data is already bounded and redraws on updates, so replacing every chart is not an evidence-based first step. | D4/D9/D10 |

Important counterevidence: bounded request coalescing, visible-page refresh, 60-sample rings, cached hwmon metadata, authorization policy consistency tests, start-time/kernel guards, and installed-version verification already exist. The roadmap repairs their specific gaps. Test annotations are not executed pass counts; source-string assertions are not installed authorization tests.

E17 is a deterministic control-flow finding, not a reproduced GUI result. E18 does not mean every scan exhausts memory. E19 identifies specific escape routes, not proof that every report leaks. These distinctions determine the regression fixtures; they do not justify leaving the gaps unowned.

## Core values and feasibility

**Fast, lightweight, native GTK4 GUI remains the product direction.** The user tightened the memory goal to below 100 MB; it is no longer sufficient to meet only the former 150 MB target. The new milestone means `<100,000,000` combined idle PSS bytes, with <75/<50 MB exploratory checkpoints. [The memory investigation](2026-10-30-memory-under-100mb.md) defines M0-M6 and the limits of that claim.

| Value | What is established | What remains to prove |
|---|---|---|
| Native graphical app | Cargo selects GTK4 with a GTK 4.6 floor; widgets/CSS implement the interface | Preserve keyboard/screen-reader usability and native Wayland/X11 behavior during optimization; no terminal-only replacement |
| Fast | Worker collection, bounded rings and visible-page updates already exist | Remove remaining GTK-thread I/O, measure usable startup and main-loop latency; 60 FPS is an interaction aspiration, not idle repainting |
| Lightweight | No permanent Maintainer daemon is intended; historical measurements and source optimizations exist | Fresh total-process PSS/CPU evidence. The ~135 MB GTK estimate is not a universal lower bound; under 100 MB is unproved, not ruled out |
| Stable, safe, private | Capability states, authorization checks, confirmations and structured redaction exist | Close the specific E/T findings through app-path and installed tests; do not trade those protections for a lower benchmark |
| Useful maintenance | Monitoring, diagnosis and reviewed actions address real system tasks | Attribute any claimed computer-speed improvement to a named workload; deleting data is not itself proof of faster Linux |

Feasibility depends on the measured renderer/toolkit baseline and actual retained app state. At 0.5% idle CPU, a 2 s interval permits only 10 ms average CPU time across all app threads. PSS headroom must be measured independently of binary/heap size. These constraints explain the plan's demand-driven GUI, bounded work and explicit memory experiments; they do not guarantee the target before Linux validation.

## Primary sources and design implications

Each link supports the adjacent narrow claim. Design recommendations remain proposals, not claims that the source endorses Maintainer.

| Source | Verified fact / implication |
|---|---|
| [Rust process documentation](https://doc.rust-lang.org/std/process/) | Pipe buffers can cause deadlock if output is not consumed. D1 must drain while running; simply increasing timeouts does not repair it. |
| [Ubuntu 22.04 systemd.exec](https://manpages.ubuntu.com/manpages/jammy/man5/systemd.exec.5.html) | Strict filesystem protection makes the hierarchy read-only except documented exceptions/allowlisted paths. Supports E2; a package operation needs a separately reviewed boundary. |
| [Linux proc documentation](https://docs.kernel.org/filesystems/proc.html) | PSS apportions shared pages; smaps_rollup aggregates mapping statistics. RSS/PSS/private-dirty are different metrics. Supports separate reporting, not a universal GTK memory floor. |
| [GTK threading](https://docs.gtk.org/gtk4/section-threading.html) | Widgets generally belong on the main thread; asynchronous I/O or workers keep blocking work away from it. Some immutable objects are thread-safe; this does not make widget mutation from workers safe. |
| [GTK list architecture](https://docs.gtk.org/gtk4/section-list-widget.html) | Views recycle a limited number of list items. This is an available optimization route; benchmark it against keyed row reuse rather than assuming it automatically fixes all update costs. |
| [Ubuntu 22.04 APT manual](https://manpages.ubuntu.com/manpages/jammy/man8/apt-get.8.html) | dist-upgrade can remove packages; simulation disables locking and nonroot configuration access may differ. Review hashes alone do not guarantee an atomic reviewed transaction. APT's no-remove option can reject removals, but cannot implement every exact-plan guarantee. |
| [pkexec manual](https://www.freedesktop.org/software/polkit/docs/latest/pkexec.1.html), [kill syscall](https://man7.org/linux/man-pages/man2/kill.2.html) | pkexec normally elevates to root; signal delivery is permission-checked. Neither PID continuity nor parenthood proves the GUI can cancel elevated work. |
| [Jammy pidfd_open](https://manpages.ubuntu.com/manpages/jammy/man2/pidfd_open.2.html), [Jammy pidfd_send_signal](https://manpages.ubuntu.com/manpages/jammy/man2/pidfd_send_signal.2.html) | PID descriptors identify processes and can deliver signals without a numeric-PID lookup at send time. Baseline flags are zero; permission checks and review-identity checks still apply. |
| [openat2 syscall](https://man7.org/linux/man-pages/man2/openat2.2.html) | Anchored resolution can constrain symlinks/escapes. An opened path is not an automatic atomic check-and-unlink solution. D7 still needs a complete move/delete design and adversarial tests. |
| [FreeDesktop Trash specification](https://specifications.freedesktop.org/trash/latest/) | Defines XDG home Trash, mount Trash checks, unique destination names, and confirmation before permanent fallback. Trashing preserves recoverable content; do not count it as physically freed storage. |
| [GTK map signal](https://docs.gtk.org/gtk4/signal.Widget.map.html) | The signal marks mapping/visibility in GTK, not proof of a frame presented by the compositor. Record that endpoint accurately and measure content/frame readiness separately. |
| [renameat2 manual](https://man7.org/linux/man-pages/man2/rename.2.html) | No-replace prevents overwriting a destination on supported filesystems; it is not a compare-source-inode-and-rename guarantee. A2 requires a full concurrency argument. |
| [PackageKit transaction interface](https://www.freedesktop.org/software/PackageKit/doc/dbus-org.freedesktop.packagekit1.Transaction.html), [backend concepts](https://www.freedesktop.org/software/PackageKit/doc/concepts.html) | Transaction state and backend-controlled cancellation are existing mechanisms worth evaluating. Latest docs identify version 1.99.0; they neither prove Jammy backend behavior nor immutable plan binding. This challenges building a custom supervisor before evaluating an existing owner. |
| [XDG base directory specification](https://specifications.freedesktop.org/basedir/latest/) | Paths must be absolute; empty/unset data-home uses the default. Existing unrelated directory permissions should not be rewritten. D13 needs valid path resolution and an application-owned private directory. |
| [SQLite backup documentation](https://www.sqlite.org/backup.html) | The online backup API supports a consistent snapshot while a source DB is active; locking/busy/restart behavior still needs handling. A raw copy during writes is not a validated recovery procedure. |
| [Cargo metadata / locked resolution](https://doc.rust-lang.org/cargo/commands/cargo-metadata.html) | `--locked` rejects a missing lockfile or resolution requiring changes. It prevents silent resolution drift; it does not prove compiler compatibility or vulnerability freedom. |
| [systemd 249 inhibition](https://www.freedesktop.org/software/systemd/man/249/systemd-inhibit.html) | Sleep/shutdown can be coordinated during critical work; delay limits and privileged override prevent an absolute interruption guarantee. D5 must address restart/power-loss recovery too. |

## Comparable tools: what is and is not different

Selection: one system monitor, one cleaner, one integrated utility, and one distribution update manager. This is a capability comparison, not exhaustive market research or a security ranking.

| Project / primary source | Existing strength | Maintainer implication |
|---|---|---|
| [GNOME System Monitor](https://apps.gnome.org/SystemMonitor/) | Processes, resource graphs, and process controls | Monitoring alone is not differentiation; integrate actionable explanation without worsening idle cost. |
| [BleachBit](https://www.bleachbit.org/features) | Cleanup preview/delete workflow, privacy features, no telemetry | Preview and privacy are shared attributes. Test recoverability and accurate outcomes rather than claiming uniqueness. |
| [Stacer repository](https://github.com/oguzhaninan/Stacer) | Integrated monitoring/optimization concept; upstream README explicitly says abandoned | Integrated tools have precedent. A maintained release/test discipline is a proposed advantage; abandonment alone proves nothing about current fork safety or speed. |
| [Linux Mint Update Manager](https://linuxmint-user-guide.readthedocs.io/en/latest/mintupdate.html) | Software/security updates, kernel management, snapshot/recovery guidance | Kernel controls are established. Respect distro recovery practices; do not promise Maintainer can roll back APT changes. |

No evidence currently supports "fastest," "safest," unique preview/privacy, or whole-computer speedup claims. Rust and GTK are implementation choices, not performance/security results. Retain the no-upload product contract; the user-confirmed speed test already contacts Cloudflare, so absolute "no network ever" wording would also be wrong.

After hardening, test the integration hypothesis with three matched tasks: explain a resource symptom, find a known duplicate pair in a mixed tree, and prepare a privacy-safe diagnostic report. A proposed six consenting participants can reveal usability failures, not population-wide superiority. Record versions, task completion, wrong actions, time and assistance; compare applicable tools on the same fixture/session, and disclose features they do not offer. Do not ask participants to perform destructive host maintenance. This product experiment is optional and does not block the safety release.

## Information gaps and closure rules

| Unknown | Required next evidence / decision |
|---|---|
| Current hardware CPU, PSS, true startup, long-run behavior | D10 on a named reference device, same build/session for comparisons; attach raw sanitized results |
| Under-100-MB feasibility with the native GUI intact | M0-M6: matched GTK baseline, total-process attribution, all-pages-visited/after-job idle results; preserve CPU/startup/accessibility |
| Installed sandbox behavior across operations | D5 VM tests with actual shipped unit/Polkit; source tests cannot close this |
| Safe package ownership/cancellation after elevation | Approved D5 decision, then D6 denial/cancel/disconnect/partial-failure tests |
| Race-resistant cleanup implementation | D7 threat review and deterministic substitution fixtures; refuse unsupported cases |
| Duplicate discovery beyond module tests / large-tree limits | D12 app-path fixture, below-top-200 pair, declared coverage and measured retained memory |
| Default privacy across free text and persistence | D13 seeded whole-report/error cases and private-directory/DB recovery tests |
| Locked dependencies and toolchain support | D0/D11 locked metadata/builds on Rust 1.85, manifest/lock hashes and dated advisory review |
| MB versus MiB | D0 written decision. README ceilings have now been aligned to AGENTS; numeric unit normalization remains unresolved |
| Supported distro versions, absent tools, hardware/driver failures | D11 matrix with exact versions, capability states, and operator sign-off |
| Whether users value the integrated workflow enough to switch | Later task-based comparison with consenting users; evaluate completion/error rates before expanding feature scope |

## Review corrections and document retirement

Rechecked source/configuration and primary references before finalizing the plan. This was a static review, not a new Linux test pass.

- Queue saturation now submits job 21 after holding four active and 16 queued jobs. The earlier 20-job example could fill capacity without testing rejection.
- D4 can remove GTK-thread I/O before the scheduler rewrite. Missing Linux baseline evidence is recorded; it does not halt independent safety fixes.
- Package creation, artifact validation and publishing are separate stages; validation does not create a packaging dependency cycle or certify rebuilt bytes.
- PID references must still match the reviewed identity. Path resolution alone does not secure later deletion. Package simulation alone does not bind execution.
- DB compatibility is checked before app downgrade. APT mutations are never described as rolled back by app downgrade.
- Benchmark rules now specify sampling, per-run gates, invalid runs and endpoint uncertainty. Proposed new limits still need an explicit decision.
- The global scheduler now covers speed tests and retained completed results, not just active/queued jobs. D12 separately bounds scan data; a four-worker pool alone does not bound memory.
- Duplicate discovery, termination implementations/callsites and end-to-end redaction were traced across app/core boundaries. D12/D13 close findings omitted by the earlier roadmap; the core termination API is not mistaken for a wired GUI route.
- A1 compares existing backend ownership with a new supervisor; A2 rejects inode/path/quarantine shortcuts that lack a complete race proof. Missing approval or proof keeps affected mutation unavailable; planning is not authorization.
- Current PackageKit documentation differs from older examples; the plan requires installed-version checks. No new API or dependency is selected merely because the latest manual exposes it.
- The native GUI/fast/lightweight values are now explicit gates. The new under-100-MB goal has byte units, whole-app accounting and a measured feasibility path; no invented savings or toolkit-memory floor is assumed.

Retired documents are recoverable from Git at the baseline commit. No current application code, shipped policy, migrations, changelog or manual test checklist was removed.

| Removed document | Reason / retained source |
|---|---|
| `docs/superpowers/plans/2026-08-23-implementation-plan.md` | Describes implemented Network/Processes/icon/token/policy work; superseded by the current roadmap. Existing source/tests remain authoritative. |
| `docs/superpowers/specs/2026-08-23-ui-decomposition.md` | Implemented feature design; its ban on collector changes conflicts with the now-needed GTK-thread I/O fix. |
| `docs/superpowers/specs/2026-08-23-polkit-policy-consistency.md` | Describes policy constants, consistency tests and postinst tests already present. Retain current threat model and installed authorization validation. |
| `docs/releases/1.2.0.md` | Duplicates CHANGELOG and says no release tag exists, although local `v1.2.0` exists. Downloadable publication remains unverified; deleting this draft does not establish publication. |

Remaining design choices have named closure tasks. No invented delivery dates, runtime pass claims, or blanket safety guarantees are needed to use the roadmap.

## Retained operational documents

The threat model is retained at its established path because README/AGENTS reference it. It now distinguishes observed controls from partial/open protections and links real test locations. No helper API, Polkit action or service setting was changed by this documentation correction.

Security, compatibility and accessibility files under `tests/system/` are retained as reusable procedures. Their stale pre-v1 labels, obsolete page layout, D-Bus-only assumption and updates-only network assumption have been corrected. They record no passed runs. README's architecture claims and conflicting numerical ceilings were corrected; the corresponding runtime work remains open.
