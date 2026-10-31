//! On-demand cleanup inspection, reviewed plans and private attempt history.
//!
//! Review retains file and parent descriptors. A root-private quarantine binds
//! the reviewed identity to unlink, with no-overwrite recovery after races.

pub mod analyze;
pub mod execute;
pub mod plan;
pub mod quarantine;
pub mod recovery;
pub mod trash;

pub use analyze::{
    CleanupAnalyzer, CleanupCategory, CleanupTarget, PrivilegeRequirement, Reversibility,
};
pub use execute::{CleanupExecutor, CleanupResult, ExecutionRequest};
pub use plan::{CleanupItem, CleanupPlan};
pub use trash::{TrashOutcome, trash_file};
