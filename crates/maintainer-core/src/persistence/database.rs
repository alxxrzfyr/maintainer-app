//! Database opening, configuration, and XDG path resolution.

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::MaintainerError;
use crate::persistence::migrations;

/// Busy timeout in milliseconds for SQLite connections.
const BUSY_TIMEOUT_MS: u32 = 5_000;

/// Wraps a SQLite connection with application-level configuration applied.
///
/// The connection is behind a `Mutex` so that `Database` is `Send + Sync`,
/// enabling repositories to satisfy trait bounds for use across threads.
pub struct Database {
    conn: Mutex<Connection>,
}

impl Database {
    /// Open (or create) the database at the default XDG data path.
    ///
    /// Creates the parent directory if needed, applies user-only permissions
    /// to the database file, enables foreign keys, sets busy timeout, and
    /// runs pending migrations.
    pub fn open_default() -> Result<Self, MaintainerError> {
        let path = default_database_path()?;
        Self::open(&path)
    }

    /// Open (or create) a database at an explicit path.
    ///
    /// Useful for testing with specific file locations.
    pub fn open(path: &Path) -> Result<Self, MaintainerError> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|e| {
                MaintainerError::io(format!("creating data directory {}", parent.display()), e)
            })?;
        }

        let conn = Connection::open(path).map_err(|e| MaintainerError::Internal {
            context: "opening database".into(),
            message: e.to_string(),
        })?;

        let perms = fs::Permissions::from_mode(0o600);
        fs::set_permissions(path, perms).map_err(|e| {
            MaintainerError::io(format!("setting permissions on {}", path.display()), e)
        })?;

        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory database for testing.
    ///
    /// Applies the same pragmas and migrations as a file-backed database.
    pub fn open_in_memory() -> Result<Self, MaintainerError> {
        let conn = Connection::open_in_memory().map_err(|e| MaintainerError::Internal {
            context: "opening in-memory database".into(),
            message: e.to_string(),
        })?;
        Self::configure_and_migrate(conn)
    }

    /// Execute a closure with a reference to the underlying connection.
    ///
    /// The Mutex ensures exclusive access; callers must not hold the lock
    /// across await points or long-running operations.
    pub fn with_connection<F, T>(&self, f: F) -> Result<T, MaintainerError>
    where
        F: FnOnce(&Connection) -> Result<T, MaintainerError>,
    {
        let conn = self.conn.lock().map_err(|_| MaintainerError::Internal {
            context: "database lock".into(),
            message: "mutex poisoned".into(),
        })?;
        f(&conn)
    }

    /// Execute a closure with a mutable reference to the underlying connection.
    ///
    /// Used internally for migrations that need transactions.
    pub(crate) fn with_connection_mut<F, T>(&self, f: F) -> Result<T, MaintainerError>
    where
        F: FnOnce(&mut Connection) -> Result<T, MaintainerError>,
    {
        let mut conn = self.conn.lock().map_err(|_| MaintainerError::Internal {
            context: "database lock".into(),
            message: "mutex poisoned".into(),
        })?;
        f(&mut conn)
    }

    /// Apply pragmas and run migrations.
    fn configure_and_migrate(conn: Connection) -> Result<Self, MaintainerError> {
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .map_err(|e| MaintainerError::Internal {
                context: "enabling foreign keys".into(),
                message: e.to_string(),
            })?;

        conn.busy_timeout(std::time::Duration::from_millis(u64::from(BUSY_TIMEOUT_MS)))
            .map_err(|e| MaintainerError::Internal {
                context: "setting busy timeout".into(),
                message: e.to_string(),
            })?;

        let db = Self {
            conn: Mutex::new(conn),
        };
        migrations::apply_all(&db)?;
        Ok(db)
    }
}

/// Resolve the default database file path using XDG conventions.
///
/// Uses `$XDG_DATA_HOME/maintainer/maintainer.db` if set,
/// otherwise `$HOME/.local/share/maintainer/maintainer.db`.
fn default_database_path() -> Result<PathBuf, MaintainerError> {
    let data_dir = if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
        PathBuf::from(xdg)
    } else if let Ok(home) = std::env::var("HOME") {
        PathBuf::from(home).join(".local").join("share")
    } else {
        return Err(MaintainerError::Internal {
            context: "resolving data directory".into(),
            message: "neither XDG_DATA_HOME nor HOME is set".into(),
        });
    };

    Ok(data_dir.join("maintainer").join("maintainer.db"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn open_creates_database_file() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");
        let db = Database::open(&db_path).unwrap();

        assert!(db_path.exists());
        let metadata = fs::metadata(&db_path).unwrap();
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);

        let fk: i32 = db
            .with_connection(|conn| {
                conn.query_row("PRAGMA foreign_keys", [], |r| r.get(0))
                    .map_err(|e| MaintainerError::Internal {
                        context: "pragma check".into(),
                        message: e.to_string(),
                    })
            })
            .unwrap();
        assert_eq!(fk, 1);
    }

    #[test]
    fn open_in_memory_applies_migrations() {
        let db = Database::open_in_memory().unwrap();
        let count: i32 = db
            .with_connection(|conn| {
                conn.query_row("SELECT COUNT(*) FROM schema_migrations", [], |r| r.get(0))
                    .map_err(|e| MaintainerError::Internal {
                        context: "count check".into(),
                        message: e.to_string(),
                    })
            })
            .unwrap();
        assert!(count >= 1);
    }

    #[test]
    fn reopen_existing_database_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        let db_path = tmp.path().join("test.db");

        let _db1 = Database::open(&db_path).unwrap();
        let db2 = Database::open(&db_path).unwrap();

        let version: i32 = db2
            .with_connection(|conn| {
                conn.query_row("SELECT MAX(version) FROM schema_migrations", [], |r| {
                    r.get(0)
                })
                .map_err(|e| MaintainerError::Internal {
                    context: "version check".into(),
                    message: e.to_string(),
                })
            })
            .unwrap();
        assert_eq!(version, 1);
    }
}
