//! Storage and duplicate scan jobs.

use std::path::PathBuf;
use std::sync::Arc;

use maintainer_core::jobs::{CancellationToken, JobExecutor};
use maintainer_core::storage::{DuplicateScanner, ScanPolicy, StorageScanner};

use super::{install_progress_poller, vector_bytes};
use crate::progress::SharedProgress;
use crate::state::ScanJobId;

/// Result of a storage scan job.
pub enum StorageScanResult {
    Completed {
        job_id: ScanJobId,
        large_files: Vec<maintainer_core::storage::LargeFile>,
        issues: Vec<maintainer_core::storage::ScanIssue>,
    },
    Cancelled {
        job_id: ScanJobId,
    },
    Failed {
        job_id: ScanJobId,
        message: String,
    },
}

/// Launch a storage scan through the shared `JobExecutor`.
///
/// Returns the cancellation token and the shared progress handle. The `on_done`
/// callback is invoked exactly once on the main thread when the job finishes.
/// Stale results (job_id mismatch) are discarded by the caller.
pub fn dispatch_storage_scan(
    executor: &Arc<JobExecutor>,
    job_id: ScanJobId,
    policy: ScanPolicy,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(StorageScanResult) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();
    let progress_for_worker = progress.clone();

    let job = move || -> maintainer_core::error::Result<StorageScanResult> {
        let scanner = StorageScanner::new(&policy, &token_for_worker, &progress_for_worker);

        match scanner.run() {
            Ok(summary) => {
                if summary.was_cancelled {
                    Ok(StorageScanResult::Cancelled { job_id })
                } else {
                    Ok(StorageScanResult::Completed {
                        job_id,
                        large_files: summary.large_files,
                        issues: summary.issues,
                    })
                }
            }
            Err(maintainer_core::error::MaintainerError::Cancelled) => {
                Ok(StorageScanResult::Cancelled { job_id })
            }
            Err(e) => Ok(StorageScanResult::Failed {
                job_id,
                message: e.to_string(),
            }),
        }
    };

    let rx = executor.execute_scan_sized(vec![job], token.clone(), storage_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, move |message| {
        StorageScanResult::Failed { job_id, message }
    });

    (token, progress)
}

/// Result of a duplicate scan job.
pub enum DuplicateScanJobResult {
    Completed {
        job_id: ScanJobId,
        groups: Vec<maintainer_core::storage::DuplicateGroup>,
        issues: Vec<maintainer_core::storage::DuplicateIssue>,
    },
    Cancelled {
        job_id: ScanJobId,
    },
    Failed {
        job_id: ScanJobId,
        message: String,
    },
}

/// Launch a duplicate scan through the shared `JobExecutor`.
pub fn dispatch_duplicate_scan(
    executor: &Arc<JobExecutor>,
    job_id: ScanJobId,
    policy: ScanPolicy,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(DuplicateScanJobResult) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();
    let progress_for_worker = progress.clone();

    let job = move || -> maintainer_core::error::Result<DuplicateScanJobResult> {
        let scanner = StorageScanner::new(&policy, &token_for_worker, &progress_for_worker)
            .with_duplicate_candidates();
        let scan_result = match scanner.run() {
            Ok(summary) => {
                if summary.was_cancelled {
                    return Ok(DuplicateScanJobResult::Cancelled { job_id });
                }
                summary
            }
            Err(maintainer_core::error::MaintainerError::Cancelled) => {
                return Ok(DuplicateScanJobResult::Cancelled { job_id });
            }
            Err(e) => {
                return Ok(DuplicateScanJobResult::Failed {
                    job_id,
                    message: e.to_string(),
                });
            }
        };

        let candidates = scan_result.duplicate_candidates;
        let metadata_issues = scan_result
            .issues
            .into_iter()
            .map(|issue| maintainer_core::storage::DuplicateIssue {
                path: issue.path,
                message: issue.message,
            })
            .collect::<Vec<_>>();

        if candidates.is_empty() {
            return Ok(DuplicateScanJobResult::Completed {
                job_id,
                groups: Vec::new(),
                issues: metadata_issues,
            });
        }

        let dup_scanner = DuplicateScanner::new(&token_for_worker, &progress_for_worker);
        match dup_scanner.scan(candidates) {
            Ok(mut dup_result) => {
                if dup_result.was_cancelled {
                    Ok(DuplicateScanJobResult::Cancelled { job_id })
                } else {
                    if dup_result.omitted_issues > 0 {
                        dup_result
                            .issues
                            .push(maintainer_core::storage::DuplicateIssue {
                                path: PathBuf::new(),
                                message: format!(
                                    "{} additional file issues omitted",
                                    dup_result.omitted_issues
                                ),
                            });
                    }
                    Ok(DuplicateScanJobResult::Completed {
                        job_id,
                        groups: dup_result.groups,
                        issues: metadata_issues
                            .into_iter()
                            .chain(dup_result.issues)
                            .collect(),
                    })
                }
            }
            Err(maintainer_core::error::MaintainerError::Cancelled) => {
                Ok(DuplicateScanJobResult::Cancelled { job_id })
            }
            Err(e) => Ok(DuplicateScanJobResult::Failed {
                job_id,
                message: e.to_string(),
            }),
        }
    };

    let rx = executor.execute_scan_sized(vec![job], token.clone(), duplicate_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, move |message| {
        DuplicateScanJobResult::Failed { job_id, message }
    });

    (token, progress)
}

fn storage_bytes(value: &StorageScanResult) -> usize {
    std::mem::size_of_val(value)
        + match value {
            StorageScanResult::Completed {
                large_files,
                issues,
                ..
            } => {
                vector_bytes(large_files, |f| f.path.capacity())
                    + vector_bytes(issues, |i| i.path.capacity() + i.message.capacity())
            }
            StorageScanResult::Failed { message, .. } => message.capacity(),
            StorageScanResult::Cancelled { .. } => 0,
        }
}

pub(super) fn duplicate_bytes(value: &DuplicateScanJobResult) -> usize {
    std::mem::size_of_val(value)
        + match value {
            DuplicateScanJobResult::Completed { groups, issues, .. } => {
                vector_bytes(groups, |g| vector_bytes(&g.files, |f| f.path.capacity()))
                    + vector_bytes(issues, |i| i.path.capacity() + i.message.capacity())
            }
            DuplicateScanJobResult::Failed { message, .. } => message.capacity(),
            DuplicateScanJobResult::Cancelled { .. } => 0,
        }
}
