# Under 100 MB: feasibility and optimization research

Date: 2026-10-30. Status: research/experiment design; no new Linux measurements. Implements the user's tighter goal while preserving **fast, lightweight, native GTK4 GUI**. Work remains in [D0/D3/D4/D9/D10/D12](../plans/development-roadmap.md); this is not a separate roadmap.

## What counts as success

The next milestone is **combined idle PSS strictly below 100,000,000 bytes** across the GUI and every Maintainer-owned worker/helper process. That is 100 decimal MB, about 95.37 MiB. Measure the full native GUI, default 2 s monitoring, normal window size, hardware rendering and the exact installed release artifact. Include warm state after visiting every page and after completing/dismissing a scan/report. State which large results remain retained. Do not certify only an empty startup window.

Lower exploratory checkpoints are <75 MB and <50 MB. Neither is promised. The former 150 MB target and 220 MB ceiling do not meet the new milestone. Existing CPU, startup, safety, accessibility and Ubuntu 22.04 compatibility constraints remain. Software-rendered results are separate: the existing memory exception is not permission to advertise under 100 MB for untested machines.

PSS estimates the app's share of resident RAM, dividing shared pages among processes. It is not RSS, Rust heap size, package size or all graphics memory. Report GUI/combined PSS, Private_Dirty, RSS, Swap/SwapPss, CPU, startup, renderer and available GPU statistics. Include an on-demand helper if it remains alive during the idle sample. If its memory requires an authorized read-only observer, keep the GUI unprivileged; missing helper data makes the aggregate unverified. Activation/shutdown and active-job peaks need separate results. [Linux proc documentation](https://docs.kernel.org/filesystems/proc.html) defines these accounting fields; [DRM statistics](https://docs.kernel.org/gpu/drm-usage-stats.html) are driver-dependent and may be unavailable at the compatibility floor. Do not add GPU totals to PSS without understanding shared-buffer overlap.

## Is it attainable?

**Not yet established or ruled out.** The repository's historical ~135 MB minimal-GTK estimate is one reported environment, not a universal toolkit minimum. No primary source reviewed here establishes a mandatory 100+ MB GTK4 footprint. That absence is also not proof that Maintainer can fit below 100 MB on every supported driver/session.

Let `B` be the matched minimal GTK fixture's measured PSS. Diagnostic headroom is `100 MB - B`; this helps choose experiments, but is not exact attribution because PSS sharing/caching can change. Example only: `B = 65 MB` leaves less than 35 MB for the measured increment; allocating the old 40 MB allowance would miss the goal. If `B >= 100 MB`, investigate toolkit/renderer mappings before promising that trimming application data will suffice. Final combined measurements decide, not the subtraction.

There is also a CPU constraint: 0.5% of one core is only 10 ms of CPU time per 2 s on average, including GTK, collection and rendering. The 1% hardware ceiling permits 20 ms. This arithmetic is a budget, not a runtime prediction. A memory reduction that exceeds CPU/startup limits is not an acceptable solution.

## Experiments, ordered by evidence

Run attribution/profiling separately from uninstrumented acceptance measurements. Use one controlled change at a time, repeated paired runs, then retest the combined candidate; savings can overlap. Keep failed experiments and exact build/environment identities. Tool installation/dependency changes need a stated purpose; no privileged profiling of unrelated user workloads.

