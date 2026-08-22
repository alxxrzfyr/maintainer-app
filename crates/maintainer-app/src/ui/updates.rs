//! Updates page UI.
//!
//! Implements the update flow: Check → Review → Confirm → Authenticate → Execute → Result.
//!
//! Invariants:
//! - Security labels only appear when backend evidence supports them.
//! - Cancellation is disabled when the backend reports it unsafe.
//! - No credentials are stored; authentication is delegated to PackageKit/Polkit.
//! - The Install button is only enabled after a plan has been reviewed.

use gtk4::prelude::*;
use gtk4::{
    self, Box as GtkBox, Button, CheckButton, Image, Label, ListBox, Orientation, ProgressBar,
    ScrolledWindow,
};
use maintainer_core::updates::{
    HoldState, UpdateBackend, UpdateCandidate, UpdateClassification, UpdatePlan,
};

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

/// Coarse selection mode controlling which candidates from the fetched
/// `UpdatePlan` are included for execution.
///
/// This is intentionally coarse: apt resolves upgrades as a dependency set,
/// so arbitrary per-package unchecking without re-running the planner would
/// silently misrepresent what will happen. A security-only/everything split
/// is safe because it does not fabricate a fake dependency state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SelectionMode {
    /// Include only candidates classified as security updates.
    SecurityOnly,
    /// Include all candidates (equivalent to "select all").
    Everything,
    /// Per-package selection via individual checkboxes. When active, the
    /// coarse toggle buttons are deactivated and a re-plan through apt's
    /// resolver is required before the Install button becomes submittable.
    Custom,
}

impl Default for SelectionMode {
    fn default() -> Self {
        Self::Everything
    }
}

/// Filter candidates according to the active selection mode.
///
/// Pure function, unit-testable without GTK.
///
/// - `SecurityOnly`: returns only candidates with `UpdateClassification::Security`.
/// - `Everything`: returns all candidates unchanged.
/// - `Custom`: returns all candidates unchanged (the actual filtering is done by
///   the per-package checkbox state tracked externally; this function cannot know
///   which individual packages are selected).
pub fn filter_candidates_for_mode(
    candidates: &[UpdateCandidate],
    mode: SelectionMode,
) -> Vec<&UpdateCandidate> {
    match mode {
        SelectionMode::SecurityOnly => candidates
            .iter()
            .filter(|c| c.classification == UpdateClassification::Security)
            .collect(),
        SelectionMode::Everything | SelectionMode::Custom => candidates.iter().collect(),
    }
}

/// The phases of the update workflow.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpdatePhase {
    /// Initial state; ready to check for updates.
    Idle,
    /// Checking for available updates.
    Checking,
    /// Displaying available updates for review.
    Reviewing,
    /// User has confirmed; awaiting authentication.
    Confirming,
    /// Authentication passed; executing update.
    Executing,
    /// Update completed (success or failure).
    Result,
}

/// Top-level updates page widget.
#[derive(Clone)]
pub struct UpdatesPage {
    container: GtkBox,
    check_button: Button,
    status_label: Label,
    // Selection mode toggle
    mode_box: GtkBox,
    everything_toggle: gtk4::ToggleButton,
    security_toggle: gtk4::ToggleButton,
    selection_mode: SelectionMode,
    // Per-package selection state (package names currently checked).
    // Shared via Rc<RefCell<>> because checkbox signal handlers need access.
    selected_packages: Rc<RefCell<HashSet<String>>>,
    // Whether the current selection has been re-planned through apt's resolver.
    // Set to `true` after a successful plan_for_selection call, reset to `false`
    // whenever a checkbox is toggled.
    selection_replanned: Rc<RefCell<bool>>,
    // Sticky top action bar
    action_bar: GtkBox,
    summary_label: Label,
    confirm_button: Button,
    review_button: Button,
    reboot_notice: Label,
    // Package list
    package_list: ListBox,
    package_scrolled: ScrolledWindow,
    // Bottom area: cancel, progress, result
    cancel_button: Button,
    progress_bar: ProgressBar,
    progress_label: Label,
    result_label: Label,
    phase: UpdatePhase,
}

