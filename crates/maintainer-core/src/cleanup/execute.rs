//! Cleanup execution engine.
//!
//! Receives an immutable plan ID and selected item IDs, revalidates each target
//! (checking inode hasn't changed), executes removals independently, checks
//! cancellation between items, collects outcomes, and records to history.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use uuid::Uuid;

use crate::cleanup::plan::{CleanupItem, CleanupPlan};
use crate::cleanup::trash::{self, TrashOutcome};
use crate::error::MaintainerError;
use crate::jobs::{Cancellation, ItemResult, JobProgress, OperationOutcome, ProgressSink};
use crate::persistence::history::{HistoryEntry, HistoryRepository, OperationStatus};

/// Request to execute specific items from a plan.
#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    /// Plan ID to execute from.
    pub plan_id: Uuid,
    /// Selected item IDs to execute.
    pub selected_item_ids: Vec<Uuid>,
    /// If true, permanently delete items that cannot be trashed.
    /// Must be explicitly confirmed by the user for permanent deletions.
    pub confirm_permanent: bool,
}

/// Result of a cleanup execution.
#[derive(Debug, Clone)]
pub struct CleanupResult {
    /// Plan ID that was executed.
    pub plan_id: Uuid,
    /// Per-item outcomes.
    pub item_results: Vec<ItemResult<CleanupItemOutcome>>,
    /// Summary.
    pub outcome: OperationOutcome,
    /// Actual bytes recovered (measured from successful deletions).
    pub actual_recovered_bytes: u64,
}

/// Outcome for a single cleaned item.
#[derive(Debug, Clone)]
pub struct CleanupItemOutcome {
    /// Item ID.
    pub item_id: Uuid,
    /// Bytes actually recovered.
    pub recovered_bytes: u64,
    /// Whether the item was trashed or permanently deleted.
    pub method: RemovalMethod,
}

/// How an item was removed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RemovalMethod {
    /// Moved to FreeDesktop Trash.
    Trashed,
    /// Permanently removed from disk.
    PermanentlyDeleted,
}

/// The cleanup executor validates and removes targets from an approved plan.
pub struct CleanupExecutor<'a> {
    home_dir: &'a Path,
}

impl<'a> CleanupExecutor<'a> {
    /// Create an executor for the given home directory.
    pub fn new(home_dir: &'a Path) -> Self {
        Self { home_dir }
    }

    /// Execute the requested cleanup items from the given plan.
    ///
    /// Revalidates each target, checks cancellation between items,
    /// collects all outcomes, and records history.
    pub fn execute(
        &self,
        plan: &CleanupPlan,
        request: &ExecutionRequest,
        cancel: &dyn Cancellation,
        progress: &dyn ProgressSink,
        history: &dyn HistoryRepository,
    ) -> Result<CleanupResult, MaintainerError> {
        // Validate request matches plan.
        if plan.id != request.plan_id {
            return Err(MaintainerError::Internal {
                context: "cleanup execution".into(),
                message: "plan ID mismatch".into(),
            });
        }

        let started_at = unix_timestamp_secs();
        let total = request.selected_item_ids.len() as u64;
        let mut item_results: Vec<ItemResult<CleanupItemOutcome>> = Vec::new();
        let mut actual_recovered_bytes = 0u64;
        let mut succeeded = 0u64;
        let mut failed = 0u64;
        let mut skipped = 0u64;
        let mut cancelled = false;
        let mut errors: Vec<String> = Vec::new();

        for (idx, item_id) in request.selected_item_ids.iter().enumerate() {
            // Check cancellation between items.
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }

            progress.report(&JobProgress {
                completed: idx as u64,
                total: Some(total),
                message: format!("Removing item {}/{total}", idx + 1),
            });

            let item = match plan.item_by_id(item_id) {
                Some(i) => i,
                None => {
                    skipped += 1;
                    item_results.push(ItemResult::Skipped {
                        item: item_id.to_string(),
                        reason: "item not found in plan".into(),
                    });
                    continue;
                }
            };

            match self.execute_item(item, request.confirm_permanent) {
                Ok(outcome) => {
                    actual_recovered_bytes =
                        actual_recovered_bytes.saturating_add(outcome.recovered_bytes);
                    succeeded += 1;
                    item_results.push(ItemResult::Ok(outcome));
                }
                Err(e) => {
                    let msg = e.to_string();
                    errors.push(msg.clone());
                    failed += 1;
                    item_results.push(ItemResult::Failed {
                        item: item.display_path.clone(),
                        message: msg,
                    });
                }
            }
        }

