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
