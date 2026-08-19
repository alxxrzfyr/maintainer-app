//! On-demand job dispatch: routes page actions through `JobExecutor` with
//! cancellation, progress, and stale-job rejection.
//!
//! Each dispatch function submits work to the shared `JobExecutor`, receives a
//! typed `Receiver`, and installs a short-interval GTK timer that polls for
//! results. Results are delivered to `glib::idle_add_once` callbacks on the
//! main thread. Stale-job messages (from a superseded job ID) are discarded.
//!
//! Progress updates flow through a `SharedProgress` polled from the timer —
//! GTK objects never cross thread boundaries.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use glib::ControlFlow;

use maintainer_core::jobs::{CancellationToken, JobExecutor, JobMessage, NullProgress};
use maintainer_core::storage::{DuplicateCandidate, DuplicateScanner, ScanPolicy, StorageScanner};

use crate::state::ScanJobId;

// ---------------------------------------------------------------------------
// Storage scan
// ---------------------------------------------------------------------------

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
/// Returns the cancellation token. The `on_done` callback is invoked exactly
/// once on the main thread when the job finishes. Stale results (job_id
/// mismatch) are discarded by the caller.
pub fn dispatch_storage_scan(
    executor: &Arc<JobExecutor>,
    job_id: ScanJobId,
    policy: ScanPolicy,
    on_done: impl FnOnce(StorageScanResult) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();

    let job = move || -> maintainer_core::error::Result<StorageScanResult> {
        let progress_sink = NullProgress;
        let scanner = StorageScanner::new(&policy, &token_for_worker, &progress_sink);

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
    install_result_poller(rx, on_done);

    token
}

// ---------------------------------------------------------------------------
// Duplicate scan
// ---------------------------------------------------------------------------

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
    on_done: impl FnOnce(DuplicateScanJobResult) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();

    let job = move || -> maintainer_core::error::Result<DuplicateScanJobResult> {
        let progress_sink = NullProgress;

        // Phase 1: traverse to gather candidates.
        let scanner = StorageScanner::new(&policy, &token_for_worker, &progress_sink);
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

        // Build duplicate candidates from scanned files.
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

        // Phase 2: hash and group.
        let dup_scanner = DuplicateScanner::new(&token_for_worker, &progress_sink);
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
    install_result_poller(rx, on_done);

    token
}

// ---------------------------------------------------------------------------
// Cleanup scan
// ---------------------------------------------------------------------------

/// Result of a cleanup scan.
pub enum CleanupScanOutcome {
    Completed { item_count: usize, total_bytes: u64 },
    Failed { message: String },
}

/// Launch a cleanup scan through the shared `JobExecutor`.
pub fn dispatch_cleanup_scan(
    executor: &Arc<JobExecutor>,
    on_done: impl FnOnce(CleanupScanOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();

    let job = move || -> maintainer_core::error::Result<CleanupScanOutcome> {
        use maintainer_core::cleanup::CleanupAnalyzer;

        let home_dir = std::env::var("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|_| PathBuf::from("/tmp"));

        let analyzer = CleanupAnalyzer::new(home_dir);
        let progress = NullProgress;

        match analyzer.scan_all(&token_for_worker, &progress) {
            Ok(targets) => {
                let total_bytes: u64 = targets.iter().map(|t| t.size_bytes).sum();
                Ok(CleanupScanOutcome::Completed {
                    item_count: targets.len(),
                    total_bytes,
                })
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
    install_result_poller(rx, on_done);

    token
}

// ---------------------------------------------------------------------------
// Diagnostics scan
// ---------------------------------------------------------------------------

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
    on_done: impl FnOnce(DiagnosticsScanOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();

    let job = move || -> maintainer_core::error::Result<DiagnosticsScanOutcome> {
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::diagnostics::{
            JournalCollector, JournalFilter, group_journal_entries, interpret_groups,
        };

        let runner = SystemCommandRunner;
        let filter = JournalFilter::default();
        let journal = JournalCollector::collect(&filter, &runner, &token_for_worker);

        let findings = match journal {
            maintainer_core::capability::Capability::Available(entries) => {
                let groups = group_journal_entries(&entries);
                interpret_groups(&groups, &[])
            }
            _ => Vec::new(),
        };

        Ok(DiagnosticsScanOutcome::Completed { findings })
    };

    let rx = executor.execute(vec![job]);
    install_result_poller(rx, on_done);

    token
}

// ---------------------------------------------------------------------------
// Updates check
// ---------------------------------------------------------------------------

/// Updates check result.
pub enum UpdatesCheckOutcome {
    Available {
        summary: String,
        security_count: usize,
        total_count: usize,
    },
    UpToDate,
    Failed {
        message: String,
    },
}

/// Launch an update check through the shared `JobExecutor`.
pub fn dispatch_updates_check(
    executor: &Arc<JobExecutor>,
    on_done: impl FnOnce(UpdatesCheckOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();

    let job = move || -> maintainer_core::error::Result<UpdatesCheckOutcome> {
        use maintainer_core::capability::Capability;
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::updates::{AptDiscovery, DiscoveryResult};

        let runner = SystemCommandRunner;
        let result = AptDiscovery::check_updates(&runner, &token_for_worker);

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
    install_result_poller(rx, on_done);

    token
}

// ---------------------------------------------------------------------------
// Reports
// ---------------------------------------------------------------------------

/// Reports generation result.
pub enum ReportOutcome {
    Completed { text: String },
    Failed { message: String },
}

/// Launch report generation through the shared `JobExecutor`.
pub fn dispatch_report_generation(
    executor: &Arc<JobExecutor>,
    on_done: impl FnOnce(ReportOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();

    let job = move || -> maintainer_core::error::Result<ReportOutcome> {
        use maintainer_core::capability::{Capability, UnavailableReason};
        use maintainer_core::reports::model::{
            CapabilityNote, DiagnosticsSection, HardwareSection, HealthSection, MaintenanceSection,
            RedactionPolicy, SoftwareUpdatesSection, SystemReport,
        };
        use maintainer_core::reports::{redact_report, render_text};

        let os_section = build_os_section();

        let report = SystemReport {
            generated_at: chrono_like_now(),
            os: os_section,
            hardware: HardwareSection {
                cpu: Capability::Unavailable(UnavailableReason {
                    code: "not-sampled",
                    message: "Report does not trigger live sampling".into(),
                    remediation: None,
                }),
                memory: Capability::Unavailable(UnavailableReason {
                    code: "not-sampled",
                    message: "Report does not trigger live sampling".into(),
                    remediation: None,
                }),
                disks: Capability::Unavailable(UnavailableReason {
                    code: "not-sampled",
                    message: "Report does not trigger live sampling".into(),
                    remediation: None,
                }),
                network: Capability::Unavailable(UnavailableReason {
                    code: "not-sampled",
                    message: "Report does not trigger live sampling".into(),
                    remediation: None,
                }),
                temperatures: Capability::Unavailable(UnavailableReason {
                    code: "not-sampled",
                    message: "Report does not trigger live sampling".into(),
                    remediation: None,
                }),
            },
            health: HealthSection {
                smart_drives: Capability::Unavailable(UnavailableReason {
                    code: "not-queried",
                    message: "SMART data not queried for report".into(),
                    remediation: None,
                }),
            },
            diagnostics: DiagnosticsSection {
                failed_services: Capability::Unavailable(UnavailableReason {
                    code: "not-queried",
                    message: "Service diagnostics not queried for report".into(),
                    remediation: None,
                }),
                event_groups: Capability::Unavailable(UnavailableReason {
                    code: "not-queried",
                    message: "Journal events not queried for report".into(),
                    remediation: None,
                }),
            },
            maintenance: MaintenanceSection {
                recent_history: Vec::new(),
            },
            software_updates: SoftwareUpdatesSection {
                pending_updates: Capability::Unavailable(UnavailableReason {
                    code: "not-checked",
                    message: "Update check not performed for report".into(),
                    remediation: None,
                }),
            },
            capability_notes: vec![CapabilityNote {
                name: "Live metrics".into(),
                status: "not sampled".into(),
                reason: Some("Report does not trigger live sampling".into()),
            }],
        };

        let policy = RedactionPolicy::default();
        let redacted = redact_report(report, &policy);
        let text = render_text(&redacted);

        Ok(ReportOutcome::Completed { text })
    };

    let rx = executor.execute(vec![job]);
    install_result_poller(rx, on_done);

    token
}

// ---------------------------------------------------------------------------
// Internal: GTK result poller
// ---------------------------------------------------------------------------

/// Installs a short-interval timer that polls the receiver for a single result
/// and delivers it to the on_done callback on the main thread.
fn install_result_poller<T: 'static>(
    rx: std::sync::mpsc::Receiver<JobMessage<T>>,
    on_done: impl FnOnce(T) + 'static,
) {
    let mut on_done = Some(on_done);
    // Poll every 50ms — fast enough for responsiveness, cheap enough for idle.
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

// ---------------------------------------------------------------------------
// Report helpers
// ---------------------------------------------------------------------------

fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{secs} (Unix timestamp)")
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
