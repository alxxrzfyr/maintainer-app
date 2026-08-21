//! Cleanup page UI.
//!
//! Implements the flow: scan → review → confirm → execute → result.
//! No "Clean Everything" action exists. Users must review and confirm
//! individual items.

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Button, Label, ListBox, Orientation, ScrolledWindow, Separator};

use crate::state::CleanupState;

/// Top-level cleanup page widget.
pub struct CleanupPage {
    container: GtkBox,
    scan_button: Button,
    status_label: Label,
    #[allow(dead_code)] // Used for signal connections in UI integration.
    items_list: ListBox,
    execute_button: Button,
    result_label: Label,
}

impl CleanupPage {
    /// Build the cleanup page widget tree.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 12);
        container.add_css_class("cleanup-page");
        container.set_margin_top(24);
        container.set_margin_bottom(24);
        container.set_margin_start(24);
        container.set_margin_end(24);

        // Header.
        let header = Label::new(Some("System Cleanup"));
        header.add_css_class("title-1");
        header.set_halign(gtk4::Align::Start);
        container.append(&header);

        let description = Label::new(Some(
            "Scan for reclaimable space in caches, thumbnails, Trash, and logs. \
             Review all items before confirming removal.",
        ));
        description.set_wrap(true);
        description.set_halign(gtk4::Align::Start);
        description.add_css_class("dim-label");
        container.append(&description);

        // Scan button.
        let scan_button = Button::with_label("Scan for Reclaimable Space");
        scan_button.add_css_class("suggested-action");
        scan_button.set_halign(gtk4::Align::Start);
        container.append(&scan_button);

        // Status label.
        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        container.append(&Separator::new(Orientation::Horizontal));

        // Scrollable items list.
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_min_content_height(200);

        let items_list = ListBox::new();
        items_list.add_css_class("boxed-list");
        items_list.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&items_list));
        container.append(&scrolled);

        // Execute button (hidden until items are reviewed).
        let execute_button = Button::with_label("Remove Selected Items");
        execute_button.add_css_class("destructive-action");
        execute_button.set_halign(gtk4::Align::Start);
        execute_button.set_visible(false);
        container.append(&execute_button);

        // Result display.
        let result_label = Label::new(None);
        result_label.set_halign(gtk4::Align::Start);
        result_label.set_wrap(true);
        result_label.set_visible(false);
        container.append(&result_label);

        Self {
            container,
            scan_button,
            status_label,
            items_list,
            execute_button,
            result_label,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Get the scan button for signal connection.
    pub fn scan_button(&self) -> &Button {
        &self.scan_button
    }

    /// Get the execute button for signal connection.
    pub fn execute_button(&self) -> &Button {
        &self.execute_button
    }

    /// Apply state to the cleanup page.
    pub fn apply_state(&self, state: &CleanupState) {
        match &state.phase {
            CleanupPhase::Idle => {
                self.scan_button.set_sensitive(true);
                self.status_label.set_visible(false);
                self.execute_button.set_visible(false);
                self.result_label.set_visible(false);
            }
            CleanupPhase::Scanning { message } => {
                self.scan_button.set_sensitive(false);
                self.status_label.set_text(message);
                self.status_label.set_visible(true);
                self.execute_button.set_visible(false);
                self.result_label.set_visible(false);
            }
            CleanupPhase::Reviewing {
                item_count,
                total_bytes,
            } => {
                self.scan_button.set_sensitive(true);
                self.status_label.set_text(&format!(
                    "Found {} items ({} reclaimable)",
                    item_count,
                    format_bytes(*total_bytes),
                ));
                self.status_label.set_visible(true);
                self.execute_button.set_visible(*item_count > 0);
                self.execute_button.set_sensitive(state.selected_count > 0);
                self.result_label.set_visible(false);
            }
            CleanupPhase::Executing { message } => {
                self.scan_button.set_sensitive(false);
                self.status_label.set_text(message);
                self.status_label.set_visible(true);
                self.execute_button.set_sensitive(false);
                self.result_label.set_visible(false);
            }
            CleanupPhase::Completed { summary } => {
                self.scan_button.set_sensitive(true);
                self.status_label.set_visible(false);
                self.execute_button.set_visible(false);
                self.result_label.set_text(summary);
                self.result_label.set_visible(true);
            }
        }
    }
}

impl Default for CleanupPage {
    fn default() -> Self {
        Self::new()
    }
}

/// Phase of the cleanup workflow visible to the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupPhase {
    /// No scan running, no results.
    Idle,
    /// Scan in progress.
    Scanning { message: String },
    /// Results available for review.
    Reviewing { item_count: usize, total_bytes: u64 },
    /// Execution in progress.
    Executing { message: String },
    /// Execution completed.
    Completed { summary: String },
}

impl Default for CleanupPhase {
    fn default() -> Self {
        Self::Idle
    }
}

/// Format bytes in human-readable form.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * 1024;
    const GIB: u64 = 1024 * 1024 * 1024;

    if bytes >= GIB {
        format!("{:.1} GiB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MiB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1} KiB", bytes as f64 / KIB as f64)
    } else {
        format!("{bytes} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(500), "500 B");
    }

    #[test]
    fn format_bytes_kib() {
        assert_eq!(format_bytes(2048), "2.0 KiB");
    }

    #[test]
    fn format_bytes_mib() {
        assert_eq!(format_bytes(5 * 1024 * 1024), "5.0 MiB");
    }

    #[test]
    fn format_bytes_gib() {
        assert_eq!(format_bytes(2 * 1024 * 1024 * 1024), "2.0 GiB");
    }

    #[test]
    fn cleanup_phase_default_is_idle() {
        assert_eq!(CleanupPhase::default(), CleanupPhase::Idle);
    }
}
