use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::thread;

use crate::error::MaintainerError;

/// Cooperative cancellation signal backed by an atomic boolean.
pub trait Cancellation: Send + Sync {
    /// Returns `true` if cancellation has been requested.
    fn is_cancelled(&self) -> bool;
}

/// Concrete cancellation token using `Arc<AtomicBool>`.
#[derive(Debug, Clone)]
pub struct CancellationToken {
    flag: Arc<AtomicBool>,
}

impl CancellationToken {
    /// Create a new token in the non-cancelled state.
    pub fn new() -> Self {
        Self {
            flag: Arc::new(AtomicBool::new(false)),
        }
    }

    /// Signal cancellation.
    pub fn cancel(&self) {
        self.flag.store(true, Ordering::Release);
    }
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl Cancellation for CancellationToken {
    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }
}

/// Current progress state for a running job.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobProgress {
    /// Items processed so far.
    pub completed: u64,
    /// Total items if known; `None` for indeterminate progress.
    pub total: Option<u64>,
    /// Current phase or description.
    pub message: String,
}

/// Sink for progress updates emitted by a job.
pub trait ProgressSink: Send + Sync {
    /// Report progress. Implementations must be cheap and non-blocking.
    fn report(&self, progress: &JobProgress);
}

/// No-op progress sink for jobs that do not report progress.
#[derive(Debug, Clone, Copy)]
pub struct NullProgress;

impl ProgressSink for NullProgress {
    fn report(&self, _progress: &JobProgress) {}
}

/// Outcome for a single item within a batch job.
#[derive(Debug, Clone)]
pub enum ItemResult<T> {
    /// The item was processed successfully.
    Ok(T),
    /// The item failed with a non-fatal error.
    Failed { item: String, message: String },
    /// The item was skipped (e.g., unsupported type).
    Skipped { item: String, reason: String },
}

/// Summary outcome for a complete operation.
#[derive(Debug, Clone)]
pub struct OperationOutcome {
    /// Number of items that succeeded.
    pub succeeded: u64,
    /// Number of items that failed.
    pub failed: u64,
    /// Number of items that were skipped.
    pub skipped: u64,
    /// Whether the operation was cancelled before completing all items.
    pub cancelled: bool,
}

impl OperationOutcome {
    /// Returns `true` if all items succeeded and nothing was cancelled.
    pub fn is_complete_success(&self) -> bool {
        self.failed == 0 && self.skipped == 0 && !self.cancelled
    }
}

/// Message sent from a worker to the executor owner.
#[derive(Debug)]
pub enum JobMessage<T> {
    /// A job completed successfully.
    Completed(T),
    /// A job failed with an error.
    Failed(MaintainerError),
    /// A worker panicked.
    Panicked(String),
}

/// Bounded thread-pool executor for CPU or blocking-I/O jobs.
///
/// Limits concurrency to `max_workers`, catches panics at worker boundaries,
/// and communicates results through a bounded channel.
pub struct JobExecutor {
    max_workers: usize,
}

impl JobExecutor {
    /// Create an executor that runs at most `max_workers` concurrent jobs.
    ///
    /// # Errors
    ///
    /// Returns an error if `max_workers` is zero.
    pub fn new(max_workers: usize) -> crate::error::Result<Self> {
        if max_workers == 0 {
            return Err(MaintainerError::Internal {
                context: "JobExecutor::new".into(),
                message: "max_workers must be at least 1".into(),
            });
        }
        Ok(Self { max_workers })
    }

    /// Execute a batch of jobs, returning a receiver for typed results.
    ///
    /// Each job runs in its own thread (bounded by `max_workers`).
    /// The channel is bounded to `max_workers` to provide backpressure.
    /// Worker panics are caught and reported as `JobMessage::Panicked`.
    pub fn execute<T, F>(&self, jobs: Vec<F>) -> Receiver<JobMessage<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel::<JobMessage<T>>(self.max_workers);
        let max_workers = self.max_workers;

