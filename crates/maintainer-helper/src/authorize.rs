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
/// Requires a live system bus connection. Fails closed on every error path:
/// network errors, vanished senders, and missing services all result in denial.
pub struct SystemPolicy {
    connection: zbus::blocking::Connection,
}

impl SystemPolicy {
    /// Create a policy backed by the given system bus connection.
    pub fn new(connection: zbus::blocking::Connection) -> Self {
        Self { connection }
    }

    /// Resolve PID and UID via org.freedesktop.DBus kernel credential methods.
    fn resolve_dbus_credentials(&self, sender: &str) -> Result<(u32, u32), HelperError> {
        let proxy = zbus::blocking::fdo::DBusProxy::new(&self.connection)
            .map_err(|_| HelperError::CallerIdentityFailed)?;

        let pid = proxy
            .get_connection_unix_process_id(
                sender
                    .try_into()
                    .map_err(|_| HelperError::CallerIdentityFailed)?,
            )
            .map_err(|_| HelperError::CallerIdentityFailed)?;

        let uid = proxy
            .get_connection_unix_user(
                sender
                    .try_into()
                    .map_err(|_| HelperError::CallerIdentityFailed)?,
            )
            .map_err(|_| HelperError::CallerIdentityFailed)?;

        Ok((pid, uid))
    }

    /// Call org.freedesktop.PolicyKit1.Authority.CheckAuthorization.
    ///
    /// Subject kind is `system-bus-name` bound to the caller's unique name,
    /// so authorization tracks the actual client connection rather than a PID
    /// that could be reused between check and action.
    fn check_polkit(
        &self,
        credentials: &CallerCredentials,
        action_id: &str,
    ) -> Result<(), HelperError> {
        use std::collections::HashMap;
        use zbus::zvariant::Value;

        let polkit_proxy = zbus::blocking::Proxy::new(
            &self.connection,
            "org.freedesktop.PolicyKit1",
            "/org/freedesktop/PolicyKit1/Authority",
            "org.freedesktop.PolicyKit1.Authority",
        )
        .map_err(|_| HelperError::AuthorizationDenied {
            action: action_id.to_string(),
        })?;

        let mut subject_details: HashMap<&str, Value<'_>> = HashMap::new();
        subject_details.insert("name", Value::from(credentials.bus_name.as_str()));

        let subject = ("system-bus-name", subject_details);

        let action_details: HashMap<&str, &str> = HashMap::new();

        // Flags: 0x1 = AllowUserInteraction — presents the desktop Polkit prompt.
        let flags: u32 = 0x1;

        let cancellation_id = "";

        let reply: Result<(bool, bool, HashMap<String, String>), zbus::Error> = polkit_proxy.call(
            "CheckAuthorization",
            &(subject, action_id, action_details, flags, cancellation_id),
        );

        match reply {
            Ok((is_authorized, _is_challenge, details)) => {
                if is_authorized {
                    Ok(())
                } else {
                    let dismissed = details
                        .get("polkit.dismissed")
                        .map(|v| v == "true")
                        .unwrap_or(false);

                    if dismissed {
                        Err(HelperError::AuthorizationDenied {
                            action: format!("{action_id} (dismissed by user)"),
                        })
                    } else {
                        Err(HelperError::AuthorizationDenied {
                            action: action_id.to_string(),
                        })
                    }
                }
            }
            Err(zbus::Error::MethodError(ref name, _, _))
                if name.as_str().contains("ServiceUnknown")
                    || name.as_str().contains("NameHasNoOwner") =>
            {
                // Polkit service is not running — unavailable, not generic failure.
                Err(HelperError::OperationFailed {
                    reason: format!("PolicyKit service unavailable; cannot authorize {action_id}"),
                })
            }
            Err(_) => Err(HelperError::AuthorizationDenied {
                action: action_id.to_string(),
            }),
        }
    }
}

impl AuthorizePolicy for SystemPolicy {
    fn resolve_caller(&self, sender: &str) -> Result<CallerCredentials, HelperError> {
        let (pid, uid) = self.resolve_dbus_credentials(sender)?;
        Ok(CallerCredentials {
            uid,
            pid,
            bus_name: sender.to_string(),
        })
    }

    fn check_authorization(
        &self,
        credentials: &CallerCredentials,
        action_id: &str,
    ) -> Result<(), HelperError> {
        self.check_polkit(credentials, action_id)
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

        /// Mock that resolves the caller but returns a Polkit-unavailable error.
        pub fn polkit_unavailable(action: &str) -> Self {
            Self {
                resolve_result: Ok(CallerCredentials {
                    uid: 1000,
                    pid: 1234,
                    bus_name: ":1.42".to_string(),
                }),
                authorize_result: Err(HelperError::OperationFailed {
                    reason: format!("PolicyKit service unavailable; cannot authorize {action}"),
                }),
            }
        }

        /// Mock that resolves the caller but returns a dismissal error.
        pub fn dismissed(action: &str) -> Self {
            Self {
                resolve_result: Ok(CallerCredentials {
                    uid: 1000,
                    pid: 1234,
                    bus_name: ":1.42".to_string(),
                }),
                authorize_result: Err(HelperError::AuthorizationDenied {
                    action: format!("{action} (dismissed by user)"),
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

    #[test]
    fn authorize_fails_when_polkit_unavailable() {
        let policy = MockPolicy::polkit_unavailable("io.github.maintainerapp.vacuum-journal");
        let result = authorize_request(&policy, ":1.42", "io.github.maintainerapp.vacuum-journal");
        assert!(matches!(result, Err(HelperError::OperationFailed { .. })));
        if let Err(HelperError::OperationFailed { reason }) = result {
            assert!(reason.contains("PolicyKit service unavailable"));
        }
    }

    #[test]
    fn authorize_fails_when_user_dismisses_prompt() {
        let policy = MockPolicy::dismissed("io.github.maintainerapp.refresh-smart");
        let result = authorize_request(&policy, ":1.42", "io.github.maintainerapp.refresh-smart");
        assert!(matches!(
            result,
            Err(HelperError::AuthorizationDenied { .. })
        ));
        if let Err(HelperError::AuthorizationDenied { action }) = result {
            assert!(action.contains("dismissed by user"));
        }
    }

    #[test]
    fn authorize_request_passes_credentials_through() {
        let policy = MockPolicy::allowing(0, 1, ":1.1");
        let creds = authorize_request(&policy, ":1.1", "io.github.maintainerapp.test").unwrap();
        assert_eq!(creds.uid, 0);
        assert_eq!(creds.pid, 1);
        assert_eq!(creds.bus_name, ":1.1");
    }

    #[test]
    fn system_policy_requires_connection() {
        let _: fn(zbus::blocking::Connection) -> SystemPolicy = SystemPolicy::new;
    }
}
