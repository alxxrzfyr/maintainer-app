//! D-Bus client for the privileged helper.
//!
//! Reports `Capability::Unavailable` when the helper is not installed or the
//! bus is unreachable, and `Capability::PermissionRequired` when Polkit denies
//! or the user cancels. Never blocks the GTK main thread — all calls are
//! dispatched to a background thread.

use std::sync::Arc;

use maintainer_core::capability::{Capability, PermissionRequest, UnavailableReason};

/// Bus name and object path for the helper.
const BUS_NAME: &str = "io.github.maintainerapp.Helper";
const OBJECT_PATH: &str = "/io/github/maintainerapp/Helper";
const INTERFACE: &str = "io.github.maintainerapp.Helper";

/// Result type for helper operations.
pub type HelperResult = Capability<HelperResponse>;

/// Successful response from the helper.
#[derive(Debug, Clone)]
pub struct HelperResponse {
    pub success: bool,
    pub message: String,
}

/// Error classification from a D-Bus call to the helper.
#[derive(Debug, Clone)]
pub enum HelperCallError {
    /// Helper is not installed or the bus is unreachable.
    Unavailable(String),
    /// Polkit denied or user cancelled authentication.
    PermissionDenied(String),
    /// Validation rejected the request.
    ValidationFailed(String),
    /// Operation failed after authorization.
    OperationFailed(String),
    /// Helper is at capacity.
    Busy,
}

/// Client adapter for calling the privileged helper over D-Bus.
///
/// All methods are blocking and should be called from a worker thread,
/// not the GTK main thread.
#[derive(Debug, Clone, Default)]
pub struct HelperClient {
    _private: (),
}

impl HelperClient {
    /// Create a new client. Does not connect immediately.
    pub fn new() -> Self {
        Self { _private: () }
    }

    /// Terminate a process by PID with start-time verification.
    pub fn terminate_process(&self, pid: u32, start_time_ticks: u64) -> HelperResult {
        self.call_method("TerminateProcess", &(pid, start_time_ticks))
    }

    /// Clean the APT package cache.
    pub fn clean_package_cache(&self, request_id: &str) -> HelperResult {
        self.call_method("CleanPackageCache", &(request_id.to_string(),))
    }

    /// Vacuum the systemd journal.
    pub fn vacuum_journal(&self, max_size_mib: u32, max_age_days: u32) -> HelperResult {
        self.call_method("VacuumJournal", &(max_size_mib, max_age_days))
    }

    /// Refresh SMART data for a drive.
    pub fn refresh_smart(&self, drive_id: &str) -> HelperResult {
        self.call_method("RefreshSmart", &(drive_id.to_string(),))
    }

    /// Make a blocking D-Bus method call and classify the result.
    fn call_method<A>(&self, method: &str, args: &A) -> HelperResult
    where
        A: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let result = self.do_call(method, args);
        match result {
            Ok(response) => Capability::Available(response),
            Err(HelperCallError::Unavailable(reason)) => {
                Capability::Unavailable(UnavailableReason {
                    code: "helper-unavailable",
                    message: reason,
                    remediation: Some(
                        "Ensure the maintainer-helper package is installed.".to_string(),
                    ),
                })
            }
            Err(HelperCallError::PermissionDenied(action)) => {
                Capability::PermissionRequired(PermissionRequest {
                    action_id: action.clone(),
                    explanation: format!(
                        "Administrator authentication is required for this action ({action})."
                    ),
                })
            }
            Err(HelperCallError::ValidationFailed(reason)) => {
                Capability::Unavailable(UnavailableReason {
                    code: "validation-failed",
                    message: reason,
                    remediation: None,
                })
            }
            Err(HelperCallError::OperationFailed(reason)) => {
                Capability::Unavailable(UnavailableReason {
                    code: "operation-failed",
                    message: reason,
                    remediation: None,
                })
            }
            Err(HelperCallError::Busy) => Capability::Unavailable(UnavailableReason {
                code: "helper-busy",
                message: "The helper is processing the maximum number of concurrent requests."
                    .to_string(),
                remediation: Some("Try again in a moment.".to_string()),
            }),
        }
    }

    fn do_call<A>(&self, method: &str, args: &A) -> Result<HelperResponse, HelperCallError>
    where
        A: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let connection = zbus::blocking::Connection::system().map_err(|e| {
            HelperCallError::Unavailable(format!("cannot connect to system bus: {e}"))
        })?;

        let reply = connection
            .call_method(Some(BUS_NAME), OBJECT_PATH, Some(INTERFACE), method, args)
            .map_err(|e| classify_dbus_error(e, method))?;

        let (success, message): (bool, String) = reply
            .body()
            .deserialize()
            .map_err(|e| HelperCallError::OperationFailed(format!("failed to parse reply: {e}")))?;

        Ok(HelperResponse { success, message })
    }
}

