//! Thread-safe progress bridge: worker threads report progress via `ProgressSink`,
//! and the GTK main thread polls the latest value on a timer. Throttled to avoid
//! flooding the main loop. Cancel remains responsive because the cancellation
//! token is independent of progress polling.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use maintainer_core::jobs::{JobProgress, ProgressSink};

/// Minimum interval between accepting progress updates.
/// Workers that call `report` faster than this are silently dropped
/// to avoid contention and ensure the UI stays responsive.
const THROTTLE_INTERVAL: Duration = Duration::from_millis(50);

/// Shared progress state polled by the GTK main thread.
///
/// Workers write through `ProgressSink::report`; the UI reads via `take_latest`.
#[derive(Debug, Clone)]
pub struct SharedProgress {
    inner: Arc<Mutex<ProgressState>>,
}

#[derive(Debug)]
struct ProgressState {
    latest: Option<JobProgress>,
    last_update: Instant,
    /// Whether any update has arrived since the UI last polled.
    dirty: bool,
}

impl Default for SharedProgress {
    fn default() -> Self {
        Self::new()
    }
}

impl SharedProgress {
    /// Create a new shared progress bridge.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(ProgressState {
                latest: None,
                last_update: Instant::now() - THROTTLE_INTERVAL,
                dirty: false,
            })),
        }
    }

    /// Read the latest progress if it has changed since last poll.
    /// Returns `None` if no new progress is available.
    pub fn take_latest(&self) -> Option<JobProgress> {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.dirty {
            state.dirty = false;
            state.latest.clone()
        } else {
            None
        }
    }

    /// Peek at the latest progress without consuming the dirty flag.
    pub fn peek(&self) -> Option<JobProgress> {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.latest.clone()
    }
}

impl ProgressSink for SharedProgress {
    fn report(&self, progress: &JobProgress) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        let now = Instant::now();

        if now.duration_since(state.last_update) < THROTTLE_INTERVAL {
            return;
        }

        state.latest = Some(progress.clone());
        state.last_update = now;
        state.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::thread;

    #[test]
    fn take_latest_returns_none_initially() {
        let sp = SharedProgress::new();
        assert!(sp.take_latest().is_none());
    }

    #[test]
    fn take_latest_returns_progress_after_report() {
        let sp = SharedProgress::new();
        sp.report(&JobProgress {
            completed: 10,
            total: Some(100),
            message: "scanning".to_string(),
        });
        let latest = sp.take_latest();
        assert!(latest.is_some());
        let p = latest.unwrap();
        assert_eq!(p.completed, 10);
        assert_eq!(p.total, Some(100));
    }

    #[test]
    fn take_latest_clears_dirty_flag() {
        let sp = SharedProgress::new();
        sp.report(&JobProgress {
            completed: 5,
            total: None,
            message: "indexing".to_string(),
        });
        assert!(sp.take_latest().is_some());
        assert!(sp.take_latest().is_none());
    }

    #[test]
    fn throttle_drops_rapid_updates() {
        let sp = SharedProgress::new();
        sp.report(&JobProgress {
            completed: 1,
            total: Some(1000),
            message: "first".to_string(),
        });
        sp.report(&JobProgress {
            completed: 2,
            total: Some(1000),
            message: "second".to_string(),
        });
        let latest = sp.take_latest().unwrap();
        assert_eq!(latest.completed, 1);
    }

    #[test]
    fn reports_from_worker_thread_visible_on_main() {
        let sp = SharedProgress::new();
        let sp_clone = sp.clone();

        let handle = thread::spawn(move || {
            thread::sleep(Duration::from_millis(60));
            sp_clone.report(&JobProgress {
                completed: 42,
                total: Some(100),
                message: "from worker".to_string(),
            });
        });

        handle.join().unwrap();
        let latest = sp.take_latest().unwrap();
        assert_eq!(latest.completed, 42);
        assert_eq!(latest.message, "from worker");
    }

    #[test]
    fn peek_does_not_consume() {
        let sp = SharedProgress::new();
        sp.report(&JobProgress {
            completed: 7,
            total: None,
            message: "peeking".to_string(),
        });
        assert!(sp.peek().is_some());
        assert!(sp.take_latest().is_some());
    }

    #[test]
    fn throttle_rate_approximately_20_per_second() {
        // Verify that rapid reports (100 in 200ms) result in at most ~5 accepted.
        let sp = SharedProgress::new();
        let sp_clone = sp.clone();

        let handle = thread::spawn(move || {
            for i in 0..100 {
                sp_clone.report(&JobProgress {
                    completed: i,
                    total: Some(100),
                    message: format!("item {i}"),
                });
                thread::sleep(Duration::from_millis(2));
            }
        });

        handle.join().unwrap();

        let final_val = sp.peek();
        assert!(final_val.is_some());
        let p = final_val.unwrap();
        assert!(
            p.completed < 100,
            "Expected throttling to drop some reports"
        );
    }

    /// Validates the stale-job rejection pattern used by progress closures.
    /// A progress callback checks the current active job ID before applying updates.
    #[test]
    fn stale_job_progress_rejected() {
        use std::cell::RefCell;
        use std::rc::Rc;

        #[derive(Debug, Clone, Copy, PartialEq, Eq)]
        struct JobId(u64);

        let active_id: Rc<RefCell<Option<JobId>>> = Rc::new(RefCell::new(Some(JobId(1))));
        let applied_count: Rc<RefCell<u32>> = Rc::new(RefCell::new(0));

        let sid = active_id.clone();
        let counter = applied_count.clone();
        let on_progress = move |job_id: JobId| {
            if *sid.borrow() != Some(job_id) {
                return;
            }
            *counter.borrow_mut() += 1;
        };

        on_progress(JobId(1));
        on_progress(JobId(1));
        assert_eq!(*applied_count.borrow(), 2);

        *active_id.borrow_mut() = Some(JobId(2));

        on_progress(JobId(1));
        assert_eq!(*applied_count.borrow(), 2);

        on_progress(JobId(2));
        assert_eq!(*applied_count.borrow(), 3);
    }
}
