use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use glib::ControlFlow;

use maintainer_core::jobs::{CancellationToken, JobExecutor, JobMessage};
use maintainer_core::storage::{DuplicateScanner, ScanPolicy, StorageScanner};

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

/// Diagnostics scan result.
pub enum DiagnosticsScanOutcome {
    Completed {
        findings: Vec<maintainer_core::diagnostics::DiagnosticFinding>,
    },
    Failed {
        message: String,
    },
}

/// Launch a diagnostics scan through the shared `JobExecutor`.
pub fn dispatch_diagnostics_scan(
    executor: &Arc<JobExecutor>,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(DiagnosticsScanOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();
    let _progress_for_worker = progress.clone();

    let job = move || -> maintainer_core::error::Result<DiagnosticsScanOutcome> {
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::diagnostics::{
            JournalCollector, JournalFilter, ServiceDiagnostics, group_journal_entries,
            interpret_groups,
        };

        let runner = SystemCommandRunner;
        let failed_services = match ServiceDiagnostics::list_failed(&runner, &token_for_worker) {
            maintainer_core::capability::Capability::Available(services) => services,
            _ => Vec::new(),
        };
        let filter = JournalFilter::default();
        let journal = JournalCollector::collect(&filter, &runner, &token_for_worker);

        let findings = match journal {
            maintainer_core::capability::Capability::Available(entries) => {
                let groups = group_journal_entries(&entries);
                interpret_groups(&groups, &failed_services)
            }
            _ => interpret_groups(&[], &failed_services),
        };

        Ok(DiagnosticsScanOutcome::Completed { findings })
    };

    let rx = executor.execute_sized(vec![job], token.clone(), diagnostics_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, |message| {
        DiagnosticsScanOutcome::Failed { message }
    });

    (token, progress)
}

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

/// Local action failures include scheduler rejection and worker termination.
pub trait LocalActionError: Send + 'static {
    fn from_job_failure(message: String) -> Self;
    fn retained_bytes(&self) -> usize;
}

impl LocalActionError for String {
    fn from_job_failure(message: String) -> Self {
        message
    }
    fn retained_bytes(&self) -> usize {
        self.capacity()
    }
}

impl LocalActionError for std::io::Error {
    fn from_job_failure(message: String) -> Self {
        Self::other(message)
    }
    fn retained_bytes(&self) -> usize {
        // io::Error hides its allocation; only bounded local filesystem errors are used here.
        4096
    }
}

/// Run a local blocking action and deliver its typed result on the GTK main thread.
pub fn dispatch_local_action<T: Send + 'static, E: LocalActionError>(
    executor: &Arc<JobExecutor>,
    operation: impl FnOnce() -> Result<T, E> + Send + 'static,
    on_done: impl FnOnce(Result<T, E>) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<Result<T, E>> { Ok(operation()) };
    let receiver = executor.execute_sized(vec![job], token.clone(), |value| {
        std::mem::size_of_val(value)
            + value
                .as_ref()
                .err()
                .map_or(0, LocalActionError::retained_bytes)
    });
    install_result_poller(receiver, on_done, |message| {
        Err(E::from_job_failure(message))
    });
    token
}

pub fn dispatch_helper_call(
    executor: &Arc<JobExecutor>,
    operation: impl FnOnce() -> crate::helper_client::HelperResult + Send + 'static,
    on_done: impl FnOnce(crate::helper_client::HelperResult) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<crate::helper_client::HelperResult> {
        Ok(operation())
    };
    let receiver = executor.execute_sized(vec![job], token.clone(), helper_bytes);
    install_result_poller(receiver, on_done, helper_failure);
    token
}

/// Result of refreshing SMART data through the privileged helper.
pub enum SmartRefreshOutcome {
    Loaded(Vec<maintainer_core::diagnostics::SmartDrive>),
    PermissionRequired,
    Unavailable(String),
}