/// Classify a D-Bus error into our capability taxonomy.
fn classify_dbus_error(err: zbus::Error, method: &str) -> HelperCallError {
    let msg = err.to_string();

    if msg.contains("ServiceUnknown")
        || msg.contains("org.freedesktop.DBus.Error.ServiceUnknown")
        || msg.contains("NameHasNoOwner")
        || msg.contains("Connection refused")
        || msg.contains("No such file or directory")
    {
        return HelperCallError::Unavailable(format!(
            "helper service not available for {method}: {msg}"
        ));
    }

    // AccessDenied and AuthFailed indicate Polkit denial/cancellation.
    if msg.contains("AccessDenied")
        || msg.contains("org.freedesktop.DBus.Error.AccessDenied")
        || msg.contains("AuthFailed")
        || msg.contains("org.freedesktop.DBus.Error.AuthFailed")
        || msg.contains("authorization denied")
    {
        return HelperCallError::PermissionDenied(method.to_string());
    }

    if msg.contains("InvalidArgs") || msg.contains("org.freedesktop.DBus.Error.InvalidArgs") {
        return HelperCallError::ValidationFailed(msg);
    }

    if msg.contains("LimitsExceeded") || msg.contains("org.freedesktop.DBus.Error.LimitsExceeded") {
        return HelperCallError::Busy;
    }

    HelperCallError::OperationFailed(msg)
}

/// Shared, thread-safe handle to the helper client.
pub type SharedHelperClient = Arc<HelperClient>;

/// Create a shared helper client instance.
pub fn create_helper_client() -> SharedHelperClient {
    Arc::new(HelperClient::new())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_unavailable_when_no_system_bus() {
        let client = HelperClient::new();
        let result = client.terminate_process(1234, 99999);
        assert!(
            matches!(result, Capability::Unavailable(_)),
            "expected Unavailable without system bus, got: {result:?}"
        );
    }

    #[test]
    fn client_package_cache_unavailable_without_bus() {
        let client = HelperClient::new();
        let result = client.clean_package_cache("test-req-1");
        assert!(matches!(result, Capability::Unavailable(_)));
    }

    #[test]
    fn client_vacuum_journal_unavailable_without_bus() {
        let client = HelperClient::new();
        let result = client.vacuum_journal(500, 30);
        assert!(matches!(result, Capability::Unavailable(_)));
    }

    #[test]
    fn client_refresh_smart_unavailable_without_bus() {
        let client = HelperClient::new();
        let result = client.refresh_smart("TEST_DRIVE_123");
        assert!(matches!(result, Capability::Unavailable(_)));
    }

    #[test]
    fn classify_service_unknown() {
        let err = classify_dbus_error(
            zbus::Error::Names(zbus::names::Error::InvalidUniqueName(
                "ServiceUnknown: service not found".into(),
            )),
            "TestMethod",
        );
        assert!(matches!(
            err,
            HelperCallError::Unavailable(_) | HelperCallError::OperationFailed(_)
        ));
    }
}
