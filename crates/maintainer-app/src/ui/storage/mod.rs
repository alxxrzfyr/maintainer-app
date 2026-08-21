//! Storage analysis UI pages.
//!
//! Provides the Overview, Large Files, and Duplicates sub-pages for the
//! Storage section. All result lists use virtualized `gio::ListStore` with
//! `ListView` or `ColumnView` to handle large result sets efficiently.

pub mod browser;
pub mod duplicates;
pub mod large_files;
pub mod overview;

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Orientation, Stack, StackSwitcher};

use crate::state::{StorageState, StorageSubPage};

/// Top-level storage page containing sub-page navigation and content stack.
pub struct StoragePage {
    container: GtkBox,
    stack: Stack,
    pub overview: overview::StorageOverview,
    pub large_files: large_files::LargeFilesView,
    pub duplicates: duplicates::DuplicatesView,
}

impl StoragePage {
    /// Build the storage page widget tree.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 0);
        container.add_css_class("storage-page");

        let stack = Stack::new();
        stack.set_vexpand(true);
        stack.set_hexpand(true);

        let overview = overview::StorageOverview::new();
        stack.add_titled(overview.widget(), Some("overview"), "Overview");

        let large_files = large_files::LargeFilesView::new();
        stack.add_titled(large_files.widget(), Some("large-files"), "Large Files");

        let duplicates = duplicates::DuplicatesView::new();
        stack.add_titled(duplicates.widget(), Some("duplicates"), "Duplicates");

        let switcher = StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_halign(gtk4::Align::Center);
        switcher.set_margin_top(12);
        switcher.set_margin_bottom(12);

        container.append(&switcher);
        container.append(&stack);

        Self {
            container,
            stack,
            overview,
            large_files,
            duplicates,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update the visible sub-page based on state.
    pub fn apply_state(&self, state: &StorageState) {
        let page_name = match state.sub_page {
            StorageSubPage::Overview => "overview",
            StorageSubPage::LargeFiles => "large-files",
            StorageSubPage::Duplicates => "duplicates",
        };
        self.stack.set_visible_child_name(page_name);
    }
}

impl Default for StoragePage {
    fn default() -> Self {
        Self::new()
    }
}
