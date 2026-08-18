//! Safe process termination with PID-reuse protection.
//!
//! Before sending a signal, re-reads `/proc/<pid>/stat` and compares the
//! `start_time_ticks` from `ProcessIdentity`. This prevents killing a
//! different process that has reused the same PID.

use std::fs;

use crate::capability::PermissionRequest;
use crate::error::MaintainerError;
use crate::metrics::processes::ProcessIdentity;

/// Outcome of a termination attempt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TerminateResult {
    /// Signal was sent successfully.
    Sent,
    /// Process already exited before signal could be sent.
    AlreadyExited,
    /// PID was reused by a different process (start_time mismatch).
    PidReused,
    /// Permission denied — requires privilege escalation.
    PermissionRequired(PermissionRequest),
    /// Operation was cancelled before signal was sent.
    Cancelled,
}

/// Trait for abstracting filesystem reads during termination.
///
/// Allows tests to inject fake process tables without touching real `/proc`.
pub trait ProcReader: Send + Sync {
    /// Read the content of `/proc/<pid>/stat`. Returns `None` if the process
    /// does not exist (already exited).
    fn read_stat(&self, pid: u32) -> Option<String>;

    /// Send SIGTERM to the specified PID. Returns an IO error on failure.
    fn send_sigterm(&self, pid: u32) -> std::result::Result<(), std::io::Error>;

    /// Get the current user's UID.
    fn current_uid(&self) -> u32;
}

/// Real implementation that reads from `/proc`.
pub struct RealProcReader {
    proc_path: std::path::PathBuf,
}

impl RealProcReader {
    /// Create a reader that uses the real `/proc` filesystem.
    pub fn new() -> Self {
        Self {
            proc_path: std::path::PathBuf::from("/proc"),
        }
    }

    /// Create a reader with a custom proc path (for integration tests).
    pub fn with_path(proc_path: impl Into<std::path::PathBuf>) -> Self {
        Self {
            proc_path: proc_path.into(),
        }
    }
}

impl Default for RealProcReader {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcReader for RealProcReader {
    fn read_stat(&self, pid: u32) -> Option<String> {
        let stat_path = self.proc_path.join(pid.to_string()).join("stat");
        fs::read_to_string(stat_path).ok()
    }

    fn send_sigterm(&self, pid: u32) -> std::result::Result<(), std::io::Error> {
        // Safety: sending SIGTERM to a valid PID is a standard POSIX operation.
        // The PID has been verified via start_time before this call.
        let result = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if result == 0 {
            Ok(())
        } else {
            Err(std::io::Error::last_os_error())
        }
    }

