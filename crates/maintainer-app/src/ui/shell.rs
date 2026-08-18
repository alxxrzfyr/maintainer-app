//! Application shell: main window layout with navigation and page stack.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, HeaderBar, Image, Label, MenuButton,
    Orientation, Popover, Stack, StackTransitionType,
};

use maintainer_core::settings::SettingsRepository;

use crate::monitoring::MonitoringMessage;
use crate::runtime::RuntimeHandle;
use crate::state::{AppState, FOOTER_PAGES, NAVIGATION_GROUPS, Page};
use crate::ui::cleanup::CleanupPage;
use crate::ui::diagnostics::DiagnosticsPage;
use crate::ui::navigation::{self, NavigationMode};
use crate::ui::overview::OverviewPage;
use crate::ui::reports::build_reports_page;
use crate::ui::settings::build_settings_page;
use crate::ui::storage::StoragePage;
use crate::ui::system::SystemPage;
use crate::ui::updates::UpdatesPage;

/// The application shell manages the window layout and page lifecycle.
pub struct Shell {
    window: ApplicationWindow,
}

impl Shell {
    /// Create a new shell attached to the given application.
    pub fn new(app: &gtk4::Application, runtime: RuntimeHandle) -> Self {
        let state = Rc::new(RefCell::new(AppState::default()));

        let window = ApplicationWindow::builder()
            .application(app)
            .title("Maintainer")
            .default_width(1100)
            .default_height(700)
            .build();

        window.add_css_class("maintainer_window");

        // Build pages once
        let overview_page = OverviewPage::new();
        let system_page = SystemPage::new();
        let storage_page = StoragePage::new();
        let cleanup_page = CleanupPage::new();
        let updates_page = UpdatesPage::new();
        let diagnostics_page = DiagnosticsPage::new();
        let reports_widget = build_reports_page();

        let settings_snapshot = runtime.borrow().settings.clone();
        let runtime_for_settings = runtime.clone();
        let settings_widget = build_settings_page(&settings_snapshot, move |intent| {
            let mut rt = runtime_for_settings.borrow_mut();
            // Apply intent to settings state and persist
            let mut settings_state =
                crate::state::SettingsState::from_snapshot(rt.settings.clone());
            if settings_state.reduce(intent) {
                rt.settings = settings_state.snapshot.clone();
                if let Some(db) = rt.database() {
                    let repo = maintainer_core::settings::SqliteSettingsRepository::new(db);
                    let core_settings = to_core_settings(&settings_state.snapshot);
                    if let Err(e) = repo.save(&core_settings) {
                        eprintln!("warning: failed to save settings: {e}");
                    }
                }
            }
        });

        // Page stack
        let page_stack = Stack::new();
        page_stack.set_transition_type(StackTransitionType::None);
        page_stack.set_hexpand(true);
        page_stack.set_vexpand(true);

        page_stack.add_named(overview_page.widget(), Some("Overview"));
        page_stack.add_named(system_page.widget(), Some("System"));
        page_stack.add_named(storage_page.widget(), Some("Storage"));
        page_stack.add_named(cleanup_page.widget(), Some("Cleanup"));
        page_stack.add_named(updates_page.widget(), Some("Updates"));
        page_stack.add_named(diagnostics_page.widget(), Some("Diagnostics"));
        page_stack.add_named(&reports_widget, Some("Reports"));
        page_stack.add_named(&settings_widget, Some("Settings"));

        page_stack.set_visible_child_name("Overview");

        // Header bar with hamburger menu for narrow widths
        let header = HeaderBar::new();
        header.add_css_class("maintainer_header");

        let page_stack_ref = page_stack.clone();
        let state_for_drawer = state.clone();
        let menu_button = Self::build_drawer_menu_button(&state_for_drawer, &page_stack_ref);
        menu_button.set_visible(false);
        header.pack_start(&menu_button);

        // Main layout
        let outer_box = GtkBox::new(Orientation::Vertical, 0);
        outer_box.append(&header);

        let root_box = GtkBox::new(Orientation::Horizontal, 0);
        root_box.set_vexpand(true);

        let sidebar_container = GtkBox::new(Orientation::Vertical, 0);
        sidebar_container.set_width_request(220);
        sidebar_container.add_css_class("maintainer_sidebar_container");

        root_box.append(&sidebar_container);
        root_box.append(&page_stack);
        outer_box.append(&root_box);

        window.set_child(Some(&outer_box));

        // Build initial sidebar
        let page_stack_for_nav = page_stack.clone();
        let state_for_nav = state.clone();
        Self::rebuild_sidebar_widget(
            &sidebar_container,
            &page_stack_for_nav,
            &state_for_nav,
            NavigationMode::Persistent,
        );

        // Adapt layout when window width changes
        let state_for_notify = state.clone();
        let sidebar_for_notify = sidebar_container.clone();
        let stack_for_notify = page_stack.clone();
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
                    &stack_for_notify,
                    &state_for_notify,
                    mode,
                );
            }
        });

        // Monitoring timer: sample metrics and update pages
        let overview_ref = Rc::new(overview_page);
        let system_ref = Rc::new(system_page);
        let runtime_for_timer = runtime.clone();
        let overview_for_timer = overview_ref.clone();
        let system_for_timer = system_ref.clone();

        // Visibility tracking for interval switching
        let runtime_for_vis = runtime.clone();
        window.connect_is_active_notify(move |win| {
            let visible = win.is_active();
            let mut rt = runtime_for_vis.borrow_mut();
            rt.monitoring
                .reduce(MonitoringMessage::VisibilityChanged(visible));
        });

        // Start the sampling timer
        Self::start_monitoring_timer(runtime_for_timer, overview_for_timer, system_for_timer);

        Shell { window }
    }

    /// Present the window.
    pub fn present(&self) {
        self.window.present();
    }

    /// Get a reference to the application window.
    pub fn window(&self) -> &ApplicationWindow {
        &self.window
    }

    fn start_monitoring_timer(
        runtime: RuntimeHandle,
        overview: Rc<OverviewPage>,
        system: Rc<SystemPage>,
    ) {
        let tick_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        // 1-second tick provides sub-interval granularity for dynamic switching.
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let mut rt = runtime.borrow_mut();
            let immediate = rt.monitoring.immediate_sample_pending;
            let interval_secs = rt.monitoring.interval.as_secs() as u32;

            let mut ticks = tick_counter.borrow_mut();
            *ticks += 1;

            let should_sample = immediate || *ticks >= interval_secs;

            if !should_sample {
                return ControlFlow::Continue;
            }

            *ticks = 0;
            rt.monitoring.immediate_sample_pending = false;

            let snapshot = rt.collect_snapshot();
            rt.monitoring
                .reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

            overview.update(&rt.monitoring.overview);
            system.update(&rt.monitoring.system);

            ControlFlow::Continue
        });
    }

    fn build_drawer_menu_button(state: &Rc<RefCell<AppState>>, page_stack: &Stack) -> MenuButton {
        let menu_button = MenuButton::new();
        menu_button.set_icon_name("open-menu-symbolic");
        menu_button.set_tooltip_text(Some("Navigation"));
        menu_button.add_css_class("maintainer_drawer_menu_button");
        menu_button.update_property(&[gtk4::accessible::Property::Label("Navigation menu")]);

        let popover = Self::build_drawer_popover(state, page_stack);
        menu_button.set_popover(Some(&popover));

        menu_button
    }

    fn build_drawer_popover(state: &Rc<RefCell<AppState>>, page_stack: &Stack) -> Popover {
        let popover = Popover::new();
        popover.add_css_class("maintainer_drawer_popover");

        let content = GtkBox::new(Orientation::Vertical, 4);
        content.set_margin_top(8);
        content.set_margin_bottom(8);
        content.set_margin_start(8);
        content.set_margin_end(8);

        for group in NAVIGATION_GROUPS {
            for &page in group.pages {
                let btn = Self::build_popover_nav_button(page, state, page_stack, &popover);
                content.append(&btn);
            }
        }

        for &page in FOOTER_PAGES {
            let btn = Self::build_popover_nav_button(page, state, page_stack, &popover);
            content.append(&btn);
        }

        popover.set_child(Some(&content));
        popover
    }

    fn build_popover_nav_button(
        page: Page,
        state: &Rc<RefCell<AppState>>,
        page_stack: &Stack,
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
        let stack_ref = page_stack.clone();
        btn.connect_clicked(move |_| {
            {
                let mut s = state_ref.borrow_mut();
                s.active_page = page;
            }
            stack_ref.set_visible_child_name(page.label());
            popover_ref.popdown();
        });

        btn
    }

    fn rebuild_sidebar_widget(
        sidebar_container: &GtkBox,
        page_stack: &Stack,
        state: &Rc<RefCell<AppState>>,
        mode: NavigationMode,
    ) {
        while let Some(child) = sidebar_container.first_child() {
            sidebar_container.remove(&child);
        }

        let active_page = state.borrow().active_page;

        let state_ref = state.clone();
        let stack_ref = page_stack.clone();

        let sidebar = navigation::build_sidebar(active_page, mode, move |page| {
            {
                let mut s = state_ref.borrow_mut();
                s.active_page = page;
            }
            stack_ref.set_visible_child_name(page.label());
        });

        sidebar_container.append(&sidebar);
    }
}

/// Convert app SettingsSnapshot to core Settings for persistence.
fn to_core_settings(snap: &crate::state::SettingsSnapshot) -> maintainer_core::settings::Settings {
    maintainer_core::settings::Settings {
        theme: match snap.theme {
            crate::state::Theme::System => maintainer_core::settings::ThemePreference::System,
            crate::state::Theme::Light => maintainer_core::settings::ThemePreference::Light,
            crate::state::Theme::Dark => maintainer_core::settings::ThemePreference::Dark,
        },
        refresh_interval_secs: snap.refresh_interval_secs,
        hidden_interval_secs: snap.hidden_interval_secs,
        scan_exclusions: snap.scan_exclusions.clone(),
        large_file_threshold_bytes: snap.large_file_threshold_bytes,
    }
}