        progress.report(&JobProgress {
            completed: total,
            total: Some(total),
            message: "Cleanup complete".into(),
        });

        let completed_at = unix_timestamp_secs();
        let outcome = OperationOutcome {
            succeeded,
            failed,
            skipped,
            cancelled,
        };

        let status = if cancelled {
            OperationStatus::Cancelled
        } else if failed == 0 {
            OperationStatus::Success
        } else if succeeded > 0 {
            OperationStatus::PartialFailure
        } else {
            OperationStatus::Failure
        };

        let errors_json = if errors.is_empty() {
            None
        } else {
            Some(format!(
                "[{}]",
                errors
                    .iter()
                    .map(|e| format!("\"{}\"", e.replace('"', "\\\"")))
                    .collect::<Vec<_>>()
                    .join(",")
            ))
        };

        // Record to history.
        let entry = HistoryEntry::new(
            "cleanup",
            started_at,
            Some(completed_at),
            status,
            actual_recovered_bytes,
            errors_json,
        );
        history.record(&entry)?;

        Ok(CleanupResult {
            plan_id: plan.id,
            item_results,
            outcome,
            actual_recovered_bytes,
        })
    }

    /// Execute a single item: revalidate then remove.
    fn execute_item(
        &self,
        item: &CleanupItem,
        confirm_permanent: bool,
    ) -> Result<CleanupItemOutcome, MaintainerError> {
        // Revalidate: check path still exists and inode matches.
        let metadata = fs::symlink_metadata(&item.path)
            .map_err(|e| MaintainerError::io(format!("revalidating {}", item.display_path), e))?;

        if metadata.ino() != item.inode || metadata.dev() != item.device {
            return Err(MaintainerError::Internal {
                context: "revalidation".into(),
                message: format!(
                    "inode/device changed for {}: expected ({}, {}), got ({}, {})",
                    item.display_path,
                    item.inode,
                    item.device,
                    metadata.ino(),
                    metadata.dev()
                ),
            });
        }

        // Reject symlinks at execution time too.
        if metadata.is_symlink() {
            return Err(MaintainerError::Internal {
                context: "revalidation".into(),
                message: format!("{} became a symlink", item.display_path),
            });
        }

        // Determine actual size for recovery accounting.
        let actual_size = if metadata.is_dir() {
            dir_size_recursive(&item.path).unwrap_or(item.size_bytes)
        } else {
            metadata.len()
        };

        // Attempt removal based on reversibility.
        match item.reversibility {
            crate::cleanup::analyze::Reversibility::TrashRecoverable => {
                match trash::trash_file(&item.path, self.home_dir)? {
                    TrashOutcome::Trashed { .. } => Ok(CleanupItemOutcome {
                        item_id: item.id,
                        recovered_bytes: actual_size,
                        method: RemovalMethod::Trashed,
                    }),
                    TrashOutcome::TrashUnavailable { reason } => {
                        if confirm_permanent {
                            self.permanent_delete(&item.path, actual_size, item.id)
                        } else {
                            Err(MaintainerError::Internal {
                                context: "trash".into(),
                                message: format!(
                                    "Trash unavailable for {}: {reason}. Permanent deletion not confirmed.",
                                    item.display_path
                                ),
                            })
                        }
                    }
                }
            }
            crate::cleanup::analyze::Reversibility::PermanentOnly => {
                if !confirm_permanent {
                    return Err(MaintainerError::Internal {
                        context: "cleanup".into(),
                        message: format!(
                            "permanent deletion not confirmed for {}",
                            item.display_path
                        ),
                    });
                }
                self.permanent_delete(&item.path, actual_size, item.id)
            }
        }
    }

    /// Permanently delete a file or directory.
    fn permanent_delete(
        &self,
        path: &Path,
        size: u64,
        item_id: Uuid,
    ) -> Result<CleanupItemOutcome, MaintainerError> {
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| MaintainerError::io(format!("stat {}", path.display()), e))?;

        if metadata.is_dir() {
            fs::remove_dir_all(path).map_err(|e| {
                MaintainerError::io(format!("removing directory {}", path.display()), e)
            })?;
        } else {
            fs::remove_file(path)
                .map_err(|e| MaintainerError::io(format!("removing file {}", path.display()), e))?;
        }

        Ok(CleanupItemOutcome {
            item_id,
            recovered_bytes: size,
            method: RemovalMethod::PermanentlyDeleted,
        })
    }
}

