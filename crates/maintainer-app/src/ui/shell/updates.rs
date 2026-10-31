//! Updates page controller and dispatch wiring.

use super::*;

impl Shell {
    pub(super) fn wire_updates_dispatch(updates_page: &UpdatesPage, runtime: &RuntimeHandle) {
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
}
