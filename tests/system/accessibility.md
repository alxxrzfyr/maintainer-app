# Accessibility validation procedure

Status: reusable procedure; accessibility has not been certified by this review. Revised 2026-10-30. Follow the [release status](../../docs/release-status.md) and [compatibility procedure](compatibility.md).

Record app/package version and checksum, distribution, desktop, GTK version, display backend, monitor resolution, 100%/200% display scale, enlarged text setting, both themes and assistive-technology/version. Record keyboard-only and screen-reader observations separately. A code-level accessible-name assertion does not prove the spoken result.

## Keyboard and assistive technology

Exercise Overview, System, Network, Processes, Storage overview/browser/large-files/duplicates, Cleanup/history, Updates, Diagnostics, Reports and Settings. Check navigation, drawer/menu and modal dialogs at each supported window size.

| Check | Expected observation |
|---|---|
| Reachability | Every exposed action is reachable without a pointer, in reading order; no keyboard trap |
| Navigation | Active destination is both visible and announced; collapsed navigation remains usable |
| Process refresh/sort | Focus/selection stays on the intended process or a predictable replacement; termination confirmation retains PID/start-time identity |
| Scans/results | Start, cancel, select and review work with keyboard; empty, unavailable and partial results are understandable |
| Confirmation | Title, affected target, irreversible consequences and button meanings are announced; dismiss/confirm returns focus to the trigger |
| Update progress | Authorization, download, configuration, verification and recovery states are distinguishable; disabled cancellation has an understandable reason |
| Native Polkit dialog | Agent is accessible and focus returns after allow/deny/dismiss; distinguish an agent issue from an application issue |
| Reports/settings | Inputs have labels; preview/copy/save, theme/interval controls and supported history deletion work without a pointer |
| Names and status | Icon buttons, metric cards, progress and capability states have meaningful names/descriptions; chart values have text equivalents |
| Live announcements | Significant state changes are announced without speaking every 2 s metric refresh; errors do not silently disappear |

## Appearance and scaling

| Check | Review target |
|---|---|
| Focus | Visible on every interactive control in both themes; returns predictably after dialogs |
| Text | No clipping/lost labels at 200% scale and enlarged text; keyboard can reach all content |
| Layout | Minimum supported width, large font and translated/long labels retain usable controls and readable confirmation text |
| Contrast | Normal text >=4.5:1; large text >=3:1; relevant controls/focus >=3:1 against adjacent colors; measure both themes and disabled/destructive states |
| Target size | Record actual logical-pixel dimensions and keyboard alternatives for small controls; the former 44 CSS-pixel rule was not an established native GTK contract |
| Non-color meaning | Severity, SMART, service/update state, scan progress, capability and operation result remain understandable without color |
| Charts/motion | Labels/text explain graphs; animation does not obscure status or move focus; record any reduced-motion limitation |

Contrast figures are review targets from the existing checklist, not evidence of compliance with an entire accessibility standard. Record failed elements and measurements rather than declaring a general pass.

## Retain results

After testing, save a dated report under `docs/validation/` with case/page, input sequence, expected/actual behavior, screenshots or spoken observations where useful, sanitized evidence, defect ID, operator and reviewer. Use pass/fail/unverified/not-applicable with reasons. Do not invent results or delete failing cases to obtain sign-off.
