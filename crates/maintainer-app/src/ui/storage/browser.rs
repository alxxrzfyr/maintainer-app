//! Directory browser sub-component for drill-down navigation within scan
//! results.
//!
//! Provides a breadcrumb-style path display and back navigation. Used by both
//! large files and duplicates views to filter results by directory prefix.

use std::path::PathBuf;

use gtk4::prelude::*;
use gtk4::{self, Align, Box as GtkBox, Button, Label, Orientation};

/// Directory browser showing current drill-down path with navigation controls.
pub struct DirectoryBrowser {
    container: GtkBox,
    path_label: Label,
    back_button: Button,
}

impl DirectoryBrowser {
    /// Build the browser widget.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Horizontal, 8);
        container.add_css_class("directory-browser");
        container.set_margin_start(12);
        container.set_margin_end(12);

        let back_button = Button::with_label("← Back");
        back_button.set_halign(Align::Start);
        back_button.set_visible(false);

        let path_label = Label::new(Some("All results"));
        path_label.set_halign(Align::Start);
        path_label.set_hexpand(true);
        path_label.set_ellipsize(gtk4::pango::EllipsizeMode::Start);
        path_label.add_css_class("directory-browser-path");

        container.append(&back_button);
        container.append(&path_label);

        Self {
            container,
            path_label,
            back_button,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update from the current drill-down path.
    pub fn apply_path(&self, path: &Option<PathBuf>) {
        match path {
            Some(p) => {
                self.path_label.set_text(&p.to_string_lossy());
                self.back_button.set_visible(true);
            }
            None => {
                self.path_label.set_text("All results");
                self.back_button.set_visible(false);
            }
        }
    }

    /// Connect back button click.
    pub fn connect_back<F: Fn() + 'static>(&self, callback: F) {
        self.back_button.connect_clicked(move |_| callback());
    }
}

impl Default for DirectoryBrowser {
    fn default() -> Self {
        Self::new()
    }
}
