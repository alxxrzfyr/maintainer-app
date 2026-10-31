//! Database opening, configuration, and XDG path resolution.

use std::fs;
use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use crate::error::MaintainerError;
use crate::persistence::migrations;
use crate::persistence::sqlite_snapshot::{self, MAX_BACKUP_BYTES};

fn backup_error(error: rusqlite::Error) -> MaintainerError {
    MaintainerError::Internal {
        context: "copying or validating local database".into(),
        message: error.to_string(),
    }
}

// Pin the verified directory descriptor so ancestor renames cannot substitute
// an unrelated path between exclusive creation and SQLite opening the file.
fn private_database_location(path: &Path) -> Result<(File, PathBuf), MaintainerError> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| {
            MaintainerError::invalid_data("backup path", "an absolute file path is required")
        })?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .ok_or_else(|| {
            MaintainerError::invalid_data("backup path", "a UTF-8 file name is required")
        })?;
    validate_absolute_directory(path)?;
    let directory = open_directory_chain(parent, false, false)?;
    let pinned = PathBuf::from(format!("/proc/self/fd/{}", directory.as_raw_fd())).join(name);
    Ok((directory, pinned))
}

fn verify_database_identity(file: &File, pinned: &Path) -> Result<(), MaintainerError> {
    let held = file
        .metadata()
        .map_err(|error| MaintainerError::io("checking backup file", error))?;
    let opened = fs::symlink_metadata(pinned)
        .map_err(|error| MaintainerError::io("verifying backup file", error))?;
    if !held.is_file()
        || held.uid() != current_uid()
        || held.nlink() != 1
        || held.mode() & 0o077 != 0
        || held.dev() != opened.dev()
        || held.ino() != opened.ino()
        || opened.file_type().is_symlink()
    {
        return Err(MaintainerError::invalid_data(
            "backup file",
            "a private regular file owned by the current user is required",
        ));
    }
    Ok(())
}

fn reject_backup_sidecars(pinned: &Path) -> Result<(), MaintainerError> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let sidecar = PathBuf::from(format!("{}{suffix}", pinned.display()));
        match fs::symlink_metadata(sidecar) {
            Ok(_) => {
                return Err(MaintainerError::invalid_data(
                    "backup file",
                    "SQLite sidecars already exist",
                ));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(MaintainerError::io("checking backup sidecars", error)),
        }
    }
    Ok(())
}

fn open_new_backup_file(path: &Path) -> Result<(File, File, PathBuf), MaintainerError> {
    let (directory, pinned) = private_database_location(path)?;
    reject_backup_sidecars(&pinned)?;
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .mode(DATABASE_FILE_MODE)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
        .open(&pinned)
        .map_err(|error| MaintainerError::io("creating new private backup", error))?;
    verify_database_identity(&file, &pinned)?;
    Ok((file, directory, pinned))
}

fn read_backup_snapshot(path: &Path) -> Result<Connection, MaintainerError> {
    let (directory, pinned) = private_database_location(path)?;
    // Only immutable standalone backups are supported, never a live WAL/journal.
    reject_backup_sidecars(&pinned)?;
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(&pinned)
        .map_err(|error| MaintainerError::io("opening backup read-only", error))?;
    verify_database_identity(&file, &pinned)?;
    let length = file
        .metadata()
        .map_err(|error| MaintainerError::io("checking backup size", error))?
        .len();
    if length > MAX_BACKUP_BYTES as u64 {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "backups larger than 64 MiB are unsupported",
        ));
    }
    let mut bytes = Vec::new();
    (&mut file)
        .take(MAX_BACKUP_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| MaintainerError::io("reading backup", error))?;
    verify_database_identity(&file, &pinned)?;
    drop(directory);
    deserialize_backup(&bytes)
}

fn deserialize_backup(bytes: &[u8]) -> Result<Connection, MaintainerError> {
    if bytes.len() < 100
        || bytes.len() > MAX_BACKUP_BYTES
        || &bytes[..16] != b"SQLite format 3\0"
        || bytes[18] != 1
        || bytes[19] != 1
    {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "a standalone SQLite backup of at most 64 MiB is required",
        ));
    }
    let connection = sqlite_snapshot::deserialize_read_only(bytes)?;
    validate_backup(&connection)?;
    Ok(connection)
}

