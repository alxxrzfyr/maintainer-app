//! Navigation sidebar for the application shell.
//!
//! Builds a grouped sidebar with page buttons. Supports three modes:
//! - Persistent labeled sidebar (≥900 px)
//! - Compact icon rail (640–899 px)
//! - Drawer/switcher (below 640 px)

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Label, Orientation, PolicyType, ScrolledWindow, Separator, Widget,
};

use crate::state::{FOOTER_PAGES, NAVIGATION_GROUPS, Page};
use crate::ui::icons;

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

/// Apply the selected style to every navigation button below `root`.
pub fn update_active_buttons(root: &impl IsA<Widget>, active_page: Page) {
    fn visit(widget: &Widget, active_page: Page) {
        if let Ok(button) = widget.clone().downcast::<Button>() {
            let is_navigation_button = button.has_css_class("maintainer_nav_button");
            let is_active = button.tooltip_text().as_deref() == Some(active_page.label());
            if is_navigation_button && is_active {
                button.add_css_class("maintainer_nav_active");
            } else if is_navigation_button {
                button.remove_css_class("maintainer_nav_active");
            }
        }

        let mut child = widget.first_child();
        while let Some(current) = child {
            visit(&current, active_page);
            child = current.next_sibling();
        }
    }

    visit(root.as_ref(), active_page);
}

/// Build the fixed navigation sidebar.
pub fn build_sidebar(
    active_page: Page,
    mode: NavigationMode,
    on_navigate: impl Fn(Page) + Clone + 'static,
) -> GtkBox {
    let sidebar = GtkBox::new(Orientation::Vertical, 0);
    sidebar.add_css_class("maintainer_sidebar");
    sidebar.set_vexpand(true);

    match mode {
        NavigationMode::Persistent => sidebar.add_css_class("maintainer_sidebar_persistent"),
        NavigationMode::Compact => sidebar.add_css_class("maintainer_sidebar_compact"),
        NavigationMode::Drawer => sidebar.add_css_class("maintainer_sidebar_drawer"),
    }

    let groups = GtkBox::new(Orientation::Vertical, 4);
    groups.add_css_class("maintainer_sidebar_groups");
    groups.set_vexpand(true);
    groups.set_margin_top(8);
    groups.set_margin_start(8);
    groups.set_margin_end(8);

    let mut buttons = Vec::with_capacity(Page::ALL.len());
    for group in NAVIGATION_GROUPS {
        if mode == NavigationMode::Persistent {
            let group_label = Label::new(Some(group.label));
            group_label.add_css_class("maintainer_nav_group_label");
            group_label.add_css_class("maintainer_text_muted");
            group_label.set_halign(Align::Start);
            group_label.set_margin_top(8);
            group_label.set_margin_bottom(2);
            group_label.set_margin_start(8);
            groups.append(&group_label);
        }

        for &page in group.pages {
            let button = build_nav_button(page, active_page, mode);
            groups.append(&button);
            buttons.push((page, button));
        }
    }
    // Keep the shell and footer fixed while tall navigation remains reachable.
    let navigation_scroll = ScrolledWindow::new();
    navigation_scroll.set_policy(PolicyType::Never, PolicyType::Automatic);
    navigation_scroll.set_min_content_height(0);
    navigation_scroll.set_vexpand(true);
    navigation_scroll.set_child(Some(&groups));
    sidebar.append(&navigation_scroll);

    let footer = GtkBox::new(Orientation::Vertical, 4);
    footer.add_css_class("maintainer_sidebar_footer");
    footer.set_margin_top(8);
    footer.set_margin_bottom(8);
    footer.set_margin_start(8);
    footer.set_margin_end(8);

    let separator = Separator::new(Orientation::Horizontal);
    separator.set_margin_bottom(8);
    footer.append(&separator);

    for &page in FOOTER_PAGES {
        let button = build_nav_button(page, active_page, mode);
        footer.append(&button);
        buttons.push((page, button));
    }
    sidebar.append(&footer);

    for (page, button) in buttons {
        let sidebar_ref = sidebar.downgrade();
        let navigate = on_navigate.clone();
        button.connect_clicked(move |_| {
            let Some(sidebar) = sidebar_ref.upgrade() else {
                return;
            };
            update_active_buttons(&sidebar, page);
            navigate(page);
        });
    }

    sidebar
}

fn build_nav_button(page: Page, active_page: Page, mode: NavigationMode) -> Button {
    let button = Button::new();
    button.add_css_class("maintainer_nav_button");
    button.set_can_focus(true);

    let content = GtkBox::new(Orientation::Horizontal, 8);
    content.set_halign(if mode == NavigationMode::Compact {
        Align::Center
    } else {
        Align::Start
    });

    let icon_size = if mode == NavigationMode::Compact {
        20
    } else {
        16
    };
    let icon = icons::icon_with_fallback(page.icon_chain(), icon_size);
    content.append(&icon);

    if mode != NavigationMode::Compact {
        content.append(&Label::new(Some(page.label())));
    }

    button.set_child(Some(&content));
    if page == active_page {
        button.add_css_class("maintainer_nav_active");
    }
    button.set_tooltip_text(Some(page.label()));
    button.update_property(&[gtk4::accessible::Property::Label(page.label())]);
    button
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
        assert_eq!(NavigationMode::from_width(639), NavigationMode::Drawer);
        assert_eq!(NavigationMode::from_width(1), NavigationMode::Drawer);
        assert_ne!(NavigationMode::from_width(640), NavigationMode::Drawer);
    }
}
