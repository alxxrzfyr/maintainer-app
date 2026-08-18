use std::collections::HashSet;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Instant;

use crate::error::MaintainerError;
use crate::jobs::{Cancellation, JobProgress, ProgressSink};
use crate::storage::large_files::{LargeFile, LargeFileRanking};
use crate::storage::mounts::{DeviceInode, crosses_filesystem};
use crate::storage::policy::ScanPolicy;

/// Maximum large files to track in the bounded ranking.
const MAX_LARGE_FILES: usize = 200;

/// How many directory entries between cancellation checks within a directory.
const CANCEL_CHECK_INTERVAL: usize = 256;

/// Minimum interval between progress emissions.
const PROGRESS_THROTTLE_MS: u128 = 100;

/// A recoverable issue encountered during scanning.
#[derive(Debug, Clone)]
pub struct ScanIssue {
    /// Path where the issue occurred.
    pub path: PathBuf,
    /// Description of the issue.
    pub message: String,
}

impl ScanIssue {
    fn new(path: PathBuf, message: impl Into<String>) -> Self {
        Self {
            path,
            message: message.into(),
        }
    }
}

/// Per-directory size accumulation.
#[derive(Debug, Clone)]
pub struct DirectoryUsage {
    /// The directory path.
    pub path: PathBuf,
    /// Total apparent size in bytes of all regular files below this directory.
    pub apparent_bytes: u64,
    /// Total allocated bytes (blocks * 512) if available.
    pub allocated_bytes: u64,
    /// Number of regular files.
    pub file_count: u64,
    /// Number of subdirectories.
    pub dir_count: u64,
}

/// Summary result from a completed or cancelled scan.
#[derive(Debug)]
pub struct ScanSummary {
    /// Total apparent bytes across all scanned regular files.
    pub total_apparent_bytes: u64,
    /// Total allocated bytes across all scanned regular files.
    pub total_allocated_bytes: u64,
    /// Total regular files encountered.
    pub total_files: u64,
    /// Total directories traversed.
    pub total_dirs: u64,
    /// Large files ranked by size (largest first).
    pub large_files: Vec<LargeFile>,
    /// Recoverable issues encountered during scanning.
    pub issues: Vec<ScanIssue>,
    /// Whether the scan was cancelled before completion.
    pub was_cancelled: bool,
    /// Set of (device, inode) pairs for hard-linked files seen multiple times.
    /// Content is only counted once in totals.
    pub hard_link_paths: Vec<(DeviceInode, Vec<PathBuf>)>,
}

/// Iterative filesystem scanner with cancellation and progress support.
pub struct StorageScanner<'a> {
    policy: &'a ScanPolicy,
    cancel: &'a dyn Cancellation,
    progress: &'a dyn ProgressSink,
}

impl<'a> StorageScanner<'a> {
    /// Create a scanner with the given policy, cancellation token, and progress
    /// sink.
    pub fn new(
        policy: &'a ScanPolicy,
        cancel: &'a dyn Cancellation,
        progress: &'a dyn ProgressSink,
    ) -> Self {
        Self {
            policy,
            cancel,
            progress,
        }
    }

