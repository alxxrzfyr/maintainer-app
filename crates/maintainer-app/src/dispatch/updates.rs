//! Update checks and selection review.

use std::sync::Arc;

use maintainer_core::jobs::{CancellationToken, JobExecutor};

use super::{install_progress_poller, vector_bytes};
use crate::progress::SharedProgress;

/// Updates check result.
pub enum UpdatesCheckOutcome {
    Available {
        summary: String,
        security_count: usize,
        total_count: usize,
        plan: maintainer_core::updates::UpdatePlan,
    },
    UpToDate,
    Failed {
        message: String,
    },
}

/// Launch an update check through the shared `JobExecutor`.
pub fn dispatch_updates_check(
    executor: &Arc<JobExecutor>,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(UpdatesCheckOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();
    let _progress_for_worker = progress.clone();

    let job = move || -> maintainer_core::error::Result<UpdatesCheckOutcome> {
        use maintainer_core::capability::Capability;
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::updates::{AptDiscovery, DiscoveryResult};

        let runner = SystemCommandRunner;
        // `check_full_upgrade` (apt-get --simulate dist-upgrade) is used rather
        // than `check_updates` (plain upgrade). Plain upgrade mode refuses to
        // pull in new packages or dependency changes, which silently hides any
        // update that requires one -- including kernel updates tied to the HWE
        // metapackage chain (linux-generic-hwe-*), which on Ubuntu 24.04
        // depends on new headers/tools/modules packages alongside the image.
        // Every mainstream update manager (Ubuntu's, Mint's) resolves against
        // the dist-upgrade equivalent for this reason; using plain upgrade
        // here made this app under-report updates relative to those tools.
        let result = AptDiscovery::check_full_upgrade(&runner, &token_for_worker);

        let outcome = match result {
            Capability::Available(DiscoveryResult::Available(plan)) => {
                let total_count = plan.candidates.len();
                if total_count == 0 {
                    UpdatesCheckOutcome::UpToDate
                } else {
                    let security_count = plan.security_count();
                    let upgrade_count = plan.candidates.iter().filter(|c| !c.is_new).count();
                    let new_count = plan.candidates.iter().filter(|c| c.is_new).count();
                    let summary = crate::ui::updates::format_update_summary(
                        upgrade_count,
                        new_count,
                        plan.removals.len(),
                        plan.held.len(),
                        plan.total_download_bytes,
                    );
                    UpdatesCheckOutcome::Available {
                        summary,
                        security_count,
                        total_count,
                        plan,
                    }
                }
            }
            Capability::Available(DiscoveryResult::UpToDate) => UpdatesCheckOutcome::UpToDate,
            Capability::Available(DiscoveryResult::Failed(err)) => UpdatesCheckOutcome::Failed {
                message: err.to_string(),
            },
            Capability::Unavailable(reason) => UpdatesCheckOutcome::Failed {
                message: reason.message,
            },
            Capability::PermissionRequired(reason) => UpdatesCheckOutcome::Failed {
                message: format!("Permission required: {}", reason.explanation),
            },
        };

        Ok(outcome)
    };

    let rx = executor.execute_sized(vec![job], token.clone(), updates_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, |message| {
        UpdatesCheckOutcome::Failed { message }
    });

    (token, progress)
}

/// Re-plan for a user-selected subset of update candidates through apt's
/// dependency resolver. Returns the same `UpdatesCheckOutcome` shape as
/// `dispatch_updates_check` so the UI can display the re-planned result
/// identically.
///
/// Safety: this is the ONLY code path that supports per-package deselection.
/// It invokes `apt-get --simulate install pkg=ver ...` to ask apt what
/// would actually happen, rather than fabricating a filtered plan client-side.
pub fn dispatch_replan_selection(
    executor: &Arc<JobExecutor>,
    selected: Vec<maintainer_core::updates::UpdateCandidate>,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(UpdatesCheckOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();

    let job = move || -> maintainer_core::error::Result<UpdatesCheckOutcome> {
        use maintainer_core::capability::Capability;
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::updates::{AptDiscovery, DiscoveryResult};

        let runner = SystemCommandRunner;
        let result = AptDiscovery::plan_for_selection(&runner, &token_for_worker, &selected);

        let outcome = match result {
            Capability::Available(DiscoveryResult::Available(plan)) => {
                let total_count = plan.candidates.len();
                if total_count == 0 {
                    UpdatesCheckOutcome::UpToDate
                } else {
                    let security_count = plan.security_count();
                    let upgrade_count = plan.candidates.iter().filter(|c| !c.is_new).count();
                    let new_count = plan.candidates.iter().filter(|c| c.is_new).count();
                    let summary = crate::ui::updates::format_update_summary(
                        upgrade_count,
                        new_count,
                        plan.removals.len(),
                        plan.held.len(),
                        plan.total_download_bytes,
                    );
                    UpdatesCheckOutcome::Available {
                        summary,
                        security_count,
                        total_count,
                        plan,
                    }
                }
            }
            Capability::Available(DiscoveryResult::UpToDate) => UpdatesCheckOutcome::UpToDate,
            Capability::Available(DiscoveryResult::Failed(err)) => UpdatesCheckOutcome::Failed {
                message: err.to_string(),
            },
            Capability::Unavailable(reason) => UpdatesCheckOutcome::Failed {
                message: reason.message,
            },
            Capability::PermissionRequired(reason) => UpdatesCheckOutcome::Failed {
                message: format!("Permission required: {}", reason.explanation),
            },
        };

        Ok(outcome)
    };

    let rx = executor.execute_sized(vec![job], token.clone(), updates_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, |message| {
        UpdatesCheckOutcome::Failed { message }
    });

    (token, progress)
}

/// A filtered list is executable only after the selection resolver returns a plan.
pub fn update_selection_reviewed(
    mode: crate::ui::updates::SelectionMode,
    replanned: bool,
    selected: &std::collections::HashSet<String>,
    plan: &maintainer_core::updates::UpdatePlan,
) -> bool {
    use crate::ui::updates::SelectionMode;
    use maintainer_core::updates::UpdateIntent;
    let intent_matches = matches!(
        (mode, &plan.intent),
        (SelectionMode::Everything, UpdateIntent::FullUpgrade)
            | (SelectionMode::Custom, UpdateIntent::Selected(_))
    );
    intent_matches
        && replanned
        && selected.len() == plan.candidates.len()
        && plan
            .candidates
            .iter()
            .all(|candidate| selected.contains(&candidate.selection_key()))
}

fn candidate_heap(candidate: &maintainer_core::updates::UpdateCandidate) -> usize {
    candidate.name.capacity()
        + candidate.architecture.as_ref().map_or(0, String::capacity)
        + candidate.available_version.capacity()
        + candidate
            .installed_version
            .as_ref()
            .map_or(0, String::capacity)
}

fn updates_bytes(value: &UpdatesCheckOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            UpdatesCheckOutcome::Available { summary, plan, .. } => {
                summary.capacity()
                    + match &plan.intent {
                        maintainer_core::updates::UpdateIntent::FullUpgrade => 0,
                        maintainer_core::updates::UpdateIntent::Selected(requests) => {
                            vector_bytes(requests, |request| {
                                request.name.capacity()
                                    + request.version.capacity()
                                    + request.architecture.as_ref().map_or(0, String::capacity)
                            })
                        }
                    }
                    + plan.plan_id.capacity()
                    + plan.content_hash.capacity()
                    + vector_bytes(&plan.candidates, candidate_heap)
                    + vector_bytes(&plan.held, candidate_heap)
                    + vector_bytes(&plan.removals, |r| {
                        r.name.capacity() + r.installed_version.capacity()
                    })
            }
            UpdatesCheckOutcome::Failed { message } => message.capacity(),
            UpdatesCheckOutcome::UpToDate => 0,
        }
}

#[cfg(test)]
mod update_execution_tests {
    use maintainer_core::updates::{
        HoldState, UpdateBackend, UpdateCandidate, UpdateClassification, UpdatePlan,
    };
    use std::time::SystemTime;

    fn candidate(name: &str, available_version: &str) -> UpdateCandidate {
        UpdateCandidate {
            architecture: None,
            name: name.to_string(),
            installed_version: Some("1.0".to_string()),
            available_version: available_version.to_string(),
            classification: UpdateClassification::Standard,
            download_bytes: None,
            hold_state: HoldState::None,
            backend: UpdateBackend::Apt,
            is_new: false,
            is_kernel: false,
        }
    }

    fn plan(candidates: Vec<UpdateCandidate>) -> UpdatePlan {
        UpdatePlan {
            intent: maintainer_core::updates::UpdateIntent::FullUpgrade,
            plan_id: "test-plan".to_string(),
            created_at: SystemTime::UNIX_EPOCH,
            candidates,
            removals: Vec::new(),
            held: Vec::new(),
            total_download_bytes: None,
            content_hash: "test-hash".to_string(),
            reboot_required: false,
        }
    }

    #[test]
    fn security_filter_requires_resolver_review_even_with_matching_counts() {
        use crate::ui::updates::SelectionMode;
        let plan = plan(vec![candidate("curl", "2.0")]);
        let selected = ["curl".to_string()].into_iter().collect();
        assert!(super::update_selection_reviewed(
            SelectionMode::Everything,
            true,
            &selected,
            &plan
        ));
        assert!(!super::update_selection_reviewed(
            SelectionMode::SecurityOnly,
            true,
            &selected,
            &plan
        ));
        assert!(!super::update_selection_reviewed(
            SelectionMode::Custom,
            true,
            &selected,
            &plan
        ));
        let mut selected_plan = plan.clone();
        selected_plan.intent = maintainer_core::updates::UpdateIntent::Selected(vec![
            maintainer_core::updates::UpdateRequest {
                architecture: None,
                name: "curl".into(),
                version: "2.0".into(),
            },
        ]);
        assert!(!super::update_selection_reviewed(
            SelectionMode::Everything,
            true,
            &selected,
            &selected_plan
        ));
        assert!(super::update_selection_reviewed(
            SelectionMode::Custom,
            true,
            &selected,
            &selected_plan
        ));
        assert!(!super::update_selection_reviewed(
            SelectionMode::Custom,
            false,
            &selected,
            &selected_plan
        ));
    }
}