fn publish_backup(
    file: &mut File,
    directory: &File,
    pinned: &Path,
    bytes: &[u8],
) -> Result<(), MaintainerError> {
    verify_database_identity(file, pinned)?;
    // Never ask SQLite to reopen a mutable pathname: writes use only the
    // exclusive file descriptor. A leaf substitution cannot touch its target.
    if bytes.len() < 100 {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "serialized SQLite header is incomplete",
        ));
    }
    // Online copying can retain a WAL source's header mode in memory. SQLite
    // documents converting header bytes 18/19 to rollback mode for standalone
    // serialized copies: https://www.sqlite.org/c3ref/deserialize.html
    file.write_all(&bytes[..18])
        .and_then(|()| file.write_all(&[1, 1]))
        .and_then(|()| file.write_all(&bytes[20..]))
        .map_err(|error| MaintainerError::io("writing private backup", error))?;
    file.sync_all()
        .map_err(|error| MaintainerError::io("syncing backup file", error))?;
    verify_database_identity(file, pinned)?;
    directory
        .sync_all()
        .map_err(|error| MaintainerError::io("syncing backup directory", error))
}

fn export_backup(source: &Connection, destination: &Path) -> Result<(), MaintainerError> {
    let mut snapshot = Connection::open_in_memory().map_err(backup_error)?;
    copy_database(source, &mut snapshot)?;
    validate_backup(&snapshot)?;
    let serialized = sqlite_snapshot::serialize(&snapshot)?;
    let bytes = serialized.as_ref();
    if bytes.len() > MAX_BACKUP_BYTES {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "backups larger than 64 MiB are unsupported",
        ));
    }
    let (mut file, directory, pinned) = open_new_backup_file(destination)?;
    publish_backup(&mut file, &directory, &pinned, bytes)?;
    let (published_directory, published_path) = private_database_location(destination)?;
    let held = directory
        .metadata()
        .map_err(|error| MaintainerError::io("checking held backup directory", error))?;
    let published = published_directory
        .metadata()
        .map_err(|error| MaintainerError::io("checking published backup directory", error))?;
    if held.dev() != published.dev() || held.ino() != published.ino() {
        return Err(MaintainerError::invalid_data(
            "backup destination",
            "the directory changed before publication",
        ));
    }
    verify_database_identity(&file, &published_path)
}

fn copy_database(source: &Connection, target: &mut Connection) -> Result<(), MaintainerError> {
    let page_size: usize = source
        .pragma_query_value(None, "page_size", |row| row.get(0))
        .map_err(backup_error)?;
    let page_count: usize = source
        .pragma_query_value(None, "page_count", |row| row.get(0))
        .map_err(backup_error)?;
    if page_size == 0 || page_count > MAX_BACKUP_BYTES / page_size {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "backups larger than 64 MiB are unsupported",
        ));
    }
    target
        .pragma_update(None, "page_size", page_size)
        .map_err(backup_error)?;
    target
        .pragma_update(None, "max_page_count", MAX_BACKUP_BYTES / page_size)
        .map_err(backup_error)?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    target
        .pragma_update(None, "journal_mode", "MEMORY")
        .map_err(backup_error)?;
    target
        .pragma_update(None, "synchronous", "FULL")
        .map_err(backup_error)?;
    let backup = rusqlite::backup::Backup::new(source, target).map_err(backup_error)?;
    loop {
        if std::time::Instant::now() >= deadline {
            return Err(MaintainerError::invalid_data(
                "database backup",
                "copy timed out; destination is unverified",
            ));
        }
        let step = backup.step(128).map_err(backup_error)?;
        if backup.progress().pagecount < 0
            || backup.progress().pagecount as usize > MAX_BACKUP_BYTES / page_size
        {
            return Err(MaintainerError::invalid_data(
                "database backup",
                "backup grew beyond the 64 MiB limit",
            ));
        }
        match step {
            rusqlite::backup::StepResult::Done => return Ok(()),
            rusqlite::backup::StepResult::More => {}
            _ => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
}

fn database_schema(
    connection: &Connection,
) -> Result<Vec<(String, String, String)>, MaintainerError> {
    let mut statement = connection.prepare("SELECT type, name, sql FROM sqlite_schema WHERE name NOT GLOB 'sqlite_*' ORDER BY type, name").map_err(backup_error)?;
    statement
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(backup_error)?
        .collect::<Result<Vec<_>, _>>()
        .map_err(backup_error)
}

fn validate_backup(connection: &Connection) -> Result<(), MaintainerError> {
    let integrity: String = connection
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .map_err(backup_error)?;
    if integrity != "ok" {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "integrity check failed",
        ));
    }
    let expected = Database::open_in_memory()?;
    expected.with_connection(|reference| {
        if database_schema(connection)? != database_schema(reference)? {
            return Err(MaintainerError::invalid_data(
                "database backup",
                "schema is unsupported or was modified",
            ));
        }
        let version: i32 = connection
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(backup_error)?;
        let supported: i32 = reference
            .query_row(
                "SELECT COALESCE(MAX(version), 0) FROM schema_migrations",
                [],
                |row| row.get(0),
            )
            .map_err(backup_error)?;
        if version != supported {
            return Err(MaintainerError::invalid_data(
                "database backup",
                "schema is unsupported or was modified",
            ));
        }
        Ok(())
    })
}

