//! SQLite 3.23 snapshot APIs without rusqlite's SQLite 3.38 error-offset feature.

use std::ffi::{c_char, c_int, c_uint};
use std::ptr::NonNull;

use rusqlite::Connection;

use crate::error::MaintainerError;

pub(super) const MAX_BACKUP_BYTES: usize = 64 * 1024 * 1024;

// The system library supplies these stable APIs on Ubuntu 22.04. Declaring
// only their ABI avoids enabling unrelated post-Jammy APIs in rusqlite.
unsafe extern "C" {
    fn sqlite3_serialize(
        database: *mut rusqlite::ffi::sqlite3,
        schema: *const c_char,
        size: *mut i64,
        flags: c_uint,
    ) -> *mut u8;
    fn sqlite3_deserialize(
        database: *mut rusqlite::ffi::sqlite3,
        schema: *const c_char,
        data: *mut u8,
        size: i64,
        capacity: i64,
        flags: c_uint,
    ) -> c_int;
}

pub(super) struct SerializedDatabase {
    pointer: NonNull<u8>,
    length: usize,
}

impl AsRef<[u8]> for SerializedDatabase {
    fn as_ref(&self) -> &[u8] {
        // SAFETY: successful serialization allocated length initialized bytes;
        // this object exclusively retains that allocation until Drop.
        unsafe { std::slice::from_raw_parts(self.pointer.as_ptr(), self.length) }
    }
}

impl Drop for SerializedDatabase {
    fn drop(&mut self) {
        // SAFETY: flag-zero sqlite3_serialize returns sqlite3_malloc64 storage.
        // The pointer is owned by this object and freed exactly once.
        unsafe { rusqlite::ffi::sqlite3_free(self.pointer.as_ptr().cast()) };
    }
}

pub(super) fn serialize(connection: &Connection) -> Result<SerializedDatabase, MaintainerError> {
    let mut length = 0_i64;
    // SAFETY: the live connection, static schema and writable length outlive
    // this synchronous call. Flag zero requests a separately owned allocation.
    let pointer =
        unsafe { sqlite3_serialize(connection.handle(), c"main".as_ptr(), &mut length, 0) };
    let pointer = NonNull::new(pointer).ok_or_else(|| {
        MaintainerError::invalid_data("database backup", "could not serialize database")
    })?;
    let mut data = SerializedDatabase { pointer, length: 0 };
    data.length = usize::try_from(length)
        .ok()
        .filter(|length| *length > 0 && *length <= MAX_BACKUP_BYTES)
        .ok_or_else(|| {
            MaintainerError::invalid_data("database backup", "invalid or oversized snapshot")
        })?;
    Ok(data)
}

pub(super) fn deserialize_read_only(bytes: &[u8]) -> Result<Connection, MaintainerError> {
    if bytes.is_empty() || bytes.len() > MAX_BACKUP_BYTES {
        return Err(MaintainerError::invalid_data(
            "database backup",
            "invalid snapshot size",
        ));
    }
    let connection = Connection::open_in_memory().map_err(snapshot_error)?;
    // SQLite advises padding untrusted snapshots. The size cap makes this
    // addition and the subsequent i64 conversions bounded on supported targets.
    let capacity = bytes.len() + 64;
    // SAFETY: the bounded nonzero allocation is checked before dereferencing.
    let pointer = unsafe { rusqlite::ffi::sqlite3_malloc64(capacity as u64) }.cast::<u8>();
    let pointer = NonNull::new(pointer).ok_or_else(|| {
        MaintainerError::invalid_data("database backup", "could not allocate backup buffer")
    })?;
    // SAFETY: the fresh allocation has capacity writable bytes, is disjoint
    // from bytes, and all padding is initialized before SQLite accesses it.
    unsafe {
        std::ptr::write_bytes(pointer.as_ptr(), 0, capacity);
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), pointer.as_ptr(), bytes.len());
    }
    const FREE_ON_CLOSE: c_uint = 1;
    const READ_ONLY: c_uint = 4;
    // SAFETY: connection exclusively owns a live SQLite handle. SQLite takes
    // this malloc allocation under FREEONCLOSE, including on failure. No Rust
    // reference accesses it after transfer. See sqlite.org/c3ref/deserialize.html.
    let status = unsafe {
        sqlite3_deserialize(
            connection.handle(),
            c"main".as_ptr(),
            pointer.as_ptr(),
            bytes.len() as i64,
            capacity as i64,
            FREE_ON_CLOSE | READ_ONLY,
        )
    };
    if status != rusqlite::ffi::SQLITE_OK {
        return Err(snapshot_error(rusqlite::Error::SqliteFailure(
            rusqlite::ffi::Error::new(status),
            None,
        )));
    }
    Ok(connection)
}

fn snapshot_error(error: rusqlite::Error) -> MaintainerError {
    MaintainerError::Internal {
        context: "opening read-only database snapshot".into(),
        message: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_round_trip_is_independent_and_read_only() {
        let source = Connection::open_in_memory().unwrap();
        source
            .execute_batch("CREATE TABLE example(value); INSERT INTO example VALUES (7)")
            .unwrap();
        let serialized = serialize(&source).unwrap();
        source.execute("DELETE FROM example", []).unwrap();
        let restored = deserialize_read_only(serialized.as_ref()).unwrap();
        drop(serialized);
        assert_eq!(
            restored
                .query_row("SELECT value FROM example", [], |row| row.get::<_, i64>(0))
                .unwrap(),
            7
        );
        assert!(restored.execute("DELETE FROM example", []).is_err());
    }

    #[test]
    fn invalid_snapshot_sizes_and_repeated_invalid_buffers_are_safe() {
        assert!(deserialize_read_only(&[]).is_err());
        for _ in 0..8 {
            let connection = deserialize_read_only(&[0; 100]).unwrap();
            assert!(
                connection
                    .query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
                    .is_err()
            );
        }
    }
}
