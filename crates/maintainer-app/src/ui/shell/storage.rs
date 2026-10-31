//! Storage page controller and dispatch wiring.

use super::*;

impl Shell {
    pub(super) fn wire_storage_dispatch(storage_page: &StoragePage, runtime: &RuntimeHandle) {
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
}
