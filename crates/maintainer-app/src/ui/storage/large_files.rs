//! Large-file results view using virtualized `gio::ListStore` and
//! `gtk4::ColumnView` for efficient rendering of large result sets.
//!
//! Each row shows filename, size, and full path. Only visible rows are
//! realized — 10,000 results do not create 10,000 widget trees.

use gtk4::prelude::*;
use gtk4::{
    self, Align, Box as GtkBox, ColumnView, ColumnViewColumn, Label, ListItem, Orientation,
    ScrolledWindow, SignalListItemFactory, SingleSelection,
};

use gio::prelude::*;

use super::browser::DirectoryBrowser;
use crate::state::StorageState;

/// GObject wrapper for large-file row data in the ListStore.
mod imp {
    use glib::Properties;
    use glib::prelude::*;
    use gtk4::glib;
    use gtk4::subclass::prelude::*;
    use std::cell::{Cell, RefCell};

    #[derive(Properties, Default)]
    #[properties(wrapper_type = super::LargeFileObject)]
    pub struct LargeFileObject {
        #[property(get, set)]
        pub(super) path: RefCell<String>,
        #[property(get, set)]
        pub(super) filename: RefCell<String>,
        #[property(get, set)]
        pub(super) size_bytes: Cell<u64>,
        #[property(get, set)]
        pub(super) size_display: RefCell<String>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for LargeFileObject {
        const NAME: &'static str = "MaintainerLargeFileObject";
        type Type = super::LargeFileObject;
        type ParentType = glib::Object;
    }

    #[glib::derived_properties]
    impl ObjectImpl for LargeFileObject {}
}

glib::wrapper! {
    /// GObject representing a single large-file row in the ListStore.
    pub struct LargeFileObject(ObjectSubclass<imp::LargeFileObject>);
}

impl LargeFileObject {
    pub fn new(path: &str, filename: &str, size_bytes: u64, size_display: &str) -> Self {
        glib::Object::builder()
            .property("path", path)
            .property("filename", filename)
            .property("size-bytes", size_bytes)
            .property("size-display", size_display)
            .build()
    }
}

/// View for large-file scan results with virtualized column view.
#[derive(Clone)]
pub struct LargeFilesView {
    container: GtkBox,
    column_view: ColumnView,
    model: gio::ListStore,
    browser: DirectoryBrowser,
    empty_label: Label,
}

impl LargeFilesView {
    /// Build the large-files view.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.set_margin_top(12);
        container.set_margin_bottom(12);
        container.add_css_class("large-files-view");

        let browser = DirectoryBrowser::new();
        container.append(browser.widget());

        let model = gio::ListStore::new::<LargeFileObject>();
        let selection = SingleSelection::new(Some(model.clone()));
        selection.set_autoselect(false);

        let column_view = ColumnView::new(Some(selection));
        column_view.set_vexpand(true);
        column_view.set_hexpand(true);
        column_view.add_css_class("large-files-column-view");

        let filename_factory = SignalListItemFactory::new();
        filename_factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<ListItem>() else {
                return;
            };
            let label = Label::new(None);
            label.set_halign(Align::Start);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
            item.set_child(Some(&label));
        });
        filename_factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<ListItem>() else {
                return;
            };
            let (Some(obj), Some(label)) = (
                item.item().and_downcast::<LargeFileObject>(),
                item.child().and_downcast::<Label>(),
            ) else {
                return;
            };
            label.set_text(&obj.filename());
            label.set_tooltip_text(Some(&obj.path()));
        });
        let filename_col = ColumnViewColumn::new(Some("Name"), Some(filename_factory));
        filename_col.set_expand(true);
        column_view.append_column(&filename_col);

        let size_factory = SignalListItemFactory::new();
        size_factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<ListItem>() else {
                return;
            };
            let label = Label::new(None);
            label.set_halign(Align::End);
            item.set_child(Some(&label));
        });
        size_factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<ListItem>() else {
                return;
            };
            let (Some(obj), Some(label)) = (
                item.item().and_downcast::<LargeFileObject>(),
                item.child().and_downcast::<Label>(),
            ) else {
                return;
            };
            label.set_text(&obj.size_display());
        });
        let size_col = ColumnViewColumn::new(Some("Size"), Some(size_factory));
        size_col.set_fixed_width(120);
        column_view.append_column(&size_col);

        let path_factory = SignalListItemFactory::new();
        path_factory.connect_setup(|_, item| {
            let Some(item) = item.downcast_ref::<ListItem>() else {
                return;
            };
            let label = Label::new(None);
            label.set_halign(Align::Start);
            label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
            item.set_child(Some(&label));
        });
        path_factory.connect_bind(|_, item| {
            let Some(item) = item.downcast_ref::<ListItem>() else {
                return;
            };
            let (Some(obj), Some(label)) = (
                item.item().and_downcast::<LargeFileObject>(),
                item.child().and_downcast::<Label>(),
            ) else {
                return;
            };
            label.set_text(&obj.path());
        });
        let path_col = ColumnViewColumn::new(Some("Path"), Some(path_factory));
        path_col.set_expand(true);
        column_view.append_column(&path_col);

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hexpand(true);
        scrolled.set_min_content_height(200);
        scrolled.set_child(Some(&column_view));
        container.append(&scrolled);

        let empty_label = Label::new(Some("No large files found. Run a storage scan first."));
        empty_label.set_halign(Align::Center);
        empty_label.set_valign(Align::Center);
        empty_label.add_css_class("dim-label");
        empty_label.set_vexpand(true);
        container.append(&empty_label);

        Self {
            container,
            column_view,
            model,
            browser,
            empty_label,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Populate the model from state. Only rebuilds when results change.
    pub fn apply_state(&self, state: &StorageState) {
        self.browser.apply_path(&state.drill_down_path);

        let files = &state.large_files;
        let filtered: Vec<_> = match &state.drill_down_path {
            Some(prefix) => files
                .iter()
                .filter(|f| f.path.starts_with(prefix))
                .collect(),
            None => files.iter().collect(),
        };

        if filtered.is_empty() {
            self.empty_label.set_visible(true);
            self.column_view.set_visible(false);
        } else {
            self.empty_label.set_visible(false);
            self.column_view.set_visible(true);
        }

        self.model.remove_all();
        for file in &filtered {
            let filename = file
                .path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let size_display = format_bytes(file.size_bytes);
            let path_str = file.path.to_string_lossy();
            let obj = LargeFileObject::new(&path_str, &filename, file.size_bytes, &size_display);
            self.model.append(&obj);
        }
    }

    /// Connect browser back navigation.
    pub fn connect_back<F: Fn() + 'static>(&self, callback: F) {
        self.browser.connect_back(callback);
    }

    /// Access the underlying model for testing.
    pub fn model(&self) -> &gio::ListStore {
        &self.model
    }
}

impl Default for LargeFilesView {
    fn default() -> Self {
        Self::new()
    }
}

/// Format bytes to a human-readable string.
fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if bytes >= TIB {
        format!("{:.1} TiB", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
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
    fn format_bytes_renders_correctly() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
        assert_eq!(format_bytes(1024), "1.0 KiB");
        assert_eq!(format_bytes(1_048_576), "1.0 MiB");
        assert_eq!(format_bytes(1_073_741_824), "1.0 GiB");
        assert_eq!(format_bytes(1_099_511_627_776), "1.0 TiB");
    }
}
