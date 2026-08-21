//! UDisks2 D-Bus adapter for SMART storage health.
//!
//! Queries UDisks2 over D-Bus for drive SMART data.
//! Uses a trait abstraction for testability — production uses the real
//! D-Bus connection, tests use a mock implementation.

use std::time::{Duration, UNIX_EPOCH};

use crate::capability::{Capability, PermissionRequest, UnavailableReason};
use crate::diagnostics::smart::{
    SmartAttribute, SmartDrive, TemperatureCelsius, classify_assessment,
};

/// Raw drive properties as read from UDisks2.
/// This is the boundary type between D-Bus data and our domain model.
#[derive(Debug, Clone)]
pub struct UDisksDriveProperties {
    /// D-Bus object path.
    pub object_path: String,
    /// Drive model.
    pub model: String,
    /// Firmware revision.
    pub firmware: String,
    /// Serial number (internal only).
    pub serial: String,
    /// Whether the drive is removable.
    pub removable: bool,
    /// Whether SMART is supported.
    pub smart_supported: bool,
    /// Whether SMART is enabled.
    pub smart_enabled: bool,
    /// Overall SMART assessment (true = passed, None = unavailable).
    pub smart_overall_passed: Option<bool>,
    /// Temperature in Kelvin, if available.
    pub temperature_kelvin: Option<f64>,
    /// Power-on hours.
    pub power_on_hours: Option<u64>,
    /// Unix timestamp of last SMART update.
    pub smart_updated_unix: Option<u64>,
    /// SMART attributes as raw tuples: (id, name, value, worst, threshold, raw, pre_fail).
    pub attributes: Vec<RawSmartAttribute>,
}

/// Raw attribute data from UDisks2.
#[derive(Debug, Clone)]
pub struct RawSmartAttribute {
    pub id: u8,
    pub name: String,
    pub value: u64,
    pub worst: u64,
    pub threshold: u64,
    pub raw: u64,
    pub pre_fail: bool,
}

/// Trait abstraction over UDisks2 D-Bus access for testability.
pub trait UDisksClient: Send + Sync {
    /// Discover all drive object paths and their SMART properties.
    /// Returns an error string if D-Bus is unreachable.
    fn get_drives(&self) -> Result<Vec<UDisksDriveProperties>, UDisksError>;

    /// Request a SMART data refresh for a specific drive.
    /// May require elevated privileges.
    fn refresh_smart(&self, object_path: &str) -> Result<(), UDisksError>;
}

/// Errors from the UDisks2 adapter.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UDisksError {
    /// D-Bus connection failed.
    ConnectionFailed(String),
    /// UDisks2 service not available.
    ServiceUnavailable(String),
    /// Permission denied for the requested operation.
    PermissionDenied(String),
    /// General D-Bus method call error.
    MethodCallFailed(String),
}

impl std::fmt::Display for UDisksError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ConnectionFailed(msg) => write!(f, "D-Bus connection failed: {msg}"),
            Self::ServiceUnavailable(msg) => write!(f, "UDisks2 service unavailable: {msg}"),
            Self::PermissionDenied(msg) => write!(f, "Permission denied: {msg}"),
            Self::MethodCallFailed(msg) => write!(f, "D-Bus method call failed: {msg}"),
        }
    }
}

impl std::error::Error for UDisksError {}

/// Critical ATA SMART attribute IDs that we extract.
/// These are the attributes most correlated with drive failure.
const CRITICAL_ATTRIBUTE_IDS: &[u8] = &[
    5,   // Reallocated Sectors Count
    10,  // Spin Retry Count
    184, // End-to-End Error
    187, // Reported Uncorrectable Errors
    188, // Command Timeout
    196, // Reallocation Event Count
    197, // Current Pending Sector Count
    198, // Offline Uncorrectable
    201, // Soft Read Error Rate
];

/// Map raw UDisks2 drive properties to our domain model.
pub fn map_drive(props: &UDisksDriveProperties) -> SmartDrive {
    let attributes: Vec<SmartAttribute> = props
        .attributes
        .iter()
        .filter(|a| CRITICAL_ATTRIBUTE_IDS.contains(&a.id))
        .map(|a| SmartAttribute {
            id: a.id,
            name: a.name.clone(),
            value: a.value,
            worst: a.worst,
            threshold: a.threshold,
            raw: a.raw,
            pre_fail: a.pre_fail,
        })
        .collect();

    let sample_time = props
        .smart_updated_unix
        .and_then(|ts| UNIX_EPOCH.checked_add(Duration::from_secs(ts)));

    let assessment = classify_assessment(
        props.smart_supported,
        props.smart_enabled,
        props.smart_overall_passed,
        &attributes,
        sample_time,
    );

    let temperature_celsius = props
        .temperature_kelvin
        .map(TemperatureCelsius::from_kelvin);

    SmartDrive {
        object_path: props.object_path.clone(),
        model: props.model.clone(),
        firmware: props.firmware.clone(),
        serial: props.serial.clone(),
        removable: props.removable,
        assessment,
        temperature_celsius,
        power_on_hours: props.power_on_hours,
        attributes,
        sample_time,
        source: "UDisks2",
    }
}

