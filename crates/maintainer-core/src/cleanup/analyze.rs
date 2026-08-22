//! Category-based cleanup target analyzers.
//!
//! Each analyzer scans a known location and returns stable target identity,
//! category, display path, bytes, privilege requirement, reversibility, and
//! evidence timestamp. Analyzers never delete anything.

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::MaintainerError;
use crate::jobs::{Cancellation, JobProgress, ProgressSink};

/// Category of cleanup target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CleanupCategory {
    /// APT package cache (`/var/cache/apt/archives`).
    PackageCache,
    /// Freedesktop thumbnail cache (`~/.cache/thumbnails`).
    ThumbnailCache,
    /// Items already in the Trash (`~/.local/share/Trash`).
    Trash,
    /// Explicitly supported user cache roots.
    UserCache,
    /// System and user log files.
    Logs,
}

impl CleanupCategory {
    /// Human-readable display name.
    pub fn label(self) -> &'static str {
        match self {
            Self::PackageCache => "Package Cache",
            Self::ThumbnailCache => "Thumbnail Cache",
            Self::Trash => "Trash",
            Self::UserCache => "Application Cache",
            Self::Logs => "Log Files",
        }
    }
}

/// Whether a target requires elevated privileges.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrivilegeRequirement {
    /// User can remove this directly.
    None,
    /// Needs polkit-authorized helper.
    Elevated,
}

/// Whether a removal is reversible.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reversibility {
    /// Can be moved to Trash and recovered.
    TrashRecoverable,
    /// Permanent removal only (e.g., already in Trash, or system paths).
    PermanentOnly,
}

/// A single cleanup target identified by an analyzer.
#[derive(Debug, Clone)]
pub struct CleanupTarget {
    /// Canonical absolute path to the file or directory.
    pub path: PathBuf,
    /// Category from which this target was discovered.
    pub category: CleanupCategory,
    /// Display-friendly path (may elide home directory).
    pub display_path: String,
    /// Size in bytes (file size or directory tree total).
    pub size_bytes: u64,
    /// Whether elevated privilege is needed.
    pub privilege: PrivilegeRequirement,
    /// Whether the target can go to Trash.
    pub reversibility: Reversibility,
    /// Inode at discovery time for path-race detection.
    pub inode: u64,
    /// Device at discovery time.
    pub device: u64,
    /// Timestamp when this target was observed.
    pub evidence_timestamp: SystemTime,
}

/// Analyzer configuration specifying which categories to scan.
#[derive(Debug, Clone)]
pub struct CleanupAnalyzer {
    /// Explicitly supported user cache roots. Not all of ~/.cache.
    user_cache_roots: Vec<PathBuf>,
    /// Home directory for the current user.
    home_dir: PathBuf,
}

impl CleanupAnalyzer {
    /// Create an analyzer with the given home directory.
    pub fn new(home_dir: PathBuf) -> Self {
        Self {
            user_cache_roots: vec![
                PathBuf::from("mesa_shader_cache"),
                PathBuf::from("fontconfig"),
                PathBuf::from("event-sound-cache"),
            ],
            home_dir,
        }
    }

    /// Override the supported user cache roots (for testing).
    pub fn with_user_cache_roots(mut self, roots: Vec<PathBuf>) -> Self {
        self.user_cache_roots = roots;
        self
    }

    /// Scan all categories and return discovered targets.
    ///
    /// Respects cancellation between categories. Reports progress.
    pub fn scan_all(
        &self,
        cancel: &dyn Cancellation,
        progress: &dyn ProgressSink,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let categories = [
            CleanupCategory::PackageCache,
            CleanupCategory::ThumbnailCache,
            CleanupCategory::Trash,
            CleanupCategory::UserCache,
            CleanupCategory::Logs,
        ];
        let total = categories.len() as u64;
        let mut targets = Vec::new();

        for (idx, cat) in categories.iter().enumerate() {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            progress.report(&JobProgress {
                completed: idx as u64,
                total: Some(total),
                message: format!("Scanning {}", cat.label()),
            });

            match self.scan_category(*cat) {
                Ok(mut found) => targets.append(&mut found),
                Err(MaintainerError::Io { .. }) => {}
                Err(e) => return Err(e),
            }
        }

        progress.report(&JobProgress {
            completed: total,
            total: Some(total),
            message: "Scan complete".into(),
        });

        Ok(targets)
    }

