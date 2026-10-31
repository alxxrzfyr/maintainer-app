use std::collections::HashSet;
use std::fs;
use std::mem::size_of;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::Instant;

use crate::error::MaintainerError;
use crate::jobs::{Cancellation, JobProgress, ProgressSink};
use crate::storage::duplicates::DuplicateCandidate;
use crate::storage::large_files::{LargeFile, LargeFileRanking};
use crate::storage::mounts::{DeviceInode, crosses_filesystem};
use crate::storage::policy::ScanPolicy;

/// Maximum large files to track in the bounded ranking.
const MAX_LARGE_FILES: usize = 200;

/// Bound duplicate discovery independently from the large-file ranking.
const MAX_DUPLICATE_CANDIDATES: usize = 100_000;
const MAX_DUPLICATE_CANDIDATE_BYTES: usize = 20 * 1024 * 1024;
const MAX_SCAN_ISSUES: usize = 1_000;
const MAX_SCAN_FRONTIER_ENTRIES: usize = 256;
const MAX_SCAN_FRONTIER_BYTES: usize = 20 * 1024 * 1024;
const MAX_HARD_LINK_INDEX_ENTRIES: usize = 100_000;
const MAX_HARD_LINK_PATHS: usize = 100_000;
const MAX_HARD_LINK_PATH_BYTES: usize = 8 * 1024 * 1024;

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