/// Query all drives and return SMART health as a capability result.
pub fn query_smart_drives(client: &dyn UDisksClient) -> Capability<Vec<SmartDrive>> {
    match client.get_drives() {
        Ok(raw_drives) => {
            let drives: Vec<SmartDrive> = raw_drives.iter().map(map_drive).collect();
            Capability::Available(drives)
        }
        Err(UDisksError::ServiceUnavailable(msg)) => Capability::Unavailable(UnavailableReason {
            code: "udisks2-unavailable",
            message: msg,
            remediation: Some("Install and start the udisks2 service".into()),
        }),
        Err(UDisksError::PermissionDenied(msg)) => {
            Capability::PermissionRequired(PermissionRequest {
                action_id: "org.freedesktop.udisks2.ata-smart-update".into(),
                explanation: msg,
            })
        }
        Err(UDisksError::ConnectionFailed(msg)) => Capability::Unavailable(UnavailableReason {
            code: "dbus-connection-failed",
            message: msg,
            remediation: Some("Ensure the system D-Bus daemon is running".into()),
        }),
        Err(UDisksError::MethodCallFailed(msg)) => Capability::Unavailable(UnavailableReason {
            code: "udisks2-method-failed",
            message: msg,
            remediation: None,
        }),
    }
}

/// Request a SMART refresh for a specific drive, returning the capability state.
pub fn refresh_drive_smart(
    client: &dyn UDisksClient,
    object_path: &str,
) -> Result<(), UDisksError> {
    client.refresh_smart(object_path)
}

/// Real UDisks2 D-Bus client (placeholder for async zbus integration).
/// In production this uses zbus to connect to the system bus.
pub struct RealUDisksClient {
    _private: (),
}

impl RealUDisksClient {
    /// Create a new client. Connection is established lazily.
    pub fn new() -> Self {
        Self { _private: () }
    }
}

impl Default for RealUDisksClient {
    fn default() -> Self {
        Self::new()
    }
}

impl UDisksClient for RealUDisksClient {
    fn get_drives(&self) -> Result<Vec<UDisksDriveProperties>, UDisksError> {
        // In production, this would:
        // 1. Connect to system bus via zbus
        // 2. Enumerate /org/freedesktop/UDisks2/drives/*
        // 3. Read org.freedesktop.UDisks2.Drive properties
        // 4. Read org.freedesktop.UDisks2.Drive.Ata properties for SMART
        //
        // For now, return ServiceUnavailable until the async integration
        // is wired up in a later task.
        Err(UDisksError::ServiceUnavailable(
            "UDisks2 D-Bus integration pending async runtime setup".into(),
        ))
    }

    fn refresh_smart(&self, _object_path: &str) -> Result<(), UDisksError> {
        Err(UDisksError::ServiceUnavailable(
            "UDisks2 D-Bus integration pending async runtime setup".into(),
        ))
    }
}

/// Produce a human-readable unavailability reason for a removable drive.
pub fn removable_drive_note() -> &'static str {
    "SMART data is typically unavailable for removable drives"
}

