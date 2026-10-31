//! Application shell: main window layout with navigation and page stack.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;
use std::time::{Duration, Instant};

use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, Label, MenuButton, Orientation, Popover,
    Stack, StackTransitionType,
};

use crate::monitoring::MonitoringMessage;
use crate::runtime::RuntimeHandle;
use crate::state::{AppState, FOOTER_PAGES, NAVIGATION_GROUPS, Page};
use crate::ui::cleanup::CleanupPage;
use crate::ui::diagnostics::DiagnosticsPage;
use crate::ui::icons;
use crate::ui::navigation::{self, NavigationMode};
use crate::ui::network::NetworkPage;
use crate::ui::overview::OverviewPage;
use crate::ui::processes::ProcessesPage;
use crate::ui::reports::build_reports_page;
use crate::ui::settings::build_settings_page;
use crate::ui::storage::StoragePage;
use crate::ui::system::SystemPage;
use crate::ui::updates::UpdatesPage;

mod cleanup;
mod diagnostics;
mod settings;
mod storage;
mod updates;

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
        // Minimum size enforces landscape orientation where width > height.
        window.set_size_request(640, 420);

        let overview_page = OverviewPage::new();
        let system_page = SystemPage::new();
        let network_page = NetworkPage::new(runtime.borrow().executor.clone());
        let processes_page = ProcessesPage::new();
        // GTK4 has no refuse-fullscreen flag. Reverse compositor or keyboard
        // fullscreen requests as soon as the asynchronous property changes.
        window.connect_fullscreened_notify(|window| {
            if window.is_fullscreen() {
                window.unfullscreen();
            }
        });
        let page_stack = Stack::new();
        page_stack.set_transition_type(StackTransitionType::None);
        page_stack.set_hexpand(true);
        page_stack.set_vexpand(true);

        page_stack.add_named(overview_page.widget(), Some("Overview"));
        page_stack.add_named(system_page.widget(), Some("System"));
        page_stack.add_named(network_page.widget(), Some("Network"));
        page_stack.add_named(processes_page.widget(), Some("Processes"));
        for page in [
            Page::Storage,
            Page::Cleanup,
            Page::Updates,
            Page::Diagnostics,
            Page::Reports,
            Page::Settings,
        ] {
            let runtime = runtime.clone();
            let window = window.downgrade();
            Self::add_lazy_page(&page_stack, page.label(), move || match page {
                Page::Storage => {
                    let page = StoragePage::new();
                    Self::wire_storage_dispatch(&page, &runtime);
                    Some(page.widget().clone().upcast())
                }
                Page::Cleanup => {
                    let window = window.upgrade()?;
                    let page = CleanupPage::new();
                    Self::wire_cleanup_dispatch(&page, &runtime, &window);
                    Some(page.widget().clone().upcast())
                }
                Page::Updates => {
                    let page = UpdatesPage::new();
                    Self::wire_updates_dispatch(&page, &runtime);
                    Some(page.widget().clone().upcast())
                }
                Page::Diagnostics => {
                    let page = DiagnosticsPage::new();
                    Self::wire_diagnostics_dispatch(&page, &runtime);
                    Some(page.widget().clone().upcast())
                }
                Page::Reports => Some(build_reports_page(runtime).upcast()),
                Page::Settings => {
                    Some(Self::build_runtime_settings_page(&runtime, &window.upgrade()?).upcast())
                }
                _ => None,
            });
        }

        page_stack.set_visible_child_name("Overview");

        let drawer_strip = GtkBox::new(Orientation::Horizontal, 0);
        drawer_strip.add_css_class("maintainer_drawer_strip");
        drawer_strip.set_visible(false);

        let page_stack_ref = page_stack.clone();
        let state_for_drawer = state.clone();
        let menu_button = Self::build_drawer_menu_button(&state_for_drawer, &page_stack_ref);
        drawer_strip.append(&menu_button);

        let outer_box = GtkBox::new(Orientation::Vertical, 0);
        outer_box.append(&drawer_strip);

        let root_box = GtkBox::new(Orientation::Horizontal, 0);
        root_box.set_vexpand(true);
        root_box.add_css_class("maintainer_layout_wide");

        let sidebar_container = GtkBox::new(Orientation::Vertical, 0);
        sidebar_container.set_width_request(220);
        sidebar_container.add_css_class("maintainer_sidebar_container");

        root_box.append(&sidebar_container);
        root_box.append(&page_stack);
        outer_box.append(&root_box);

        window.set_child(Some(&outer_box));

        let page_stack_for_nav = page_stack.clone();
        let state_for_nav = state.clone();
        Self::rebuild_sidebar_widget(
            &sidebar_container,
            &page_stack_for_nav,
            &state_for_nav,
            NavigationMode::Persistent,
        );

        let state_for_resize = state.clone();
        let sidebar_for_resize = sidebar_container.clone();
        let stack_for_resize = page_stack.clone();
        let menu_button_for_resize = menu_button.clone();
        let drawer_strip_for_resize = drawer_strip.clone();
        let root_for_resize = root_box.clone();
        window.connect_realize(move |win| {
            let Some(surface) = win.surface() else {
                return;
            };

            let state = state_for_resize.clone();
            let sidebar = sidebar_for_resize.clone();
            let stack = stack_for_resize.clone();
            let menu_button = menu_button_for_resize.clone();
            let drawer_strip = drawer_strip_for_resize.clone();
            let root = root_for_resize.clone();
            let apply_width = Rc::new(move |width: i32| {
                if width <= 0 {
                    return;
                }
                let mode = NavigationMode::from_width(width);
                for class_name in [
                    "maintainer_layout_wide",
                    "maintainer_layout_compact",
                    "maintainer_layout_narrow",
                ] {
                    root.remove_css_class(class_name);
                }
                root.add_css_class(match mode {
                    NavigationMode::Persistent => "maintainer_layout_wide",
                    NavigationMode::Compact => "maintainer_layout_compact",
                    NavigationMode::Drawer => "maintainer_layout_narrow",
                });
                let new_width = match mode {
                    NavigationMode::Persistent => 220,
                    NavigationMode::Compact => 60,
                    NavigationMode::Drawer => 0,
                };
                sidebar.set_width_request(new_width);
                sidebar.set_visible(mode != NavigationMode::Drawer);
                menu_button.set_visible(mode == NavigationMode::Drawer);
                drawer_strip.set_visible(mode == NavigationMode::Drawer);
                Self::rebuild_sidebar_widget(&sidebar, &stack, &state, mode);
            });

            apply_width(surface.width());
            let apply_width_for_notify = apply_width.clone();
            surface.connect_width_notify(move |surface| {
                apply_width_for_notify(surface.width());
            });
        });

        let page_stack_for_overview = page_stack.clone();
        let sidebar_for_overview = sidebar_container.clone();
        let state_for_overview = state.clone();
        overview_page.connect_navigate(move |target_page| {
            state_for_overview
                .borrow_mut()
                .reduce(crate::state::AppIntent::Navigate(target_page));
            page_stack_for_overview.set_visible_child_name(target_page.label());
            crate::ui::navigation::update_active_buttons(&sidebar_for_overview, target_page);
        });

        let overview_ref = Rc::new(overview_page);
        let system_ref = Rc::new(system_page);
        let network_ref = Rc::new(network_page);
        let processes_ref = Rc::new(processes_page);

        let runtime_for_proc_sort = runtime.clone();
        let processes_for_sort = processes_ref.clone();
        processes_ref.connect_sort_changed(move |sort| {
            let mut rt = runtime_for_proc_sort.borrow_mut();
            rt.monitoring
                .reduce(MonitoringMessage::SetProcessSort(sort));
            processes_for_sort.update(&rt.monitoring.processes_vm);
        });

        let runtime_for_proc_terminate = runtime.clone();
        let proc_terminate_status = processes_ref.operation_status_handle().clone();
        let parent_for_process = window.downgrade();
        processes_ref.connect_terminate(move |pid, start_time_ticks| {
            proc_terminate_status.remove_css_class("error");
            proc_terminate_status.set_text("Preparing process review…");
            proc_terminate_status.set_visible(true);
            let rt = runtime_for_proc_terminate.borrow();
            let helper = rt.helper.clone();
            let executor = rt.executor.clone();
            drop(rt);
            let executor_for_review = executor.clone();
            let status = proc_terminate_status.clone();
            let parent = parent_for_process.clone();
            crate::dispatch::dispatch_local_action(
                &executor,
                move || maintainer_core::metrics::ProcessHandle::acquire(
                    maintainer_core::metrics::ProcessIdentity { pid, start_time_ticks }
                ).map_err(|error| error.to_string()),
                move |result| {
                    let handle = match result {
                        Ok(handle) => handle,
                        Err(error) => {
                            status.add_css_class("error");
                            status.set_text(&error);
                            return;
                        }
                    };
                    let Some(parent) = parent.upgrade() else { return; };
                    let dialog = gtk4::MessageDialog::builder()
                        .transient_for(&parent).modal(true)
                        .message_type(gtk4::MessageType::Warning)
                        .text("End this process?")
                        .secondary_text(format!(
                            "{} (PID {}, start tick {}). Unsaved work may be lost. This sends SIGTERM; exit is not guaranteed.",
                            handle.name(), handle.identity().pid, handle.identity().start_time_ticks
                        )).build();
                    dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
                    dialog.add_button("Send SIGTERM", gtk4::ResponseType::Accept);
                    dialog.set_default_response(gtk4::ResponseType::Cancel);
                    let retained_handle = RefCell::new(Some(handle));
                    status.set_text("Review the process before authorization.");
                    dialog.connect_response(move |dialog, response| {
                        dialog.close();
                        let Some(handle) = retained_handle.borrow_mut().take() else { return; };
                        if response != gtk4::ResponseType::Accept {
                            status.set_text("Cancelled; no signal was sent.");
                            return;
                        }
                        status.set_text("Requesting authorization…");
                        let status = status.clone();
                        let helper = helper.clone();
                        crate::dispatch::dispatch_helper_call(
                            &executor_for_review,
                            move || helper.terminate_process(handle),
                            move |result| Self::show_helper_result(&status, result),
                        );
                    });
                    dialog.present();
                },
            );
        });
        let runtime_for_timer = runtime.clone();
        let overview_for_timer = overview_ref.clone();
        let system_for_timer = system_ref.clone();
        let network_for_timer = network_ref.clone();
        let processes_for_timer = processes_ref.clone();

        let runtime_for_vis = runtime.clone();
        window.connect_visible_notify(move |win| {
            runtime_for_vis
                .borrow_mut()
                .monitoring
                .reduce(MonitoringMessage::VisibilityChanged(
                    Self::monitoring_window_visible(win),
                ));
        });
        let runtime_for_surface = runtime.clone();
        window.connect_realize(move |win| {
            if let Some(toplevel) = win.surface().and_downcast::<gtk4::gdk::Toplevel>() {
                let window = win.downgrade();
                let runtime = runtime_for_surface.clone();
                toplevel.connect_state_notify(move |_| {
                    if let Some(window) = window.upgrade() {
                        runtime.borrow_mut().monitoring.reduce(
                            MonitoringMessage::VisibilityChanged(Self::monitoring_window_visible(
                                &window,
                            )),
                        );
                    }
                });
            }
        });

        Self::start_monitoring_timer(
            runtime_for_timer,
            overview_for_timer,
            system_for_timer,
            network_for_timer,
            processes_for_timer,
            page_stack.clone(),
        );

        Self::wire_settings_close_drain(&window, &runtime, &page_stack);
        crate::probe::install_navigation_canary(&window, &page_stack, &sidebar_container);

        Shell { window }
    }

    fn monitoring_window_visible(window: &ApplicationWindow) -> bool {
        window.is_visible()
            && !window
                .surface()
                .and_downcast::<gtk4::gdk::Toplevel>()
                .is_some_and(|toplevel| {
                    toplevel
                        .state()
                        .contains(gtk4::gdk::ToplevelState::MINIMIZED)
                })
    }

    fn add_lazy_page(
        stack: &Stack,
        name: &'static str,
        factory: impl FnOnce() -> Option<gtk4::Widget> + 'static,
    ) {
        let host = GtkBox::new(Orientation::Vertical, 0);
        host.set_hexpand(true);
        host.set_vexpand(true);
        stack.add_named(&host, Some(name));
        let factory = RefCell::new(Some(factory));
        stack.connect_visible_child_name_notify(move |stack| {
            if stack.visible_child_name().as_deref() != Some(name) {
                return;
            }
            // Consume before constructing widgets: notifications cannot build a
            // second instance, and cached roots retain active jobs and UI state.
            let build = factory.borrow_mut().take();
            if let Some(widget) = build.and_then(|build| build()) {
                host.append(&widget);
            }
        });
    }

    /// Present the window.
    pub fn present(&self) {
        self.window.present();
    }

    fn show_helper_result(label: &Label, result: crate::helper_client::HelperResult) {
        use maintainer_core::capability::Capability;

        match result {
            Capability::Available(response) => {
                label.remove_css_class("error");
                label.add_css_class("success");
                label.set_text(&response.message);
            }
            Capability::Unavailable(reason) => {
                label.remove_css_class("success");
                label.add_css_class("error");
                label.set_text(&reason.message);
            }
            Capability::PermissionRequired(request) => {
                label.remove_css_class("success");
                label.add_css_class("error");
                label.set_text(&request.explanation);
            }
        }
        label.set_visible(true);
    }

    /// Get a reference to the application window.
    pub fn window(&self) -> &ApplicationWindow {
        &self.window
    }

    fn start_monitoring_timer(
        runtime: RuntimeHandle,
        overview: Rc<OverviewPage>,
        system: Rc<SystemPage>,
        network: Rc<NetworkPage>,
        processes: Rc<ProcessesPage>,
        page_stack: Stack,
    ) {
        let tick_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        let latest_snapshot = std::sync::Arc::new(std::sync::Mutex::new(None));
        let snapshot_for_worker = latest_snapshot.clone();
        let (device_info_tx, device_info_rx) =
            std::sync::mpsc::channel::<crate::monitoring::DeviceInfo>();
        let mut connection_pending: Option<
            maintainer_core::jobs::JobReceiver<maintainer_core::metrics::ConnectionDetails>,
        > = None;

        // Coalesce worker requests. Only the Processes page requests a scan;
        // its view model owns the retained rows between fresh process samples.
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<bool>(1);

        let device_info_worker_started = std::thread::Builder::new()
            .name("device-info-worker".into())
            .spawn(move || {
                let info = crate::monitoring::collect_device_info();
                let _ = device_info_tx.send(info);
            })
            .is_ok();
        if !device_info_worker_started {
            let info = crate::monitoring::DeviceInfo {
                model: "Unavailable".into(),
                os: "Unavailable".into(),
                kernel: "Unavailable".into(),
                cpu: "Unavailable".into(),
                gpu: "Unavailable".into(),
                memory: "Unavailable".into(),
                storage: "Unavailable".into(),
                desktop_env: "Unavailable".into(),
                display_server: "Unavailable".into(),
            };
            runtime.borrow_mut().monitoring.overview.device_info = info;
        }

        let metric_worker_started = std::thread::Builder::new()
            .name("metric-worker".into())
            .spawn(move || {
                let mut collector = maintainer_core::metrics::SystemCollector::new();
                while let Ok(mut needs_processes) = request_rx.recv() {
                    // Drain coalesced ticks; keep the most recent request's flag.
                    while let Ok(latest) = request_rx.try_recv() {
                        needs_processes = latest;
                    }

                    let snapshot = collector.collect(needs_processes);

                    match snapshot_for_worker.lock() {
                        Ok(mut latest) => *latest = Some(Box::new(snapshot)),
                        Err(_) => break,
                    }
                }
            })
            .is_ok();
        if !metric_worker_started {
            let mut runtime = runtime.borrow_mut();
            runtime.monitoring.overview.cpu_text = "Unavailable".into();
            runtime.monitoring.overview.memory_text = "Unavailable".into();
            runtime.monitoring.overview.disk_text = "Unavailable".into();
            runtime.monitoring.overview.network_text = "Unavailable".into();
            runtime.monitoring.overview.cpu_unavailable_reason =
                Some("Monitoring worker could not start".into());
            runtime.monitoring.overview.memory_unavailable_reason =
                Some("Monitoring worker could not start".into());
            runtime.monitoring.network_vm.unavailable_reason =
                Some("Monitoring worker could not start".into());
            runtime.monitoring.processes_vm.unavailable_reason =
                Some("Monitoring worker could not start".into());
        }

        let last_connection_request = Rc::new(Cell::new(None::<Instant>));

        // Refresh a page as soon as it becomes visible. The timer only updates
        // the page on screen, so without this a page switched to would show
        // whatever it rendered when it was last visible until the next tick.
        {
            let runtime_for_switch = runtime.clone();
            let overview_for_switch = overview.clone();
            let system_for_switch = system.clone();
            let network_for_switch = network.clone();
            let processes_for_switch = processes.clone();
            let last_connection_request = last_connection_request.clone();
            page_stack.connect_visible_child_name_notify(move |stack| {
                let rt = runtime_for_switch.borrow();
                match stack.visible_child_name().as_deref() {
                    Some("Overview") => overview_for_switch.update(&rt.monitoring.overview),
                    Some("System") => system_for_switch.update(&rt.monitoring.system),
                    Some("Network") => {
                        last_connection_request.set(None);
                        network_for_switch.update(&rt.monitoring.network_vm);
                    }
                    Some("Processes") => processes_for_switch.update(&rt.monitoring.processes_vm),
                    _ => {}
                }
            });
        }

        // Timer tick: request samples and drain results.
        glib::timeout_add_local(Duration::from_secs(1), move || {
            if page_stack.root().is_none() {
                return ControlFlow::Break;
            }
            let mut rt = runtime.borrow_mut();
            let was_loading = rt.db_loading;
            let loaded_theme = rt.poll_database_initialization();
            let write_result = rt.poll_settings_writes();
            let reset_settings = write_result
                .as_ref()
                .is_some_and(|result| result.reset_defaults);
            let settings_theme = reset_settings.then_some(rt.settings.theme).or(loaded_theme);
            let database_finished = was_loading && !rt.db_loading;
            if let Some(widget) = page_stack.child_by_name("Settings") {
                widget.set_sensitive(!rt.db_loading && !rt.clearing_settings);
            }
            if let Some(result) = write_result.filter(|_| {
                page_stack
                    .root()
                    .is_some_and(|window| window.is_sensitive())
            }) {
                if let Some(window) = page_stack.root().and_downcast::<ApplicationWindow>() {
                    match result.result {
                        Ok(deleted) if result.cleared => Self::show_settings_result(
                            &window,
                            true,
                            &format!(
                                "Deleted {deleted} local record(s). Settings restored to defaults."
                            ),
                        ),
                        Err(message) => Self::show_settings_result(&window, false, &message),
                        Ok(_) => {}
                    }
                }
            }
            if database_finished || reset_settings {
                drop(rt);
                if let Some(window) = page_stack.root().and_downcast::<ApplicationWindow>() {
                    Self::refresh_mounted_settings(&runtime, &window, &page_stack);
                }
                if let Some(theme) = settings_theme {
                    crate::application::apply_theme(theme);
                }
                rt = runtime.borrow_mut();
            }
            let immediate = rt.monitoring.immediate_sample_pending;
            let interval_secs = rt.monitoring.interval.as_secs() as u32;

            let mut ticks = tick_counter.borrow_mut();
            *ticks += 1;

            let should_sample = immediate || *ticks >= interval_secs;

            // Only refresh the page on screen. Both update() paths rebuild
            // widget subtrees, which dirties the frame and forces a repaint;
            // doing that for a hidden page costs CPU and changes nothing
            // visible. The view models are still updated by reduce(), and the
            // page-switch handler repaints the incoming page immediately.
            let visible_page = page_stack.visible_child_name();
            let visible_page = visible_page.as_deref();
            while let Ok(info) = device_info_rx.try_recv() {
                rt.monitoring.overview.device_info = info;
                if visible_page == Some("Overview") {
                    overview.update(&rt.monitoring.overview);
                }
            }
            let connection_result = connection_pending
                .as_ref()
                .map(|receiver| receiver.try_recv());
            if let Some(result) = connection_result {
                match result {
                    Ok(maintainer_core::jobs::JobMessage::Completed(details)) => {
                        rt.monitoring.network_vm.connection_details = details;
                        rt.monitoring.network_vm.connection_unavailable_reason = None;
                        connection_pending = None;
                    }
                    Err(std::sync::mpsc::TryRecvError::Empty) => {}
                    _ => {
                        rt.monitoring.network_vm.connection_details = Default::default();
                        rt.monitoring.network_vm.connection_unavailable_reason = Some(
                            "Connection details are unavailable; another operation may be busy."
                                .into(),
                        );
                        connection_pending = None;
                    }
                }
                if visible_page == Some("Network") {
                    network.update(&rt.monitoring.network_vm);
                }
            }
            let snapshot = latest_snapshot
                .lock()
                .ok()
                .and_then(|mut latest| latest.take());
            if let Some(snapshot) = snapshot {
                rt.monitoring
                    .reduce(MonitoringMessage::SnapshotReady(snapshot));
                match visible_page {
                    Some("Overview") => overview.update(&rt.monitoring.overview),
                    Some("System") => system.update(&rt.monitoring.system),
                    Some("Network") => network.update(&rt.monitoring.network_vm),
                    Some("Processes") => processes.update(&rt.monitoring.processes_vm),
                    _ => {}
                }
                crate::probe::mark_once("first-data");
            }

            if visible_page == Some("Network") {
                let due = last_connection_request
                    .get()
                    .is_none_or(|last| last.elapsed() >= Duration::from_secs(10));
                if due && connection_pending.is_none() {
                    let token = maintainer_core::jobs::CancellationToken::new();
                    let worker_token = token.clone();
                    connection_pending = Some(rt.executor.execute_sized(vec![move || {
                        Ok(maintainer_core::metrics::connection::read_connection_details_cancellable(&worker_token))
                    }], token, |details| {
                        std::mem::size_of_val(details)
                            + details.ip_address.as_ref().map_or(0, String::capacity)
                            + details.gateway.as_ref().map_or(0, String::capacity)
                            + details.dns_servers.capacity() * std::mem::size_of::<String>()
                            + details.dns_servers.iter().map(String::capacity).sum::<usize>()
                            + details.wifi.as_ref().map_or(0, |wifi| std::mem::size_of_val(wifi) + wifi.ssid.capacity() + wifi.signal.capacity() + wifi.link_speed.capacity() + wifi.frequency.capacity())
                    }));
                    last_connection_request.set(Some(Instant::now()));
                }
            }

            if !should_sample {
                return ControlFlow::Continue;
            }

            *ticks = 0;
            rt.monitoring.immediate_sample_pending = false;

            // Non-blocking: if worker is busy, this tick coalesces.
            let needs_processes = page_stack.visible_child_name().as_deref() == Some("Processes");
            if metric_worker_started {
                let _ = request_tx.try_send(needs_processes);
            }

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

        let icon = icons::icon_with_fallback(page.icon_chain(), 16);
        btn_content.append(&icon);

        let label = Label::new(Some(page.label()));
        btn_content.append(&label);

        btn.set_child(Some(&btn_content));
        btn.set_tooltip_text(Some(page.label()));
        btn.update_property(&[gtk4::accessible::Property::Label(page.label())]);
        if state.borrow().active_page == page {
            btn.add_css_class("maintainer_nav_active");
        }

        let popover_ref = popover.clone();
        let state_ref = state.clone();
        let stack_ref = page_stack.clone();
        btn.connect_clicked(move |_| {
            {
                let mut s = state_ref.borrow_mut();
                s.active_page = page;
            }
            navigation::update_active_buttons(&popover_ref, page);
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

    /// Wire storage scans to state and render their results.
    /// Wire cleanup scan, review, confirmation, and execution.
    /// Wire update discovery, review, and helper-authorized APT execution.
    /// Wire diagnostics collection and render evidence-first findings.
    fn format_size(bytes: u64) -> String {
        const KIB: f64 = 1024.0;
        const MIB: f64 = KIB * 1024.0;
        const GIB: f64 = MIB * 1024.0;
        let bytes = bytes as f64;
        if bytes >= GIB {
            format!("{:.1} GiB", bytes / GIB)
        } else if bytes >= MIB {
            format!("{:.1} MiB", bytes / MIB)
        } else if bytes >= KIB {
            format!("{:.1} KiB", bytes / KIB)
        } else {
            format!("{} B", bytes as u64)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run this exact test under Xvfb"]
    fn secondary_page_factories_mount_once_and_preserve_navigation_and_widgets() {
        gtk4::init().unwrap();
        let stack = Stack::new();
        let primary = GtkBox::new(Orientation::Vertical, 0);
        stack.add_named(&primary, Some("Overview"));
        stack.set_visible_child_name("Overview");
        let storage_count = Rc::new(Cell::new(0));
        let settings_count = Rc::new(Cell::new(0));
        let storage_widget = Rc::new(RefCell::new(None::<Label>));
        let storage_calls = storage_count.clone();
        let storage = storage_widget.clone();
        Shell::add_lazy_page(&stack, "Storage", move || {
            storage_calls.set(storage_calls.get() + 1);
            let widget = Label::new(Some("Initial state"));
            *storage.borrow_mut() = Some(widget.clone());
            Some(widget.upcast())
        });
        let settings_calls = settings_count.clone();
        Shell::add_lazy_page(&stack, "Settings", move || {
            settings_calls.set(settings_calls.get() + 1);
            Some(Label::new(Some("Settings content")).upcast())
        });
        let storage_host = stack.child_by_name("Storage").unwrap();
        let settings_host = stack.child_by_name("Settings").unwrap();
        assert!(storage_widget.borrow().is_none());
        assert_eq!(storage_count.get(), 0);
        assert_eq!(settings_count.get(), 0);
        assert!(storage_host.first_child().is_none());
        assert!(settings_host.first_child().is_none());

        stack.set_visible_child_name("Storage");
        assert_eq!(storage_count.get(), 1);
        assert_eq!(settings_count.get(), 0);
        let storage_widget = storage_widget.borrow().clone().unwrap();
        assert_eq!(
            storage_host.first_child(),
            Some(storage_widget.clone().upcast())
        );
        storage_widget.set_text("State from an active operation");
        stack.set_visible_child_name("Settings");
        let settings_content = settings_host.first_child().unwrap();
        stack.set_visible_child_name("Overview");
        stack.set_visible_child_name("Storage");
        stack.set_visible_child_name("Settings");
        assert_eq!(storage_count.get(), 1);
        assert_eq!(settings_count.get(), 1);
        assert_eq!(stack.child_by_name("Storage"), Some(storage_host.clone()));
        assert_eq!(stack.child_by_name("Settings"), Some(settings_host.clone()));
        assert_eq!(
            storage_host.first_child(),
            Some(storage_widget.clone().upcast())
        );
        assert_eq!(storage_widget.text(), "State from an active operation");
        assert_eq!(settings_host.first_child(), Some(settings_content));
        assert_eq!(stack.visible_child(), Some(settings_host));
        assert_eq!(stack.visible_child_name().as_deref(), Some("Settings"));
    }

    #[test]
    #[ignore = "requires a GTK display; run this exact test under Xvfb"]
    fn shell_geometry_and_navigation_follow_allocated_surface_width() {
        gtk4::init().unwrap();
        let runtime = crate::runtime::Runtime::isolated_for_test();
        let app = gtk4::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
        app.register(None::<&gio::Cancellable>).unwrap();
        let shell = Shell::new(&app, runtime);
        let window = shell.window();
        assert_eq!(window.default_size(), (1100, 700));
        assert_eq!(window.size_request(), (640, 420));
        assert!(window.titlebar().is_none());
        let outer = window.child().unwrap();
        let root = outer.last_child().unwrap();
        let sidebar = root.first_child().unwrap();
        let mut pending = vec![outer.clone()];
        while let Some(widget) = pending.pop() {
            assert!(!widget.is::<gtk4::HeaderBar>());
            let mut child = widget.first_child();
            while let Some(current) = child {
                child = current.next_sibling();
                pending.push(current);
            }
        }
        let wait_for = |condition: &dyn Fn() -> bool| {
            let deadline = Instant::now() + Duration::from_secs(3);
            while !condition() && Instant::now() < deadline {
                while glib::MainContext::default().pending() {
                    glib::MainContext::default().iteration(false);
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            assert!(
                condition(),
                "surface allocation did not reach expected navigation mode"
            );
        };
        window.present();
        wait_for(&|| root.has_css_class("maintainer_layout_wide"));
        wait_for(&|| {
            window
                .surface()
                .is_some_and(|surface| surface.width() >= 900)
        });
        window.set_default_size(700, 700);
        // A requested default size does not change the allocated surface yet.
        // Navigation must wait for the actual allocation rather than this setter.
        assert!(window.surface().unwrap().width() >= 900);
        assert!(root.has_css_class("maintainer_layout_wide"));
        wait_for(&|| root.has_css_class("maintainer_layout_compact"));
        assert_eq!(sidebar.width_request(), 60);
        // A child minimum can also resize the surface, without a size request.
        root.set_size_request(1000, -1);
        wait_for(&|| root.has_css_class("maintainer_layout_wide"));
        assert!(window.surface().unwrap().width() >= 1000);
        assert_eq!(sidebar.width_request(), 220);
        assert!(sidebar.is_visible());
        window.close();
    }
}
