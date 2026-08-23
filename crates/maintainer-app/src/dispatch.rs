use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use glib::ControlFlow;

use maintainer_core::jobs::{CancellationToken, JobExecutor, JobMessage, ProgressSink};
use maintainer_core::storage::{DuplicateCandidate, DuplicateScanner, ScanPolicy, StorageScanner};

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

    let rx = executor.execute(vec![job]);
    install_progress_poller(rx, progress.clone(), on_progress, on_done);

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
        let scanner = StorageScanner::new(&policy, &token_for_worker, &progress_for_worker);
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

        let candidates: Vec<DuplicateCandidate> = scan_result
            .large_files
            .iter()
            .map(|f| DuplicateCandidate {
                path: f.path.clone(),
                size_bytes: f.size_bytes,
                identity: maintainer_core::storage::DeviceInode {
                    device: 0,
                    inode: 0,
                },
                modified: std::time::SystemTime::now(),
            })
            .collect();

        if candidates.is_empty() {
            return Ok(DuplicateScanJobResult::Completed {
                job_id,
                groups: Vec::new(),
                issues: Vec::new(),
            });
        }

        let dup_scanner = DuplicateScanner::new(&token_for_worker, &progress_for_worker);
        match dup_scanner.scan(candidates) {
            Ok(dup_result) => {
                if dup_result.was_cancelled {
                    Ok(DuplicateScanJobResult::Cancelled { job_id })
                } else {
                    Ok(DuplicateScanJobResult::Completed {
                        job_id,
                        groups: dup_result.groups,
                        issues: dup_result.issues,
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

    let rx = executor.execute(vec![job]);
    install_progress_poller(rx, progress.clone(), on_progress, on_done);

    (token, progress)
}

/// Result of a cleanup scan.
pub enum CleanupScanOutcome {
    Completed {
        targets: Vec<maintainer_core::cleanup::CleanupTarget>,
    },
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

        let analyzer = CleanupAnalyzer::new(home_dir);

        match analyzer.scan_all(&token_for_worker, &progress_for_worker) {
            Ok(mut targets) => {
                targets.retain(|target| {
                    target.privilege == maintainer_core::cleanup::PrivilegeRequirement::None
                });
                Ok(CleanupScanOutcome::Completed { targets })
            }
            Err(maintainer_core::error::MaintainerError::Cancelled) => {
                Ok(CleanupScanOutcome::Failed {
                    message: "Scan cancelled".to_string(),
                })
            }
            Err(e) => Ok(CleanupScanOutcome::Failed {
                message: e.to_string(),
            }),
        }
    };

    let rx = executor.execute(vec![job]);
    install_progress_poller(rx, progress.clone(), on_progress, on_done);

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

    let receiver = executor.execute(vec![job]);
    install_progress_poller(receiver, progress.clone(), on_progress, on_done);
    (token, progress)
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

    let rx = executor.execute(vec![job]);
    install_progress_poller(rx, progress.clone(), on_progress, on_done);

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

    let rx = executor.execute(vec![job]);
    install_progress_poller(rx, progress.clone(), on_progress, on_done);

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

    let rx = executor.execute(vec![job]);
    install_progress_poller(rx, progress.clone(), on_progress, on_done);

    (token, progress)
}

/// Progress text used while pkexec may still be safely cancelled before APT starts.
pub const UPDATE_AUTHENTICATION_PROGRESS: &str = "Waiting for administrator authentication…";

/// Result of executing a reviewed update plan.
pub enum UpdateExecutionOutcome {
    Completed {
        message: String,
        restart_required: bool,
    },
    Failed {
        message: String,
    },
}

fn current_plan_candidate_count(
    current: &maintainer_core::capability::Capability<maintainer_core::updates::DiscoveryResult>,
) -> usize {
    match current {
        maintainer_core::capability::Capability::Available(
            maintainer_core::updates::DiscoveryResult::Available(p),
        ) => p.candidates.len(),
        _ => 0,
    }
}

fn build_apt_update_command(
    plan: &maintainer_core::updates::UpdatePlan,
    current_candidate_count: usize,
) -> Result<maintainer_core::command::CommandSpec, maintainer_core::error::CommandSpecError> {
    let mut arguments = vec![
        "/usr/libexec/maintainer-helper".to_string(),
        "install-updates".to_string(),
    ];
    if !plan.candidates.is_empty() && plan.candidates.len() < current_candidate_count {
        arguments.push("selected".to_string());
        arguments.extend(
            plan.candidates
                .iter()
                .map(|candidate| format!("{}={}", candidate.name, candidate.available_version)),
        );
    } else {
        arguments.push("full".to_string());
    }

    maintainer_core::command::CommandSpec::new("/usr/bin/pkexec", arguments)
        .map(|command| command.with_timeout(Duration::from_secs(6 * 60 * 60)))
}

fn verify_installed_versions(
    runner: &dyn maintainer_core::command::CommandRunner,
    cancel: &dyn maintainer_core::jobs::Cancellation,
    candidates: &[maintainer_core::updates::UpdateCandidate],
) -> Result<(), String> {
    let mut mismatches = Vec::new();

    for candidate in candidates {
        let command = maintainer_core::command::CommandSpec::new(
            "/usr/bin/dpkg-query",
            [
                "-W".to_string(),
                "-f=${Version}".to_string(),
                "--".to_string(),
                candidate.name.clone(),
            ],
        )
        .map_err(|error| {
            format!(
                "cannot construct dpkg-query for {}: {error}",
                candidate.name
            )
        })?
        .with_timeout(Duration::from_secs(30));

        let output = runner.run(&command, cancel).map_err(|error| {
            format!(
                "cannot query installed version of {}: {error}",
                candidate.name
            )
        })?;
        if !output.success() {
            let detail = output
                .stderr
                .lines()
                .find(|line| !line.trim().is_empty())
                .unwrap_or("dpkg-query returned a non-zero exit code");
            mismatches.push(format!("{}: {detail}", candidate.name));
            continue;
        }

        let installed = output.stdout.trim();
        if installed != candidate.available_version {
            let found = if installed.is_empty() {
                "not installed"
            } else {
                installed
            };
            mismatches.push(format!(
                "{} expected {}, found {found}",
                candidate.name, candidate.available_version
            ));
        }
    }

    if mismatches.is_empty() {
        Ok(())
    } else {
        Err(mismatches.join("; "))
    }
}
/// Revalidate and execute an immutable update plan.
pub fn dispatch_update_execution(
    executor: &Arc<JobExecutor>,

    plan: maintainer_core::updates::UpdatePlan,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(UpdateExecutionOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let worker_token = token.clone();
    let progress = SharedProgress::new();
    let worker_progress = progress.clone();

    let job = move || -> maintainer_core::error::Result<UpdateExecutionOutcome> {
        use maintainer_core::capability::Capability;
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::updates::{AptDiscovery, DiscoveryResult};

        let runner = SystemCommandRunner;
        // Must use the same discovery mode as the plan being verified
        // (check_full_upgrade), or content_hash will never match and every
        // install would be rejected as "changed since review".
        let current = AptDiscovery::check_full_upgrade(&runner, &worker_token);
        match &current {
            Capability::Available(DiscoveryResult::Available(current_plan))
                if current_plan.content_hash == plan.content_hash => {}
            Capability::Available(DiscoveryResult::UpToDate) => {
                return Ok(UpdateExecutionOutcome::Completed {
                    message: "The system is already up to date.".to_string(),
                    restart_required: false,
                });
            }
            Capability::Available(DiscoveryResult::Available(_)) => {
                return Ok(UpdateExecutionOutcome::Failed {
                    message: "Available updates changed. Check again before installing."
                        .to_string(),
                });
            }
            Capability::Available(DiscoveryResult::Failed(error)) => {
                return Ok(UpdateExecutionOutcome::Failed {
                    message: error.to_string(),
                });
            }
            Capability::Unavailable(reason) => {
                return Ok(UpdateExecutionOutcome::Failed {
                    message: reason.message.clone(),
                });
            }
            Capability::PermissionRequired(request) => {
                return Ok(UpdateExecutionOutcome::Failed {
                    message: request.explanation.clone(),
                });
            }
        }

        if plan.candidates.iter().any(|candidate| {
            candidate.name.is_empty()
                || !candidate
                    .name
                    .chars()
                    .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.' | ':'))
        }) {
            return Ok(UpdateExecutionOutcome::Failed {
                message: "The reviewed plan contains an invalid package name.".to_string(),
            });
        }

        worker_progress.report(&maintainer_core::jobs::JobProgress {
            completed: 0,
            total: None,
            message: UPDATE_AUTHENTICATION_PROGRESS.to_string(),
        });

        use maintainer_core::command::run_streaming;
        use maintainer_core::updates::apt_parser::parse_apt_progress_line;

        let command = build_apt_update_command(&plan, current_plan_candidate_count(&current))
            .map_err(
                |error| maintainer_core::error::MaintainerError::CommandFailed {
                    message: error.to_string(),
                },
            )?;

        let mut current_status_msg = "Installing updates…".to_string();
        let stream_result = run_streaming(&command, &worker_token, |line| {
            if let Some(parsed) = parse_apt_progress_line(line) {
                let percentage = parsed.percentage();
                current_status_msg = parsed.display_message();
                worker_progress.report(&maintainer_core::jobs::JobProgress {
                    completed: percentage.unwrap_or(0),
                    total: percentage.map(|_| 100),
                    message: current_status_msg.clone(),
                });
            }
        });

        match stream_result {
            Ok(output) if output.success() => {
                worker_progress.report(&maintainer_core::jobs::JobProgress {
                    completed: 1,
                    total: Some(1),
                    message: "Verifying installed package versions…".to_string(),
                });

                // Once APT has exited successfully, verification must complete
                // even if the user pressed Cancel at the end of the transaction.
                // Otherwise cancellation could suppress the only check that
                // prevents a false success result.
                let verification_cancel = CancellationToken::new();
                if let Err(error) =
                    verify_installed_versions(&runner, &verification_cancel, &plan.candidates)
                {
                    return Ok(UpdateExecutionOutcome::Failed {
                        message: format!(
                            "APT exited successfully, but installed package verification failed: \
                             {error}. No success was recorded."
                        ),
                    });
                }

                let combined = format!("{}\n{}", output.stdout, output.stderr);
                let restart_required = combined.to_ascii_lowercase().contains("restart")
                    || maintainer_core::updates::reboot_required(std::path::Path::new("/var/run"));
                Ok(UpdateExecutionOutcome::Completed {
                    message: "Updates installed and verified successfully.".to_string(),
                    restart_required,
                })
            }
            Ok(output) => {
                let err_combined = format!("{}\n{}", output.stderr, output.stdout);
                if err_combined.contains("dismissed")
                    || err_combined.contains("not authorized")
                    || err_combined.contains("Authentication cancelled")
                    || output.exit_code == Some(126)
                    || output.exit_code == Some(127)
                {
                    return Ok(UpdateExecutionOutcome::Failed {
                        message: "Authentication was cancelled.".to_string(),
                    });
                }
                if err_combined.contains("Could not get lock")
                    || err_combined.contains("Resource temporarily unavailable")
                {
                    return Ok(UpdateExecutionOutcome::Failed {
                        message: "Another package manager is currently running. Try again later."
                            .to_string(),
                    });
                }
                let message = output
                    .stderr
                    .lines()
                    .chain(output.stdout.lines())
                    .find(|line| !line.trim().is_empty())
                    .unwrap_or("Failed to install updates")
                    .to_string();
                Ok(UpdateExecutionOutcome::Failed { message })
            }
            Err(maintainer_core::error::MaintainerError::Cancelled) => {
                Ok(UpdateExecutionOutcome::Failed {
                    message: "Update cancelled.".to_string(),
                })
            }
            Err(error) => Ok(UpdateExecutionOutcome::Failed {
                message: error.to_string(),
            }),
        }
    };

    let receiver = executor.execute(vec![job]);
    install_progress_poller(receiver, progress.clone(), on_progress, on_done);
    (token, progress)
}

/// Run a blocking privileged-helper call on the bounded worker pool.
/// Run a local blocking action on the bounded worker pool and deliver its
/// typed result on the GTK main thread.
pub fn dispatch_local_action<T: Send + 'static>(
    executor: &Arc<JobExecutor>,
    operation: impl FnOnce() -> T + Send + 'static,
    on_done: impl FnOnce(T) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<T> { Ok(operation()) };
    let receiver = executor.execute(vec![job]);
    install_result_poller(receiver, on_done);
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
    let receiver = executor.execute(vec![job]);
    install_result_poller(receiver, on_done);
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
    let receiver = executor.execute(vec![job]);
    install_result_poller(receiver, on_done);
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

    let receiver = executor.execute(vec![job]);
    install_result_poller(receiver, on_done);
    token
}

/// Installs a short-interval timer that polls the receiver for a single result
/// and delivers it to the on_done callback on the main thread.
fn install_result_poller<T: 'static>(
    rx: std::sync::mpsc::Receiver<JobMessage<T>>,
    on_done: impl FnOnce(T) + 'static,
) {
    let mut on_done = Some(on_done);
    glib::timeout_add_local(Duration::from_millis(50), move || match rx.try_recv() {
        Ok(JobMessage::Completed(value)) => {
            if let Some(cb) = on_done.take() {
                cb(value);
            }
            ControlFlow::Break
        }
        Ok(JobMessage::Failed(e)) => {
            eprintln!("job failed: {e}");
            ControlFlow::Break
        }
        Ok(JobMessage::Panicked(msg)) => {
            eprintln!("job panicked: {msg}");
            ControlFlow::Break
        }
        Err(std::sync::mpsc::TryRecvError::Empty) => ControlFlow::Continue,
        Err(std::sync::mpsc::TryRecvError::Disconnected) => ControlFlow::Break,
    });
}

/// Installs a timer that both polls progress (via `SharedProgress`) and
/// checks for job completion. Progress callbacks are invoked on the main
/// thread at the polling interval, independent of cancel responsiveness.
fn install_progress_poller<T: 'static>(
    rx: std::sync::mpsc::Receiver<JobMessage<T>>,
    progress: SharedProgress,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(T) + 'static,
) {
    let mut on_done = Some(on_done);
    glib::timeout_add_local(Duration::from_millis(50), move || {
        // Poll progress every tick — cheap because SharedProgress throttles internally.
        if let Some(p) = progress.take_latest() {
            on_progress(&p);
        }

        match rx.try_recv() {
            Ok(JobMessage::Completed(value)) => {
                if let Some(p) = progress.take_latest() {
                    on_progress(&p);
                }
                if let Some(cb) = on_done.take() {
                    cb(value);
                }
                ControlFlow::Break
            }
            Ok(JobMessage::Failed(e)) => {
                eprintln!("job failed: {e}");
                ControlFlow::Break
            }
            Ok(JobMessage::Panicked(msg)) => {
                eprintln!("job panicked: {msg}");
                ControlFlow::Break
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => ControlFlow::Continue,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => ControlFlow::Break,
        }
    });
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
    use super::build_apt_update_command;
    use maintainer_core::updates::{
        HoldState, UpdateBackend, UpdateCandidate, UpdateClassification, UpdatePlan,
    };
    use std::time::SystemTime;

    fn candidate(name: &str, available_version: &str) -> UpdateCandidate {
        UpdateCandidate {
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
    fn full_plan_uses_pkexec_with_fixed_helper_operation() {
        let plan = plan(vec![candidate("curl", "2.0"), candidate("wget", "3.0")]);
        let command = build_apt_update_command(&plan, 2).unwrap();

        assert_eq!(command.program(), "/usr/bin/pkexec");
        assert_eq!(
            command.args(),
            ["/usr/libexec/maintainer-helper", "install-updates", "full"]
        );
        assert!(!command.args().iter().any(|arg| arg.contains("apt-get")));
    }

    #[test]
    fn selected_plan_passes_only_exact_reviewed_versions_to_helper() {
        let plan = plan(vec![candidate("curl", "2.0")]);
        let command = build_apt_update_command(&plan, 2).unwrap();

        assert_eq!(command.program(), "/usr/bin/pkexec");
        assert_eq!(
            command.args(),
            [
                "/usr/libexec/maintainer-helper",
                "install-updates",
                "selected",
                "curl=2.0"
            ]
        );
        assert!(!command.args().iter().any(|arg| arg.contains("apt-get")));
    }
}

#[cfg(test)]
mod installed_version_verification_tests {
    use std::sync::Mutex;

    use maintainer_core::command::{CommandOutput, CommandRunner, CommandSpec};
    use maintainer_core::jobs::Cancellation;
    use maintainer_core::updates::{
        HoldState, UpdateBackend, UpdateCandidate, UpdateClassification,
    };

    use super::verify_installed_versions;

    struct FixedRunner {
        output: CommandOutput,
        calls: Mutex<Vec<Vec<String>>>,
    }

    impl CommandRunner for FixedRunner {
        fn run(
            &self,
            spec: &CommandSpec,
            _cancel: &dyn Cancellation,
        ) -> maintainer_core::error::Result<CommandOutput> {
            self.calls.lock().unwrap().push(spec.args().to_vec());
            Ok(self.output.clone())
        }
    }

    fn candidate(name: &str, target: &str) -> UpdateCandidate {
        UpdateCandidate {
            name: name.to_string(),
            installed_version: Some("1.0".to_string()),
            available_version: target.to_string(),
            classification: UpdateClassification::Standard,
            download_bytes: None,
            hold_state: HoldState::None,
            backend: UpdateBackend::Apt,
            is_new: false,
            is_kernel: false,
        }
    }

    fn output(stdout: &str, exit_code: i32) -> CommandOutput {
        CommandOutput {
            exit_code: Some(exit_code),
            stdout: stdout.to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    #[test]
    fn exact_installed_version_is_verified() {
        let runner = FixedRunner {
            output: output("2.0", 0),
            calls: Mutex::new(Vec::new()),
        };
        let cancel = maintainer_core::jobs::CancellationToken::new();

        assert!(verify_installed_versions(&runner, &cancel, &[candidate("wget", "2.0")]).is_ok());
        assert_eq!(
            runner.calls.lock().unwrap().as_slice(),
            &[vec![
                "-W".to_string(),
                "-f=${Version}".to_string(),
                "--".to_string(),
                "wget".to_string(),
            ]]
        );
    }

    #[test]
    fn unchanged_version_rejects_false_success() {
        let runner = FixedRunner {
            output: output("1.0", 0),
            calls: Mutex::new(Vec::new()),
        };
        let cancel = maintainer_core::jobs::CancellationToken::new();

        let error = verify_installed_versions(&runner, &cancel, &[candidate("wget", "2.0")])
            .expect_err("unchanged package must reject success");
        assert!(error.contains("wget"));
        assert!(error.contains("expected 2.0"));
        assert!(error.contains("found 1.0"));
    }

    #[test]
    fn dpkg_query_failure_rejects_success() {
        let runner = FixedRunner {
            output: output("", 1),
            calls: Mutex::new(Vec::new()),
        };
        let cancel = maintainer_core::jobs::CancellationToken::new();

        assert!(verify_installed_versions(&runner, &cancel, &[candidate("wget", "2.0")]).is_err());
    }
}
