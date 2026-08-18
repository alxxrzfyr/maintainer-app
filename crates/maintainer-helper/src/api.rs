//! Fixed D-Bus method definitions and typed DTOs for the privileged helper.
//!
//! No method accepts a program name, argument vector, shell text, arbitrary
//! environment, arbitrary filesystem root, or arbitrary signal.

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Result type for all helper operations.
pub type HelperResult = Result<OperationResponse, HelperError>;

/// Errors returned by the privileged helper over D-Bus.
#[derive(Debug, Error, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum HelperError {
    #[error("authorization denied: {action}")]
    AuthorizationDenied { action: String },

    #[error("caller identity could not be verified")]
    CallerIdentityFailed,

    #[error("validation failed: {reason}")]
    ValidationFailed { reason: String },

    #[error("operation failed: {reason}")]
    OperationFailed { reason: String },

    #[error("helper is busy: concurrent request limit reached")]
    Busy,

    #[error("pid reuse detected: process start time changed")]
    PidReuse,
}

/// Successful operation response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OperationResponse {
    pub success: bool,
    pub message: String,
}

/// Request to terminate a process by PID with start-time revalidation.
///
/// The signal is always SIGTERM. No signal field is exposed.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TerminateProcessRequest {
    /// Process ID to terminate.
    pub pid: u32,
    /// Process start time (clock ticks since boot, field 22 of /proc/<pid>/stat).
    /// Used to prevent PID reuse races.
    pub start_time_ticks: u64,
}

/// Request to clean the APT package cache.
///
/// Uses a fixed `apt-get clean` invocation. No paths or arguments from the caller.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PackageCacheRequest {
    /// Request identifier for tracking.
    pub request_id: String,
}

/// Request to vacuum the systemd journal.
///
/// Size and age are bounded by validation policy.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct JournalVacuumRequest {
    /// Maximum journal size in mebibytes. Bounds: 100..=4096.
    pub max_size_mib: u32,
    /// Maximum journal age in days. Bounds: 1..=365.
    pub max_age_days: u32,
}

/// Request to refresh SMART data for a drive via UDisks2.
///
/// The device identity is a UDisks2 drive identifier (alphanumeric + underscores),
/// not an arbitrary filesystem path.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SmartRefreshRequest {
    /// UDisks2 drive identifier (e.g., "SAMSUNG_MZVL21T0HCLR_0BL2_S64FNX0T506050").
    /// Validated against `[a-zA-Z0-9_]` pattern only.
    pub drive_id: String,
}

/// Polkit action identifiers for each operation.
pub mod actions {
    pub const TERMINATE_PROCESS: &str = "io.github.maintainerapp.terminate-process";
    pub const CLEAN_PACKAGE_CACHE: &str = "io.github.maintainerapp.clean-package-cache";
    pub const VACUUM_JOURNAL: &str = "io.github.maintainerapp.vacuum-journal";
    pub const REFRESH_SMART: &str = "io.github.maintainerapp.refresh-smart";
}

/// The fixed helper API trait. Each method maps to one D-Bus method and one Polkit action.
pub trait HelperApi {
    fn terminate_process(&self, request: TerminateProcessRequest) -> HelperResult;
    fn clean_package_cache(&self, request: PackageCacheRequest) -> HelperResult;
    fn vacuum_journal(&self, request: JournalVacuumRequest) -> HelperResult;
    fn refresh_smart(&self, request: SmartRefreshRequest) -> HelperResult;
}

/// Maximum number of concurrent in-flight requests.
pub const MAX_CONCURRENT_REQUESTS: usize = 4;

/// Idle timeout in seconds before the helper exits.
pub const IDLE_TIMEOUT_SECS: u64 = 60;
