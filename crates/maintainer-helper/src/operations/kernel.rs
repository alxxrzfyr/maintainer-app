//! Kernel removal remains unavailable without a complete bound removal plan.

use crate::api::{HelperError, OperationResponse, RemoveKernelRequest};

/// Caller authorization and a validated kernel name do not approve resolver
/// side effects. The request has no complete reviewed dependency/removal plan,
/// and the legacy process cannot preserve package ownership after disconnect.
/// Reject before spawning APT or modifying dpkg/boot state.
pub fn execute_remove_kernel(
    _request: &RemoveKernelRequest,
) -> Result<OperationResponse, HelperError> {
    Err(HelperError::OperationFailed {
        reason: maintainer_core::updates::PACKAGE_EXECUTION_UNAVAILABLE_REASON.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validated_kernel_name_does_not_authorize_unreviewed_dependencies() {
        let request = RemoveKernelRequest {
            package_name: "linux-image-6.5.0-35-generic".to_string(),
        };
        assert!(matches!(
            execute_remove_kernel(&request),
            Err(HelperError::OperationFailed { reason })
                if reason.contains("reviewed-plan binding")
        ));
    }
}
