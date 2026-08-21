//! Schema migration runner.
//!
//! Migrations are embedded at compile time and executed in order. Each
//! migration runs in its own transaction; a failure rolls back only that
//! migration, leaving the database at the previous version.

use crate::error::MaintainerError;
use crate::persistence::database::Database;

/// Embedded migration scripts, ordered by version number.
static MIGRATIONS: &[(i32, &str)] = &[(1, include_str!("../../../../migrations/001_initial.sql"))];

/// Apply all pending migrations to the database.
///
/// Creates the `schema_migrations` table if it does not exist, then runs
/// each migration whose version exceeds the current maximum.
pub fn apply_all(db: &Database) -> Result<(), MaintainerError> {
    db.with_connection_mut(|conn| {
        // Bootstrap: ensure schema_migrations exists so we can query it.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS schema_migrations (
                version INTEGER PRIMARY KEY NOT NULL,
                applied_at INTEGER NOT NULL
            );",
        )
        .map_err(|e| MaintainerError::Internal {
            context: "creating schema_migrations bootstrap".into(),
            message: e.to_string(),
        })?;

        let current_version: i32 = conn
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |r| r.get(0),
            )
            .map_err(|e| MaintainerError::Internal {
                context: "reading current schema version".into(),
                message: e.to_string(),
            })?;

        for &(version, sql) in MIGRATIONS {
            if version <= current_version {
                continue;
            }

            let tx = conn.transaction().map_err(|e| MaintainerError::Internal {
                context: format!("beginning transaction for migration {version}"),
                message: e.to_string(),
            })?;

            tx.execute_batch(sql)
                .map_err(|e| MaintainerError::Internal {
                    context: format!("applying migration {version}"),
                    message: e.to_string(),
                })?;

            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs() as i64;

            tx.execute(
                "INSERT INTO schema_migrations (version, applied_at) VALUES (?1, ?2)",
                rusqlite::params![version, now],
            )
            .map_err(|e| MaintainerError::Internal {
                context: format!("recording migration {version}"),
                message: e.to_string(),
            })?;

            tx.commit().map_err(|e| MaintainerError::Internal {
                context: format!("committing migration {version}"),
                message: e.to_string(),
            })?;
        }

        Ok(())
    })
}

/// Return the current schema version, or 0 if no migrations have been applied.
pub fn current_version(db: &Database) -> Result<i32, MaintainerError> {
    db.with_connection(|conn| {
        conn.query_row(
            "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
            [],
            |r| r.get(0),
        )
        .map_err(|e| MaintainerError::Internal {
            context: "reading schema version".into(),
            message: e.to_string(),
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migrations_apply_to_fresh_database() {
        let db = Database::open_in_memory().unwrap();
        let version = current_version(&db).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn migrations_are_idempotent() {
        let db = Database::open_in_memory().unwrap();
        // apply_all was already called in open_in_memory; call again.
        apply_all(&db).unwrap();
        let version = current_version(&db).unwrap();
        assert_eq!(version, 1);
    }

    #[test]
    fn tables_exist_after_migration() {
        let db = Database::open_in_memory().unwrap();
        let tables: Vec<String> = db
            .with_connection(|conn| {
                let mut stmt = conn
                    .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                    .map_err(|e| MaintainerError::Internal {
                        context: "listing tables".into(),
                        message: e.to_string(),
                    })?;
                let rows = stmt.query_map([], |row| row.get(0)).map_err(|e| {
                    MaintainerError::Internal {
                        context: "querying tables".into(),
                        message: e.to_string(),
                    }
                })?;
                let mut result = Vec::new();
                for r in rows {
                    result.push(r.map_err(|e| MaintainerError::Internal {
                        context: "reading table name".into(),
                        message: e.to_string(),
                    })?);
                }
                Ok(result)
            })
            .unwrap();

        assert!(tables.contains(&"settings".to_string()));
        assert!(tables.contains(&"operation_history".to_string()));
        assert!(tables.contains(&"scan_metadata".to_string()));
        assert!(tables.contains(&"diagnostic_history".to_string()));
        assert!(tables.contains(&"schema_migrations".to_string()));
    }
}
