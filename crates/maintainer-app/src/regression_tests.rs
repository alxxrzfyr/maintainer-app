//! Regression coverage for shell navigation and responsive behavior.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use gtk4::prelude::*;
use gtk4::{Button, Widget};

use crate::state::Page;
use crate::ui::navigation::{NavigationMode, build_sidebar};

static GTK_TEST_LOCK: Mutex<()> = Mutex::new(());

fn initialize_gtk_test() -> MutexGuard<'static, ()> {
    let guard = GTK_TEST_LOCK.lock().expect("GTK test lock poisoned");
    assert!(
        !gtk4::is_initialized() || gtk4::is_initialized_main_thread(),
        "Run each GTK test in its own process using scripts/test-native.sh"
    );
    gtk4::init().expect("GTK display required; use scripts/test-native.sh");
    guard
}

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
fn storage_apply_state_does_not_reenter_borrowed_state() {
    // Reproduces a real crash: StoragePage::apply_state calls
    // Stack::set_visible_child_name, which fires notify::visible-child-name
    // synchronously. The shell's handler for that signal used to borrow_mut()
    // the same StorageState RefCell that a caller of apply_state may still
    // hold borrowed (the common call pattern is
    // `page.apply_state(&state.borrow())`), which panicked with
    // BorrowMutError inside a GTK C signal callback — a context that cannot
    // unwind, so the process aborted instead of merely failing a test.
    let _gtk_guard = initialize_gtk_test();

    use crate::state::{StorageState, StorageSubPage};
    use crate::ui::storage::StoragePage;
    use std::cell::RefCell;

    let page = StoragePage::new();
    let state: Rc<RefCell<StorageState>> = Rc::new(RefCell::new(StorageState::default()));

    // Wire the same reentrant handler the shell installs in production.
    let programmatic_nav_guard = page.programmatic_navigation.clone();
    let state_for_notify = state.clone();
    page.stack_handle()
        .connect_visible_child_name_notify(move |stack| {
            if programmatic_nav_guard.get() {
                return;
            }
            if let Some(name) = stack.visible_child_name() {
                let sub = match name.as_str() {
                    "large-files" => StorageSubPage::LargeFiles,
                    "duplicates" => StorageSubPage::Duplicates,
                    _ => StorageSubPage::Overview,
                };
                state_for_notify.borrow_mut().sub_page = sub;
            }
        });

    state.borrow_mut().sub_page = StorageSubPage::LargeFiles;

    // The exact pattern from the crash: apply_state is called while the
    // caller still holds an immutable borrow of the same RefCell the notify
    // handler above would need to borrow_mut(). Before the fix, this
    // panicked; the fix makes this a no-op reentrant call instead.
    page.apply_state(&state.borrow());

    assert_eq!(
        page.stack_handle().visible_child_name().as_deref(),
        Some("large-files"),
        "the stack must still reach the requested page despite the guard"
    );
}

#[test]
fn rebuilt_sidebars_release_their_widgets() {
    let _gtk_guard = initialize_gtk_test();
    for _ in 0..3 {
        for mode in [
            NavigationMode::Persistent,
            NavigationMode::Compact,
            NavigationMode::Drawer,
        ] {
            let sidebar = build_sidebar(Page::Overview, mode, |_| {});
            let weak_sidebar = sidebar.downgrade();
            let weak_button = descendants_of_type::<Button>(&sidebar)[0].downgrade();
            drop(sidebar);
            assert!(
                weak_sidebar.upgrade().is_none(),
                "old sidebar retained after rebuild"
            );
            assert!(
                weak_button.upgrade().is_none(),
                "old navigation button retained after rebuild"
            );
        }
    }
}