    /// Scan a single category.
    pub fn scan_category(
        &self,
        category: CleanupCategory,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        match category {
            CleanupCategory::PackageCache => self.scan_package_cache(),
            CleanupCategory::ThumbnailCache => self.scan_thumbnail_cache(),
            CleanupCategory::Trash => self.scan_trash(),
            CleanupCategory::UserCache => self.scan_user_cache(),
            CleanupCategory::Logs => self.scan_logs(),
        }
    }

    fn scan_package_cache(&self) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let cache_dir = PathBuf::from("/var/cache/apt/archives");
        self.scan_directory_files(
            &cache_dir,
            CleanupCategory::PackageCache,
            PrivilegeRequirement::Elevated,
            Reversibility::PermanentOnly,
        )
    }

    fn scan_thumbnail_cache(&self) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let thumb_dir = self.home_dir.join(".cache/thumbnails");
        self.scan_directory_files(
            &thumb_dir,
            CleanupCategory::ThumbnailCache,
            PrivilegeRequirement::None,
            Reversibility::PermanentOnly, // Thumbnails regenerate; no Trash needed.
        )
    }

    fn scan_trash(&self) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let trash_dir = self.home_dir.join(".local/share/Trash/files");
        self.scan_directory_files(
            &trash_dir,
            CleanupCategory::Trash,
            PrivilegeRequirement::None,
            Reversibility::PermanentOnly, // Already in Trash.
        )
    }

    fn scan_user_cache(&self) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let cache_base = self.home_dir.join(".cache");
        let mut targets = Vec::new();
        for root_name in &self.user_cache_roots {
            let dir = cache_base.join(root_name);
            match self.scan_directory_files(
                &dir,
                CleanupCategory::UserCache,
                PrivilegeRequirement::None,
                Reversibility::PermanentOnly, // Cache data regenerates.
            ) {
                Ok(mut found) => targets.append(&mut found),
                Err(MaintainerError::Io { .. }) => {} // Missing dirs are normal.
                Err(e) => return Err(e),
            }
        }
        Ok(targets)
    }

    fn scan_logs(&self) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let user_log_dir = self.home_dir.join(".local/share/systemd");
        let mut targets = Vec::new();
        match self.scan_directory_files(
            &user_log_dir,
            CleanupCategory::Logs,
            PrivilegeRequirement::None,
            Reversibility::PermanentOnly,
        ) {
            Ok(mut found) => targets.append(&mut found),
            Err(MaintainerError::Io { .. }) => {}
            Err(e) => return Err(e),
        }
        Ok(targets)
    }

    /// Scan a directory for files, returning one target per file found.
    fn scan_directory_files(
        &self,
        dir: &Path,
        category: CleanupCategory,
        privilege: PrivilegeRequirement,
        reversibility: Reversibility,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let entries = fs::read_dir(dir)
            .map_err(|e| MaintainerError::io(format!("reading {}", dir.display()), e))?;

        let mut targets = Vec::new();
        let now = SystemTime::now();

        for entry_result in entries {
            let entry = match entry_result {
                Ok(e) => e,
                Err(_) => continue, // Skip unreadable entries.
            };
            let path = entry.path();

            let metadata = match fs::symlink_metadata(&path) {
                Ok(m) => m,
                Err(_) => continue,
            };

            if metadata.is_symlink() {
                continue;
            }

            let size_bytes = if metadata.is_dir() {
                dir_size(&path).unwrap_or(0)
            } else {
                metadata.len()
            };

            if size_bytes == 0 {
                continue; // Skip empty items.
            }

            let display_path = self.make_display_path(&path);

            targets.push(CleanupTarget {
                path,
                category,
                display_path,
                size_bytes,
                privilege,
                reversibility,
                inode: metadata.ino(),
                device: metadata.dev(),
                evidence_timestamp: now,
            });
        }

        Ok(targets)
    }

    /// Produce a display path that elides the home directory.
    fn make_display_path(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home_dir) {
            Ok(relative) => format!("~/{}", relative.display()),
            Err(_) => path.display().to_string(),
        }
    }
}

