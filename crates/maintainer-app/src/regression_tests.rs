//! Regression coverage for shell navigation and responsive behavior.

use std::cell::Cell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Button, Widget};

use crate::state::Page;
use crate::ui::navigation::{NavigationMode, build_sidebar};

fn descendants_of_type<T: IsA<Widget> + Clone + 'static>(root: &impl IsA<Widget>) -> Vec<T> {
    fn visit<T: IsA<Widget> + Clone + 'static>(widget: &Widget, found: &mut Vec<T>) {
        if let Ok(item) = widget.clone().downcast::<T>() {
            found.push(item);
        }
        let mut child = widget.first_child();
        while let Some(current) = child {
            visit::<T>(&current, found);
            child = current.next_sibling();
        }
    }

    let mut found = Vec::new();
    visit::<T>(root.as_ref(), &mut found);
    found
}

#[test]
fn sidebar_is_fixed_pinned_and_updates_active_destination() {
    if gtk4::init().is_err() {
        return;
    }

    let selected = Rc::new(Cell::new(Page::Overview));
    let selected_for_callback = selected.clone();
    let sidebar = build_sidebar(Page::Overview, NavigationMode::Persistent, move |page| {
        selected_for_callback.set(page);
    });

    assert!(
        descendants_of_type::<gtk4::ScrolledWindow>(&sidebar).is_empty(),
        "sidebar navigation must remain fixed"
    );

    let footer = sidebar
        .last_child()
        .expect("sidebar must have a dedicated footer region");
    assert!(footer.has_css_class("maintainer_sidebar_footer"));
    let footer_buttons = descendants_of_type::<Button>(&footer);
    assert_eq!(footer_buttons.len(), 1);
    assert_eq!(
        footer_buttons[0].tooltip_text().as_deref(),
        Some(Page::Settings.label())
    );

    let buttons = descendants_of_type::<Button>(&sidebar);
    assert_eq!(buttons.len(), Page::ALL.len());
    for (button, page) in buttons.iter().zip(Page::ALL.iter().copied()) {
        button.emit_clicked();
        assert_eq!(selected.get(), page);
        for (candidate, candidate_page) in buttons.iter().zip(Page::ALL.iter().copied()) {
            assert_eq!(
                candidate.has_css_class("maintainer_nav_active"),
                candidate_page == page,
                "active styling did not move to {}",
                page.label()
            );
        }
    }

    crate::application::apply_theme(crate::state::Theme::Dark);
    let settings = gtk4::Settings::default().expect("GTK settings must exist");
    assert!(
        settings.is_gtk_application_prefer_dark_theme(),
        "dark mode must theme native GTK controls, not only custom surfaces"
    );
    crate::application::apply_theme(crate::state::Theme::Light);
    assert!(!settings.is_gtk_application_prefer_dark_theme());
    crate::application::apply_theme(crate::state::Theme::System);
}

#[test]
fn shell_chrome_and_cleanup_layout_avoid_screenshot_regressions() {
    let shell = include_str!("ui/shell.rs");
    let cleanup = include_str!("ui/cleanup.rs");
    let light_css = include_str!("../../../data/styles/light.css");
    let dark_css = include_str!("../../../data/styles/dark.css");

    assert!(
        !shell.contains("HeaderBar"),
        "an in-content HeaderBar duplicates compositor window controls"
    );
    assert!(
        !cleanup.contains("FlowBox"),
        "cleanup actions must not reflow beneath the expanding results pane"
    );
    for css in [light_css, dark_css] {
        assert!(
            !css.contains("box-shadow: inset 3px 0 @maintainer_accent"),
            "active navigation must use a quiet selected surface without an accent stripe"
        );
    }
}

#[test]
fn shell_responds_to_live_allocated_width_not_default_width() {
    let source = include_str!("ui/shell.rs");
    assert!(source.contains("connect_width_notify"));
    assert!(!source.contains("connect_default_width_notify"));
    assert!(
        source.contains("crate::application::apply_theme(theme)"),
        "Settings theme changes must apply immediately, not only persist"
    );
}
