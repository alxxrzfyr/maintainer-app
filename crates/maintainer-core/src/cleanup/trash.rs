//! FreeDesktop Trash specification implementation.
//!
//! Moves files to `.Trash-$uid` (or `$XDG_DATA_HOME/Trash`) with matching
//! `.trashinfo` metadata. If Trash is unavailable (cross-filesystem, no
//! writable Trash directory), returns a permanent-deletion requirement rather
//! than silently unlinking.

use std::fs;
use std::io::Write;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::MaintainerError;

/// Result of a trash operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashOutcome {
    /// File was moved to Trash successfully.
    Trashed {
        /// Path in the trash files directory.
        trash_path: PathBuf,
    },
    /// Trash is not available; permanent deletion is required.
    TrashUnavailable {
        /// Human-readable reason.
        reason: String,
    },
}

/// Move a file or directory to the FreeDesktop Trash.
///
/// Implements the spec at <https://specifications.freedesktop.org/trash-spec/latest/>.
/// On success returns the path within trash. If Trash cannot be used, returns
/// `TrashUnavailable` — the caller must then ask for permanent deletion confirmation.
pub fn trash_file(path: &Path, home_dir: &Path) -> Result<TrashOutcome, MaintainerError> {
    // Resolve the source metadata to check filesystem.
    let source_meta = fs::symlink_metadata(path)
        .map_err(|e| MaintainerError::io(format!("stat {}", path.display()), e))?;

    let trash_dir = resolve_trash_dir(path, home_dir, &source_meta)?;

    let trash_dir = match trash_dir {
        Some(d) => d,
        None => {
            return Ok(TrashOutcome::TrashUnavailable {
                reason: "no writable Trash directory on the same filesystem".into(),
            });
        }
    };

    let files_dir = trash_dir.join("files");
    let info_dir = trash_dir.join("info");

    fs::create_dir_all(&files_dir).map_err(|e| MaintainerError::io("creating Trash/files", e))?;
    fs::create_dir_all(&info_dir).map_err(|e| MaintainerError::io("creating Trash/info", e))?;

    // Generate a unique trash name (filename + optional counter).
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "unnamed".into());

    let (trash_name, trash_file_path) = unique_trash_name(&files_dir, &file_name)?;

    // Write .trashinfo BEFORE moving (atomic: if info write fails, we don't move).
    let info_content = format_trashinfo(path)?;
    let info_path = info_dir.join(format!("{trash_name}.trashinfo"));

    let mut info_file =
        fs::File::create(&info_path).map_err(|e| MaintainerError::io("creating .trashinfo", e))?;
    info_file
        .write_all(info_content.as_bytes())
        .map_err(|e| MaintainerError::io("writing .trashinfo", e))?;

    // Move file to trash.
    match fs::rename(path, &trash_file_path) {
        Ok(()) => Ok(TrashOutcome::Trashed {
            trash_path: trash_file_path,
        }),
        Err(e) => {
            // Clean up the info file on move failure.
            let _ = fs::remove_file(&info_path);
            // Cross-device link means different filesystem.
            if e.raw_os_error() == Some(libc::EXDEV) {
                Ok(TrashOutcome::TrashUnavailable {
                    reason: "source and Trash are on different filesystems".into(),
                })
            } else {
                Err(MaintainerError::io(
                    format!("moving {} to Trash", path.display()),
                    e,
                ))
            }
        }
    }
}

/// Resolve the appropriate Trash directory for a path.
///
/// Uses $HOME/.local/share/Trash for items on the same filesystem,
/// or `.Trash-$uid` on the mount root for items on other filesystems.
fn resolve_trash_dir(
    path: &Path,
    home_dir: &Path,
    source_meta: &fs::Metadata,
) -> Result<Option<PathBuf>, MaintainerError> {
    let source_dev = source_meta.dev();

    // Primary: $HOME/.local/share/Trash
    let home_trash = home_dir.join(".local/share/Trash");
    let home_dev = fs::metadata(home_dir).map(|m| m.dev()).unwrap_or(0);

    if source_dev == home_dev {
        return Ok(Some(home_trash));
    }

    // Secondary: .Trash-$uid on the mount point of the source.
    // We approximate the mount root by walking up until device changes.
    let uid = unsafe { libc::getuid() };
    let mount_root = find_mount_root(path, source_dev)?;
    let alt_trash = mount_root.join(format!(".Trash-{uid}"));

    // Check if we can use this directory.
    if alt_trash.exists() || fs::create_dir_all(&alt_trash).is_ok() {
        return Ok(Some(alt_trash));
    }

    Ok(None)
}

