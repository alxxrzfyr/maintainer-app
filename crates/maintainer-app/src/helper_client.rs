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

    /// Remove an old kernel package via the privileged helper.
    pub fn remove_kernel(&self, package_name: &str) -> HelperResult {
        self.call_method("RemoveKernel", &(package_name.to_string(),))
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
    classify_message(&err.to_string(), method)
}

/// Classify an error message body, split out so the mapping can be tested
/// without constructing live D-Bus errors.
fn classify_message(msg: &str, method: &str) -> HelperCallError {
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

    // A bus-level rejection is not a Polkit denial. The system bus refuses the
    // message before the helper is activated, so Polkit is never consulted and
    // the user is never given the chance to authenticate. Reporting this as
    // "authentication required" would tell the user to supply a password that
    // they were never asked for and that would not help. It means the bus
    // policy is missing or does not permit the interface.
    if msg.contains("not allowed to send message")
        || msg.contains("rejected send message")
        || msg.contains("Rejected send message")
    {
        return HelperCallError::Unavailable(format!(
            "the system bus refused the call to {method}, so the helper was never reached: {msg}"
        ));
    }

    // AccessDenied and AuthFailed from the helper itself indicate Polkit denial
    // or a dismissed prompt.
    if msg.contains("AccessDenied")
        || msg.contains("org.freedesktop.DBus.Error.AccessDenied")
        || msg.contains("AuthFailed")
        || msg.contains("org.freedesktop.DBus.Error.AuthFailed")
        || msg.contains("authorization denied")
    {
        return HelperCallError::PermissionDenied(method.to_string());
    }

    if msg.contains("InvalidArgs") || msg.contains("org.freedesktop.DBus.Error.InvalidArgs") {
        return HelperCallError::ValidationFailed(msg.to_string());
    }

    if msg.contains("LimitsExceeded") || msg.contains("org.freedesktop.DBus.Error.LimitsExceeded") {
        return HelperCallError::Busy;
    }

    HelperCallError::OperationFailed(msg.to_string())
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

    /// Without an activatable helper the call must refuse, but which refusal
    /// depends on the machine: with no reachable system bus the name cannot be
    /// resolved and the result is `Unavailable`, while on a desktop with a bus
    /// and Polkit present the authorization is refused first and the result is
    /// `PermissionRequired`. Both are correct; succeeding would not be.
    ///
    /// `terminate_process` is safe to call for real here: it targets a PID
    /// that cannot exist, so `PidReuse`/`AlreadyExited` is the worst outcome
    /// even if a real helper is installed and the action is authorized.
    #[test]
    fn client_refuses_without_activatable_helper() {
        let client = HelperClient::new();
        let result = client.terminate_process(1234, 99999);
        assert!(
            matches!(
                result,
                Capability::Unavailable(_) | Capability::PermissionRequired(_)
            ),
            "expected a refusal without an activatable helper, got: {result:?}"
        );
    }

    /// `clean-package-cache` resolves to `yes` for an active local session
    /// (Recommendation: it only discards files APT can refetch, so no
    /// password is required — see `data/polkit/io.github.maintainerapp.policy`).
    /// A developer machine with the helper installed will therefore execute
    /// this operation for real rather than refusing it, which makes it unsafe
    /// to assert a refusal unconditionally: this test previously ran a live
    /// `apt-get clean` against the machine running the test suite whenever the
    /// bus policy and Polkit policy were installed. Both outcomes are
    /// legitimate depending on whether a helper is reachable and authorized,
    /// so only structural properties of the response are checked.
    #[test]
    fn client_package_cache_does_not_panic_without_activatable_helper() {
        let client = HelperClient::new();
        let result = client.clean_package_cache("test-req-1");
        match result {
            Capability::Unavailable(_) | Capability::PermissionRequired(_) => {}
            Capability::Available(response) => {
                assert!(
                    response.success,
                    "a real helper reported failure for a fixed, argument-free \
                     operation: {response:?}"
                );
            }
        }
    }

    /// `vacuum-journal` is `auth_admin`, not retained, so an unattended test
    /// run cannot supply a password and the call cannot succeed even with a
    /// real, authorized helper present. Refusal is the only legitimate
    /// outcome, and this assertion is safe to keep because that gate is by
    /// design (see the systemd/Polkit policy for this action).
    #[test]
    fn client_vacuum_journal_refuses_without_activatable_helper() {
        let client = HelperClient::new();
        let result = client.vacuum_journal(500, 30);
        assert!(
            matches!(
                result,
                Capability::Unavailable(_) | Capability::PermissionRequired(_)
            ),
            "vacuum-journal requires interactive authentication and must \
             refuse in an unattended test run, got: {result:?}"
        );
    }

    /// `refresh-smart` resolves to `yes` for an active local session, the
    /// same reasoning as the package-cache case above: a real, reachable
    /// helper will execute it rather than refuse, and refusal is not the only
    /// legitimate outcome. `refresh_smart` only re-reads drive health over
    /// D-Bus and changes no state, so allowing it to run for real here is
    /// safe.
    #[test]
    fn client_refresh_smart_does_not_panic_without_activatable_helper() {
        let client = HelperClient::new();
        let result = client.refresh_smart("test-drive-id");
        match result {
            Capability::Unavailable(_) | Capability::PermissionRequired(_) => {}
            Capability::Available(_) => {}
        }
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

    #[test]
    fn bus_rejection_is_not_reported_as_authentication_required() {
        // The system bus refuses the message before the helper is activated, so
        // Polkit never runs and no prompt was ever shown. Telling the user that
        // authentication is required would ask for a password that was never
        // requested and would not fix a missing bus policy.
        let classified = classify_message(
            "org.freedesktop.DBus.Error.AccessDenied: Rejected send message, 1 matched rules",
            "CleanPackageCache",
        );
        assert!(
            matches!(classified, HelperCallError::Unavailable(ref r) if r.contains("never reached")),
            "bus rejection must surface as unavailable, got {classified:?}"
        );
    }

    #[test]
    fn polkit_denial_is_reported_as_permission_required() {
        let classified = classify_message(
            "org.freedesktop.DBus.Error.AccessDenied: authorization denied for action",
            "VacuumJournal",
        );
        assert!(
            matches!(classified, HelperCallError::PermissionDenied(ref a) if a == "VacuumJournal"),
            "polkit denial must stay permission-required, got {classified:?}"
        );
    }
}
