//! Reviewed descriptor-bound cleanup with cancellation and private history.

use std::path::Path;
use std::time::SystemTime;

use uuid::Uuid;

use crate::cleanup::plan::CleanupPlan;
use crate::error::MaintainerError;
use crate::jobs::{Cancellation, ItemResult, JobProgress, OperationOutcome, ProgressSink};
use crate::persistence::history::{HistoryEntry, HistoryRepository, OperationStatus};

/// Shared capability reason for every unproved filesystem mutation route.
pub const CLEANUP_UNAVAILABLE_REASON: &str = "Removal requires the installed administrator-authorized helper and a supported regular file on its quarantine filesystem. Trash has no permanent-deletion fallback.";

/// Request to execute specific items from a plan.
#[derive(Debug, Clone)]
pub struct ExecutionRequest {
    /// Plan ID to execute from.
    pub plan_id: Uuid,
    /// Selected item IDs to execute.
    pub selected_item_ids: Vec<Uuid>,
    /// Explicit stronger confirmation for permanent deletion.
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
    /// Conservative physical reclamation count; retained descriptors can delay freeing.
    pub actual_recovered_bytes: u64,
}

/// Outcome for a single cleaned item.
#[derive(Debug, Clone)]
pub struct CleanupItemOutcome {
    /// A committed operation can retain a durability warning without inviting a retry.
    pub notice: Option<String>,
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

/// Executes reviewed files through the protected privileged helper.
pub struct CleanupExecutor<'a> {
    _home_dir: &'a Path,
}

impl<'a> CleanupExecutor<'a> {
    /// Create an executor for the given home directory.
    pub fn new(home_dir: &'a Path) -> Self {
        Self {
            _home_dir: home_dir,
        }
    }

