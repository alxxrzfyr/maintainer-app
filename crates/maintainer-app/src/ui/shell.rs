//! Application shell: the main window layout with navigation and content area.
//!
//! The shell manages adaptive layout transitions based on window width,
//! switching between persistent sidebar, compact rail, and drawer modes.
//! At narrow widths (<640px), a hamburger menu in the header provides
//! page navigation when the sidebar is hidden.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, HeaderBar, Image, Label, MenuButton,
    Orientation, Popover, Widget,
};

use crate::state::{AppState, FOOTER_PAGES, NAVIGATION_GROUPS, Page};
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

        // Content area (created early so we can close over it)
        let content_area = GtkBox::new(Orientation::Vertical, 0);
        content_area.set_hexpand(true);
        content_area.set_vexpand(true);
        content_area.add_css_class("maintainer_content_area");

        // Header bar with hamburger menu for narrow widths
        let header = HeaderBar::new();
        header.add_css_class("maintainer_header");

        let menu_button = Self::build_drawer_menu_button(&state, &content_area);
        menu_button.set_visible(false);
        header.pack_start(&menu_button);

        // Main layout
        let outer_box = GtkBox::new(Orientation::Vertical, 0);
        outer_box.append(&header);

        let root_box = GtkBox::new(Orientation::Horizontal, 0);
        root_box.set_vexpand(true);

        // Sidebar container (rebuilt on width change)
        let sidebar_container = GtkBox::new(Orientation::Vertical, 0);
        sidebar_container.set_width_request(220);
        sidebar_container.add_css_class("maintainer_sidebar_container");

        root_box.append(&sidebar_container);
        root_box.append(&content_area);
        outer_box.append(&root_box);

        window.set_child(Some(&outer_box));

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
        let menu_button_for_notify = menu_button.clone();
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
                menu_button_for_notify.set_visible(mode == NavigationMode::Drawer);
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

    /// Build the hamburger menu button with a popover containing page navigation.
    ///
    /// This provides navigation when the sidebar is hidden in drawer mode (<640px).
    fn build_drawer_menu_button(
        state: &Rc<RefCell<AppState>>,
        content_area: &GtkBox,
    ) -> MenuButton {
        let menu_button = MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some("Navigation"));
        menu_button.add_css_class("maintainer_drawer_menu_button");
        menu_button.update_property(&[gtk4::accessible::Property::Label("Navigation menu")]);

        let popover = Self::build_drawer_popover(state, content_area);
        menu_button.set_popover(Some(&popover));

        menu_button
    }

    /// Build the popover content listing all navigable pages.
    fn build_drawer_popover(state: &Rc<RefCell<AppState>>, content_area: &GtkBox) -> Popover {
        let popover = Popover::new();
        popover.add_css_class("maintainer_drawer_popover");

        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(8);
        content.set_margin_end(8);

        // Add all pages from navigation groups
        for group in NAVIGATION_GROUPS {
            for &page in group.pages {
                let btn = Self::build_popover_nav_button(page, state, content_area, &popover);
                content.append(&btn);
            }
        }

        // Footer pages
        for &page in FOOTER_PAGES {
            let btn = Self::build_popover_nav_button(page, state, content_area, &popover);
            content.append(&btn);
        }

        popover.set_child(Some(&content));
        popover
    }

    /// Build a single navigation button inside the drawer popover.
    fn build_popover_nav_button(
        page: Page,
        state: &Rc<RefCell<AppState>>,
        content_area: &GtkBox,
        popover: &Popover,
    ) -> Button {
        let btn = Button::new();
        btn.add_css_class("maintainer_nav_button");

        let btn_content = GtkBox::new(Orientation::Horizontal, 8);
        btn_content.set_halign(Align::Start);

        let icon = Image::from_icon_name(page.icon_name());
        icon.set_pixel_size(20);
        btn_content.append(&icon);

        let label = Label::new(Some(page.label()));
        btn_content.append(&label);

        btn.set_child(Some(&btn_content));
        btn.set_tooltip_text(Some(page.label()));
        btn.update_property(&[gtk4::accessible::Property::Label(page.label())]);

        let popover_ref = popover.clone();
        let state_ref = state.clone();
        let content_ref = content_area.clone();
        btn.connect_clicked(move |_| {
            {
                let mut s = state_ref.borrow_mut();
                s.active_page = page;
            }
            Self::set_placeholder(&content_ref, page);
            popover_ref.popdown();
        });

        btn
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