/// Recursively compute directory size (not following symlinks).
fn dir_size(path: &Path) -> Result<u64, std::io::Error> {
    let mut total = 0u64;
    for entry in fs::read_dir(path)? {
        let entry = entry?;
        let meta = fs::symlink_metadata(entry.path())?;
        if meta.is_dir() {
            total += dir_size(&entry.path()).unwrap_or(0);
        } else if !meta.is_symlink() {
            total += meta.len();
        }
    }
    Ok(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{CancellationToken, NullProgress};
    use std::fs::File;
    use std::io::Write;
    use tempfile::TempDir;

    fn setup_temp_home() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn scan_thumbnail_cache_finds_files() {
        let home = setup_temp_home();
        let thumb_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&thumb_dir).unwrap();

        let mut f = File::create(thumb_dir.join("abc.png")).unwrap();
        f.write_all(&[0u8; 1024]).unwrap();

        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let targets = analyzer
            .scan_category(CleanupCategory::ThumbnailCache)
            .unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].category, CleanupCategory::ThumbnailCache);
        assert_eq!(targets[0].size_bytes, 1024);
        assert_eq!(targets[0].privilege, PrivilegeRequirement::None);
        assert_eq!(targets[0].reversibility, Reversibility::PermanentOnly);
    }

    #[test]
    fn scan_user_cache_only_supported_roots() {
        let home = setup_temp_home();
        let cache_base = home.path().join(".cache");

        let supported = cache_base.join("fontconfig");
        fs::create_dir_all(&supported).unwrap();
        let mut f = File::create(supported.join("cache.dat")).unwrap();
        f.write_all(&[0u8; 512]).unwrap();

        let unsupported = cache_base.join("chromium");
        fs::create_dir_all(&unsupported).unwrap();
        let mut f2 = File::create(unsupported.join("data.bin")).unwrap();
        f2.write_all(&[0u8; 2048]).unwrap();

        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let targets = analyzer.scan_category(CleanupCategory::UserCache).unwrap();

        assert_eq!(targets.len(), 1);
        assert!(targets[0].path.starts_with(&supported));
    }

    #[test]
    fn scan_trash_finds_trashed_items() {
        let home = setup_temp_home();
        let trash_files = home.path().join(".local/share/Trash/files");
        fs::create_dir_all(&trash_files).unwrap();

        let mut f = File::create(trash_files.join("old_doc.txt")).unwrap();
        f.write_all(&[0u8; 256]).unwrap();

        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let targets = analyzer.scan_category(CleanupCategory::Trash).unwrap();

        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].category, CleanupCategory::Trash);
        assert_eq!(targets[0].reversibility, Reversibility::PermanentOnly);
    }

    #[test]
    fn scan_skips_symlinks() {
        let home = setup_temp_home();
        let thumb_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&thumb_dir).unwrap();

        let real_file = thumb_dir.join("real.png");
        let mut f = File::create(&real_file).unwrap();
        f.write_all(&[0u8; 100]).unwrap();

        std::os::unix::fs::symlink(&real_file, thumb_dir.join("link.png")).unwrap();

        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let targets = analyzer
            .scan_category(CleanupCategory::ThumbnailCache)
            .unwrap();

        assert_eq!(targets.len(), 1);
        assert!(targets[0].path.ends_with("real.png"));
    }

    #[test]
    fn scan_skips_empty_files() {
        let home = setup_temp_home();
        let thumb_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&thumb_dir).unwrap();

        File::create(thumb_dir.join("empty.png")).unwrap();

        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let targets = analyzer
            .scan_category(CleanupCategory::ThumbnailCache)
            .unwrap();

        assert!(targets.is_empty());
    }

    #[test]
    fn scan_all_respects_cancellation() {
        let home = setup_temp_home();
        let thumb_dir = home.path().join(".cache/thumbnails");
        fs::create_dir_all(&thumb_dir).unwrap();
        let mut f = File::create(thumb_dir.join("t.png")).unwrap();
        f.write_all(&[0u8; 100]).unwrap();

        let token = CancellationToken::new();
        token.cancel();

        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let result = analyzer.scan_all(&token, &NullProgress);

        assert!(matches!(result, Err(MaintainerError::Cancelled)));
    }

    #[test]
    fn missing_directory_returns_io_error() {
        let home = setup_temp_home();
        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let result = analyzer.scan_category(CleanupCategory::ThumbnailCache);
        assert!(matches!(result, Err(MaintainerError::Io { .. })));
    }

    #[test]
    fn display_path_elides_home() {
        let home = setup_temp_home();
        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let path = home.path().join(".cache/thumbnails/foo.png");
        let display = analyzer.make_display_path(&path);
        assert!(display.starts_with("~/"));
        assert!(display.contains("thumbnails/foo.png"));
    }

    #[test]
    fn display_path_non_home_shows_full() {
        let home = setup_temp_home();
        let analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        let path = PathBuf::from("/var/cache/apt/archives/pkg.deb");
        let display = analyzer.make_display_path(&path);
        assert_eq!(display, "/var/cache/apt/archives/pkg.deb");
    }
}
