# Maintainer — Design Specification

Status: Approved direction · Last reviewed: 2026-08-18

## Design intent

Maintainer should feel like a polished modern dashboard while behaving like a native, lightweight Linux utility. Information hierarchy, spacing, typography, and clear states create polish—not web imitation, blur, gradients, large assets, or constant animation.

Three questions guide every view:

1. What is happening?
2. Does it deserve attention?
3. What safe action is available?

## Source of truth

Until application styles exist, this document is authoritative. After scaffolding, semantic tokens live in GTK CSS under `data/styles/`; code values win and this document must be updated when intent changes.

Use GTK4 only. Do not add libadwaita, WebView, Electron, or browser-oriented component systems.

## Visual language

- Quiet neutral background with clear white or dark surfaces.
- One blue accent for selection and primary action.
- Status colors supplement icons and labels; color never carries meaning alone.
- Flat surfaces with subtle borders. Shadows are reserved for transient overlays if GTK rendering remains inexpensive.
- Compact desktop density with enough space for touchpads, motor accessibility, and localization.
- System font stack and native text rendering.
- Rounded cards support grouping; avoid turning every row into a floating card.

## Semantic color tokens

GTK CSS token names use the `maintainer_` prefix.

### Light

| Token | Value | Use |
|---|---:|---|
| `maintainer_bg` | `#F5F6F8` | Window and page background |
| `maintainer_surface` | `#FFFFFF` | Cards, panels, dialogs, menus |
| `maintainer_surface_alt` | `#ECEFF3` | Selected or grouped secondary surface |
| `maintainer_text` | `#1D232B` | Primary text |
| `maintainer_text_muted` | `#5B6673` | Secondary text |
| `maintainer_accent` | `#245FD6` | Active navigation and primary action |
| `maintainer_success` | `#1F7250` | Confirmed success or healthy state |
| `maintainer_warning` | `#8A5700` | Conditions requiring review |
| `maintainer_danger` | `#B42318` | Destructive actions and confirmed errors |
| `maintainer_border` | `#D7DCE3` | Dividers, outlines, field borders |
| `maintainer_focus` | `#1B64E8` | Keyboard focus indicator |

### Dark

| Token | Value | Use |
|---|---:|---|
| `maintainer_bg` | `#111419` | Window and page background |
| `maintainer_surface` | `#191D23` | Cards, panels, dialogs, menus |
| `maintainer_surface_alt` | `#232932` | Selected or grouped secondary surface |
| `maintainer_text` | `#F1F3F5` | Primary text |
| `maintainer_text_muted` | `#B4BDC8` | Secondary text |
| `maintainer_accent` | `#8BB4FF` | Active navigation and primary action |
| `maintainer_success` | `#63D3A0` | Confirmed success or healthy state |
| `maintainer_warning` | `#F0B65A` | Conditions requiring review |
| `maintainer_danger` | `#FF8A80` | Destructive actions and confirmed errors |
| `maintainer_border` | `#353C47` | Dividers, outlines, field borders |
| `maintainer_focus` | `#A9C7FF` | Keyboard focus indicator |

Verify final rendered contrast in GTK on both themes. Never assume source hex values guarantee contrast after state styling.

## Type, spacing, and shape

Use the system UI font. Weight and size establish hierarchy.

- Type scale: 12, 14, 16, 20, 28, 36 px.
- Body default: 14 px; supporting metadata: 12 px; card values: 28 px; page title: 28 px.
- Spacing scale: 4, 8, 12, 16, 24, 32, 48 px.
- Radius scale: 6 px controls, 10 px cards, 14 px dialogs and large panels.
- Borders: 1 px. Avoid decorative thick outlines.
- Page content width: fluid; cap reading-heavy report/settings content near 960 px.

Off-scale values require a documented component need. Do not shrink text to fit untranslated copy.

## Adaptive layout

GTK size allocation, not device identity, controls adaptation.

| Available width | Navigation | Content behavior |
|---|---|---|
| Below 640 px | Sidebar becomes an explicit navigation drawer or page switcher | Cards stack; tables expose essential columns and details view |
| 640–899 px | Compact icon rail when labels remain discoverable | Two-column cards where content fits |
| 900 px and above | Persistent labeled sidebar | Dashboard grid and full tables |
| 1200 px and above | Persistent sidebar; content width remains intentional | Add columns only when scanability improves |

Never require horizontal window scrolling. Data tables may scroll internally, but primary actions and status remain visible. Collapsed navigation must retain tooltips and accessible names.

## Navigation

Persistent desktop sidebar groups:

- General: Overview
- Monitor: System
- Storage: Storage
- Maintenance: Cleanup, Updates
- Diagnostics: Diagnostics, Reports
- Footer: Settings

Activity tracking is not shown in v1 navigation. Active page uses accent, icon, label, and selected surface. Icons come from the system icon theme or compact project SVGs; no emoji as interface icons.

## Page patterns

### Overview

- Page title, optional greeting, and one-line system summary.
- Metric cards for CPU, memory, disk, temperature, and battery where present.
- Attention list outranks recent activity and quick actions.
- Each attention row includes severity icon, plain-language title, evidence, and destination.
- Sparklines show short trends only; they do not animate continuously.

### System

- Metric summary and details share one page with clear sections.
- Graphs use bounded samples and update only changed drawing areas.
- Process table supports sortable columns, selection, keyboard access, and confirmed termination.
- Linux cache is described as useful memory, never “wasted RAM.”

### Storage

- Mount overview precedes directory analysis.
- Scan controls stay visible with current root, exclusions, progress, elapsed state, cancel, and result count.
- Large files and duplicates use list/table presentation with full path available before action.
- Duplicate groups visually preserve one recommended “keep” candidate only as guidance; nothing is automatically selected for deletion.

### Cleanup and Updates

Use a review workflow rather than a single destructive dashboard button:

```text
Scan or check
Review selections
Confirm exact effects
Authenticate if required
Execute with progress
Show per-item result
```

Primary button labels name the action, such as “Review cleanup” or “Install 12 updates.” Never use “Optimize,” “Fix all,” or “Clean everything.”

### Diagnostics

Diagnostic cards always order content as:

1. Detected
2. Evidence
3. Possible cause, when justified
4. Recommendation

Possible causes use cautious language and never borrow error styling reserved for detected facts. Repeated journal events appear as grouped occurrences with time range and expandable examples.

### Reports and Settings

Reports are readable documents, not dashboard card grids. Redaction state and preview appear before copy/save actions. Settings use ordinary grouped rows, concise explanations, and immediate validation; avoid deep nesting.

## Component conventions

- One GTK widget or focused widget family per module under `src/ui/` once scaffolded.
- Views consume presentation models and emit typed user intents. They do not collect system data, query SQLite, or call the helper.
- Parent containers own outer spacing.
- Prefer composition over widgets with many boolean modes.
- Centralize semantic status, size, and emphasis variants; do not accumulate one-off CSS classes.
- Lists with many items use GTK list virtualization patterns rather than one permanent widget tree per result.
- Every async surface defines loading, progress, unavailable, permission-required, empty, success, partial-failure, failure, and cancelled states as applicable.
- Toasts confirm non-critical outcomes. Errors requiring decisions stay in context or use dialogs.

## Interaction and motion

- State transition: 120 ms.
- View entrance: at most 180 ms and used sparingly.
- Prefer opacity and small translation. Never animate live chart geometry between every sample.
- Respect GTK animation settings and reduced-motion preferences by removing nonessential movement.
- Do not delay input, progress, or result presentation for animation.
- Destructive confirmations identify target count, examples, reversibility, and expected effect.
- Closing a running job prompts only when cancellation could lose meaningful progress or leave an operation incomplete.

## Accessibility

| Rule | Requirement |
|---|---|
| Contrast | At least 4.5:1 for normal text and 3:1 for large text, focus, and meaningful UI boundaries |
| Keyboard | Every action reachable and operable; logical focus order; visible focus never removed |
| Names | Controls, icons, charts, progress, and status expose useful GTK accessible names/descriptions |
| Targets | Interactive targets normally at least 40×40 px; dense table rows remain keyboard accessible |
| Status | Icon and text accompany color; live progress announcements are useful but not noisy |
| Charts | Equivalent current values and summaries exist in text |
| Forms | Labels remain visible; errors identify the field and corrective action |
| Zoom and text | Layout tolerates increased font size without clipping actions or values |

## Content style

- Lead with fact: “1 service failed,” not “Your system may be unhealthy.”
- Use exact quantities and time ranges.
- Say “Unavailable” with reason instead of “Unknown error.”
- Say “Potentially reclaimable” until cleanup completes.
- Avoid blame, alarmism, fake urgency, and unexplained Linux jargon.
- Distinguish detected evidence from possible interpretation in both wording and styling.

## Visual performance constraints

Do not use blur, glass effects, gradients, video, large raster assets, full-screen decoration, constant animation, or unbounded chart histories. Use native GTK widgets, system fonts, vector icons, small bounded sparklines, incremental updates, virtualized lists, and on-demand detail views.

A visual feature does not ship if it causes the application to miss the PRD performance contract without a stronger user benefit and explicit approval.
