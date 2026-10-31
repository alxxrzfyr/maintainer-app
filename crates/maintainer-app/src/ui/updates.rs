//! Updates page UI.
//!
//! Implements the update flow: Check → Review → Confirm → Authenticate → Execute → Result.
//!
//! Invariants:
//! - Security labels only appear when backend evidence supports them.
//! - Cancellation is disabled when the backend reports it unsafe.
//! - No credentials are stored; authentication is delegated to the fixed helper/Polkit path.
//! - Installation requires a second, authoritative review from the native package owner.

use gtk4::prelude::*;
use gtk4::{
    self, Box as GtkBox, Button, CheckButton, Image, Label, ListBox, Orientation, ProgressBar,
    ScrolledWindow,
};
use maintainer_core::updates::{
    HoldState, UpdateBackend, UpdateCandidate, UpdateClassification, UpdatePlan,
};

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::rc::Rc;
use std::time::Duration;

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

fn progress_fraction(completed: u64, total: Option<u64>) -> Option<f64> {
    total
        .filter(|total| *total > 0)
        .map(|total| (completed as f64 / total as f64).clamp(0.0, 1.0))
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
    resume_button: Button,
    recheck_button: Button,
    kernel_expander: gtk4::Expander,
    kernels: Rc<RefCell<crate::ui::kernel_section::KernelSection>>,
    status_label: Label,
    // Selection mode toggle
    mode_row: GtkBox,
    custom_selection_label: Label,
    everything_toggle: gtk4::ToggleButton,
    security_toggle: gtk4::ToggleButton,
    selection_mode: Rc<Cell<SelectionMode>>,
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
    progress_indeterminate: Rc<Cell<bool>>,
    progress_generation: Rc<Cell<u64>>,
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
            "Check available updates, then review the complete locked package plan before applying changes.",
        ));
        description.set_wrap(true);
        description.set_halign(gtk4::Align::Start);
        description.add_css_class("dim-label");
        container.append(&description);

        // --- Check button ---
        let check_button = Button::with_label("Check for Updates");
        check_button.add_css_class("suggested-action");
        check_button.set_halign(gtk4::Align::Start);
        let header_actions = GtkBox::new(Orientation::Horizontal, 8);
        header_actions.append(&check_button);
        let resume_button = Button::with_label("Resume Package Operation");
        header_actions.append(&resume_button);
        let recheck_button = Button::with_label("Recheck recovery");
        recheck_button.set_tooltip_text(Some(
            "Read-only check after repairing the package manager with your normal system tools.",
        ));
        recheck_button.set_visible(false);
        header_actions.append(&recheck_button);
        container.append(&header_actions);

        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        // --- Selection mode row (hidden until reviewing) ---
        // Wraps the coarse toggle and the custom-selection indicator together
        // so the indicator reads as "what replaced the toggle" rather than an
        // unrelated label elsewhere on the page.
        let mode_row = GtkBox::new(Orientation::Horizontal, 8);
        mode_row.set_halign(gtk4::Align::Start);
        mode_row.set_visible(false);

        let mode_box = GtkBox::new(Orientation::Horizontal, 0);
        mode_box.add_css_class("linked");

        let everything_toggle = gtk4::ToggleButton::with_label("Everything");
        everything_toggle.set_active(true);

        let security_toggle = gtk4::ToggleButton::with_label("Security Only");
        security_toggle.set_group(Some(&everything_toggle));

        mode_box.append(&everything_toggle);
        mode_box.append(&security_toggle);
        mode_row.append(&mode_box);

        // Shown only while per-package checkboxes have overridden the coarse
        // toggle, so the mode change is never communicated by the toggle
        // buttons silently going blank alone.
        let custom_selection_label = Label::new(Some("Custom selection"));
        custom_selection_label.add_css_class("custom-selection-indicator");
        custom_selection_label.set_visible(false);
        custom_selection_label.set_tooltip_text(Some(
            "A package was individually checked or unchecked, overriding the \
             Everything / Security Only toggle above.",
        ));
        mode_row.append(&custom_selection_label);

        container.append(&mode_row);

        // --- Top action bar (summary + Install button) ----
        // Pinned above the package list's own ScrolledWindow, not the page
        // viewport: the page itself does not scroll as a whole, only the
        // package list does, so this bar stays visible without needing a
        // GTK "sticky" mechanism.
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

        let confirm_button = Button::with_label("Review Installation");
        confirm_button.add_css_class("suggested-action");
        confirm_button.set_sensitive(false);
        confirm_button.set_tooltip_text(Some(
            "Authenticate and review the complete native package plan before applying it.",
        ));
        action_bar.append(&confirm_button);

        container.append(&action_bar);

        // --- Reboot notice (page-level, visible when kernel update + reboot required) ---
        let reboot_notice = Label::new(Some(
            "⟳ A system restart will be required after installing these updates.",
        ));
        reboot_notice.add_css_class("reboot-notice");
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
        cancel_button.add_css_class("maintainer_destructive");
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

        let kernels = Rc::new(RefCell::new(crate::ui::kernel_section::KernelSection::new()));
        let kernel_scroll = ScrolledWindow::builder()
            .max_content_height(200)
            .propagate_natural_height(true)
            .child(kernels.borrow().widget())
            .build();
        let kernel_expander = gtk4::Expander::builder()
            .label("Installed kernels")
            .expanded(false)
            .child(&kernel_scroll)
            .build();
        container.append(&kernel_expander);

        Self {
            container,
            check_button,
            resume_button,
            recheck_button,
            kernel_expander,
            kernels,
            status_label,
            mode_row,
            custom_selection_label,
            everything_toggle,
            security_toggle,
            selection_mode: Rc::new(Cell::new(SelectionMode::Everything)),
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
            progress_indeterminate: Rc::new(Cell::new(false)),
            progress_generation: Rc::new(Cell::new(0)),
            result_label,
            phase: UpdatePhase::Idle,
        }
    }

    /// Returns the top-level container widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    pub fn resume_button(&self) -> &Button {
        &self.resume_button
    }

    pub fn recheck_button(&self) -> &Button {
        &self.recheck_button
    }

    pub fn kernel_expander(&self) -> &gtk4::Expander {
        &self.kernel_expander
    }

    pub fn kernels(&self) -> Rc<RefCell<crate::ui::kernel_section::KernelSection>> {
        self.kernels.clone()
    }

    /// Returns the current update phase.
    pub fn phase(&self) -> UpdatePhase {
        self.phase
    }

    /// Returns the current selection mode.
    pub fn selection_mode(&self) -> SelectionMode {
        self.selection_mode.get()
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
    ///
    /// Keeps the visible "Custom selection" indicator in lockstep with the
    /// mode, so Custom can never be active while the indicator is hidden.
    pub fn set_selection_mode(&mut self, mode: SelectionMode) {
        self.selection_mode.set(mode);
        self.custom_selection_label
            .set_visible(mode == SelectionMode::Custom);
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
        self.confirm_button
            .set_sensitive(!self.selected_packages.borrow().is_empty());
    }

    /// Transition to the checking state.
    pub fn set_checking(&mut self) {
        self.phase = UpdatePhase::Checking;
        self.review_button.set_sensitive(true);
        self.check_button.set_label("Checking for Updates…");
        self.check_button.set_sensitive(false);
        self.status_label.set_text("Checking for updates…");
        self.status_label.set_visible(true);
        self.action_bar.set_visible(false);
        self.confirm_button.set_sensitive(false);
        self.review_button.set_visible(false);
        self.mode_row.set_visible(false);
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
        // Keep inspection summary visible while installation stays unavailable.
        self.action_bar.set_visible(true);
        self.summary_label.set_text(summary);
        self.confirm_button.set_sensitive(false);
        // Keep coarse mode toggle hidden by default for a clean, unified view
        self.mode_row.set_visible(false);
    }

    /// Show the read-only update plan for inspection.
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
    ///
    /// Kernel candidates sharing a version are grouped into a single visual row
    /// (display-only grouping). The full ungrouped candidate list is preserved
    /// in `selected_packages` and flows to execution unchanged.
    pub fn populate_package_list(&self, candidates: &[UpdateCandidate]) {
        use super::kernel_grouping::{DisplayRow, group_kernel_candidates};

        while let Some(child) = self.package_list.first_child() {
            self.package_list.remove(&child);
        }

        // Initialize selected_packages to contain all displayed candidates.
        {
            let mut selected = self.selected_packages.borrow_mut();
            selected.clear();
            for c in candidates {
                selected.insert(c.selection_key());
            }
        }
        // A fresh populate means selection matches the plan; mark as replanned.
        *self.selection_replanned.borrow_mut() = true;
        // This enables requesting an authoritative plan; Apply exists only in its later dialog.
        self.review_button.set_visible(false);
        self.confirm_button.set_sensitive(!candidates.is_empty());

        let display_rows = group_kernel_candidates(candidates);

        for display_row in &display_rows {
            match display_row {
                DisplayRow::Single(idx) => {
                    let candidate = &candidates[*idx];
                    self.append_single_candidate_row(candidate);
                }
                DisplayRow::KernelGroup(group) => {
                    self.append_kernel_group_row(group);
                }
            }
        }
    }

    /// Render a single non-kernel candidate as one list row.
    fn append_single_candidate_row(&self, candidate: &UpdateCandidate) {
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
        let title = Label::new(Some(&candidate.selection_key()));
        title.set_halign(gtk4::Align::Start);
        title.add_css_class("heading");
        title_box.append(&title);

        // Category / Desktop environment badge
        let desktop_env = maintainer_core::updates::DesktopEnvironment::detect();
        let category = maintainer_core::updates::classify_package(&candidate.name, desktop_env);
        let cat_badge = Label::new(Some(category.badge_label()));
        cat_badge.add_css_class(category.css_class());
        title_box.append(&cat_badge);

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
        let pkg_name = candidate.selection_key();
        let selected_ref = self.selected_packages.clone();
        let replanned_ref = self.selection_replanned.clone();
        let review_btn = self.review_button.clone();
        let confirm_btn = self.confirm_button.clone();
        let everything_toggle = self.everything_toggle.clone();
        let security_toggle = self.security_toggle.clone();
        let custom_label = self.custom_selection_label.clone();
        let selection_mode = self.selection_mode.clone();

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
            selection_mode.set(SelectionMode::Custom);

            // Deactivate coarse toggle buttons to indicate Custom mode,
            // and show the indicator label so the mode change is visible
            // rather than communicated only by the toggles going blank.
            everything_toggle.set_active(false);
            security_toggle.set_active(false);
            custom_label.set_visible(true);

            // Show Review Selection button, disable Install.
            review_btn.set_visible(true);
            confirm_btn.set_sensitive(false);
        });
    }

    /// Render a grouped kernel update as one list row with a count badge.
    ///
    /// A single checkbox controls all packages in the group: toggling it
    /// inserts or removes every package name in the group from the selection
    /// set atomically.
    fn append_kernel_group_row(&self, group: &super::kernel_grouping::KernelGroup) {
        let outer_row = GtkBox::new(Orientation::Horizontal, 8);
        outer_row.set_margin_top(6);
        outer_row.set_margin_bottom(6);
        outer_row.set_margin_start(8);
        outer_row.set_margin_end(8);

        // --- Group checkbox (selects/deselects ALL packages in the group) ---
        let check = CheckButton::new();
        check.set_active(true);
        check.set_valign(gtk4::Align::Center);
        outer_row.append(&check);

        let row = GtkBox::new(Orientation::Vertical, 4);

        let title_box = GtkBox::new(Orientation::Horizontal, 8);
        let architectures: std::collections::BTreeSet<_> = group
            .package_names
            .iter()
            .filter_map(|name| name.rsplit_once(':').map(|(_, architecture)| architecture))
            .collect();
        let title_text = if architectures.is_empty() {
            format!("Linux kernel {}", group.version)
        } else {
            format!(
                "Linux kernel {} ({})",
                group.version,
                architectures.into_iter().collect::<Vec<_>>().join(", ")
            )
        };
        let title = Label::new(Some(&title_text));
        title.set_halign(gtk4::Align::Start);
        title.add_css_class("heading");
        title_box.append(&title);

        let cat_badge = Label::new(Some("Kernel"));
        cat_badge.add_css_class("maintainer_badge_kernel");
        title_box.append(&cat_badge);

        // Kernel icon (reuse existing system-run-symbolic convention)
        let kernel_icon = Image::from_icon_name("system-run-symbolic");
        kernel_icon.set_icon_size(gtk4::IconSize::Normal);
        kernel_icon.set_tooltip_text(Some("Kernel update"));
        kernel_icon.add_css_class("kernel-icon");
        title_box.append(&kernel_icon);

        // Security icon if any package in the group is a security update
        if group.is_security {
            let sec_icon = Image::from_icon_name("security-high-symbolic");
            sec_icon.set_icon_size(gtk4::IconSize::Normal);
            sec_icon.set_tooltip_text(Some("Security update"));
            sec_icon.add_css_class("security-icon");
            title_box.append(&sec_icon);
        }

        // Count badge
        let badge_text = format!("{} packages", group.package_count);
        let badge = Label::new(Some(&badge_text));
        badge.add_css_class("dim-label");
        badge.set_tooltip_text(Some(&group.package_names.join("\n")));
        title_box.append(&badge);

        row.append(&title_box);

        // Version transition line
        if let Some(ref transition) = group.version_transition {
            let versions = Label::new(Some(transition));
            versions.set_halign(gtk4::Align::Start);
            versions.add_css_class("dim-label");
            row.append(&versions);
        }

        outer_row.append(&row);
        self.package_list.append(&outer_row);

        // --- Group checkbox toggle handler: select/deselect ALL packages ---
        let group_packages = group.package_names.clone();
        let selected_ref = self.selected_packages.clone();
        let replanned_ref = self.selection_replanned.clone();
        let review_btn = self.review_button.clone();
        let confirm_btn = self.confirm_button.clone();
        let everything_toggle = self.everything_toggle.clone();
        let security_toggle = self.security_toggle.clone();
        let custom_label = self.custom_selection_label.clone();
        let selection_mode = self.selection_mode.clone();

        check.connect_toggled(move |btn| {
            let mut selected = selected_ref.borrow_mut();
            if btn.is_active() {
                // Select ALL packages in the kernel group.
                for pkg in &group_packages {
                    selected.insert(pkg.clone());
                }
            } else {
                // Deselect ALL packages in the kernel group.
                for pkg in &group_packages {
                    selected.remove(pkg);
                }
            }
            drop(selected);

            // Any manual checkbox toggle invalidates the current re-plan
            // and switches to Custom mode.
            *replanned_ref.borrow_mut() = false;
            selection_mode.set(SelectionMode::Custom);

            // Deactivate coarse toggle buttons to indicate Custom mode.
            everything_toggle.set_active(false);
            security_toggle.set_active(false);
            custom_label.set_visible(true);

            // Show Review Selection button, disable Install.
            review_btn.set_visible(true);
            confirm_btn.set_sensitive(false);
        });
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
        self.mode_row.set_visible(false);
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
        self.mode_row.set_visible(false);
        self.reboot_notice.set_visible(false);
    }

    /// Transition to executing state.
    pub fn set_executing(&mut self) {
        self.phase = UpdatePhase::Executing;
        self.action_bar.set_visible(false);
        self.confirm_button.set_sensitive(false);
        self.review_button.set_sensitive(false);
        self.mode_row.set_visible(false);
        self.reboot_notice.set_visible(false);
        self.result_label.set_visible(false);
        self.check_button.set_label("Installing Updates…");
        self.check_button.set_sensitive(false);
        self.progress_bar.set_visible(true);
        self.progress_bar.set_fraction(0.0);
        self.progress_bar.set_text(Some("In progress"));
        self.progress_bar.set_pulse_step(0.08);
        self.progress_label.set_visible(true);
        self.progress_label.set_text("Preparing…");
        self.cancel_button.set_visible(true);
        self.cancel_button.set_sensitive(true);
        self.cancel_button.set_tooltip_text(None);

        self.progress_indeterminate.set(true);
        let generation = self.progress_generation.get().wrapping_add(1);
        self.progress_generation.set(generation);
        let progress_bar = self.progress_bar.clone();
        let indeterminate = self.progress_indeterminate.clone();
        let current_generation = self.progress_generation.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || {
            if current_generation.get() != generation || !progress_bar.is_visible() {
                return glib::ControlFlow::Break;
            }
            if indeterminate.get() {
                progress_bar.pulse();
            }
            glib::ControlFlow::Continue
        });
    }

    /// Update progress display during execution.
    pub fn set_progress(
        &self,
        completed: u64,
        total: Option<u64>,
        message: &str,
        allow_cancel: bool,
    ) {
        match progress_fraction(completed, total) {
            Some(fraction) => {
                self.progress_indeterminate.set(false);
                self.progress_bar.set_fraction(fraction);
                self.progress_bar.set_text(None);
            }
            None => {
                self.progress_indeterminate.set(true);
                self.progress_bar.pulse();
                self.progress_bar.set_text(Some("In progress"));
            }
        }
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
        self.progress_indeterminate.set(false);
        self.progress_generation
            .set(self.progress_generation.get().wrapping_add(1));
        self.progress_bar.set_visible(false);
        self.progress_label.set_visible(false);
        self.cancel_button.set_visible(false);
        self.action_bar.set_visible(false);
        self.mode_row.set_visible(false);
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
            architecture: None,
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
    fn installation_review_requires_a_matching_selection() {
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        let mut page = UpdatesPage::new();
        page.set_checking();
        assert!(!page.confirm_button.is_sensitive());
        page.populate_package_list(&[make_candidate(
            "curl",
            UpdateClassification::Standard,
            false,
        )]);
        assert!(page.confirm_button.is_sensitive());
        assert_eq!(
            page.confirm_button.label().as_deref(),
            Some("Review Installation")
        );
        *page.selection_replanned.borrow_mut() = false;
        page.confirm_button.set_sensitive(false);
        page.set_selection_mode(SelectionMode::Custom);
        assert!(!page.confirm_button.is_sensitive());
        page.mark_selection_replanned();
        assert!(page.confirm_button.is_sensitive());
        assert!(!page.kernel_expander.is_expanded());
    }

    #[test]
    fn multiarch_checkboxes_preserve_only_the_selected_exact_request() {
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        let page = UpdatesPage::new();
        let mut native = make_candidate("libexample", UpdateClassification::Standard, false);
        native.architecture = Some("amd64".into());
        let mut foreign = native.clone();
        foreign.architecture = Some("i386".into());
        let candidates = vec![native, foreign];
        page.populate_package_list(&candidates);
        assert_eq!(page.selected_packages().len(), 2);
        let row = page
            .package_list
            .first_child()
            .unwrap()
            .downcast::<gtk4::ListBoxRow>()
            .unwrap();
        let contents = row.child().unwrap().downcast::<GtkBox>().unwrap();
        let checkbox = contents
            .first_child()
            .unwrap()
            .downcast::<CheckButton>()
            .unwrap();
        checkbox.set_active(false);
        assert_eq!(
            page.selected_packages(),
            HashSet::from(["libexample:i386".into()])
        );
        assert!(!page.is_selection_replanned());
        assert!(!page.confirm_button().is_sensitive());
        let selected = page.selected_packages();
        let requests: Vec<_> = candidates
            .iter()
            .filter(|candidate| selected.contains(&candidate.selection_key()))
            .map(|candidate| {
                maintainer_core::updates::UpdateRequest {
                    name: candidate.name.clone(),
                    architecture: candidate.architecture.clone(),
                    version: candidate.available_version.clone(),
                }
                .package_spec()
            })
            .collect();
        assert_eq!(requests, vec!["libexample:i386=2.0.0"]);
        let mut qualified = candidates[1].clone();
        qualified.name = "libexample:i386".into();
        assert_eq!(qualified.selection_key(), "libexample:i386");
    }

    #[test]
    fn cloned_pages_share_selection_mode() {
        assert!(
            !gtk4::is_initialized() || gtk4::is_initialized_main_thread(),
            "Run each GTK fixture in its own process using scripts/test-native.sh"
        );
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        let page = UpdatesPage::new();
        let mut cloned = page.clone();
        cloned.set_selection_mode(SelectionMode::SecurityOnly);
        assert_eq!(page.selection_mode(), SelectionMode::SecurityOnly);
        cloned.set_selection_mode(SelectionMode::Custom);
        assert_eq!(page.selection_mode(), SelectionMode::Custom);
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

#[cfg(test)]
mod progress_tests {
    use super::progress_fraction;

    #[test]
    fn unknown_total_is_indeterminate_not_zero_percent() {
        assert_eq!(progress_fraction(0, None), None);
        assert_eq!(progress_fraction(50, None), None);
        assert_eq!(progress_fraction(1, Some(0)), None);
    }

    #[test]
    fn known_total_produces_clamped_fraction() {
        assert_eq!(progress_fraction(25, Some(100)), Some(0.25));
        assert_eq!(progress_fraction(150, Some(100)), Some(1.0));
    }
}
