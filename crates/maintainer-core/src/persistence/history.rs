//! Operation history persistence.
//!
//! Records completed maintenance operations (cleanup, scans, updates) with
//! their result status, recovered bytes, and any error details.

use crate::error::MaintainerError;
use crate::persistence::database::Database;

/// Status of a completed operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationStatus {
    Success,
    PartialFailure,
    Failure,
    Cancelled,
}

impl OperationStatus {
    fn as_str(self) -> &'static str {
        match self {
            Self::Success => "success",
            Self::PartialFailure => "partial_failure",
            Self::Failure => "failure",
            Self::Cancelled => "cancelled",
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s {
            "success" => Some(Self::Success),
            "partial_failure" => Some(Self::PartialFailure),
            "failure" => Some(Self::Failure),
            "cancelled" => Some(Self::Cancelled),
            _ => None,
        }
    }
}

/// A single recorded operation in the history.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEntry {
    /// Unique identifier (UUID v4).
    pub id: String,
    /// Type of operation (e.g. "cleanup", "scan", "update").
    pub operation: String,
    /// When the operation started (UTC Unix seconds).
    pub started_at: i64,
    /// When the operation completed (UTC Unix seconds), or None if still running.
    pub completed_at: Option<i64>,
    /// Final status of the operation.
    pub status: OperationStatus,
    /// Bytes recovered (for cleanup operations), always >= 0.
    pub recovered_bytes: u64,
    /// Error messages, if any. Stored as JSON array.
    pub errors: Option<String>,
}

impl HistoryEntry {
    /// Create a new history entry with a generated UUID.
    pub fn new(
        operation: impl Into<String>,
        started_at: i64,
        completed_at: Option<i64>,
        status: OperationStatus,
        recovered_bytes: u64,
        errors: Option<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            operation: operation.into(),
            started_at,
            completed_at,
            status,
            recovered_bytes,
            errors,
        }
    }
}

/// Trait for history persistence operations.
pub trait HistoryRepository: Send + Sync {
    /// Record a completed operation.
    fn record(&self, entry: &HistoryEntry) -> Result<(), MaintainerError>;
    /// Retrieve the most recent operations, ordered newest first.
    fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, MaintainerError>;
    /// Delete a single history entry by ID. Returns true if the entry existed.
    fn delete_one(&self, id: &str) -> Result<bool, MaintainerError>;
    /// Delete all history entries. Returns the number of rows deleted.
    fn delete_all(&self) -> Result<u64, MaintainerError>;
}

/// SQLite-backed implementation of [`HistoryRepository`].
pub struct SqliteHistoryRepository<'a> {
    db: &'a Database,
}

impl<'a> SqliteHistoryRepository<'a> {
    /// Create a new repository backed by the given database.
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }
}

impl HistoryRepository for SqliteHistoryRepository<'_> {
    fn record(&self, entry: &HistoryEntry) -> Result<(), MaintainerError> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO operation_history (id, operation, started_at, completed_at, status, recovered_bytes, errors)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![
                    entry.id,
                    entry.operation,
                    entry.started_at,
                    entry.completed_at,
                    entry.status.as_str(),
                    entry.recovered_bytes as i64,
                    entry.errors,
                ],
            )
            .map_err(|e| MaintainerError::Internal {
                context: "recording history entry".into(),
                message: e.to_string(),
            })?;
            Ok(())
        })
    }

    fn recent(&self, limit: usize) -> Result<Vec<HistoryEntry>, MaintainerError> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, operation, started_at, completed_at, status, recovered_bytes, errors
                     FROM operation_history
                     ORDER BY started_at DESC
                     LIMIT ?1",
                )
                .map_err(|e| MaintainerError::Internal {
                    context: "preparing history query".into(),
                    message: e.to_string(),
                })?;

            let rows = stmt
                .query_map(rusqlite::params![limit as i64], |row| {
                    Ok(HistoryEntryRow {
                        id: row.get(0)?,
                        operation: row.get(1)?,
                        started_at: row.get(2)?,
                        completed_at: row.get(3)?,
                        status_str: row.get(4)?,
                        recovered_bytes: row.get(5)?,
                        errors: row.get(6)?,
                    })
                })
                .map_err(|e| MaintainerError::Internal {
                    context: "querying history".into(),
                    message: e.to_string(),
                })?;

            let mut entries = Vec::new();
            for row_result in rows {
                let row = row_result.map_err(|e| MaintainerError::Internal {
                    context: "reading history row".into(),
                    message: e.to_string(),
                })?;

                let status =
                    OperationStatus::from_str(&row.status_str).unwrap_or_else(|| {
                        eprintln!(
                            "warning: unknown operation status '{}' for entry {}, defaulting to Failure",
                            row.status_str, row.id
                        );
                        OperationStatus::Failure
                    });

                entries.push(HistoryEntry {
                    id: row.id,
                    operation: row.operation,
                    started_at: row.started_at,
                    completed_at: row.completed_at,
                    status,
                    recovered_bytes: row.recovered_bytes as u64,
                    errors: row.errors,
                });
            }

            Ok(entries)
        })
    }

    fn delete_one(&self, id: &str) -> Result<bool, MaintainerError> {
        self.db.with_connection(|conn| {
            let deleted = conn
                .execute(
                    "DELETE FROM operation_history WHERE id = ?1",
                    rusqlite::params![id],
                )
                .map_err(|e| MaintainerError::Internal {
                    context: "deleting history entry".into(),
                    message: e.to_string(),
                })?;
            Ok(deleted > 0)
        })
    }

    fn delete_all(&self) -> Result<u64, MaintainerError> {
        self.db.with_connection(|conn| {
            let deleted = conn
                .execute("DELETE FROM operation_history", [])
                .map_err(|e| MaintainerError::Internal {
                    context: "deleting history".into(),
                    message: e.to_string(),
                })?;
            Ok(deleted as u64)
        })
    }
}

