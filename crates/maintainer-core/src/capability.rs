/// Reason a capability is unavailable on this system.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnavailableReason {
    /// Machine-readable code (e.g., "sensor-missing").
    pub code: &'static str,
    /// Human-readable explanation.
    pub message: String,
    /// Optional remediation advice.
    pub remediation: Option<String>,
}

/// A request to escalate privilege for a specific action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionRequest {
    /// D-Bus/Polkit action ID.
    pub action_id: String,
    /// User-facing explanation of what the permission enables.
    pub explanation: String,
}

/// Three-state capability result: available, unavailable with reason,
/// or requires elevated permission.
#[derive(Debug, Clone)]
pub enum Capability<T> {
    /// The capability is available and carries its value.
    Available(T),
    /// The capability is not available on this system.
    Unavailable(UnavailableReason),
    /// The capability requires elevated privilege.
    PermissionRequired(PermissionRequest),
}

impl<T> Capability<T> {
    /// Returns the reason if the capability is unavailable.
    pub fn reason(&self) -> Option<&UnavailableReason> {
        match self {
            Self::Unavailable(r) => Some(r),
            _ => None,
        }
    }

    /// Returns the permission request if privilege is required.
    pub fn permission(&self) -> Option<&PermissionRequest> {
        match self {
            Self::PermissionRequired(p) => Some(p),
            _ => None,
        }
    }

    /// Returns `true` if the capability is available.
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available(_))
    }

    /// Returns the inner value if available, `None` otherwise.
    pub fn value(self) -> Option<T> {
        match self {
            Self::Available(v) => Some(v),
            _ => None,
        }
    }

    /// Returns a reference to the inner value if available.
    pub fn as_value(&self) -> Option<&T> {
        match self {
            Self::Available(v) => Some(v),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unavailable_preserves_reason_and_remediation() {
        let value = Capability::<()>::Unavailable(UnavailableReason {
            code: "sensor-missing",
            message: "No readable temperature sensor was found".into(),
            remediation: Some("Install a supported hwmon driver".into()),
        });
        assert_eq!(value.reason().unwrap().code, "sensor-missing");
        assert_eq!(
            value.reason().unwrap().remediation.as_deref(),
            Some("Install a supported hwmon driver")
        );
    }

    #[test]
    fn available_provides_value() {
        let cap = Capability::Available(42u32);
        assert!(cap.is_available());
        assert_eq!(cap.value(), Some(42));
    }

    #[test]
    fn permission_required_exposes_action_id() {
        let cap = Capability::<()>::PermissionRequired(PermissionRequest {
            action_id: "com.example.install-update".into(),
            explanation: "Install system updates".into(),
        });
        let perm = cap.permission().unwrap();
        assert_eq!(perm.action_id, "com.example.install-update");
        assert_eq!(perm.explanation, "Install system updates");
    }

    #[test]
    fn unavailable_has_no_value() {
        let cap = Capability::<String>::Unavailable(UnavailableReason {
            code: "missing",
            message: "not present".into(),
            remediation: None,
        });
        assert!(!cap.is_available());
        assert!(cap.as_value().is_none());
    }
}
