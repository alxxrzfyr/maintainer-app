//! Diagnostics page controller and dispatch wiring.

use super::*;

impl Shell {
    pub(super) fn wire_diagnostics_dispatch(
        diagnostics_page: &DiagnosticsPage,
        runtime: &RuntimeHandle,
    ) {
        use crate::dispatch::{self, DiagnosticsScanOutcome, SmartRefreshOutcome};
        use crate::ui::diagnostics::{DiagnosticsIntent, DiagnosticsState};

        let page = diagnostics_page.clone();
        let state = Rc::new(RefCell::new(DiagnosticsState::default()));
        load_hardware(&runtime.borrow().executor, &page, &state, || {
            Ok(maintainer_core::hardware::read_hardware_snapshot())
        });
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
}

fn load_hardware(
    executor: &std::sync::Arc<maintainer_core::jobs::JobExecutor>,
    page: &DiagnosticsPage,
    state: &Rc<RefCell<crate::ui::diagnostics::DiagnosticsState>>,
    collect: impl FnOnce()
        -> maintainer_core::error::Result<maintainer_core::hardware::HardwareSnapshot>
    + Send
    + 'static,
) {
    use crate::ui::diagnostics::{DiagnosticsIntent, HardwareState};
    let page = page.clone();
    let state = state.clone();
    crate::dispatch::dispatch_hardware_collection(executor, collect, move |result| {
        state
            .borrow_mut()
            .reduce(DiagnosticsIntent::HardwareUpdated(match result {
                Ok(snapshot) => HardwareState::Loaded(Rc::new(snapshot)),
                Err(message) => HardwareState::Failed(message),
            }));
        page.update_hardware(&state.borrow().hardware);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::diagnostics::{DiagnosticsState, HardwareState};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };

    #[test]
    #[ignore = "requires a GTK display; run this exact test under Xvfb"]
    fn lazy_hardware_collection_keeps_gtk_responsive_and_runs_once() {
        gtk4::init().unwrap();
        let executor = Arc::new(maintainer_core::jobs::JobExecutor::new(1).unwrap());
        let calls = Arc::new(AtomicUsize::new(0));
        let (entered_tx, entered_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        let state = Rc::new(RefCell::new(DiagnosticsState::default()));
        let stack = Stack::new();
        stack.add_named(&Label::new(Some("Overview")), Some("Overview"));
        let worker_calls = calls.clone();
        let mounted_state = state.clone();
        let mounted_executor = executor.clone();
        Shell::add_lazy_page(&stack, "Diagnostics", move || {
            let page = DiagnosticsPage::new();
            load_hardware(&mounted_executor, &page, &mounted_state, move || {
                worker_calls.fetch_add(1, Ordering::SeqCst);
                entered_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(maintainer_core::hardware::HardwareSnapshot::default())
            });
            Some(page.widget().clone().upcast())
        });
        assert_eq!(calls.load(Ordering::SeqCst), 0);
        let mount_started = Instant::now();
        stack.set_visible_child_name("Diagnostics");
        let mount_elapsed = mount_started.elapsed();
        assert!(mount_elapsed < Duration::from_secs(2));
        // Waiting for worker entry does not dispatch its completion on the GTK thread.
        entered_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        assert_eq!(state.borrow().hardware, HardwareState::Loading);
        let idle_ran = Rc::new(Cell::new(false));
        let idle_marker = idle_ran.clone();
        glib::idle_add_local_once(move || idle_marker.set(true));
        let context = glib::MainContext::default();
        let event_started = Instant::now();
        let deadline = event_started + Duration::from_secs(2);
        while !idle_ran.get() && Instant::now() < deadline {
            context.iteration(false);
        }
        eprintln!(
            "Diagnostics mount: {mount_elapsed:?}; blocked-worker GTK event response: {:?}",
            event_started.elapsed()
        );
        assert!(
            idle_ran.get(),
            "GTK must process events while collection is blocked"
        );
        stack.set_visible_child_name("Overview");
        stack.set_visible_child_name("Diagnostics");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(state.borrow().hardware, HardwareState::Loading) && Instant::now() < deadline
        {
            context.iteration(false);
        }
        assert!(matches!(state.borrow().hardware, HardwareState::Loaded(_)));
        stack.set_visible_child_name("Overview");
        stack.set_visible_child_name("Diagnostics");
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    #[ignore = "requires a GTK display; run this exact test under Xvfb"]
    fn hardware_worker_errors_leave_loading_state() {
        gtk4::init().unwrap();
        let executor = Arc::new(maintainer_core::jobs::JobExecutor::new(1).unwrap());
        let page = DiagnosticsPage::new();
        let state = Rc::new(RefCell::new(DiagnosticsState::default()));
        load_hardware(&executor, &page, &state, || {
            Err(maintainer_core::error::MaintainerError::Internal {
                context: "hardware".into(),
                message: "collector failed".into(),
            })
        });
        let context = glib::MainContext::default();
        let deadline = Instant::now() + Duration::from_secs(2);
        while matches!(state.borrow().hardware, HardwareState::Loading) && Instant::now() < deadline
        {
            context.iteration(false);
        }
        assert!(
            matches!(&state.borrow().hardware, HardwareState::Failed(message) if message.contains("collector failed"))
        );
    }
}
