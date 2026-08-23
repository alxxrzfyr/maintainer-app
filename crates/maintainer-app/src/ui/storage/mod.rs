//! Storage analysis UI pages.
//!
//! Provides the Overview, Large Files, and Duplicates sub-pages for the
//! Storage section. All result lists use virtualized `gio::ListStore` with
//! `ListView` or `ColumnView` to handle large result sets efficiently.

pub mod browser;
pub mod duplicates;
pub mod large_files;
pub mod overview;

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Orientation, Stack, StackSwitcher};

use crate::state::{StorageState, StorageSubPage};

/// Top-level storage page containing sub-page navigation and content stack.
#[derive(Clone)]
pub struct StoragePage {
    container: GtkBox,
    stack: Stack,
    pub overview: overview::StorageOverview,
    pub large_files: large_files::LargeFilesView,
    pub duplicates: duplicates::DuplicatesView,
    /// Set while `apply_state` is driving the stack's visible child
    /// programmatically. `Stack::set_visible_child_name` fires the
    /// `notify::visible-child-name` signal synchronously, and the shell's
    /// handler for that signal re-borrows the same shared `StorageState`
    /// `RefCell` that a caller of `apply_state` may still be holding
    /// borrowed (e.g. `apply_state(&state.borrow())`), which panics with
    /// `BorrowMutError` and aborts, since the panic crosses a GTK C signal
    /// boundary that cannot unwind. This flag lets that handler recognize
    /// a programmatic transition and skip re-deriving state from it.
    pub programmatic_navigation: Rc<Cell<bool>>,
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
        switcher.add_css_class("maintainer_stack_switcher");

        container.append(&switcher);
        container.append(&stack);

        Self {
            container,
            stack,
            overview,
            large_files,
            duplicates,
            programmatic_navigation: Rc::new(Cell::new(false)),
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Get the storage stack widget.
    pub fn stack_handle(&self) -> &Stack {
        &self.stack
    }

    /// Update all storage sub-pages from the current state.
    pub fn apply_state(&self, state: &StorageState) {
        let page_name = match state.sub_page {
            StorageSubPage::Overview => "overview",
            StorageSubPage::LargeFiles => "large-files",
            StorageSubPage::Duplicates => "duplicates",
        };
        // Guard the transition: see `programmatic_navigation`'s doc comment.
        // Always cleared before returning, including if a future change adds
        // an early return above, so a stuck `true` cannot permanently disable
        // the shell's user-driven sub-page tracking.
        self.programmatic_navigation.set(true);
        self.stack.set_visible_child_name(page_name);
        self.programmatic_navigation.set(false);
        self.overview.apply_state(state);
        self.large_files.apply_state(state);
        self.duplicates.apply_state(state);
    }
}

impl Default for StoragePage {
    fn default() -> Self {
        Self::new()
    }
}
