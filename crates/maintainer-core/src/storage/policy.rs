use std::path::PathBuf;

/// One gibibyte in bytes.
const ONE_GIB: u64 = 1024 * 1024 * 1024;

/// Explicit configuration controlling which paths a storage scan visits and how
/// it handles boundary conditions.
///
/// Callers must provide at least one root. The scanner never silently adds
/// system roots.
#[derive(Debug, Clone)]
pub struct ScanPolicy {
    /// Directories to scan. Must contain at least one entry.
    pub roots: Vec<PathBuf>,
    /// Paths excluded from traversal. Matched by exact prefix comparison
    /// against canonical paths.
    pub exclusions: Vec<PathBuf>,
    /// When `false` (default), the scanner stops at filesystem boundaries
    /// detected via device ID comparison.
    pub cross_filesystems: bool,
    /// When `false` (default), symlinks are not followed. They are reported
    /// but not dereferenced.
    pub follow_symlinks: bool,
    /// Minimum file size to include in the large-file results.
    pub large_file_threshold_bytes: u64,
}

impl Default for ScanPolicy {
    fn default() -> Self {
        Self {
            roots: Vec::new(),
            exclusions: Vec::new(),
            cross_filesystems: false,
            follow_symlinks: false,
            large_file_threshold_bytes: ONE_GIB,
        }
    }
}

impl ScanPolicy {
    /// Create a policy with the given roots and all other fields at their
    /// defaults.
    pub fn with_roots(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            ..Default::default()
        }
    }

    /// Returns `true` if the given path should be skipped based on exclusions.
    pub fn is_excluded(&self, path: &std::path::Path) -> bool {
        self.exclusions.iter().any(|excl| path.starts_with(excl))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn default_does_not_follow_symlinks() {
        let policy = ScanPolicy::default();
        assert!(!policy.follow_symlinks);
    }

    #[test]
    fn default_does_not_cross_filesystems() {
        let policy = ScanPolicy::default();
        assert!(!policy.cross_filesystems);
    }

    #[test]
    fn default_threshold_is_one_gib() {
        let policy = ScanPolicy::default();
        assert_eq!(policy.large_file_threshold_bytes, 1024 * 1024 * 1024);
    }

    #[test]
    fn default_has_no_roots() {
        let policy = ScanPolicy::default();
        assert!(policy.roots.is_empty());
    }

    #[test]
    fn with_roots_sets_roots() {
        let policy = ScanPolicy::with_roots(vec![PathBuf::from("/home")]);
        assert_eq!(policy.roots.len(), 1);
        assert_eq!(policy.roots[0], Path::new("/home"));
    }

    #[test]
    fn is_excluded_matches_prefix() {
        let policy = ScanPolicy {
            exclusions: vec![PathBuf::from("/home/user/.cache")],
            ..Default::default()
        };
        assert!(policy.is_excluded(Path::new("/home/user/.cache/foo")));
        assert!(!policy.is_excluded(Path::new("/home/user/docs")));
    }
}

/// Resolve each component against an already opened directory. Never follow a
/// symlink in an ancestor or the leaf, including when a path is concurrently replaced.
pub(crate) fn open_anchored(
    path: &std::path::Path,
    directory: bool,
) -> std::io::Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    use std::path::Component;
    let mut current = std::fs::File::open(if path.is_absolute() { "/" } else { "." })?;
    let components: Vec<_> = path
        .components()
        .filter(|c| !matches!(c, Component::RootDir | Component::CurDir))
        .collect();
    if components.len() > 256 {
        return Err(std::io::Error::other("directory depth limit reached"));
    }
    for (index, component) in components.iter().enumerate() {
        if !matches!(component, Component::Normal(_)) {
            return Err(std::io::Error::other(
                "parent path components are unsupported",
            ));
        }
        let name = CString::new(component.as_os_str().as_bytes())?;
        let flags = libc::O_RDONLY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK
            | if directory || index + 1 < components.len() {
                libc::O_DIRECTORY
            } else {
                0
            };
        // SAFETY: current owns a live directory descriptor; name is NUL terminated.
        // openat returns a new owned descriptor and never follows symbolic links.
        let fd = unsafe { libc::openat(current.as_raw_fd(), name.as_ptr(), flags) };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: successful openat returned a fresh descriptor owned here exactly once.
        current = unsafe { std::fs::File::from_raw_fd(fd) };
    }
    Ok(current)
}