fn record_issue(issues: &mut Vec<ScanIssue>, omitted: &mut u64, issue: ScanIssue) {
    if issues.len() < MAX_SCAN_ISSUES - 1 {
        issues.push(issue);
    } else {
        *omitted = omitted.saturating_add(1);
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
    /// File candidates for duplicate discovery, independent of large-file ranking.
    pub duplicate_candidates: Vec<DuplicateCandidate>,
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
    collect_duplicates: bool,
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
            collect_duplicates: false,
            cancel,
            progress,
        }
    }

    /// Include bounded duplicate metadata only when duplicate discovery is requested.
    pub fn with_duplicate_candidates(mut self) -> Self {
        self.collect_duplicates = true;
        self
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

        if self.policy.follow_symlinks {
            return Err(MaintainerError::invalid_data(
                "storage scan",
                "following symbolic links is unsupported by anchored scans",
            ));
        }
        let mut total_apparent_bytes: u64 = 0;
        let mut total_allocated_bytes: u64 = 0;
        let mut total_files: u64 = 0;
        let mut total_dirs: u64 = 0;
        let mut issues: Vec<ScanIssue> = Vec::new();
        let mut omitted_issue_count = 0u64;
        let mut ranking =
            LargeFileRanking::new(MAX_LARGE_FILES, self.policy.large_file_threshold_bytes);
        let mut duplicate_candidates = Vec::new();
        let mut duplicate_candidate_bytes = 0usize;
        let mut duplicate_candidates_truncated = false;
        let mut was_cancelled = false;

        let mut seen_inodes: HashSet<DeviceInode> = HashSet::new();
        let mut hard_link_map: std::collections::HashMap<DeviceInode, Vec<PathBuf>> =
            std::collections::HashMap::new();

        let mut stack: Vec<(PathBuf, u16, fs::File, u64)> = Vec::new();
        let mut retained_budget = self
            .policy
            .roots
            .capacity()
            .saturating_add(self.policy.exclusions.capacity())
            .saturating_mul(size_of::<PathBuf>())
            .saturating_add(128 * 1024);
        for path in self.policy.roots.iter().chain(&self.policy.exclusions) {
            retained_budget = retained_budget.saturating_add(path.capacity());
        }
        let mut budget_truncated = false;
        let mut frontier_bytes = 0usize;
        let mut frontier_truncated = false;
        let mut depth_truncated = false;
        let mut hard_link_index_truncated = false;
        let mut hard_link_path_count = 0usize;
        let mut hard_link_path_bytes = 0usize;

        for root in self.policy.roots.iter().rev() {
            retained_budget = retained_budget
                .saturating_add(root.capacity().saturating_mul(4).saturating_add(4096));
            if retained_budget > 64 * 1024 * 1024 {
                budget_truncated = true;
                break;
            }
            if self.policy.is_excluded(root) {
                continue;
            }
            match crate::storage::policy::open_anchored(root, true)
                .and_then(|file| file.metadata().map(|meta| (file, meta)))
            {
                Ok((_, meta)) if !meta.is_dir() => {
                    record_issue(
                        &mut issues,
                        &mut omitted_issue_count,
                        ScanIssue::new(
                            root.clone(),
                            "scan root must be a directory, not a symlink or file",
                        ),
                    );
                }
                Ok((file, meta)) => {
                    let root_cost = path_storage_cost(root);
                    if stack.len() < MAX_SCAN_FRONTIER_ENTRIES
                        && frontier_bytes.saturating_add(root_cost) <= MAX_SCAN_FRONTIER_BYTES
                    {
                        frontier_bytes += root_cost;
                        stack.push((root.clone(), 0, file, meta.dev()));
                    } else {
                        frontier_truncated = true;
                    }
                }
                Err(e) => {
                    record_issue(
                        &mut issues,
                        &mut omitted_issue_count,
                        ScanIssue::new(root.clone(), format!("cannot access root: {e}")),
                    );
                }
            }
        }

        if budget_truncated || frontier_truncated {
            stack.clear();
        }
        let mut last_progress = Instant::now();
        let mut entries_since_cancel_check: usize;

        'scan: while let Some((dir_path, depth, directory, device)) = stack.pop() {
            frontier_bytes = frontier_bytes.saturating_sub(path_storage_cost(&dir_path));
            if self.cancel.is_cancelled() {
                was_cancelled = true;
                break;
            }

            total_dirs += 1;

            let root_device = Some(device);

            let read_dir = match fs::read_dir(crate::storage::policy::descriptor_path(&directory)) {
                Ok(rd) => rd,
                Err(e) => {
                    record_issue(
                        &mut issues,
                        &mut omitted_issue_count,
                        ScanIssue::new(dir_path.clone(), format!("cannot read directory: {e}")),
                    );
                    continue;
                }
            };

            entries_since_cancel_check = 0;

            for entry_result in read_dir {
                entries_since_cancel_check += 1;

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
                        record_issue(
                            &mut issues,
                            &mut omitted_issue_count,
                            ScanIssue::new(dir_path.clone(), format!("entry read error: {e}")),
                        );
                        continue;
                    }
                };

                let entry_path = dir_path.join(entry.file_name());
                // Conservative monotonic ledger: charge all retained clones, hash-table
                // load-factor slack, ranking and issue storage before accepting work.
                let charge = entry_path.capacity().saturating_mul(8).saturating_add(4096);
                retained_budget = retained_budget.saturating_add(charge);
                if retained_budget > 64 * 1024 * 1024 {
                    budget_truncated = true;
                    break 'scan;
                }

                if self.policy.is_excluded(&entry_path) {
                    continue;
                }

                let meta = match fs::symlink_metadata(entry.path()) {
                    Ok(meta) => meta,
                    Err(error) => {
                        record_issue(
                            &mut issues,
                            &mut omitted_issue_count,
                            ScanIssue::new(entry_path.clone(), format!("cannot stat: {error}")),
                        );
                        continue;
                    }
                };

                let file_type = meta.file_type();

                if file_type.is_symlink() {
                    continue;
                }

                if file_type.is_dir() {
                    if depth >= 256 {
                        depth_truncated = true;
                        break 'scan;
                    }
                    if !self.policy.cross_filesystems {
                        if let Some(root_dev) = root_device {
                            if crosses_filesystem(root_dev, meta.dev()) {
                                continue;
                            }
                        }
                    }
                    let path_cost = path_storage_cost(&entry_path);
                    if stack.len() < MAX_SCAN_FRONTIER_ENTRIES
                        && frontier_bytes.saturating_add(path_cost) <= MAX_SCAN_FRONTIER_BYTES
                    {
                        frontier_bytes += path_cost;
                        match crate::storage::policy::open_child_directory(
                            &directory,
                            &entry.file_name(),
                            self.policy.cross_filesystems,
                        ) {
                            Ok(child) => {
                                let actual = child.metadata().map_err(|e| {
                                    MaintainerError::io("checking opened directory", e)
                                })?;
                                if actual.dev() != meta.dev() || actual.ino() != meta.ino() {
                                    record_issue(
                                        &mut issues,
                                        &mut omitted_issue_count,
                                        ScanIssue::new(
                                            entry_path,
                                            "directory changed before opening",
                                        ),
                                    );
                                    continue;
                                }
                                stack.push((entry_path, depth + 1, child, device));
                            }
                            Err(error) => record_issue(
                                &mut issues,
                                &mut omitted_issue_count,
                                ScanIssue::new(
                                    entry_path,
                                    format!("cannot open directory: {error}"),
                                ),
                            ),
                        }
                    } else {
                        frontier_truncated = true;
                        break 'scan;
                    }
                } else if file_type.is_file() {
                    let di = DeviceInode::from_metadata(&meta);
                    let candidate_bytes = size_of::<DuplicateCandidate>()
                        .saturating_add(entry_path.as_os_str().as_bytes().len());
                    if self.collect_duplicates
                        && duplicate_candidates.len() < MAX_DUPLICATE_CANDIDATES
                        && duplicate_candidate_bytes.saturating_add(candidate_bytes)
                            <= MAX_DUPLICATE_CANDIDATE_BYTES
                    {
                        match meta.modified() {
                            Ok(modified) => {
                                duplicate_candidate_bytes += candidate_bytes;
                                duplicate_candidates.push(DuplicateCandidate {
                                    path: entry_path.clone(),
                                    size_bytes: meta.len(),
                                    identity: di,
                                    modified,
                                });
                            }
                            Err(error) => record_issue(
                                &mut issues,
                                &mut omitted_issue_count,
                                ScanIssue::new(
                                    entry_path.clone(),
                                    format!("cannot read modification time: {error}"),
                                ),
                            ),
                        }
                    } else if self.collect_duplicates {
                        duplicate_candidates_truncated = true;
                        break 'scan;
                    }
                    let nlink = meta.nlink();
                    let apparent = meta.len();
                    let allocated = meta.blocks() * 512;

                    if nlink > 1 {
                        if seen_inodes.contains(&di) {
                            let path_cost = path_storage_cost(&entry_path);
                            if hard_link_path_count < MAX_HARD_LINK_PATHS
                                && hard_link_path_bytes.saturating_add(path_cost)
                                    <= MAX_HARD_LINK_PATH_BYTES
                            {
                                hard_link_map
                                    .entry(di)
                                    .or_default()
                                    .push(entry_path.clone());
                                hard_link_path_count += 1;
                                hard_link_path_bytes += path_cost;
                            } else {
                                hard_link_index_truncated = true;
                                break 'scan;
                            }
                            ranking.offer(entry_path, apparent);
                            total_files += 1;
                            continue;
                        }
                        if seen_inodes.len() < MAX_HARD_LINK_INDEX_ENTRIES {
                            seen_inodes.insert(di);
                            let path_cost = path_storage_cost(&entry_path);
                            if hard_link_path_count < MAX_HARD_LINK_PATHS
                                && hard_link_path_bytes.saturating_add(path_cost)
                                    <= MAX_HARD_LINK_PATH_BYTES
                            {
                                hard_link_map
                                    .entry(di)
                                    .or_default()
                                    .push(entry_path.clone());
                                hard_link_path_count += 1;
                                hard_link_path_bytes += path_cost;
                            } else {
                                hard_link_index_truncated = true;
                                break 'scan;
                            }
                        } else {
                            hard_link_index_truncated = true;
                            break 'scan;
                        }
                    }

                    total_apparent_bytes += apparent;
                    total_allocated_bytes += allocated;
                    total_files += 1;

                    ranking.offer(entry_path, apparent);
                }

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

        self.progress.report(&JobProgress {
            completed: total_files,
            total: Some(total_files),
            message: if was_cancelled {
                "Scan cancelled".into()
            } else if budget_truncated
                || depth_truncated
                || frontier_truncated
                || duplicate_candidates_truncated
                || hard_link_index_truncated
            {
                "Scan partial: working-set limit reached".into()
            } else {
                "Scan complete".into()
            },
        });

        let hard_link_paths: Vec<(DeviceInode, Vec<PathBuf>)> = hard_link_map
            .into_iter()
            .filter(|(_, paths)| paths.len() > 1)
            .collect();

        if budget_truncated {
            issues.push(ScanIssue::new(
                PathBuf::new(),
                "64 MiB scan working-set budget reached; scan coverage is incomplete",
            ));
        }
        if duplicate_candidates_truncated {
            record_issue(
                &mut issues,
                &mut omitted_issue_count,
                ScanIssue::new(
                    self.policy.roots[0].clone(),
                    "duplicate candidate limit reached; duplicate results may be incomplete",
                ),
            );
        }
        if depth_truncated {
            record_issue(
                &mut issues,
                &mut omitted_issue_count,
                ScanIssue::new(
                    self.policy.roots[0].clone(),
                    "directory depth limit reached; scan coverage is incomplete",
                ),
            );
        }
        if frontier_truncated {
            record_issue(
                &mut issues,
                &mut omitted_issue_count,
                ScanIssue::new(
                    self.policy.roots[0].clone(),
                    "directory frontier limit reached; scan coverage is incomplete",
                ),
            );
        }
        if hard_link_index_truncated {
            record_issue(
                &mut issues,
                &mut omitted_issue_count,
                ScanIssue::new(
                    self.policy.roots[0].clone(),
                    "hard-link index limit reached; storage totals may be incomplete",
                ),
            );
        }
        if omitted_issue_count > 0 {
            issues.push(ScanIssue::new(
                self.policy.roots[0].clone(),
                format!("{omitted_issue_count} additional scan issues omitted"),
            ));
        }

        Ok(ScanSummary {
            total_apparent_bytes,
            total_allocated_bytes,
            total_files,
            total_dirs,
            large_files: ranking.into_sorted(),
            duplicate_candidates,
            issues,
            was_cancelled,
            hard_link_paths,
        })
    }
}