/// Busy timeout in milliseconds for SQLite connections.
const BUSY_TIMEOUT_MS: u32 = 5_000;
const APP_DIRECTORY_MODE: u32 = 0o700;
const DATABASE_FILE_MODE: u32 = 0o600;

/// Wraps a SQLite connection with application-level configuration applied.
///
/// The connection is behind a `Mutex` so that `Database` is `Send + Sync`,
/// enabling repositories to satisfy trait bounds for use across threads.
pub struct Database {
    conn: Mutex<Connection>,
    // Retain the verified directory inode while SQLite is using its path.
    _data_dir: Option<File>,
}

impl Database {
    /// Open (or create) the database at the default XDG data path.
    ///
    /// Creates the parent directory if needed, applies user-only permissions
    /// to the database file, enables foreign keys, sets busy timeout, and
    /// runs pending migrations.
    pub fn open_default() -> Result<Self, MaintainerError> {
        let path = default_database_path()?;
        let parent = path.parent().ok_or_else(|| {
            MaintainerError::invalid_data("database path", "a parent directory is required")
        })?;
        validate_absolute_directory(parent)?;
        let _private_dir = open_directory_chain(parent, true, true)?;
        Self::open(&path)
    }

    /// Open (or create) a database at an explicit path.
    ///
    /// Useful for testing with specific file locations.
    pub fn open(path: &Path) -> Result<Self, MaintainerError> {
        let parent = path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
            .ok_or_else(|| {
                MaintainerError::invalid_data("database path", "an absolute file path is required")
            })?;
        let _file_name = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|name| !matches!(*name, "." | ".."))
            .ok_or_else(|| {
                MaintainerError::invalid_data(
                    "database path",
                    "a valid UTF-8 file name is required",
                )
            })?;
        validate_absolute_directory(parent)?;

        let data_dir = open_directory_chain(parent, false, false)?;
        secure_existing_sidecars(&data_dir, _file_name)?;
        let database_file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .mode(DATABASE_FILE_MODE)
            .custom_flags(libc::O_CLOEXEC | libc::O_NOFOLLOW)
            .open(path)
            .map_err(|e| MaintainerError::io("opening private database file", e))?;
        let metadata = database_file
            .metadata()
            .map_err(|e| MaintainerError::io("checking database file", e))?;
        if !metadata.file_type().is_file()
            || metadata.uid() != current_uid()
            || metadata.nlink() != 1
        {
            return Err(MaintainerError::invalid_data(
                "database file",
                "the target must be an unlinked regular file owned by the current user",
            ));
        }
        if metadata.mode() & 0o200 == 0 {
            return Err(MaintainerError::io(
                "opening read-only database",
                std::io::Error::new(
                    std::io::ErrorKind::PermissionDenied,
                    "database owner write permission is required",
                ),
            ));
        }
        set_fd_mode(&database_file, DATABASE_FILE_MODE, "securing database file")?;

        let conn = Connection::open_with_flags(
            path,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_WRITE
                | rusqlite::OpenFlags::SQLITE_OPEN_CREATE
                | rusqlite::OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )
        .map_err(|e| MaintainerError::Internal {
            context: "opening database".into(),
            message: e.to_string(),
        })?;
        let opened_metadata = fs::symlink_metadata(path)
            .map_err(|e| MaintainerError::io("verifying opened database file", e))?;
        if opened_metadata.dev() != metadata.dev()
            || opened_metadata.ino() != metadata.ino()
            || opened_metadata.uid() != current_uid()
            || opened_metadata.nlink() != 1
            || opened_metadata.mode() & 0o777 != DATABASE_FILE_MODE
        {
            return Err(MaintainerError::invalid_data(
                "database file",
                "the target changed while opening",
            ));
        }
        let opened_parent_metadata = fs::symlink_metadata(parent)
            .map_err(|e| MaintainerError::io("verifying application data directory", e))?;
        let held_parent_metadata = data_dir
            .metadata()
            .map_err(|e| MaintainerError::io("checking held data directory", e))?;
        if opened_parent_metadata.file_type().is_symlink()
            || opened_parent_metadata.dev() != held_parent_metadata.dev()
            || opened_parent_metadata.ino() != held_parent_metadata.ino()
        {
            return Err(MaintainerError::invalid_data(
                "data directory",
                "the directory changed while opening",
            ));
        }

