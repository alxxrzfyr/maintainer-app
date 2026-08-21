//! Storage overview sub-page showing scan root selection, scan status, and
//! summary statistics.

use gtk4::prelude::*;
use gtk4::{self, Align, Box as GtkBox, Button, Label, Orientation};

use crate::state::{ScanPhase, StorageState};
use crate::ui::widgets::job_progress::JobProgress;

/// The overview panel for storage analysis: root selection, scan controls,
/// and summary statistics.
pub struct StorageOverview {
    container: GtkBox,
    root_label: Label,
    scan_button: Button,
    duplicate_button: Button,
    progress: JobProgress,
    summary_label: Label,
    issues_label: Label,
}

impl StorageOverview {
    /// Build the overview widget tree.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 16);
        container.set_margin_top(24);
        container.set_margin_bottom(24);
        container.set_margin_start(24);
        container.set_margin_end(24);
        container.add_css_class("storage-overview");

        // Root selection row.
        let root_row = GtkBox::new(Orientation::Horizontal, 12);
        let root_title = Label::new(Some("Scan root:"));
        root_title.set_halign(Align::Start);
        root_title.add_css_class("dim-label");

        let root_label = Label::new(Some("No directory selected"));
        root_label.set_halign(Align::Start);
        root_label.set_hexpand(true);
        root_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);

        root_row.append(&root_title);
        root_row.append(&root_label);
        container.append(&root_row);

        // Scan action buttons.
        let button_row = GtkBox::new(Orientation::Horizontal, 12);
        let scan_button = Button::with_label("Scan Storage");
        scan_button.add_css_class("suggested-action");

        let duplicate_button = Button::with_label("Find Duplicates");
        duplicate_button.add_css_class("suggested-action");

        button_row.append(&scan_button);
        button_row.append(&duplicate_button);
        container.append(&button_row);

        // Progress indicator.
        let progress = JobProgress::new("Ready");
        progress.set_visible(false);
        container.append(progress.widget());

        // Summary.
        let summary_label = Label::new(None);
        summary_label.set_halign(Align::Start);
        summary_label.set_wrap(true);
        summary_label.add_css_class("storage-summary");
        container.append(&summary_label);

        // Issues count.
        let issues_label = Label::new(None);
        issues_label.set_halign(Align::Start);
        issues_label.add_css_class("dim-label");
        container.append(&issues_label);

        Self {
            container,
            root_label,
            scan_button,
            duplicate_button,
            progress,
            summary_label,
            issues_label,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update the overview from current state.
    pub fn apply_state(&self, state: &StorageState) {
        // Root label.
        match &state.scan_root {
            Some(path) => self.root_label.set_text(&path.to_string_lossy()),
            None => self.root_label.set_text("No directory selected"),
        }

        // Scan button sensitivity.
        let can_scan = state.scan_root.is_some()
            && !matches!(
                state.storage_scan.as_ref().map(|s| &s.phase),
                Some(ScanPhase::Running { .. })
            );
        self.scan_button.set_sensitive(can_scan);

        let can_dup = state.scan_root.is_some()
            && !matches!(
                state.duplicate_scan.as_ref().map(|s| &s.phase),
                Some(ScanPhase::Running { .. })
            );
        self.duplicate_button.set_sensitive(can_dup);

        // Progress.
        if let Some(ref scan) = state.storage_scan {
            match &scan.phase {
                ScanPhase::Running { completed, total } => {
                    self.progress.set_visible(true);
                    self.progress.set_status("Scanning storage…");
                    self.progress.set_progress(*completed, *total);
                    self.progress.set_cancel_visible(true);
                }
                ScanPhase::Completed => {
                    self.progress.set_visible(false);
                }
                ScanPhase::Cancelled => {
                    self.progress.set_visible(false);
                }
            }
        } else {
            self.progress.set_visible(false);
        }

        // Summary.
        let file_count = state.large_files.len();
        let issue_count = state.scan_issues.len();
        if file_count > 0 {
            self.summary_label
                .set_text(&format!("{file_count} large file(s) found."));
        } else if state.storage_scan.is_some() {
            self.summary_label.set_text("No large files found.");
        } else {
            self.summary_label.set_text("");
        }

        if issue_count > 0 {
            self.issues_label
                .set_text(&format!("{issue_count} recoverable issue(s) during scan."));
            self.issues_label.set_visible(true);
        } else {
            self.issues_label.set_visible(false);
        }
    }

    /// Connect the scan button click handler.
    pub fn connect_scan<F: Fn() + 'static>(&self, callback: F) {
        self.scan_button.connect_clicked(move |_| callback());
    }

    /// Connect the duplicate scan button click handler.
    pub fn connect_duplicate_scan<F: Fn() + 'static>(&self, callback: F) {
        self.duplicate_button.connect_clicked(move |_| callback());
    }

    /// Connect progress cancel.
    pub fn connect_cancel<F: Fn() + 'static>(&self, callback: F) {
        self.progress.connect_cancel(callback);
    }
}

impl Default for StorageOverview {
    fn default() -> Self {
        Self::new()
    }
}