fn path_storage_cost(path: &std::path::Path) -> usize {
    size_of::<PathBuf>().saturating_add(path.as_os_str().as_bytes().len())
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
    fn symlink_root_is_not_traversed() {
        let temporary = TempDir::new().unwrap();
        let actual = temporary.path().join("actual");
        fs::create_dir(&actual).unwrap();
        fs::write(actual.join("file"), b"data").unwrap();
        let link = temporary.path().join("link");
        unix_fs::symlink(actual, &link).unwrap();
        let policy = ScanPolicy::with_roots(vec![link]);
        let summary = StorageScanner::new(&policy, &null_cancel(), &NullProgress)
            .run()
            .unwrap();
        assert_eq!(summary.total_files, 0);
        assert_eq!(summary.issues.len(), 1);
    }

    #[test]
    fn directory_depth_limit_reports_incomplete_coverage() {
        let temporary = TempDir::new().unwrap();
        let mut directory = temporary.path().to_path_buf();
        for _ in 0..258 {
            directory.push("d");
            fs::create_dir(&directory).unwrap();
        }
        fs::write(directory.join("out-of-depth"), b"data").unwrap();
        let policy = ScanPolicy::with_roots(vec![temporary.path().to_path_buf()]);
        let summary = StorageScanner::new(&policy, &null_cancel(), &NullProgress)
            .run()
            .unwrap();
        assert_eq!(summary.total_files, 0);
        assert!(
            summary
                .issues
                .iter()
                .any(|issue| issue.message.contains("depth limit"))
        );
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
        assert!(summary.duplicate_candidates.is_empty());
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

        assert_eq!(summary.total_apparent_bytes, 14); // "shared content" = 14 bytes
        assert_eq!(summary.total_files, 2);
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
        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o000)).unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();

        assert!(!summary.issues.is_empty());

        fs::set_permissions(&unreadable, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn scan_large_files_detected() {
        let tmp = TempDir::new().unwrap();
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
    fn duplicate_candidates_include_files_outside_large_file_ranking() {
        let tmp = TempDir::new().unwrap();
        for index in 0..202 {
            fs::write(tmp.path().join(format!("copy-{index}")), b"same").unwrap();
        }

        let policy = ScanPolicy {
            roots: vec![tmp.path().to_path_buf()],
            large_file_threshold_bytes: 1,
            ..Default::default()
        };
        let cancel = null_cancel();
        let summary = StorageScanner::new(&policy, &cancel, &NullProgress)
            .with_duplicate_candidates()
            .run()
            .unwrap();

        assert_eq!(summary.large_files.len(), 200);
        assert_eq!(summary.duplicate_candidates.len(), 202);
        assert!(
            summary
                .duplicate_candidates
                .iter()
                .all(|candidate| candidate.identity.inode != 0)
        );

        let duplicates = crate::storage::DuplicateScanner::new(&cancel, &NullProgress)
            .scan(summary.duplicate_candidates)
            .unwrap();
        assert_eq!(duplicates.groups.len(), 1);
        assert_eq!(duplicates.groups[0].files.len(), 202);
    }

    #[test]
    fn scan_symlink_cycle_does_not_hang() {
        let tmp = TempDir::new().unwrap();
        let dir_a = tmp.path().join("a");
        fs::create_dir(&dir_a).unwrap();
        unix_fs::symlink(tmp.path(), dir_a.join("cycle")).unwrap();

        let policy = ScanPolicy::with_roots(vec![tmp.path().to_path_buf()]);
        let cancel = null_cancel();
        let progress = NullProgress;
        let scanner = StorageScanner::new(&policy, &cancel, &progress);
        let summary = scanner.run().unwrap();
        assert!(!summary.was_cancelled);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scan_non_utf8_filename() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;

        let tmp = TempDir::new().unwrap();
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

/// Bounded cancellable size walk used by cleanup discovery. Owned directory
/// descriptors keep queued work attached to discovered directories after renames.
pub(crate) fn directory_size(
    directory: fs::File,
    cancel: &dyn Cancellation,
) -> crate::error::Result<u64> {
    let device = directory
        .metadata()
        .map_err(|e| MaintainerError::io("reading cleanup directory", e))?
        .dev();
    let mut stack = vec![(directory, 0usize)];
    let mut total = 0u64;
    let mut budget = 0usize;
    while let Some((directory, depth)) = stack.pop() {
        for entry in fs::read_dir(crate::storage::policy::descriptor_path(&directory))
            .map_err(|e| MaintainerError::io("reading cleanup directory", e))?
        {
            if cancel.is_cancelled() {
                return Err(MaintainerError::Cancelled);
            }
            budget = budget.saturating_add(4096);
            if budget > 64 * 1024 * 1024 {
                return Err(MaintainerError::invalid_data(
                    "cleanup scan",
                    "working-set limit reached; size is incomplete",
                ));
            }
            let entry = entry.map_err(|e| MaintainerError::io("reading cleanup entry", e))?;
            let metadata = entry
                .path()
                .symlink_metadata()
                .map_err(|e| MaintainerError::io("reading cleanup metadata", e))?;
            if metadata.is_symlink() {
                continue;
            }
            if metadata.dev() != device {
                return Err(MaintainerError::invalid_data(
                    "cleanup scan",
                    "filesystem boundary reached; size is incomplete",
                ));
            }
            if metadata.is_dir() {
                if depth >= 256 || stack.len() >= MAX_SCAN_FRONTIER_ENTRIES {
                    return Err(MaintainerError::invalid_data(
                        "cleanup scan",
                        "directory limit reached; size is incomplete",
                    ));
                }
                let child = crate::storage::policy::open_child_directory(
                    &directory,
                    &entry.file_name(),
                    false,
                )
                .map_err(|e| MaintainerError::io("opening cleanup directory", e))?;
                let actual = child
                    .metadata()
                    .map_err(|e| MaintainerError::io("checking cleanup directory", e))?;
                if actual.dev() != metadata.dev() || actual.ino() != metadata.ino() {
                    return Err(MaintainerError::invalid_data(
                        "cleanup scan",
                        "directory changed during scan",
                    ));
                }
                stack.push((child, depth + 1));
            } else if metadata.is_file() {
                total = total.saturating_add(metadata.len());
            }
        }
    }
    Ok(total)
}

#[cfg(test)]
mod cleanup_walk_tests {
    use super::*;
    #[test]
    fn cleanup_size_walk_cancels_between_entries() {
        let temporary = tempfile::tempdir().unwrap();
        fs::write(temporary.path().join("file"), b"data").unwrap();
        let directory = crate::storage::policy::open_anchored(temporary.path(), true).unwrap();
        let cancel = crate::jobs::CancellationToken::new();
        cancel.cancel();
        assert!(matches!(
            directory_size(directory, &cancel),
            Err(MaintainerError::Cancelled)
        ));
    }
}
