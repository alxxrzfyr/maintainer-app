//! Cleanup page UI.
//!
//! Implements the flow: scan → review → confirm → execute → result.
//! No "Clean Everything" action exists. Users must review and confirm
//! individual items.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    self, Box as GtkBox, Button, CheckButton, Label, ListBox, Orientation, ScrolledWindow,
    Separator,
};
use maintainer_core::cleanup::CleanupPlan;

use crate::state::CleanupState;

/// Top-level cleanup page widget.
#[derive(Clone)]
pub struct CleanupPage {
    container: GtkBox,
    scan_button: Button,
    package_cache_button: Button,
    journal_vacuum_button: Button,
    status_label: Label,
    select_toolbar: GtkBox,
    items_list: ListBox,
    execute_button: Button,
    result_label: Label,
    selections: Rc<RefCell<Vec<bool>>>,
    check_buttons: Rc<RefCell<Vec<CheckButton>>>,
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

        let action_row = GtkBox::new(Orientation::Horizontal, 8);
        action_row.set_vexpand(false);
        let scan_button = Button::with_label("Scan for Reclaimable Space");
        scan_button.add_css_class("suggested-action");
        let package_cache_button = Button::with_label("Clean Package Cache");
        let journal_vacuum_button = Button::with_label("Vacuum Journal");
        action_row.append(&scan_button);
        action_row.append(&package_cache_button);
        action_row.append(&journal_vacuum_button);
        action_row.set_halign(gtk4::Align::Start);
        container.append(&action_row);

        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        container.append(&Separator::new(Orientation::Horizontal));

        let select_toolbar = GtkBox::new(Orientation::Horizontal, 8);
        select_toolbar.set_halign(gtk4::Align::End);
        let select_all_button = Button::with_label("Select All");
        select_all_button.add_css_class("flat");
        let deselect_all_button = Button::with_label("Deselect All");
        deselect_all_button.add_css_class("flat");
        select_toolbar.append(&select_all_button);
        select_toolbar.append(&deselect_all_button);
        select_toolbar.set_visible(false);
        container.append(&select_toolbar);

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_min_content_height(200);

        let items_list = ListBox::new();
        items_list.add_css_class("boxed-list");
        items_list.set_selection_mode(gtk4::SelectionMode::None);
        scrolled.set_child(Some(&items_list));
        container.append(&scrolled);

        let execute_button = Button::with_label("Remove Selected Items");
        execute_button.add_css_class("destructive-action");
        execute_button.set_halign(gtk4::Align::Start);
        execute_button.set_visible(false);
        container.append(&execute_button);

        let result_label = Label::new(None);
        result_label.set_halign(gtk4::Align::Start);
        result_label.set_wrap(true);
        result_label.set_visible(false);
        container.append(&result_label);

        let check_buttons: Rc<RefCell<Vec<CheckButton>>> = Rc::new(RefCell::new(Vec::new()));
        let selections = Rc::new(RefCell::new(Vec::new()));

        let cb_for_all = check_buttons.clone();
        select_all_button.connect_clicked(move |_| {
            for btn in cb_for_all.borrow().iter() {
                btn.set_active(true);
            }
        });

        let cb_for_none = check_buttons.clone();
        deselect_all_button.connect_clicked(move |_| {
            for btn in cb_for_none.borrow().iter() {
                btn.set_active(false);
            }
        });

