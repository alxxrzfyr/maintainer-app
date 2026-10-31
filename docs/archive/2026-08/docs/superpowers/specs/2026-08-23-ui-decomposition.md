# Spec: UI Decomposition — Network Page, Processes Page, Temperature Dedup, Icons, Colors

**Date:** 2026-08-23
**Scope:** UI architecture, monitoring state, CSS tokens, icon robustness

---

## Problem

1. **System page monolith (36KB):** CPU, memory, disks, network, temperatures, and a 50-row process table all in one module — too many concerns, hard to maintain.
2. **Temperature spam:** On multi-sensor hardware, 10–20+ individual readings fill the FlowBox with no deduplication or grouping.
3. **Missing icons on Ubuntu 22.04:** 5+ icon names used in the app are not available in Adwaita on GTK 4.6 — they render as invisible/missing.
4. **No semantic colors for network or process state:** rx/tx and process states use unstyled plain text.

## Requirements

- R1: Extract a dedicated **Network** page with per-interface detail, rate sparklines, and aggregate summary.
- R2: Extract a dedicated **Processes** page with sort controls and terminate action.
- R3: System page retains only CPU, memory, disks, and grouped temperatures.
- R4: Temperature readings are grouped per hwmon chip with a representative (Package id for coretemp, max for others).
- R5: Icon fallback utility resolves at-risk icons via `IconTheme::has_icon()` with predefined chains.
- R6: CSS gains semantic tokens for `rx`, `tx`, severity tiers, and process states in both dark and light themes.
- R7: Navigation expands the "Monitor" group to include System, Network, Processes.

## Architecture

```
Page::System   — CPU, memory, disks, temperature groups
Page::Network  — per-interface rates, aggregate sparkline
Page::Processes — sortable table, terminate action
```

Data flow unchanged: metric-worker → `SystemSnapshot` → `MonitoringState.reduce()` → per-page view models → page `.update()`.

New view models: `NetworkViewModel`, `ProcessesViewModel`. Existing `SystemViewModel` shrinks.

`group_temperatures()` lives in `maintainer-core/src/metrics/temperature.rs` (data transform, not UI).

Icon fallback lives in `maintainer-app/src/ui/icons.rs` — runtime resolution via `IconTheme`.

## Constraints

- No changes to the metric-worker thread's collection logic (it already collects everything).
- `needs_processes` flag must fire when either System or Processes page is visible.
- Raw temperature readings remain accessible for diagnostic reports.
- No libadwaita, no new crate dependencies for this work.
- Performance: no additional allocations per refresh cycle beyond what's removed from the System page.

## Deliverables

| New File | Purpose |
|---|---|
| `crates/maintainer-app/src/ui/network.rs` | Network page widget (~250 lines) |
| `crates/maintainer-app/src/ui/processes.rs` | Processes page widget (~300 lines) |
| `crates/maintainer-app/src/ui/icons.rs` | Icon fallback utility (~80 lines) |

| Modified File | Change |
|---|---|
| `crates/maintainer-app/src/ui/mod.rs` | Add `pub mod network; pub mod processes; pub mod icons;` |
| `crates/maintainer-app/src/state.rs` | Add `Page::Network`, `Page::Processes`; update `ALL`, `label()`, `icon_name()`, nav groups |
| `crates/maintainer-app/src/monitoring.rs` | Add `NetworkViewModel`, `ProcessesViewModel`; extract process fields; add `temperature_groups` |
| `crates/maintainer-app/src/ui/system.rs` | Remove network detail section, remove process section, render grouped temps |
| `crates/maintainer-app/src/ui/shell.rs` | Register new pages, wire dispatch, update `needs_processes` condition |
| `crates/maintainer-core/src/metrics/temperature.rs` | Add `TemperatureGroup`, `group_temperatures()` |
| `data/styles/dark.css` | Add semantic tokens and classes |
| `data/styles/light.css` | Mirror |

## Verification

```bash
cargo test --workspace --all-features
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo fmt --all -- --check
```
