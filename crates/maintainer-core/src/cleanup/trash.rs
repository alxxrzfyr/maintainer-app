//! Public Trash mutation gate. No name-based move is safe against arbitrary writers.

use std::path::{Path, PathBuf};

use crate::cleanup::execute::CLEANUP_UNAVAILABLE_REASON;
use crate::error::MaintainerError;

/// Result of a Trash request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrashOutcome {
    /// Reserved for an implementation with reviewed source-identity binding.
    Trashed { trash_path: PathBuf },
    /// The operation is unavailable. This does not authorize permanent deletion.
    TrashUnavailable { reason: String },
}

/// Refuse before resolving any source or Trash path, creating metadata, or
/// moving a named entry. No fallback to permanent deletion is authorized.
pub fn trash_file(_path: &Path, _home_dir: &Path) -> Result<TrashOutcome, MaintainerError> {
    Ok(TrashOutcome::TrashUnavailable {
        reason: CLEANUP_UNAVAILABLE_REASON.to_string(),
    })
}

/// XDG Base Directory semantics: unset, empty and relative values use the default.
pub(crate) fn data_home(home: &Path, xdg: Option<&std::ffi::OsStr>) -> PathBuf {
    xdg.map(Path::new)
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".local/share"))
}

/// Prepare user-owned Trash directories without following symlink substitutions.
/// This creates directories only; mutation remains exclusively helper-mediated.
pub(crate) fn open_trash(home: &Path) -> std::io::Result<(std::fs::File, std::fs::File)> {
    let root = data_home(home, std::env::var_os("XDG_DATA_HOME").as_deref());
    open_trash_root(home, &root)
}

fn open_trash_root(home: &Path, root: &Path) -> std::io::Result<(std::fs::File, std::fs::File)> {
    use std::fs::{File, OpenOptions};
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
    let home = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open(home)?;
    let uid = home.metadata()?.uid();
    if !root.is_absolute()
        || root
            .components()
            .any(|part| matches!(part, std::path::Component::ParentDir))
    {
        return Err(std::io::Error::other(
            "Trash data directory must be an absolute path without parent substitutions",
        ));
    }
    let mut directory = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/")?;
    for part in root.components() {
        if let std::path::Component::Normal(name) = part {
            directory = child_directory(&directory, name, None)?;
        }
    }
    if directory.metadata()?.uid() != uid {
        return Err(std::io::Error::other(
            "Trash data directory is not owned by the user",
        ));
    }
    directory = child_directory(&directory, std::ffi::OsStr::new("Trash"), Some(uid))?;
    fn child_directory(
        parent: &File,
        name: &std::ffi::OsStr,
        uid: Option<u32>,
    ) -> std::io::Result<File> {
        let name = std::ffi::CString::new(name.as_bytes()).map_err(std::io::Error::other)?;
        // SAFETY: fixed name relative to an anchored directory; existing entries are opened without following links.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() != std::io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        // SAFETY: the CString and parent descriptor are retained across the syscall.
        let fd = unsafe {
            libc::openat(
                parent.as_raw_fd(),
                name.as_ptr(),
                libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: openat returned an owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        if uid.is_some_and(|uid| metadata.uid() != uid) {
            return Err(std::io::Error::other(
                "Trash directory is not owned by the user",
            ));
        }
        Ok(file)
    }
    Ok((
        child_directory(&directory, std::ffi::OsStr::new("files"), Some(uid))?,
        child_directory(&directory, std::ffi::OsStr::new("info"), Some(uid))?,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn assert_unavailable(path: &Path, home: &Path) {
        assert!(matches!(
            trash_file(path, home).unwrap(),
            TrashOutcome::TrashUnavailable { .. }
        ));
    }

    #[test]
    fn xdg_data_home_uses_absolute_override_and_defaults_invalid_values() {
        let home = Path::new("/home/test-user");
        assert_eq!(
            data_home(home, Some(std::ffi::OsStr::new("/custom/data"))),
            Path::new("/custom/data")
        );
        for value in [
            None,
            Some(std::ffi::OsStr::new("")),
            Some(std::ffi::OsStr::new("relative/data")),
        ] {
            assert_eq!(data_home(home, value), home.join(".local/share"));
        }
    }

    #[test]
    fn custom_xdg_trash_is_created_at_selected_data_home_and_symlinks_are_refused() {
        let home = tempfile::tempdir().unwrap();
        let custom = tempfile::tempdir().unwrap();
        let root = custom.path().join("data");
        let (files, info) = open_trash_root(home.path(), &root).unwrap();
        use std::os::unix::fs::MetadataExt;
        assert_eq!(
            files.metadata().unwrap().ino(),
            fs::metadata(root.join("Trash/files")).unwrap().ino()
        );
        assert_eq!(
            info.metadata().unwrap().ino(),
            fs::metadata(root.join("Trash/info")).unwrap().ino()
        );
        assert!(!home.path().join(".local").exists());
        let alias = custom.path().join("alias");
        std::os::unix::fs::symlink(&root, &alias).unwrap();
        assert!(open_trash_root(home.path(), &alias).is_err());
    }

    #[test]
    fn trash_file_stays_in_place_without_creating_metadata() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("test_file.txt");
        fs::write(&path, b"hello").unwrap();
        assert_unavailable(&path, home.path());
        assert_eq!(fs::read(&path).unwrap(), b"hello");
        assert!(!home.path().join(".local").exists());
    }

    #[test]
    fn trash_collision_preserves_source_destination_and_metadata() {
        let home = tempfile::tempdir().unwrap();
        let files = home.path().join(".local/share/Trash/files");
        let info = home.path().join(".local/share/Trash/info");
        fs::create_dir_all(&files).unwrap();
        fs::create_dir_all(&info).unwrap();
        let path = home.path().join("collide.txt");
        let destination = files.join("collide.txt");
        let metadata = info.join("collide.txt.trashinfo");
        fs::write(&path, b"source").unwrap();
        fs::write(&destination, b"existing destination").unwrap();
        fs::write(&metadata, b"existing metadata").unwrap();
        assert_unavailable(&path, home.path());
        assert_eq!(fs::read(&path).unwrap(), b"source");
        assert_eq!(fs::read(&destination).unwrap(), b"existing destination");
        assert_eq!(fs::read(&metadata).unwrap(), b"existing metadata");
        assert_eq!(fs::read_dir(files).unwrap().count(), 1);
        assert_eq!(fs::read_dir(info).unwrap().count(), 1);
    }

    #[test]
    fn nonexistent_source_is_unavailable_without_creating_trash() {
        let home = tempfile::tempdir().unwrap();
        assert_unavailable(&home.path().join("absent"), home.path());
        assert!(!home.path().join(".local").exists());
    }

    #[test]
    fn trash_directory_preserves_descendants() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join("subdir");
        fs::create_dir(&directory).unwrap();
        fs::write(directory.join("inner.txt"), b"content").unwrap();
        assert_unavailable(&directory, home.path());
        assert_eq!(fs::read(directory.join("inner.txt")).unwrap(), b"content");
        assert!(!home.path().join(".local").exists());
    }

    #[test]
    fn symlinked_trash_root_is_never_followed() {
        let home = tempfile::tempdir().unwrap();
        let victim = tempfile::tempdir().unwrap();
        std::os::unix::fs::symlink(victim.path(), home.path().join(".local")).unwrap();
        let source = home.path().join("source");
        fs::write(&source, b"source").unwrap();
        assert_unavailable(&source, home.path());
        assert_eq!(fs::read(&source).unwrap(), b"source");
        assert_eq!(fs::read_dir(victim.path()).unwrap().count(), 0);
    }
}