        thread::spawn(move || {
            let (token_tx, token_rx) = mpsc::sync_channel::<()>(max_workers);
            for _ in 0..max_workers {
                let _ = token_tx.send(());
            }

            for job in jobs {
                if token_rx.recv().is_err() {
                    break;
                }
                let tx = tx.clone();
                let token_tx = token_tx.clone();
                thread::spawn(move || {
                    let msg = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                        Ok(Ok(value)) => JobMessage::Completed(value),
                        Ok(Err(e)) => JobMessage::Failed(e),
                        Err(panic_info) => {
                            let message = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            JobMessage::Panicked(message)
                        }
                    };
                    let _ = tx.send(msg);
                    let _ = token_tx.send(());
                });
            }
        });

        rx
    }

    /// Execute a batch of jobs with bounded concurrency, blocking until all complete.
    ///
    /// Returns results in completion order. Limits active threads to `max_workers`.
    pub fn execute_all<T, F>(&self, jobs: Vec<F>) -> Vec<JobMessage<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
    {
        let job_count = jobs.len();
        let (tx, rx) = mpsc::sync_channel::<JobMessage<T>>(self.max_workers);

        let (token_tx, token_rx) = mpsc::sync_channel::<()>(self.max_workers);
        for _ in 0..self.max_workers {
            let _ = token_tx.send(());
        }

        let dispatch = thread::spawn(move || {
            for job in jobs {
                if token_rx.recv().is_err() {
                    break;
                }
                let tx = tx.clone();
                let token_tx = token_tx.clone();
                thread::spawn(move || {
                    let msg = match std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)) {
                        Ok(Ok(value)) => JobMessage::Completed(value),
                        Ok(Err(e)) => JobMessage::Failed(e),
                        Err(panic_info) => {
                            let message = if let Some(s) = panic_info.downcast_ref::<&str>() {
                                (*s).to_string()
                            } else if let Some(s) = panic_info.downcast_ref::<String>() {
                                s.clone()
                            } else {
                                "unknown panic".to_string()
                            };
                            JobMessage::Panicked(message)
                        }
                    };
                    let _ = tx.send(msg);
                    let _ = token_tx.send(());
                });
            }
        });

        let mut results = Vec::with_capacity(job_count);
        for _ in 0..job_count {
            match rx.recv() {
                Ok(msg) => results.push(msg),
                Err(_) => break,
            }
        }

        let _ = dispatch.join();
        results
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::time::Duration;

    #[test]
    fn cancellation_token_starts_not_cancelled() {
        let token = CancellationToken::new();
        assert!(!token.is_cancelled());
    }

    #[test]
    fn cancellation_token_becomes_cancelled_after_cancel() {
        let token = CancellationToken::new();
        token.cancel();
        assert!(token.is_cancelled());
    }

    #[test]
    fn cancellation_visible_across_clones() {
        let token = CancellationToken::new();
        let clone = token.clone();
        token.cancel();
        assert!(clone.is_cancelled());
    }

    #[test]
    fn cancelled_job_returns_cancelled_error() {
        let token = CancellationToken::new();
        token.cancel();

        let result: crate::error::Result<()> = if token.is_cancelled() {
            Err(MaintainerError::Cancelled)
        } else {
            Ok(())
        };

        assert!(matches!(result, Err(MaintainerError::Cancelled)));
    }

    #[test]
    fn cancellation_during_iteration() {
        let token = CancellationToken::new();
        let mut processed = 0u32;

        for i in 0..100 {
            if token.is_cancelled() {
                break;
            }
            processed += 1;
            if i == 5 {
                token.cancel();
            }
        }

        assert_eq!(processed, 6);
    }

    #[test]
    fn progress_with_known_total() {
        let progress = JobProgress {
            completed: 50,
            total: Some(100),
            message: "Scanning files".into(),
        };
        assert_eq!(progress.completed, 50);
        assert_eq!(progress.total, Some(100));
    }

    #[test]
    fn progress_with_unknown_total() {
        let progress = JobProgress {
            completed: 12,
            total: None,
            message: "Discovering items".into(),
        };
        assert!(progress.total.is_none());
    }

    #[test]
    fn null_progress_is_noop() {
        let sink = NullProgress;
        let progress = JobProgress {
            completed: 1,
            total: Some(10),
            message: "test".into(),
        };
        sink.report(&progress);
    }

    #[test]
    fn operation_outcome_complete_success() {
        let outcome = OperationOutcome {
            succeeded: 10,
            failed: 0,
            skipped: 0,
            cancelled: false,
        };
        assert!(outcome.is_complete_success());
    }

    #[test]
    fn operation_outcome_partial_failure() {
        let outcome = OperationOutcome {
            succeeded: 8,
            failed: 2,
            skipped: 0,
            cancelled: false,
        };
        assert!(!outcome.is_complete_success());
    }

    #[test]
    fn operation_outcome_cancelled() {
        let outcome = OperationOutcome {
            succeeded: 5,
            failed: 0,
            skipped: 0,
            cancelled: true,
        };
        assert!(!outcome.is_complete_success());
    }

    #[test]
    fn job_executor_rejects_zero_workers() {
        let result = JobExecutor::new(0);
        assert!(result.is_err());
    }

    #[test]
    fn job_executor_runs_jobs_to_completion() {
        let executor = JobExecutor::new(2).unwrap();
        let jobs: Vec<Box<dyn FnOnce() -> crate::error::Result<u32> + Send>> =
            vec![Box::new(|| Ok(1)), Box::new(|| Ok(2)), Box::new(|| Ok(3))];
        let results = executor.execute_all(jobs);
        assert_eq!(results.len(), 3);

        let mut values: Vec<u32> = results
            .into_iter()
            .filter_map(|msg| match msg {
                JobMessage::Completed(v) => Some(v),
                _ => None,
            })
            .collect();
        values.sort();
        assert_eq!(values, vec![1, 2, 3]);
    }

    #[test]
    fn job_executor_catches_panics() {
        let executor = JobExecutor::new(2).unwrap();
        let jobs: Vec<Box<dyn FnOnce() -> crate::error::Result<u32> + Send>> = vec![
            Box::new(|| Ok(1)),
            Box::new(|| panic!("intentional test panic")),
            Box::new(|| Ok(3)),
        ];
        let results = executor.execute_all(jobs);
        assert_eq!(results.len(), 3);

        let panics: Vec<_> = results
            .iter()
            .filter(|msg| matches!(msg, JobMessage::Panicked(_)))
            .collect();
        assert_eq!(panics.len(), 1);

        if let JobMessage::Panicked(msg) = &panics[0] {
            assert!(msg.contains("intentional test panic"));
        }
    }

    #[test]
    fn job_executor_respects_max_workers_bound() {
        let executor = JobExecutor::new(2).unwrap();
        let active_count = Arc::new(AtomicU32::new(0));
        let max_observed = Arc::new(AtomicU32::new(0));

        let jobs: Vec<Box<dyn FnOnce() -> crate::error::Result<()> + Send>> = (0..6)
            .map(|_| {
                let active = Arc::clone(&active_count);
                let max_obs = Arc::clone(&max_observed);
                let f: Box<dyn FnOnce() -> crate::error::Result<()> + Send> = Box::new(move || {
                    let current = active.fetch_add(1, Ordering::SeqCst) + 1;
                    max_obs.fetch_max(current, Ordering::SeqCst);
                    thread::sleep(Duration::from_millis(20));
                    active.fetch_sub(1, Ordering::SeqCst);
                    Ok(())
                });
                f
            })
            .collect();

        let results = executor.execute_all(jobs);
        assert_eq!(results.len(), 6);

        let max = max_observed.load(Ordering::SeqCst);
        assert!(max <= 2, "max concurrent workers was {max}, expected <= 2");
    }

    #[test]
    fn job_executor_reports_errors_not_panics_for_failures() {
        let executor = JobExecutor::new(2).unwrap();
        let jobs: Vec<Box<dyn FnOnce() -> crate::error::Result<u32> + Send>> =
            vec![Box::new(|| {
                Err(MaintainerError::io(
                    "test",
                    std::io::Error::new(std::io::ErrorKind::Other, "simulated"),
                ))
            })];
        let results = executor.execute_all(jobs);
        assert_eq!(results.len(), 1);
        assert!(matches!(&results[0], JobMessage::Failed(_)));
    }
}
