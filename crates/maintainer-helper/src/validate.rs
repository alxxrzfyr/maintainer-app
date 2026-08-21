//! Parameter validation for helper operation requests.
//!
//! All validation happens before any privileged action is taken.
//! Bounds are chosen to prevent abuse while remaining useful.

use crate::api::{
    HelperError, JournalVacuumRequest, PackageCacheRequest, SmartRefreshRequest,
    TerminateProcessRequest,
};

/// Minimum journal vacuum size in MiB.
pub const JOURNAL_MIN_SIZE_MIB: u32 = 100;
/// Maximum journal vacuum size in MiB.
pub const JOURNAL_MAX_SIZE_MIB: u32 = 4096;
/// Minimum journal vacuum age in days.
pub const JOURNAL_MIN_AGE_DAYS: u32 = 1;
/// Maximum journal vacuum age in days.
pub const JOURNAL_MAX_AGE_DAYS: u32 = 365;

/// Minimum PID value (PID 1 is init, not terminable by this helper).
pub const MIN_TERMINABLE_PID: u32 = 2;

/// Maximum reasonable request ID length.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Maximum drive ID length.
const MAX_DRIVE_ID_LEN: usize = 256;

/// Validate a process termination request.
pub fn validate_terminate_process(request: &TerminateProcessRequest) -> Result<(), HelperError> {
    if request.pid < MIN_TERMINABLE_PID {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "pid {} is below minimum terminable pid {}",
                request.pid, MIN_TERMINABLE_PID
            ),
        });
    }

    if request.start_time_ticks == 0 {
        return Err(HelperError::ValidationFailed {
            reason: "start_time_ticks must be non-zero".to_string(),
        });
    }

    Ok(())
}

/// Validate a package cache clean request.
pub fn validate_package_cache(request: &PackageCacheRequest) -> Result<(), HelperError> {
    if request.request_id.is_empty() {
        return Err(HelperError::ValidationFailed {
            reason: "request_id must not be empty".to_string(),
        });
    }

    if request.request_id.len() > MAX_REQUEST_ID_LEN {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "request_id length {} exceeds maximum {}",
                request.request_id.len(),
                MAX_REQUEST_ID_LEN
            ),
        });
    }

    // Request ID must be printable ASCII only (no control chars or binary).
    if !request
        .request_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(HelperError::ValidationFailed {
            reason: "request_id contains invalid characters; only [a-zA-Z0-9_-] allowed"
                .to_string(),
        });
    }

    Ok(())
}

/// Validate a journal vacuum request against bounded size and age policy.
pub fn validate_journal_vacuum(request: &JournalVacuumRequest) -> Result<(), HelperError> {
    if request.max_size_mib < JOURNAL_MIN_SIZE_MIB || request.max_size_mib > JOURNAL_MAX_SIZE_MIB {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "max_size_mib {} outside bounds [{}, {}]",
                request.max_size_mib, JOURNAL_MIN_SIZE_MIB, JOURNAL_MAX_SIZE_MIB
            ),
        });
    }

    if request.max_age_days < JOURNAL_MIN_AGE_DAYS || request.max_age_days > JOURNAL_MAX_AGE_DAYS {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "max_age_days {} outside bounds [{}, {}]",
                request.max_age_days, JOURNAL_MIN_AGE_DAYS, JOURNAL_MAX_AGE_DAYS
            ),
        });
    }

    Ok(())
}

/// Validate a SMART refresh request.
///
/// Drive ID must match the UDisks2 drive identifier pattern:
/// alphanumeric characters and underscores only.
pub fn validate_smart_refresh(request: &SmartRefreshRequest) -> Result<(), HelperError> {
    if request.drive_id.is_empty() {
        return Err(HelperError::ValidationFailed {
            reason: "drive_id must not be empty".to_string(),
        });
    }

    if request.drive_id.len() > MAX_DRIVE_ID_LEN {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "drive_id length {} exceeds maximum {}",
                request.drive_id.len(),
                MAX_DRIVE_ID_LEN
            ),
        });
    }

    // UDisks2 drive identifiers are strictly alphanumeric + underscore.
    // No slashes, dots, or other characters that could enable path traversal.
    if !request
        .drive_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(HelperError::ValidationFailed {
            reason: "drive_id contains invalid characters; only [a-zA-Z0-9_] allowed".to_string(),
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminate_process_rejects_pid_zero() {
        let req = TerminateProcessRequest {
            pid: 0,
            start_time_ticks: 12345,
        };
        let err = validate_terminate_process(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn terminate_process_rejects_pid_one() {
        let req = TerminateProcessRequest {
            pid: 1,
            start_time_ticks: 12345,
        };
        let err = validate_terminate_process(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn terminate_process_rejects_zero_start_time() {
        let req = TerminateProcessRequest {
            pid: 100,
            start_time_ticks: 0,
        };
        let err = validate_terminate_process(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn terminate_process_accepts_valid_request() {
        let req = TerminateProcessRequest {
            pid: 1234,
            start_time_ticks: 98765,
        };
        assert!(validate_terminate_process(&req).is_ok());
    }

    #[test]
    fn package_cache_rejects_empty_request_id() {
        let req = PackageCacheRequest {
            request_id: String::new(),
        };
        let err = validate_package_cache(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn package_cache_rejects_oversized_request_id() {
        let req = PackageCacheRequest {
            request_id: "a".repeat(MAX_REQUEST_ID_LEN + 1),
        };
        let err = validate_package_cache(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn package_cache_rejects_special_characters() {
        let req = PackageCacheRequest {
            request_id: "valid-id/../../../etc/passwd".to_string(),
        };
        let err = validate_package_cache(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn package_cache_accepts_valid_id() {
        let req = PackageCacheRequest {
            request_id: "req-123_abc".to_string(),
        };
        assert!(validate_package_cache(&req).is_ok());
    }

    #[test]
    fn journal_vacuum_rejects_size_below_minimum() {
        let req = JournalVacuumRequest {
            max_size_mib: 50,
            max_age_days: 30,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_rejects_size_above_maximum() {
        let req = JournalVacuumRequest {
            max_size_mib: 5000,
            max_age_days: 30,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_rejects_age_below_minimum() {
        let req = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 0,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_rejects_age_above_maximum() {
        let req = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 400,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_accepts_valid_bounds() {
        let req = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 30,
        };
        assert!(validate_journal_vacuum(&req).is_ok());
    }

    #[test]
    fn smart_refresh_rejects_empty_drive_id() {
        let req = SmartRefreshRequest {
            drive_id: String::new(),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_rejects_path_traversal() {
        let req = SmartRefreshRequest {
            drive_id: "../../etc/shadow".to_string(),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_rejects_slashes() {
        let req = SmartRefreshRequest {
            drive_id: "drive/with/slashes".to_string(),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_rejects_oversized_id() {
        let req = SmartRefreshRequest {
            drive_id: "A".repeat(MAX_DRIVE_ID_LEN + 1),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_accepts_valid_udisks_id() {
        let req = SmartRefreshRequest {
            drive_id: "SAMSUNG_MZVL21T0HCLR_0BL2_S64FNX0T506050".to_string(),
        };
        assert!(validate_smart_refresh(&req).is_ok());
    }
}
