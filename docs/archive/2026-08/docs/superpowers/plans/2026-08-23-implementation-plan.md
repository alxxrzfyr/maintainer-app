# Implementation Plan: Polkit Fix + UI Decomposition

**Date:** 2026-08-23
**Specs:** `specs/2026-08-23-polkit-policy-consistency.md`, `specs/2026-08-23-ui-decomposition.md`
**Method:** TDD — write failing test, implement minimal fix, verify green.

---

## Task 1: Policy-code consistency test (RED → GREEN)

**Objective:** Guard that Polkit XML action IDs stay in sync with code constants.

**Steps:**
1. Add `INSTALL_UPDATES` constant to `crates/maintainer-helper/src/api.rs::actions`:
   ```rust
   pub const INSTALL_UPDATES: &str = "io.github.maintainerapp.install-updates";
   ```
2. Create `crates/maintainer-helper/tests/policy_consistency.rs`:
   - Read `../../data/polkit/io.github.maintainerapp.policy` (relative to `CARGO_MANIFEST_DIR`).
   - Extract `<action id="...">` values via simple string parsing (no XML crate needed).
   - Assert extracted set == `{TERMINATE_PROCESS, CLEAN_PACKAGE_CACHE, VACUUM_JOURNAL, REFRESH_SMART, REMOVE_KERNEL, INSTALL_UPDATES}`.
   - Assert the `org.freedesktop.policykit.exec.path` annotation on `install-updates` == `/usr/libexec/maintainer-helper`.

**Test:** `cargo test -p maintainer-helper policy_xml_action_ids_match_code_constants -- --exact`

**Demo:** Test passes; removing any action constant causes test failure.

---

## Task 2: Packaging postinst Polkit reload test (RED → GREEN)

**Objective:** Ensure `.deb` postinst reloads Polkit so new policies take effect immediately.

**Steps:**
1. Create `crates/maintainer-app/tests/packaging_postinst.rs`:
   - Read `../../packaging/debian/postinst` (relative to `CARGO_MANIFEST_DIR`).
   - Assert content contains `polkit` reload command (regex: `systemctl.*(restart|reload).*polkit`).
2. Run test — expect RED (postinst currently lacks reload).
3. Add to `packaging/debian/postinst` after the desktop-database block:
   ```bash
   # Reload PolicyKit so new action definitions are recognized without reboot.
   if systemctl is-active --quiet polkit.service 2>/dev/null; then
       systemctl restart polkit.service || true
   fi
   ```
4. Run test — expect GREEN.

**Test:** `cargo test -p maintainer-app postinst_reloads_polkit_after_policy_install -- --exact`

**Demo:** Test passes; removing the reload line causes failure.

---

## Task 3: Temperature grouping in `maintainer-core` (RED → GREEN)

**Objective:** Pure data transformation that groups readings per hwmon chip with a representative.

**Steps:**
1. In `crates/maintainer-core/src/metrics/temperature.rs`, add after existing types:
   - `TemperatureGroup` struct: `chip_name: String`, `representative: TemperatureReading`, `details: Vec<TemperatureReading>`.
   - `pub fn group_temperatures(readings: &[TemperatureReading]) -> Vec<TemperatureGroup>`.
2. Add `#[cfg(test)] mod group_tests` with tests:
   - `coretemp_package_is_representative`
   - `no_package_uses_max_temperature`
   - `multiple_chips_produce_multiple_groups`
   - `single_sensor_chip_passes_through`
   - `empty_input_returns_empty_groups`
3. Implement `group_temperatures`:
   - Group by `hwmon_name`.
   - For "coretemp": prefer reading whose label starts with "Package".
   - Others: pick reading with highest `Capability::Available(celsius)`.
   - Sort groups by `chip_name`.

**Test:** `cargo test -p maintainer-core group_temperatures -- --nocapture`

**Demo:** All grouping tests pass; function is usable from app crate.

---

## Task 4: Icon fallback utility (RED → GREEN)

**Objective:** Provide runtime icon resolution with fallback chains for GTK 4.6 compatibility.