        Self::configure_and_migrate(conn, Some(data_dir))
    }

    /// Create a consistent SQLite backup at a new private destination.
    ///
    /// Call only from a background worker: copying and integrity checks perform
    /// blocking I/O. Snapshots over 64 MiB are unsupported. Existing
    /// destinations are never opened or replaced. On
    /// failure a private, unverified destination may remain; use a new path.
    pub fn backup_to_new(&self, destination: &Path) -> Result<(), MaintainerError> {
        self.with_connection(|source| export_backup(source, destination))
    }

    /// Recover a verified standalone backup into a NEW private file, preserving
    /// the source and every existing destination. The source must have no live
    /// SQLite sidecars; inputs over 64 MiB are unsupported. Call from a worker.
    pub fn restore_backup_to_new(backup: &Path, destination: &Path) -> Result<(), MaintainerError> {
        let source = read_backup_snapshot(backup)?;
        export_backup(&source, destination)
    }

    /// Open an in-memory database for testing.
    ///
    /// Applies the same pragmas and migrations as a file-backed database.
    pub fn open_in_memory() -> Result<Self, MaintainerError> {
        let conn = Connection::open_in_memory().map_err(|e| MaintainerError::Internal {
            context: "opening in-memory database".into(),
            message: e.to_string(),
        })?;
        Self::configure_and_migrate(conn, None)
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

    /// Delete all locally stored operation, diagnostic, and scan history.
    pub fn clear_history(&self) -> Result<u64, MaintainerError> {
        self.with_connection_mut(|conn| {
            let transaction = conn
                .transaction()
                .map_err(|error| MaintainerError::Internal {
                    context: "starting history deletion transaction".into(),
                    message: error.to_string(),
                })?;
            let mut deleted = 0_u64;
            for table in ["operation_history", "diagnostic_history", "scan_metadata"] {
                deleted = deleted.saturating_add(
                    transaction
                        .execute(&format!("DELETE FROM {table}"), [])
                        .map_err(|error| MaintainerError::Internal {
                            context: "deleting local history".into(),
                            message: error.to_string(),
                        })? as u64,
                );
            }
            transaction
                .commit()
                .map_err(|error| MaintainerError::Internal {
                    context: "committing history deletion".into(),
                    message: error.to_string(),
                })?;
            Ok(deleted)
        })
    }

    /// Delete all user-owned application rows while preserving the schema.
    pub fn clear_user_data(&self) -> Result<u64, MaintainerError> {
        self.with_connection_mut(|conn| {
            let transaction = conn
                .transaction()
                .map_err(|error| MaintainerError::Internal {
                    context: "starting application data deletion transaction".into(),
                    message: error.to_string(),
                })?;
            let mut deleted = 0_u64;
            for table in [
                "operation_history",
                "diagnostic_history",
                "scan_metadata",
                "settings",
            ] {
                deleted = deleted.saturating_add(
                    transaction
                        .execute(&format!("DELETE FROM {table}"), [])
                        .map_err(|error| MaintainerError::Internal {
                            context: "deleting application data".into(),
                            message: error.to_string(),
                        })? as u64,
                );
            }
            transaction
                .commit()
                .map_err(|error| MaintainerError::Internal {
                    context: "committing application data deletion".into(),
                    message: error.to_string(),
                })?;
            Ok(deleted)
        })
    }

    /// Apply pragmas and run migrations.
    fn configure_and_migrate(
        conn: Connection,
        data_dir: Option<File>,
    ) -> Result<Self, MaintainerError> {
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
            _data_dir: data_dir,
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
    resolve_database_path(std::env::var_os("XDG_DATA_HOME"), std::env::var_os("HOME"))
}

fn resolve_database_path(
    xdg_data_home: Option<std::ffi::OsString>,
    home: Option<std::ffi::OsString>,
) -> Result<PathBuf, MaintainerError> {
    let data_dir = match xdg_data_home.filter(|value| !value.is_empty()) {
        Some(value) => {
            let path = PathBuf::from(value);
            validate_absolute_directory(&path)?;
            path
        }
        None => {
            let home = home
                .filter(|value| !value.is_empty())
                .map(PathBuf::from)
                .ok_or_else(|| {
                    MaintainerError::invalid_data(
                        "XDG data directory",
                        "XDG_DATA_HOME or HOME must be set",
                    )
                })?;
            validate_absolute_directory(&home)?;
            home.join(".local").join("share")
        }
    };

    Ok(data_dir.join("maintainer").join("maintainer.db"))
}

fn validate_absolute_directory(path: &Path) -> Result<(), MaintainerError> {
    if !path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                std::path::Component::CurDir | std::path::Component::ParentDir
            )
        })
    {
        return Err(MaintainerError::invalid_data(
            "data directory",
            "an absolute normalized path is required",
        ));
    }
    Ok(())
}

