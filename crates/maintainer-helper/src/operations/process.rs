//! SIGTERM delivery through a descriptor retained across review and Polkit.

use std::os::fd::BorrowedFd;

use maintainer_core::metrics::ProcessIdentity;
use maintainer_core::metrics::terminate::{ProcessHandleError, send_sigterm, validate_reference};

use crate::api::{HelperError, OperationResponse, TerminateProcessRequest};

fn process_error(error: ProcessHandleError) -> HelperError {
    match error {
        ProcessHandleError::InvalidIdentity => HelperError::ValidationFailed {
            reason: error.to_string(),
        },
        ProcessHandleError::IdentityChanged => HelperError::PidReuse,
        ProcessHandleError::Vanished => HelperError::ProcessVanished,
        ProcessHandleError::Unavailable => HelperError::ProcessUnavailable,
        ProcessHandleError::PermissionRequired => HelperError::AuthorizationDenied {
            action: crate::api::actions::TERMINATE_PROCESS.into(),
        },
        _ => HelperError::OperationFailed {
            reason: error.to_string(),
        },
    }
}

/// Reject forged descriptors or mismatched metadata before prompting Polkit.
pub fn validate_handle(
    request: &TerminateProcessRequest,
    fd: BorrowedFd<'_>,
) -> Result<(), HelperError> {
    validate_reference(
        fd,
        &ProcessIdentity {
            pid: request.pid,
            start_time_ticks: request.start_time_ticks,
        },
    )
    .map(|_| ())
    .map_err(process_error)
}

pub fn execute_terminate(fd: BorrowedFd<'_>) -> Result<OperationResponse, HelperError> {
    send_sigterm(fd).map_err(process_error)?;
    Ok(OperationResponse {
        success: true,
        message: "SIGTERM accepted by the kernel; process exit is not yet confirmed.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::fd::AsFd;

    #[test]
    fn arbitrary_descriptor_cannot_enter_authorization() {
        let file = std::fs::File::open("/dev/null").unwrap();
        let request = TerminateProcessRequest {
            pid: std::process::id(),
            start_time_ticks: 1,
        };
        assert!(matches!(
            validate_handle(&request, file.as_fd()),
            Err(HelperError::ValidationFailed { .. })
        ));
    }
}
