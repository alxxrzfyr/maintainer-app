//! Duplicate-file results view.
//!
//! Groups are shown in a virtualized `ListView` with per-file checkboxes.
//! Selections default empty — the user must explicitly check each file
//! to include in the removal plan. Full paths are always visible.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, Button, CheckButton, Label, ListItem, ListView, Orientation,
    ScrolledWindow, SignalListItemFactory, SingleSelection,
};

use gio::prelude::*;

use crate::state::{RemovalPlanRequest, StorageState};

type SelectionCallback = Rc<RefCell<Option<Box<dyn Fn(usize, usize)>>>>;

/// GObject for a row in the duplicates list (one file within a group).
mod imp {
    use glib::Properties;
    use glib::prelude::*;
    use gtk4::glib;
    use gtk4::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::DuplicateRowObject)]
    pub struct DuplicateRowObject {
        #[property(get, set)]
        pub(super) path: RefCell<String>,
        #[property(get, set)]
        pub(super) group_idx: Cell<u32>,
        #[property(get, set)]
        pub(super) file_idx: Cell<u32>,
        #[property(get, set)]
        pub(super) size_display: RefCell<String>,
        #[property(get, set)]
        pub(super) digest_short: RefCell<String>,
        #[property(get, set)]
        pub(super) selected: Cell<bool>,
        #[property(get, set)]
        pub(super) is_group_header: Cell<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for DuplicateRowObject {
        const NAME: &'static str = "MaintainerDuplicateRowObject";
        type Type = super::DuplicateRowObject;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for DuplicateRowObject {}
}

glib::wrapper! {
    /// GObject for a duplicate file row in the list store.
    pub struct DuplicateRowObject(ObjectSubclass<imp::DuplicateRowObject>);
}

impl DuplicateRowObject {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        path: &str,
        group_idx: u32,
        file_idx: u32,
        size_display: &str,
        digest_short: &str,
        selected: bool,
        is_group_header: bool,
    ) -> Self {
        glib::Object::builder()
            .property("path", path)
            .property("group-idx", group_idx)
            .property("file-idx", file_idx)
            .property("size-display", size_display)
            .property("digest-short", digest_short)
            .property("selected", selected)
            .property("is-group-header", is_group_header)
            .build()
    }
}

/// View for duplicate file results.
#[derive(Clone)]
pub struct DuplicatesView {
    container: GtkBox,
    list_view: ListView,
    model: gio::ListStore,
    empty_label: Label,
    coverage_label: Label,
    review_button: Button,
    selection_count_label: Label,
    review_status: Label,
    on_selection_changed: SelectionCallback,
}

impl DuplicatesView {
    /// Build the duplicates view.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.set_margin_top(12);
        container.set_margin_bottom(12);
        container.add_css_class("duplicates-view");

        let header = GtkBox::new(Orientation::Horizontal, 12);
        header.set_margin_start(12);
        header.set_margin_end(12);

        let selection_count_label = Label::new(Some("No files selected for removal"));
        selection_count_label.set_halign(Align::Start);
        selection_count_label.set_hexpand(true);
        selection_count_label.add_css_class("dim-label");

        let review_button = Button::with_label("Review Removal Plan");
        review_button.set_sensitive(false);
        review_button.add_css_class("suggested-action");
        review_button.set_halign(Align::End);

        header.append(&selection_count_label);
        header.append(&review_button);
        container.append(&header);

        let review_status = Label::new(None);
        review_status.set_halign(Align::Start);
        review_status.set_margin_start(12);
        review_status.set_margin_end(12);
        review_status.set_wrap(true);
        review_status.set_visible(false);
        review_status.add_css_class("maintainer_text_muted");
        container.append(&review_status);

        let coverage_label = Label::new(None);
        coverage_label.set_wrap(true);
        coverage_label.set_halign(Align::Start);
        coverage_label.set_visible(false);
        container.append(&coverage_label);