#[test]
fn sidebar_fits_default_window_and_keeps_footer_reachable() {
    let _gtk_guard = initialize_gtk_test();
    crate::application::apply_theme(crate::state::Theme::Light);
    let sidebar = build_sidebar(Page::Overview, NavigationMode::Persistent, |_| {});
    sidebar.set_width_request(220);
    let layout = gtk4::Box::new(gtk4::Orientation::Horizontal, 0);
    layout.append(&sidebar);
    let content = gtk4::Box::new(gtk4::Orientation::Vertical, 0);
    content.set_hexpand(true);
    layout.append(&content);
    let window = gtk4::Window::builder()
        .default_width(1100)
        .default_height(700)
        .build();
    window.add_css_class("maintainer_window");
    window.set_child(Some(&layout));
    let minimum_height = sidebar.measure(gtk4::Orientation::Vertical, 220).0;
    window.present();
    let context = glib::MainContext::default();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while window.height() == 0 && std::time::Instant::now() < deadline {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    eprintln!(
        "sidebar minimum height: {minimum_height}; window allocated: {}x{}",
        window.width(),
        window.height()
    );
    assert_eq!(window.width(), 1100);
    assert_eq!(
        window.height(),
        700,
        "navigation must not enlarge the default viewport"
    );
    let footer = sidebar.last_child().unwrap();
    let footer_bounds = footer.compute_bounds(&window).unwrap();
    assert!(footer_bounds.y() >= 0.0);
    assert!(footer_bounds.y() + footer_bounds.height() <= 700.0);
    let buttons = descendants_of_type::<Button>(&sidebar);
    assert!(buttons[0].grab_focus());
    let mut reached = std::collections::HashSet::new();
    for _ in 0..Page::ALL.len() * 3 {
        if let Some(button) = gtk4::prelude::GtkWindowExt::focus(&window).and_downcast::<Button>() {
            if let Some(label) = button.tooltip_text() {
                reached.insert(label.to_string());
            }
        }
        window.child_focus(gtk4::DirectionType::TabForward);
        while context.pending() {
            context.iteration(false);
        }
    }
    for page in Page::ALL {
        assert!(
            reached.contains(page.label()),
            "keyboard focus did not reach {}",
            page.label()
        );
    }
    window.close();
}

#[test]
fn sidebar_is_fixed_pinned_and_updates_active_destination() {
    let _gtk_guard = initialize_gtk_test();

    let selected = Rc::new(Cell::new(Page::Overview));
    let selected_for_callback = selected.clone();
    let sidebar = build_sidebar(Page::Overview, NavigationMode::Persistent, move |page| {
        selected_for_callback.set(page);
    });

    let scroll_regions = descendants_of_type::<gtk4::ScrolledWindow>(&sidebar);
    assert_eq!(scroll_regions.len(), 1, "only the navigation items scroll");
    let navigation_scroll = &scroll_regions[0];
    assert_eq!(
        navigation_scroll.hscrollbar_policy(),
        gtk4::PolicyType::Never
    );
    assert_eq!(
        navigation_scroll.vscrollbar_policy(),
        gtk4::PolicyType::Automatic
    );
    assert!(
        descendants_of_type::<gtk4::Box>(navigation_scroll)
            .iter()
            .any(|widget| widget.has_css_class("maintainer_sidebar_groups"))
    );

    let footer = sidebar
        .last_child()
        .expect("sidebar must have a dedicated footer region");
    assert!(footer.has_css_class("maintainer_sidebar_footer"));
    assert!(descendants_of_type::<gtk4::ScrolledWindow>(&footer).is_empty());
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
fn cleanup_actions_share_a_fixed_row_above_expanding_results() {
    let _guard = initialize_gtk_test();
    let page = crate::ui::cleanup::CleanupPage::new();
    let actions = page
        .scan_button()
        .parent()
        .unwrap()
        .downcast::<gtk4::Box>()
        .unwrap();
    assert_eq!(actions.orientation(), gtk4::Orientation::Horizontal);
    assert!(!actions.vexpands());
    assert_eq!(
        page.package_cache_button().parent().as_ref(),
        Some(actions.upcast_ref())
    );
    assert_eq!(
        page.journal_vacuum_button().parent().as_ref(),
        Some(actions.upcast_ref())
    );
    let results = descendants_of_type::<gtk4::ScrolledWindow>(page.widget());
    assert_eq!(results.len(), 1);
    assert!(results[0].vexpands());
    assert_eq!(
        results[0].parent().as_ref(),
        Some(page.widget().upcast_ref())
    );
    let mut sibling = actions.next_sibling();
    while let Some(widget) = sibling {
        if widget == results[0].clone().upcast::<Widget>() {
            return;
        }
        sibling = widget.next_sibling();
    }
    panic!("expanding results must follow the fixed action row");
}

#[test]
fn navigation_css_uses_a_quiet_selected_surface() {
    for css in [
        include_str!("../../../data/styles/light.css"),
        include_str!("../../../data/styles/dark.css"),
    ] {
        assert!(!css.contains("box-shadow: inset 3px 0 @maintainer_accent"));
    }
}

#[test]
fn system_details_use_compact_two_column_rows() {
    let _guard = initialize_gtk_test();
    let page = crate::ui::system::SystemPage::new();
    let grid = descendants_of_type::<gtk4::Grid>(page.widget())
        .into_iter()
        .find(|grid| grid.has_css_class("maintainer_resource_grid"))
        .unwrap();
    assert!(grid.is_column_homogeneous());
    for (column, title) in [(0, "Per-Core Usage"), (1, "Memory Composition")] {
        let card = grid
            .child_at(column, 0)
            .expect("detail card in the first row");
        assert_eq!(grid.query_child(&card), (column, 0, 1, 1));
        assert!(
            descendants_of_type::<gtk4::Label>(&card)
                .iter()
                .any(|label| label.text() == title)
        );
    }
    assert!(grid.child_at(0, 1).is_none());
    let labels = descendants_of_type::<gtk4::Label>(page.widget());
    assert_eq!(
        labels
            .iter()
            .filter(|label| label.text() == "Network: —")
            .count(),
        1
    );
    assert!(
        descendants_of_type::<gtk4::Box>(page.widget())
            .iter()
            .any(|panel| panel.has_css_class("maintainer_temperature_panel"))
    );
}

#[test]
fn incomplete_duplicate_scan_shows_coverage_warning() {
    let _guard = initialize_gtk_test();
    let view = crate::ui::storage::duplicates::DuplicatesView::new();
    let mut state = crate::state::StorageState::default();
    state
        .duplicate_issues
        .push(maintainer_core::storage::DuplicateIssue {
            path: std::path::PathBuf::new(),
            message: "candidate limit reached".into(),
        });
    view.apply_state(&state);
    let labels = descendants_of_type::<gtk4::Label>(view.widget());
    assert!(
        labels
            .iter()
            .any(|label| label.is_visible() && label.text().contains("coverage is incomplete"))
    );
    assert!(!labels.iter().any(|label| label.is_visible()
        && label.text() == "No duplicates found. Run a duplicate scan first."));
}

#[test]
fn process_rows_reuse_identity_keyed_widgets() {
    let _guard = initialize_gtk_test();
    use crate::monitoring::ProcessesViewModel;
    use maintainer_core::metrics::{ProcessIdentity, ProcessSnapshot};
    let process = |pid, start| ProcessSnapshot {
        identity: ProcessIdentity {
            pid,
            start_time_ticks: start,
        },
        name: format!("process-{pid}"),
        state: 'S',
        ppid: 1,
        uid: 1000,
        rss_bytes: 1024,
        utime_ticks: 0,
        stime_ticks: 0,
        num_threads: 1,
    };
    let page = crate::ui::processes::ProcessesPage::new();
    let ended = Rc::new(std::cell::RefCell::new(Vec::new()));
    let ended_for_callback = ended.clone();
    page.connect_terminate(move |pid, start| ended_for_callback.borrow_mut().push((pid, start)));
    let mut model = ProcessesViewModel {
        available: true,
        total_count: 2,
        processes: vec![process(10, 100), process(20, 200)],
        ..Default::default()
    };
    let buttons = || {
        descendants_of_type::<Button>(page.widget())
            .into_iter()
            .filter(|button| button.label().as_deref() == Some("End"))
            .collect::<Vec<_>>()
    };
    page.update(&model);
    let original = buttons();
    assert_eq!(original.len(), 2);
    model.processes.reverse();
    model.processes[0].rss_bytes = 2048;
    page.update(&model);
    let reordered = buttons();
    assert_eq!(reordered, vec![original[1].clone(), original[0].clone()]);
    reordered[0].emit_clicked();
    assert_eq!(*ended.borrow(), vec![(20, 200)]);
    model.processes[0] = process(20, 201);
    page.update(&model);
    let reused_pid = buttons();
    assert_ne!(reused_pid[0], original[1]);
    reused_pid[0].emit_clicked();
    assert_eq!(*ended.borrow(), vec![(20, 200), (20, 201)]);
}

#[test]
fn update_progress_uses_continuous_indeterminate_animation() {
    let _guard = initialize_gtk_test();
    let mut page = crate::ui::updates::UpdatesPage::new();
    let window = gtk4::Window::builder()
        .default_width(600)
        .default_height(500)
        .build();
    window.set_child(Some(page.widget()));
    window.present();
    page.set_executing();
    let bar = page.progress_bar_handle();
    let context = glib::MainContext::default();
    let sample = || {
        descendants_of_type::<Widget>(&bar)
            .into_iter()
            .find(|node| node.css_name() == "progress")
            .and_then(|node| node.compute_bounds(&bar))
            .map(|bounds| (bounds.x().to_bits(), bounds.width().to_bits()))
    };
    let allocation_deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while (!bar.is_mapped() || bar.width() == 0) && std::time::Instant::now() < allocation_deadline
    {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert!(bar.is_mapped() && bar.width() > 0);
    // Observe later movement, after a single initial pulse would have stopped.
    // Rendering startup is outside the observation window, including on busy CI.
    let settle_until = std::time::Instant::now() + std::time::Duration::from_millis(500);
    while std::time::Instant::now() < settle_until {
        context.iteration(false);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let mut positions = std::collections::HashSet::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(4);
    while positions.len() <= 3 && std::time::Instant::now() < deadline {
        context.iteration(false);
        if let Some(position) = sample() {
            positions.insert(position);
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    assert!(
        positions.len() > 3,
        "progress must keep moving without worker output: {positions:?}"
    );
    page.set_progress(25, Some(100), "Installing", false);
    assert_eq!(bar.fraction(), 0.25);
    page.set_result(true, "Done", false);
    assert!(!bar.is_visible());
    window.close();
}

#[test]
fn update_execution_buttons_follow_transaction_safety() {
    let _gtk_guard = initialize_gtk_test();

    let mut page = crate::ui::updates::UpdatesPage::new();
    assert!(page.check_button().is_sensitive());

    page.set_executing();
    assert!(!page.check_button().is_sensitive());
    assert!(page.cancel_button().is_visible());
    assert!(page.cancel_button().is_sensitive());
    assert_eq!(
        page.progress_bar_handle().text().as_deref(),
        Some("In progress")
    );

    page.set_progress(10, Some(100), "Installing packages…", false);
    assert!(!page.cancel_button().is_sensitive());
    assert_eq!(
        page.cancel_button().tooltip_text().as_deref(),
        Some("Cannot cancel at this stage")
    );

    page.set_result(false, "Verification failed", false);
    assert!(page.check_button().is_sensitive());
    assert!(!page.cancel_button().is_visible());
    assert!(!page.progress_bar_handle().is_visible());
}

#[test]
fn css_defines_all_required_semantic_tokens() {
    let dark_css = include_str!("../../../data/styles/dark.css");
    let light_css = include_str!("../../../data/styles/light.css");

    let required_tokens = [
        "maintainer_rx",
        "maintainer_tx",
        "maintainer_severity_info",
        "maintainer_severity_warn",
        "maintainer_severity_crit",
        "maintainer_severity_ok",
        "maintainer_process_running",
        "maintainer_process_zombie",
    ];

    for token in &required_tokens {
        let define = format!("@define-color {token}");
        assert!(
            dark_css.contains(&define),
            "dark.css missing @define-color for '{token}'"
        );
        assert!(
            light_css.contains(&define),
            "light.css missing @define-color for '{token}'"
        );
    }
}

#[test]
fn destructive_buttons_use_project_specific_class() {
    let _guard = initialize_gtk_test();
    let settings = crate::ui::settings::build_settings_page(
        &crate::state::SettingsSnapshot::default(),
        |_| {},
    );
    let cleanup = crate::ui::cleanup::CleanupPage::new();
    let history = crate::ui::history::build_history_page(
        &[crate::ui::history::HistoryRow {
            id: "fixture".into(),
            operation: "Cleanup".into(),
            time: "today".into(),
            result: "Success".into(),
            recovered_bytes: "1 MiB".into(),
            error_summary: None,
        }],
        |_| {},
    );
    let updates = crate::ui::updates::UpdatesPage::new();
    let mut kernels = crate::ui::kernel_section::KernelSection::new();
    kernels.set_loaded(&[maintainer_core::kernel::KernelInfo {
        package_name: "linux-image-fixture".into(),
        version: "fixture".into(),
        is_running: false,
        is_removable: true,
    }]);
    let confirm = crate::ui::dialogs::confirm_cleanup::ConfirmCleanupDialog::new(&[], 0, false);
    for (root, expected_labels) in [
        (
            settings.upcast_ref::<Widget>(),
            vec!["Delete all history", "Delete all Maintainer data"],
        ),
        (
            cleanup.widget().upcast_ref::<Widget>(),
            vec!["Review Selected Files"],
        ),
        (
            history.upcast_ref::<Widget>(),
            vec!["Delete selected", "Delete all"],
        ),
        (updates.widget().upcast_ref::<Widget>(), vec!["Cancel"]),
        (
            kernels.widget().upcast_ref::<Widget>(),
            vec!["Review Removal"],
        ),
        (
            confirm.widget().upcast_ref::<Widget>(),
            vec!["Move to Trash"],
        ),
    ] {
        let buttons = descendants_of_type::<Button>(root);
        assert!(
            buttons
                .iter()
                .all(|button| !button.has_css_class("destructive-action"))
        );
        for label in expected_labels {
            let button = buttons
                .iter()
                .find(|button| button.label().as_deref() == Some(label))
                .unwrap_or_else(|| {
                    panic!(
                        "missing destructive button {label}; found {:?}",
                        buttons
                            .iter()
                            .map(|button| button.label())
                            .collect::<Vec<_>>()
                    )
                });
            assert!(button.has_css_class("maintainer_destructive"), "{label}");
        }
    }
    for css in [
        include_str!("../../../data/styles/dark.css"),
        include_str!("../../../data/styles/light.css"),
    ] {
        for selector in [
            ".maintainer_destructive {",
            ".maintainer_destructive:hover {",
            ".maintainer_destructive:disabled {",
        ] {
            assert!(css.contains(selector), "missing CSS state {selector}");
        }
    }
}

#[test]
fn temperature_display_filters_phantom_zero_readings() {
    let _guard = initialize_gtk_test();
    use maintainer_core::capability::Capability;
    use maintainer_core::metrics::{TemperatureGroup, TemperatureReading};
    let reading = |label: &str, value| TemperatureReading {
        hwmon_name: "fixture-chip".into(),
        label: Some(label.into()),
        celsius: Capability::Available(value),
    };
    let group = |label: &str, value| TemperatureGroup {
        chip_name: label.into(),
        representative: reading(label, value),
        details: vec![
            reading(label, value),
            reading("Core 1", 47.0),
            reading("phantom", 0.0),
        ],
    };
    let page = crate::ui::system::SystemPage::new();
    let mut vm = crate::monitoring::SystemViewModel {
        temperature_groups: vec![
            group("CPU", 45.0),
            group("zero", 0.0),
            group("negative", -1.0),
        ],
        ..Default::default()
    };
    page.update(&vm);
    let chips = || {
        descendants_of_type::<gtk4::Box>(page.widget())
            .into_iter()
            .filter(|chip| chip.has_css_class("maintainer_temperature_chip"))
            .collect::<Vec<_>>()
    };
    let rendered = chips();
    assert_eq!(rendered.len(), 1);
    assert_eq!(
        rendered[0].tooltip_text().as_deref(),
        Some("CPU: 45°C\n  Core 1: 47°C")
    );
    let flow = descendants_of_type::<gtk4::FlowBox>(page.widget())
        .into_iter()
        .find(|flow| flow.has_css_class("maintainer_temperature_flow"))
        .unwrap();
    assert!(flow.is_homogeneous());
    vm.temperature_groups[0].representative.celsius = Capability::Available(0.0);
    page.update(&vm);
    assert!(
        chips().is_empty(),
        "a previously valid sensor must disappear when it stops reporting"
    );
}

#[test]
fn network_rate_labels_use_monospace() {
    let dark_css = include_str!("../../../data/styles/dark.css");
    let light_css = include_str!("../../../data/styles/light.css");

    for (name, css) in [("dark.css", dark_css), ("light.css", light_css)] {
        // Find .maintainer_net_rx block and verify it contains monospace.
        let rx_idx = css.find(".maintainer_net_rx {").unwrap_or(0);
        let rx_block = &css[rx_idx..rx_idx + 120.min(css.len() - rx_idx)];
        assert!(
            rx_block.contains("monospace"),
            "{name} .maintainer_net_rx must use monospace font for stable column width"
        );
    }
}

#[test]
fn sidebar_icons_are_differentiated() {
    // Each page must have a distinct primary icon (first in chain).
    let all_firsts: Vec<&str> = Page::ALL.iter().map(|p| p.icon_chain()[0]).collect();

    // No two pages should share the same primary icon.
    for (i, a) in all_firsts.iter().enumerate() {
        for (j, b) in all_firsts.iter().enumerate() {
            if i != j {
                assert_ne!(
                    a,
                    b,
                    "Pages {} and {} share primary icon '{a}'",
                    Page::ALL[i].label(),
                    Page::ALL[j].label()
                );
            }
        }
    }
}