pub(crate) fn descriptor_path(file: &std::fs::File) -> PathBuf {
    use std::os::fd::AsRawFd;
    PathBuf::from(format!("/proc/self/fd/{}", file.as_raw_fd()))
}

pub(crate) fn open_child_directory(
    parent: &std::fs::File,
    name: &std::ffi::OsStr,
    cross_filesystems: bool,
) -> std::io::Result<std::fs::File> {
    use std::ffi::CString;
    use std::os::fd::{AsRawFd, FromRawFd};
    use std::os::unix::ffi::OsStrExt;
    let name = CString::new(name.as_bytes())?;
    #[repr(C)]
    struct OpenHow {
        flags: u64,
        mode: u64,
        resolve: u64,
    }
    let how = OpenHow {
        flags: (libc::O_RDONLY
            | libc::O_DIRECTORY
            | libc::O_CLOEXEC
            | libc::O_NOFOLLOW
            | libc::O_NONBLOCK) as u64,
        mode: 0,
        // RESOLVE_BENEATH | RESOLVE_NO_SYMLINKS, optionally RESOLVE_NO_XDEV.
        // NO_XDEV rejects bind mounts as well as device changes.
        resolve: 0x08 | 0x04 | if cross_filesystems { 0 } else { 0x01 },
    };
    // SAFETY: parent is live, name is NUL terminated and how has the Linux
    // open_how ABI. A failed/unsupported syscall returns an error, never a
    // weaker path-based fallback. Ubuntu 22.04's kernel supports openat2.
    let fd = unsafe {
        libc::syscall(
            libc::SYS_openat2,
            parent.as_raw_fd(),
            name.as_ptr(),
            &how,
            std::mem::size_of::<OpenHow>(),
        )
    } as libc::c_int;
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: fd is a newly allocated successful openat descriptor.
    Ok(unsafe { std::fs::File::from_raw_fd(fd) })
}

#[cfg(test)]
mod anchored_tests {
    use super::*;
    use std::os::unix::fs::symlink;

    #[test]
    fn queued_directory_stays_attached_after_ancestor_swap() {
        let temporary = tempfile::tempdir().unwrap();
        let inside = temporary.path().join("inside");
        let outside = temporary.path().join("outside");
        std::fs::create_dir(&inside).unwrap();
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(inside.join("original"), b"inside").unwrap();
        std::fs::write(outside.join("secret"), b"outside").unwrap();
        let anchor = open_anchored(&inside, true).unwrap();
        std::fs::rename(&inside, temporary.path().join("moved")).unwrap();
        symlink(&outside, &inside).unwrap();
        let names: Vec<_> = std::fs::read_dir(descriptor_path(&anchor))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names, vec![std::ffi::OsString::from("original")]);
        assert!(open_anchored(&inside.join("secret"), false).is_err());
    }

    #[test]
    fn discovered_child_replacement_cannot_redirect_to_symlink() {
        let temporary = tempfile::tempdir().unwrap();
        let parent = open_anchored(temporary.path(), true).unwrap();
        let child = temporary.path().join("child");
        std::fs::create_dir(&child).unwrap();
        std::fs::rename(&child, temporary.path().join("moved")).unwrap();
        symlink("/tmp", &child).unwrap();
        assert!(open_child_directory(&parent, std::ffi::OsStr::new("child"), false).is_err());
    }
}
