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

    /// Share cancellation with an adapter that already uses an atomic flag.
    pub fn from_shared_flag(flag: Arc<AtomicBool>) -> Self {
        Self { flag }
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

const MAX_QUEUED_JOBS: usize = 16;
const MAX_ACCEPTED_JOBS: usize = 24;
const MAX_RETAINED_RESULT_BYTES: usize = 16 * 1024 * 1024;
const TERMINAL_RESULT_BYTES: usize = 4096;

type Task = Box<dyn FnOnce() + Send>;
struct QueuedJob {
    task: Task,
    cancellation: CancellationToken,
}
struct SchedulerState {
    queue: std::collections::VecDeque<QueuedJob>,
    active: usize,
    accepted: usize,
    retained_bytes: usize,
    shutdown: bool,
    scans: usize,
    cancellations: Vec<std::sync::Weak<AtomicBool>>,
}
struct Scheduler {
    state: std::sync::Mutex<SchedulerState>,
    ready: std::sync::Condvar,
    max_workers: usize,
}

struct PermitState {
    stopped: bool,
    consumed: bool,
    released: bool,
    bytes: usize,
    scan: bool,
}
struct JobPermit {
    scheduler: Arc<Scheduler>,
    state: std::sync::Mutex<PermitState>,
}
impl JobPermit {
    fn update(&self, stopped: bool, consumed: bool) {
        let mut permit = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let scan_finished = stopped && !permit.stopped && permit.scan;
        permit.stopped |= stopped;
        permit.consumed |= consumed;
        let release = permit.stopped && permit.consumed && !permit.released;
        if scan_finished || release {
            let mut scheduler = self
                .scheduler
                .state
                .lock()
                .unwrap_or_else(|poison| poison.into_inner());
            if scan_finished {
                scheduler.scans -= 1;
            }
            if release {
                permit.released = true;
                scheduler.accepted -= 1;
                scheduler.retained_bytes -= permit.bytes;
            }
        }
    }
    fn reserve_bytes(&self, bytes: usize) -> bool {
        let mut permit = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut scheduler = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let bytes = bytes.max(TERMINAL_RESULT_BYTES);
        let available =
            MAX_RETAINED_RESULT_BYTES.saturating_sub(scheduler.retained_bytes - permit.bytes);
        if bytes > available {
            return false;
        }
        scheduler.retained_bytes = scheduler.retained_bytes - permit.bytes + bytes;
        permit.bytes = bytes;
        true
    }
}

/// A result subscription retains its admission permits until work stops and
/// results are consumed or dropped. Dropping it requests cooperative cancellation.
pub struct JobReceiver<T> {
    receiver: Receiver<(JobMessage<T>, Option<Arc<JobPermit>>)>,
    permits: Vec<Arc<JobPermit>>,
    cancellation: CancellationToken,
}
impl<T> JobReceiver<T> {
    pub fn try_recv(&self) -> std::result::Result<JobMessage<T>, mpsc::TryRecvError> {
        self.receiver.try_recv().map(|(message, permit)| {
            if let Some(permit) = permit {
                permit.update(false, true);
            }
            message
        })
    }
    pub fn recv(&self) -> std::result::Result<JobMessage<T>, mpsc::RecvError> {
        self.receiver.recv().map(|(message, permit)| {
            if let Some(permit) = permit {
                permit.update(false, true);
            }
            message
        })
    }
}
impl<T> Drop for JobReceiver<T> {
    fn drop(&mut self) {
        self.cancellation.cancel();
        for permit in &self.permits {
            permit.update(false, true);
        }
    }
}

/// One shared, bounded worker pool for all submissions to this executor.
pub struct JobExecutor {
    scheduler: Arc<Scheduler>,
}
impl JobExecutor {
    pub fn new(max_workers: usize) -> crate::error::Result<Self> {
        if max_workers == 0 || max_workers > MAX_ACCEPTED_JOBS {
            return Err(MaintainerError::invalid_data(
                "job scheduler",
                "worker count must be between 1 and 24",
            ));
        }
        let scheduler = Self::scheduler(max_workers, false);
        for index in 0..max_workers {
            let worker = scheduler.clone();
            if let Err(error) = thread::Builder::new()
                .name(format!("job-worker-{index}"))
                .spawn(move || {
                    loop {
                        let task = {
                            let mut state = worker
                                .state
                                .lock()
                                .unwrap_or_else(|poison| poison.into_inner());
                            while state.queue.is_empty() && !state.shutdown {
                                state = worker
                                    .ready
                                    .wait(state)
                                    .unwrap_or_else(|poison| poison.into_inner());
                            }
                            let Some(job) = state.queue.pop_front() else {
                                return;
                            };
                            state.active += 1;
                            job.task
                        };
                        task();
                        let mut state = worker
                            .state
                            .lock()
                            .unwrap_or_else(|poison| poison.into_inner());
                        state.active -= 1;
                    }
                })
            {
                scheduler
                    .state
                    .lock()
                    .unwrap_or_else(|poison| poison.into_inner())
                    .shutdown = true;
                scheduler.ready.notify_all();
                return Err(MaintainerError::io("starting job worker", error));
            }
        }
        Ok(Self { scheduler })
    }

    fn scheduler(max_workers: usize, shutdown: bool) -> Arc<Scheduler> {
        Arc::new(Scheduler {
            state: std::sync::Mutex::new(SchedulerState {
                queue: std::collections::VecDeque::new(),
                active: 0,
                accepted: 0,
                retained_bytes: 0,
                shutdown,
                scans: 0,
                cancellations: Vec::new(),
            }),
            ready: std::sync::Condvar::new(),
            max_workers,
        })
    }

    /// An explicit unavailable capability when OS worker creation fails.
    pub fn unavailable() -> Self {
        Self {
            scheduler: Self::scheduler(0, true),
        }
    }

    pub fn execute<T, F>(&self, jobs: Vec<F>) -> JobReceiver<T>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
    {
        self.execute_cancellable(jobs, CancellationToken::new())
    }

    /// Submit atomically; saturation produces a bounded terminal busy error.
    /// Heap-owning outputs should use execute_sized to account for their payload.
    pub fn execute_cancellable<T, F>(
        &self,
        jobs: Vec<F>,
        cancellation: CancellationToken,
    ) -> JobReceiver<T>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
    {
        self.execute_sized(jobs, cancellation, |_| std::mem::size_of::<T>())
    }

    pub fn execute_sized<T, F, S>(
        &self,
        jobs: Vec<F>,
        cancellation: CancellationToken,
        result_bytes: S,
    ) -> JobReceiver<T>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
        S: Fn(&T) -> usize + Send + Sync + 'static,
    {
        self.submit_sized(jobs, cancellation, result_bytes, false)
    }

    /// Reserve the filesystem lane at admission; workers never wait for it.
    pub fn execute_scan_sized<T, F, S>(
        &self,
        jobs: Vec<F>,
        cancellation: CancellationToken,
        result_bytes: S,
    ) -> JobReceiver<T>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
        S: Fn(&T) -> usize + Send + Sync + 'static,
    {
        self.submit_sized(jobs, cancellation, result_bytes, true)
    }

    fn submit_sized<T, F, S>(
        &self,
        jobs: Vec<F>,
        cancellation: CancellationToken,
        result_bytes: S,
        scan: bool,
    ) -> JobReceiver<T>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
        S: Fn(&T) -> usize + Send + Sync + 'static,
    {
        let count = jobs.len();
        let (sender, receiver) = mpsc::sync_channel(count.clamp(1, MAX_ACCEPTED_JOBS));
        let mut subscription = JobReceiver {
            receiver,
            permits: Vec::new(),
            cancellation: cancellation.clone(),
        };
        let mut state = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.shutdown
            || count.saturating_mul(TERMINAL_RESULT_BYTES)
                > MAX_RETAINED_RESULT_BYTES.saturating_sub(state.retained_bytes)
            || (scan && (count != 1 || state.scans != 0))
            || count > MAX_ACCEPTED_JOBS.saturating_sub(state.accepted)
            || count
                > (self.scheduler.max_workers + MAX_QUEUED_JOBS)
                    .saturating_sub(state.active + state.queue.len())
        {
            // Rejected batches execute nothing and report one admission failure.
            let _ = sender.send((
                JobMessage::Failed(MaintainerError::invalid_data(
                    "job scheduler",
                    if state.shutdown {
                        "background workers are unavailable or shutting down"
                    } else {
                        "busy; try again after another operation finishes"
                    },
                )),
                None,
            ));
            return subscription;
        }
        state.accepted += count;
        state.retained_bytes += count * TERMINAL_RESULT_BYTES;
        if scan {
            state.scans += count;
        }
        state
            .cancellations
            .retain(|token| token.strong_count() != 0);
        state.cancellations.push(Arc::downgrade(&cancellation.flag));
        let result_bytes = Arc::new(result_bytes);
        for job in jobs {
            let sender = sender.clone();
            let token = cancellation.clone();
            let permit = Arc::new(JobPermit {
                scheduler: self.scheduler.clone(),
                state: std::sync::Mutex::new(PermitState {
                    stopped: false,
                    consumed: false,
                    released: false,
                    bytes: TERMINAL_RESULT_BYTES,
                    scan,
                }),
            });
            subscription.permits.push(permit.clone());
            let size = result_bytes.clone();
            state.queue.push_back(QueuedJob {
                cancellation: token.clone(),
                task: Box::new(move || {
                    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        if token.is_cancelled() {
                            return Err(MaintainerError::Cancelled);
                        }
                        let value = job()?;
                        if !permit.reserve_bytes(size(&value)) {
                            return Err(MaintainerError::invalid_data(
                                "job result",
                                "result exceeds the retained data budget",
                            ));
                        }
                        Ok(value)
                    }));
                    let message = match outcome {
                        Ok(Ok(value)) => JobMessage::Completed(value),
                        Ok(Err(MaintainerError::Cancelled)) => {
                            JobMessage::Failed(MaintainerError::Cancelled)
                        }
                        Ok(Err(error)) => {
                            let rendered = error.to_string();
                            let mut end = rendered.len().min(3072);
                            while !rendered.is_char_boundary(end) {
                                end -= 1;
                            }
                            // Copy the bounded slice: truncate alone retains an oversized allocation.
                            let message = rendered[..end].to_owned();
                            JobMessage::Failed(MaintainerError::invalid_data(
                                "background operation",
                                message,
                            ))
                        }
                        Err(_) => JobMessage::Panicked("background operation panicked".into()),
                    };
                    let _ = sender.send((message, Some(permit.clone())));
                    permit.update(true, false);
                }),
            });
        }
        drop(state);
        self.scheduler.ready.notify_all();
        subscription
    }

    pub fn execute_all<T, F>(&self, jobs: Vec<F>) -> Vec<JobMessage<T>>
    where
        T: Send + 'static,
        F: FnOnce() -> crate::error::Result<T> + Send + 'static,
    {
        let receiver = self.execute(jobs);
        let mut results = Vec::new();
        while let Ok(message) = receiver.recv() {
            results.push(message);
        }
        results
    }

    pub fn shutdown(&self) {
        let mut state = self
            .scheduler
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.shutdown = true;
        for token in &state.cancellations {
            if let Some(flag) = token.upgrade() {
                flag.store(true, Ordering::Release);
            }
        }
        for job in &state.queue {
            job.cancellation.cancel();
        }
        self.scheduler.ready.notify_all();
    }
}
impl Drop for JobExecutor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicU32;
    use std::time::Duration;

    fn wait_idle(executor: &JobExecutor) {
        let deadline = std::time::Instant::now() + Duration::from_secs(3);
        loop {
            let state = executor.scheduler.state.lock().unwrap();
            if state.active == 0 && state.queue.is_empty() {
                return;
            }
            drop(state);
            assert!(
                std::time::Instant::now() < deadline,
                "workers failed to stop"
            );
            thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn independent_submissions_share_four_workers_and_sixteen_queue_slots() {
        let executor = JobExecutor::new(4).unwrap();
        let gate = Arc::new((std::sync::Mutex::new(false), std::sync::Condvar::new()));
        let (started_tx, started_rx) = mpsc::channel();
        let mut running = Vec::new();
        for _ in 0..4 {
            let gate = gate.clone();
            let started = started_tx.clone();
            running.push(executor.execute(vec![move || {
                started.send(()).unwrap();
                let (mutex, changed) = &*gate;
                let mut released = mutex.lock().unwrap();
                while !*released {
                    released = changed.wait(released).unwrap();
                }
                Ok(())
            }]));
        }
        for _ in 0..4 {
            started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        }
        let queued: Vec<_> = (0..16).map(|_| executor.execute(vec![|| Ok(())])).collect();
        assert!(matches!(
            executor.execute(vec![|| Ok(())]).try_recv(),
            Ok(JobMessage::Failed(_))
        ));
        drop(running);
        // Dropping running subscriptions must not free worker capacity.
        assert!(matches!(
            executor.execute(vec![|| Ok(())]).try_recv(),
            Ok(JobMessage::Failed(_))
        ));
        *gate.0.lock().unwrap() = true;
        gate.1.notify_all();
        for receiver in queued {
            assert!(matches!(
                receiver.recv().unwrap(),
                JobMessage::Completed(())
            ));
        }
        wait_idle(&executor);
    }

    #[test]
    fn completed_results_retain_admission_until_consumed_or_dropped() {
        let executor = JobExecutor::new(4).unwrap();
        let mut results = Vec::new();
        for _ in 0..24 {
            results.push(executor.execute(vec![|| Ok(())]));
            wait_idle(&executor);
        }
        assert!(matches!(
            executor.execute(vec![|| Ok(())]).try_recv(),
            Ok(JobMessage::Failed(_))
        ));
        drop(results.pop());
        let accepted = executor.execute(vec![|| Ok(42)]);
        assert!(matches!(
            accepted.recv().unwrap(),
            JobMessage::Completed(42)
        ));
    }

    #[test]
    fn oversized_batches_are_rejected_atomically() {
        let executor = JobExecutor::new(4).unwrap();
        let executed = Arc::new(AtomicU32::new(0));
        let jobs: Vec<_> = (0..21)
            .map(|_| {
                let executed = executed.clone();
                move || {
                    executed.fetch_add(1, Ordering::SeqCst);
                    Ok(())
                }
            })
            .collect();
        assert!(matches!(
            executor.execute(jobs).recv().unwrap(),
            JobMessage::Failed(_)
        ));
        assert_eq!(executed.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn queued_cancellation_skips_body_and_recovers_after_panic() {
        let executor = JobExecutor::new(1).unwrap();
        let (release_tx, release_rx) = mpsc::channel();
        let (start_tx, start_rx) = mpsc::channel();
        let running = executor.execute(vec![move || {
            start_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        }]);
        start_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let token = CancellationToken::new();
        let queued = executor.execute_cancellable(
            vec![|| -> crate::error::Result<()> { panic!("cancelled job ran") }],
            token.clone(),
        );
        token.cancel();
        release_tx.send(()).unwrap();
        assert!(matches!(running.recv().unwrap(), JobMessage::Completed(())));
        assert!(matches!(
            queued.recv().unwrap(),
            JobMessage::Failed(MaintainerError::Cancelled)
        ));
        let panicked = executor.execute(vec![|| -> crate::error::Result<()> {
            panic!("private panic")
        }]);
        assert!(matches!(panicked.recv().unwrap(), JobMessage::Panicked(_)));
        assert!(matches!(
            executor.execute(vec![|| Ok(7)]).recv().unwrap(),
            JobMessage::Completed(7)
        ));
    }

    #[test]
    fn retained_payload_budget_is_aggregate_and_released_on_consumption() {
        let executor = JobExecutor::new(2).unwrap();
        let first = executor.execute_sized(
            vec![|| Ok(vec![0u8; 9 * 1024 * 1024])],
            CancellationToken::new(),
            Vec::capacity,
        );
        wait_idle(&executor);
        let second = executor.execute_sized(
            vec![|| Ok(vec![0u8; 9 * 1024 * 1024])],
            CancellationToken::new(),
            Vec::capacity,
        );
        assert!(matches!(second.recv().unwrap(), JobMessage::Failed(_)));
        drop(first);
        let third = executor.execute_sized(
            vec![|| Ok(vec![0u8; 9 * 1024 * 1024])],
            CancellationToken::new(),
            Vec::capacity,
        );
        assert!(matches!(third.recv().unwrap(), JobMessage::Completed(_)));
    }

    #[test]
    fn shutdown_requests_running_cancellation_and_rejects_submissions() {
        let executor = JobExecutor::new(1).unwrap();
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let (started_tx, started_rx) = mpsc::channel();
        let running = executor.execute_cancellable(
            vec![move || {
                started_tx.send(()).unwrap();
                while !worker_token.is_cancelled() {
                    thread::sleep(Duration::from_millis(2));
                }
                Err::<(), _>(MaintainerError::Cancelled)
            }],
            token,
        );
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        executor.shutdown();
        assert!(matches!(
            running.recv().unwrap(),
            JobMessage::Failed(MaintainerError::Cancelled)
        ));
        assert!(matches!(
            executor.execute(vec![|| Ok(())]).recv().unwrap(),
            JobMessage::Failed(_)
        ));
    }

    #[test]
    fn filesystem_lane_is_reserved_without_blocking_a_worker() {
        let executor = JobExecutor::new(2).unwrap();
        let (release_tx, release_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let scan = executor.execute_scan_sized(
            vec![move || {
                started_tx.send(()).unwrap();
                release_rx.recv().unwrap();
                Ok(())
            }],
            CancellationToken::new(),
            |_| 0,
        );
        started_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let rejected =
            executor.execute_scan_sized(vec![|| Ok(())], CancellationToken::new(), |_| 0);
        assert!(matches!(rejected.recv().unwrap(), JobMessage::Failed(_)));
        assert!(matches!(
            executor.execute(vec![|| Ok(())]).recv().unwrap(),
            JobMessage::Completed(())
        ));
        release_tx.send(()).unwrap();
        assert!(matches!(scan.recv().unwrap(), JobMessage::Completed(())));
    }

    #[test]
    fn unavailable_executor_refuses_work_without_panicking() {
        let executor = JobExecutor::unavailable();
        assert!(matches!(
            executor.execute(vec![|| Ok(())]).recv().unwrap(),
            JobMessage::Failed(_)
        ));
    }

    #[test]
    fn terminal_failures_fit_the_reserved_budget() {
        let executor = JobExecutor::new(1).unwrap();
        let receiver = executor.execute(vec![|| {
            Err::<(), _>(MaintainerError::invalid_data(
                "fixture",
                "x".repeat(100_000),
            ))
        }]);
        match receiver.recv().unwrap() {
            JobMessage::Failed(MaintainerError::InvalidData { message, .. }) => {
                assert!(message.len() <= 3072);
                assert!(message.capacity() < TERMINAL_RESULT_BYTES);
            }
            _ => panic!("expected bounded failure"),
        }
        wait_idle(&executor);
        assert_eq!(executor.scheduler.state.lock().unwrap().retained_bytes, 0);
    }

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
            assert_eq!(msg, "background operation panicked");
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