/// Discover drives, refresh each fixed UDisks object through the helper, then reload data.
pub fn dispatch_smart_refresh(
    executor: &Arc<JobExecutor>,
    helper: crate::helper_client::SharedHelperClient,
    on_done: impl FnOnce(SmartRefreshOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<SmartRefreshOutcome> {
        use maintainer_core::capability::Capability;
        use maintainer_core::diagnostics::{RealUDisksClient, query_smart_drives};

        const DRIVE_PREFIX: &str = "/org/freedesktop/UDisks2/drives/";
        let client = RealUDisksClient::new();
        let drives = match query_smart_drives(&client) {
            Capability::Available(drives) => drives,
            Capability::Unavailable(reason) => {
                return Ok(SmartRefreshOutcome::Unavailable(reason.message));
            }
            Capability::PermissionRequired(_) => {
                return Ok(SmartRefreshOutcome::PermissionRequired);
            }
        };

        for drive in &drives {
            if drive.removable {
                continue;
            }
            let Some(drive_id) = drive.object_path.strip_prefix(DRIVE_PREFIX) else {
                continue;
            };
            match helper.refresh_smart(drive_id) {
                Capability::Available(_) => {}
                Capability::Unavailable(reason) => {
                    return Ok(SmartRefreshOutcome::Unavailable(reason.message));
                }
                Capability::PermissionRequired(_) => {
                    return Ok(SmartRefreshOutcome::PermissionRequired);
                }
            }
        }

        match query_smart_drives(&client) {
            Capability::Available(drives) => Ok(SmartRefreshOutcome::Loaded(drives)),
            Capability::Unavailable(reason) => Ok(SmartRefreshOutcome::Unavailable(reason.message)),
            Capability::PermissionRequired(_) => Ok(SmartRefreshOutcome::PermissionRequired),
        }
    };
    let receiver = executor.execute_sized(vec![job], token.clone(), smart_bytes);
    install_result_poller(receiver, on_done, SmartRefreshOutcome::Unavailable);
    token
}

/// Reports generation result.
pub enum ReportOutcome {
    Completed { text: String },
    Failed { message: String },
}

/// Launch report generation through the shared `JobExecutor`.
pub fn dispatch_report_generation(
    executor: &Arc<JobExecutor>,
    database: Option<Arc<maintainer_core::persistence::database::Database>>,
    on_done: impl FnOnce(ReportOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let worker_token = token.clone();

    let job = move || -> maintainer_core::error::Result<ReportOutcome> {
        use std::path::Path;
        use std::time::Instant;

        use maintainer_core::capability::{Capability, UnavailableReason};
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::diagnostics::{
            JournalCollector, JournalFilter, RealUDisksClient, ServiceDiagnostics,
            group_journal_entries, query_smart_drives,
        };
        use maintainer_core::metrics::disk::read_mounts_with_capacity;
        use maintainer_core::metrics::{
            NetworkSnapshot, compute_cpu_snapshot, read_cpu_sample, read_memory, read_net_dev,
            read_temperatures,
        };
        use maintainer_core::persistence::history::{HistoryRepository, SqliteHistoryRepository};
        use maintainer_core::reports::model::{
            DiagnosticsSection, HardwareSection, HealthSection, MaintenanceSection,
            RedactionPolicy, SoftwareUpdatesSection, SystemReport, UpdateInfo,
        };
        use maintainer_core::reports::{redact_report, render_text};
        use maintainer_core::updates::{AptDiscovery, DiscoveryResult};

        fn unavailable<T>(code: &'static str, message: String) -> Capability<T> {
            Capability::Unavailable(UnavailableReason {
                code,
                message,
                remediation: None,
            })
        }

        let proc_path = Path::new("/proc");
        let sys_path = Path::new("/sys");
        let first_cpu = read_cpu_sample(proc_path, Instant::now());
        std::thread::sleep(Duration::from_millis(100));
        let cpu = match (first_cpu, read_cpu_sample(proc_path, Instant::now())) {
            (Ok(first), Ok(second)) => Capability::Available(compute_cpu_snapshot(&first, &second)),
            (Err(error), _) | (_, Err(error)) => unavailable("proc-stat-read", error.to_string()),
        };
        let memory = read_memory(proc_path)
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("meminfo-read", error.to_string()));
        let disks = read_mounts_with_capacity(proc_path)
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("mounts-read", error.to_string()));
        let network = read_net_dev(proc_path)
            .map(|counters| {
                counters
                    .into_iter()
                    .map(|(interface, value)| NetworkSnapshot {
                        is_loopback: interface == "lo",
                        interface,
                        rx_bytes: value.rx_bytes,
                        tx_bytes: value.tx_bytes,
                        rx_packets: value.rx_packets,
                        tx_packets: value.tx_packets,
                        rx_errors: value.rx_errors,
                        tx_errors: value.tx_errors,
                        rx_bytes_per_sec: None,
                        tx_bytes_per_sec: None,
                    })
                    .collect()
            })
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("net-dev-read", error.to_string()));
        let temperatures = read_temperatures(sys_path)
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("hwmon-read", error.to_string()));
        let smart_drives = query_smart_drives(&RealUDisksClient::new());

        let runner = SystemCommandRunner;
        let failed_services = ServiceDiagnostics::list_failed(&runner, &worker_token);
        let event_groups =
            match JournalCollector::collect(&JournalFilter::default(), &runner, &worker_token) {
                Capability::Available(entries) => {
                    Capability::Available(group_journal_entries(&entries))
                }
                Capability::Unavailable(reason) => Capability::Unavailable(reason),
                Capability::PermissionRequired(request) => Capability::PermissionRequired(request),
            };
        // Same discovery mode as the Updates page, so the count shown here
        // matches what a user would see there rather than under-reporting.
        let pending_updates = match AptDiscovery::check_full_upgrade(&runner, &worker_token) {
            Capability::Available(DiscoveryResult::Available(plan)) => Capability::Available(
                plan.candidates
                    .into_iter()
                    .map(|candidate| UpdateInfo {
                        package_name: candidate.name,
                        current_version: candidate
                            .installed_version
                            .unwrap_or_else(|| "not installed".to_string()),
                        available_version: candidate.available_version,
                    })
                    .collect(),
            ),
            Capability::Available(DiscoveryResult::UpToDate) => Capability::Available(Vec::new()),
            Capability::Available(DiscoveryResult::Failed(error)) => {
                unavailable("apt-check-failed", error.to_string())
            }
            Capability::Unavailable(reason) => Capability::Unavailable(reason),
            Capability::PermissionRequired(request) => Capability::PermissionRequired(request),
        };
        let recent_history = database
            .as_deref()
            .and_then(|database| SqliteHistoryRepository::new(database).recent(20).ok())
            .unwrap_or_default();

        let report = SystemReport {
            generated_at: chrono_like_now(),
            os: build_os_section(),
            hardware: HardwareSection {
                cpu,
                memory,
                disks,
                network,
                temperatures,
            },
            health: HealthSection { smart_drives },
            software_updates: SoftwareUpdatesSection { pending_updates },
            diagnostics: DiagnosticsSection {
                failed_services,
                event_groups,
            },
            maintenance: MaintenanceSection { recent_history },
            capability_notes: Vec::new(),
        };
        let text = render_text(&redact_report(report, &RedactionPolicy::default()));
        Ok(ReportOutcome::Completed { text })
    };

    let receiver = executor.execute_sized(vec![job], token.clone(), report_bytes);
    install_result_poller(receiver, on_done, |message| ReportOutcome::Failed {
        message,
    });
    token
}