impl Default for UpdatesPage {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdatesPage {
    /// Build the updates page widget tree.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 12);
        container.add_css_class("updates-page");
        container.set_margin_top(24);
        container.set_margin_bottom(24);
        container.set_margin_start(24);
        container.set_margin_end(24);

        // --- Header ---
        let header = Label::new(Some("System Updates"));
        header.add_css_class("title-1");
        header.set_halign(gtk4::Align::Start);
        container.append(&header);

        let description = Label::new(Some(
            "Check for available system, application, and firmware updates. \
             Review all changes before installing.",
        ));
        description.set_wrap(true);
        description.set_halign(gtk4::Align::Start);
        description.add_css_class("dim-label");
        container.append(&description);

        // --- Check button ---
        let check_button = Button::with_label("Check for Updates");
        check_button.add_css_class("suggested-action");
        check_button.set_halign(gtk4::Align::Start);
        container.append(&check_button);

        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        // --- Selection mode toggle (hidden until reviewing) ---
        let mode_box = GtkBox::new(Orientation::Horizontal, 0);
        mode_box.add_css_class("linked");
        mode_box.set_halign(gtk4::Align::Start);
        mode_box.set_visible(false);

        let everything_toggle = gtk4::ToggleButton::with_label("Everything");
        everything_toggle.set_active(true);

        let security_toggle = gtk4::ToggleButton::with_label("Security Only");
        security_toggle.set_group(Some(&everything_toggle));

        mode_box.append(&everything_toggle);
        mode_box.append(&security_toggle);
        container.append(&mode_box);

        // --- Sticky top action bar (summary + Install button) ---
        let action_bar = GtkBox::new(Orientation::Horizontal, 12);
        action_bar.add_css_class("action-bar-top");
        action_bar.set_visible(false);
        action_bar.set_margin_top(4);
        action_bar.set_margin_bottom(4);

        let summary_label = Label::new(None);
        summary_label.set_halign(gtk4::Align::Start);
        summary_label.set_hexpand(true);
        summary_label.set_wrap(true);
        action_bar.append(&summary_label);

        // "Review Selection" button: visible only in Custom mode when the
        // current per-package selection has not yet been re-planned through
        // apt's dependency resolver.
        let review_button = Button::with_label("Review Selection");
        review_button.add_css_class("suggested-action");
        review_button.set_visible(false);
        review_button.set_tooltip_text(Some(
            "Re-run apt's dependency resolver for your custom package selection",
        ));
        action_bar.append(&review_button);

        let confirm_button = Button::with_label("Install Updates");
        confirm_button.add_css_class("suggested-action");
        confirm_button.set_sensitive(false);
        action_bar.append(&confirm_button);

        container.append(&action_bar);

        // --- Reboot notice (page-level, visible when kernel update + reboot required) ---
        let reboot_notice = Label::new(Some(
            "⟳ A system restart will be required after installing these updates.",
        ));
        reboot_notice.add_css_class("warning");
        reboot_notice.set_halign(gtk4::Align::Start);
        reboot_notice.set_wrap(true);
        reboot_notice.set_visible(false);
        container.append(&reboot_notice);

        // --- Package list ---
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_min_content_height(200);
        scrolled.set_visible(false);

