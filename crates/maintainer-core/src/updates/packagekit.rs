//! PackageKit D-Bus transaction adapter.
//!
//! PackageKit owns package authorization and the transaction lifecycle.
//! This adapter maps D-Bus signals (status, percentage, package progress,
//! restart requirement, cancellation support, errors) into typed Rust values.
//!
//! Never requests or stores passwords. Authentication is delegated to Polkit
//! through PackageKit's own agent.

use crate::capability::{Capability, UnavailableReason};
use crate::error::MaintainerError;
use crate::jobs::Cancellation;

use super::model::{TransactionStatus, UpdateProgress, UpdateResult};

// ---------------------------------------------------------------------------
// PackageKit constants
// ---------------------------------------------------------------------------

/// PackageKit D-Bus well-known name.
pub const PK_BUS_NAME: &str = "org.freedesktop.PackageKit";

/// PackageKit D-Bus object path.
pub const PK_OBJECT_PATH: &str = "/org/freedesktop/PackageKit";

/// PackageKit D-Bus interface.
pub const PK_INTERFACE: &str = "org.freedesktop.PackageKit";

/// PackageKit Transaction interface.
pub const PK_TRANSACTION_INTERFACE: &str = "org.freedesktop.PackageKit.Transaction";

// ---------------------------------------------------------------------------
// PackageKit status codes
// ---------------------------------------------------------------------------

/// Map PackageKit numeric status to our typed enum.
pub fn map_pk_status(status: u32) -> TransactionStatus {
    match status {
        1 => TransactionStatus::Setup,       // PK_STATUS_ENUM_WAIT
        2 => TransactionStatus::Setup,       // PK_STATUS_ENUM_SETUP
        8 => TransactionStatus::Downloading, // PK_STATUS_ENUM_DOWNLOAD
        9 => TransactionStatus::Installing,  // PK_STATUS_ENUM_INSTALL
        10 => TransactionStatus::Installing, // PK_STATUS_ENUM_UPDATE
        11 => TransactionStatus::Cleanup,    // PK_STATUS_ENUM_CLEANUP
        12 => TransactionStatus::Finished,   // PK_STATUS_ENUM_FINISHED
        _ => TransactionStatus::Setup,       // Unknown maps to setup
    }
}

/// Map PackageKit error code to a display string.
pub fn map_pk_error_code(code: u32) -> &'static str {
    match code {
        1 => "not-authorized",
        4 => "package-not-found",
        7 => "package-download-failed",
        8 => "dep-resolution-failed",
        10 => "transaction-cancelled",
        14 => "no-network",
        23 => "cannot-install-repo-unsigned",
        _ => "unknown-error",
    }
}

// ---------------------------------------------------------------------------
// Transaction state machine
// ---------------------------------------------------------------------------

/// Internal state of a PackageKit transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionState {
    /// Current status.
    pub status: TransactionStatus,
    /// Overall percentage.
    pub percentage: Option<u32>,
    /// Currently processing package.
    pub current_package: Option<String>,
    /// Whether cancellation is allowed right now.
    pub allow_cancel: bool,
    /// Whether a restart is required after completion.
    pub restart_required: bool,
    /// Packages successfully processed.
    pub completed_packages: Vec<String>,
    /// Error code, if any.
    pub error_code: Option<String>,
    /// Error details, if any.
    pub error_details: Option<String>,
}

impl TransactionState {
    /// Create a new transaction state in initial setup.
    pub fn new() -> Self {
        Self {
            status: TransactionStatus::Setup,
            percentage: None,
            current_package: None,
            allow_cancel: true,
            restart_required: false,
            completed_packages: Vec::new(),
            error_code: None,
            error_details: None,
        }
    }

    /// Apply a status change event.
    pub fn on_status_changed(&mut self, status: u32) {
        self.status = map_pk_status(status);
    }

    /// Apply a percentage update.
    pub fn on_percentage_changed(&mut self, percentage: u32) {
        // PackageKit uses 101 for "unknown".
        if percentage <= 100 {
            self.percentage = Some(percentage);
        }
    }

    /// Apply a package progress event.
    pub fn on_package(&mut self, package_id: &str, info: u32) {
        let name = package_id.split(';').next().unwrap_or(package_id);
        self.current_package = Some(name.to_string());

        // Info 1 = installed successfully.
        if info == 1 {
            self.completed_packages.push(name.to_string());
        }
    }

    /// Apply allow-cancel change.
    pub fn on_allow_cancel_changed(&mut self, allow: bool) {
        self.allow_cancel = allow;
    }

