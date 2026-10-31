//! Cleanup scan, execution and recovery jobs.

use std::path::PathBuf;
use std::sync::Arc;

use maintainer_core::jobs::{CancellationToken, JobExecutor};

use super::{install_progress_poller, vector_bytes};
use crate::progress::SharedProgress;

/// Result of a cleanup scan.
pub enum CleanupScanOutcome {
    Completed {
        plan: maintainer_core::cleanup::CleanupPlan,
    },
    Cancelled,
    Failed {
        message: String,
    },
}

/// Launch a cleanup scan through the shared `JobExecutor`.
pub fn dispatch_cleanup_scan(
    executor: &Arc<JobExecutor>,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(CleanupScanOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();
    let progress_for_worker = progress.clone();

    let job = move || -> maintainer_core::error::Result<CleanupScanOutcome> {
        use maintainer_core::cleanup::CleanupAnalyzer;

        let home_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));

        let analyzer = CleanupAnalyzer::new(home_dir.clone());

        match analyzer.scan_all(&token_for_worker, &progress_for_worker) {
            Ok(mut targets) => {
                targets.retain(|target| {
                    target.privilege == maintainer_core::cleanup::PrivilegeRequirement::None
                });
                let plan = match maintainer_core::cleanup::CleanupPlan::from_targets_cancellable(
                    targets,
                    &home_dir,
                    &token_for_worker,
                ) {
                    Ok(plan) => plan,
                    Err(maintainer_core::error::MaintainerError::Cancelled) => {
                        return Ok(CleanupScanOutcome::Cancelled);
                    }
                    Err(error) => return Err(error),
                };
                Ok(CleanupScanOutcome::Completed { plan })
            }
            Err(maintainer_core::error::MaintainerError::Cancelled) => {
                Ok(CleanupScanOutcome::Cancelled)
            }
            Err(e) => Ok(CleanupScanOutcome::Failed {
                message: e.to_string(),
            }),
        }
    };

    let rx = executor.execute_scan_sized(vec![job], token.clone(), cleanup_scan_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, |message| {
        CleanupScanOutcome::Failed { message }
    });

    (token, progress)
}

/// Result of executing a reviewed cleanup plan.
pub enum CleanupExecutionOutcome {
    Completed(maintainer_core::cleanup::CleanupResult),
    Failed { message: String },
}

/// Execute explicitly selected cleanup items after confirmation.
pub fn dispatch_cleanup_execution(
    executor: &Arc<JobExecutor>,
    plan: maintainer_core::cleanup::CleanupPlan,
    selected_indices: Vec<usize>,
    database: Arc<maintainer_core::persistence::database::Database>,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(CleanupExecutionOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let progress = SharedProgress::new();
    let worker_progress = progress.clone();

    let job = move || -> maintainer_core::error::Result<CleanupExecutionOutcome> {
        use maintainer_core::cleanup::{CleanupExecutor, ExecutionRequest};
        use maintainer_core::persistence::history::SqliteHistoryRepository;

        let home = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));
        let selected_item_ids = selected_indices
            .into_iter()
            .filter_map(|index| plan.items.get(index).map(|item| item.id))
            .collect();
        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids,
            confirm_permanent: true,
        };
        let history = SqliteHistoryRepository::new(&database);
        let cleanup = CleanupExecutor::new(&home);
        match cleanup.execute(&plan, &request, &worker_token, &worker_progress, &history) {
            Ok(result) => Ok(CleanupExecutionOutcome::Completed(result)),
            Err(error) => Ok(CleanupExecutionOutcome::Failed {
                message: error.to_string(),
            }),
        }
    };

    let receiver = executor.execute_sized(vec![job], token.clone(), cleanup_execution_bytes);
    install_progress_poller(
        receiver,
        progress.clone(),
        on_progress,
        on_done,
        |message| CleanupExecutionOutcome::Failed { message },
    );
    (token, progress)
}

/// On-demand recovery list or single-entry restoration result.
pub enum CleanupRecoveryOutcome {
    Listed(Vec<maintainer_core::cleanup::quarantine::Recovery>),
    Restored { notice: Option<String> },
    Failed(String),
}

pub fn dispatch_cleanup_recovery(
    executor: &Arc<JobExecutor>,
    restore: Option<(
        maintainer_core::cleanup::quarantine::Recovery,
        Option<PathBuf>,
    )>,
    on_done: impl FnOnce(CleanupRecoveryOutcome) + 'static,
) {
    let job = move || -> maintainer_core::error::Result<CleanupRecoveryOutcome> {
        let result = if let Some((item, elsewhere)) = restore {
            maintainer_core::cleanup::recovery::restore(&item, elsewhere.as_deref())
                .map(|notice| CleanupRecoveryOutcome::Restored { notice })
        } else {
            maintainer_core::cleanup::recovery::list().map(CleanupRecoveryOutcome::Listed)
        };
        Ok(result.unwrap_or_else(CleanupRecoveryOutcome::Failed))
    };
    let token = CancellationToken::new();
    let progress = SharedProgress::new();
    let rx = executor.execute_sized(vec![job], token, |outcome| {
        std::mem::size_of_val(outcome)
            + match outcome {
                CleanupRecoveryOutcome::Listed(rows) => vector_bytes(rows, |row| {
                    row.id.capacity() + row.basename.capacity() + row.parent_path.capacity()
                }),
                CleanupRecoveryOutcome::Failed(message) => message.capacity(),
                CleanupRecoveryOutcome::Restored { notice } => {
                    notice.as_ref().map_or(0, String::capacity)
                }
            }
    });
    install_progress_poller(
        rx,
        progress,
        |_| {},
        on_done,
        CleanupRecoveryOutcome::Failed,
    );
}

fn cleanup_scan_bytes(value: &CleanupScanOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            CleanupScanOutcome::Completed { plan } => vector_bytes(&plan.items, |t| {
                t.path.capacity() + t.display_path.capacity()
            }),
            CleanupScanOutcome::Cancelled => 0,
            CleanupScanOutcome::Failed { message } => message.capacity(),
        }
}

fn cleanup_execution_bytes(value: &CleanupExecutionOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            CleanupExecutionOutcome::Completed(result) => {
                vector_bytes(&result.item_results, |item| match item {
                    maintainer_core::jobs::ItemResult::Ok(item) => {
                        item.notice.as_ref().map_or(0, String::capacity)
                    }
                    maintainer_core::jobs::ItemResult::Failed { item, message } => {
                        item.capacity() + message.capacity()
                    }
                    maintainer_core::jobs::ItemResult::Skipped { item, reason } => {
                        item.capacity() + reason.capacity()
                    }
                })
            }
            CleanupExecutionOutcome::Failed { message } => message.capacity(),
        }
}