    /// Execute the scan. Returns a summary or an error (only for true failures
    /// like missing roots; permission issues within the scan are reported as
    /// issues).
    pub fn run(&self) -> crate::error::Result<ScanSummary> {
        if self.policy.roots.is_empty() {
            return Err(MaintainerError::Internal {
                context: "storage scan".into(),
                message: "no scan roots provided".into(),
            });
        }

        let mut total_apparent_bytes: u64 = 0;
        let mut total_allocated_bytes: u64 = 0;
        let mut total_files: u64 = 0;
        let mut total_dirs: u64 = 0;
        let mut issues: Vec<ScanIssue> = Vec::new();
        let mut ranking =
            LargeFileRanking::new(MAX_LARGE_FILES, self.policy.large_file_threshold_bytes);
        let mut was_cancelled = false;

        // Track hard links: inode -> list of paths
        let mut seen_inodes: HashSet<DeviceInode> = HashSet::new();
        let mut hard_link_map: std::collections::HashMap<DeviceInode, Vec<PathBuf>> =
            std::collections::HashMap::new();

        // Track device IDs for each root to detect filesystem boundaries.
        let mut root_devices: std::collections::HashMap<PathBuf, u64> =
            std::collections::HashMap::new();

        // Explicit stack for iterative DFS.
        let mut stack: Vec<PathBuf> = Vec::new();

        // Initialize stack with roots (reverse so first root is processed first).
        for root in self.policy.roots.iter().rev() {
            if self.policy.is_excluded(root) {
                continue;
            }
            match root.symlink_metadata() {
                Ok(meta) => {
                    root_devices.insert(root.clone(), meta.dev());
                    stack.push(root.clone());
                }
                Err(e) => {
                    issues.push(ScanIssue::new(
                        root.clone(),
                        format!("cannot access root: {e}"),
                    ));
                }
            }
        }

        let mut last_progress = Instant::now();
        let mut entries_since_cancel_check: usize;

        while let Some(dir_path) = stack.pop() {
            // Check cancellation at each directory.
            if self.cancel.is_cancelled() {
                was_cancelled = true;
                break;
            }

            total_dirs += 1;

            // Determine root device for this path.
            let root_device = self.find_root_device(&dir_path, &root_devices);

            let read_dir = match fs::read_dir(&dir_path) {
                Ok(rd) => rd,
                Err(e) => {
                    issues.push(ScanIssue::new(
                        dir_path.clone(),
                        format!("cannot read directory: {e}"),
                    ));
                    continue;
                }
            };

            entries_since_cancel_check = 0;

            for entry_result in read_dir {
                entries_since_cancel_check += 1;

                // Periodic cancellation check within a directory.
                if entries_since_cancel_check >= CANCEL_CHECK_INTERVAL {
                    entries_since_cancel_check = 0;
                    if self.cancel.is_cancelled() {
                        was_cancelled = true;
                        break;
                    }
                }

                let entry = match entry_result {
                    Ok(e) => e,
                    Err(e) => {
                        issues.push(ScanIssue::new(
                            dir_path.clone(),
                            format!("entry read error: {e}"),
                        ));
                        continue;
                    }
                };

                let entry_path = entry.path();

                // Check exclusions.
                if self.policy.is_excluded(&entry_path) {
                    continue;
                }

                // Get metadata without following symlinks.
                let meta = if self.policy.follow_symlinks {
                    match fs::metadata(&entry_path) {
                        Ok(m) => m,
                        Err(e) => {
                            issues.push(ScanIssue::new(
                                entry_path.clone(),
                                format!("cannot stat: {e}"),
                            ));
                            continue;
                        }
                    }
                } else {
                    match fs::symlink_metadata(&entry_path) {
                        Ok(m) => m,
                        Err(e) => {
                            issues.push(ScanIssue::new(
                                entry_path.clone(),
                                format!("cannot stat: {e}"),
                            ));
                            continue;
                        }
                    }
                };

                let file_type = meta.file_type();

                // Symlink handling when not following.
                if file_type.is_symlink() {
                    // Record but don't follow.
                    continue;
                }

                if file_type.is_dir() {
                    // Check filesystem boundary.
                    if !self.policy.cross_filesystems {
                        if let Some(root_dev) = root_device {
                            if crosses_filesystem(root_dev, meta.dev()) {
                                continue;
                            }
                        }
                    }
                    stack.push(entry_path);
                } else if file_type.is_file() {
                    let di = DeviceInode::from_metadata(&meta);
                    let nlink = meta.nlink();
                    let apparent = meta.len();
                    let allocated = meta.blocks() * 512;

                    // Hard link handling: count content once.
                    if nlink > 1 {
                        if seen_inodes.contains(&di) {
                            // Already counted; just record the additional path.
                            hard_link_map
                                .entry(di)
                                .or_default()
                                .push(entry_path.clone());
                            // Still offer to large-file ranking (each path listed).
                            ranking.offer(entry_path, apparent);
                            total_files += 1;
                            continue;
                        }
                        seen_inodes.insert(di);
                        hard_link_map
                            .entry(di)
                            .or_default()
                            .push(entry_path.clone());
                    }

                    total_apparent_bytes += apparent;
                    total_allocated_bytes += allocated;
                    total_files += 1;

                    ranking.offer(entry_path, apparent);
                }

                // Throttled progress reporting.
                let now = Instant::now();
                if now.duration_since(last_progress).as_millis() >= PROGRESS_THROTTLE_MS {
                    last_progress = now;
                    self.progress.report(&JobProgress {
                        completed: total_files,
                        total: None,
                        message: format!("Scanning: {} files, {} dirs", total_files, total_dirs),
                    });
                }
            }

            if was_cancelled {
                break;
            }
        }

        // Final progress.
        self.progress.report(&JobProgress {
            completed: total_files,
            total: Some(total_files),
            message: if was_cancelled {
                "Scan cancelled".into()
            } else {
                "Scan complete".into()
            },
        });

        // Collect hard links with multiple paths only.
        let hard_link_paths: Vec<(DeviceInode, Vec<PathBuf>)> = hard_link_map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect();

        Ok(ScanSummary {
            total_apparent_bytes,
            total_allocated_bytes,
            total_files,
            total_dirs,
            large_files: ranking.into_sorted(),
            issues,
            was_cancelled,
            hard_link_paths,
        })
    }

