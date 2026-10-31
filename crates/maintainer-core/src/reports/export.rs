//! Publish a private report without replacing any existing directory entry.

use std::ffi::CString;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

/// The complete file exists in both outcomes; only `Saved` confirms durability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportExportOutcome {
    Saved,
    DurabilityUnknown,
}

/// Save exactly this preview revision. Existing names are never overwritten.
/// Filesystems without anonymous temporary files or procfs linking are unsupported.
pub fn save_report_revision(
    path: &Path,
    content: &str,
    revision: &AtomicU64,
    expected: u64,
    commit_lock: &Mutex<()>,
) -> io::Result<ReportExportOutcome> {
    save_internal(
        path,
        content,
        revision,
        expected,
        commit_lock,
        || {},
        File::sync_all,
    )
}

fn save_internal(
    path: &Path,
    content: &str,
    revision: &AtomicU64,
    expected: u64,
    commit_lock: &Mutex<()>,
    before_commit: impl FnOnce(),
    sync_directory: impl FnOnce(&File) -> io::Result<()>,
) -> io::Result<ReportExportOutcome> {
    let check_revision = || {
        if revision.load(Ordering::Acquire) == expected {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "report preview changed before export",
            ))
        }
    };
    check_revision()?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid report name"))?;
    let name = CString::new(name.as_bytes())?;
    let directory =
        crate::storage::policy::open_anchored(path.parent().unwrap_or(Path::new(".")), true)?;
    // SAFETY: the directory is held for the entire operation; "." is a constant
    // terminated relative path. The returned anonymous inode has no mutable name.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            c".".as_ptr(),
            libc::O_TMPFILE | libc::O_WRONLY | libc::O_CLOEXEC,
            0o600 as libc::mode_t,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: successful openat returned a newly owned descriptor.
    let mut file = unsafe { File::from_raw_fd(fd) };
    file.write_all(content.as_bytes())?;
    file.sync_all()?;
    before_commit();
    // GTK regeneration only uses try_lock, so it cannot block on export I/O.
    let _guard = commit_lock
        .lock()
        .map_err(|_| io::Error::other("report commit unavailable"))?;
    check_revision()?;
    let source = CString::new(format!("/proc/self/fd/{}", file.as_raw_fd()))?;
    // SAFETY: procfs names our still-owned anonymous file, never a caller path.
    // linkat publishes into the held directory and atomically refuses every
    // existing destination, including entries created after preview/review.
    let result = unsafe {
        libc::linkat(
            libc::AT_FDCWD,
            source.as_ptr(),
            directory.as_raw_fd(),
            name.as_ptr(),
            libc::AT_SYMLINK_FOLLOW,
        )
    };
    if result < 0 {
        return Err(io::Error::last_os_error());
    }
    // A failure here means the complete file exists but durability is uncertain.
    // Never unlink by name on failure: another writer may have replaced it.
    Ok(if sync_directory(&directory).is_ok() {
        ReportExportOutcome::Saved
    } else {
        ReportExportOutcome::DurabilityUnknown
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, symlink};

    fn save(path: &Path, content: &str) -> io::Result<ReportExportOutcome> {
        save_report_revision(path, content, &AtomicU64::new(1), 1, &Mutex::new(()))
    }

    #[test]
    fn new_report_is_complete_private_and_has_no_named_temporary_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report.txt");
        save(&path, "private report\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "private report\n");
        assert_eq!(fs::metadata(&path).unwrap().mode() & 0o777, 0o600);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn existing_file_symlink_and_directory_are_preserved() {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, "keep").unwrap();
        let link = dir.path().join("symlink");
        symlink(&victim, &link).unwrap();
        let subdir = dir.path().join("directory");
        fs::create_dir(&subdir).unwrap();
        for path in [&victim, &link, &subdir] {
            assert_eq!(
                save(path, "replace").unwrap_err().kind(),
                io::ErrorKind::AlreadyExists
            );
        }
        assert_eq!(fs::read_to_string(&victim).unwrap(), "keep");
        assert!(fs::symlink_metadata(&link).unwrap().is_symlink());
        assert!(subdir.is_dir());
    }

    #[test]
    fn racing_destination_collision_never_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report");
        let result = save_internal(
            &path,
            "report",
            &AtomicU64::new(1),
            1,
            &Mutex::new(()),
            || {
                fs::write(&path, "racing writer").unwrap();
            },
            File::sync_all,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(fs::read_to_string(&path).unwrap(), "racing writer");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[test]
    fn changed_preview_during_write_leaves_no_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report");
        let revision = AtomicU64::new(1);
        let result = save_internal(
            &path,
            "stale",
            &revision,
            1,
            &Mutex::new(()),
            || {
                revision.store(2, Ordering::Release);
            },
            File::sync_all,
        );
        assert_eq!(result.unwrap_err().kind(), io::ErrorKind::Interrupted);
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }

    #[test]
    fn replaced_parent_does_not_redirect_export() {
        let root = tempfile::tempdir().unwrap();
        let parent = root.path().join("chosen");
        let held = root.path().join("held");
        fs::create_dir(&parent).unwrap();
        let path = parent.join("report");
        save_internal(
            &path,
            "report",
            &AtomicU64::new(1),
            1,
            &Mutex::new(()),
            || {
                fs::rename(&parent, &held).unwrap();
                fs::create_dir(&parent).unwrap();
                fs::write(parent.join("report"), "victim").unwrap();
            },
            File::sync_all,
        )
        .unwrap();
        assert_eq!(fs::read_to_string(held.join("report")).unwrap(), "report");
        assert_eq!(fs::read_to_string(&path).unwrap(), "victim");
    }

    #[test]
    fn symlink_ancestor_is_rejected_without_creating_a_file() {
        let root = tempfile::tempdir().unwrap();
        let actual = root.path().join("actual");
        fs::create_dir(&actual).unwrap();
        let alias = root.path().join("alias");
        symlink(&actual, &alias).unwrap();
        assert!(save(&alias.join("report"), "report").is_err());
        assert_eq!(fs::read_dir(actual).unwrap().count(), 0);
    }
    #[test]
    fn directory_sync_failure_reports_existing_complete_export() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("report");
        let outcome = save_internal(
            &path,
            "complete report",
            &AtomicU64::new(1),
            1,
            &Mutex::new(()),
            || {},
            |_| Err(io::Error::from_raw_os_error(libc::EIO)),
        )
        .unwrap();
        assert_eq!(outcome, ReportExportOutcome::DurabilityUnknown);
        assert_eq!(fs::read_to_string(path).unwrap(), "complete report");
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 1);
    }
}