/// Filter out removable drives from a SMART report if desired.
pub fn non_removable_drives(drives: &[SmartDrive]) -> Vec<&SmartDrive> {
    drives.iter().filter(|d| !d.removable).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::smart::SmartAssessment;
    use std::time::SystemTime;

    /// Mock UDisks2 client for testing.
    struct MockUDisksClient {
        drives: Result<Vec<UDisksDriveProperties>, UDisksError>,
        refresh_result: Result<(), UDisksError>,
    }

    impl MockUDisksClient {
        fn with_drives(drives: Vec<UDisksDriveProperties>) -> Self {
            Self {
                drives: Ok(drives),
                refresh_result: Ok(()),
            }
        }

        fn with_error(error: UDisksError) -> Self {
            Self {
                drives: Err(error),
                refresh_result: Ok(()),
            }
        }

        fn with_refresh_error(mut self, error: UDisksError) -> Self {
            self.refresh_result = Err(error);
            self
        }
    }

    impl UDisksClient for MockUDisksClient {
        fn get_drives(&self) -> Result<Vec<UDisksDriveProperties>, UDisksError> {
            self.drives.clone()
        }

        fn refresh_smart(&self, _object_path: &str) -> Result<(), UDisksError> {
            self.refresh_result.clone()
        }
    }

    fn make_ata_drive() -> UDisksDriveProperties {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        UDisksDriveProperties {
            object_path: "/org/freedesktop/UDisks2/drives/WDC_WD10EZEX".into(),
            model: "WDC WD10EZEX-00BN5A0".into(),
            firmware: "01.01A01".into(),
            serial: "WD-WMC4N0123456".into(),
            removable: false,
            smart_supported: true,
            smart_enabled: true,
            smart_overall_passed: Some(true),
            temperature_kelvin: Some(313.15), // 40°C
            power_on_hours: Some(12345),
            smart_updated_unix: Some(now_unix),
            attributes: vec![
                RawSmartAttribute {
                    id: 5,
                    name: "Reallocated_Sector_Ct".into(),
                    value: 100,
                    worst: 100,
                    threshold: 36,
                    raw: 0,
                    pre_fail: true,
                },
                RawSmartAttribute {
                    id: 197,
                    name: "Current_Pending_Sector".into(),
                    value: 100,
                    worst: 100,
                    threshold: 0,
                    raw: 0,
                    pre_fail: true,
                },
                RawSmartAttribute {
                    id: 9,
                    name: "Power_On_Hours".into(),
                    value: 80,
                    worst: 80,
                    threshold: 0,
                    raw: 12345,
                    pre_fail: false, // old-age, not pre-fail
                },
            ],
        }
    }

    fn make_nvme_drive() -> UDisksDriveProperties {
        let now_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        UDisksDriveProperties {
            object_path: "/org/freedesktop/UDisks2/drives/Samsung_SSD_970".into(),
            model: "Samsung SSD 970 EVO Plus 1TB".into(),
            firmware: "2B2QEXM7".into(),
            serial: "S4EWNX0M123456".into(),
            removable: false,
            smart_supported: true,
            smart_enabled: true,
            smart_overall_passed: Some(true),
            temperature_kelvin: Some(308.15), // 35°C
            power_on_hours: Some(5678),
            smart_updated_unix: Some(now_unix),
            // NVMe drives report fewer traditional ATA attributes
            attributes: vec![],
        }
    }

    fn make_removable_drive() -> UDisksDriveProperties {
        UDisksDriveProperties {
            object_path: "/org/freedesktop/UDisks2/drives/USB_Flash".into(),
            model: "USB Flash Drive".into(),
            firmware: "1.00".into(),
            serial: "USB123".into(),
            removable: true,
            smart_supported: false,
            smart_enabled: false,
            smart_overall_passed: None,
            temperature_kelvin: None,
            power_on_hours: None,
            smart_updated_unix: None,
            attributes: vec![],
        }
    }

    #[test]
    fn map_ata_drive_healthy() {
        let props = make_ata_drive();
        let drive = map_drive(&props);

        assert_eq!(drive.model, "WDC WD10EZEX-00BN5A0");
        assert_eq!(drive.firmware, "01.01A01");
        assert_eq!(drive.serial, "WD-WMC4N0123456");
        assert!(!drive.removable);
        assert_eq!(drive.assessment, SmartAssessment::Passed);
        assert_eq!(drive.source, "UDisks2");
        assert_eq!(drive.power_on_hours, Some(12345));

        // Temperature conversion from Kelvin
        let temp = drive.temperature_celsius.unwrap();
        assert!((temp.value() - 40.0).abs() < 0.01);

        // Only critical attributes should be mapped (id 5 and 197, not 9)
        assert_eq!(drive.attributes.len(), 2);
        assert!(drive.attributes.iter().any(|a| a.id == 5));
        assert!(drive.attributes.iter().any(|a| a.id == 197));
    }

    #[test]
    fn map_nvme_drive_healthy() {
        let props = make_nvme_drive();
        let drive = map_drive(&props);

        assert_eq!(drive.model, "Samsung SSD 970 EVO Plus 1TB");
        assert_eq!(drive.assessment, SmartAssessment::Passed);
        assert!(drive.attributes.is_empty());

        let temp = drive.temperature_celsius.unwrap();
        assert!((temp.value() - 35.0).abs() < 0.01);
    }

    #[test]
    fn map_drive_with_failing_assessment() {
        let mut props = make_ata_drive();
        props.smart_overall_passed = Some(false);
        let drive = map_drive(&props);

        assert_eq!(drive.assessment, SmartAssessment::FailureReportedByDrive);
    }

    #[test]
    fn map_drive_with_critical_attribute_failing() {
        let mut props = make_ata_drive();
        // Make attribute 5 (Reallocated Sectors) fail
        props.attributes[0].value = 10;
        props.attributes[0].worst = 10;
        let drive = map_drive(&props);

        assert_eq!(drive.assessment, SmartAssessment::WarningReportedByDrive);
    }

    #[test]
    fn map_drive_stale_data() {
        let mut props = make_ata_drive();
        // Set sample time to 10 days ago
        let old_time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs()
            - (10 * 24 * 3600);
        props.smart_updated_unix = Some(old_time);
        let drive = map_drive(&props);

        assert_eq!(drive.assessment, SmartAssessment::DataStale);
    }

    #[test]
    fn map_removable_drive_unavailable() {
        let props = make_removable_drive();
        let drive = map_drive(&props);

        assert!(drive.removable);
        assert_eq!(drive.assessment, SmartAssessment::Unavailable);
        assert!(drive.temperature_celsius.is_none());
        assert!(drive.power_on_hours.is_none());
    }

    #[test]
    fn kelvin_conversion_in_mapping() {
        let mut props = make_ata_drive();
        props.temperature_kelvin = Some(273.15); // 0°C
        let drive = map_drive(&props);
        assert!((drive.temperature_celsius.unwrap().value()).abs() < 0.01);

        props.temperature_kelvin = Some(373.15); // 100°C
        let drive = map_drive(&props);
        assert!((drive.temperature_celsius.unwrap().value() - 100.0).abs() < 0.01);
    }

    #[test]
    fn query_returns_available_with_drives() {
        let client = MockUDisksClient::with_drives(vec![make_ata_drive(), make_nvme_drive()]);
        let result = query_smart_drives(&client);

        assert!(result.is_available());
        let drives = result.value().unwrap();
        assert_eq!(drives.len(), 2);
    }

    #[test]
    fn query_returns_unavailable_when_service_missing() {
        let client = MockUDisksClient::with_error(UDisksError::ServiceUnavailable(
            "UDisks2 not installed".into(),
        ));
        let result = query_smart_drives(&client);

        assert!(!result.is_available());
        let reason = result.reason().unwrap();
        assert_eq!(reason.code, "udisks2-unavailable");
        assert!(reason.remediation.is_some());
    }

    #[test]
    fn query_returns_permission_required() {
        let client = MockUDisksClient::with_error(UDisksError::PermissionDenied(
            "Need elevated privileges to read SMART data".into(),
        ));
        let result = query_smart_drives(&client);

        let perm = result.permission().unwrap();
        assert_eq!(perm.action_id, "org.freedesktop.udisks2.ata-smart-update");
    }

    #[test]
    fn refresh_returns_permission_error() {
        let client = MockUDisksClient::with_drives(vec![])
            .with_refresh_error(UDisksError::PermissionDenied("Not authorized".into()));

        let result = refresh_drive_smart(&client, "/org/freedesktop/UDisks2/drives/WDC");
        assert_eq!(
            result,
            Err(UDisksError::PermissionDenied("Not authorized".into()))
        );
    }

    #[test]
    fn refresh_succeeds() {
        let client = MockUDisksClient::with_drives(vec![]);
        let result = refresh_drive_smart(&client, "/org/freedesktop/UDisks2/drives/WDC");
        assert_eq!(result, Ok(()));
    }

    #[test]
    fn non_removable_drives_filters_correctly() {
        let client = MockUDisksClient::with_drives(vec![make_ata_drive(), make_removable_drive()]);
        let all_drives = client.get_drives().unwrap();
        let mapped: Vec<SmartDrive> = all_drives.iter().map(map_drive).collect();

        let filtered = non_removable_drives(&mapped);
        assert_eq!(filtered.len(), 1);
        assert!(!filtered[0].removable);
    }

    #[test]
    fn connection_failure_returns_unavailable() {
        let client = MockUDisksClient::with_error(UDisksError::ConnectionFailed(
            "Could not connect to system bus".into(),
        ));
        let result = query_smart_drives(&client);

        let reason = result.reason().unwrap();
        assert_eq!(reason.code, "dbus-connection-failed");
    }
}