/// Convert every terminal transport state into one domain result.
fn terminal_result<T>(
    message: Result<JobMessage<T>, std::sync::mpsc::TryRecvError>,
    on_error: &impl Fn(String) -> T,
) -> Option<T> {
    match message {
        Ok(JobMessage::Completed(value)) => Some(value),
        Ok(JobMessage::Failed(error)) => Some(on_error(error.to_string())),
        Ok(JobMessage::Panicked(_)) => Some(on_error(
            "The background operation stopped unexpectedly. Try again.".into(),
        )),
        Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(on_error(
            "The background operation ended without a result. Try again.".into(),
        )),
        Err(std::sync::mpsc::TryRecvError::Empty) => None,
    }
}

fn install_result_poller<T: 'static>(
    rx: maintainer_core::jobs::JobReceiver<T>,
    on_done: impl FnOnce(T) + 'static,
    on_error: impl Fn(String) -> T + 'static,
) {
    install_progress_poller(rx, SharedProgress::new(), |_| {}, on_done, on_error);
}

fn install_progress_poller<T: 'static>(
    rx: maintainer_core::jobs::JobReceiver<T>,
    progress: SharedProgress,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(T) + 'static,
    on_error: impl Fn(String) -> T + 'static,
) {
    let mut on_done = Some(on_done);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        if let Some(p) = progress.take_latest() {
            on_progress(&p);
        }
        if let Some(value) = terminal_result(rx.try_recv(), &on_error) {
            if let Some(p) = progress.take_latest() {
                on_progress(&p);
            }
            if let Some(callback) = on_done.take() {
                callback(value);
            }
            ControlFlow::Break
        } else {
            ControlFlow::Continue
        }
    });
}

