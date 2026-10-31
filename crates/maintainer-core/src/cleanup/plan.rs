//! Immutable cleanup plans.
//!
//! A `CleanupPlan` is created from analyzed targets and becomes immutable once
//! constructed. It records the exact targets, estimated reclaimable bytes, and
//! a unique ID for traceability. Execution references the plan by ID plus
//! selected item IDs.

use std::fs::{File, OpenOptions};
use std::os::fd::{AsFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

use uuid::Uuid;

use crate::cleanup::analyze::{
    CleanupCategory, CleanupTarget, PrivilegeRequirement, Reversibility,
};
use crate::error::MaintainerError;

/// A single item within an immutable cleanup plan.
#[derive(Debug, Clone)]
pub struct CleanupItem {
    /// Retained before review; clones share the same pinned identity.
    pub reviewed: Option<Arc<ReviewedFile>>,
    /// Unsupported entries remain visible with an explicit reason.
    pub unavailable_reason: Option<String>,
    /// Unique item identifier within the plan.
    pub id: Uuid,
    /// Canonical path to the target.
    pub path: PathBuf,
    /// Category.
    pub category: CleanupCategory,
    /// Display-friendly path.
    pub display_path: String,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Privilege requirement.
    pub privilege: PrivilegeRequirement,
    /// Reversibility.
    pub reversibility: Reversibility,
    /// Inode at plan creation for race detection.
    pub inode: u64,
    /// Device at plan creation for race detection.
    pub device: u64,
}

/// Descriptor capability retained by an immutable review plan.
#[derive(Debug)]
pub struct ReviewedFile {
    pub parent: File,
    pub file: File,
    pub basename: String,
}

impl ReviewedFile {
    fn pin(path: &Path) -> std::io::Result<Self> {
        let parent_path = path
            .parent()
            .ok_or_else(|| std::io::Error::other("missing parent"))?;
        // Every ancestor is opened relative to the retained preceding directory.
        let mut parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open("/")?;
        for part in parent_path.components() {
            let std::path::Component::Normal(part) = part else {
                continue;
            };
            let name = std::ffi::CString::new(part.as_bytes()).map_err(std::io::Error::other)?;
            // SAFETY: the retained directory and component string live through openat.
            let fd = unsafe {
                libc::openat(
                    parent.as_fd().as_raw_fd(),
                    name.as_ptr(),
                    libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                )
            };
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            // SAFETY: openat returned an owned directory descriptor.
            parent = unsafe { File::from_raw_fd(fd) };
        }
        let basename = path
            .file_name()
            .and_then(|s| s.to_str())
            .ok_or_else(|| std::io::Error::other("unsupported filename"))?
            .to_string();
        let name = crate::cleanup::quarantine::component(&basename)?;
        // SAFETY: O_PATH cannot block on a substituted FIFO; O_NOFOLLOW pins symlinks themselves for rejection.
        let fd = unsafe {
            libc::openat(
                parent.as_fd().as_raw_fd(),
                name.as_ptr(),
                libc::O_PATH | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            )
        };
        if fd < 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: openat returned an owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        let metadata = file.metadata()?;
        if !metadata.is_file() || metadata.nlink() != 1 {
            return Err(std::io::Error::other(
                "only single-link regular files are supported",
            ));
        }
        Ok(Self {
            parent,
            file,
            basename,
        })
    }
}

use std::os::fd::AsRawFd;

/// An immutable cleanup plan ready for user review.
///
/// Once created, items cannot be added or removed. The user confirms
/// specific items from this plan for execution.
#[derive(Debug, Clone)]
pub struct CleanupPlan {
    /// Unique plan identifier.
    pub id: Uuid,
    /// When this plan was created.
    pub created_at: SystemTime,
    /// All items in the plan.
    pub items: Vec<CleanupItem>,
    /// Total bytes that would be reclaimed if all items succeed.
    pub estimated_reclaimable_bytes: u64,
    /// The review covers a bounded manifest; folders are never recursively deleted.
    pub coverage_notice: Option<String>,
}

impl CleanupPlan {
    /// Create an immutable plan from analyzed targets.
    ///
    /// Each target is validated before inclusion. Invalid targets (with `..`
    /// components, symlinks to different locations, root filesystem targets,
    /// or the home directory itself) are rejected.
    pub fn from_targets(
        targets: Vec<CleanupTarget>,
        home_dir: &Path,
    ) -> Result<Self, MaintainerError> {
        Self::from_targets_cancellable(targets, home_dir, &crate::jobs::CancellationToken::new())
    }

    /// Build the bounded manifest in a worker, stopping before the next filesystem entry.
    pub fn from_targets_cancellable(
        targets: Vec<CleanupTarget>,
        home_dir: &Path,
        cancel: &dyn crate::jobs::Cancellation,
    ) -> Result<Self, MaintainerError> {
        const MAX_REVIEW_FILES: usize = 256;
        for target in &targets {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            validate_path(&target.path, home_dir)?;
        }
        let mut expanded = Vec::new();
        let mut pending = targets;
        let mut coverage_notice = None;
        let mut visited = 0usize;
        while let Some(target) = pending.pop() {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            visited += 1;
            if expanded.len() >= MAX_REVIEW_FILES || visited > 4096 {
                coverage_notice = Some("Partial review: limited to 256 entries and 4096 visited paths. Scan again for remaining files. Folders are retained.".into());
                break;
            }
            match std::fs::symlink_metadata(&target.path) {
                Ok(metadata) if metadata.is_dir() => match std::fs::read_dir(&target.path) {
                    Ok(entries) => {
                        for entry in entries.take(4097usize.saturating_sub(visited + pending.len()))
                        {
                            if cancel.is_cancelled() {
                                return Err(MaintainerError::Cancelled);
                            }
                            let Ok(entry) = entry else {
                                coverage_notice =
                                    Some("Partial review: some folders could not be read.".into());
                                continue;
                            };
                            let mut child = target.clone();
                            child.path = entry.path();
                            child.display_path = format!(
                                "~/{}",
                                child
                                    .path
                                    .strip_prefix(home_dir)
                                    .unwrap_or(&child.path)
                                    .display()
                            );
                            pending.push(child);
                        }
                    }
                    Err(_) => {
                        coverage_notice =
                            Some("Partial review: some folders could not be read.".into())
                    }
                },
                _ => expanded.push(target),
            }
        }
        let mut items = Vec::with_capacity(expanded.len());
        let mut estimated_reclaimable_bytes = 0u64;
        for target in expanded {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            let (reviewed, unavailable_reason, inode, device, size_bytes) =
                match ReviewedFile::pin(&target.path) {
                    Ok(pinned) => {
                        let metadata = pinned
                            .file
                            .metadata()
                            .map_err(|error| MaintainerError::io("pinning reviewed file", error))?;
                        (
                            Some(Arc::new(pinned)),
                            None,
                            metadata.ino(),
                            metadata.dev(),
                            metadata.len(),
                        )
                    }
                    Err(error) => (
                        None,
                        Some(error.to_string()),
                        target.inode,
                        target.device,
                        target.size_bytes,
                    ),
                };
            estimated_reclaimable_bytes = estimated_reclaimable_bytes.saturating_add(size_bytes);
            items.push(CleanupItem {
                reviewed,
                unavailable_reason,
                id: Uuid::new_v4(),
                path: target.path,
                category: target.category,
                display_path: target.display_path,
                size_bytes,
                privilege: target.privilege,
                reversibility: target.reversibility,
                inode,
                device,
            });
        }

        Ok(Self {
            id: Uuid::new_v4(),
            created_at: SystemTime::now(),
            items,
            estimated_reclaimable_bytes,
            coverage_notice,
        })
    }

    /// Look up an item by its ID. Returns `None` if not found.
    pub fn item_by_id(&self, id: &Uuid) -> Option<&CleanupItem> {
        self.items.iter().find(|item| item.id == *id)
    }

    /// Return true if the plan contains items requiring permanent deletion.
    pub fn has_permanent_deletions(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.reversibility == Reversibility::PermanentOnly)
    }

    /// Return true if the plan contains items requiring elevated privileges.
    pub fn requires_elevation(&self) -> bool {
        self.items
            .iter()
            .any(|item| item.privilege == PrivilegeRequirement::Elevated)
    }
}