        let model = gio::ListStore::new::<DuplicateRowObject>();
        let selection = SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);

        let on_selection_changed: SelectionCallback = Rc::new(RefCell::new(None));
        let on_selection_for_setup = on_selection_changed.clone();
        let factory = SignalListItemFactory::new();
        factory.connect_setup(move |_, item| {
            let item = item.downcast_ref::<ListItem>().unwrap();
            let row = GtkBox::new(Orientation::Horizontal, 8);
            row.set_margin_start(12);
            row.set_margin_end(12);
            row.set_margin_top(4);
            row.set_margin_bottom(4);

            let check = CheckButton::new();
            check.set_halign(Align::Start);
            let item_weak = item.downgrade();
            let on_selection = on_selection_for_setup.clone();
            check.connect_toggled(move |check| {
                let Some(item) = item_weak.upgrade() else {
                    return;
                };
                let Some(obj) = item.item().and_downcast::<DuplicateRowObject>() else {
                    return;
                };
                if obj.is_group_header() || check.is_active() == obj.selected() {
                    return;
                }
                obj.set_selected(check.is_active());
                if let Some(callback) = on_selection.borrow().as_ref() {
                    callback(obj.group_idx() as usize, obj.file_idx() as usize);
                }
            });

            let path_label = Label::new(None);
            path_label.set_halign(Align::Start);
            path_label.set_hexpand(true);
            path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);

            let size_label = Label::new(None);
            size_label.set_halign(Align::End);
            size_label.add_css_class("dim-label");

            row.append(&check);
            row.append(&path_label);
            row.append(&size_label);
            item.set_child(Some(&row));
        });
        factory.connect_bind(|_, item| {
            let item = item.downcast_ref::<ListItem>().unwrap();
            let obj = item.item().and_downcast::<DuplicateRowObject>().unwrap();
            let row = item.child().and_downcast::<GtkBox>().unwrap();

            let check = row.first_child().and_downcast::<CheckButton>().unwrap();
            let path_label = check.next_sibling().and_downcast::<Label>().unwrap();
            let size_label = path_label.next_sibling().and_downcast::<Label>().unwrap();

            if obj.is_group_header() {
                check.set_visible(false);
                row.add_css_class("duplicates-group-header");
                path_label.set_text(&format!(
                    "{} — {} each",
                    obj.digest_short(),
                    obj.size_display()
                ));
                path_label.remove_css_class("heading");
                size_label.set_text("Oldest copy kept by default — review before confirming");
                size_label.remove_css_class("dim-label");
                size_label.add_css_class("duplicates-keep-hint");
            } else {
                row.remove_css_class("duplicates-group-header");
                check.set_visible(true);
                check.set_active(obj.selected());
                check.update_property(&[gtk4::accessible::Property::Label(&format!(
                    "Select {}",
                    obj.path()
                ))]);
                path_label.set_text(&obj.path());
                path_label.remove_css_class("heading");
                size_label.set_text(&obj.size_display());
                size_label.remove_css_class("duplicates-keep-hint");
                size_label.add_css_class("dim-label");
            }
        });

        let list_view = ListView::new(Some(selection), Some(factory));
        list_view.set_vexpand(true);
        list_view.set_hexpand(true);
        list_view.add_css_class("duplicates-list");

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        scrolled.set_min_content_height(200);
        scrolled.set_child(Some(&list_view));
        container.append(&scrolled);

        let empty_label = Label::new(Some("No duplicates found. Run a duplicate scan first."));
        empty_label.set_halign(Align::Center);
        empty_label.set_valign(Align::Center);
        empty_label.add_css_class("dim-label");
        empty_label.set_vexpand(true);
        container.append(&empty_label);

        Self {
            container,
            list_view,
            model,
            empty_label,
            coverage_label,
            review_button,
            selection_count_label,
            review_status,
            on_selection_changed,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update from current state. Rebuilds the flat list model from groups.
    pub fn apply_state(&self, state: &StorageState) {
        let groups = &state.duplicate_groups;
        let incomplete = !state.duplicate_issues.is_empty();
        self.coverage_label.set_visible(incomplete);
        if incomplete {
            self.coverage_label.set_text(&format!(
                "Scan coverage is incomplete ({} issues). Results cover successfully inspected files only.",
                state.duplicate_issues.len()
            ));
        }
        self.empty_label.set_text(if incomplete {
            "No duplicates confirmed in inspected files. Scan coverage is incomplete."
        } else {
            "No duplicates found. Run a duplicate scan first."
        });

        if groups.is_empty() {
            self.empty_label.set_visible(true);
            self.list_view.set_visible(false);
        } else {
            self.empty_label.set_visible(false);
            self.list_view.set_visible(true);
        }

        self.model.remove_all();
        for (group_idx, group) in groups.iter().enumerate() {
            let digest_short = group.digest.to_hex();
            let digest_display = if digest_short.len() > 12 {
                &digest_short[..12]
            } else {
                &digest_short
            };
            let size_display = format_bytes(group.size_bytes);

            let header = DuplicateRowObject::new(
                "",
                group_idx as u32,
                0,
                &size_display,
                digest_display,
                false,
                true,
            );
            self.model.append(&header);

            for (file_idx, file) in group.files.iter().enumerate() {
                let is_selected = state.duplicate_selections.contains(&(group_idx, file_idx));
                let path_str = file.path.to_string_lossy();
                let row = DuplicateRowObject::new(
                    &path_str,
                    group_idx as u32,
                    file_idx as u32,
                    &size_display,
                    digest_display,
                    is_selected,
                    false,
                );
                self.model.append(&row);
            }
        }

        let count = state.duplicate_selections.len();
        if count == 0 {
            self.selection_count_label
                .set_text("No files selected for removal");
            self.review_button.set_sensitive(false);
        } else {
            self.selection_count_label
                .set_text(&format!("{count} file(s) selected for removal"));
            self.review_button.set_sensitive(true);
        }
    }

    /// Connect duplicate selection changes.
    pub fn connect_selection_changed<F: Fn(usize, usize) + 'static>(&self, callback: F) {
        *self.on_selection_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Connect the review button.
    pub fn connect_review<F: Fn() + 'static>(&self, callback: F) {
        self.review_button.connect_clicked(move |_| callback());
    }

    /// Show the immutable removal plan without deleting anything automatically.
    pub fn show_review_plan(&self, review: &RemovalPlanRequest) {
        self.review_status.set_text(&format!(
            "Review ready: {} file(s), {} total. Duplicate files are never removed automatically.",
            review.targets.len(),
            format_bytes(review.total_bytes)
        ));
        self.review_status.set_visible(true);
    }

    /// Access the underlying model for testing.
    pub fn model(&self) -> &gio::ListStore {
        &self.model
    }
}

impl Default for DuplicatesView {
    fn default() -> Self {
        Self::new()
    }
}

/// Format bytes to human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;

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
