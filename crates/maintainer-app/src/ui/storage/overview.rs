//! Storage overview sub-page showing scan root selection, scan status, and
//! summary statistics.

use gtk4::prelude::*;
use gtk4::{self, Align, Box as GtkBox, Button, Image, Label, Orientation};

use crate::state::{ScanPhase, StorageState};
use crate::ui::widgets::job_progress::JobProgress;

/// The overview panel for storage analysis: root selection, scan controls,
/// and summary statistics.
#[derive(Clone)]
pub struct StorageOverview {
    container: GtkBox,
    root_label: Label,
    change_root_button: Button,
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

        let header = Label::new(Some("Storage Analysis"));
        header.add_css_class("title-1");
        header.set_halign(Align::Start);
        container.append(&header);

        let subtitle = Label::new(Some(
            "Scan file trees for large space consumers, inspect directory trees, and detect duplicate files.",
        ));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(Align::Start);
        subtitle.set_wrap(true);
        container.append(&subtitle);

        let root_card = GtkBox::new(Orientation::Vertical, 0);
        root_card.add_css_class("maintainer_surface");
        root_card.set_margin_top(4);
        root_card.set_margin_bottom(4);

        let root_inner = GtkBox::new(Orientation::Horizontal, 12);
        root_inner.set_margin_top(14);
        root_inner.set_margin_bottom(14);
        root_inner.set_margin_start(18);
        root_inner.set_margin_end(18);
        root_inner.set_valign(Align::Center);

        let folder_icon = Image::from_icon_name("folder-symbolic");
        folder_icon.set_pixel_size(18);
        folder_icon.add_css_class("maintainer_text_muted");
        root_inner.append(&folder_icon);

        let root_title = Label::new(Some("Scan Target:"));
        root_title.set_halign(Align::Start);
        root_title.add_css_class("heading");
        root_inner.append(&root_title);

        let root_label = Label::new(Some("No directory selected"));
        root_label.set_halign(Align::Start);
        root_label.set_hexpand(true);
        root_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        root_inner.append(&root_label);

        let change_root_button = Button::with_label("Change Folder…");
        change_root_button.set_valign(Align::Center);
        root_inner.append(&change_root_button);

        root_card.append(&root_inner);
        container.append(&root_card);

        let button_row = GtkBox::new(Orientation::Horizontal, 12);
        let scan_button = Button::with_label("Scan Storage");
        scan_button.add_css_class("suggested-action");

        let duplicate_button = Button::with_label("Find Duplicates");
        duplicate_button.add_css_class("suggested-action");

        button_row.append(&scan_button);
        button_row.append(&duplicate_button);
        container.append(&button_row);

        let progress = JobProgress::new("Ready");
        progress.set_visible(false);
        container.append(progress.widget());

        let summary_label = Label::new(None);
        summary_label.set_halign(Align::Start);
        summary_label.set_wrap(true);
        summary_label.add_css_class("storage-summary");
        container.append(&summary_label);

        let issues_label = Label::new(None);
        issues_label.set_halign(Align::Start);
        issues_label.add_css_class("dim-label");
        container.append(&issues_label);

        Self {
            container,
            root_label,
            change_root_button,
            scan_button,
            duplicate_button,
            progress,
            summary_label,
            issues_label,
        }
    }

    /// Access change root button for signal connections.
    pub fn change_root_button(&self) -> &Button {
        &self.change_root_button
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update the overview from current state.
    pub fn apply_state(&self, state: &StorageState) {
        match &state.scan_root {
            Some(path) => self.root_label.set_text(&path.to_string_lossy()),
            None => self.root_label.set_text("No directory selected"),
        }

        let is_storage_running = matches!(
            state.storage_scan.as_ref().map(|s| &s.phase),
            Some(ScanPhase::Running { .. })
        );
        let is_dup_running = matches!(
            state.duplicate_scan.as_ref().map(|s| &s.phase),
            Some(ScanPhase::Running { .. })
        );

        let can_scan = state.scan_root.is_some() && !is_storage_running && !is_dup_running;
        self.scan_button.set_sensitive(can_scan);
        if is_storage_running {
            self.scan_button.set_label("Scanning Storage…");
        } else {
            self.scan_button.set_label("Scan Storage");
        }

        let can_dup = state.scan_root.is_some() && !is_storage_running && !is_dup_running;
        self.duplicate_button.set_sensitive(can_dup);
        if is_dup_running {
            self.duplicate_button.set_label("Finding Duplicates…");
        } else {
            self.duplicate_button.set_label("Find Duplicates");
        }

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
        } else if let Some(ref scan) = state.duplicate_scan {
            match &scan.phase {
                ScanPhase::Running { completed, total } => {
                    self.progress.set_visible(true);
                    self.progress.set_status("Finding duplicate files…");
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

    /// Show scan progress directly without full state re-application.
    pub fn show_progress(&self, completed: u64, total: Option<u64>, message: &str) {
        self.progress.set_visible(true);
        self.progress.set_status(message);
        self.progress.set_progress(completed, total);
        self.progress.set_cancel_visible(true);
    }

    /// Hide the progress widget (e.g. on completion or cancellation).
    pub fn hide_progress(&self) {
        self.progress.set_visible(false);
    }

    /// Show a storage or duplicate scan failure.
    pub fn show_error(&self, message: &str) {
        self.progress.set_visible(true);
        self.progress.set_cancel_visible(false);
        self.progress.set_status(message);
    }

    /// Get a clone of the progress widget for use in progress closures.
    pub fn progress_handle(&self) -> crate::ui::widgets::job_progress::JobProgress {
        self.progress.clone()
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