fn open_directory_chain(
    path: &Path,
    create_missing: bool,
    secure_final: bool,
) -> Result<File, MaintainerError> {
    validate_absolute_directory(path)?;
    let mut directory =
        File::open("/").map_err(|error| MaintainerError::io("opening filesystem root", error))?;

    for component in path.components().skip(1) {
        let std::path::Component::Normal(name) = component else {
            return Err(MaintainerError::invalid_data(
                "data directory",
                "only normalized directory paths are supported",
            ));
        };
        let name = std::ffi::CString::new(name.as_bytes()).map_err(|_| {
            MaintainerError::invalid_data("data directory", "invalid directory component")
        })?;
        let mut fd = open_directory_at(directory.as_raw_fd(), &name);
        if fd.is_err() && create_missing {
            // SAFETY: `directory` is an open directory descriptor and `name`
            // is a NUL-terminated single path component.
            let result = unsafe {
                libc::mkdirat(
                    directory.as_raw_fd(),
                    name.as_ptr(),
                    APP_DIRECTORY_MODE as libc::mode_t,
                )
            };
            if result == 0
                || std::io::Error::last_os_error().kind() == std::io::ErrorKind::AlreadyExists
            {
                fd = open_directory_at(directory.as_raw_fd(), &name);
            }
        }
        let fd =
            fd.map_err(|error| MaintainerError::io("opening data directory component", error))?;
        // SAFETY: openat returned a new owned descriptor on success.
        directory = unsafe { File::from(OwnedFd::from_raw_fd(fd)) };
    }

    let metadata = directory
        .metadata()
        .map_err(|error| MaintainerError::io("checking application data directory", error))?;
    if !metadata.is_dir() || metadata.uid() != current_uid() {
        return Err(MaintainerError::invalid_data(
            "data directory",
            "the directory must be owned by the current user",
        ));
    }
    if secure_final {
        set_fd_mode(
            &directory,
            APP_DIRECTORY_MODE,
            "securing application data directory",
        )?;
    } else if metadata.mode() & 0o077 != 0 {
        return Err(MaintainerError::invalid_data(
            "data directory",
            "the directory must be private to the current user",
        ));
    }
    Ok(directory)
}

