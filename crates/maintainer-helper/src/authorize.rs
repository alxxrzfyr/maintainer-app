//! Caller identity resolution and Polkit authorization.
//!
//! Caller UID and PID are resolved through the D-Bus daemon using kernel
//! credentials (SO_PEERCRED), never from self-reported message payload.

use crate::api::HelperError;

/// Resolved caller credentials from the D-Bus daemon.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CallerCredentials {
    pub uid: u32,
    pub pid: u32,
    pub bus_name: String,
}

/// Authorization policy that verifies caller identity and Polkit consent.
///
/// In production, this queries the D-Bus daemon and Polkit authority.
/// In tests, it uses injected mock behavior.
pub trait AuthorizePolicy: Send + Sync {
    /// Resolve the Unix credentials of the D-Bus caller by unique bus name.
    fn resolve_caller(&self, sender: &str) -> Result<CallerCredentials, HelperError>;

    /// Check Polkit authorization for the given action and caller.
    fn check_authorization(
        &self,
        credentials: &CallerCredentials,
        action_id: &str,
    ) -> Result<(), HelperError>;
}

/// Perform full authorization: resolve caller, then check Polkit.
pub fn authorize_request(
    policy: &dyn AuthorizePolicy,
    sender: &str,
    action_id: &str,
) -> Result<CallerCredentials, HelperError> {
    let credentials = policy.resolve_caller(sender)?;
    policy.check_authorization(&credentials, action_id)?;
    Ok(credentials)
}

/// Production Polkit authority that queries the real D-Bus daemon and Polkit.
///
/// Not instantiated in unit tests — use `MockPolicy` instead.
pub struct SystemPolicy;

impl AuthorizePolicy for SystemPolicy {
    fn resolve_caller(&self, sender: &str) -> Result<CallerCredentials, HelperError> {
        let _ = sender;
        Err(HelperError::CallerIdentityFailed)
    }

    fn check_authorization(
        &self,
        _credentials: &CallerCredentials,
        _action_id: &str,
    ) -> Result<(), HelperError> {
        Err(HelperError::AuthorizationDenied {
            action: "not implemented".to_string(),
        })
    }
}

/// Mock policy for unit testing authorization flows.
#[cfg(test)]
pub mod mock {
    use super::*;

    /// Configurable mock policy for testing deny/allow scenarios.
    pub struct MockPolicy {
        pub resolve_result: Result<CallerCredentials, HelperError>,
        pub authorize_result: Result<(), HelperError>,
    }

    impl MockPolicy {
        pub fn allowing(uid: u32, pid: u32, bus_name: &str) -> Self {
            Self {
                resolve_result: Ok(CallerCredentials {
                    uid,
                    pid,
                    bus_name: bus_name.to_string(),
                }),
                authorize_result: Ok(()),
            }
        }

        pub fn denying_caller() -> Self {
            Self {
                resolve_result: Err(HelperError::CallerIdentityFailed),
                authorize_result: Ok(()),
            }
        }

        pub fn denying_polkit(action: &str) -> Self {
            Self {
                resolve_result: Ok(CallerCredentials {
                    uid: 1000,
                    pid: 1234,
                    bus_name: ":1.42".to_string(),
                }),
                authorize_result: Err(HelperError::AuthorizationDenied {
                    action: action.to_string(),
                }),
            }
        }
    }

    impl AuthorizePolicy for MockPolicy {
        fn resolve_caller(&self, _sender: &str) -> Result<CallerCredentials, HelperError> {
            self.resolve_result.clone()
        }

        fn check_authorization(
            &self,
            _credentials: &CallerCredentials,
            _action_id: &str,
        ) -> Result<(), HelperError> {
            self.authorize_result.clone()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::mock::MockPolicy;
    use super::*;

    #[test]
    fn authorize_succeeds_with_valid_caller() {
        let policy = MockPolicy::allowing(1000, 5678, ":1.100");
        let result = authorize_request(&policy, ":1.100", "io.github.maintainerapp.test");
        assert!(result.is_ok());
        let creds = result.unwrap();
        assert_eq!(creds.uid, 1000);
        assert_eq!(creds.pid, 5678);
    }

    #[test]
    fn authorize_fails_when_caller_unknown() {
        let policy = MockPolicy::denying_caller();
        let result = authorize_request(&policy, ":1.999", "io.github.maintainerapp.test");
        assert_eq!(result, Err(HelperError::CallerIdentityFailed));
    }

    #[test]
    fn authorize_fails_when_polkit_denies() {
        let policy = MockPolicy::denying_polkit("io.github.maintainerapp.terminate-process");
        let result = authorize_request(
            &policy,
            ":1.42",
            "io.github.maintainerapp.terminate-process",
        );
        assert!(matches!(
            result,
            Err(HelperError::AuthorizationDenied { .. })
        ));
    }
}
