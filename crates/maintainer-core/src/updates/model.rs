//! Update system data model.
//!
//! Defines typed candidates, classification, review plans, and execution results
//! for the updates subsystem. An `UpdatePlan` is immutable once created; any
//! change to the package set invalidates the plan.

use std::time::SystemTime;

/// Classification of an update's source and urgency.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateClassification {
    /// Confirmed security update (backend evidence present).
    Security,
    /// Standard release update from the distribution.
    Standard,
    /// Classification unknown; backend did not provide evidence.
    Unknown,
}

/// The backend that will perform the update.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum UpdateBackend {
    /// Debian/Ubuntu APT + PackageKit.
    Apt,
    /// Flatpak runtime/application updates.
    Flatpak,
    /// Firmware updates via fwupd.
    Fwupd,
}

/// Whether a package is held back or phased.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum HoldState {
    /// Package will be upgraded normally.
    None,
    /// Package is held by dpkg or apt preferences.
    Held,
    /// Package is in phased rollout and not yet offered to this system.
    Phased,
}

/// A single package that has an update available.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCandidate {
    /// Architecture confirmed by the backend; absent when unresolved.
    pub architecture: Option<String>,
    /// Package name.
    pub name: String,
    /// Currently installed version, if any.
    pub installed_version: Option<String>,
    /// Available version.
    pub available_version: String,
    /// Security classification, derived from repository origin.
    pub classification: UpdateClassification,
    /// Download size in bytes, if known.
    pub download_bytes: Option<u64>,
    /// Hold/phased state.
    pub hold_state: HoldState,
    /// Backend that owns this update.
    pub backend: UpdateBackend,
    /// Whether this package is being newly installed (dependency).
    pub is_new: bool,
    /// Whether this is a kernel package (linux-image-*, linux-headers-*, etc.).
    ///
    /// This is a separate boolean flag rather than an `UpdateClassification` variant
    /// because a kernel update can simultaneously be a security update. The kernel
    /// nature of a package and its security classification are orthogonal dimensions
    /// that are not mutually exclusive.
    pub is_kernel: bool,
}

impl UpdateCandidate {
    /// Stable selection identity, including a confirmed architecture.
    pub fn selection_key(&self) -> String {
        if self.name.contains(':') {
            self.name.clone()
        } else if let Some(architecture) = &self.architecture {
            format!("{}:{architecture}", self.name)
        } else {
            self.name.clone()
        }
    }
}

/// A package scheduled for removal as part of the update.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RemovalCandidate {
    /// Package name.
    pub name: String,
    /// Currently installed version being removed.
    pub installed_version: String,
}

/// An exact package/version explicitly requested before dependency resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateRequest {
    pub architecture: Option<String>,
    pub name: String,
    pub version: String,
}

impl UpdateRequest {
    /// Preserve an explicit architecture without adding a second qualifier.
    pub fn package_spec(&self) -> String {
        let name = if self.name.contains(':') {
            self.name.clone()
        } else if let Some(architecture) = &self.architecture {
            format!("{}:{architecture}", self.name)
        } else {
            self.name.clone()
        };
        format!("{name}={}", self.version)
    }
}

/// Resolver intent, independent of the number of resolved package operations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UpdateIntent {
    FullUpgrade,
    Selected(Vec<UpdateRequest>),
}

/// An immutable, reviewed update plan.
///
/// Once created, no mutations are allowed. If the package set changes (e.g.,
/// a new `apt update` reveals different candidates), the plan must be discarded
/// and a fresh review performed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdatePlan {
    /// Original resolver request, excluding dependencies added by APT.
    pub intent: UpdateIntent,
    /// Unique plan identifier.
    pub plan_id: String,
    /// When the plan was created.
    pub created_at: SystemTime,
    /// Packages to be upgraded or newly installed.
    pub candidates: Vec<UpdateCandidate>,
    /// Packages to be removed.
    pub removals: Vec<RemovalCandidate>,
    /// Packages held back.
    pub held: Vec<UpdateCandidate>,
    /// Total download size in bytes, if all sizes are known.
    pub total_download_bytes: Option<u64>,
    /// Content hash of the candidate set for staleness detection.
    pub content_hash: String,
    /// Whether the system currently requires a reboot (e.g., due to a previously
    /// installed kernel update). Detected by the presence of `/var/run/reboot-required`.
    pub reboot_required: bool,
}

impl UpdatePlan {
    /// Returns `true` if there are no candidates to install or upgrade.
    pub fn is_empty(&self) -> bool {
        self.candidates.is_empty()
    }

    /// Count of security updates.
    pub fn security_count(&self) -> usize {
        self.candidates
            .iter()
            .filter(|c| c.classification == UpdateClassification::Security)
            .count()
    }

    /// Returns `true` if any removals are part of this plan.
    pub fn has_removals(&self) -> bool {
        !self.removals.is_empty()
    }
}

/// Transaction execution status from PackageKit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransactionStatus {
    /// Setting up the transaction.
    Setup,
    /// Downloading packages.
    Downloading,
    /// Installing packages.
    Installing,
    /// Cleaning up after installation.
    Cleanup,
    /// Transaction completed successfully.
    Finished,
    /// Transaction failed.
    Failed,
    /// Transaction was cancelled.
    Cancelled,
}

/// Progress of an ongoing update transaction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateProgress {
    /// Current transaction status.
    pub status: TransactionStatus,
    /// Overall percentage (0–100), if known.
    pub percentage: Option<u32>,
    /// Package currently being processed.
    pub current_package: Option<String>,
    /// Whether the backend allows cancellation at this point.
    pub allow_cancel: bool,
}

/// Final result of an update execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateResult {
    /// Whether the transaction completed successfully.
    pub success: bool,
    /// Packages that were successfully updated.
    pub updated_packages: Vec<String>,
    /// Packages that failed to update.
    pub failed_packages: Vec<String>,
    /// Whether a system restart is required.
    pub restart_required: bool,
    /// Error message if the transaction failed.
    pub error_message: Option<String>,
    /// Error code from the backend.
    pub error_code: Option<String>,
}

/// Discovery result from checking for available updates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryResult {
    /// Updates available; plan is ready for review.
    Available(UpdatePlan),
    /// No updates available.
    UpToDate,
    /// Discovery failed with a reason.
    Failed(DiscoveryError),
}

/// Reasons update discovery can fail.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiscoveryError {
    /// The dpkg lock is held by another process.
    DpkgLocked,
    /// Network repositories are unreachable.
    RepositoryOffline { details: String },
    /// The backend command failed.
    CommandFailed { message: String },
    /// Cancelled by the user.
    Cancelled,
}

impl std::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::DpkgLocked => write!(f, "Another package manager is running"),
            Self::RepositoryOffline { details } => {
                write!(f, "Repository unreachable: {details}")
            }
            Self::CommandFailed { message } => write!(f, "Update check failed: {message}"),
            Self::Cancelled => write!(f, "Update check was cancelled"),
        }
    }
}
