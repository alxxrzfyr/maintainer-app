//! Application shell: main window layout with navigation and page stack.

use std::cell::RefCell;
use std::path::PathBuf;
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

        // --- Job dispatch wiring ---
        // Storage: wire scan and duplicate-scan buttons.
        Self::wire_storage_dispatch(&storage_page, &runtime);
        // Cleanup: wire scan button.
        Self::wire_cleanup_dispatch(&cleanup_page, &runtime);
        // Updates: wire check button.
        Self::wire_updates_dispatch(&updates_page, &runtime);
        // Diagnostics: wire scan button.
        Self::wire_diagnostics_dispatch(&diagnostics_page, &runtime);

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
        Self::start_monitoring_timer(
            runtime_for_timer,
            overview_for_timer,
            system_for_timer,
            page_stack.clone(),
        );

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
        page_stack: Stack,
    ) {
        let tick_counter: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        // std channel: worker sends snapshots, main thread polls in the timer.
        let (snapshot_tx, snapshot_rx) =
            std::sync::mpsc::channel::<Box<maintainer_core::metrics::SystemSnapshot>>();

        // Bounded request channel (capacity=1) provides tick coalescing.
        // The bool signals whether the caller currently needs the per-process
        // table (only the System page renders it); when false the worker
        // reuses the last collected process list instead of re-reading all of
        // /proc/[pid], which is the single most expensive collector per tick.
        let (request_tx, request_rx) = std::sync::mpsc::sync_channel::<bool>(1);

        // Worker thread owns its own metric state.
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

        // Refresh a page as soon as it becomes visible. Because the timer only
        // updates the visible page, a page switched to would otherwise show
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

            // Drain any ready snapshots from the worker.
            // Only refresh the page the user is actually looking at: both
            // update() paths rebuild widget subtrees (the temperature flow and
            // process rows are torn down and recreated), which dirties the
            // frame and forces a repaint. Doing that for a hidden page costs
            // CPU and changes nothing on screen. The view models are still
            // updated by reduce(), so switching pages shows current data.
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
            // Only request the (expensive) per-process list when the System
            // page is actually visible — it is the only consumer.
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

    /// Wire storage page buttons to JobExecutor-backed dispatch.
    fn wire_storage_dispatch(storage_page: &StoragePage, runtime: &RuntimeHandle) {
        use crate::dispatch;
        use crate::state::ScanJobId;

        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));
        let dup_active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));

        // Storage scan button.
        let runtime_for_scan = runtime.clone();
        let token_ref = active_token.clone();
        storage_page.overview.connect_scan(move || {
            let rt = runtime_for_scan.borrow();
            let settings = rt.settings.clone();
            let executor = rt.executor.clone();
            drop(rt);

            let root = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home".to_string()));
            let mut policy = maintainer_core::storage::ScanPolicy::with_roots(vec![root]);
            policy.large_file_threshold_bytes = settings.large_file_threshold_bytes;
            for excl in &settings.scan_exclusions {
                policy.exclusions.push(PathBuf::from(excl));
            }

            let job_id = ScanJobId(1);
            let (token, _progress) = dispatch::dispatch_storage_scan(
                &executor,
                job_id,
                policy,
                |_p| {
                    // Progress delivered to UI via JobProgress widget.
                },
                |_result| {
                    // Result delivered; UI update handled through state.
                },
            );
            *token_ref.borrow_mut() = Some(token);
        });

        // Cancel.
        let cancel_token_ref = active_token.clone();
        storage_page.overview.connect_cancel(move || {
            if let Some(token) = cancel_token_ref.borrow().as_ref() {
                token.cancel();
            }
        });

        // Duplicate scan button.
        let runtime_for_dup = runtime.clone();
        let dup_token_ref = dup_active_token.clone();
        storage_page.overview.connect_duplicate_scan(move || {
            let rt = runtime_for_dup.borrow();
            let settings = rt.settings.clone();
            let executor = rt.executor.clone();
            drop(rt);

            let root = PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| "/home".to_string()));
            let mut policy = maintainer_core::storage::ScanPolicy::with_roots(vec![root]);
            policy.large_file_threshold_bytes = settings.large_file_threshold_bytes;

            let job_id = ScanJobId(2);
            let (token, _progress) =
                dispatch::dispatch_duplicate_scan(&executor, job_id, policy, |_p| {}, |_result| {});
            *dup_token_ref.borrow_mut() = Some(token);
        });
    }

    /// Wire cleanup page scan button to dispatch.
    fn wire_cleanup_dispatch(cleanup_page: &CleanupPage, runtime: &RuntimeHandle) {
        use crate::dispatch;

        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));
        let token_ref = active_token.clone();
        let runtime_ref = runtime.clone();

        cleanup_page.scan_button().connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let btn_ref = btn.clone();
            let executor = runtime_ref.borrow().executor.clone();
            let (token, _progress) = dispatch::dispatch_cleanup_scan(
                &executor,
                |_p| {},
                move |_outcome| {
                    btn_ref.set_sensitive(true);
                },
            );
            *token_ref.borrow_mut() = Some(token);
        });
    }

    /// Wire updates page check button to dispatch.
    fn wire_updates_dispatch(updates_page: &UpdatesPage, runtime: &RuntimeHandle) {
        use crate::dispatch;

        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));
        let token_ref = active_token.clone();
        let runtime_ref = runtime.clone();

        updates_page.check_button().connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let btn_ref = btn.clone();
            let executor = runtime_ref.borrow().executor.clone();
            let (token, _progress) = dispatch::dispatch_updates_check(
                &executor,
                |_p| {},
                move |_outcome| {
                    btn_ref.set_sensitive(true);
                },
            );
            *token_ref.borrow_mut() = Some(token);
        });
    }

    /// Wire diagnostics page scan button to dispatch.
    fn wire_diagnostics_dispatch(diagnostics_page: &DiagnosticsPage, runtime: &RuntimeHandle) {
        use crate::dispatch;

        let active_token: Rc<RefCell<Option<maintainer_core::jobs::CancellationToken>>> =
            Rc::new(RefCell::new(None));
        let token_ref = active_token.clone();
        let runtime_ref = runtime.clone();

        diagnostics_page.scan_button().connect_clicked(move |btn| {
            btn.set_sensitive(false);
            let btn_ref = btn.clone();
            let executor = runtime_ref.borrow().executor.clone();
            let (token, _progress) = dispatch::dispatch_diagnostics_scan(
                &executor,
                |_p| {},
                move |_outcome| {
                    btn_ref.set_sensitive(true);
                },
            );
            *token_ref.borrow_mut() = Some(token);
        });
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
