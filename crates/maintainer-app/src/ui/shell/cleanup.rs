//! Cleanup page controller and dispatch wiring.

use super::*;

impl Shell {
    pub(super) fn wire_cleanup_dispatch(
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
}