**Steps:**
1. Create `crates/maintainer-app/src/ui/icons.rs`:
   - `pub fn resolve_icon(names: &[&str]) -> &str` — uses `IconTheme::has_icon()` or falls through to last.
   - `pub fn icon_with_fallback(names: &[&str], pixel_size: i32) -> gtk4::Image`.
   - `pub mod chains` with constants: `TEMPERATURE`, `MEMORY`, `FIRMWARE`, `LOADING`, `UNAVAILABLE`, `LAPTOP`, `NETWORK_ACTIVITY`, `PROCESS`.
2. Add `#[cfg(test)] mod tests`:
   - `resolve_returns_last_when_no_display` (headless CI will hit this path).
   - `all_chains_have_at_least_two_entries`.
   - `all_chain_final_entries_are_universally_available` (assert safe prefixes).
3. Register in `ui/mod.rs`: add `pub mod icons;`.

**Test:** `cargo test -p maintainer-app icons -- --nocapture`

**Demo:** Tests pass; utility is importable from other UI modules.

---

## Task 5: CSS semantic tokens (RED → GREEN)

**Objective:** Add network, severity, and process-state color tokens to both themes.

**Steps:**
1. Add regression test in `crates/maintainer-app/src/regression_tests.rs`:
   - `css_defines_all_required_semantic_tokens` — asserts both `dark.css` and `light.css` contain `@define-color` for: `maintainer_rx`, `maintainer_tx`, `maintainer_severity_info`, `maintainer_severity_warn`, `maintainer_severity_crit`, `maintainer_severity_ok`, `maintainer_process_running`, `maintainer_process_zombie`.
2. Run test — expect RED.
3. Add to `data/styles/dark.css`:
   ```css
   @define-color maintainer_rx @maintainer_success;
   @define-color maintainer_tx @maintainer_accent;
   @define-color maintainer_severity_info @maintainer_accent;
   @define-color maintainer_severity_warn @maintainer_warning;
   @define-color maintainer_severity_crit @maintainer_danger;
   @define-color maintainer_severity_ok @maintainer_success;
   @define-color maintainer_process_running @maintainer_success;
   @define-color maintainer_process_zombie @maintainer_danger;
   ```
   Plus classes: `.maintainer_net_rx`, `.maintainer_net_tx`, `.maintainer_severity_*`, `.maintainer_process_state_*`.
4. Mirror in `data/styles/light.css`.
5. Run test — expect GREEN.

**Test:** `cargo test -p maintainer-app css_defines_all_required_semantic_tokens -- --exact`

**Demo:** Test passes; tokens are usable in new page widgets.

---

## Task 6: State and view model expansion (RED → GREEN)

**Objective:** Add `Page::Network`, `Page::Processes`, `NetworkViewModel`, `ProcessesViewModel` to state and monitoring.

**Steps:**
1. In `crates/maintainer-app/src/state.rs`:
   - Add `Network` and `Processes` variants to `Page`.
   - Extend `ALL`, `label()`, `icon_name()`.
   - Update navigation group from `&[Page::System]` to `&[Page::System, Page::Network, Page::Processes]`.
2. In `crates/maintainer-app/src/monitoring.rs`:
   - Add `NetworkViewModel` struct with: `interfaces`, `rx_rings`, `tx_rings`, `total_rx_rate`, `total_tx_rate`, `available`, `unavailable_reason`.
   - Add `ProcessesViewModel` struct with: `processes`, `sort`, `available`, `unavailable_reason`, `total_count`.
   - Add `temperature_groups: Vec<TemperatureGroup>` to `SystemViewModel`.
   - Add `network_vm` and `processes_vm` fields to `MonitoringState`.
   - In `MonitoringState::apply_snapshot()`: populate `network_vm` and `processes_vm` from snapshot data.
   - Call `group_temperatures()` when applying temperature data.
3. Add/update tests in `tests/monitor_state.rs`:
   - `network_vm_populated_from_snapshot`
   - `processes_vm_sorted_by_active_sort`
   - `temperature_groups_populated`

**Test:** `cargo test -p maintainer-app monitor_state -- --nocapture`

**Demo:** Existing and new monitor state tests pass; view models are populated.

---

## Task 7: Network page widget + shell wiring