/// Walk up from `path` to find the mount root (where device ID changes).
fn find_mount_root(path: &Path, dev: u64) -> Result<PathBuf, MaintainerError> {
    let mut current = path.to_path_buf();
    loop {
        let parent = match current.parent() {
            Some(p) => p.to_path_buf(),
            None => return Ok(current),
        };
        let parent_dev = fs::metadata(&parent).map(|m| m.dev()).unwrap_or(dev);
        if parent_dev != dev {
            return Ok(current);
        }
        if parent == current {
            return Ok(current);
        }
        current = parent;
    }
}

/// Generate a unique trash filename, appending a counter if needed.
fn unique_trash_name(
    files_dir: &Path,
    base_name: &str,
) -> Result<(String, PathBuf), MaintainerError> {
    let candidate = files_dir.join(base_name);
    if !candidate.exists() {
        return Ok((base_name.to_string(), candidate));
    }

    // Split into stem and extension for counter insertion.
    let path_ref = Path::new(base_name);
    let stem = path_ref
        .file_stem()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_else(|| base_name.to_string());
    let ext = path_ref
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()));

    for counter in 2..=9999u32 {
        let name = match &ext {
            Some(e) => format!("{stem}.{counter}{e}"),
            None => format!("{stem}.{counter}"),
        };
        let candidate = files_dir.join(&name);
        if !candidate.exists() {
            return Ok((name, candidate));
        }
    }

    Err(MaintainerError::Internal {
        context: "trash naming".into(),
        message: format!("could not find unique name for {base_name}"),
    })
}

/// Format the .trashinfo file content per FreeDesktop spec.
fn format_trashinfo(original_path: &Path) -> Result<String, MaintainerError> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_err(|e| MaintainerError::Internal {
            context: "system time".into(),
            message: e.to_string(),
        })?;

    // Format as ISO 8601 basic.
    let secs = now.as_secs();
    let datetime = format_timestamp(secs);

    // URL-encode the path (percent-encode non-ASCII and special chars).
    let encoded_path = percent_encode_path(original_path);

    Ok(format!(
        "[Trash Info]\nPath={encoded_path}\nDeletionDate={datetime}\n"
    ))
}