/// Recursively compute directory size.
fn dir_size_recursive(path: &Path) -> Result<u64, std::io::Error> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_dir() {
            total += dir_size_recursive(&entry.path()).unwrap_or(0);
        } else if !meta.is_symlink() {
            total += meta.len();
        }
    }
    Ok(total)
}

/// Current Unix timestamp in seconds.
fn unix_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::analyze::{
        CleanupCategory, CleanupTarget, PrivilegeRequirement, Reversibility,
    };
    use crate::cleanup::plan::CleanupPlan;
    use crate::jobs::{CancellationToken, NullProgress};
    use crate::persistence::database::Database;
    use crate::persistence::history::SqliteHistoryRepository;
    use std::fs::File;
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    fn make_file(dir: &Path, name: &str, content: &[u8]) -> CleanupTarget {
        let path = dir.join(name);
        let mut f = File::create(&path).unwrap();
        f.write_all(content).unwrap();
        let metadata = fs::symlink_metadata(&path).unwrap();
        CleanupTarget {
            path: path.clone(),
            category: CleanupCategory::ThumbnailCache,
            display_path: format!("~/.cache/{name}"),
            size_bytes: content.len() as u64,
            privilege: PrivilegeRequirement::None,
            reversibility: Reversibility::PermanentOnly,
            inode: metadata.ino(),
            device: metadata.dev(),
            evidence_timestamp: SystemTime::now(),
        }
    }

    #[test]
    fn execute_permanently_deletes_confirmed_items() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let target = make_file(&cache_dir, "thumb.png", &[0u8; 512]);
        let file_path = target.path.clone();

        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: vec![plan.items[0].id],
            confirm_permanent: true,
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());
        let token = CancellationToken::new();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        assert_eq!(result.outcome.succeeded, 1);
        assert_eq!(result.outcome.failed, 0);
        assert_eq!(result.actual_recovered_bytes, 512);
        assert!(!file_path.exists());

        // Check history was recorded.
        let history = repo.recent(1).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].operation, "cleanup");
        assert_eq!(history[0].status, OperationStatus::Success);
    }

    #[test]
    fn execute_rejects_without_permanent_confirmation() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let target = make_file(&cache_dir, "thumb.png", &[0u8; 256]);

        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: vec![plan.items[0].id],
            confirm_permanent: false, // Not confirmed!
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());
        let token = CancellationToken::new();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        assert_eq!(result.outcome.succeeded, 0);
        assert_eq!(result.outcome.failed, 1);
        // File should still exist.
        assert!(home.path().join(".cache/thumbnails/thumb.png").exists());
    }

    #[test]
    fn execute_detects_inode_change() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let target = make_file(&cache_dir, "racing.png", &[0u8; 128]);
        let file_path = target.path.clone();

        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();

        // Simulate a race: delete and recreate with different inode.
        fs::remove_file(&file_path).unwrap();
        let mut f = File::create(&file_path).unwrap();
        f.write_all(&[0u8; 128]).unwrap();

        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: vec![plan.items[0].id],
            confirm_permanent: true,
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());
        let token = CancellationToken::new();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        // Should fail due to inode mismatch.
        assert_eq!(result.outcome.failed, 1);
        assert_eq!(result.outcome.succeeded, 0);
        // File should still exist (not deleted).
        assert!(file_path.exists());
    }

    #[test]
    fn execute_cancellation_stops_processing() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let t1 = make_file(&cache_dir, "a.png", &[0u8; 100]);
        let t2 = make_file(&cache_dir, "b.png", &[0u8; 200]);

        let plan = CleanupPlan::from_targets(vec![t1, t2], home.path()).unwrap();
        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: plan.items.iter().map(|i| i.id).collect(),
            confirm_permanent: true,
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());

        // Cancel immediately.
        let token = CancellationToken::new();
        token.cancel();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        assert!(result.outcome.cancelled);
        assert_eq!(result.outcome.succeeded, 0);

        // History should record cancelled status.
        let history = repo.recent(1).unwrap();
        assert_eq!(history[0].status, OperationStatus::Cancelled);
    }

    #[test]
    fn execute_plan_id_mismatch_errors() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let target = make_file(&cache_dir, "x.png", &[0u8; 64]);
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();

        let request = ExecutionRequest {
            plan_id: Uuid::new_v4(), // Wrong ID!
            selected_item_ids: vec![plan.items[0].id],
            confirm_permanent: true,
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());
        let token = CancellationToken::new();

        let result = executor.execute(&plan, &request, &token, &NullProgress, &repo);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("mismatch"));
    }

    #[test]
    fn execute_partial_failure_records_correctly() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let t1 = make_file(&cache_dir, "good.png", &[0u8; 100]);
        let t2 = make_file(&cache_dir, "will_vanish.png", &[0u8; 200]);
        let vanish_path = t2.path.clone();

        let plan = CleanupPlan::from_targets(vec![t1, t2], home.path()).unwrap();

        // Remove second file to cause revalidation failure.
        fs::remove_file(&vanish_path).unwrap();

        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: plan.items.iter().map(|i| i.id).collect(),
            confirm_permanent: true,
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());
        let token = CancellationToken::new();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        assert_eq!(result.outcome.succeeded, 1);
        assert_eq!(result.outcome.failed, 1);

        let history = repo.recent(1).unwrap();
        assert_eq!(history[0].status, OperationStatus::PartialFailure);
    }

    #[test]
    fn execute_trash_recoverable_item() {
        let home = tempfile::tempdir().unwrap();
        let doc_dir = home.path().join("Documents");
        fs::create_dir_all(&doc_dir).unwrap();

        let file_path = doc_dir.join("recoverable.txt");
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"important data").unwrap();

        let metadata = fs::symlink_metadata(&file_path).unwrap();
        let target = CleanupTarget {
            path: file_path.clone(),
            category: CleanupCategory::UserCache,
            display_path: "~/Documents/recoverable.txt".into(),
            size_bytes: 14,
            privilege: PrivilegeRequirement::None,
            reversibility: Reversibility::TrashRecoverable,
            inode: metadata.ino(),
            device: metadata.dev(),
            evidence_timestamp: SystemTime::now(),
        };

        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: vec![plan.items[0].id],
            confirm_permanent: false, // Trash doesn't need permanent confirmation.
        };

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let executor = CleanupExecutor::new(home.path());
        let token = CancellationToken::new();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        assert_eq!(result.outcome.succeeded, 1);
        assert!(!file_path.exists());
        // Should be in trash.
        let trash_files = home.path().join(".local/share/Trash/files");
        assert!(trash_files.exists());
    }
}
