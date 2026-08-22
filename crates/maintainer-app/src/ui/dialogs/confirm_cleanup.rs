//! Cleanup confirmation dialog.
//!
//! Shows exact targets, total reclaimable bytes, and requires explicit
//! confirmation. Permanent deletion uses stronger copy and no preselected
//! confirmation state.

use gtk4::prelude::*;
use gtk4::{
    self, Box as GtkBox, Button, CheckButton, Label, ListBox, ListBoxRow, Orientation,
    ScrolledWindow,
};

/// User response from the confirmation dialog.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmResponse {
    /// User confirmed the cleanup.
    Confirmed,
    /// User cancelled.
    Cancelled,
}

/// Information about an item to display in the confirmation dialog.
#[derive(Debug, Clone)]
pub struct ConfirmItem {
    /// Display path.
    pub display_path: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Whether this is permanent (vs Trash).
    pub is_permanent: bool,
}

/// Build the cleanup confirmation dialog content.
///
/// This creates the widget tree for the confirmation dialog. The actual
/// dialog window management is handled by the caller (typically using
/// a GTK4 `Window` or `AlertDialog`).
pub struct ConfirmCleanupDialog {
    container: GtkBox,
    confirm_button: Button,
    cancel_button: Button,
    permanent_check: Option<CheckButton>,
}

impl ConfirmCleanupDialog {
    /// Build confirmation dialog content for the given items.
    ///
    /// `has_permanent` indicates whether any items require permanent deletion.
    /// When true, a stronger warning and explicit checkbox are shown.
    pub fn new(items: &[ConfirmItem], total_bytes: u64, has_permanent: bool) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 12);
        container.set_margin_top(24);
        container.set_margin_bottom(24);
        container.set_margin_start(24);
        container.set_margin_end(24);

        let title = if has_permanent {
            Label::new(Some("Permanently Remove Files"))
        } else {
            Label::new(Some("Move Files to Trash"))
        };
        title.add_css_class("title-2");
        title.set_halign(gtk4::Align::Start);
        container.append(&title);

        if has_permanent {
            let warning = Label::new(Some(
                "⚠️ The following items will be permanently deleted. \
                 This action cannot be undone.",
            ));
            warning.set_wrap(true);
            warning.add_css_class("error");
            warning.set_halign(gtk4::Align::Start);
            container.append(&warning);
        }

        let summary = Label::new(Some(&format!(
            "{} items • {} will be freed",
            items.len(),
            format_bytes_short(total_bytes),
        )));
        summary.set_halign(gtk4::Align::Start);
        summary.add_css_class("dim-label");
        container.append(&summary);

        let scrolled = ScrolledWindow::new();
        scrolled.set_max_content_height(300);
        scrolled.set_propagate_natural_height(true);

        let list = ListBox::new();
        list.set_selection_mode(gtk4::SelectionMode::None);
        list.add_css_class("boxed-list");

        for item in items {
            let row = ListBoxRow::new();
            let row_box = GtkBox::new(Orientation::Horizontal, 8);
            row_box.set_margin_top(4);
            row_box.set_margin_bottom(4);
            row_box.set_margin_start(8);
            row_box.set_margin_end(8);

            let path_label = Label::new(Some(&item.display_path));
            path_label.set_hexpand(true);
            path_label.set_halign(gtk4::Align::Start);
            path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            row_box.append(&path_label);

            let size_label = Label::new(Some(&format_bytes_short(item.size_bytes)));
            size_label.add_css_class("dim-label");
            row_box.append(&size_label);

            if item.is_permanent {
                let perm_badge = Label::new(Some("permanent"));
                perm_badge.add_css_class("error");
                row_box.append(&perm_badge);
            }

            row.set_child(Some(&row_box));
            list.append(&row);
        }

        scrolled.set_child(Some(&list));
        container.append(&scrolled);

        let permanent_check = if has_permanent {
            let check = CheckButton::with_label(
                "I understand these files will be permanently deleted and cannot be recovered",
            );
            check.set_active(false); // Never preselected.
            container.append(&check);
            Some(check)
        } else {
            None
        };

        let button_box = GtkBox::new(Orientation::Horizontal, 8);
        button_box.set_halign(gtk4::Align::End);
        button_box.set_margin_top(12);

        let cancel_button = Button::with_label("Cancel");
        button_box.append(&cancel_button);

        let confirm_label = if has_permanent {
            "Permanently Delete"
        } else {
            "Move to Trash"
        };
        let confirm_button = Button::with_label(confirm_label);
        confirm_button.add_css_class("destructive-action");

        if has_permanent {
            confirm_button.set_sensitive(false);
        }

        button_box.append(&confirm_button);
        container.append(&button_box);

        if let Some(ref check) = permanent_check {
            let btn = confirm_button.clone();
            check.connect_toggled(move |c| {
                btn.set_sensitive(c.is_active());
            });
        }

        Self {
            container,
            confirm_button,
            cancel_button,
            permanent_check,
        }
    }

    /// Get the container widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Get the confirm button for signal connection.
    pub fn confirm_button(&self) -> &Button {
        &self.confirm_button
    }

    /// Get the cancel button for signal connection.
    pub fn cancel_button(&self) -> &Button {
        &self.cancel_button
    }

    /// Whether the permanent deletion checkbox is checked (if present).
    pub fn is_permanent_confirmed(&self) -> bool {
        self.permanent_check
            .as_ref()
            .map(|c| c.is_active())
            .unwrap_or(true) // If no checkbox, considered confirmed.
    }
}

/// Format bytes in a compact format.
fn format_bytes_short(bytes: u64) -> String {
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
    fn format_bytes_short_values() {
        assert_eq!(format_bytes_short(0), "0 B");
        assert_eq!(format_bytes_short(1023), "1023 B");
        assert_eq!(format_bytes_short(1024), "1.0 KiB");
        assert_eq!(format_bytes_short(1024 * 1024), "1.0 MiB");
    }

    #[test]
    fn confirm_item_permanent_flag() {
        let item = ConfirmItem {
            display_path: "~/test.txt".into(),
            size_bytes: 100,
            is_permanent: true,
        };
        assert!(item.is_permanent);
    }

    #[test]
    fn confirm_response_variants() {
        assert_ne!(ConfirmResponse::Confirmed, ConfirmResponse::Cancelled);
    }
}
