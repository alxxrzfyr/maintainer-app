# Accessibility Review Checklist

Status: Template — complete before v1 release.

## Test environment

- Distribution: ___
- Desktop: ___
- GTK version: ___
- Display scale: 100% and 200%
- Theme: light and dark
- Assistive technology: ___

## Keyboard navigation (all pages, no pointer)

| Page / Area | Tab order logical | All controls reachable | No keyboard traps | Focus visible | Notes |
|---|:---:|:---:|:---:|:---:|---|
| Overview | [ ] | [ ] | [ ] | [ ] | |
| System — CPU | [ ] | [ ] | [ ] | [ ] | |
| System — Memory | [ ] | [ ] | [ ] | [ ] | |
| System — Storage | [ ] | [ ] | [ ] | [ ] | |
| System — Network | [ ] | [ ] | [ ] | [ ] | |
| System — Processes | [ ] | [ ] | [ ] | [ ] | |
| Storage analysis | [ ] | [ ] | [ ] | [ ] | |
| Large files | [ ] | [ ] | [ ] | [ ] | |
| Duplicates | [ ] | [ ] | [ ] | [ ] | |
| Cleanup | [ ] | [ ] | [ ] | [ ] | |
| History | [ ] | [ ] | [ ] | [ ] | |
| Updates | [ ] | [ ] | [ ] | [ ] | |
| Diagnostics — Services | [ ] | [ ] | [ ] | [ ] | |
| Diagnostics — Journal | [ ] | [ ] | [ ] | [ ] | |
| Diagnostics — SMART | [ ] | [ ] | [ ] | [ ] | |
| Reports | [ ] | [ ] | [ ] | [ ] | |
| Settings | [ ] | [ ] | [ ] | [ ] | |

## Keyboard operations

| Operation | Works with keyboard | Notes |
|---|:---:|---|
| Navigate between pages (sidebar) | [ ] | |
| Sort table/list columns | [ ] | |
| Select items in lists | [ ] | |
| Start a scan | [ ] | |
| Cancel a scan | [ ] | |
| Open confirmation dialogs | [ ] | |
| Confirm/deny in dialogs | [ ] | |
| Review updates | [ ] | |
| Copy/save report | [ ] | |
| Toggle settings | [ ] | |
| Delete history items | [ ] | |
| Terminate a process | [ ] | |

## Focus visibility

| Criterion | Pass | Notes |
|---|:---:|---|
| Focus ring visible on all interactive elements | [ ] | |
| Focus ring has sufficient contrast (≥ 3:1) against backgrounds | [ ] | |
| Focus moves predictably (logical reading order) | [ ] | |
| Focus returns to trigger after modal/dialog close | [ ] | |
| No invisible focus states (element focused but ring absent) | [ ] | |

## Accessible names and descriptions

| Criterion | Pass | Notes |
|---|:---:|---|
| All buttons have accessible names (label or aria-label equivalent) | [ ] | |
| Icon-only buttons have tooltip and accessible name | [ ] | |
| Form inputs have associated labels | [ ] | |
| Graphs/charts have text equivalents or descriptions | [ ] | |
| Progress indicators have accessible value text | [ ] | |
| Status indicators have accessible descriptions | [ ] | |
| Dialogs have accessible titles | [ ] | |
| Navigation landmarks are properly labeled | [ ] | |

## Text scaling (200%)

| Criterion | Pass | Notes |
|---|:---:|---|
| All text remains readable at 200% system scale | [ ] | |
| No text clipping or overflow hidden | [ ] | |
| Layout adapts without horizontal scrolling on primary content | [ ] | |
| Interactive targets remain minimum 44×44 CSS px equivalent | [ ] | |
| Line spacing remains adequate | [ ] | |

## Color and contrast

| Criterion | Pass | Notes |
|---|:---:|---|
| Text contrast ≥ 4.5:1 (normal text) in light theme | [ ] | |
| Text contrast ≥ 4.5:1 (normal text) in dark theme | [ ] | |
| Large text contrast ≥ 3:1 in both themes | [ ] | |
| UI component contrast ≥ 3:1 against adjacent colors | [ ] | |
| Focus indicator contrast ≥ 3:1 | [ ] | |
| Status not conveyed by color alone (icons, text, patterns) | [ ] | |
| Attention items use icon + text, not just color | [ ] | |
| Chart data distinguishable without color (pattern/label) | [ ] | |
| Error states use icon + text, not just red | [ ] | |

## Non-color status indicators

| Status type | Has non-color indicator | Type (icon/text/pattern) | Notes |
|---|:---:|---|---|
| Attention severity (critical/warning/info) | [ ] | | |
| SMART health (healthy/warning/failing) | [ ] | | |
| Service status (running/failed/inactive) | [ ] | | |
| Update classification (security/regular) | [ ] | | |
| Scan progress (running/complete/cancelled) | [ ] | | |
| Capability state (available/unavailable/permission) | [ ] | | |
| Operation result (success/partial/failure) | [ ] | | |

## Sign-off

| Reviewer | Date | Result |
|---|---|---|
| ___ | ___ | ___ |