| ID / work | Concrete experiment | Keep only if |
|---|---|---|
| M0 / D0, D10 | Build a minimal GTK4 window using the same Rust/GTK versions and release settings; compare with the app shell and full app on the Ubuntu floor and a current supported GTK profile. Match dimensions, scale, theme, renderer and session. Record every process and raw smaps samples. | The measurement is repeatable and identifies whether memory is primarily toolkit/driver, app data or retained UI. No guessed baseline subtraction. |
| M1 / D10 | Attribute anonymous/private memory, stacks, mapped code/libraries and graphics mappings. Use an available native heap/CPU profiler to identify allocation owners and lifetimes; include Rust and GTK/GLib allocations. Count live widgets, timers, channels and retained results. | The proposed optimization addresses a measured contributor. Heap totals and instrumented PSS are not substituted for release PSS. |
| M2 / D4, D9 | `Shell::new()` eagerly constructs all ten pages. Prototype lazy secondary-page creation; then bound/release inactive heavy view caches while retaining typed state. Audit strong-reference cycles and disconnect page-owned signals/timers. | Under-100-MB progress persists after all-page navigation; widgets actually release; focus/selection/jobs remain correct and first navigation stays responsive. |
| M3 / D3, D12, D13 | Remove duplicate snapshots/text/model copies identified by M1; coalesce messages, reuse bounded buffers, release scan intermediates and cap retained results/preview bytes by actual idle headroom. Account for active and completed jobs. | Correct scope/results/privacy remain intact, saturation stays bounded, and memory returns to a stable measured range after work. Incomplete results must be labelled. |
| M4 / D9, D10 | On each installed GTK version, compare its supported default/alternative renderers in isolated test sessions. Measure scale/window-area effects and actual repaint frequency; reduce expensive offscreen effects/textures only when the profile identifies them. | Total resource use, CPU, startup and visual/accessibility checks improve together on the intended profile. No forced global renderer, reduced text scale or disabled accessibility to obtain a smaller number. |
| M5 / D0, D10, D11 | Inspect `cargo tree --locked --workspace -e features` and mapped code. Test justified feature reductions and separate release-profile variants: opt-level 3 versus s/z, thin LTO and codegen-unit changes. Keep production policy unchanged until reviewed. | Exact-artifact PSS/startup/CPU improves, Rust 1.85/floor builds and behavior pass. Smaller binaries alone are insufficient. Preserve panic-unwind recovery; `panic=abort` would defeat current worker panic handling. |
| M6 / D3, D10 | Only if M1 shows allocator retention after objects drop: compare fewer idle threads/reused arenas and supported allocator settings in an experiment. Audit stack requirements before reducing thread stacks. | Repeated scan/navigation cycles improve resident memory without stack failures, allocator bugs or extra CPU stalls. No periodic forced heap trimming, cache dropping or new allocator dependency by default. |

M0 can disprove the assumed baseline. M2 can improve startup yet fail sustained memory once every page is visited. M5 can shrink disk size with no material PSS change. M6 can exchange memory for CPU. These are reasons to measure each result, not predictions of savings.

## Primary evidence and version limits

| Source | Relevant conclusion |
|---|---|
| [GTK threading](https://docs.gtk.org/gtk4/section-threading.html) | Widgets generally stay on the main thread; I/O/long work belongs in async APIs or workers. This supports responsive GUI design, not a memory-size claim. Latest examples may use APIs newer than GTK 4.6. |
| [GTK renderer changes, 2025](https://blog.gtk.org/2025/02/01/whats-new-in-gtk-winter-2025-edition/), [GTK runtime documentation](https://docs.gtk.org/gtk4/running.html) | Renderer availability changes between versions; the old GL renderer was removed in newer GTK. Debug environment settings are not a stable product configuration API. A renderer workaround cannot be copied across all supported releases. |
| [GNOME Sysprof guidance](https://developer.gnome.org/documentation/tools/sysprof.html), [Massif guidance](https://developer.gnome.org/documentation/tools/massif.html) | Native allocation and main-loop profiling can locate expensive ownership/call paths. Profiling has overhead; it identifies causes rather than certifying idle budgets. |
| [Cargo profiles](https://doc.rust-lang.org/cargo/reference/profiles.html) | Size/speed optimization, LTO, symbol stripping and panic behavior have different purposes. Benchmark the candidate; a smaller executable is not guaranteed to be faster or use less resident memory. |
| [Linux drop_caches guidance](https://docs.kernel.org/admin-guide/sysctl/vm.html#drop-caches) | Forcing cache eviction can create additional CPU/I/O work. RAM cleaning and dropping caches are excluded from product optimization and benchmark shortcuts. This is distinct from user-requested deletion of obsolete disk files. |

## Stop, continue and claim rules

Use D10's 30 s settling, 300 s sampling, three runs per monitoring page, hidden cases, cold/warm startup and soak protocol. Add an all-pages-visited idle run and an idle run after a bounded scan/report is completed and dismissed. Repeat the memory claim at the normal 1100x700 default and 200% scale on the declared hardware profiles; large-window/driver costs must be disclosed, not hidden by shrinking the benchmark. A safe local test fixture can include 1,000 process records and a tree exceeding the duplicate shortlist.

- **Pass:** every required valid idle sample is below 100,000,000 combined PSS bytes, other required budgets/behaviors pass, and exact-artifact evidence is retained. Report observed maximum and margin; a 99.9 MB result is fragile and requires uncertainty/repeatability review.
- **Continue:** measurable headroom remains; prioritize the largest verified contribution. Test <75/<50 MB only while preserving the product values. Do not add projected savings from unrelated benchmarks.
- **Design blocked for a profile:** the tested toolkit/driver costs leave no demonstrated path after attribution and relevant experiments. Publish the measured limit and remaining options. A toolkit switch, feature/safety reduction or changed target requires an explicit decision; no silent waiver or unsupported "impossible" claim.

No new result in this note demonstrates attainment. The deliverable is a falsifiable route to the goal: a test can show both success and failure, and the documents require reporting either.
