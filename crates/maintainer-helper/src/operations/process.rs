//! Process termination with PID reuse prevention.
//!
//! Always sends SIGTERM. Never exposes signal selection.
//! Validates process start time immediately before kill to prevent
//! sending signals to a recycled PID.

use std::fs;
use std::io;

use crate::api::{HelperError, OperationResponse, TerminateProcessRequest};

/// Read the start time (field 22, `starttime`) from `/proc/<pid>/stat`.
///
/// Returns `None` if the process does not exist or the file cannot be parsed.
pub fn read_process_start_time(pid: u32) -> Result<u64, HelperError> {
    let stat_path = format!("/proc/{pid}/stat");
    let contents = fs::read_to_string(&stat_path).map_err(|e| match e.kind() {
        io::ErrorKind::NotFound => HelperError::OperationFailed {
            reason: format!("process {pid} does not exist"),
        },
        io::ErrorKind::PermissionDenied => HelperError::OperationFailed {
            reason: format!("permission denied reading /proc/{pid}/stat"),
        },
        _ => HelperError::OperationFailed {
            reason: format!("failed to read /proc/{pid}/stat: {e}"),
        },
    })?;

    parse_start_time(&contents).ok_or_else(|| HelperError::OperationFailed {
        reason: format!("failed to parse start time from /proc/{pid}/stat"),
    })
}

/// Parse start time from the contents of `/proc/<pid>/stat`.
///
/// Field 22 (0-indexed from the end of the comm field in parens) is `starttime`.
/// The comm field (field 2) is enclosed in parentheses and may contain spaces or
/// other special characters, so we find the last `)` to split reliably.
fn parse_start_time(stat_contents: &str) -> Option<u64> {
    // Find last ')' to skip the comm field which may contain spaces/parens.
    let after_comm = stat_contents.rfind(')')? + 1;
    let remainder = stat_contents.get(after_comm..)?;

    // Fields after comm: state(3), ppid(4), pgrp(5), session(6), tty_nr(7),
    // tpgid(8), flags(9), minflt(10), cminflt(11), majflt(12), cmajflt(13),
    // utime(14), stime(15), cutime(16), cstime(17), priority(18), nice(19),
    // num_threads(20), itrealvalue(21), starttime(22)
    // After the ')' we have fields 3..N, so starttime is at index 19 (0-based)
    // from the fields after ')'.
    let fields: Vec<&str> = remainder.split_whitespace().collect();
    // Field index 19 after `)` = starttime (field 22 in 1-based /proc stat)
    let start_time_str = fields.get(19)?;
    start_time_str.parse::<u64>().ok()
}

/// Execute process termination with start-time revalidation.
///
/// # Safety contract
///
/// This function sends SIGTERM to a process. It re-reads the process start time
/// immediately before the kill syscall to prevent PID reuse attacks.
pub fn execute_terminate(request: &TerminateProcessRequest) -> Result<OperationResponse, HelperError> {
    // Revalidate start time immediately before signal delivery.
    let current_start_time = read_process_start_time(request.pid)?;

    if current_start_time != request.start_time_ticks {
        return Err(HelperError::PidReuse);
    }

    // Send SIGTERM only. Signal is not configurable.
    let result = unsafe { libc::kill(request.pid as libc::pid_t, libc::SIGTERM) };

    if result == 0 {
        Ok(OperationResponse {
            success: true,
            message: format!("SIGTERM sent to pid {}", request.pid),
        })
    } else {
        let errno = io::Error::last_os_error();
        Err(HelperError::OperationFailed {
            reason: format!("kill({}, SIGTERM) failed: {errno}", request.pid),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_start_time_normal_comm() {
        // Simulated /proc/1234/stat content
        let stat = "1234 (bash) S 1233 1234 1234 0 -1 4194304 1000 0 0 0 10 5 0 0 20 0 1 0 98765 10000000 500 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let start_time = parse_start_time(stat);
        assert_eq!(start_time, Some(98765));
    }

    #[test]
    fn parse_start_time_comm_with_spaces() {
        // Comm field can contain spaces: "(Web Content)"
        let stat = "5678 (Web Content) S 5600 5678 5600 0 -1 4194304 2000 0 0 0 50 10 0 0 20 0 4 0 54321 20000000 1000 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let start_time = parse_start_time(stat);
        assert_eq!(start_time, Some(54321));
    }

    #[test]
    fn parse_start_time_comm_with_parens() {
        // Comm field can contain parentheses
        let stat = "9999 (foo (bar)) S 9998 9999 9998 0 -1 4194304 500 0 0 0 5 2 0 0 20 0 1 0 11111 5000000 250 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 0 0 0 0 0 0";
        let start_time = parse_start_time(stat);
        assert_eq!(start_time, Some(11111));
    }

    #[test]
    fn parse_start_time_empty_string() {
        assert_eq!(parse_start_time(""), None);
    }

    #[test]
    fn parse_start_time_truncated() {
        let stat = "1234 (bash) S 1233";
        assert_eq!(parse_start_time(stat), None);
    }

    #[test]
    fn execute_terminate_detects_nonexistent_process() {
        let request = TerminateProcessRequest {
            pid: 999_999_999,
            start_time_ticks: 12345,
        };
        let result = execute_terminate(&request);
        assert!(result.is_err());
    }
}
