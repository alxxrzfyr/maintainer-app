//! Journal vacuum via `journalctl` with bounded size and age.
//!
//! Only accepts validated numeric parameters within policy bounds.
//! No shell invocation, no arbitrary arguments.

use std::process::Command;

use crate::api::{HelperError, JournalVacuumRequest, OperationResponse};

/// Fixed path to journalctl binary.
const JOURNALCTL_PATH: &str = "/usr/bin/journalctl";

/// Execute journal vacuum with validated, bounded size and age parameters.
///
/// Parameters are pre-validated by `validate::validate_journal_vacuum` before
/// this function is called.
pub fn execute_vacuum_journal(
    request: &JournalVacuumRequest,
) -> Result<OperationResponse, HelperError> {
    let vacuum_size = format!("{}M", request.max_size_mib);
    let vacuum_time = format!("{}d", request.max_age_days);

    let output = Command::new(JOURNALCTL_PATH)
        .arg("--vacuum-size")
        .arg(&vacuum_size)
        .arg("--vacuum-time")
        .arg(&vacuum_time)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .output()
        .map_err(|e| HelperError::OperationFailed {
            reason: format!("failed to execute journalctl: {e}"),
        })?;

    if output.status.success() {
        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(OperationResponse {
            success: true,
            message: format!("journal vacuumed: {stdout}"),
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(HelperError::OperationFailed {
            reason: format!("journalctl vacuum failed: {stderr}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn journalctl_path_is_absolute() {
        assert!(JOURNALCTL_PATH.starts_with('/'));
    }

    #[test]
    fn vacuum_args_are_numeric_only() {
        let request = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 30,
        };
        let size_arg = format!("{}M", request.max_size_mib);
        let time_arg = format!("{}d", request.max_age_days);

        assert_eq!(size_arg, "500M");
        assert_eq!(time_arg, "30d");

        assert!(size_arg.chars().all(|c| c.is_ascii_digit() || c == 'M'));
        assert!(time_arg.chars().all(|c| c.is_ascii_digit() || c == 'd'));
    }
}