        Self {
            container,
            scan_button,
            package_cache_button,
            journal_vacuum_button,
            status_label,
            select_toolbar,
            items_list,
            execute_button,
            result_label,
            selections,
            check_buttons,
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

    /// Get the fixed package-cache cleanup action.
    pub fn package_cache_button(&self) -> &Button {
        &self.package_cache_button
    }

    /// Get the bounded journal-vacuum action.
    pub fn journal_vacuum_button(&self) -> &Button {
        &self.journal_vacuum_button
    }

    /// Get the execute button for signal connection.
    pub fn execute_button(&self) -> &Button {
        &self.execute_button
    }

    /// Show scan progress directly.
    pub fn show_scan_progress(&self, completed: u64, total: Option<u64>, message: &str) {
        self.scan_button.set_sensitive(false);
        let text = match total {
            Some(t) if t > 0 => format!("{message} ({completed}/{t})"),
            _ => format!("{message} ({completed} items)"),
        };
        self.status_label.set_text(&text);
        self.status_label.set_visible(true);
    }

    /// Get a clone of the status label for use in progress closures.
    pub fn status_label_handle(&self) -> Label {
        self.status_label.clone()
    }

    /// Render discovered cleanup items for explicit review.
    pub fn show_plan(&self, plan: &CleanupPlan) {
        while let Some(child) = self.items_list.first_child() {
            self.items_list.remove(&child);
        }

        *self.selections.borrow_mut() = vec![false; plan.items.len()];
        let mut buttons = Vec::with_capacity(plan.items.len());
        for (index, item) in plan.items.iter().enumerate() {
            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            row.set_margin_start(12);
            row.set_margin_end(12);

            let check = CheckButton::new();
            buttons.push(check.clone());
            let label = Label::new(Some(&item.display_path));
            label.set_halign(gtk4::Align::Start);
            label.set_hexpand(true);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);

            let size_badge = Label::new(Some(&format_bytes(item.size_bytes)));
            size_badge.set_halign(gtk4::Align::End);
            size_badge.add_css_class("maintainer_process_memory");

            let selections = self.selections.clone();
            let execute = self.execute_button.clone();
            check.connect_toggled(move |button| {
                if let Some(selected) = selections.borrow_mut().get_mut(index) {
                    *selected = button.is_active();
                }
                execute.set_sensitive(selections.borrow().iter().any(|selected| *selected));
            });

            row.append(&check);
            row.append(&label);
            row.append(&size_badge);
            self.items_list.append(&row);
        }
        *self.check_buttons.borrow_mut() = buttons;

        self.scan_button.set_label("Scan Again");
        self.scan_button.set_sensitive(true);
        self.select_toolbar.set_visible(!plan.items.is_empty());
        self.status_label.set_text(&format!(
            "Found {} items ({} reclaimable). Select items to remove.",
            plan.items.len(),
            format_bytes(plan.estimated_reclaimable_bytes)
        ));
        self.status_label.set_visible(true);
        self.execute_button.set_visible(!plan.items.is_empty());
        self.execute_button.set_sensitive(false);
        self.result_label.set_visible(false);
    }

    /// Indices selected by the user in the current immutable plan.
    pub fn selected_indices(&self) -> Vec<usize> {
        self.selections
            .borrow()
            .iter()
            .enumerate()
            .filter_map(|(index, selected)| selected.then_some(index))
            .collect()
    }

    /// Connect the reviewed cleanup action.
    pub fn connect_execute<F: Fn() + 'static>(&self, callback: F) {
        self.execute_button.connect_clicked(move |_| callback());
    }

    /// Show a cleanup scan or plan error.
    pub fn show_error(&self, message: &str) {
        self.scan_button.set_label("Scan for Reclaimable Space");
        self.scan_button.set_sensitive(true);
        self.status_label.set_text(message);
        self.status_label.set_visible(true);
        self.status_label.add_css_class("error");
        self.execute_button.set_visible(false);
    }

    /// Show cleanup execution state.
    pub fn show_executing(&self, message: &str) {
        self.scan_button.set_label("Scanning…");
        self.scan_button.set_sensitive(false);
        self.execute_button.set_sensitive(false);
        self.status_label.remove_css_class("error");
        self.status_label.set_text(message);
        self.status_label.set_visible(true);
    }

    /// Show cleanup execution result.
    pub fn show_result(&self, message: &str, success: bool) {
        self.scan_button.set_label("Scan for Reclaimable Space");
        self.scan_button.set_sensitive(true);
        self.execute_button.set_visible(false);
        self.status_label.set_visible(false);
        self.result_label.set_text(message);
        self.result_label.set_visible(true);
        if success {
            self.result_label.remove_css_class("error");
            self.result_label.add_css_class("success");
        } else {
            self.result_label.remove_css_class("success");
            self.result_label.add_css_class("error");
        }
    }

    /// Apply state to the cleanup page.
    pub fn apply_state(&self, state: &CleanupState) {
        match &state.phase {
            CleanupPhase::Idle => {
                self.scan_button.set_label("Scan for Reclaimable Space");
                self.scan_button.set_sensitive(true);
                self.status_label.set_visible(false);
                self.execute_button.set_visible(false);
                self.result_label.set_visible(false);
            }
            CleanupPhase::Scanning { message } => {
                self.scan_button.set_label("Scanning…");
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
                self.scan_button.set_label("Scan Again");
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
                self.scan_button.set_label("Cleaning…");
                self.scan_button.set_sensitive(false);
                self.status_label.set_text(message);
                self.status_label.set_visible(true);
                self.execute_button.set_sensitive(false);
                self.result_label.set_visible(false);
            }
            CleanupPhase::Completed { summary } => {
                self.scan_button.set_label("Scan for Reclaimable Space");
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
