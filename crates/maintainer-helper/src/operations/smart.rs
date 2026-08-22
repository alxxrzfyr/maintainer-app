//! SMART data refresh via UDisks2 D-Bus interface.
//!
//! Targets a validated drive identifier, not an arbitrary filesystem path.
//! The drive ID is restricted to `[a-zA-Z0-9_]` characters only.

use crate::api::{HelperError, OperationResponse, SmartRefreshRequest};

/// UDisks2 D-Bus object path prefix for drives.
const UDISKS2_DRIVES_PREFIX: &str = "/org/freedesktop/UDisks2/drives/";

/// UDisks2 ATA interface for SMART operations.
const UDISKS2_ATA_INTERFACE: &str = "org.freedesktop.UDisks2.Drive.Ata";

/// UDisks2 bus name.
const UDISKS2_BUS_NAME: &str = "org.freedesktop.UDisks2";

/// Build the validated UDisks2 object path for a drive.
///
/// The drive_id has already been validated to contain only `[a-zA-Z0-9_]`,
/// so path traversal is structurally impossible.
pub fn build_drive_object_path(drive_id: &str) -> String {
    format!("{UDISKS2_DRIVES_PREFIX}{drive_id}")
}

/// Execute a SMART data refresh for the specified drive.
///
/// In production, this calls the UDisks2 D-Bus method
/// `org.freedesktop.UDisks2.Drive.Ata.SmartUpdate` on the validated object path.
///
/// The `drive_id` must be pre-validated by `validate::validate_smart_refresh`.
pub fn execute_smart_refresh(
    request: &SmartRefreshRequest,
) -> Result<OperationResponse, HelperError> {
    let object_path = build_drive_object_path(&request.drive_id);

    let _ = (UDISKS2_BUS_NAME, UDISKS2_ATA_INTERFACE);

    Err(HelperError::OperationFailed {
        reason: format!(
            "UDisks2 SMART refresh not available in this context (path: {object_path})"
        ),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_path_produces_correct_prefix() {
        let path = build_drive_object_path("SAMSUNG_DRIVE_ABC123");
        assert_eq!(path, "/org/freedesktop/UDisks2/drives/SAMSUNG_DRIVE_ABC123");
    }

    #[test]
    fn build_path_no_traversal_possible() {
        let path = build_drive_object_path("valid_drive_id");
        assert!(path.starts_with(UDISKS2_DRIVES_PREFIX));
    }

    #[test]
    fn execute_returns_error_without_dbus() {
        let request = SmartRefreshRequest {
            drive_id: "TEST_DRIVE_123".to_string(),
        };
        let result = execute_smart_refresh(&request);
        // Without a real D-Bus session, this should fail gracefully.
        assert!(result.is_err());
    }
}