fn open_directory_at(parent_fd: i32, name: &std::ffi::CStr) -> std::io::Result<i32> {
    // SAFETY: `parent_fd` is an open directory descriptor and `name` is a
    // NUL-terminated single component; flags reject symlinks and non-directories.
    let fd = unsafe {
        libc::openat(
            parent_fd,
            name.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC | libc::O_NOFOLLOW,
        )
    };
    if fd == -1 {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn secure_existing_sidecars(directory: &File, database_name: &str) -> Result<(), MaintainerError> {
    for suffix in ["-journal", "-wal", "-shm"] {
        let name = std::ffi::CString::new(format!("{database_name}{suffix}")).map_err(|_| {
            MaintainerError::invalid_data("database sidecar", "invalid sidecar name")
        })?;
        // SAFETY: directory is a verified open directory and name is one
        // NUL-terminated SQLite sidecar component.
        let fd = unsafe {
            libc::openat(
                directory.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_CLOEXEC | libc::O_NOFOLLOW | libc::O_NONBLOCK,
            )
        };
        if fd == -1 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::NotFound {
                continue;
            }
            return Err(MaintainerError::io("opening SQLite sidecar", error));
        }
        // SAFETY: openat returned a newly owned descriptor on success.
        let file = unsafe { File::from(OwnedFd::from_raw_fd(fd)) };
        let metadata = file
            .metadata()
            .map_err(|error| MaintainerError::io("checking SQLite sidecar", error))?;
        if !metadata.is_file() || metadata.uid() != current_uid() || metadata.nlink() != 1 {
            return Err(MaintainerError::invalid_data(
                "database sidecar",
                "sidecars must be unlinked regular files owned by the current user",
            ));
        }
        set_fd_mode(&file, DATABASE_FILE_MODE, "securing SQLite sidecar")?;
    }
    Ok(())
}

fn set_fd_mode(file: &File, mode: u32, context: &'static str) -> Result<(), MaintainerError> {
    // SAFETY: the descriptor is valid for the lifetime of `file`; fchmod only
    // changes permissions on the opened inode and cannot follow a path swap.
    let result = unsafe { libc::fchmod(file.as_raw_fd(), mode as libc::mode_t) };
    if result == -1 {
        return Err(MaintainerError::io(
            context,
            std::io::Error::last_os_error(),
        ));
    }
    Ok(())
}

fn current_uid() -> u32 {
    // SAFETY: geteuid has no pointer parameters and is safe to call here.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    #[test]
    fn open_creates_database_file() {
        let tmp = TempDir::new().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
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
    fn open_rejects_symlink_database_without_chmodding_target() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("unrelated.db");
        let link = tmp.path().join("maintainer.db");
        fs::write(&target, "preserve me").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, &link).unwrap();

        assert!(Database::open(&link).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "preserve me");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o644);
    }

    #[test]
    fn open_rejects_hard_link_without_chmodding_target() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("unrelated.db");
        let link = tmp.path().join("maintainer.db");
        fs::write(&target, "preserve me").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        fs::hard_link(&target, &link).unwrap();

        assert!(Database::open(&link).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "preserve me");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o644);
    }

    #[test]
    fn open_rejects_symlinked_sidecar_without_chmodding_target() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let target = tmp.path().join("unrelated.wal");
        let sidecar = tmp.path().join("maintainer.db-wal");
        fs::write(&target, "preserve me").unwrap();
        fs::set_permissions(&target, fs::Permissions::from_mode(0o644)).unwrap();
        symlink(&target, &sidecar).unwrap();

        assert!(Database::open(&tmp.path().join("maintainer.db")).is_err());
        assert_eq!(fs::read_to_string(&target).unwrap(), "preserve me");
        assert_eq!(fs::metadata(&target).unwrap().mode() & 0o777, 0o644);
    }

    #[test]
    fn open_rejects_symlink_directory_component() {
        use std::os::unix::fs::symlink;

        let tmp = TempDir::new().unwrap();
        let target_dir = tmp.path().join("target");
        let link_dir = tmp.path().join("linked");
        fs::create_dir(&target_dir).unwrap();
        fs::set_permissions(&target_dir, fs::Permissions::from_mode(0o700)).unwrap();
        symlink(&target_dir, &link_dir).unwrap();

        assert!(Database::open(&link_dir.join("test.db")).is_err());
        assert!(!target_dir.join("test.db").exists());
    }

    #[test]
    fn corrupt_database_is_preserved_on_open_failure() {
        let tmp = TempDir::new().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = tmp.path().join("corrupt.db");
        let contents = b"not a sqlite database";
        fs::write(&path, contents).unwrap();

        assert!(Database::open(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), contents);
    }

    #[test]
    fn read_only_database_is_preserved_on_open_failure() {
        let tmp = TempDir::new().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let path = tmp.path().join("readonly.db");
        let contents = b"existing read-only data";
        fs::write(&path, contents).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o400)).unwrap();

        assert!(Database::open(&path).is_err());
        assert_eq!(fs::read(&path).unwrap(), contents);
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o400);
    }

    #[test]
    fn open_rejects_permissive_parent_without_chmodding_it() {
        let tmp = TempDir::new().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o755)).unwrap();

        assert!(Database::open(&tmp.path().join("test.db")).is_err());
        assert_eq!(fs::metadata(tmp.path()).unwrap().mode() & 0o777, 0o755);
    }

    #[test]
    fn default_path_uses_xdg_or_home_fallback() {
        let home = std::ffi::OsString::from("/home/tester");
        assert_eq!(
            resolve_database_path(
                Some(std::ffi::OsString::from("/data/custom")),
                Some(home.clone()),
            )
            .unwrap(),
            PathBuf::from("/data/custom/maintainer/maintainer.db")
        );
        for empty_xdg in [None, Some(std::ffi::OsString::new())] {
            assert_eq!(
                resolve_database_path(empty_xdg, Some(home.clone())).unwrap(),
                PathBuf::from("/home/tester/.local/share/maintainer/maintainer.db")
            );
        }
    }

    #[test]
    fn default_path_rejects_relative_xdg_and_home_values() {
        assert!(
            resolve_database_path(
                Some(std::ffi::OsString::from("relative/data")),
                Some(std::ffi::OsString::from("/home/tester")),
            )
            .is_err()
        );
        assert!(
            resolve_database_path(None, Some(std::ffi::OsString::from("relative-home")),).is_err()
        );
        assert!(resolve_database_path(None, None).is_err());
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
    fn clear_history_preserves_settings_and_schema() {
        let db = Database::open_in_memory().unwrap();
        db.with_connection(|conn| {
            conn.execute("INSERT INTO settings (key, value) VALUES ('theme', 'dark')", [])
                .map_err(|error| MaintainerError::Internal {
                    context: "seeding settings".into(),
                    message: error.to_string(),
                })?;
            conn.execute(
                "INSERT INTO operation_history (id, operation, started_at, status) VALUES ('1', 'cleanup', 1, 'success')",
                [],
            )
            .map_err(|error| MaintainerError::Internal {
                context: "seeding history".into(),
                message: error.to_string(),
            })?;
            Ok(())
        })
        .unwrap();

        assert_eq!(db.clear_history().unwrap(), 1);
        db.with_connection(|conn| {
            let settings: i64 = conn
                .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
                .unwrap();
            let migrations: i64 = conn
                .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(settings, 1);
            assert!(migrations >= 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn clear_user_data_preserves_migrations() {
        let db = Database::open_in_memory().unwrap();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('theme', 'dark')",
                [],
            )
            .map_err(|error| MaintainerError::Internal {
                context: "seeding settings".into(),
                message: error.to_string(),
            })?;
            Ok(())
        })
        .unwrap();

        assert_eq!(db.clear_user_data().unwrap(), 1);
        db.with_connection(|conn| {
            let settings: i64 = conn
                .query_row("SELECT COUNT(*) FROM settings", [], |row| row.get(0))
                .unwrap();
            let migrations: i64 = conn
                .query_row("SELECT COUNT(*) FROM schema_migrations", [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(settings, 0);
            assert!(migrations >= 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn reopen_existing_database_is_idempotent() {
        let tmp = TempDir::new().unwrap();
        fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o700)).unwrap();
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
    fn seed_backup_data(database: &Database) {
        database.with_connection(|connection| {
            connection.execute_batch("INSERT INTO settings VALUES ('theme', 'dark'); INSERT INTO operation_history (id, operation, started_at, status) VALUES ('backup-test', 'scan', 1, 'success');")
                .map_err(backup_error)
        }).unwrap();
    }

    fn assert_backup_data(database: &Database) {
        database
            .with_connection(|connection| {
                let theme: String = connection
                    .query_row(
                        "SELECT value FROM settings WHERE key = 'theme'",
                        [],
                        |row| row.get(0),
                    )
                    .map_err(backup_error)?;
                let history: i64 = connection
                    .query_row("SELECT COUNT(*) FROM operation_history", [], |row| {
                        row.get(0)
                    })
                    .map_err(backup_error)?;
                assert_eq!(theme, "dark");
                assert_eq!(history, 1);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn online_backup_roundtrip_preserves_settings_history_and_source() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let source_path = directory.path().join("source.db");
        let backup_path = directory.path().join("backup.db");
        let restored_path = directory.path().join("restored.db");
        let source = Database::open(&source_path).unwrap();
        seed_backup_data(&source);
        source
            .with_connection(|connection| {
                connection
                    .pragma_update(None, "journal_mode", "WAL")
                    .map_err(backup_error)
            })
            .unwrap();
        source.backup_to_new(&backup_path).unwrap();
        let backup_before = fs::read(&backup_path).unwrap();
        Database::restore_backup_to_new(&backup_path, &restored_path).unwrap();
        let restored = Database::open(&restored_path).unwrap();
        assert_backup_data(&restored);
        assert_backup_data(&source);
        assert_eq!(fs::read(&backup_path).unwrap(), backup_before);
        assert_eq!(fs::metadata(&backup_path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::metadata(&restored_path).unwrap().mode() & 0o777, 0o600);
    }

    #[test]
    fn backup_and_recovery_collisions_never_replace_files() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let source = Database::open_in_memory().unwrap();
        seed_backup_data(&source);
        let backup = directory.path().join("backup.db");
        source.backup_to_new(&backup).unwrap();
        let collision = directory.path().join("existing.db");
        fs::write(&collision, b"preserve unrelated file").unwrap();
        assert!(source.backup_to_new(&collision).is_err());
        assert!(Database::restore_backup_to_new(&backup, &collision).is_err());
        assert_eq!(fs::read(&collision).unwrap(), b"preserve unrelated file");
        let link = directory.path().join("linked.db");
        std::os::unix::fs::symlink(&collision, &link).unwrap();
        assert!(source.backup_to_new(&link).is_err());
        assert!(Database::restore_backup_to_new(&backup, &link).is_err());
        assert_eq!(fs::read(&collision).unwrap(), b"preserve unrelated file");
        assert_backup_data(&source);
    }

    #[test]
    fn recovery_rejects_corrupt_newer_and_modified_schema_without_touching_source() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let backup = directory.path().join("bad.db");
        let destination = directory.path().join("restore.db");
        fs::write(&backup, b"not a SQLite backup").unwrap();
        fs::set_permissions(&backup, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(Database::restore_backup_to_new(&backup, &destination).is_err());
        assert!(!destination.exists());
        assert_eq!(fs::read(&backup).unwrap(), b"not a SQLite backup");
        fs::remove_file(&backup).unwrap();
        let newer = Database::open(&backup).unwrap();
        seed_backup_data(&newer);
        newer
            .with_connection(|connection| {
                connection
                    .execute("INSERT INTO schema_migrations VALUES (999, 0)", [])
                    .map(|_| ())
                    .map_err(backup_error)
            })
            .unwrap();
        drop(newer);
        let before = fs::read(&backup).unwrap();
        let error = Database::restore_backup_to_new(&backup, &destination)
            .err()
            .unwrap();
        assert!(error.to_string().contains("schema"));
        assert!(!destination.exists());
        assert_eq!(fs::read(&backup).unwrap(), before);
        let connection = Connection::open(&backup).unwrap();
        connection.execute_batch("DELETE FROM schema_migrations WHERE version = 999; CREATE TRIGGER changed AFTER INSERT ON settings BEGIN DELETE FROM operation_history; END;").unwrap();
        drop(connection);
        assert!(Database::restore_backup_to_new(&backup, &destination).is_err());
        assert!(!destination.exists());
    }

    #[test]
    fn backup_excludes_uncommitted_transaction_and_source_rollback_remains_intact() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let source_path = directory.path().join("source.db");
        let source = Database::open(&source_path).unwrap();
        seed_backup_data(&source);
        let mut writer = Connection::open(&source_path).unwrap();
        let transaction = writer.transaction().unwrap();
        transaction
            .execute(
                "UPDATE settings SET value = 'light' WHERE key = 'theme'",
                [],
            )
            .unwrap();
        let backup = directory.path().join("backup.db");
        source.backup_to_new(&backup).unwrap();
        drop(transaction);
        assert_backup_data(&source);
        let restored_path = directory.path().join("restored.db");
        Database::restore_backup_to_new(&backup, &restored_path).unwrap();
        let restored = Database::open(&restored_path).unwrap();
        assert_backup_data(&restored);
    }

    #[test]
    fn backup_rejects_public_directory_and_destination_sidecars() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let source = Database::open_in_memory().unwrap();
        let path = directory.path().join("backup.db");
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o755)).unwrap();
        assert!(source.backup_to_new(&path).is_err());
        assert!(!path.exists());
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::write(directory.path().join("backup.db-wal"), "preserve sidecar").unwrap();
        assert!(source.backup_to_new(&path).is_err());
        assert!(!path.exists());
        assert_eq!(
            fs::read(directory.path().join("backup.db-wal")).unwrap(),
            b"preserve sidecar"
        );
    }
    #[test]
    fn recovery_buffer_is_read_only_and_malformed_input_is_rejected() {
        let source = Database::open_in_memory().unwrap();
        seed_backup_data(&source);
        let bytes = source
            .with_connection(|connection| {
                sqlite_snapshot::serialize(connection).map(|bytes| bytes.as_ref().to_vec())
            })
            .unwrap();
        let restored = deserialize_backup(&bytes).unwrap();
        assert!(restored.execute("DELETE FROM settings", []).is_err());
        let mut malformed = vec![0; 100];
        malformed[..16].copy_from_slice(b"SQLite format 3\0");
        malformed[18] = 1;
        malformed[19] = 1;
        for _ in 0..8 {
            assert!(deserialize_backup(&malformed).is_err());
        }
        let mut wal = bytes;
        wal[18] = 2;
        wal[19] = 2;
        assert!(deserialize_backup(&wal).is_err());
    }

    #[test]
    fn oversized_recovery_is_bounded_and_does_not_create_destination() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let backup = directory.path().join("oversized.db");
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&backup)
            .unwrap();
        file.set_len(MAX_BACKUP_BYTES as u64 + 1).unwrap();
        let destination = directory.path().join("restored.db");
        assert!(Database::restore_backup_to_new(&backup, &destination).is_err());
        assert!(!destination.exists());
        assert_eq!(file.metadata().unwrap().len(), MAX_BACKUP_BYTES as u64 + 1);
    }

    #[test]
    fn publication_rejects_substituted_leaf_without_touching_unrelated_target() {
        let directory = TempDir::new().unwrap();
        fs::set_permissions(directory.path(), fs::Permissions::from_mode(0o700)).unwrap();
        let destination = directory.path().join("backup.db");
        let victim = directory.path().join("unrelated.txt");
        fs::write(&victim, "preserve unrelated data").unwrap();
        let (mut file, held_directory, pinned) = open_new_backup_file(&destination).unwrap();
        fs::rename(&destination, directory.path().join("original.db")).unwrap();
        std::os::unix::fs::symlink(&victim, &destination).unwrap();
        assert!(publish_backup(&mut file, &held_directory, &pinned, b"new bytes").is_err());
        assert_eq!(fs::read(&victim).unwrap(), b"preserve unrelated data");
        assert_eq!(file.metadata().unwrap().len(), 0);
    }
}