        let package_list = ListBox::new();
        package_list.add_css_class("boxed-list");
        package_list.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&package_list));
        container.append(&scrolled);

        // --- Bottom footer: cancel button for in-progress execution ---
        let bottom_footer = GtkBox::new(Orientation::Horizontal, 8);
        bottom_footer.set_halign(gtk4::Align::Start);

        let cancel_button = Button::with_label("Cancel");
        cancel_button.add_css_class("destructive-action");
        cancel_button.set_visible(false);
        bottom_footer.append(&cancel_button);

        container.append(&bottom_footer);

        // --- Progress ---
        let progress_bar = ProgressBar::new();
        progress_bar.set_visible(false);
        progress_bar.set_show_text(true);
        container.append(&progress_bar);

        let progress_label = Label::new(None);
        progress_label.set_halign(gtk4::Align::Start);
        progress_label.set_visible(false);
        container.append(&progress_label);

        // --- Result ---
        let result_label = Label::new(None);
        result_label.set_halign(gtk4::Align::Start);
        result_label.set_visible(false);
        result_label.set_wrap(true);
        container.append(&result_label);

        Self {
            container,
            check_button,
            status_label,
            mode_box,
            everything_toggle,
            security_toggle,
            selection_mode: SelectionMode::Everything,
            selected_packages: Rc::new(RefCell::new(HashSet::new())),
            selection_replanned: Rc::new(RefCell::new(false)),
            action_bar,
            summary_label,
            confirm_button,
            review_button,
            reboot_notice,
            package_list,
            package_scrolled: scrolled,
            cancel_button,
            progress_bar,
            progress_label,
            result_label,
            phase: UpdatePhase::Idle,
        }
    }

    /// Returns the top-level container widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Returns the current update phase.
    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    /// Returns the current selection mode.
    pub fn selection_mode(&self) -> SelectionMode {
        self.selection_mode
    }

    /// Returns the "Everything" toggle button for signal connections.
    pub fn everything_toggle(&self) -> &gtk4::ToggleButton {
        &self.everything_toggle
    }

    /// Returns the "Security Only" toggle button for signal connections.
    pub fn security_toggle(&self) -> &gtk4::ToggleButton {
        &self.security_toggle
    }

    /// Update the internal selection mode. Called from toggle signal handlers.
    pub fn set_selection_mode(&mut self, mode: SelectionMode) {
        self.selection_mode = mode;
    }

    /// Returns the "Review Selection" button for signal connections.
    pub fn review_button(&self) -> &Button {
        &self.review_button
    }

    /// Returns a clone of the per-package selection set (package names currently checked).
    pub fn selected_packages(&self) -> HashSet<String> {
        self.selected_packages.borrow().clone()
    }

    /// Returns whether the current custom selection has been re-planned.
    pub fn is_selection_replanned(&self) -> bool {
        *self.selection_replanned.borrow()
    }

    /// Mark the current custom selection as having been re-planned (after a
    /// successful `plan_for_selection` call).
    pub fn mark_selection_replanned(&self) {
        *self.selection_replanned.borrow_mut() = true;
        self.review_button.set_visible(false);
        self.confirm_button.set_sensitive(true);
    }

    /// Transition to the checking state.
    pub fn set_checking(&mut self) {
        self.phase = UpdatePhase::Checking;
        self.check_button.set_label("Checking for Updates…");
        self.check_button.set_sensitive(false);
        self.status_label.set_text("Checking for updates…");
        self.status_label.set_visible(true);
        self.action_bar.set_visible(false);
        self.confirm_button.set_sensitive(false);
        self.review_button.set_visible(false);
        self.mode_box.set_visible(false);
        self.reboot_notice.set_visible(false);
        self.result_label.set_visible(false);
    }

    /// Transition to reviewing state with update summary.
    pub fn set_reviewing(&mut self, summary: &str) {
        self.phase = UpdatePhase::Reviewing;
        self.check_button.set_label("Check Again");
        self.check_button.set_sensitive(true);
        self.status_label.set_text(summary);
        self.status_label.set_visible(true);
        // Show top action bar with summary and enabled Install button
        self.action_bar.set_visible(true);
        self.summary_label.set_text(summary);
        self.confirm_button.set_sensitive(true);
        // Show mode toggle
        self.mode_box.set_visible(true);
    }

    /// Show the immutable update plan that will be handed to PackageKit.
    pub fn set_reviewing_plan(&mut self, summary: &str, plan: &UpdatePlan) {
        self.set_reviewing(summary);
        self.populate_package_list(&plan.candidates);
        self.package_scrolled
            .set_visible(!plan.candidates.is_empty());
        self.progress_bar.set_visible(false);

        // Show page-level reboot notice when plan requires reboot and has kernel updates.
        let has_kernel = plan.candidates.iter().any(|c| c.is_kernel);
        self.reboot_notice
            .set_visible(plan.reboot_required && has_kernel);
    }

    /// Repopulate the visible package list for a given candidate slice.
    ///
    /// Used both on initial plan display and when selection mode changes.
    /// Each row includes a checkbox for per-package selection. All checkboxes
    /// default to checked (matching the coarse "everything selected" state).
    pub fn populate_package_list(&self, candidates: &[UpdateCandidate]) {
        while let Some(child) = self.package_list.first_child() {
            self.package_list.remove(&child);
        }

        // Initialize selected_packages to contain all displayed candidates.
        {
            let mut selected = self.selected_packages.borrow_mut();
            selected.clear();
            for c in candidates {
                selected.insert(c.name.clone());
            }
        }
        // A fresh populate means selection matches the plan; mark as replanned.
        *self.selection_replanned.borrow_mut() = true;
        // Hide review button and enable confirm since selection == plan.
        self.review_button.set_visible(false);
        self.confirm_button.set_sensitive(true);

        for candidate in candidates {
            let outer_row = GtkBox::new(Orientation::Horizontal, 8);
            outer_row.set_margin_top(6);
            outer_row.set_margin_bottom(6);
            outer_row.set_margin_start(8);
            outer_row.set_margin_end(8);

            // --- Per-package checkbox ---
            let check = CheckButton::new();
            check.set_active(true);
            check.set_valign(gtk4::Align::Center);
            outer_row.append(&check);

            let row = GtkBox::new(Orientation::Vertical, 4);

            let title_box = GtkBox::new(Orientation::Horizontal, 8);
            let title = Label::new(Some(&candidate.name));
            title.set_halign(gtk4::Align::Start);
            title.add_css_class("heading");
            title_box.append(&title);

            // --- Status icons ---

            // 1. Security update indicator
            if candidate.classification == UpdateClassification::Security {
                let icon = Image::from_icon_name("security-high-symbolic");
                icon.set_icon_size(gtk4::IconSize::Normal);
                icon.set_tooltip_text(Some("Security update"));
                icon.add_css_class("security-icon");
                title_box.append(&icon);
            }

            // 2. Kernel update indicator (orthogonal to security)
            if candidate.is_kernel {
                let icon = Image::from_icon_name("system-run-symbolic");
                icon.set_icon_size(gtk4::IconSize::Normal);
                icon.set_tooltip_text(Some("Kernel update"));
                icon.add_css_class("kernel-icon");
                title_box.append(&icon);
            }

            // 3. Held / Phased indicator
            match candidate.hold_state {
                HoldState::Held => {
                    let icon = Image::from_icon_name("changes-prevent-symbolic");
                    icon.set_icon_size(gtk4::IconSize::Normal);
                    icon.set_tooltip_text(Some("Held back"));
                    icon.add_css_class("held-icon");
                    title_box.append(&icon);
                }
                HoldState::Phased => {
                    let icon = Image::from_icon_name("changes-prevent-symbolic");
                    icon.set_icon_size(gtk4::IconSize::Normal);
                    icon.set_tooltip_text(Some("Phased rollout, not yet offered"));
                    icon.add_css_class("held-icon");
                    title_box.append(&icon);
                }
                HoldState::None => {}
            }

            // 4. Backend indicator for non-Apt entries
            match candidate.backend {
                UpdateBackend::Flatpak => {
                    let icon = Image::from_icon_name("package-x-generic-symbolic");
                    icon.set_icon_size(gtk4::IconSize::Normal);
                    icon.set_tooltip_text(Some("Flatpak"));
                    icon.add_css_class("backend-icon");
                    title_box.append(&icon);
                }
                UpdateBackend::Fwupd => {
                    let icon = Image::from_icon_name("firmware-symbolic");
                    icon.set_icon_size(gtk4::IconSize::Normal);
                    icon.set_tooltip_text(Some("Firmware (fwupd)"));
                    icon.add_css_class("backend-icon");
                    title_box.append(&icon);
                }
                UpdateBackend::Apt => {}
            }

            row.append(&title_box);

            let current = candidate
                .installed_version
                .as_deref()
                .unwrap_or("not installed");
            let versions = Label::new(Some(&format!(
                "{} → {}",
                current, candidate.available_version
            )));
            versions.set_halign(gtk4::Align::Start);
            versions.add_css_class("dim-label");
            row.append(&versions);

            outer_row.append(&row);
            self.package_list.append(&outer_row);

            // --- Checkbox toggle handler ---
            let pkg_name = candidate.name.clone();
            let selected_ref = self.selected_packages.clone();
            let replanned_ref = self.selection_replanned.clone();
            let review_btn = self.review_button.clone();
            let confirm_btn = self.confirm_button.clone();
            let everything_toggle = self.everything_toggle.clone();
            let security_toggle = self.security_toggle.clone();

            check.connect_toggled(move |btn| {
                let mut selected = selected_ref.borrow_mut();
                if btn.is_active() {
                    selected.insert(pkg_name.clone());
                } else {
                    selected.remove(&pkg_name);
                }
                drop(selected);

                // Any manual checkbox toggle invalidates the current re-plan
                // and switches to Custom mode.
                *replanned_ref.borrow_mut() = false;

                // Deactivate coarse toggle buttons to indicate Custom mode.
                // GTK ToggleButton in a group requires at least one active;
                // we set neither active by temporarily removing from group.
                // Since we can't truly deactivate both in a linked group
                // cleanly, we unset both explicitly.
                everything_toggle.set_active(false);
                security_toggle.set_active(false);

                // Show Review Selection button, disable Install.
                review_btn.set_visible(true);
                confirm_btn.set_sensitive(false);
            });
        }
    }

    /// Update the top action bar summary text (e.g. after mode change).
    pub fn update_summary_text(&self, summary: &str) {
        self.summary_label.set_text(summary);
        self.status_label.set_text(summary);
    }

    /// Show that no updates are available.
    pub fn set_up_to_date(&mut self) {
        self.phase = UpdatePhase::Idle;
        self.check_button.set_label("Check for Updates");
        self.check_button.set_sensitive(true);
        self.status_label.set_text("System is up to date.");
        self.status_label.set_visible(true);
        self.action_bar.set_visible(false);
        self.confirm_button.set_sensitive(false);
        self.mode_box.set_visible(false);
        self.reboot_notice.set_visible(false);
        self.package_scrolled.set_visible(false);
        self.progress_bar.set_visible(false);
    }

    /// Show a discovery error.
    pub fn set_check_failed(&mut self, message: &str) {
        self.phase = UpdatePhase::Idle;
        self.check_button.set_label("Check for Updates");
        self.check_button.set_sensitive(true);
        self.status_label.set_text(message);
        self.status_label.set_visible(true);
        self.status_label.add_css_class("error");
        self.action_bar.set_visible(false);
        self.confirm_button.set_sensitive(false);
        self.mode_box.set_visible(false);
        self.reboot_notice.set_visible(false);
    }

    /// Transition to executing state.
    pub fn set_executing(&mut self) {
        self.phase = UpdatePhase::Executing;
        self.action_bar.set_visible(false);
        self.confirm_button.set_sensitive(false);
        self.mode_box.set_visible(false);
        self.reboot_notice.set_visible(false);
        self.check_button.set_label("Installing Updates…");
        self.check_button.set_sensitive(false);
        self.progress_bar.set_visible(true);
        self.progress_bar.set_fraction(0.0);
        self.progress_label.set_visible(true);
        self.progress_label.set_text("Preparing…");
        self.cancel_button.set_visible(true);
        self.cancel_button.set_sensitive(true);
    }

    /// Update progress display during execution.
    pub fn set_progress(&self, fraction: f64, message: &str, allow_cancel: bool) {
        self.progress_bar.set_fraction(fraction);
        self.progress_label.set_text(message);
        self.cancel_button.set_sensitive(allow_cancel);

        if !allow_cancel {
            self.cancel_button
                .set_tooltip_text(Some("Cannot cancel at this stage"));
        } else {
            self.cancel_button.set_tooltip_text(None);
        }
    }

    /// Show the final result.
    pub fn set_result(&mut self, success: bool, message: &str, restart_required: bool) {
        self.phase = UpdatePhase::Result;
        self.progress_bar.set_visible(false);
        self.progress_label.set_visible(false);
        self.cancel_button.set_visible(false);
        self.action_bar.set_visible(false);
        self.mode_box.set_visible(false);
        self.reboot_notice.set_visible(false);
        self.check_button.set_label("Check for Updates");
        self.check_button.set_sensitive(true);

        let mut text = message.to_string();
        if restart_required {
            text.push_str("\n\nA system restart is recommended to complete the update.");
        }

        self.result_label.set_text(&text);
        self.result_label.set_visible(true);

        if success {
            self.result_label.remove_css_class("error");
            self.result_label.add_css_class("success");
        } else {
            self.result_label.remove_css_class("success");
            self.result_label.add_css_class("error");
        }
    }

    /// Get a reference to the check button for signal connections.
    pub fn check_button(&self) -> &Button {
        &self.check_button
    }

    /// Show progress during the update check phase.
    pub fn show_check_progress(&self, completed: u64, total: Option<u64>, message: &str) {
        let text = match total {
            Some(t) if t > 0 => format!("{message} ({completed}/{t})"),
            _ => format!("{message} ({completed} items)"),
        };
        self.status_label.set_text(&text);
        self.status_label.set_visible(true);
        self.progress_bar.set_visible(true);
        match total {
            Some(t) if t > 0 => {
                let fraction = completed as f64 / t as f64;
                self.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
            }
            _ => {
                self.progress_bar.pulse();
            }
        }
    }

    /// Get a clone of the status label for use in progress closures.
    pub fn status_label_handle(&self) -> Label {
        self.status_label.clone()
    }

    /// Get a clone of the progress bar for use in progress closures.
    pub fn progress_bar_handle(&self) -> ProgressBar {
        self.progress_bar.clone()
    }

    /// Get a reference to the confirm button for signal connections.
    pub fn confirm_button(&self) -> &Button {
        &self.confirm_button
    }

    /// Get a reference to the cancel button for signal connections.
    pub fn cancel_button(&self) -> &Button {
        &self.cancel_button
    }
}