**Objective:** Render network detail on a dedicated page.

**Steps:**
1. Create `crates/maintainer-app/src/ui/network.rs`:
   - `NetworkPage` struct with `root: ScrolledWindow`, summary card, per-interface list, status view.
   - `pub fn new() -> Self`, `pub fn widget()`, `pub fn update(&self, vm: &NetworkViewModel)`.
   - Handle unavailable state via `StatusView`.
2. Register in `ui/mod.rs`.
3. In `ui/shell.rs`:
   - Add `NetworkPage` to struct fields.
   - Add to `Stack` with name `"Network"`.
   - In dispatch: call `network_page.update()` when monitoring updates and page is visible.
4. In `ui/system.rs`: remove the network detail section (keep only the overview MetricCard on the Overview page — that's in `overview.rs`, untouched).

**Test:** Build succeeds, `cargo test --workspace` passes, network page renders when navigated.

**Demo:** Navigate to Network page; see interface list with rates (or unavailable status).

---

## Task 8: Processes page widget + shell wiring

**Objective:** Render process table on a dedicated page with sort and terminate.

**Steps:**
1. Create `crates/maintainer-app/src/ui/processes.rs`:
   - `ProcessesPage` struct with scroll, count label, sort buttons, process list box, operation status.
   - `pub fn new()`, `pub fn widget()`, `pub fn update(&self, vm: &ProcessesViewModel)`.
   - `connect_sort_changed()`, `connect_terminate()`.
2. Register in `ui/mod.rs`.
3. In `ui/shell.rs`:
   - Add `ProcessesPage` to struct fields and stack.
   - Wire sort callback → `AppIntent` → re-sort.
   - Wire terminate callback → helper client.
   - Update `needs_processes` condition: true when page is `"System"` OR `"Processes"`.
4. In `ui/system.rs`: remove the process table section entirely.

**Test:** Build succeeds, `cargo test --workspace` passes, processes page renders.

**Demo:** Navigate to Processes page; see sortable process list; terminate button triggers Polkit prompt.

---

## Task 9: Icon migration for at-risk icons

**Objective:** Replace hardcoded at-risk icon names with fallback chains.

**Steps:**
1. `grep` for at-risk icon names: `sensors-temperature-symbolic`, `content-loading-symbolic`, `action-unavailable-symbolic`, `firmware-symbolic`, `media-memory-symbolic`.
2. Replace each `Image::from_icon_name("at-risk-name")` with `icons::icon_with_fallback(chains::X, pixel_size)`.
3. Affected files: `ui/system.rs`, `ui/overview.rs`, `ui/updates.rs`, `ui/widgets/status_view.rs`.
4. Verify no remaining raw usage of at-risk names outside `icons::chains`.

**Test:** `cargo test --workspace` passes; `cargo clippy --workspace --all-targets --all-features -- -D warnings` clean.

**Demo:** On Ubuntu 22.04 with stock Adwaita theme, no missing/invisible icons.

---

## Task 10: Final verification and cleanup

**Objective:** Full workspace check, no regressions.

**Steps:**
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets --all-features -- -D warnings`
3. `cargo test --workspace --all-features`
4. Verify System page is lighter (no network detail, no process table, grouped temperatures).
5. Verify navigation sidebar shows Monitor → System, Network, Processes.

**Demo:** All CI-equivalent checks pass. App runs with decomposed pages.

---

## Dependency Graph

```
Task 1 ─┐
Task 2 ─┤ (independent, can parallel)
Task 3 ─┤
Task 4 ─┤
Task 5 ─┘
         │
         ▼
Task 6 ── depends on Tasks 3, 4, 5 (uses TemperatureGroup, icons, CSS tokens)
         │
         ▼
Task 7 ── depends on Task 6 (needs NetworkViewModel, Page::Network)
Task 8 ── depends on Task 6 (needs ProcessesViewModel, Page::Processes)
         │
         ▼
Task 9 ── depends on Tasks 4, 7, 8 (icons utility exists, pages exist to migrate)
         │
         ▼
Task 10 ─ depends on all above
```

Tasks 1–5 are parallelizable. Tasks 7–8 are parallelizable after 6.
