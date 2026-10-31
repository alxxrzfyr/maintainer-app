# Compatibility validation procedure

Procedure only; completed results belong in dated validation reports. Use [release status](../../docs/release-status.md) for open gates and the [security procedure](security.md) for privileged cases.

## Select and record environments

| Candidate environment | Purpose |
|---|---|
| Ubuntu 22.04 amd64, GTK 4.6, Wayland | Compatibility floor |
| Ubuntu 22.04 amd64, X11 | Floor with alternate display backend |
| Ubuntu 24.04; latest Ubuntu LTS selected for support | Test each exact release and session advertised; avoid a stale "current LTS" label |
| Linux Mint Cinnamon; supported Pop!_OS release | Verify derivative packaging, desktop authentication and unavailable integrations |
| Ubuntu arm64 | Native artifact installation, GUI and authorization |
| Named 4 GB hardware device with a working GPU | Under-100-MB combined idle PSS, CPU, usable startup and GUI responsiveness; normal and 200% scale |
| Xvfb/software rendering | Headless regressions; cannot certify hardware performance |

These rows are test candidates, not pass results. Record exact distribution, version, kernel, GTK/GLib, architecture, desktop, display, renderer and optional tool/service versions. Do not assume that all derivatives share package names, firmware access or an authentication agent.

## Execute against the installed artifact

Start from a clean VM snapshot where privileged/destructive actions are needed. Record commit and .deb checksum. Install as administrator; run the GUI as a normal desktop user. Use `pass`, `fail`, `unverified`, or `not applicable` with a reason for every case.

| Case | Expected behavior / evidence |
|---|---|
| Fresh install and upgrade from previous supported version | Dependencies, schemas, desktop entry, icons, CSS, bus policy, service and Polkit files load; running old app is handled explicitly |
| Launch | Normal-user usable shell/navigation appears within the startup limit in AGENTS.md; loading states remain explicit. Record window map/first live data separately; process survival is insufficient |
| Navigation | Overview, System, Network, Processes, Storage subpages, Cleanup/history, Updates, Diagnostics, Reports and Settings render without crash |
| Monitoring | Rates/units are credible; default visible/hidden sampling is 2 s/10 s; saved settings apply; resumed views show current status |
| Dynamic devices/processes | Interface reorder, sensor removal, PID reuse and permission loss do not show another identity's history or stale values as live |
| Storage/duplicates | Known duplicates appear through the app, including beyond the largest 200 files; limits/inaccessible paths remain visibly partial. Progress/cancel/empty/failure are distinct; no automatic deletion |
| Cleanup | Explicit preview/confirmation; Trash never falls back to permanent deletion, which needs its own stronger confirmation; accurate result/history including partial failures |
| Updates/kernel | Discovery matches recorded APT fixtures/state; chosen intent is preserved; installed allow/deny/cancel/recovery tests follow security procedure |
| Optional integrations | Missing SMART/sensors/udisks2/Flatpak/fwupd/iw yield explained capability states. Resolve curl only after speed-test confirmation in the worker; show its failure reason if unavailable. Missing is not zero or success |
| Hardware diagnostics | Opening Diagnostics mounts once and collects in a bounded worker with loading/failure states. Missing USB/DMI values stay unavailable; unprobed graphics APIs stay unverified |
| Reports | Generate/preview/copy/save shares one revision; seeded free-text/private data is redacted, capped reports disclose omissions, saved files remain private |
| Settings/DB | Theme and intervals persist in order under saturation; invalid XDG and unavailable/corrupt/locked/newer DB do not prevent monitoring or silently reset data; tested backup/restore works |
| Accessibility | Complete the separate keyboard, scaling, contrast and assistive-technology procedure |
| Exit/reopen | No crash, stray scan or uncontrolled package transaction; in-progress package behavior follows its reviewed ownership policy |
| Remove/purge | Binaries/system integration disappear; settings preservation/removal matches documented behavior; do not assume dpkg can purge every user's home directory |
| Downgrade | Test old-binary/schema compatibility or reviewed backup restore; otherwise record downgrade as unsupported |

Do not call an intentionally unavailable capability a product failure when that state is expected. Do fail unexplained loss of a claimed capability or fabricated values. If a required environment remains unverified, narrow the published support claim instead of marking it green.

## Retain results

Save a dated report under `docs/validation/YYYY-MM-DD/report.md` after the run, with environment IDs, package checksum, actions/commands, expected/actual behavior, sanitized evidence, issue links, operator and reviewer. Record performance results with whole-app PSS, renderer, scaling, all-pages-visited and after-job idle states; a short Xvfb survival test is insufficient. This file remains a reusable procedure and must not accumulate undated checkmarks.
