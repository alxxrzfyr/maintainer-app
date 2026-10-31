//! Local and privileged action scheduling.

use std::sync::Arc;

use maintainer_core::jobs::{CancellationToken, JobExecutor};

use super::install_result_poller;

/// Local action failures include scheduler rejection and worker termination.
pub trait LocalActionError: Send + 'static {
    fn from_job_failure(message: String) -> Self;
    fn retained_bytes(&self) -> usize;
}

impl LocalActionError for String {
    fn from_job_failure(message: String) -> Self {
        message
    }
    fn retained_bytes(&self) -> usize {
        self.capacity()
    }
}

impl LocalActionError for std::io::Error {
    fn from_job_failure(message: String) -> Self {
        Self::other(message)
    }
    fn retained_bytes(&self) -> usize {
        // io::Error hides its allocation; only bounded local filesystem errors are used here.
        4096
    }
}

/// Run a local blocking action and deliver its typed result on the GTK main thread.
pub fn dispatch_local_action<T: Send + 'static, E: LocalActionError>(
    executor: &Arc<JobExecutor>,
    operation: impl FnOnce() -> Result<T, E> + Send + 'static,
    on_done: impl FnOnce(Result<T, E>) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<Result<T, E>> { Ok(operation()) };
    let receiver = executor.execute_sized(vec![job], token.clone(), |value| {
        std::mem::size_of_val(value)
            + value
                .as_ref()
                .err()
                .map_or(0, LocalActionError::retained_bytes)
    });
    install_result_poller(receiver, on_done, |message| {
        Err(E::from_job_failure(message))
    });
    token
}

pub fn dispatch_helper_call(
    executor: &Arc<JobExecutor>,
    operation: impl FnOnce() -> crate::helper_client::HelperResult + Send + 'static,
    on_done: impl FnOnce(crate::helper_client::HelperResult) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<crate::helper_client::HelperResult> {
        Ok(operation())
    };
    let receiver = executor.execute_sized(vec![job], token.clone(), helper_bytes);
    install_result_poller(receiver, on_done, helper_failure);
    token
}

fn helper_failure(message: String) -> crate::helper_client::HelperResult {
    maintainer_core::capability::Capability::Unavailable(
        maintainer_core::capability::UnavailableReason {
            code: "background-operation-failed",
            message,
            remediation: None,
        },
    )
}

fn helper_bytes(value: &crate::helper_client::HelperResult) -> usize {
    use maintainer_core::capability::Capability;
    std::mem::size_of_val(value)
        + match value {
            Capability::Available(response) => response.message.capacity(),
            Capability::Unavailable(reason) => {
                reason.message.capacity() + reason.remediation.as_ref().map_or(0, String::capacity)
            }
            Capability::PermissionRequired(request) => {
                request.action_id.capacity() + request.explanation.capacity()
            }
        }
}
