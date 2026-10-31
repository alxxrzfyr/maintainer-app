//! Bounded background job transport and feature dispatch.

use std::time::Duration;

use glib::ControlFlow;

use maintainer_core::jobs::JobMessage;

use crate::progress::SharedProgress;

mod actions;
mod cleanup;
mod diagnostics;
mod reports;
mod storage;
mod updates;

pub use actions::{LocalActionError, dispatch_helper_call, dispatch_local_action};
pub use cleanup::{
    CleanupExecutionOutcome, CleanupRecoveryOutcome, CleanupScanOutcome,
    dispatch_cleanup_execution, dispatch_cleanup_recovery, dispatch_cleanup_scan,
};
pub(crate) use diagnostics::dispatch_hardware_collection;
pub use diagnostics::{
    DiagnosticsScanOutcome, SmartRefreshOutcome, dispatch_diagnostics_scan, dispatch_smart_refresh,
};
pub use reports::{ReportOutcome, dispatch_report_generation};
pub use storage::{
    DuplicateScanJobResult, StorageScanResult, dispatch_duplicate_scan, dispatch_storage_scan,
};
pub use updates::{
    UpdatesCheckOutcome, dispatch_replan_selection, dispatch_updates_check,
    update_selection_reviewed,
};

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

#[cfg(test)]
mod terminal_tests {
    use super::*;
    use super::{reports::report_bytes, storage::duplicate_bytes};
    use crate::state::ScanJobId;

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
