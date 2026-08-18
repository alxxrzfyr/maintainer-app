//! Application shell: the main window layout with navigation and content area.
//!
//! The shell manages adaptive layout transitions based on window width,
//! switching between persistent sidebar, compact rail, and drawer modes.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{ApplicationWindow, Box as GtkBox, Label, Orientation, Widget};

use crate::state::{AppState, Page};
use crate::ui::navigation::{self, NavigationMode};

/// The application shell manages the window layout.
pub struct Shell {
    window: ApplicationWindow,
    _state: Rc<RefCell<AppState>>,
}

impl Shell {
    /// Create a new shell attached to the given application.
    pub fn new(app: &gtk4::Application) -> Self {
        let state = Rc::new(RefCell::new(AppState::default()));

        let window = ApplicationWindow::builder()
            .application(app)
            .title("Maintainer")
            .default_width(1100)
            .default_height(700)
            .build();

        window.add_css_class("maintainer_window");

        let root_box = GtkBox::new(Orientation::Horizontal, 0);

        // Sidebar container (rebuilt on width change)
        let sidebar_container = GtkBox::new(Orientation::Vertical, 0);
        sidebar_container.set_width_request(220);
        sidebar_container.add_css_class("maintainer_sidebar_container");

        // Content area
        let content_area = GtkBox::new(Orientation::Vertical, 0);
        content_area.set_hexpand(true);
        content_area.set_vexpand(true);
        content_area.add_css_class("maintainer_content_area");

        root_box.append(&sidebar_container);
        root_box.append(&content_area);

        window.set_child(Some(&root_box));

        // Build initial sidebar
        Self::rebuild_sidebar_widget(
            &sidebar_container,
            &content_area,
            &state,
            NavigationMode::Persistent,
        );
        Self::set_placeholder(&content_area, state.borrow().active_page);

        // Adapt layout when window width changes
        let state_for_notify = state.clone();
        let sidebar_for_notify = sidebar_container.clone();
        let content_for_notify = content_area.clone();
        window.connect_default_width_notify(move |win| {
            let width = win.width();
            if width > 0 {
                let mode = NavigationMode::from_width(width);
                let new_width = match mode {
                    NavigationMode::Persistent => 220,
                    NavigationMode::Compact => 60,
                    NavigationMode::Drawer => 0,
                };
                sidebar_for_notify.set_width_request(new_width);
                sidebar_for_notify.set_visible(mode != NavigationMode::Drawer);
                Self::rebuild_sidebar_widget(
                    &sidebar_for_notify,
                    &content_for_notify,
                    &state_for_notify,
                    mode,
                );
            }
        });

        Shell {
            window,
            _state: state,
        }
    }

    /// Show the window.
    pub fn present(&self) {
        self.window.present();
    }

    /// Get a reference to the application window.
    pub fn window(&self) -> &ApplicationWindow {
        &self.window
    }

    fn rebuild_sidebar_widget(
        sidebar_container: &GtkBox,
        content_area: &GtkBox,
        state: &Rc<RefCell<AppState>>,
        mode: NavigationMode,
    ) {
        // Remove existing sidebar children
        while let Some(child) = sidebar_container.first_child() {
            sidebar_container.remove(&child);
        }

        let active_page = state.borrow().active_page;

        let state_ref = state.clone();
        let content_ref = content_area.clone();

        let sidebar = navigation::build_sidebar(active_page, mode, move |page| {
            {
                let mut s = state_ref.borrow_mut();
                s.active_page = page;
            }
            Self::set_placeholder(&content_ref, page);
        });

        sidebar_container.append(&sidebar);
    }

    fn set_placeholder(content_area: &GtkBox, page: Page) {
        while let Some(child) = content_area.first_child() {
            content_area.remove(&child);
        }
        let placeholder = Label::new(Some(&format!("{} (coming soon)", page.label())));
        placeholder.add_css_class("maintainer_text_muted");
        placeholder.set_hexpand(true);
        placeholder.set_vexpand(true);
        content_area.append(&placeholder);
    }

    /// Set the page content widget (replaces current content).
    pub fn set_page_content(&self, _content: &impl IsA<Widget>) {
        // Will be used by page implementations in later tasks.
    }
}