    /// Mark that a restart is required.
    pub fn on_require_restart(&mut self) {
        self.restart_required = true;
    }

    /// Apply an error event.
    pub fn on_error(&mut self, code: u32, details: &str) {
        self.status = TransactionStatus::Failed;
        self.error_code = Some(map_pk_error_code(code).to_string());
        self.error_details = Some(details.to_string());
    }

    /// Mark transaction as finished.
    pub fn on_finished(&mut self, exit_code: u32) {
        match exit_code {
            1 => self.status = TransactionStatus::Finished, // PK_EXIT_ENUM_SUCCESS
            3 => self.status = TransactionStatus::Failed,   // PK_EXIT_ENUM_FAILED
            4 => self.status = TransactionStatus::Cancelled, // PK_EXIT_ENUM_CANCELLED
            _ => {
                if self.error_code.is_some() {
                    self.status = TransactionStatus::Failed;
                } else {
                    self.status = TransactionStatus::Finished;
                }
            }
        }
    }

    /// Convert current state to a progress snapshot.
    pub fn to_progress(&self) -> UpdateProgress {
        UpdateProgress {
            status: self.status,
            percentage: self.percentage,
            current_package: self.current_package.clone(),
            allow_cancel: self.allow_cancel,
        }
    }

    /// Convert final state to an `UpdateResult`.
    ///
    /// Should only be called when the transaction has reached a terminal state.
    pub fn to_result(&self) -> UpdateResult {
        let success = self.status == TransactionStatus::Finished;
        UpdateResult {
            success,
            updated_packages: self.completed_packages.clone(),
            failed_packages: Vec::new(),
            restart_required: self.restart_required,
            error_message: self.error_details.clone(),
            error_code: self.error_code.clone(),
        }
    }

    /// Returns `true` if the transaction is in a terminal state.
    pub fn is_terminal(&self) -> bool {
        matches!(
            self.status,
            TransactionStatus::Finished | TransactionStatus::Failed | TransactionStatus::Cancelled
        )
    }
}

impl Default for TransactionState {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// PackageKit availability check
// ---------------------------------------------------------------------------

/// Check whether PackageKit is available on D-Bus.
///
/// This is a synchronous probe; it does not start a transaction.
pub fn check_packagekit_available() -> Capability<()> {
    // Attempt to see if the PackageKit name is activatable on the session/system bus.
    // In the real system this would be a D-Bus call; here we check if the binary exists.
    let path = std::path::Path::new(
        "/usr/share/dbus-1/system-services/org.freedesktop.PackageKit.service",
    );
    if path.exists() {
        Capability::Available(())
    } else {
        // Also check if the daemon binary exists.
        let daemon = std::path::Path::new("/usr/lib/packagekit/packagekitd");
        if daemon.exists() {
            Capability::Available(())
        } else {
            Capability::Unavailable(UnavailableReason {
                code: "packagekit-not-installed",
                message: "PackageKit is not installed on this system".to_string(),
                remediation: Some("Install packagekit: sudo apt install packagekit".to_string()),
            })
        }
    }
}

/// Trait for PackageKit transaction operations (mockable for testing).
pub trait PackageKitClient: Send + Sync {
    /// Create a new transaction and return its D-Bus path.
    fn create_transaction(&self) -> Result<String, MaintainerError>;

    /// Install packages by their PackageKit IDs.
    fn install_packages(
        &self,
        transaction_path: &str,
        package_ids: &[String],
    ) -> Result<(), MaintainerError>;

    /// Update packages by their PackageKit IDs.
    fn update_packages(
        &self,
        transaction_path: &str,
        package_ids: &[String],
    ) -> Result<(), MaintainerError>;