    fn current_uid(&self) -> u32 {
        unsafe { libc::getuid() }
    }
}

/// Extract `start_time_ticks` from a `/proc/<pid>/stat` content string.
///
/// The stat format is: `pid (comm) state ppid ... start_time ...`
/// start_time is field 22 (1-indexed), which is index 19 after the comm field.
fn parse_start_time(stat_content: &str) -> Result<u64, MaintainerError> {
    let content = stat_content.trim();
    let close_paren = content.rfind(')').ok_or_else(|| {
        MaintainerError::invalid_data("/proc/[pid]/stat", "missing closing parenthesis")
    })?;

    let rest = content
        .get(close_paren + 2..)
        .ok_or_else(|| MaintainerError::invalid_data("/proc/[pid]/stat", "no fields after comm"))?;

    let fields: Vec<&str> = rest.split_whitespace().collect();
    if fields.len() < 20 {
        return Err(MaintainerError::invalid_data(
            "/proc/[pid]/stat",
            format!("too few fields: {} (need 20)", fields.len()),
        ));
    }

    // Field index 19 (0-based after comm) = start_time
    fields[19].parse::<u64>().map_err(|_| {
        MaintainerError::invalid_data("/proc/[pid]/stat", "start_time not a valid u64")
    })
}

/// Attempt to terminate a process identified by `ProcessIdentity`.
///
/// Re-reads `/proc/<pid>/stat` and verifies the start time matches before
/// sending SIGTERM. This prevents killing a different process that reused the PID.
///
/// If `cancelled` is true, returns `Cancelled` immediately without sending a signal.
pub fn terminate_process(
    identity: &ProcessIdentity,
    reader: &dyn ProcReader,
    cancelled: bool,
) -> Result<TerminateResult, MaintainerError> {
    if cancelled {
        return Ok(TerminateResult::Cancelled);
    }

    // Re-read the process stat to verify identity
    let stat_content = match reader.read_stat(identity.pid) {
        Some(content) => content,
        None => return Ok(TerminateResult::AlreadyExited),
    };

    // Verify start_time matches
    let current_start_time = parse_start_time(&stat_content)?;
    if current_start_time != identity.start_time_ticks {
        return Ok(TerminateResult::PidReused);
    }

    // Check if current user owns the process (heuristic: compare UID)
    // For now, attempt the signal and handle EPERM
    match reader.send_sigterm(identity.pid) {
        Ok(()) => Ok(TerminateResult::Sent),
        Err(e) if e.raw_os_error() == Some(libc::EPERM) => {
            Ok(TerminateResult::PermissionRequired(PermissionRequest {
                action_id: "io.github.maintainerapp.terminate-process".into(),
                explanation: format!("Terminate process {} (PID {})", "(foreign)", identity.pid),
            }))
        }
        Err(e)
            if e.kind() == std::io::ErrorKind::NotFound
                || e.raw_os_error() == Some(libc::ESRCH) =>
        {
            Ok(TerminateResult::AlreadyExited)
        }
        Err(e) => Err(MaintainerError::io(
            format!("sending SIGTERM to PID {}", identity.pid),
            e,
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Fake proc reader for testing.
    struct FakeProcReader {
        stat_content: Mutex<Option<String>>,
        sigterm_result: Mutex<std::result::Result<(), std::io::Error>>,
        uid: u32,
        sigterm_called: Mutex<bool>,
    }

    impl FakeProcReader {
        fn with_stat(stat: &str) -> Self {
            Self {
                stat_content: Mutex::new(Some(stat.to_string())),
                sigterm_result: Mutex::new(Ok(())),
                uid: 1000,
                sigterm_called: Mutex::new(false),
            }
        }

        fn exited() -> Self {
            Self {
                stat_content: Mutex::new(None),
                sigterm_result: Mutex::new(Ok(())),
                uid: 1000,
                sigterm_called: Mutex::new(false),
            }
        }

        fn with_permission_denied(stat: &str) -> Self {
            Self {
                stat_content: Mutex::new(Some(stat.to_string())),
                sigterm_result: Mutex::new(Err(std::io::Error::from_raw_os_error(libc::EPERM))),
                uid: 1000,
                sigterm_called: Mutex::new(false),
            }
        }

        fn was_sigterm_called(&self) -> bool {
            *self.sigterm_called.lock().unwrap()
        }
    }

    impl ProcReader for FakeProcReader {
        fn read_stat(&self, _pid: u32) -> Option<String> {
            self.stat_content.lock().unwrap().clone()
        }

        fn send_sigterm(&self, _pid: u32) -> std::result::Result<(), std::io::Error> {
            *self.sigterm_called.lock().unwrap() = true;
            let mut result = self.sigterm_result.lock().unwrap();
            // Take the result to satisfy borrow checker; replace with Ok for subsequent calls
            std::mem::replace(&mut *result, Ok(()))
        }

        fn current_uid(&self) -> u32 {
            self.uid
        }
    }

    const SAMPLE_STAT: &str = "1234 (firefox) S 1100 1234 1100 0 -1 4194304 50000 0 0 0 12345 6789 0 0 20 0 30 0 98765 1234567890 234567 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 1 0 0 0 0 0 0 0 0 0 0 0 0 0";

    #[test]
    fn same_pid_same_start_time_sends_sigterm() {
        let identity = ProcessIdentity {
            pid: 1234,
            start_time_ticks: 98765,
        };
        let reader = FakeProcReader::with_stat(SAMPLE_STAT);

        let result = terminate_process(&identity, &reader, false).unwrap();
        assert_eq!(result, TerminateResult::Sent);
        assert!(reader.was_sigterm_called());
    }

    #[test]
    fn pid_reused_different_start_time() {
        let identity = ProcessIdentity {
            pid: 1234,
            start_time_ticks: 11111, // Different from 98765 in stat
        };
        let reader = FakeProcReader::with_stat(SAMPLE_STAT);

        let result = terminate_process(&identity, &reader, false).unwrap();
        assert_eq!(result, TerminateResult::PidReused);
        assert!(!reader.was_sigterm_called());
    }

    #[test]
    fn already_exited_process() {
        let identity = ProcessIdentity {
            pid: 1234,
            start_time_ticks: 98765,
        };
        let reader = FakeProcReader::exited();

        let result = terminate_process(&identity, &reader, false).unwrap();
        assert_eq!(result, TerminateResult::AlreadyExited);
        assert!(!reader.was_sigterm_called());
    }

    #[test]
    fn permission_denied_for_foreign_process() {
        let identity = ProcessIdentity {
            pid: 1234,
            start_time_ticks: 98765,
        };
        let reader = FakeProcReader::with_permission_denied(SAMPLE_STAT);

        let result = terminate_process(&identity, &reader, false).unwrap();
        assert!(matches!(result, TerminateResult::PermissionRequired(_)));
    }

    #[test]
    fn cancellation_before_signal() {
        let identity = ProcessIdentity {
            pid: 1234,
            start_time_ticks: 98765,
        };
        let reader = FakeProcReader::with_stat(SAMPLE_STAT);

        let result = terminate_process(&identity, &reader, true).unwrap();
        assert_eq!(result, TerminateResult::Cancelled);
        assert!(!reader.was_sigterm_called());
    }

    #[test]
    fn parse_start_time_from_stat() {
        let start_time = parse_start_time(SAMPLE_STAT).unwrap();
        assert_eq!(start_time, 98765);
    }

    #[test]
    fn parse_start_time_with_spaces_in_name() {
        let stat = "12345 (Web Content) S 1000 12345 1000 0 -1 4194304 100 0 0 0 200 50 0 0 20 0 5 0 5000 1234567890 5678 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 1 0 0 0 0 0 0 0 0 0 0 0 0 0";
        let start_time = parse_start_time(stat).unwrap();
        assert_eq!(start_time, 5000);
    }
}