    /// Validate plan membership, report each selected item unavailable, and
    /// preserve cancellation and history without accessing filesystem targets.
    pub fn execute(
        &self,
        plan: &CleanupPlan,
        request: &ExecutionRequest,
        cancel: &dyn Cancellation,
        progress: &dyn ProgressSink,
        history: &dyn HistoryRepository,
    ) -> Result<CleanupResult, MaintainerError> {
        if plan.id != request.plan_id {
            return Err(MaintainerError::Internal {
                context: "cleanup execution".into(),
                message: "plan ID mismatch".into(),
            });
        }

        if request.selected_item_ids.is_empty() {
            return Err(MaintainerError::io(
                "cleanup unavailable",
                std::io::Error::new(std::io::ErrorKind::Unsupported, CLEANUP_UNAVAILABLE_REASON),
            ));
        }

        let started_at = unix_timestamp_secs();
        let total = request.selected_item_ids.len() as u64;
        let mut item_results: Vec<ItemResult<CleanupItemOutcome>> = Vec::new();
        let mut failed = 0u64;
        let mut succeeded = 0u64;
        let mut cancelled = false;
        let manifest = reviewed_manifest(plan, request);
        let mut batch: Option<Result<CleanupBatch, String>> = None;

        for (idx, item_id) in request.selected_item_ids.iter().enumerate() {
            if cancel.is_cancelled() {
                cancelled = true;
                break;
            }

            progress.report(&JobProgress {
                completed: idx as u64,
                total: Some(total),
                message: format!("Checking item {}/{total}", idx + 1),
            });

            let item = match plan.item_by_id(item_id) {
                Some(i) => i,
                None => {
                    failed += 1;
                    item_results.push(ItemResult::Failed {
                        item: item_id.to_string(),
                        message: "item not found in plan".into(),
                    });
                    continue;
                }
            };

            let result = (|| -> Result<String, String> {
                if item.reversibility == crate::cleanup::Reversibility::PermanentOnly
                    && !request.confirm_permanent
                {
                    return Err("Permanent removal was not explicitly confirmed".into());
                }
                let pinned = item.reviewed.as_ref().ok_or_else(|| {
                    item.unavailable_reason.clone().unwrap_or_else(|| {
                        "File removal unavailable: no retained review descriptor".into()
                    })
                })?;
                use std::os::unix::fs::MetadataExt;
                let metadata = pinned.file.metadata().map_err(|error| error.to_string())?;
                if item.inode != metadata.ino() || item.device != metadata.dev() {
                    return Err("File removal unavailable: review metadata changed".into());
                }
                let manifest = manifest.as_ref().map_err(Clone::clone)?;
                let index = manifest
                    .iter()
                    .position(|(id, _)| id == item_id)
                    .ok_or_else(|| "Reviewed file is outside the authorized manifest".to_string())?
                    as u32;
                let batch = batch
                    .get_or_insert_with(|| CleanupBatch::begin(manifest))
                    .as_ref()
                    .map_err(Clone::clone)?;
                remove_via_helper(
                    batch,
                    index,
                    pinned,
                    item.reversibility == crate::cleanup::Reversibility::TrashRecoverable,
                    self._home_dir,
                )
            })();
            match result {
                Ok(message) => {
                    succeeded += 1;
                    item_results.push(ItemResult::Ok(CleanupItemOutcome {
                        notice: (!message.is_empty()).then_some(message),
                        item_id: item.id,
                        recovered_bytes: 0,
                        method: if item.reversibility
                            == crate::cleanup::Reversibility::TrashRecoverable
                        {
                            RemovalMethod::Trashed
                        } else {
                            RemovalMethod::PermanentlyDeleted
                        },
                    }));
                }
                Err(message) => {
                    failed += 1;
                    item_results.push(ItemResult::Failed {
                        item: item.display_path.clone(),
                        message,
                    });
                }
            }
        }

        if let Some(Ok(batch)) = batch {
            if let Err(message) = batch.end() {
                failed += 1;
                item_results.push(ItemResult::Failed {
                    item: "cleanup authorization".into(),
                    message,
                });
            }
        }

        progress.report(&JobProgress {
            completed: item_results.len() as u64,
            total: Some(total),
            message: "Reviewed cleanup finished".into(),
        });

        let completed_at = unix_timestamp_secs();
        let outcome = OperationOutcome {
            succeeded,
            failed,
            skipped: 0,
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
        // One fixed category is sufficient regardless of item count. Paths and
        // arbitrary failure text never enter the private history record.
        let errors_json = (failed > 0).then(|| r#"["file-cleanup-failed"]"#.to_string());

        let entry = HistoryEntry::new(
            "cleanup",
            started_at,
            Some(completed_at),
            status,
            0,
            errors_json,
        );
        history.record(&entry)?;

        Ok(CleanupResult {
            plan_id: plan.id,
            item_results,
            outcome,
            actual_recovered_bytes: 0,
        })
    }
}

type ManifestEntry = (String, u64, u64, u64, u64, bool);
fn reviewed_manifest(
    plan: &CleanupPlan,
    request: &ExecutionRequest,
) -> Result<Vec<(Uuid, ManifestEntry)>, String> {
    use std::os::unix::fs::MetadataExt;
    let mut manifest = Vec::new();
    for id in &request.selected_item_ids {
        let Some(item) = plan.item_by_id(id) else {
            continue;
        };
        let Some(pinned) = item.reviewed.as_ref() else {
            continue;
        };
        if item.reversibility == crate::cleanup::Reversibility::PermanentOnly
            && !request.confirm_permanent
        {
            continue;
        }
        let parent = pinned
            .parent
            .metadata()
            .map_err(|error| error.to_string())?;
        manifest.push((
            *id,
            (
                pinned.basename.clone(),
                parent.dev(),
                parent.ino(),
                item.device,
                item.inode,
                item.reversibility == crate::cleanup::Reversibility::TrashRecoverable,
            ),
        ));
    }
    Ok(manifest)
}

struct CleanupBatch {
    connection: zbus::blocking::Connection,
    grant: String,
}
impl CleanupBatch {
    fn begin(manifest: &[(Uuid, ManifestEntry)]) -> Result<Self, String> {
        let connection = zbus::blocking::Connection::system()
            .map_err(|error| format!("File removal unavailable: {error}"))?;
        let entries = manifest
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect::<Vec<_>>();
        let reply = connection
            .call_method(
                Some("io.github.maintainerapp.Helper"),
                "/io/github/maintainerapp/Helper",
                Some("io.github.maintainerapp.Helper"),
                "BeginCleanupBatch",
                &(entries,),
            )
            .map_err(|error| format!("File removal unavailable: {error}"))?;
        let grant = reply
            .body()
            .deserialize()
            .map_err(|error| error.to_string())?;
        Ok(Self { connection, grant })
    }
    fn end(self) -> Result<(), String> {
        self.connection.call_method(Some("io.github.maintainerapp.Helper"), "/io/github/maintainerapp/Helper", Some("io.github.maintainerapp.Helper"), "EndCleanupBatch", &(self.grant.as_str(),)).map_err(|error| format!("Cleanup authorization could not be released; it will expire without replay. Review again before another attempt: {error}"))?;
        Ok(())
    }
}

fn remove_via_helper(
    batch: &CleanupBatch,
    index: u32,
    pinned: &crate::cleanup::plan::ReviewedFile,
    trash: bool,
    home: &Path,
) -> Result<String, String> {
    use std::os::fd::AsFd;
    let trash_directories = if trash {
        Some(
            crate::cleanup::trash::open_trash(home)
                .map_err(|error| format!("Trash unavailable: {error}"))?,
        )
    } else {
        None
    };
    let files = trash_directories
        .as_ref()
        .map(|pair| &pair.0)
        .unwrap_or(&pinned.parent);
    let info = trash_directories
        .as_ref()
        .map(|pair| &pair.1)
        .unwrap_or(&pinned.parent);
    let reply = batch
        .connection
        .call_method(
            Some("io.github.maintainerapp.Helper"),
            "/io/github/maintainerapp/Helper",
            Some("io.github.maintainerapp.Helper"),
            "RemoveReviewedBatchFile",
            &(
                batch.grant.as_str(),
                index,
                zbus::zvariant::Fd::from(pinned.parent.as_fd()),
                zbus::zvariant::Fd::from(pinned.file.as_fd()),
                pinned.basename.as_str(),
                trash,
                zbus::zvariant::Fd::from(files.as_fd()),
                zbus::zvariant::Fd::from(info.as_fd()),
            ),
        )
        .map_err(|error| format!("File removal unavailable: {error}"))?;
    let (success, message): (bool, String) = reply
        .body()
        .deserialize()
        .map_err(|error| error.to_string())?;
    if success { Ok(message) } else { Err(message) }
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
    use std::fs::{self, File};
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn persisted_cleanup_errors_use_private_bounded_categories() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();
        let target = make_file(&cache_dir, "private-name.png", b"content");
        let path = target.path.clone();
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        let request = ExecutionRequest {
            plan_id: plan.id,
            selected_item_ids: vec![plan.items[0].id],
            confirm_permanent: true,
        };
        fs::remove_file(path).unwrap();

        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let result = CleanupExecutor::new(home.path()).execute(
            &plan,
            &request,
            &CancellationToken::new(),
            &NullProgress,
            &repo,
        );

        assert!(result.is_ok());
        let errors = repo.recent(1).unwrap()[0].errors.clone().unwrap();
        assert_eq!(errors, r#"["file-cleanup-failed"]"#);
        assert!(!errors.contains("private-name.png"));
        assert!(!errors.contains(home.path().to_string_lossy().as_ref()));
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
    fn execute_refuses_confirmed_items_without_deleting_or_crediting_space() {
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

        assert_eq!(result.outcome.succeeded, 0);
        assert_eq!(result.outcome.failed, 1);
        assert_eq!(result.actual_recovered_bytes, 0);
        assert!(file_path.exists());

        let history = repo.recent(1).unwrap();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].operation, "cleanup");
        assert_eq!(history[0].status, OperationStatus::Failure);
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
        assert!(home.path().join(".cache/thumbnails/thumb.png").exists());
    }

    #[test]
    fn stale_inode_cannot_enable_mutation() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let target = make_file(&cache_dir, "racing.png", &[0u8; 128]);
        let file_path = target.path.clone();

        let mut plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();

        // Simulate a race by recording a stale inode in the plan.
        // On tmpfs, re-creating a file often reuses the same inode, so we
        let current_inode = fs::symlink_metadata(&file_path).unwrap().ino();
        plan.items[0].inode = current_inode.wrapping_add(999);

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

        assert_eq!(result.outcome.failed, 1);
        assert_eq!(result.outcome.succeeded, 0);
        assert!(file_path.exists());
    }

