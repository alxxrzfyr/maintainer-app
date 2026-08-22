//! Application shell: main window layout with navigation and page stack.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use glib::ControlFlow;
use gtk4::prelude::*;
use gtk4::{
    Align, ApplicationWindow, Box as GtkBox, Button, Image, Label, MenuButton, Orientation,
    Popover, Stack, StackTransitionType,
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
        window.set_size_request(360, 480);

        let overview_page = OverviewPage::new();
        let system_page = SystemPage::new();
        let storage_page = StoragePage::new();
        let cleanup_page = CleanupPage::new();
        let updates_page = UpdatesPage::new();
        let diagnostics_page = DiagnosticsPage::new();
        let reports_widget = build_reports_page(runtime.clone());

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
                let mut settings_state =
                    crate::state::SettingsState::from_snapshot(rt.settings.clone());
                if settings_state.reduce(intent) {
                    rt.settings = settings_state.snapshot.clone();
                    if let Some(db) = rt.database() {
                        let repo = maintainer_core::settings::SqliteSettingsRepository::new(db);
                        let core_settings = to_core_settings(&settings_state.snapshot);
                        if let Err(error) = repo.save(&core_settings) {
                            eprintln!("warning: failed to save settings: {error}");
                        }
                    }
                    drop(rt);
                    if let Some(theme) = requested_theme {
                        crate::application::apply_theme(theme);
                    }
                }
            }
        });

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

        let runtime_for_sort = runtime.clone();
        let system_for_sort = system_ref.clone();
        system_ref.connect_sort_changed(move |sort| {
            let mut rt = runtime_for_sort.borrow_mut();
            rt.monitoring
                .reduce(MonitoringMessage::SetProcessSort(sort));
            system_for_sort.update(&rt.monitoring.system);
        });

        let runtime_for_terminate = runtime.clone();
        let terminate_status = system_ref.operation_status_handle();
        system_ref.connect_terminate(move |pid, start_time_ticks| {
            terminate_status.remove_css_class("error");
            terminate_status.set_text("Requesting authorization…");
            terminate_status.set_visible(true);

            let rt = runtime_for_terminate.borrow();
            let helper = rt.helper.clone();
            let executor = rt.executor.clone();
            drop(rt);
            let status = terminate_status.clone();
            crate::dispatch::dispatch_helper_call(
                &executor,
                move || helper.terminate_process(pid, start_time_ticks),
                move |result| Self::show_helper_result(&status, result),
            );
        });
        Self::wire_storage_dispatch(&storage_page, &runtime);
        Self::wire_cleanup_dispatch(&cleanup_page, &runtime, &window);
        Self::wire_updates_dispatch(&updates_page, &runtime);
        Self::wire_diagnostics_dispatch(&diagnostics_page, &runtime);

        let runtime_for_timer = runtime.clone();
        let overview_for_timer = overview_ref.clone();
        let system_for_timer = system_ref.clone();

        let runtime_for_vis = runtime.clone();
        window.connect_is_active_notify(move |win| {
            let visible = win.is_active();
            let mut rt = runtime_for_vis.borrow_mut();
            rt.monitoring
                .reduce(MonitoringMessage::VisibilityChanged(visible));
        });

        Self::start_monitoring_timer(
            runtime_for_timer,
            overview_for_timer,
            system_for_timer,
            page_stack.clone(),
        );

        Shell { window }
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

            let runtime_for_done = runtime_for_response.clone();
            let parent_for_done = parent_for_response.clone();
            crate::dispatch::dispatch_local_action(
                &executor,
                move || {
                    let result = if delete_all_data {
                        database.clear_user_data()
                    } else {
                        database.clear_history()
                    };
                    result.map_err(|error| error.to_string())
                },
                move |result| match result {
                    Ok(deleted) => {
                        if delete_all_data {
                            let defaults = crate::state::SettingsSnapshot::default();
                            runtime_for_done.borrow_mut().settings = defaults.clone();
                            if let Some(database) = runtime_for_done.borrow().db.as_deref() {
                                let repository =
                                    maintainer_core::settings::SqliteSettingsRepository::new(database);
                                if let Err(error) = repository.save(&to_core_settings(&defaults)) {
                                    Self::show_settings_result(
                                        &parent_for_done,
                                        false,
                                        &format!("Data was cleared, but defaults could not be saved: {error}"),
                                    );
                                    return;
                                }
                            }
                        }
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
        page_stack: Stack,
    ) {
        let tick_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        let (snapshot_tx, snapshot_rx) =
            std::sync::mpsc::channel::<Box<maintainer_core::metrics::SystemSnapshot>>();

        // Bounded request channel (capacity=1) provides tick coalescing.
        // reuses the last collected process list instead of re-reading all of
        // /proc/[pid], which is the single most expensive collector per tick.
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<bool>(1);

        std::thread::Builder::new()
            .name("metric-worker".into())
            .spawn(move || {
                use maintainer_core::capability::{Capability, UnavailableReason};
                use maintainer_core::metrics::disk::read_mounts_with_capacity;
                use maintainer_core::metrics::network::NetCounters;
                use maintainer_core::metrics::{
                    CpuSample, NetworkSnapshot, ProcessSnapshot, SystemSnapshot,
                    compute_cpu_snapshot, read_cpu_sample, read_memory, read_net_dev,
                    read_processes,
                };
                use std::collections::HashMap;
                use std::path::Path;
                use std::time::Instant;

                let proc_path = Path::new("/proc");
                let sys_path = Path::new("/sys");
                let mut prev_cpu: Option<CpuSample> = None;
                let mut prev_net: Option<(HashMap<String, NetCounters>, Instant)> = None;
                let mut last_processes: Option<Capability<Vec<ProcessSnapshot>>> = None;
                let mut temperature_collector =
                    maintainer_core::metrics::TemperatureCollector::new();

                while let Ok(mut needs_processes) = request_rx.recv() {
                    // Drain coalesced ticks; keep the most recent request's flag.
                    while let Ok(latest) = request_rx.try_recv() {
                        needs_processes = latest;
                    }

                    let now = Instant::now();

                    let cpu = match read_cpu_sample(proc_path, now) {
                        Ok(sample) => {
                            let snapshot = match &prev_cpu {
                                Some(prev) => compute_cpu_snapshot(prev, &sample),
                                None => maintainer_core::metrics::CpuSnapshot {
                                    total_percent: 0.0,
                                    per_core_percent: vec![0.0; sample.cores.len()],
                                },
                            };
                            prev_cpu = Some(sample);
                            Capability::Available(snapshot)
                        }
                        Err(e) => Capability::Unavailable(UnavailableReason {
                            code: "proc-stat-read",
                            message: e.to_string(),
                            remediation: None,
                        }),
                    };

                    let memory = match read_memory(proc_path) {
                        Ok(m) => Capability::Available(m),
                        Err(e) => Capability::Unavailable(UnavailableReason {
                            code: "meminfo-read",
                            message: e.to_string(),
                            remediation: None,
                        }),
                    };

                    let disks = match read_mounts_with_capacity(proc_path) {
                        Ok(d) => Capability::Available(d),
                        Err(e) => Capability::Unavailable(UnavailableReason {
                            code: "mounts-read",
                            message: e.to_string(),
                            remediation: None,
                        }),
                    };

                    let network = match read_net_dev(proc_path) {
                        Ok(counters) => {
                            let prev = prev_net.take();
                            let snapshots: Vec<NetworkSnapshot> = counters
                                .iter()
                                .map(|(iface, c)| {
                                    let (rx_rate, tx_rate) = match &prev {
                                        Some((prev_counters, prev_time)) => {
                                            let elapsed =
                                                now.duration_since(*prev_time).as_secs_f64();
                                            if elapsed > 0.0 {
                                                if let Some(pc) = prev_counters.get(iface) {
                                                    let rx =
                                                        (c.rx_bytes.saturating_sub(pc.rx_bytes))
                                                            as f64
                                                            / elapsed;
                                                    let tx =
                                                        (c.tx_bytes.saturating_sub(pc.tx_bytes))
                                                            as f64
                                                            / elapsed;
                                                    (Some(rx), Some(tx))
                                                } else {
                                                    (None, None)
                                                }
                                            } else {
                                                (None, None)
                                            }
                                        }
                                        None => (None, None),
                                    };
                                    NetworkSnapshot {
                                        interface: iface.clone(),
                                        is_loopback: iface == "lo",
                                        rx_bytes: c.rx_bytes,
                                        tx_bytes: c.tx_bytes,
                                        rx_packets: c.rx_packets,
                                        tx_packets: c.tx_packets,
                                        rx_errors: c.rx_errors,
                                        tx_errors: c.tx_errors,
                                        rx_bytes_per_sec: rx_rate,
                                        tx_bytes_per_sec: tx_rate,
                                    }
                                })
                                .collect();
                            prev_net = Some((counters, now));
                            Capability::Available(snapshots)
                        }
                        Err(e) => Capability::Unavailable(UnavailableReason {
                            code: "net-dev-read",
                            message: e.to_string(),
                            remediation: None,
                        }),
                    };

                    let temperatures = match temperature_collector.read(sys_path) {
                        Ok(t) => Capability::Available(t),
                        Err(e) => Capability::Unavailable(UnavailableReason {
                            code: "hwmon-read",
                            message: e.to_string(),
                            remediation: None,
                        }),
                    };

                    let processes = if needs_processes {
                        let result = match read_processes(proc_path) {
                            Ok(p) => Capability::Available(p),
                            Err(e) => Capability::Unavailable(UnavailableReason {
                                code: "proc-read",
                                message: e.to_string(),
                                remediation: None,
                            }),
                        };
                        last_processes = Some(result.clone());
                        result
                    } else {
                        last_processes.clone().unwrap_or(Capability::Unavailable(
                            UnavailableReason {
                                code: "proc-not-requested",
                                message: "Process list not requested for this sample".into(),
                                remediation: None,
                            },
                        ))
                    };

                    let snapshot = SystemSnapshot {
                        cpu,
                        memory,
                        disks,
                        network,
                        temperatures,
                        processes,
                    };

                    if snapshot_tx.send(Box::new(snapshot)).is_err() {
                        break;
                    }
                }
            })
            .expect("failed to spawn metric-worker thread");

        // Refresh a page as soon as it becomes visible. The timer only updates
        // the page on screen, so without this a page switched to would show
        // whatever it rendered when it was last visible until the next tick.
        {
            let runtime_for_switch = runtime.clone();
            let overview_for_switch = overview.clone();
            let system_for_switch = system.clone();
            page_stack.connect_visible_child_name_notify(move |stack| {
                let rt = runtime_for_switch.borrow();
                match stack.visible_child_name().as_deref() {
                    Some("Overview") => overview_for_switch.update(&rt.monitoring.overview),
                    Some("System") => system_for_switch.update(&rt.monitoring.system),
                    _ => {}
                }
            });
        }

        // Timer tick: request samples and drain results.
        glib::timeout_add_local(Duration::from_secs(1), move || {
            let mut rt = runtime.borrow_mut();
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
            while let Ok(snapshot) = snapshot_rx.try_recv() {
                rt.monitoring
                    .reduce(MonitoringMessage::SnapshotReady(snapshot));
                match visible_page {
                    Some("Overview") => overview.update(&rt.monitoring.overview),
                    Some("System") => system.update(&rt.monitoring.system),
                    _ => {}
                }
            }

            if !should_sample {
                return ControlFlow::Continue;
            }

            *ticks = 0;
            rt.monitoring.immediate_sample_pending = false;

            // Non-blocking: if worker is busy, this tick coalesces.
            let needs_processes = page_stack.visible_child_name().as_deref() == Some("System");
            let _ = request_tx.try_send(needs_processes);

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
        storage_page
            .stack_handle()
            .connect_visible_child_name_notify(move |stack| {
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
                        CleanupScanOutcome::Completed { targets } => {
                            let home = std::env::var("HOME")
                                .map(PathBuf::from)
                                .unwrap_or_else(|_| PathBuf::from("/tmp"));
                            match CleanupPlan::from_targets(targets, &home) {
                                Ok(plan) => {
                                    done_page.show_plan(&plan);
                                    *done_plan.borrow_mut() = Some(plan);
                                }
                                Err(error) => done_page.show_error(&error.to_string()),
                            }
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
            let confirm_indices = selected_indices.clone();
            confirmation.confirm_button().connect_clicked(move |_| {
                confirm_dialog.close();
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
                                    "Removed {} item(s), recovered {}.",
                                    result.outcome.succeeded,
                                    Self::format_size(result.actual_recovered_bytes)
                                );
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

    /// Wire update discovery, review, and PackageKit execution.
    fn wire_updates_dispatch(updates_page: &UpdatesPage, runtime: &RuntimeHandle) {
        use crate::dispatch::{self, UpdateExecutionOutcome, UpdatesCheckOutcome};
        use crate::ui::updates::{
            SelectionMode, filter_candidates_for_mode, format_filtered_summary,
        };
        use maintainer_core::updates::UpdatePlan;

        let page = updates_page.clone();
        let current_plan: Rc<RefCell<Option<UpdatePlan>>> = Rc::new(RefCell::new(None));
        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));

        // --- Wire selection mode toggles ---
        let plan_for_everything = current_plan.clone();
        let page_for_everything = page.clone();
        updates_page
            .everything_toggle()
            .connect_toggled(move |btn| {
                if btn.is_active() {
                    let mut p = page_for_everything.clone();
                    p.set_selection_mode(SelectionMode::Everything);
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
                .filter(|c| selected_names.contains(&c.name))
                .cloned()
                .collect();

            if selected_candidates.is_empty() {
                page_for_replan.update_summary_text("No packages selected.");
                return;
            }

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

        // --- Wire confirm (Install) button ---
        let runtime_for_execute = runtime.clone();
        let page_for_execute = page.clone();
        let plan_for_execute = current_plan.clone();
        let token_for_execute = active_token.clone();
        updates_page.confirm_button().connect_clicked(move |_| {
            let Some(full_plan) = plan_for_execute.borrow().clone() else {
                let mut ui = page_for_execute.clone();
                ui.set_check_failed("Update plan expired. Check for updates again.");
                return;
            };
            // Apply selection mode filter to the plan before execution.
            let mode = page_for_execute.selection_mode();
            let plan = match mode {
                SelectionMode::Everything => full_plan,
                SelectionMode::SecurityOnly => {
                    let filtered = filter_candidates_for_mode(
                        &full_plan.candidates,
                        SelectionMode::SecurityOnly,
                    );
                    let filtered_candidates: Vec<_> = filtered.into_iter().cloned().collect();
                    UpdatePlan {
                        candidates: filtered_candidates,
                        ..full_plan
                    }
                }
                SelectionMode::Custom => {
                    // Safety: in Custom mode, execution is only allowed after
                    // the user's selection has been re-planned through apt's
                    // resolver (plan_for_selection). The re-planned result
                    // replaces `current_plan`, so we use it directly.
                    if !page_for_execute.is_selection_replanned() {
                        // This should not happen because the Install button is
                        // disabled until re-planning completes, but guard anyway.
                        let mut ui = page_for_execute.clone();
                        ui.set_check_failed(
                            "Custom selection requires re-planning. \
                             Use 'Review Selection' first.",
                        );
                        return;
                    }
                    full_plan
                }
            };
            let executor = runtime_for_execute.borrow().executor.clone();
            let mut executing_page = page_for_execute.clone();
            executing_page.set_executing();
            let progress_page = page_for_execute.clone();
            let mut done_page = page_for_execute.clone();
            let done_token = token_for_execute.clone();
            let (token, _) = dispatch::dispatch_update_execution(
                &executor,
                plan,
                move |progress| {
                    let fraction = progress
                        .total
                        .filter(|total| *total > 0)
                        .map(|total| progress.completed as f64 / total as f64)
                        .unwrap_or(0.0);
                    progress_page.set_progress(fraction, &progress.message, true);
                },
                move |outcome| {
                    match outcome {
                        UpdateExecutionOutcome::Completed {
                            message,
                            restart_required,
                        } => done_page.set_result(true, &message, restart_required),
                        UpdateExecutionOutcome::Failed { message } => {
                            done_page.set_result(false, &message, false)
                        }
                    }
                    done_token.borrow_mut().take();
                },
            );
            *token_for_execute.borrow_mut() = Some(token);
        });

        let token_for_cancel = active_token;
        updates_page.cancel_button().connect_clicked(move |_| {
            if let Some(token) = token_for_cancel.borrow().as_ref() {
                token.cancel();
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
