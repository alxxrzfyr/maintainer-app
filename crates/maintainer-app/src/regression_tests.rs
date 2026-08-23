//! Regression coverage for shell navigation and responsive behavior.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::{Mutex, MutexGuard};

use gtk4::prelude::*;
use gtk4::{Button, Widget};

use crate::state::Page;
use crate::ui::navigation::{NavigationMode, build_sidebar};

static GTK_TEST_LOCK: Mutex<()> = Mutex::new(());

fn initialize_gtk_test() -> Option<MutexGuard<'static, ()>> {
    let guard = GTK_TEST_LOCK.lock().ok()?;
    if gtk4::is_initialized() {
        if !gtk4::is_initialized_main_thread() {
            return None;
        }
    } else if gtk4::init().is_err() {
        return None;
    }
    Some(guard)
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
    let Some(_gtk_guard) = initialize_gtk_test() else {
        return;
    };

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
fn sidebar_is_fixed_pinned_and_updates_active_destination() {
    let Some(_gtk_guard) = initialize_gtk_test() else {
        return;
    };

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
        shell.contains("connect_fullscreened_notify"),
        "the app must immediately reverse compositor fullscreen requests"
    );
    assert!(
        shell.contains(".default_width(1100)")
            && shell.contains(".default_height(700)")
            && shell.contains("set_size_request(640, 420)"),
        "default and minimum window dimensions must remain wider than tall"
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

#[test]
fn system_details_use_compact_two_column_rows() {
    let system = include_str!("ui/system.rs");

    assert!(
        system.contains("detail_grid.attach(&cpu_detail_box, 0, 0, 1, 1)")
            && system.contains("detail_grid.attach(&memory_detail_box, 1, 0, 1, 1)"),
        "per-core CPU and memory composition must share one equal two-column row"
    );
    assert!(
        system.contains("network_summary_label"),
        "system page must have a compact network summary (not a full table)"
    );
    assert!(
        system.contains("maintainer_temperature_panel"),
        "temperature sensors must use the compact telemetry panel"
    );
    assert!(
        system.contains("temperature_groups"),
        "temperature display must use grouped readings, not raw duplicates"
    );
}

#[test]
fn update_progress_uses_continuous_indeterminate_animation() {
    let updates = include_str!("ui/updates.rs");
    assert!(
        updates.contains("timeout_add_local") && updates.contains("progress_indeterminate"),
        "authentication/loading progress must pulse continuously, not only when output arrives"
    );
}

#[test]
fn update_execution_buttons_follow_transaction_safety() {
    let Some(_gtk_guard) = initialize_gtk_test() else {
        return;
    };

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
fn network_page_does_not_access_system_interfaces_directly() {
    let network_src = include_str!("ui/network.rs");

    // Must not read /proc, /sys, /etc directly.
    assert!(
        !network_src.contains("read_to_string(\"/proc"),
        "network.rs must not read /proc directly — use maintainer_core services"
    );
    assert!(
        !network_src.contains("read_to_string(\"/etc"),
        "network.rs must not read /etc directly — use maintainer_core services"
    );
    assert!(
        !network_src.contains("read_to_string(\"/sys"),
        "network.rs must not read /sys directly — use maintainer_core services"
    );

    // Must not spawn commands directly (except via core).
    assert!(
        !network_src.contains("Command::new(\"curl\""),
        "network.rs must not spawn curl directly — use speed_test core service"
    );
    assert!(
        !network_src.contains("Command::new(\"ip\""),
        "network.rs must not spawn ip directly — use connection core service"
    );
    assert!(
        !network_src.contains("Command::new(\"iw\""),
        "network.rs must not spawn iw directly — use connection core service"
    );
}

#[test]
fn destructive_buttons_use_project_specific_class() {
    let settings = include_str!("ui/settings.rs");
    let cleanup = include_str!("ui/cleanup.rs");
    let history = include_str!("ui/history.rs");
    let updates = include_str!("ui/updates.rs");
    let kernel = include_str!("ui/kernel_section.rs");
    let confirm = include_str!("ui/dialogs/confirm_cleanup.rs");

    for (name, src) in [
        ("settings.rs", settings),
        ("cleanup.rs", cleanup),
        ("history.rs", history),
        ("updates.rs", updates),
        ("kernel_section.rs", kernel),
        ("confirm_cleanup.rs", confirm),
    ] {
        assert!(
            !src.contains("\"destructive-action\""),
            "{name} must use \"maintainer_destructive\" instead of GTK's \"destructive-action\" \
             to avoid collision with built-in theming"
        );
    }

    let dark_css = include_str!("../../../data/styles/dark.css");
    let light_css = include_str!("../../../data/styles/light.css");
    for (name, css) in [("dark.css", dark_css), ("light.css", light_css)] {
        assert!(
            css.contains(".maintainer_destructive {"),
            "{name} must define .maintainer_destructive class"
        );
        assert!(
            css.contains(".maintainer_destructive:hover {"),
            "{name} must define .maintainer_destructive:hover"
        );
        assert!(
            css.contains(".maintainer_destructive:disabled {"),
            "{name} must define .maintainer_destructive:disabled"
        );
    }
}

#[test]
fn temperature_display_filters_phantom_zero_readings() {
    let system_src = include_str!("ui/system.rs");

    // Must filter 0°C readings.
    assert!(
        system_src.contains("*celsius <= 0.0"),
        "system.rs must filter phantom 0°C readings from non-reporting sensors"
    );

    // Must use homogeneous FlowBox.
    assert!(
        system_src.contains("set_homogeneous(true)"),
        "temperature FlowBox must be homogeneous for uniform chip sizes"
    );

    // Must have tooltip support.
    assert!(
        system_src.contains("set_tooltip_text"),
        "temperature chips must have tooltips for details"
    );
}

#[test]
fn speed_test_uses_core_service_not_inline_curl() {
    let network_src = include_str!("ui/network.rs");

    // Must use core speed test service.
    assert!(
        network_src.contains("maintainer_core::metrics::run_speed_test")
            || network_src.contains("maintainer_core::metrics::speed_test"),
        "network.rs must delegate speed test to maintainer_core"
    );

    // Must report stage progress.
    assert!(
        network_src.contains("stage.label()") || network_src.contains("stage_rx"),
        "speed test must report stage-level progress (latency/download/upload)"
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