    fn assert_denied(plan: &CleanupPlan, home: &Path, confirmed: bool) {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let result = CleanupExecutor::new(home)
            .execute(
                plan,
                &ExecutionRequest {
                    plan_id: plan.id,
                    selected_item_ids: plan.items.iter().map(|item| item.id).collect(),
                    confirm_permanent: confirmed,
                },
                &CancellationToken::new(),
                &NullProgress,
                &repo,
            )
            .unwrap();
        assert_eq!(result.outcome.succeeded, 0);
        assert_eq!(result.outcome.failed, plan.items.len() as u64);
        assert_eq!(result.actual_recovered_bytes, 0);
        assert!(result.item_results.iter().all(|item| matches!(item,
            ItemResult::Failed { message, .. } if message.contains("unavailable") || message.contains("confirmed")
        )));
        let history = repo.recent(1).unwrap();
        assert_eq!(history[0].status, OperationStatus::Failure);
        assert_eq!(history[0].recovered_bytes, 0);
    }

    #[test]
    fn replacement_leaf_is_preserved_for_both_confirmation_states() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&directory).unwrap();
        let target = make_file(&directory, "reviewed", b"original");
        let path = target.path.clone();
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        fs::rename(&path, directory.join("retained-original")).unwrap();
        fs::write(&path, b"replacement").unwrap();
        for confirmed in [false, true] {
            assert_denied(&plan, home.path(), confirmed);
            assert_eq!(fs::read(&path).unwrap(), b"replacement");
            assert_eq!(
                fs::read(directory.join("retained-original")).unwrap(),
                b"original"
            );
        }
    }

    #[test]
    fn replacement_ancestor_does_not_touch_outside_victim() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&directory).unwrap();
        let target = make_file(&directory, "reviewed", b"original");
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        let outside = tempfile::tempdir().unwrap();
        fs::write(outside.path().join("reviewed"), b"outside victim").unwrap();
        let retained = home.path().join("retained-original");
        fs::rename(&directory, &retained).unwrap();
        std::os::unix::fs::symlink(outside.path(), &directory).unwrap();
        assert_denied(&plan, home.path(), true);
        assert_eq!(
            fs::read(outside.path().join("reviewed")).unwrap(),
            b"outside victim"
        );
        assert_eq!(fs::read(retained.join("reviewed")).unwrap(), b"original");
    }

    #[test]
    fn added_descendants_and_existing_open_writer_remain_untouched() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&directory).unwrap();
        let mut target = make_file(&directory, "reviewed", b"original");
        let mut writer = fs::OpenOptions::new()
            .append(true)
            .open(&target.path)
            .unwrap();
        target.path = directory.clone();
        let metadata = fs::symlink_metadata(&directory).unwrap();
        target.inode = metadata.ino();
        target.device = metadata.dev();
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        fs::write(directory.join("added-after-review"), b"unapproved").unwrap();
        writer.write_all(b" writer change").unwrap();
        assert_denied(&plan, home.path(), true);
        writer.write_all(b" still open").unwrap();
        assert_eq!(
            fs::read(directory.join("reviewed")).unwrap(),
            b"original writer change still open"
        );
        assert_eq!(
            fs::read(directory.join("added-after-review")).unwrap(),
            b"unapproved"
        );
    }

    #[test]
    fn empty_or_unknown_selection_cannot_report_success() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&directory).unwrap();
        let target = make_file(&directory, "reviewed", b"original");
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        for selected_item_ids in [vec![], vec![Uuid::new_v4()]] {
            let result = CleanupExecutor::new(home.path()).execute(
                &plan,
                &ExecutionRequest {
                    plan_id: plan.id,
                    selected_item_ids,
                    confirm_permanent: true,
                },
                &CancellationToken::new(),
                &NullProgress,
                &repo,
            );
            if let Ok(result) = result {
                assert!(!result.outcome.is_complete_success());
                assert_eq!(result.actual_recovered_bytes, 0);
            }
            assert_eq!(fs::read(directory.join("reviewed")).unwrap(), b"original");
        }
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

        let token = CancellationToken::new();
        token.cancel();

        let result = executor
            .execute(&plan, &request, &token, &NullProgress, &repo)
            .unwrap();

        assert!(result.outcome.cancelled);
        assert_eq!(result.outcome.succeeded, 0);

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
    fn mixed_existing_and_missing_items_record_no_success() {
        let home = tempfile::tempdir().unwrap();
        let cache_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&cache_dir).unwrap();

        let t1 = make_file(&cache_dir, "good.png", &[0u8; 100]);
        let t2 = make_file(&cache_dir, "will_vanish.png", &[0u8; 200]);
        let vanish_path = t2.path.clone();

        let plan = CleanupPlan::from_targets(vec![t1, t2], home.path()).unwrap();

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

        assert_eq!(result.outcome.succeeded, 0);
        assert_eq!(result.outcome.failed, 2);
        assert_eq!(result.actual_recovered_bytes, 0);

        let history = repo.recent(1).unwrap();
        assert_eq!(history[0].status, OperationStatus::Failure);
    }

    #[test]
    fn trash_recoverable_item_stays_in_place() {
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

        assert_eq!(result.outcome.succeeded, 0);
        assert!(file_path.exists());
        let trash_files = home.path().join(".local/share/Trash/files");
        assert!(!trash_files.exists() || fs::read_dir(trash_files).unwrap().next().is_none());
        assert_eq!(result.actual_recovered_bytes, 0);
    }
}
