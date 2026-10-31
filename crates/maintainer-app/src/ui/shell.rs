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

    fn refresh_mounted_settings(
        runtime: &RuntimeHandle,
        window: &ApplicationWindow,
        stack: &Stack,
    ) {
        let Some(host) = stack.child_by_name("Settings").and_downcast::<GtkBox>() else {
            return;
        };
        if host.first_child().is_none() {
            return;
        }
        while let Some(child) = host.first_child() {
            host.remove(&child);
        }
        host.append(&Self::build_runtime_settings_page(runtime, window));
    }

    fn wire_settings_close_drain(
        window: &ApplicationWindow,
        runtime: &RuntimeHandle,
        pages: &Stack,
    ) {
        use crate::runtime::SettingsCloseState;
        let allow_close = Rc::new(Cell::new(false));
        let draining = Rc::new(Cell::new(false));
        let runtime = runtime.clone();
        let pages = pages.clone();
        window.connect_close_request(move |window| {
            if allow_close.get() {
                return glib::Propagation::Proceed;
            }
            if draining.get() {
                return glib::Propagation::Stop;
            }
            if runtime.borrow().settings_close_state(false) == SettingsCloseState::Ready {
                return glib::Propagation::Proceed;
            }
            draining.set(true);
            window.set_sensitive(false);
            let progress = gtk4::MessageDialog::builder().transient_for(window).modal(true)
                .text("Saving local changes before closing…")
                .secondary_text("Please wait for settings and confirmed data clearing to finish.")
                .build();
            let allow_close = allow_close.clone();
            let draining = draining.clone();
            let runtime = runtime.clone();
            let pages = pages.clone();
            let window = window.clone();
            let deadline = Instant::now() + Duration::from_secs(5);
            progress.present();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let mut rt = runtime.borrow_mut();
                let reset = rt.poll_settings_writes().is_some_and(|result| result.reset_defaults);
                let state = rt.settings_close_state(Instant::now() >= deadline);
                let theme = rt.settings.theme;
                drop(rt);
                if reset {
                    crate::application::apply_theme(theme);
                    Self::refresh_mounted_settings(&runtime, &window, &pages);
                }
                match state {
                    SettingsCloseState::Pending => ControlFlow::Continue,
                    SettingsCloseState::Ready => {
                        progress.close();
                        allow_close.set(true);
                        window.close();
                        ControlFlow::Break
                    }
                    SettingsCloseState::Failed(error) => {
                        progress.close();
                        let failure = gtk4::MessageDialog::builder().transient_for(&window).modal(true)
                            .message_type(gtk4::MessageType::Warning)
                            .text("Local changes could not be confirmed saved")
                            .secondary_text(format!("{error}\n\nStay open to keep working, or exit with unsaved or incomplete changes."))
                            .build();
                        failure.add_button("Stay Open", gtk4::ResponseType::Cancel);
                        failure.add_button("Exit Anyway", gtk4::ResponseType::Accept);
                        failure.set_default_response(gtk4::ResponseType::Cancel);
                        let window = window.clone();
                        let allow_close = allow_close.clone();
                        let draining = draining.clone();
                        failure.connect_response(move |dialog, response| {
                            dialog.close();
                            window.set_sensitive(true);
                            draining.set(false);
                            if response == gtk4::ResponseType::Accept {
                                allow_close.set(true);
                                window.close();
                            }
                        });
                        failure.present();
                        ControlFlow::Break
                    }
                }
            });
            glib::Propagation::Stop
        });
    }

    fn build_runtime_settings_page(
        runtime: &RuntimeHandle,
        window: &ApplicationWindow,
    ) -> gtk4::ScrolledWindow {
        let settings_snapshot = runtime.borrow().settings.clone();
        let runtime_for_settings = runtime.clone();
        let window_for_settings = window.clone();
        let settings_widget = build_settings_page(&settings_snapshot, move |intent| match intent {
            crate::state::SettingsIntent::RequestDeleteAllHistory => {
                Self::confirm_settings_deletion(&window_for_settings, &runtime_for_settings, false);
            }
            crate::state::SettingsIntent::RequestDeleteAllData => {
                Self::confirm_settings_deletion(&window_for_settings, &runtime_for_settings, true);
            }
            intent => {
                let requested_theme = match &intent {
                    crate::state::SettingsIntent::SetTheme(theme) => Some(*theme),
                    _ => None,
                };
                let mut rt = runtime_for_settings.borrow_mut();
                if rt.db_loading || rt.clearing_settings {
                    return;
                }
                let mut settings_state =
                    crate::state::SettingsState::from_snapshot(rt.settings.clone());
                if settings_state.reduce(intent) {
                    rt.settings = settings_state.snapshot.clone();
                    rt.monitoring.set_sampling_intervals(
                        settings_state.snapshot.refresh_interval_secs,
                        settings_state.snapshot.hidden_interval_secs,
                    );
                    let saved = rt.save_settings(to_core_settings(&settings_state.snapshot));
                    drop(rt);
                    if let Err(message) = saved {
                        Self::show_settings_result(&window_for_settings, false, &message);
                    }
                    if let Some(theme) = requested_theme {
                        crate::application::apply_theme(theme);
                    }
                }
            }
        });

        settings_widget.set_sensitive({
            let rt = runtime.borrow();
            !rt.db_loading && !rt.clearing_settings
        });
        settings_widget
    }

    fn confirm_settings_deletion(
        parent: &ApplicationWindow,
        runtime: &RuntimeHandle,
        delete_all_data: bool,
    ) {
        let (title, body, confirm_label) = if delete_all_data {
            (
                "Delete all Maintainer data?",
                "This permanently removes local settings and maintenance history. The database schema is preserved and default settings are recreated.",
                "Delete All Data",
            )
        } else {
            (
                "Delete all history?",
                "This permanently removes operation, diagnostic, and scan history. Settings are preserved.",
                "Delete History",
            )
        };

        let dialog = gtk4::MessageDialog::builder()
            .transient_for(parent)
            .modal(true)
            .message_type(gtk4::MessageType::Warning)
            .text(title)
            .secondary_text(body)
            .build();
        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        dialog.add_button(confirm_label, gtk4::ResponseType::Accept);

        let runtime_for_response = runtime.clone();
        let parent_for_response = parent.clone();
        dialog.connect_response(move |dialog, response| {
            dialog.close();
            if response != gtk4::ResponseType::Accept {
                return;
            }

            if delete_all_data {
                let result = runtime_for_response.borrow_mut().clear_settings_data();
                if let Err(message) = result {
                    Self::show_settings_result(&parent_for_response, false, &message);
                }
                return;
            }

            let rt = runtime_for_response.borrow();
            let Some(database) = rt.db.clone() else {
                Self::show_settings_result(
                    &parent_for_response,
                    false,
                    "Local database is unavailable; no data was changed.",
                );
                return;
            };
            let executor = rt.executor.clone();
            drop(rt);

            let parent_for_done = parent_for_response.clone();
            crate::dispatch::dispatch_local_action(
                &executor,
                move || {
                    let result = database.clear_history();
                    result.map_err(|error| error.to_string())
                },
                move |result| match result {
                    Ok(deleted) => {
                        Self::show_settings_result(
                            &parent_for_done,
                            true,
                            &format!("Deleted {deleted} local record(s)."),
                        );
                    }
                    Err(error) => Self::show_settings_result(
                        &parent_for_done,
                        false,
                        &format!("Nothing was deleted: {error}"),
                    ),
                },
            );
        });
        dialog.present();
    }

    fn show_settings_result(parent: &ApplicationWindow, success: bool, message: &str) {
        let dialog = gtk4::MessageDialog::builder()
            .transient_for(parent)
            .modal(true)
            .message_type(if success {
                gtk4::MessageType::Info
            } else {
                gtk4::MessageType::Error
            })
            .text(if success {
                "Completed"
            } else {
                "Action failed"
            })
            .secondary_text(message)
            .build();
        dialog.add_button("Close", gtk4::ResponseType::Close);
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
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
    fn wire_storage_dispatch(storage_page: &StoragePage, runtime: &RuntimeHandle) {
        use crate::dispatch::{self, DuplicateScanJobResult, StorageScanResult};
        use crate::state::{StorageIntent, StorageState, StorageSubPage};
        use std::cell::Cell;

        let page = storage_page.clone();
        let home = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home".to_string()));
        let storage_state = Rc::new(RefCell::new(StorageState::default()));
        storage_state
            .borrow_mut()
            .reduce(StorageIntent::SetScanRoot(home));
        page.apply_state(&storage_state.borrow());

        let state_for_change = storage_state.clone();
        let page_for_change = page.clone();
        storage_page
            .overview
            .change_root_button()
            .connect_clicked(move |_| {
                let dialog = gtk4::FileChooserDialog::new(
                    Some("Select Scan Directory"),
                    gtk4::Window::NONE,
                    gtk4::FileChooserAction::SelectFolder,
                    &[
                        ("Cancel", gtk4::ResponseType::Cancel),
                        ("Select", gtk4::ResponseType::Accept),
                    ],
                );
                let state_done = state_for_change.clone();
                let page_done = page_for_change.clone();
                dialog.connect_response(move |dialog, response| {
                    if response == gtk4::ResponseType::Accept {
                        if let Some(file) = dialog.file() {
                            if let Some(path) = file.path() {
                                state_done
                                    .borrow_mut()
                                    .reduce(StorageIntent::SetScanRoot(path));
                                page_done.apply_state(&state_done.borrow());
                            }
                        }
                    }
                    dialog.close();
                });
                dialog.show();
            });

        let state_for_subpage = storage_state.clone();
        let programmatic_nav_guard = storage_page.programmatic_navigation.clone();
        storage_page
            .stack_handle()
            .connect_visible_child_name_notify(move |stack| {
                // Skip re-deriving state for a transition StoragePage::apply_state
                // itself drove: state.sub_page is already authoritative in that
                // case, and re-entering this borrow while apply_state's caller
                // may still hold the same RefCell borrowed panics with
                // BorrowMutError inside a GTK signal callback, which aborts the
                // process rather than unwinding. See StoragePage::apply_state.
                if programmatic_nav_guard.get() {
                    return;
                }
                if let Some(name) = stack.visible_child_name() {
                    let sub = match name.as_str() {
                        "large-files" => StorageSubPage::LargeFiles,
                        "duplicates" => StorageSubPage::Duplicates,
                        _ => StorageSubPage::Overview,
                    };
                    let mut st = state_for_subpage.borrow_mut();
                    if st.sub_page != sub {
                        st.sub_page = sub;
                    }
                }
            });

        let active_kind = Rc::new(Cell::new(0_u8));
        let scan_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));
        let duplicate_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));

        let runtime_for_scan = runtime.clone();
        let state_for_scan = storage_state.clone();
        let page_for_scan = page.clone();
        let token_for_scan = scan_token.clone();
        let kind_for_scan = active_kind.clone();
        storage_page.overview.connect_scan(move || {
            if kind_for_scan.get() != 0 {
                return;
            }

            let (job_id, mut policy) = {
                let mut state = state_for_scan.borrow_mut();
                if !state.reduce(StorageIntent::StartStorageScan) {
                    return;
                }
                let Some(scan) = state.storage_scan.as_ref() else {
                    return;
                };
                (scan.job_id, scan.policy.clone())
            };

            let rt = runtime_for_scan.borrow();
            policy.large_file_threshold_bytes = rt.settings.large_file_threshold_bytes;
            policy
                .exclusions
                .extend(rt.settings.scan_exclusions.iter().map(PathBuf::from));
            let executor = rt.executor.clone();
            drop(rt);

            kind_for_scan.set(1);
            page_for_scan.apply_state(&state_for_scan.borrow());

            let progress_state = state_for_scan.clone();
            let progress_page = page_for_scan.clone();
            let done_state = state_for_scan.clone();
            let done_page = page_for_scan.clone();
            let done_kind = kind_for_scan.clone();
            let done_token = token_for_scan.clone();
            let (token, _) = dispatch::dispatch_storage_scan(
                &executor,
                job_id,
                policy,
                move |progress| {
                    if progress_state
                        .borrow_mut()
                        .reduce(StorageIntent::StorageScanProgress {
                            job_id,
                            completed: progress.completed,
                            total: progress.total,
                        })
                    {
                        progress_page.overview.apply_state(&progress_state.borrow());
                    }
                },
                move |result| {
                    let accepted = match result {
                        StorageScanResult::Completed {
                            job_id,
                            large_files,
                            issues,
                        } => {
                            let mut state = done_state.borrow_mut();
                            state.reduce(StorageIntent::StorageScanCompleted {
                                job_id,
                                large_files,
                                issues,
                            }) && state
                                .reduce(StorageIntent::NavigateSubPage(StorageSubPage::LargeFiles))
                        }
                        StorageScanResult::Cancelled { job_id } => done_state
                            .borrow_mut()
                            .reduce(StorageIntent::CancelStorageScan { job_id }),
                        StorageScanResult::Failed { job_id, message } => {
                            let accepted = done_state
                                .borrow_mut()
                                .reduce(StorageIntent::CancelStorageScan { job_id });
                            if accepted {
                                done_page.overview.show_error(&message);
                            }
                            accepted
                        }
                    };
                    if accepted {
                        done_page.apply_state(&done_state.borrow());
                        done_kind.set(0);
                        done_token.borrow_mut().take();
                    }
                },
            );
            *token_for_scan.borrow_mut() = Some(token);
        });

        let runtime_for_duplicates = runtime.clone();
        let state_for_duplicates = storage_state.clone();
        let page_for_duplicates = page.clone();
        let token_for_duplicates = duplicate_token.clone();
        let kind_for_duplicates = active_kind.clone();
        storage_page.overview.connect_duplicate_scan(move || {
            if kind_for_duplicates.get() != 0 {
                return;
            }

            let (job_id, mut policy) = {
                let mut state = state_for_duplicates.borrow_mut();
                if !state.reduce(StorageIntent::StartDuplicateScan) {
                    return;
                }
                let Some(scan) = state.duplicate_scan.as_ref() else {
                    return;
                };
                (scan.job_id, scan.policy.clone())
            };

            let rt = runtime_for_duplicates.borrow();
            policy.large_file_threshold_bytes = 1;
            policy
                .exclusions
                .extend(rt.settings.scan_exclusions.iter().map(PathBuf::from));
            let executor = rt.executor.clone();
            drop(rt);

            kind_for_duplicates.set(2);
            page_for_duplicates.apply_state(&state_for_duplicates.borrow());

            let progress_state = state_for_duplicates.clone();
            let progress_page = page_for_duplicates.clone();
            let done_state = state_for_duplicates.clone();
            let done_page = page_for_duplicates.clone();
            let done_kind = kind_for_duplicates.clone();
            let done_token = token_for_duplicates.clone();
            let (token, _) = dispatch::dispatch_duplicate_scan(
                &executor,
                job_id,
                policy,
                move |progress| {
                    if progress_state
                        .borrow_mut()
                        .reduce(StorageIntent::DuplicateScanProgress {
                            job_id,
                            completed: progress.completed,
                            total: progress.total,
                        })
                    {
                        progress_page.overview.apply_state(&progress_state.borrow());
                    }
                },
                move |result| {
                    let accepted = match result {
                        DuplicateScanJobResult::Completed {
                            job_id,
                            groups,
                            issues,
                        } => {
                            let mut state = done_state.borrow_mut();
                            state.reduce(StorageIntent::DuplicateScanCompleted {
                                job_id,
                                groups,
                                issues,
                            }) && state
                                .reduce(StorageIntent::NavigateSubPage(StorageSubPage::Duplicates))
                        }
                        DuplicateScanJobResult::Cancelled { job_id } => done_state
                            .borrow_mut()
                            .reduce(StorageIntent::CancelDuplicateScan { job_id }),
                        DuplicateScanJobResult::Failed { job_id, message } => {
                            let accepted = done_state
                                .borrow_mut()
                                .reduce(StorageIntent::CancelDuplicateScan { job_id });
                            if accepted {
                                done_page.overview.show_error(&message);
                            }
                            accepted
                        }
                    };
                    if accepted {
                        done_page.apply_state(&done_state.borrow());
                        done_kind.set(0);
                        done_token.borrow_mut().take();
                    }
                },
            );
            *token_for_duplicates.borrow_mut() = Some(token);
        });

        let state_for_back = storage_state.clone();
        let page_for_back = page.clone();
        storage_page.large_files.connect_back(move || {
            if state_for_back.borrow_mut().reduce(StorageIntent::DrillUp) {
                page_for_back.apply_state(&state_for_back.borrow());
            }
        });

        let state_for_selection = storage_state.clone();
        let page_for_selection = page.clone();
        storage_page
            .duplicates
            .connect_selection_changed(move |group_idx, file_idx| {
                if state_for_selection.borrow_mut().reduce(
                    StorageIntent::ToggleDuplicateSelection {
                        group_idx,
                        file_idx,
                    },
                ) {
                    page_for_selection.apply_state(&state_for_selection.borrow());
                }
            });

        let state_for_review = storage_state.clone();
        let page_for_review = page.clone();
        storage_page.duplicates.connect_review(move || {
            if state_for_review
                .borrow_mut()
                .reduce(StorageIntent::BuildRemovalPlan)
            {
                if let Some(review) = state_for_review.borrow().removal_review.as_ref() {
                    page_for_review.duplicates.show_review_plan(review);
                }
            }
        });

        let cancel_kind = active_kind;
        storage_page
            .overview
            .connect_cancel(move || match cancel_kind.get() {
                1 => {
                    if let Some(token) = scan_token.borrow().as_ref() {
                        token.cancel();
                    }
                }
                2 => {
                    if let Some(token) = duplicate_token.borrow().as_ref() {
                        token.cancel();
                    }
                }
                _ => {}
            });
    }

    /// Wire cleanup scan, review, confirmation, and execution.
    fn wire_cleanup_dispatch(
        cleanup_page: &CleanupPage,
        runtime: &RuntimeHandle,
        parent: &ApplicationWindow,
    ) {
        use crate::dispatch::{self, CleanupExecutionOutcome, CleanupScanOutcome};
        use crate::ui::dialogs::confirm_cleanup::{ConfirmCleanupDialog, ConfirmItem};
        use maintainer_core::cleanup::CleanupPlan;

        let page = cleanup_page.clone();
        let current_plan: Rc<RefCell<Option<CleanupPlan>>> = Rc::new(RefCell::new(None));
        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));

        let cancel_token = active_token.clone();
        let cancel_status = cleanup_page.status_label_handle();
        cleanup_page.cancel_button().connect_clicked(move |_| {
            if let Some(token) = cancel_token.borrow().as_ref() { token.cancel(); }
            cancel_status.set_text("Cancellation requested. An authorized file operation already in progress may finish.");
            cancel_status.set_visible(true);
        });

        let runtime_for_scan = runtime.clone();
        let page_for_scan = page.clone();
        let plan_for_scan = current_plan.clone();

        let runtime_for_cache = runtime.clone();
        let cache_status = cleanup_page.status_label_handle();
        cleanup_page
            .package_cache_button()
            .connect_clicked(move |btn| {
                btn.set_sensitive(false);
                btn.set_label("Cleaning Package Cache…");
                cache_status.remove_css_class("error");
                cache_status.set_text("Requesting authorization to clean package cache…");
                cache_status.set_visible(true);
                let rt = runtime_for_cache.borrow();
                let helper = rt.helper.clone();
                let executor = rt.executor.clone();
                drop(rt);
                let status = cache_status.clone();
                let btn_done = btn.clone();
                let request_id = format!(
                    "cache-{}-{}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::SystemTime::UNIX_EPOCH)
                        .map(|duration| duration.as_secs())
                        .unwrap_or(0)
                );
                dispatch::dispatch_helper_call(
                    &executor,
                    move || helper.clean_package_cache(&request_id),
                    move |result| {
                        btn_done.set_sensitive(true);
                        btn_done.set_label("Clean Package Cache");
                        Self::show_helper_result(&status, result);
                    },
                );
            });

        let runtime_for_journal = runtime.clone();
        let journal_status = cleanup_page.status_label_handle();
        cleanup_page
            .journal_vacuum_button()
            .connect_clicked(move |btn| {
                btn.set_sensitive(false);
                btn.set_label("Vacuuming Journal…");
                journal_status.remove_css_class("error");
                journal_status.set_text(
                    "Requesting authorization to retain 500 MiB / 30 days of journal data…",
                );
                journal_status.set_visible(true);
                let rt = runtime_for_journal.borrow();
                let helper = rt.helper.clone();
                let executor = rt.executor.clone();
                drop(rt);
                let status = journal_status.clone();
                let btn_done = btn.clone();
                dispatch::dispatch_helper_call(
                    &executor,
                    move || helper.vacuum_journal(500, 30),
                    move |result| {
                        btn_done.set_sensitive(true);
                        btn_done.set_label("Vacuum Journal");
                        Self::show_helper_result(&status, result);
                    },
                );
            });
        let recovery_runtime = runtime.clone();
        let recovery_parent = parent.clone();
        let recovery_status = cleanup_page.status_label_handle();
        cleanup_page.recovery_button().connect_clicked(move |button| {
            button.set_sensitive(false);
            recovery_status.set_text("Requesting authorization to inspect retained cleanup entries…");
            recovery_status.set_visible(true);
            let executor = recovery_runtime.borrow().executor.clone();
            let button = button.clone();
            let parent = recovery_parent.clone();
            let status = recovery_status.clone();
            let restore_executor = executor.clone();
            dispatch::dispatch_cleanup_recovery(&executor, None, move |outcome| {
                button.set_sensitive(true);
                match outcome {
                    dispatch::CleanupRecoveryOutcome::Listed(items) => {
                        status.set_text(&format!("{} retained cleanup entries", items.len()));
                        let dialog = gtk4::Window::builder().title("Cleanup Recovery").transient_for(&parent).modal(true).default_width(600).default_height(360).build();
                        let content = gtk4::Box::new(gtk4::Orientation::Vertical, 12);
                        content.set_margin_top(18); content.set_margin_bottom(18);
                        content.set_margin_start(18); content.set_margin_end(18);
                        let explanation = gtk4::Label::new(Some("Restore returns an entry to its original folder. If that folder is gone, choose Restore elsewhere. The original name is retained and nothing is overwritten. Select a folder on the same filesystem."));
                        explanation.set_wrap(true); content.append(&explanation);
                        if items.is_empty() { content.append(&gtk4::Label::new(Some("No cleanup recovery is needed."))); }
                        for item in items {
                            let row = gtk4::Box::new(gtk4::Orientation::Horizontal, 12);
                            let label = gtk4::Label::new(Some(&format!("{}  [{}]", item.basename, item.id)));
                            label.set_hexpand(true); label.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                            let restore = gtk4::Button::with_label("Restore");
                            let elsewhere = gtk4::Button::with_label("Restore elsewhere…");
                            let elsewhere_item = item.clone();
                            let executor = restore_executor.clone();
                            let alternate = elsewhere.clone();
                            restore.connect_clicked(move |button| {
                                button.set_sensitive(false); alternate.set_sensitive(false);
                                let button = button.clone(); let alternate = alternate.clone();
                                dispatch::dispatch_cleanup_recovery(&executor, Some((item.clone(), None)), move |outcome| {
                                    match outcome {
                                        dispatch::CleanupRecoveryOutcome::Restored { notice } => {
                                            button.set_label(if notice.is_some() { "Restored; durability unconfirmed" } else { "Restored" });
                                            button.set_tooltip_text(notice.as_deref());
                                        },
                                        dispatch::CleanupRecoveryOutcome::Failed(message) => {
                                            button.set_label("Retry Restore"); button.set_sensitive(true); alternate.set_sensitive(true);
                                            button.set_tooltip_text(Some(&message));
                                        }
                                        _ => {}
                                    }
                                });
                            });
                            let executor = restore_executor.clone();
                            let original = restore.clone();
                            let recovery_dialog = dialog.clone();
                            elsewhere.connect_clicked(move |button| {
                                button.set_sensitive(false); original.set_sensitive(false);
                                let chooser = gtk4::FileChooserDialog::new(Some("Restore Retained Entry Elsewhere"), Some(&recovery_dialog), gtk4::FileChooserAction::SelectFolder, &[("Cancel", gtk4::ResponseType::Cancel), ("Restore Here", gtk4::ResponseType::Accept)]);
                                let button = button.clone(); let original = original.clone(); let item = elsewhere_item.clone(); let executor = executor.clone();
                                chooser.connect_response(move |chooser, response| {
                                    let destination = if response == gtk4::ResponseType::Accept { chooser.file().and_then(|file| file.path()) } else { None };
                                    chooser.close();
                                    let Some(destination) = destination else { button.set_sensitive(true); original.set_sensitive(true); return; };
                                    let button = button.clone(); let original = original.clone();
                                    dispatch::dispatch_cleanup_recovery(&executor, Some((item.clone(), Some(destination))), move |outcome| {
                                        match outcome {
                                            dispatch::CleanupRecoveryOutcome::Restored { notice } => {
                                                button.set_label(if notice.is_some() { "Restored; durability unconfirmed" } else { "Restored elsewhere" }); button.set_tooltip_text(notice.as_deref());
                                            }
                                            dispatch::CleanupRecoveryOutcome::Failed(message) => { button.set_sensitive(true); original.set_sensitive(true); button.set_tooltip_text(Some(&message)); }
                                            _ => {}
                                        }
                                    });
                                });
                                chooser.present();
                            });
                            row.append(&label); row.append(&restore); row.append(&elsewhere); content.append(&row);
                        }
                        let scrolled = gtk4::ScrolledWindow::new(); scrolled.set_child(Some(&content));
                        dialog.set_child(Some(&scrolled)); dialog.present();
                    }
                    dispatch::CleanupRecoveryOutcome::Failed(message) => status.set_text(&message),
                    _ => {}
                }
            });
        });
        let token_for_scan = active_token.clone();
        cleanup_page.scan_button().connect_clicked(move |_| {
            page_for_scan.show_executing("Scanning reclaimable files…");
            plan_for_scan.borrow_mut().take();
            let executor = runtime_for_scan.borrow().executor.clone();
            let progress_page = page_for_scan.clone();
            let done_page = page_for_scan.clone();
            let done_plan = plan_for_scan.clone();
            let done_token = token_for_scan.clone();
            let (token, _) = dispatch::dispatch_cleanup_scan(
                &executor,
                move |progress| {
                    progress_page.show_scan_progress(
                        progress.completed,
                        progress.total,
                        &progress.message,
                    );
                },
                move |outcome| {
                    match outcome {
                        CleanupScanOutcome::Completed { plan } => {
                            done_page.show_plan(&plan);
                            *done_plan.borrow_mut() = Some(plan);
                        }
                        CleanupScanOutcome::Cancelled => {
                            done_page.show_result("Scan cancelled. No files were changed.", false)
                        }
                        CleanupScanOutcome::Failed { message } => done_page.show_error(&message),
                    }
                    done_token.borrow_mut().take();
                },
            );
            *token_for_scan.borrow_mut() = Some(token);
        });

        let runtime_for_execute = runtime.clone();
        let page_for_execute = page.clone();
        let plan_for_execute = current_plan.clone();
        let parent_for_execute = parent.clone();
        let token_for_execute = active_token.clone();
        cleanup_page.connect_execute(move || {
            let selected_indices = page_for_execute.selected_indices();
            if selected_indices.is_empty() {
                return;
            }
            let Some(plan) = plan_for_execute.borrow().clone() else {
                page_for_execute.show_error("Cleanup plan expired. Scan again.");
                return;
            };

            let items: Vec<ConfirmItem> = selected_indices
                .iter()
                .filter_map(|index| plan.items.get(*index))
                .map(|item| ConfirmItem {
                    display_path: item.display_path.clone(),
                    size_bytes: item.size_bytes,
                    is_permanent: matches!(
                        item.reversibility,
                        maintainer_core::cleanup::Reversibility::PermanentOnly
                    ),
                })
                .collect();
            let total_bytes = items.iter().map(|item| item.size_bytes).sum();
            let has_permanent = items.iter().any(|item| item.is_permanent);
            let confirmation = ConfirmCleanupDialog::new(&items, total_bytes, has_permanent);
            let dialog = gtk4::Window::builder()
                .title("Confirm Cleanup")
                .transient_for(&parent_for_execute)
                .modal(true)
                .default_width(560)
                .default_height(480)
                .build();
            dialog.add_css_class("maintainer_dialog");
            dialog.set_child(Some(confirmation.widget()));

            let cancel_dialog = dialog.clone();
            confirmation
                .cancel_button()
                .connect_clicked(move |_| cancel_dialog.close());

            let confirm_dialog = dialog.clone();
            let confirm_page = page_for_execute.clone();
            let confirm_runtime = runtime_for_execute.clone();
            let confirm_token = token_for_execute.clone();
            let confirm_plan = plan.clone();
            let release_plan = plan_for_execute.clone();
            let confirm_indices = selected_indices.clone();
            confirmation.confirm_button().connect_clicked(move |_| {
                confirm_dialog.close();
                release_plan.borrow_mut().take();
                let rt = confirm_runtime.borrow();
                let Some(database) = rt.db.clone() else {
                    confirm_page.show_error("Cleanup history database is unavailable.");
                    return;
                };
                let executor = rt.executor.clone();
                drop(rt);

                confirm_page.show_executing("Removing selected items…");
                let progress_page = confirm_page.clone();
                let done_page = confirm_page.clone();
                let done_token = confirm_token.clone();
                let (token, _) = dispatch::dispatch_cleanup_execution(
                    &executor,
                    confirm_plan.clone(),
                    confirm_indices.clone(),
                    database,
                    move |progress| {
                        progress_page.show_executing(&progress.message);
                    },
                    move |outcome| {
                        match outcome {
                            CleanupExecutionOutcome::Completed(result) => {
                                let summary = format!(
                                    "Removed {} item(s); physical reclaimed space measured: {}.",
                                    result.outcome.succeeded,
                                    Self::format_size(result.actual_recovered_bytes)
                                );
                                let failures = result
                                    .item_results
                                    .iter()
                                    .filter_map(|item| match item {
                                        maintainer_core::jobs::ItemResult::Failed {
                                            message,
                                            ..
                                        } => Some(message.as_str()),
                                        maintainer_core::jobs::ItemResult::Ok(item) => {
                                            item.notice.as_deref()
                                        }
                                        _ => None,
                                    })
                                    .take(4)
                                    .collect::<Vec<_>>()
                                    .join("\n");
                                let summary = if failures.is_empty() {
                                    summary
                                } else {
                                    format!("{summary}\n{failures}")
                                };
                                done_page.show_result(&summary, result.outcome.failed == 0);
                            }
                            CleanupExecutionOutcome::Failed { message } => {
                                done_page.show_result(&message, false);
                            }
                        }
                        done_token.borrow_mut().take();
                    },
                );
                *confirm_token.borrow_mut() = Some(token);
            });
            dialog.present();
        });
    }

    /// Wire update discovery, review, and helper-authorized APT execution.
    fn wire_updates_dispatch(updates_page: &UpdatesPage, runtime: &RuntimeHandle) {
        use crate::dispatch::{self, UpdatesCheckOutcome};
        use crate::ui::updates::{
            SelectionMode, filter_candidates_for_mode, format_filtered_summary,
        };
        use maintainer_core::updates::UpdatePlan;

        let page = updates_page.clone();
        let current_plan: Rc<RefCell<Option<UpdatePlan>>> = Rc::new(RefCell::new(None));
        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));

        let workflow = crate::package_workflow::PackageWorkflow::new(
            updates_page,
            runtime.borrow().executor.clone(),
        );

        // --- Wire selection mode toggles ---
        let plan_for_everything = current_plan.clone();
        let page_for_everything = page.clone();
        updates_page
            .everything_toggle()
            .connect_toggled(move |btn| {
                if btn.is_active() {
                    let mut p = page_for_everything.clone();
                    p.set_selection_mode(SelectionMode::Everything);
                    let needs_full_review =
                        plan_for_everything.borrow().as_ref().is_some_and(|plan| {
                            matches!(
                                plan.intent,
                                maintainer_core::updates::UpdateIntent::Selected(_)
                            )
                        });
                    if needs_full_review {
                        p.check_button().emit_clicked();
                        return;
                    }
                    if let Some(plan) = plan_for_everything.borrow().as_ref() {
                        let filtered =
                            filter_candidates_for_mode(&plan.candidates, SelectionMode::Everything);
                        let summary = format_filtered_summary(
                            &filtered,
                            plan.removals.len(),
                            plan.held.len(),
                        );
                        let owned: Vec<_> = filtered.into_iter().cloned().collect();
                        p.populate_package_list(&owned);
                        p.update_summary_text(&summary);
                    }
                }
            });

        let plan_for_security = current_plan.clone();
        let page_for_security = page.clone();
        updates_page.security_toggle().connect_toggled(move |btn| {
            if btn.is_active() {
                let mut p = page_for_security.clone();
                p.set_selection_mode(SelectionMode::SecurityOnly);
                if let Some(plan) = plan_for_security.borrow().as_ref() {
                    let filtered =
                        filter_candidates_for_mode(&plan.candidates, SelectionMode::SecurityOnly);
                    let summary =
                        format_filtered_summary(&filtered, plan.removals.len(), plan.held.len());
                    let owned: Vec<_> = filtered.into_iter().cloned().collect();
                    p.populate_package_list(&owned);
                    p.update_summary_text(&summary);
                }
                // Resolve the security request before enabling installation; the
                // displayed subset is not itself an executable reviewed plan.
                p.confirm_button().set_sensitive(false);
                p.review_button().emit_clicked();
            }
        });

        // --- Wire Review Selection button (re-plan through apt for custom selection) ---
        let runtime_for_replan = runtime.clone();
        let page_for_replan = page.clone();
        let plan_for_replan = current_plan.clone();
        let token_for_replan = active_token.clone();
        updates_page.review_button().connect_clicked(move |_| {
            let Some(full_plan) = plan_for_replan.borrow().clone() else {
                let mut ui = page_for_replan.clone();
                ui.set_check_failed("Update plan expired. Check for updates again.");
                return;
            };

            // Collect the selected package names and find their candidates.
            let selected_names = page_for_replan.selected_packages();
            let selected_candidates: Vec<_> = full_plan
                .candidates
                .iter()
                .filter(|c| selected_names.contains(&c.selection_key()))
                .cloned()
                .collect();

            if selected_candidates.is_empty() {
                page_for_replan.update_summary_text("No packages selected.");
                return;
            }

            // Invalidate the old plan while the resolver reviews the request.
            plan_for_replan.borrow_mut().take();
            let mut starting_page = page_for_replan.clone();
            starting_page.set_checking();
            // Dispatch re-planning through apt's resolver on a background thread.
            let executor = runtime_for_replan.borrow().executor.clone();
            let mut done_page = page_for_replan.clone();
            let done_plan = plan_for_replan.clone();
            let done_token = token_for_replan.clone();

            let (token, _) = dispatch::dispatch_replan_selection(
                &executor,
                selected_candidates,
                move |_progress| {
                    // Re-planning is typically fast; no granular progress needed.
                },
                move |outcome| {
                    match outcome {
                        UpdatesCheckOutcome::Available {
                            mut summary,
                            security_count,
                            total_count,
                            plan,
                        } => {
                            if let Some(security) = crate::ui::updates::format_security_label(
                                security_count,
                                total_count,
                            ) {
                                summary.push_str(&format!(". {security}."));
                            }
                            done_page.set_reviewing_plan(&summary, &plan);
                            // Stay in Custom mode but mark as replanned so
                            // Install becomes submittable with this plan.
                            done_page.set_selection_mode(SelectionMode::Custom);
                            done_page.mark_selection_replanned();
                            *done_plan.borrow_mut() = Some(plan);
                        }
                        UpdatesCheckOutcome::UpToDate => done_page.set_up_to_date(),
                        UpdatesCheckOutcome::Failed { message } => {
                            done_page.set_check_failed(&message)
                        }
                    }
                    done_token.borrow_mut().take();
                },
            );
            *token_for_replan.borrow_mut() = Some(token);
        });

        // --- Wire check button ---
        let runtime_for_check = runtime.clone();
        let page_for_check = page.clone();
        let plan_for_check = current_plan.clone();
        let token_for_check = active_token.clone();
        updates_page.check_button().connect_clicked(move |_| {
            let mut starting_page = page_for_check.clone();
            starting_page.set_checking();
            plan_for_check.borrow_mut().take();
            let executor = runtime_for_check.borrow().executor.clone();
            let progress_page = page_for_check.clone();
            let mut done_page = page_for_check.clone();
            let done_plan = plan_for_check.clone();
            let done_token = token_for_check.clone();
            let (token, _) = dispatch::dispatch_updates_check(
                &executor,
                move |progress| {
                    progress_page.show_check_progress(
                        progress.completed,
                        progress.total,
                        &progress.message,
                    );
                },
                move |outcome| {
                    match outcome {
                        UpdatesCheckOutcome::Available {
                            mut summary,
                            security_count,
                            total_count,
                            plan,
                        } => {
                            if let Some(security) = crate::ui::updates::format_security_label(
                                security_count,
                                total_count,
                            ) {
                                summary.push_str(&format!(". {security}."));
                            }
                            done_page.set_selection_mode(SelectionMode::Everything);
                            done_page.set_reviewing_plan(&summary, &plan);
                            *done_plan.borrow_mut() = Some(plan);
                        }
                        UpdatesCheckOutcome::UpToDate => done_page.set_up_to_date(),
                        UpdatesCheckOutcome::Failed { message } => {
                            done_page.set_check_failed(&message)
                        }
                    }
                    done_token.borrow_mut().take();
                },
            );
            *token_for_check.borrow_mut() = Some(token);
        });

        // Authentication obtains a retained native plan; only its separate
        // complete-plan dialog can send Approve with the bound fingerprint.
        let page_for_execute = page.clone();
        let plan_for_execute = current_plan;
        updates_page.confirm_button().connect_clicked(move |_| {
            let Some(plan) = plan_for_execute.borrow().clone() else {
                return;
            };
            if !crate::dispatch::update_selection_reviewed(
                page_for_execute.selection_mode(),
                page_for_execute.is_selection_replanned(),
                &page_for_execute.selected_packages(),
                &plan,
            ) {
                let mut page = page_for_execute.clone();
                page.set_check_failed(
                    "Selection requires re-planning. Use Review Selection first.",
                );
                return;
            }
            match plan.intent {
                maintainer_core::updates::UpdateIntent::FullUpgrade => {
                    workflow.begin("full", vec![])
                }
                maintainer_core::updates::UpdateIntent::Selected(requests) => workflow.begin(
                    "selected",
                    requests
                        .iter()
                        .map(|request| request.package_spec())
                        .collect(),
                ),
            }
        });
    }

    /// Wire diagnostics collection and render evidence-first findings.
    fn wire_diagnostics_dispatch(diagnostics_page: &DiagnosticsPage, runtime: &RuntimeHandle) {
        use crate::dispatch::{self, DiagnosticsScanOutcome, SmartRefreshOutcome};
        use crate::ui::diagnostics::{DiagnosticsIntent, DiagnosticsState};

        let page = diagnostics_page.clone();
        let state = Rc::new(RefCell::new(DiagnosticsState::default()));
        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));
        let runtime_for_scan = runtime.clone();
        let state_for_scan = state.clone();
        let page_for_scan = page.clone();
        let token_for_scan = active_token.clone();

        let state_for_expand = state.clone();
        let page_for_expand = page.clone();
        diagnostics_page.connect_toggle_expanded(move |index| {
            state_for_expand
                .borrow_mut()
                .reduce(DiagnosticsIntent::ToggleExpanded(index));
            page_for_expand.update(&state_for_expand.borrow());
        });

        diagnostics_page.scan_button().connect_clicked(move |_| {
            state_for_scan
                .borrow_mut()
                .reduce(DiagnosticsIntent::StartScan);
            page_for_scan.update(&state_for_scan.borrow());
            let executor = runtime_for_scan.borrow().executor.clone();
            let progress_page = page_for_scan.clone();
            let done_state = state_for_scan.clone();
            let done_page = page_for_scan.clone();
            let done_token = token_for_scan.clone();
            let (token, _) = dispatch::dispatch_diagnostics_scan(
                &executor,
                move |progress| {
                    progress_page.show_scan_progress(
                        progress.completed,
                        progress.total,
                        &progress.message,
                    );
                },
                move |outcome| {
                    match outcome {
                        DiagnosticsScanOutcome::Completed { findings } => done_state
                            .borrow_mut()
                            .reduce(DiagnosticsIntent::ScanCompleted(findings)),
                        DiagnosticsScanOutcome::Failed { message } => done_state
                            .borrow_mut()
                            .reduce(DiagnosticsIntent::ScanFailed(message)),
                    }
                    done_page.update(&done_state.borrow());
                    done_token.borrow_mut().take();
                },
            );
            *token_for_scan.borrow_mut() = Some(token);
        });

        let runtime_for_smart = runtime.clone();
        let state_for_smart = state.clone();
        let page_for_smart = page.clone();
        let token_for_smart = active_token;
        diagnostics_page
            .smart_refresh_button()
            .connect_clicked(move |_| {
                state_for_smart
                    .borrow_mut()
                    .reduce(DiagnosticsIntent::RefreshSmart);
                page_for_smart.update(&state_for_smart.borrow());
                let rt = runtime_for_smart.borrow();
                let executor = rt.executor.clone();
                let helper = rt.helper.clone();
                drop(rt);
                let done_state = state_for_smart.clone();
                let done_page = page_for_smart.clone();
                let done_token = token_for_smart.clone();
                let token = dispatch::dispatch_smart_refresh(&executor, helper, move |outcome| {
                    match outcome {
                        SmartRefreshOutcome::Loaded(drives) => {
                            let views = drives
                                .iter()
                                .map(crate::ui::diagnostics::SmartDriveView::from_smart_drive)
                                .collect();
                            done_state
                                .borrow_mut()
                                .reduce(DiagnosticsIntent::SmartLoaded(views));
                        }
                        SmartRefreshOutcome::PermissionRequired => {
                            done_state
                                .borrow_mut()
                                .reduce(DiagnosticsIntent::SmartPermissionRequired);
                        }
                        SmartRefreshOutcome::Unavailable(reason) => {
                            done_state
                                .borrow_mut()
                                .reduce(DiagnosticsIntent::SmartUnavailable(reason));
                        }
                    }
                    done_page.update(&done_state.borrow());
                    done_token.borrow_mut().take();
                });
                *token_for_smart.borrow_mut() = Some(token);
            });
    }

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
}