/// Validate a path for inclusion in a cleanup plan.
///
/// Rejects:
/// - Paths containing `..` components
/// - The home directory itself
/// - Root filesystem targets (`/`, `/bin`, `/usr`, etc.)
/// - Paths that are symlinks (at plan creation time)
pub fn validate_path(path: &Path, home_dir: &Path) -> Result<(), MaintainerError> {
    let path_str = path.to_string_lossy();
    if !path.is_absolute() {
        return Err(MaintainerError::Internal {
            context: "path validation".into(),
            message: "cleanup paths must be absolute".into(),
        });
    }

    for component in path.components() {
        if matches!(component, std::path::Component::ParentDir) {
            return Err(MaintainerError::Internal {
                context: "path validation".into(),
                message: format!("path contains '..': {}", path_str),
            });
        }
    }

    if path == home_dir {
        return Err(MaintainerError::Internal {
            context: "path validation".into(),
            message: "cannot target home directory itself".into(),
        });
    }

    let root_paths: &[&str] = &[
        "/", "/bin", "/boot", "/dev", "/etc", "/lib", "/lib64", "/opt", "/proc", "/root", "/run",
        "/sbin", "/srv", "/sys", "/usr", "/var",
    ];
    for root in root_paths {
        if path == Path::new(root) {
            return Err(MaintainerError::Internal {
                context: "path validation".into(),
                message: format!("cannot target root filesystem path: {}", path_str),
            });
        }
    }

    if path.exists() {
        let metadata = std::fs::symlink_metadata(path).map_err(|e| {
            MaintainerError::io(format!("checking symlink status of {}", path_str), e)
        })?;
        if metadata.is_symlink() {
            return Err(MaintainerError::Internal {
                context: "path validation".into(),
                message: format!("path is a symlink: {}", path_str),
            });
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cleanup::analyze::{CleanupCategory, PrivilegeRequirement, Reversibility};
    use std::fs::{self, File};
    use std::io::Write;
    use std::os::unix::fs::MetadataExt;
    use std::path::{Path, PathBuf};

    fn make_target(path: PathBuf, home: &Path) -> CleanupTarget {
        let metadata = fs::symlink_metadata(&path).unwrap();
        CleanupTarget {
            display_path: format!("~/{}", path.strip_prefix(home).unwrap_or(&path).display()),
            path,
            category: CleanupCategory::ThumbnailCache,
            size_bytes: 1024,
            privilege: PrivilegeRequirement::None,
            reversibility: Reversibility::PermanentOnly,
            inode: metadata.ino(),
            device: metadata.dev(),
            evidence_timestamp: SystemTime::now(),
        }
    }

    #[test]
    fn plan_from_valid_targets() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("a.png");
        let mut f = File::create(&file_path).unwrap();
        f.write_all(&[0u8; 1024]).unwrap();

        let target = make_target(file_path, home.path());
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();

        assert_eq!(plan.items.len(), 1);
        assert_eq!(plan.estimated_reclaimable_bytes, 1024);
        assert_ne!(plan.id, Uuid::nil());
    }

    #[test]
    fn plan_rejects_dotdot_path() {
        let home = tempfile::tempdir().unwrap();
        let bad_path = home.path().join(".cache/../../../etc/passwd");
        let target = CleanupTarget {
            path: bad_path,
            category: CleanupCategory::UserCache,
            display_path: "bad".into(),
            size_bytes: 100,
            privilege: PrivilegeRequirement::None,
            reversibility: Reversibility::PermanentOnly,
            inode: 0,
            device: 0,
            evidence_timestamp: SystemTime::now(),
        };
        let result = CleanupPlan::from_targets(vec![target], home.path());
        assert!(result.is_err());
        let msg = result.unwrap_err().to_string();
        assert!(msg.contains(".."));
    }

    #[test]
    fn plan_rejects_home_directory() {
        let home = tempfile::tempdir().unwrap();
        let target = CleanupTarget {
            path: home.path().to_path_buf(),
            category: CleanupCategory::UserCache,
            display_path: "~".into(),
            size_bytes: 100,
            privilege: PrivilegeRequirement::None,
            reversibility: Reversibility::PermanentOnly,
            inode: 0,
            device: 0,
            evidence_timestamp: SystemTime::now(),
        };
        let result = CleanupPlan::from_targets(vec![target], home.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("home directory"));
    }

    #[test]
    fn plan_rejects_root_filesystem_target() {
        let home = tempfile::tempdir().unwrap();
        let target = CleanupTarget {
            path: PathBuf::from("/usr"),
            category: CleanupCategory::Logs,
            display_path: "/usr".into(),
            size_bytes: 100,
            privilege: PrivilegeRequirement::Elevated,
            reversibility: Reversibility::PermanentOnly,
            inode: 0,
            device: 0,
            evidence_timestamp: SystemTime::now(),
        };
        let result = CleanupPlan::from_targets(vec![target], home.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("root filesystem"));
    }

    #[test]
    fn plan_rejects_symlink_target() {
        let home = tempfile::tempdir().unwrap();
        let real_file = home.path().join("real.txt");
        let mut f = File::create(&real_file).unwrap();
        f.write_all(&[0u8; 100]).unwrap();

        let link_path = home.path().join("link.txt");
        std::os::unix::fs::symlink(&real_file, &link_path).unwrap();

        let target = CleanupTarget {
            path: link_path,
            category: CleanupCategory::UserCache,
            display_path: "~/link.txt".into(),
            size_bytes: 100,
            privilege: PrivilegeRequirement::None,
            reversibility: Reversibility::PermanentOnly,
            inode: 0,
            device: 0,
            evidence_timestamp: SystemTime::now(),
        };
        let result = CleanupPlan::from_targets(vec![target], home.path());
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("symlink"));
    }

    #[test]
    fn plan_item_lookup_by_id() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("b.png");
        let mut f = File::create(&file_path).unwrap();
        f.write_all(&[0u8; 512]).unwrap();

        let target = make_target(file_path, home.path());
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();

        let item_id = plan.items[0].id;
        assert!(plan.item_by_id(&item_id).is_some());
        assert!(plan.item_by_id(&Uuid::new_v4()).is_none());
    }

    #[test]
    fn plan_has_permanent_deletions() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&dir).unwrap();
        let file_path = dir.join("c.png");
        let mut f = File::create(&file_path).unwrap();
        f.write_all(&[0u8; 100]).unwrap();

        let target = make_target(file_path, home.path());
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        assert!(plan.has_permanent_deletions());
    }

    #[test]
    fn directory_review_is_bounded_and_new_files_are_not_selected() {
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join("cache");
        fs::create_dir(&directory).unwrap();
        for index in 0..300 {
            fs::write(directory.join(format!("file-{index}")), b"data").unwrap();
        }
        let target = make_target(directory.clone(), home.path());
        let plan = CleanupPlan::from_targets(vec![target], home.path()).unwrap();
        assert_eq!(plan.items.len(), 256);
        assert!(plan.items.iter().all(|item| item.reviewed.is_some()));
        assert!(
            plan.coverage_notice
                .as_deref()
                .unwrap()
                .contains("Partial review")
        );
        fs::write(directory.join("added-after-review"), b"unreviewed").unwrap();
        assert!(
            plan.items
                .iter()
                .all(|item| item.path.file_name().unwrap() != "added-after-review")
        );
    }

    #[test]
    fn review_descriptor_pins_the_original_inode_through_replacement() {
        let home = tempfile::tempdir().unwrap();
        let path = home.path().join("file");
        fs::write(&path, b"original").unwrap();
        let plan =
            CleanupPlan::from_targets(vec![make_target(path.clone(), home.path())], home.path())
                .unwrap();
        fs::remove_file(&path).unwrap();
        fs::write(&path, b"replacement").unwrap();
        let item = &plan.items[0];
        let retained = item.reviewed.as_ref().unwrap().file.metadata().unwrap();
        assert_eq!(retained.ino(), item.inode);
        assert_eq!(retained.nlink(), 0);
        assert_ne!(fs::metadata(&path).unwrap().ino(), retained.ino());
    }

    #[test]
    fn review_manifest_cancels_before_and_during_directory_expansion() {
        use crate::jobs::{Cancellation, CancellationToken};
        use std::sync::atomic::{AtomicUsize, Ordering};
        let home = tempfile::tempdir().unwrap();
        let directory = home.path().join("cache");
        fs::create_dir(&directory).unwrap();
        for index in 0..40 {
            fs::write(directory.join(format!("file-{index}")), b"reviewed").unwrap();
        }
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert!(matches!(
            CleanupPlan::from_targets_cancellable(
                vec![make_target(directory.clone(), home.path())],
                home.path(),
                &cancelled
            ),
            Err(MaintainerError::Cancelled)
        ));
        struct CancelAfter(AtomicUsize);
        impl Cancellation for CancelAfter {
            fn is_cancelled(&self) -> bool {
                self.0.fetch_add(1, Ordering::SeqCst) >= 12
            }
        }
        let cancel = CancelAfter(AtomicUsize::new(0));
        assert!(matches!(
            CleanupPlan::from_targets_cancellable(
                vec![make_target(directory.clone(), home.path())],
                home.path(),
                &cancel
            ),
            Err(MaintainerError::Cancelled)
        ));
        assert_eq!(cancel.0.load(Ordering::SeqCst), 13);
        assert_eq!(fs::read_dir(directory).unwrap().count(), 40);
    }

    #[test]
    fn estimated_reclaimable_bytes_sums_all_items() {
        let home = tempfile::tempdir().unwrap();
        let dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&dir).unwrap();

        let mut targets = Vec::new();
        for i in 0..3 {
            let file_path = dir.join(format!("f{i}.png"));
            let mut f = File::create(&file_path).unwrap();
            f.write_all(&[0u8; 100]).unwrap();
            targets.push(make_target(file_path, home.path()));
        }

        let plan = CleanupPlan::from_targets(targets, home.path()).unwrap();
        assert_eq!(plan.estimated_reclaimable_bytes, 300);
    }
}