/// Intermediate struct for row mapping to avoid complex closures.
struct HistoryEntryRow {
    id: String,
    operation: String,
    started_at: i64,
    completed_at: Option<i64>,
    status_str: String,
    recovered_bytes: i64,
    errors: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn record_and_retrieve_entry() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);

        let entry = HistoryEntry::new(
            "cleanup",
            1_700_000_000,
            Some(1_700_000_060),
            OperationStatus::Success,
            1024,
            None,
        );
        repo.record(&entry).unwrap();

        let recent = repo.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].id, entry.id);
        assert_eq!(recent[0].operation, "cleanup");
        assert_eq!(recent[0].status, OperationStatus::Success);
        assert_eq!(recent[0].recovered_bytes, 1024);
        assert!(recent[0].errors.is_none());
    }

    #[test]
    fn recent_returns_newest_first() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);

        let e1 = HistoryEntry::new(
            "scan",
            1_700_000_000,
            Some(1_700_000_010),
            OperationStatus::Success,
            0,
            None,
        );
        let e2 = HistoryEntry::new(
            "cleanup",
            1_700_000_100,
            Some(1_700_000_200),
            OperationStatus::Success,
            500,
            None,
        );
        let e3 = HistoryEntry::new(
            "update",
            1_700_000_200,
            Some(1_700_000_300),
            OperationStatus::Failure,
            0,
            Some("[\"apt failed\"]".into()),
        );

        repo.record(&e1).unwrap();
        repo.record(&e2).unwrap();
        repo.record(&e3).unwrap();

        let recent = repo.recent(2).unwrap();
        assert_eq!(recent.len(), 2);
        assert_eq!(recent[0].operation, "update");
        assert_eq!(recent[1].operation, "cleanup");
    }

    #[test]
    fn partial_failure_with_errors() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);

        let errors =
            Some("[\"permission denied on /var/log\",\"timeout reading disk\"]".to_string());
        let entry = HistoryEntry::new(
            "cleanup",
            1_700_000_000,
            Some(1_700_000_030),
            OperationStatus::PartialFailure,
            256,
            errors.clone(),
        );
        repo.record(&entry).unwrap();

        let recent = repo.recent(1).unwrap();
        assert_eq!(recent[0].status, OperationStatus::PartialFailure);
        assert_eq!(recent[0].errors, errors);
        assert_eq!(recent[0].recovered_bytes, 256);
    }

    #[test]
    fn delete_all_removes_everything() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);

        for i in 0..5 {
            let entry = HistoryEntry::new(
                "scan",
                1_700_000_000 + i,
                Some(1_700_000_010 + i),
                OperationStatus::Success,
                0,
                None,
            );
            repo.record(&entry).unwrap();
        }

        let deleted = repo.delete_all().unwrap();
        assert_eq!(deleted, 5);

        let recent = repo.recent(100).unwrap();
        assert!(recent.is_empty());
    }

    #[test]
    fn delete_one_removes_specific_entry() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);

        let entry1 = HistoryEntry::new(
            "scan",
            1_700_000_000,
            Some(1_700_000_010),
            OperationStatus::Success,
            0,
            None,
        );
        let entry2 = HistoryEntry::new(
            "cleanup",
            1_700_000_100,
            Some(1_700_000_200),
            OperationStatus::Success,
            512,
            None,
        );
        let id1 = entry1.id.clone();
        repo.record(&entry1).unwrap();
        repo.record(&entry2).unwrap();

        let deleted = repo.delete_one(&id1).unwrap();
        assert!(deleted);

        let recent = repo.recent(10).unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].operation, "cleanup");
    }

    #[test]
    fn delete_one_nonexistent_returns_false() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let deleted = repo.delete_one("nonexistent-id").unwrap();
        assert!(!deleted);
    }

    #[test]
    fn delete_all_on_empty_returns_zero() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);
        let deleted = repo.delete_all().unwrap();
        assert_eq!(deleted, 0);
    }

    #[test]
    fn cancelled_operation_stored_correctly() {
        let db = test_db();
        let repo = SqliteHistoryRepository::new(&db);

        let entry = HistoryEntry::new(
            "scan",
            1_700_000_000,
            None,
            OperationStatus::Cancelled,
            0,
            None,
        );
        repo.record(&entry).unwrap();

        let recent = repo.recent(1).unwrap();
        assert_eq!(recent[0].status, OperationStatus::Cancelled);
        assert!(recent[0].completed_at.is_none());
    }
}