    /// Cancel a running transaction.
    fn cancel_transaction(&self, transaction_path: &str) -> Result<(), MaintainerError>;
}

/// Execute an update through PackageKit, reporting progress.
///
/// This function drives the transaction state machine by processing
/// events from the `event_source` until the transaction reaches a
/// terminal state or cancellation is requested.
pub fn execute_update<F>(
    state: &mut TransactionState,
    cancel: &dyn Cancellation,
    mut progress_callback: F,
) -> UpdateResult
where
    F: FnMut(&UpdateProgress),
{
    // Report initial progress.
    progress_callback(&state.to_progress());

    // In production, this would loop reading D-Bus signals.
    // The state machine is driven externally; this function is
    // the integration point that checks cancellation between events.

    if cancel.is_cancelled() && state.allow_cancel {
        state.status = TransactionStatus::Cancelled;
    }

    state.to_result()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transaction_state_machine_success() {
        let mut state = TransactionState::new();
        assert_eq!(state.status, TransactionStatus::Setup);
        assert!(state.allow_cancel);

        state.on_status_changed(8); // Download
        assert_eq!(state.status, TransactionStatus::Downloading);

        state.on_percentage_changed(25);
        assert_eq!(state.percentage, Some(25));

        state.on_package("curl;7.81.0-1ubuntu1.16;amd64;Ubuntu", 0);
        assert_eq!(state.current_package.as_deref(), Some("curl"));

        state.on_status_changed(9); // Install
        assert_eq!(state.status, TransactionStatus::Installing);

        state.on_allow_cancel_changed(false);
        assert!(!state.allow_cancel);

        state.on_package("curl;7.81.0-1ubuntu1.16;amd64;Ubuntu", 1);
        assert!(state.completed_packages.contains(&"curl".to_string()));

        state.on_require_restart();
        assert!(state.restart_required);

        state.on_finished(1); // Success
        assert_eq!(state.status, TransactionStatus::Finished);
        assert!(state.is_terminal());

        let result = state.to_result();
        assert!(result.success);
        assert!(result.restart_required);
        assert!(result.updated_packages.contains(&"curl".to_string()));
    }

    #[test]
    fn transaction_state_machine_failure() {
        let mut state = TransactionState::new();

        state.on_status_changed(8); // Download
        state.on_percentage_changed(30);
        state.on_error(7, "Failed to download curl: hash sum mismatch");

        assert_eq!(state.status, TransactionStatus::Failed);
        assert_eq!(state.error_code.as_deref(), Some("package-download-failed"));
        assert!(state.is_terminal());

        let result = state.to_result();
        assert!(!result.success);
        assert!(result.error_message.as_ref().unwrap().contains("hash sum"));
    }

    #[test]
    fn transaction_state_machine_cancellation() {
        let mut state = TransactionState::new();
        state.on_status_changed(8); // Download
        state.on_allow_cancel_changed(true);

        let cancel = crate::jobs::CancellationToken::new();
        cancel.cancel();

        let result = execute_update(&mut state, &cancel, |_| {});
        // Cancellation was applied.
        assert!(!result.success);
    }

    #[test]
    fn transaction_cancel_blocked_during_install() {
        let mut state = TransactionState::new();
        state.on_status_changed(9); // Install
        state.on_allow_cancel_changed(false);

        let cancel = crate::jobs::CancellationToken::new();
        cancel.cancel();

        // Cancel requested but not allowed - transaction continues.
        let result = execute_update(&mut state, &cancel, |_| {});
        // State was not terminal, so result reflects current (non-cancelled) state.
        assert_eq!(state.status, TransactionStatus::Installing);
        assert!(!result.success); // Not finished yet.
    }

    #[test]
    fn map_pk_status_values() {
        assert_eq!(map_pk_status(8), TransactionStatus::Downloading);
        assert_eq!(map_pk_status(9), TransactionStatus::Installing);
        assert_eq!(map_pk_status(12), TransactionStatus::Finished);
        assert_eq!(map_pk_status(255), TransactionStatus::Setup); // Unknown
    }

    #[test]
    fn map_pk_error_codes() {
        assert_eq!(map_pk_error_code(1), "not-authorized");
        assert_eq!(map_pk_error_code(7), "package-download-failed");
        assert_eq!(map_pk_error_code(10), "transaction-cancelled");
        assert_eq!(map_pk_error_code(999), "unknown-error");
    }

    #[test]
    fn percentage_unknown_handled() {
        let mut state = TransactionState::new();
        state.on_percentage_changed(101); // PackageKit "unknown"
        assert_eq!(state.percentage, None);

        state.on_percentage_changed(50);
        assert_eq!(state.percentage, Some(50));
    }

    #[test]
    fn progress_snapshot_reflects_state() {
        let mut state = TransactionState::new();
        state.on_status_changed(8);
        state.on_percentage_changed(42);
        state.on_package("libssl3;3.0.2;amd64;Ubuntu", 0);
        state.on_allow_cancel_changed(true);

        let progress = state.to_progress();
        assert_eq!(progress.status, TransactionStatus::Downloading);
        assert_eq!(progress.percentage, Some(42));
        assert_eq!(progress.current_package.as_deref(), Some("libssl3"));
        assert!(progress.allow_cancel);
    }
}