/// Format a Unix timestamp as ISO 8601 (YYYY-MM-DDTHH:MM:SS).
fn format_timestamp(secs: u64) -> String {
    // Simple UTC timestamp formatting without external dependencies.
    let days = secs / 86400;
    let time_secs = secs % 86400;
    let hours = time_secs / 3600;
    let minutes = (time_secs % 3600) / 60;
    let seconds = time_secs % 60;

    // Compute date from days since epoch (1970-01-01).
    let (year, month, day) = days_to_date(days);

    format!("{year:04}-{month:02}-{day:02}T{hours:02}:{minutes:02}:{seconds:02}")
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_date(days: u64) -> (u64, u64, u64) {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

/// Percent-encode a path for .trashinfo (encode everything except unreserved).
fn percent_encode_path(path: &Path) -> String {
    let s = path.to_string_lossy();
    let mut out = String::with_capacity(s.len());
    for byte in s.as_bytes() {
        match byte {
            // Unreserved characters (RFC 3986) plus `/`.
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(*byte as char);
            }
            _ => {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use std::io::Write;

    #[test]
    fn trash_file_moves_to_trash() {
        let home = tempfile::tempdir().unwrap();
        let file_path = home.path().join("test_file.txt");
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"hello").unwrap();

        let result = trash_file(&file_path, home.path()).unwrap();
        match result {
            TrashOutcome::Trashed { trash_path } => {
                assert!(trash_path.exists());
                assert!(!file_path.exists());
                // Check .trashinfo was created.
                let info_dir = home.path().join(".local/share/Trash/info");
                let entries: Vec<_> = fs::read_dir(&info_dir)
                    .unwrap()
                    .filter_map(|e| e.ok())
                    .collect();
                assert_eq!(entries.len(), 1);
                let info_content = fs::read_to_string(entries[0].path()).unwrap();
                assert!(info_content.contains("[Trash Info]"));
                assert!(info_content.contains("Path="));
                assert!(info_content.contains("DeletionDate="));
            }
            TrashOutcome::TrashUnavailable { .. } => {
                panic!("expected trash to succeed on same filesystem");
            }
        }
    }

    #[test]
    fn trash_file_handles_name_collision() {
        let home = tempfile::tempdir().unwrap();
        let trash_files = home.path().join(".local/share/Trash/files");
        let trash_info = home.path().join(".local/share/Trash/info");
        fs::create_dir_all(&trash_files).unwrap();
        fs::create_dir_all(&trash_info).unwrap();

        // Pre-occupy the trash name.
        File::create(trash_files.join("collide.txt")).unwrap();

        let file_path = home.path().join("collide.txt");
        let mut f = File::create(&file_path).unwrap();
        f.write_all(b"data").unwrap();

        let result = trash_file(&file_path, home.path()).unwrap();
        match result {
            TrashOutcome::Trashed { trash_path } => {
                assert!(trash_path.exists());
                // Should have a counter suffix.
                let name = trash_path.file_name().unwrap().to_string_lossy();
                assert!(name.contains("2"), "expected counter in name: {name}");
            }
            TrashOutcome::TrashUnavailable { .. } => {
                panic!("expected trash to succeed");
            }
        }
    }

    #[test]
    fn trash_nonexistent_file_returns_error() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("does_not_exist.txt");
        let result = trash_file(&path, home.path());
        assert!(result.is_err());
    }

    #[test]
    fn trash_directory() {
        let home = tempfile::tempdir().unwrap();
        let dir_path = home.path().join("subdir");
        fs::create_dir(&dir_path).unwrap();
        let mut f = File::create(dir_path.join("inner.txt")).unwrap();
        f.write_all(b"content").unwrap();

        let result = trash_file(&dir_path, home.path()).unwrap();
        match result {
            TrashOutcome::Trashed { trash_path } => {
                assert!(trash_path.exists());
                assert!(!dir_path.exists());
            }
            TrashOutcome::TrashUnavailable { .. } => {
                panic!("expected trash to succeed");
            }
        }
    }

    #[test]
    fn format_trashinfo_contains_required_fields() {
        let path = Path::new("/home/user/Documents/report.pdf");
        let info = format_trashinfo(path).unwrap();
        assert!(info.starts_with("[Trash Info]\n"));
        assert!(info.contains("Path=/home/user/Documents/report.pdf"));
        assert!(info.contains("DeletionDate="));
    }

    #[test]
    fn percent_encode_path_handles_spaces() {
        let path = Path::new("/home/user/my documents/file name.txt");
        let encoded = percent_encode_path(path);
        assert!(encoded.contains("%20"));
        assert!(encoded.contains("file%20name.txt"));
    }

    #[test]
    fn unique_trash_name_no_collision() {
        let dir = tempfile::tempdir().unwrap();
        let (name, path) = unique_trash_name(dir.path(), "test.txt").unwrap();
        assert_eq!(name, "test.txt");
        assert_eq!(path, dir.path().join("test.txt"));
    }

    #[test]
    fn unique_trash_name_with_collision() {
        let dir = tempfile::tempdir().unwrap();
        File::create(dir.path().join("test.txt")).unwrap();
        let (name, _path) = unique_trash_name(dir.path(), "test.txt").unwrap();
        assert_eq!(name, "test.2.txt");
    }

    #[test]
    fn days_to_date_epoch() {
        let (y, m, d) = days_to_date(0);
        assert_eq!((y, m, d), (1970, 1, 1));
    }

    #[test]
    fn days_to_date_known_date() {
        // 2024-01-01 is day 19723
        let (y, m, d) = days_to_date(19723);
        assert_eq!((y, m, d), (2024, 1, 1));
    }

    #[test]
    fn format_timestamp_epoch() {
        let ts = format_timestamp(0);
        assert_eq!(ts, "1970-01-01T00:00:00");
    }
}