/// Reducer: format a security count label.
///
/// Returns `None` if classification has no evidence (classification == Unknown).
pub fn format_security_label(security_count: usize, total_count: usize) -> Option<String> {
    if security_count == 0 {
        return None;
    }
    Some(format!(
        "{security_count} of {total_count} updates address security issues"
    ))
}

/// Reducer: format overall update summary.
pub fn format_update_summary(
    upgrade_count: usize,
    new_count: usize,
    removal_count: usize,
    held_count: usize,
    download_bytes: Option<u64>,
) -> String {
    let mut parts = Vec::new();

    if upgrade_count > 0 {
        parts.push(format!("{upgrade_count} to upgrade"));
    }
    if new_count > 0 {
        parts.push(format!("{new_count} newly installed"));
    }
    if removal_count > 0 {
        parts.push(format!("{removal_count} to remove"));
    }
    if held_count > 0 {
        parts.push(format!("{held_count} held back"));
    }

    let mut summary = parts.join(", ");

    if let Some(bytes) = download_bytes {
        let size_str = format_bytes(bytes);
        summary.push_str(&format!(" ({size_str} to download)"));
    }

    if summary.is_empty() {
        "No updates available".to_string()
    } else {
        summary
    }
}

/// Build a summary string for a filtered subset of candidates.
///
/// Uses `format_update_summary` under the hood, computing counts from the
/// given candidate slice.
pub fn format_filtered_summary(
    candidates: &[&UpdateCandidate],
    removal_count: usize,
    held_count: usize,
) -> String {
    let upgrade_count = candidates.iter().filter(|c| !c.is_new).count();
    let new_count = candidates.iter().filter(|c| c.is_new).count();
    let download_bytes: Option<u64> = if candidates.is_empty() {
        None
    } else {
        let mut total = 0u64;
        let mut any_known = false;
        for c in candidates {
            if let Some(b) = c.download_bytes {
                total += b;
                any_known = true;
            }
        }
        if any_known { Some(total) } else { None }
    };
    format_update_summary(
        upgrade_count,
        new_count,
        removal_count,
        held_count,
        download_bytes,
    )
}

