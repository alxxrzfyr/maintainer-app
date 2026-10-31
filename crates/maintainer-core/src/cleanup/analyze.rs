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
    data_home: PathBuf,
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
            data_home: crate::cleanup::trash::data_home(
                &home_dir,
                std::env::var_os("XDG_DATA_HOME").as_deref(),
            ),
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

            match self.scan_category_cancellable(*cat, cancel) {
                Ok(mut found) => {
                    if targets.len().saturating_add(found.len()) > 1_000 {
                        return Err(MaintainerError::invalid_data(
                            "cleanup scan",
                            "aggregate target limit reached; results are incomplete",
                        ));
                    }
                    targets.append(&mut found);
                }
                Err(MaintainerError::Io { ref source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound => {}
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
        self.scan_category_cancellable(category, &crate::jobs::CancellationToken::new())
    }

    fn scan_category_cancellable(
        &self,
        category: CleanupCategory,
        cancel: &dyn Cancellation,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        match category {
            CleanupCategory::PackageCache => self.scan_package_cache(cancel),
            CleanupCategory::ThumbnailCache => self.scan_thumbnail_cache(cancel),
            CleanupCategory::Trash => self.scan_trash(cancel),
            CleanupCategory::UserCache => self.scan_user_cache(cancel),
            CleanupCategory::Logs => self.scan_logs(cancel),
        }
    }

    fn scan_package_cache(
        &self,
        cancel: &dyn Cancellation,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let cache_dir = PathBuf::from("/var/cache/apt/archives");
        self.scan_directory_files(
            &cache_dir,
            CleanupCategory::PackageCache,
            PrivilegeRequirement::Elevated,
            cancel,
            Reversibility::PermanentOnly,
        )
    }

    fn scan_thumbnail_cache(
        &self,
        cancel: &dyn Cancellation,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let thumb_dir = self.home_dir.join(".cache/thumbnails");
        self.scan_directory_files(
            &thumb_dir,
            CleanupCategory::ThumbnailCache,
            PrivilegeRequirement::None,
            cancel,
            Reversibility::PermanentOnly, // Thumbnails regenerate; no Trash needed.
        )
    }

    fn scan_trash(&self, cancel: &dyn Cancellation) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let trash_dir = self.data_home.join("Trash/files");
        self.scan_directory_files(
            &trash_dir,
            CleanupCategory::Trash,
            PrivilegeRequirement::None,
            cancel,
            Reversibility::PermanentOnly, // Already in Trash.
        )
    }

    fn scan_user_cache(
        &self,
        cancel: &dyn Cancellation,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let cache_base = self.home_dir.join(".cache");
        let mut targets = Vec::new();
        for root_name in &self.user_cache_roots {
            let dir = cache_base.join(root_name);
            match self.scan_directory_files(
                &dir,
                CleanupCategory::UserCache,
                PrivilegeRequirement::None,
                cancel,
                Reversibility::PermanentOnly, // Cache data regenerates.
            ) {
                Ok(mut found) => {
                    if targets.len().saturating_add(found.len()) > 1_000 {
                        return Err(MaintainerError::invalid_data(
                            "cleanup scan",
                            "aggregate target limit reached; results are incomplete",
                        ));
                    }
                    targets.append(&mut found);
                }
                Err(MaintainerError::Io { ref source, .. })
                    if source.kind() == std::io::ErrorKind::NotFound => {} // Missing dirs are normal.
                Err(e) => return Err(e),
            }
        }
        Ok(targets)
    }

    fn scan_logs(&self, cancel: &dyn Cancellation) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let user_log_dir = self.home_dir.join(".local/share/systemd");
        let mut targets = Vec::new();
        match self.scan_directory_files(
            &user_log_dir,
            CleanupCategory::Logs,
            PrivilegeRequirement::None,
            cancel,
            Reversibility::PermanentOnly,
        ) {
            Ok(mut found) => {
                if targets.len().saturating_add(found.len()) > 1_000 {
                    return Err(MaintainerError::invalid_data(
                        "cleanup scan",
                        "aggregate target limit reached; results are incomplete",
                    ));
                }
                targets.append(&mut found);
            }
            Err(MaintainerError::Io { ref source, .. })
                if source.kind() == std::io::ErrorKind::NotFound => {}
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
        cancel: &dyn Cancellation,
        reversibility: Reversibility,
    ) -> Result<Vec<CleanupTarget>, MaintainerError> {
        let directory = crate::storage::policy::open_anchored(dir, true)
            .map_err(|e| MaintainerError::io(format!("opening {}", dir.display()), e))?;
        let entries = fs::read_dir(crate::storage::policy::descriptor_path(&directory))
            .map_err(|e| MaintainerError::io(format!("reading {}", dir.display()), e))?;

        let mut targets = Vec::new();
        let now = SystemTime::now();

        for entry_result in entries {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            if targets.len() >= 1_000 {
                return Err(MaintainerError::invalid_data(
                    "cleanup scan",
                    "target limit reached; results are incomplete",
                ));
            }
            let entry =
                entry_result.map_err(|e| MaintainerError::io("reading cleanup target", e))?;
            let path = dir.join(entry.file_name());
            let metadata = fs::symlink_metadata(entry.path())
                .map_err(|e| MaintainerError::io("reading cleanup target metadata", e))?;

            if metadata.is_symlink() {
                continue;
            }

            let size_bytes = if metadata.is_dir() {
                let child = crate::storage::policy::open_child_directory(
                    &directory,
                    &entry.file_name(),
                    false,
                )
                .map_err(|e| MaintainerError::io("opening cleanup target", e))?;
                let actual = child
                    .metadata()
                    .map_err(|e| MaintainerError::io("checking cleanup target", e))?;
                if actual.dev() != metadata.dev() || actual.ino() != metadata.ino() {
                    return Err(MaintainerError::invalid_data(
                        "cleanup scan",
                        "target directory changed during discovery",
                    ));
                }
                crate::storage::traverse::directory_size(child, cancel)?
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
    fn trash_scan_uses_the_same_custom_data_home_as_mutation() {
        let home = tempfile::tempdir().unwrap();
        let data = tempfile::tempdir().unwrap();
        let files = data.path().join("Trash/files");
        std::fs::create_dir_all(&files).unwrap();
        std::fs::write(files.join("desktop-visible"), b"trash data").unwrap();
        let mut analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        analyzer.data_home =
            crate::cleanup::trash::data_home(home.path(), Some(data.path().as_os_str()));
        let targets = analyzer
            .scan_trash(&crate::jobs::CancellationToken::new())
            .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].path, files.join("desktop-visible"));
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

        let mut analyzer = CleanupAnalyzer::new(home.path().to_path_buf());
        analyzer.data_home = home.path().join(".local/share");
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
