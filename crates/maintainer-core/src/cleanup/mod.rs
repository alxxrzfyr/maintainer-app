//! Previewed cleanup with category analyzers, immutable plans, Trash support,
//! confirmed execution, and durable operation history.
//!
//! Flow: analyze → plan → confirm → execute → record.
//! Nothing is deleted without explicit user confirmation through a reviewed plan.

pub mod analyze;
pub mod execute;
pub mod plan;
pub mod trash;

pub use analyze::{
    CleanupAnalyzer, CleanupCategory, CleanupTarget, PrivilegeRequirement, Reversibility,
};
pub use execute::{CleanupExecutor, CleanupResult, ExecutionRequest};
pub use plan::{CleanupItem, CleanupPlan};
pub use trash::{TrashOutcome, trash_file};