/// Format bytes into a human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1_000;
    const MB: u64 = 1_000_000;
    const GB: u64 = 1_000_000_000;

    if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} kB", bytes as f64 / KB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maintainer_core::updates::{HoldState, UpdateBackend};

    fn make_candidate(
        name: &str,
        classification: UpdateClassification,
        is_kernel: bool,
    ) -> UpdateCandidate {
        UpdateCandidate {
            name: name.to_string(),
            installed_version: Some("1.0.0".to_string()),
            available_version: "2.0.0".to_string(),
            classification,
            download_bytes: Some(1_000_000),
            hold_state: HoldState::None,
            backend: UpdateBackend::Apt,
            is_new: false,
            is_kernel,
        }
    }

    #[test]
    fn filter_everything_returns_all() {
        let candidates = vec![
            make_candidate("libfoo", UpdateClassification::Standard, false),
            make_candidate("libbar", UpdateClassification::Security, false),
            make_candidate("linux-image", UpdateClassification::Security, true),
        ];
        let filtered = filter_candidates_for_mode(&candidates, SelectionMode::Everything);
        assert_eq!(filtered.len(), 3);
    }

    #[test]
    fn filter_security_only_returns_security() {
        let candidates = vec![
            make_candidate("libfoo", UpdateClassification::Standard, false),
            make_candidate("libbar", UpdateClassification::Security, false),
            make_candidate("linux-image", UpdateClassification::Security, true),
            make_candidate("unknown-pkg", UpdateClassification::Unknown, false),
        ];
        let filtered = filter_candidates_for_mode(&candidates, SelectionMode::SecurityOnly);
        assert_eq!(filtered.len(), 2);
        assert!(
            filtered
                .iter()
                .all(|c| c.classification == UpdateClassification::Security)
        );
    }

    #[test]
    fn filter_empty_input_returns_empty() {
        let candidates: Vec<UpdateCandidate> = vec![];
        let filtered_all = filter_candidates_for_mode(&candidates, SelectionMode::Everything);
        assert!(filtered_all.is_empty());
        let filtered_sec = filter_candidates_for_mode(&candidates, SelectionMode::SecurityOnly);
        assert!(filtered_sec.is_empty());
    }

    #[test]
    fn filter_security_only_with_no_security_candidates() {
        let candidates = vec![
            make_candidate("libfoo", UpdateClassification::Standard, false),
            make_candidate("unknown-pkg", UpdateClassification::Unknown, false),
        ];
        let filtered = filter_candidates_for_mode(&candidates, SelectionMode::SecurityOnly);
        assert!(filtered.is_empty());
    }

    #[test]
    fn format_filtered_summary_computes_counts() {
        let candidates = [
            make_candidate("libfoo", UpdateClassification::Standard, false),
            make_candidate("libbar", UpdateClassification::Security, false),
        ];
        let refs: Vec<&UpdateCandidate> = candidates.iter().collect();
        let summary = format_filtered_summary(&refs, 1, 0);
        assert!(summary.contains("2 to upgrade"));
        assert!(summary.contains("1 to remove"));
        assert!(summary.contains("2.0 MB to download"));
    }

    #[test]
    fn format_filtered_summary_empty_candidates() {
        let refs: Vec<&UpdateCandidate> = vec![];
        let summary = format_filtered_summary(&refs, 0, 0);
        assert_eq!(summary, "No updates available");
    }

    #[test]
    fn format_security_label_with_evidence() {
        let label = format_security_label(3, 10);
        assert_eq!(
            label,
            Some("3 of 10 updates address security issues".to_string())
        );
    }

    #[test]
    fn format_security_label_none_when_no_security() {
        let label = format_security_label(0, 5);
        assert_eq!(label, None);
    }

    #[test]
    fn format_summary_with_all_fields() {
        let summary = format_update_summary(5, 1, 2, 1, Some(85_300_000));
        assert!(summary.contains("5 to upgrade"));
        assert!(summary.contains("1 newly installed"));
        assert!(summary.contains("2 to remove"));
        assert!(summary.contains("1 held back"));
        assert!(summary.contains("85.3 MB to download"));
    }

    #[test]
    fn format_summary_empty() {
        let summary = format_update_summary(0, 0, 0, 0, None);
        assert_eq!(summary, "No updates available");
    }

    #[test]
    fn format_bytes_units() {
        assert_eq!(format_bytes(500), "500 B");
        assert_eq!(format_bytes(2_500), "2.5 kB");
        assert_eq!(format_bytes(85_300_000), "85.3 MB");
        assert_eq!(format_bytes(1_200_000_000), "1.2 GB");
    }

    #[test]
    fn filter_custom_returns_all() {
        // Custom mode returns all candidates (actual filtering by per-package
        // checkboxes is handled externally, not by this function).
        let candidates = vec![
            make_candidate("libfoo", UpdateClassification::Standard, false),
            make_candidate("libbar", UpdateClassification::Security, false),
        ];
        let filtered = filter_candidates_for_mode(&candidates, SelectionMode::Custom);
        assert_eq!(filtered.len(), 2);
    }
}
