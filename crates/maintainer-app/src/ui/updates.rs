//! Updates page UI.
//!
//! Implements the update flow: Check → Review → Confirm → Authenticate → Execute → Result.
//!
//! Invariants:
//! - Security labels only appear when backend evidence supports them.
//! - Cancellation is disabled when the backend reports it unsafe.
//! - No credentials are stored; authentication is delegated to PackageKit/Polkit.

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Button, Label, ListBox, Orientation, ProgressBar, ScrolledWindow};

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
pub struct UpdatesPage {
    container: GtkBox,
    check_button: Button,
    status_label: Label,
    #[allow(dead_code)]
    package_list: ListBox,
    confirm_button: Button,
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

        // Header.
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

        // Check button.
        let check_button = Button::with_label("Check for Updates");
        check_button.add_css_class("suggested-action");
        check_button.set_halign(gtk4::Align::Start);
        container.append(&check_button);

        // Status label.
        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        // Scrollable package list.
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_min_content_height(200);
        scrolled.set_visible(false);

        let package_list = ListBox::new();
        package_list.add_css_class("boxed-list");
        package_list.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&package_list));
        container.append(&scrolled);

        // Confirm button (shown during review).
        let confirm_button = Button::with_label("Install Updates");
        confirm_button.add_css_class("suggested-action");
        confirm_button.set_halign(gtk4::Align::Start);
        confirm_button.set_visible(false);
        container.append(&confirm_button);

        // Cancel button (shown during execution when allowed).
        let cancel_button = Button::with_label("Cancel");
        cancel_button.add_css_class("destructive-action");
        cancel_button.set_halign(gtk4::Align::Start);
        cancel_button.set_visible(false);
        container.append(&cancel_button);

        // Progress display.
        let progress_bar = ProgressBar::new();
        progress_bar.set_visible(false);
        progress_bar.set_show_text(true);
        container.append(&progress_bar);

        let progress_label = Label::new(None);
        progress_label.set_halign(gtk4::Align::Start);
        progress_label.set_visible(false);
        container.append(&progress_label);

        // Result display.
        let result_label = Label::new(None);
        result_label.set_halign(gtk4::Align::Start);
        result_label.set_visible(false);
        result_label.set_wrap(true);
        container.append(&result_label);

        Self {
            container,
            check_button,
            status_label,
            package_list,
            confirm_button,
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

    /// Transition to the checking state.
    pub fn set_checking(&mut self) {
        self.phase = UpdatePhase::Checking;
        self.check_button.set_sensitive(false);
        self.status_label.set_text("Checking for updates…");
        self.status_label.set_visible(true);
        self.confirm_button.set_visible(false);
        self.result_label.set_visible(false);
    }

    /// Transition to reviewing state with update summary.
    pub fn set_reviewing(&mut self, summary: &str) {
        self.phase = UpdatePhase::Reviewing;
        self.check_button.set_sensitive(true);
        self.status_label.set_text(summary);
        self.status_label.set_visible(true);
        self.confirm_button.set_visible(true);
        self.confirm_button.set_sensitive(true);
    }

    /// Show that no updates are available.
    pub fn set_up_to_date(&mut self) {
        self.phase = UpdatePhase::Idle;
        self.check_button.set_sensitive(true);
        self.status_label.set_text("System is up to date.");
        self.status_label.set_visible(true);
        self.confirm_button.set_visible(false);
    }

    /// Show a discovery error.
    pub fn set_check_failed(&mut self, message: &str) {
        self.phase = UpdatePhase::Idle;
        self.check_button.set_sensitive(true);
        self.status_label.set_text(message);
        self.status_label.set_visible(true);
        self.status_label.add_css_class("error");
        self.confirm_button.set_visible(false);
    }

    /// Transition to executing state.
    pub fn set_executing(&mut self) {
        self.phase = UpdatePhase::Executing;
        self.confirm_button.set_visible(false);
        self.check_button.set_sensitive(false);
        self.progress_bar.set_visible(true);
        self.progress_bar.set_fraction(0.0);
        self.progress_label.set_visible(true);
        self.progress_label.set_text("Preparing…");
        self.cancel_button.set_visible(true);
        self.cancel_button.set_sensitive(true);
    }

    /// Update progress display during execution.
    pub fn set_progress(&mut self, fraction: f64, message: &str, allow_cancel: bool) {
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
}