// Count reserved container storage, including unused capacity, plus nested allocations.
fn vector_bytes<T>(values: &Vec<T>, heap: impl Fn(&T) -> usize) -> usize {
    values
        .capacity()
        .saturating_mul(std::mem::size_of::<T>())
        .saturating_add(values.iter().map(heap).fold(0usize, usize::saturating_add))
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

fn duplicate_bytes(value: &DuplicateScanJobResult) -> usize {
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

fn diagnostics_bytes(value: &DiagnosticsScanOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            DiagnosticsScanOutcome::Completed { findings } => vector_bytes(findings, |f| {
                f.detected.capacity()
                    + vector_bytes(&f.evidence, |e| e.observation.capacity())
                    + vector_bytes(&f.possible_causes, String::capacity)
                    + f.recommendation.as_ref().map_or(0, String::capacity)
            }),
            DiagnosticsScanOutcome::Failed { message } => message.capacity(),
        }
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

fn helper_failure(message: String) -> crate::helper_client::HelperResult {
    maintainer_core::capability::Capability::Unavailable(
        maintainer_core::capability::UnavailableReason {
            code: "background-operation-failed",
            message,
            remediation: None,
        },
    )
}

fn helper_bytes(value: &crate::helper_client::HelperResult) -> usize {
    use maintainer_core::capability::Capability;
    std::mem::size_of_val(value)
        + match value {
            Capability::Available(response) => response.message.capacity(),
            Capability::Unavailable(reason) => {
                reason.message.capacity() + reason.remediation.as_ref().map_or(0, String::capacity)
            }
            Capability::PermissionRequired(request) => {
                request.action_id.capacity() + request.explanation.capacity()
            }
        }
}

fn smart_bytes(value: &SmartRefreshOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            SmartRefreshOutcome::Loaded(drives) => vector_bytes(drives, |d| {
                d.object_path.capacity()
                    + d.model.capacity()
                    + d.firmware.capacity()
                    + d.serial.capacity()
                    + vector_bytes(&d.attributes, |a| a.name.capacity())
            }),
            SmartRefreshOutcome::Unavailable(message) => message.capacity(),
            SmartRefreshOutcome::PermissionRequired => 0,
        }
}

fn report_bytes(value: &ReportOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            ReportOutcome::Completed { text } => text.capacity(),
            ReportOutcome::Failed { message } => message.capacity(),
        }
}

#[cfg(test)]
mod terminal_tests {
    use super::*;

    #[test]
    fn failures_hide_panic_payloads_and_handle_disconnect() {
        let outcome = terminal_result::<ReportOutcome>(
            Ok(JobMessage::Panicked("private path".into())),
            &|message| ReportOutcome::Failed { message },
        );
        let Some(ReportOutcome::Failed { message }) = outcome else {
            panic!("missing terminal failure")
        };
        assert!(!message.contains("private path"));
        assert!(
            terminal_result::<String>(Err(std::sync::mpsc::TryRecvError::Empty), &|s| s).is_none()
        );
        assert!(
            terminal_result::<String>(Err(std::sync::mpsc::TryRecvError::Disconnected), &|s| s)
                .is_some()
        );
    }

    #[test]
    fn scan_failure_retains_job_identity() {
        let result = terminal_result(Ok(JobMessage::Panicked("private".into())), &|message| {
            StorageScanResult::Failed {
                job_id: ScanJobId(42),
                message,
            }
        });
        assert!(matches!(
            result,
            Some(StorageScanResult::Failed {
                job_id: ScanJobId(42),
                ..
            })
        ));
    }

    #[test]
    fn scheduler_failure_becomes_domain_failure() {
        let result = terminal_result(
            Ok(JobMessage::Failed(
                maintainer_core::error::MaintainerError::CommandFailed {
                    message: "Busy".into(),
                },
            )),
            &|message| ReportOutcome::Failed { message },
        );
        assert!(matches!(result, Some(ReportOutcome::Failed { .. })));
    }

    #[test]
    fn retained_storage_counts_reserved_capacity() {
        let mut text = String::with_capacity(4096);
        text.push('x');
        assert!(report_bytes(&ReportOutcome::Completed { text }) >= 4096);
        let groups = Vec::with_capacity(100);
        assert!(
            duplicate_bytes(&DuplicateScanJobResult::Completed {
                job_id: ScanJobId(1),
                groups,
                issues: Vec::new()
            }) >= 100 * std::mem::size_of::<maintainer_core::storage::DuplicateGroup>()
        );
    }
}
fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_time_utc(duration.as_secs())
}

fn format_unix_time_utc(secs: u64) -> String {
    let mut days = (secs / 86400) as i64;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 142401) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}

fn build_os_section() -> maintainer_core::reports::model::OsSection {
    use std::fs;

    let distro = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let kernel = fs::read_to_string("/proc/version")
        .unwrap_or_default()
        .split_whitespace()
        .nth(2)
        .unwrap_or("unknown")
        .to_string();

    let uptime_seconds = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|c| {
            c.split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
        })
        .map(|s| s as u64)
        .unwrap_or(0);

    let hostname = fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_string();

    let username = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();

    maintainer_core::reports::model::OsSection {
        distro,
        kernel,
        uptime_seconds,
        desktop,
        hostname,
        username,
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
