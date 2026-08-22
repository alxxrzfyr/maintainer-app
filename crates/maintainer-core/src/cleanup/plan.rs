//! Immutable cleanup plans.
//!
//! A `CleanupPlan` is created from analyzed targets and becomes immutable once
//! constructed. It records the exact targets, estimated reclaimable bytes, and
//! a unique ID for traceability. Execution references the plan by ID plus
//! selected item IDs.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use uuid::Uuid;

use crate::cleanup::analyze::{
    CleanupCategory, CleanupTarget, PrivilegeRequirement, Reversibility,
};
use crate::error::MaintainerError;

/// A single item within an immutable cleanup plan.
#[derive(Debug, Clone)]
pub struct CleanupItem {
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
        let mut items = Vec::with_capacity(targets.len());
        let mut estimated_reclaimable_bytes = 0u64;

        for target in targets {
            validate_path(&target.path, home_dir)?;
            estimated_reclaimable_bytes =
                estimated_reclaimable_bytes.saturating_add(target.size_bytes);
            items.push(CleanupItem {
                id: Uuid::new_v4(),
                path: target.path,
                category: target.category,
                display_path: target.display_path,
                size_bytes: target.size_bytes,
                privilege: target.privilege,
                reversibility: target.reversibility,
                inode: target.inode,
                device: target.device,
            });
        }

        Ok(Self {
            id: Uuid::new_v4(),
            created_at: SystemTime::now(),
            items,
            estimated_reclaimable_bytes,
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
        assert_eq!(plan.estimated_reclaimable_bytes, 3 * 1024);
    }
}
