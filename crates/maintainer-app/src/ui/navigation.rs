//! Navigation sidebar for the application shell.
//!
//! Builds a grouped sidebar with page buttons. Supports three modes:
//! - Persistent labeled sidebar (≥900 px)
//! - Compact icon rail (640–899 px)
//! - Drawer/switcher (below 640 px)

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Image, Label, Orientation, ScrolledWindow, Separator};

use crate::state::{FOOTER_PAGES, NAVIGATION_GROUPS, Page};

/// Layout mode for the navigation sidebar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationMode {
    /// Full sidebar with labels (≥900 px).
    Persistent,
    /// Compact icon rail (640–899 px).
    Compact,
    /// Drawer or switcher (below 640 px).
    Drawer,
}

impl NavigationMode {
    /// Determine the navigation mode from available width in pixels.
    pub fn from_width(width: i32) -> Self {
        if width >= 900 {
            NavigationMode::Persistent
        } else if width >= 640 {
            NavigationMode::Compact
        } else {
            NavigationMode::Drawer
        }
    }
}

/// Builds the navigation sidebar widget.
///
/// The `on_navigate` callback is invoked with the target `Page` when
/// a navigation button is clicked.
pub fn build_sidebar(
    active_page: Page,
    mode: NavigationMode,
    on_navigate: impl Fn(Page) + Clone + 'static,
) -> GtkBox {
    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.add_css_class("maintainer_sidebar");

    match mode {
        NavigationMode::Persistent => sidebar.add_css_class("maintainer_sidebar_persistent"),
        NavigationMode::Compact => sidebar.add_css_class("maintainer_sidebar_compact"),
        NavigationMode::Drawer => sidebar.add_css_class("maintainer_sidebar_drawer"),
    }

    // Scrollable content area
    let scroll = ScrolledWindow::new();
    scroll.set_vexpand(true);
    scroll.set_hscrollbar_policy(gtk4::PolicyType::Never);

    let content = GtkBox::new(Orientation::Vertical, 4);
    content.set_margin_top(8);
    content.set_margin_bottom(8);
    content.set_margin_start(8);
    content.set_margin_end(8);

    // Navigation groups
    for group in NAVIGATION_GROUPS {
        // Group label (only in persistent mode)
        if mode == NavigationMode::Persistent {
            let group_label = Label::new(Some(group.label));
            group_label.add_css_class("maintainer_nav_group_label");
            group_label.add_css_class("maintainer_text_muted");
            group_label.set_halign(Align::Start);
            group_label.set_margin_top(12);
            group_label.set_margin_bottom(4);
            group_label.set_margin_start(8);
            content.append(&group_label);
        }

        for &page in group.pages {
            let btn = build_nav_button(page, active_page, mode, on_navigate.clone());
            content.append(&btn);
        }
    }

    // Separator before footer
    let sep = Separator::new(Orientation::Horizontal);
    sep.set_margin_top(8);
    sep.set_margin_bottom(8);
    content.append(&sep);

    // Footer pages
    for &page in FOOTER_PAGES {
        let btn = build_nav_button(page, active_page, mode, on_navigate.clone());
        content.append(&btn);
    }

    scroll.set_child(Some(&content));
    sidebar.append(&scroll);

    sidebar
}

/// Build a single navigation button for a page.
fn build_nav_button(
    page: Page,
    active_page: Page,
    mode: NavigationMode,
    on_navigate: impl Fn(Page) + 'static,
) -> Button {
    let btn = Button::new();
    btn.add_css_class("maintainer_nav_button");
    btn.set_can_focus(true);

    let btn_content = GtkBox::new(Orientation::Horizontal, 8);
    btn_content.set_halign(Align::Start);

    let icon = Image::from_icon_name(page.icon_name());
    icon.set_pixel_size(20);
    btn_content.append(&icon);

    // Show label in persistent and drawer modes
    if mode != NavigationMode::Compact {
        let label = Label::new(Some(page.label()));
        btn_content.append(&label);
    }

    btn.set_child(Some(&btn_content));

    // Active state
    if page == active_page {
        btn.add_css_class("maintainer_nav_active");
    }

    // Tooltip always set (required when compact or drawer)
    btn.set_tooltip_text(Some(page.label()));

    // Accessible name
    btn.update_property(&[gtk4::accessible::Property::Label(page.label())]);

    btn.connect_clicked(move |_| {
        on_navigate(page);
    });

    btn
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn navigation_mode_from_width() {
        assert_eq!(NavigationMode::from_width(1200), NavigationMode::Persistent);
        assert_eq!(NavigationMode::from_width(900), NavigationMode::Persistent);
        assert_eq!(NavigationMode::from_width(899), NavigationMode::Compact);
        assert_eq!(NavigationMode::from_width(640), NavigationMode::Compact);
        assert_eq!(NavigationMode::from_width(639), NavigationMode::Drawer);
        assert_eq!(NavigationMode::from_width(320), NavigationMode::Drawer);
    }

    #[test]
    fn drawer_mode_triggers_below_640() {
        // Boundary values for drawer mode
        assert_eq!(NavigationMode::from_width(639), NavigationMode::Drawer);
        assert_eq!(NavigationMode::from_width(1), NavigationMode::Drawer);
        // 640 is compact, not drawer
        assert_ne!(NavigationMode::from_width(640), NavigationMode::Drawer);
    }
}