    /// Find the root device for a path by checking which root it's under.
    fn find_root_device(
        &self,
        path: &std::path::Path,
        root_devices: &std::collections::HashMap<PathBuf, u64>,
    ) -> Option<u64> {
        for (root, dev) in root_devices {
            if path.starts_with(root) {
                return Some(*dev);
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{CancellationToken, NullProgress};
    use std::os::unix::fs as unix_fs;
    use tempfile::TempDir;

    fn null_cancel() -> CancellationToken {
        CancellationToken::new()
    }

    #[test]
    fn scan_empty_roots_returns_error() {
        let policy = ScanPolicy::default();
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let result = scanner.run();
        assert!(result.is_err());
    }

    #[test]
    fn scan_basic_directory_counts_files() {
        let tmp = TempDir::new().unwrap();
        fs::write(tmp.path().join("a.txt"), b"hello").unwrap();
        fs::write(tmp.path().join("b.txt"), b"world!").unwrap();
        fs::create_dir(tmp.path().join("sub")).unwrap();
        fs::write(tmp.path().join("sub/c.txt"), b"nested").unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert_eq!(summary.total_files, 3);
        assert_eq!(summary.total_apparent_bytes, 5 + 6 + 6);
        assert!(!summary.was_cancelled);
    }

    #[test]
    fn scan_respects_exclusions() {
        let tmp = TempDir::new().unwrap();
        let excluded = tmp.path().join("excluded");
        fs::create_dir(&excluded).unwrap();
        fs::write(excluded.join("secret.txt"), b"data").unwrap();
        fs::write(tmp.path().join("visible.txt"), b"ok").unwrap();

        let policy = ScanPolicy {
            roots: vec![tmp.path().to_path_buf()],
            exclusions: vec![excluded],
            ..Default::default()
        };
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert_eq!(summary.total_files, 1);
        assert_eq!(summary.total_apparent_bytes, 2);
    }

    #[test]
    fn scan_cancellation_stops_early() {
        let tmp = TempDir::new().unwrap();
        for i in 0..10 {
            fs::write(tmp.path().join(format!("{i}.txt")), b"x").unwrap();
        }

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = CancellationToken::new();
        cancel.cancel(); // Cancel immediately.
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert!(summary.was_cancelled);
    }

    #[test]
    fn scan_does_not_follow_symlinks_by_default() {
        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target.txt");
        fs::write(&target, b"data").unwrap();
        let link = tmp.path().join("link.txt");
        unix_fs::symlink(&target, &link).unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        // Only the real file, not the symlink target counted again.
        assert_eq!(summary.total_files, 1);
    }

    #[test]
    fn scan_detects_hard_links() {
        let tmp = TempDir::new().unwrap();
        let original = tmp.path().join("original.txt");
        fs::write(&original, b"shared content").unwrap();
        let hardlink = tmp.path().join("hardlink.txt");
        fs::hard_link(&original, &hardlink).unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        // Content counted once.
        assert_eq!(summary.total_apparent_bytes, 14); // "shared content" = 14 bytes
        // Both paths counted as files.
        assert_eq!(summary.total_files, 2);
        // Hard link recorded.
        assert_eq!(summary.hard_link_paths.len(), 1);
        assert_eq!(summary.hard_link_paths[0].1.len(), 2);
    }

    #[test]
    fn scan_reports_unreadable_directory() {
        use std::os::unix::fs::PermissionsExt;

        let tmp = TempDir::new().unwrap();
        let unreadable = tmp.path().join("noperm");
        fs::create_dir(&unreadable).unwrap();
        fs::write(unreadable.join("hidden.txt"), b"secret").unwrap();
        // Remove read permission on directory.
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        // Should have at least one issue.
        assert!(!summary.issues.is_empty());

        // Clean up permissions so tempfile can remove it.
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn scan_large_files_detected() {
        let tmp = TempDir::new().unwrap();
        // Use a small threshold for testing.
        let policy = ScanPolicy {
            roots: vec![tmp.path().to_path_buf()],
            large_file_threshold_bytes: 10,
            ..Default::default()
        };

        fs::write(tmp.path().join("small.txt"), b"tiny").unwrap();
        fs::write(
            tmp.path().join("big.txt"),
            b"this is more than ten bytes long",
        )
        .unwrap();

        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert_eq!(summary.large_files.len(), 1);
        assert!(summary.large_files[0].size_bytes >= 10);
    }

    #[test]
    fn scan_symlink_cycle_does_not_hang() {
        let tmp = TempDir::new().unwrap();
        let dir_a = tmp.path().join("a");
        fs::create_dir(&dir_a).unwrap();
        // Create a symlink cycle: a/link -> parent
        unix_fs::symlink(tmp.path(), dir_a.join("cycle")).unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        // Should complete without hanging (symlinks not followed).
        let summary = scanner.run().unwrap();
        assert!(!summary.was_cancelled);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scan_non_utf8_filename() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let tmp = TempDir::new().unwrap();
        // Create a file with non-UTF-8 name.
        let bad_name = OsStr::from_bytes(b"bad\xff\xfename.txt");
        let bad_path = tmp.path().join(bad_name);
        fs::write(&bad_path, b"content").unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert_eq!(summary.total_files, 1);
        assert_eq!(summary.total_apparent_bytes, 7);
    }

    #[test]
    fn scan_nonexistent_root_reported_as_issue() {
        let policy = ScanPolicy::with_roots(vec![PathBuf::from("/nonexistent_root_xyz_999")]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert!(!summary.issues.is_empty());
        assert_eq!(summary.total_files, 0);
    }
}
