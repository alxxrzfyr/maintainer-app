//! Confirmed duplicate file detection using staged hashing.
//!
//! Pipeline:
//! 1. Group files by size (different sizes cannot be duplicates).
//! 2. Within each size group, compute partial BLAKE3 (prefix + suffix).
//! 3. Files with matching partial hashes get a full BLAKE3 confirmation.
//! 4. Re-stat before and after full hashing to reject files that changed.
//!
//! The scanner never chooses a file to remove. It produces `DuplicateGroup`
//! values representing confirmed identical content.

use std::collections::HashMap;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::error::MaintainerError;
use crate::jobs::{Cancellation, JobProgress, ProgressSink};
use crate::storage::hash::{ContentDigest, full_hash_expected, partial_hash_expected};
use crate::storage::mounts::DeviceInode;

/// Minimum file size to consider for duplicate scanning.
/// Files of zero bytes are trivially identical but not useful duplicates.
const MIN_DUPLICATE_SIZE_BYTES: u64 = 1;

/// A confirmed duplicate file within a group.
#[derive(Debug, Clone)]
pub struct DuplicateFile {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// Device and inode identity.
    pub identity: DeviceInode,
    /// Last modification time at confirmation.
    pub modified: SystemTime,
}

/// A group of files confirmed to have identical content.
#[derive(Debug, Clone)]
pub struct DuplicateGroup {
    /// BLAKE3 digest of the shared content.
    pub digest: ContentDigest,
    /// Size in bytes (all files in the group share this size).
    pub size_bytes: u64,
    /// The files in this group (always >= 2).
    pub files: Vec<DuplicateFile>,
}

/// A recoverable issue encountered during duplicate scanning.
#[derive(Debug, Clone)]
pub struct DuplicateIssue {
    /// Path where the issue occurred.
    pub path: PathBuf,
    /// Description of the issue.
    pub message: String,
}

/// Result of a duplicate scan.
#[derive(Debug)]
pub struct DuplicateScanResult {
    /// Confirmed duplicate groups.
    pub groups: Vec<DuplicateGroup>,
    /// Recoverable issues encountered.
    pub issues: Vec<DuplicateIssue>,
    pub omitted_issues: u64,
    /// Whether the scan was cancelled.
    pub was_cancelled: bool,
    /// Total bytes hashed during the scan.
    pub bytes_hashed: u64,
    /// Total files considered as candidates.
    pub files_considered: u64,
}

impl DuplicateScanResult {
    fn record_issue(&mut self, issue: DuplicateIssue) {
        if self.issues.len() < 1_000 {
            self.issues.push(issue);
        } else {
            self.omitted_issues = self.omitted_issues.saturating_add(1);
        }
    }
}

/// Input candidate for duplicate scanning: a file with its metadata.
#[derive(Debug, Clone)]
pub struct DuplicateCandidate {
    /// Absolute path to the file.
    pub path: PathBuf,
    /// File size in bytes.
    pub size_bytes: u64,
    /// Device and inode identity.
    pub identity: DeviceInode,
    /// Last modification time.
    pub modified: SystemTime,
}

/// File metadata snapshot for stability verification.
#[derive(Debug, Clone, PartialEq, Eq)]
struct FileSnapshot {
    device: u64,
    inode: u64,
    size: u64,
    mtime_secs: i64,
    mtime_nsecs: i64,
}

impl FileSnapshot {
    fn capture(path: &Path) -> std::io::Result<Self> {
        let meta = path.symlink_metadata()?;
        Ok(Self {
            device: meta.dev(),
            inode: meta.ino(),
            size: meta.size(),
            mtime_secs: meta.mtime(),
            mtime_nsecs: meta.mtime_nsec(),
        })
    }
}

/// Scanner that detects duplicate files through staged hashing.
///
/// Processes one size group at a time to bound memory usage.
/// Does not choose files for deletion.
pub struct DuplicateScanner<'a> {
    cancel: &'a dyn Cancellation,
    progress: &'a dyn ProgressSink,
}

impl<'a> DuplicateScanner<'a> {
    /// Create a new scanner with the given cancellation and progress handles.
    pub fn new(cancel: &'a dyn Cancellation, progress: &'a dyn ProgressSink) -> Self {
        Self { cancel, progress }
    }

    /// Run the duplicate detection pipeline on the given candidates.
    ///
    /// # Errors
    ///
    /// Returns `MaintainerError::Cancelled` if cancellation is requested.
    /// Other I/O errors on individual files are recorded as issues.
    pub fn scan(
        &self,
        candidates: Vec<DuplicateCandidate>,
    ) -> crate::error::Result<DuplicateScanResult> {
        // Reserve headroom for size/hash maps, index vectors, confirmed paths and
        // bounded issues while candidates remain alive. Reject before grouping.
        let retained = candidates
            .capacity()
            .saturating_mul(std::mem::size_of::<DuplicateCandidate>())
            .saturating_add(
                candidates
                    .iter()
                    .map(|candidate| {
                        candidate
                            .path
                            .capacity()
                            .saturating_mul(4)
                            .saturating_add(1024)
                    })
                    .fold(0usize, usize::saturating_add),
            );
        if retained > 64 * 1024 * 1024 {
            return Err(MaintainerError::invalid_data(
                "duplicate scan",
                "64 MiB working-set budget reached before hashing; results are incomplete",
            ));
        }
        let files_considered = candidates.len() as u64;
        let mut result = DuplicateScanResult {
            groups: Vec::new(),
            issues: Vec::new(),
            omitted_issues: 0,
            was_cancelled: false,
            bytes_hashed: 0,
            files_considered,
        };

        if self.cancel.is_cancelled() {
            result.was_cancelled = true;
            return Ok(result);
        }

        let size_groups = self.group_by_size(candidates);

        self.progress.report(&JobProgress {
            completed: 0,
            total: Some(size_groups.len() as u64),
            message: "Grouping files by size".into(),
        });

        let total_groups = size_groups.len() as u64;
        for (group_idx, (size, files)) in size_groups.into_iter().enumerate() {
            if self.cancel.is_cancelled() {
                result.was_cancelled = true;
                return Ok(result);
            }

            self.progress.report(&JobProgress {
                completed: group_idx as u64,
                total: Some(total_groups),
                message: format!("Processing size group {size} bytes"),
            });

            let distinct_inodes = distinct_inode_count(&files);
            if distinct_inodes < 2 {
                continue;
            }

            let partial_groups = self.partial_hash_group(size, &files, &mut result);

            for (_partial_digest, partial_matches) in partial_groups {
                if partial_matches.len() < 2 {
                    continue;
                }
                if distinct_inode_count_candidates(&files, &partial_matches) < 2 {
                    continue;
                }

                self.full_hash_group(size, &files, partial_matches, &mut result)?;
            }
        }

        self.progress.report(&JobProgress {
            completed: total_groups,
            total: Some(total_groups),
            message: "Duplicate scan complete".into(),
        });

        Ok(result)
    }

    /// Group candidates by file size, filtering out sizes with no potential duplicates.
    fn group_by_size(
        &self,
        candidates: Vec<DuplicateCandidate>,
    ) -> Vec<(u64, Vec<DuplicateCandidate>)> {
        let mut groups: HashMap<u64, Vec<DuplicateCandidate>> = HashMap::new();
        for candidate in candidates {
            if candidate.size_bytes < MIN_DUPLICATE_SIZE_BYTES {
                continue;
            }
            groups
                .entry(candidate.size_bytes)
                .or_default()
                .push(candidate);
        }

        groups
            .into_iter()
            .filter(|(_, files)| files.len() >= 2)
            .collect()
    }

    /// Compute partial hashes for a size group, returning sub-groups by partial digest.
    fn partial_hash_group(
        &self,
        size: u64,
        files: &[DuplicateCandidate],
        result: &mut DuplicateScanResult,
    ) -> HashMap<ContentDigest, Vec<usize>> {
        let mut partial_groups: HashMap<ContentDigest, Vec<usize>> = HashMap::new();

        for (index, candidate) in files.iter().enumerate() {
            if self.cancel.is_cancelled() {
                return partial_groups;
            }

            match partial_hash_expected(&candidate.path, size, candidate.identity, self.cancel) {
                Ok(digest) => {
                    partial_groups.entry(digest).or_default().push(index);
                }
                Err(MaintainerError::Cancelled) => {
                    return partial_groups;
                }
                Err(e) => {
                    result.record_issue(DuplicateIssue {
                        path: candidate.path.clone(),
                        message: format!("partial hash failed: {e}"),
                    });
                }
            }
        }

        partial_groups
    }

    /// Perform full hashing with pre/post stat verification.
    fn full_hash_group(
        &self,
        size: u64,
        candidates: &[DuplicateCandidate],
        indexes: Vec<usize>,
        result: &mut DuplicateScanResult,
    ) -> crate::error::Result<()> {
        let mut confirmed: HashMap<ContentDigest, Vec<DuplicateFile>> = HashMap::new();

        for index in indexes {
            let candidate = &candidates[index];
            if self.cancel.is_cancelled() {
                result.was_cancelled = true;
                return Ok(());
            }

            let pre_stat = match FileSnapshot::capture(&candidate.path) {
                Ok(s) => s,
                Err(e) => {
                    result.record_issue(DuplicateIssue {
                        path: candidate.path.clone(),
                        message: format!("pre-hash stat failed: {e}"),
                    });
                    continue;
                }
            };

            if pre_stat.size != candidate.size_bytes
                || pre_stat.device != candidate.identity.device
                || pre_stat.inode != candidate.identity.inode
            {
                result.record_issue(DuplicateIssue {
                    path: candidate.path.clone(),
                    message: "file changed between discovery and hashing".into(),
                });
                continue;
            }

            let (digest, bytes_read) = match full_hash_expected(
                &candidate.path,
                candidate.identity,
                candidate.size_bytes,
                self.cancel,
            ) {
                Ok(v) => v,
                Err(MaintainerError::Cancelled) => {
                    result.was_cancelled = true;
                    return Ok(());
                }
                Err(e) => {
                    result.record_issue(DuplicateIssue {
                        path: candidate.path.clone(),
                        message: format!("full hash failed: {e}"),
                    });
                    continue;
                }
            };
            result.bytes_hashed += bytes_read;

            let post_stat = match FileSnapshot::capture(&candidate.path) {
                Ok(s) => s,
                Err(e) => {
                    result.record_issue(DuplicateIssue {
                        path: candidate.path.clone(),
                        message: format!("post-hash stat failed: {e}"),
                    });
                    continue;
                }
            };

            if pre_stat != post_stat {
                result.record_issue(DuplicateIssue {
                    path: candidate.path.clone(),
                    message: "file changed during hashing".into(),
                });
                continue;
            }

            let modified = candidate.modified;
            confirmed.entry(digest).or_default().push(DuplicateFile {
                path: candidate.path.clone(),
                identity: candidate.identity,
                modified,
            });
        }

        for (digest, files) in confirmed {
            if files.len() >= 2 && distinct_inode_count_files(&files) >= 2 {
                result.groups.push(DuplicateGroup {
                    digest,
                    size_bytes: size,
                    files,
                });
            }
        }

        Ok(())
    }
}

/// Count distinct inodes in a slice of candidates.
fn distinct_inode_count(files: &[DuplicateCandidate]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for f in files {
        seen.insert((f.identity.device, f.identity.inode));
    }
    seen.len()
}

/// Count distinct inodes in candidates (used after partial hash grouping).
fn distinct_inode_count_candidates(files: &[DuplicateCandidate], indexes: &[usize]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for index in indexes {
        let file = &files[*index];
        seen.insert((file.identity.device, file.identity.inode));
    }
    seen.len()
}

/// Count distinct inodes in confirmed duplicate files.
fn distinct_inode_count_files(files: &[DuplicateFile]) -> usize {
    let mut seen = std::collections::HashSet::new();
    for f in files {
        seen.insert((f.identity.device, f.identity.inode));
    }
    seen.len()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::{CancellationToken, NullProgress};
    use std::os::unix::fs::MetadataExt;
    use tempfile::TempDir;

    #[test]
    fn duplicate_issue_samples_are_bounded_with_omitted_count() {
        let cancel = CancellationToken::new();
        let mut result = DuplicateScanner::new(&cancel, &NullProgress)
            .scan(Vec::new())
            .unwrap();
        for _ in 0..1_003 {
            result.record_issue(DuplicateIssue {
                path: PathBuf::new(),
                message: "unreadable file".into(),
            });
        }
        assert_eq!(result.issues.len(), 1_000);
        assert_eq!(result.omitted_issues, 3);
    }

    fn make_candidate(path: &Path) -> DuplicateCandidate {
        let meta = path.symlink_metadata().unwrap();
        DuplicateCandidate {
            path: path.to_path_buf(),
            size_bytes: meta.size(),
            identity: DeviceInode::from_metadata(&meta),
            modified: meta.modified().unwrap(),
        }
    }

    fn no_cancel() -> CancellationToken {
        CancellationToken::new()
    }

    #[test]
    fn different_sizes_never_hash() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.bin");
        let p2 = tmp.path().join("b.bin");
        std::fs::write(&p1, b"short").unwrap();
        std::fs::write(&p2, b"much longer content").unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), make_candidate(&p2)];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.groups.is_empty());
        assert_eq!(result.bytes_hashed, 0);
    }

    #[test]
    fn same_size_different_prefix_stops_before_full_hash() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.bin");
        let p2 = tmp.path().join("b.bin");
        std::fs::write(&p1, b"aaaa1234").unwrap();
        std::fs::write(&p2, b"bbbb1234").unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), make_candidate(&p2)];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.groups.is_empty());
        assert_eq!(result.bytes_hashed, 0);
    }

    #[test]
    fn same_partial_hash_different_full_content_no_group() {
        let tmp = TempDir::new().unwrap();
        let size = 16384usize; // larger than 2 * PARTIAL_CHUNK_BYTES (8192)

        let mut content1 = vec![0xAAu8; size];
        let mut content2 = vec![0xAAu8; size];
        content1[5000] = 0x01;
        content2[5000] = 0x02;

        let p1 = tmp.path().join("a.bin");
        let p2 = tmp.path().join("b.bin");
        std::fs::write(&p1, &content1).unwrap();
        std::fs::write(&p2, &content2).unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), make_candidate(&p2)];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.groups.is_empty());
        assert!(result.bytes_hashed > 0);
    }

    #[test]
    fn identical_content_forms_group() {
        let tmp = TempDir::new().unwrap();
        let content = b"identical duplicate content here";
        let p1 = tmp.path().join("copy1.dat");
        let p2 = tmp.path().join("copy2.dat");
        let p3 = tmp.path().join("copy3.dat");
        std::fs::write(&p1, content).unwrap();
        std::fs::write(&p2, content).unwrap();
        std::fs::write(&p3, content).unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![
            make_candidate(&p1),
            make_candidate(&p2),
            make_candidate(&p3),
        ];
        let result = scanner.scan(candidates).unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].files.len(), 3);
        assert_eq!(result.groups[0].size_bytes, content.len() as u64);
    }

    #[test]
    fn same_inode_not_presented_as_reclaimable_duplicate() {
        let tmp = TempDir::new().unwrap();
        let original = tmp.path().join("original.dat");
        let hardlink = tmp.path().join("hardlink.dat");
        std::fs::write(&original, b"hardlink content").unwrap();
        std::fs::hard_link(&original, &hardlink).unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&original), make_candidate(&hardlink)];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.groups.is_empty());
    }

    #[test]
    fn hardlink_with_separate_duplicate_forms_correct_group() {
        let tmp = TempDir::new().unwrap();
        let content = b"content for hardlink test";
        let original = tmp.path().join("original.dat");
        let hardlink = tmp.path().join("hardlink.dat");
        let separate = tmp.path().join("separate.dat");
        std::fs::write(&original, content).unwrap();
        std::fs::hard_link(&original, &hardlink).unwrap();
        std::fs::write(&separate, content).unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![
            make_candidate(&original),
            make_candidate(&hardlink),
            make_candidate(&separate),
        ];
        let result = scanner.scan(candidates).unwrap();

        assert_eq!(result.groups.len(), 1);
        assert_eq!(result.groups[0].files.len(), 3);
    }

    #[test]
    fn cancellation_stops_scan_promptly() {
        let tmp = TempDir::new().unwrap();
        let content = b"cancel test content";
        let p1 = tmp.path().join("a.dat");
        let p2 = tmp.path().join("b.dat");
        std::fs::write(&p1, content).unwrap();
        std::fs::write(&p2, content).unwrap();

        let cancel = CancellationToken::new();
        cancel.cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), make_candidate(&p2)];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.was_cancelled);
        assert!(result.groups.is_empty());
    }

    #[test]
    fn file_changed_while_hashing_is_rejected() {
        let tmp = TempDir::new().unwrap();
        let content = b"stable content here";
        let p1 = tmp.path().join("stable.dat");
        let p2 = tmp.path().join("changed.dat");
        std::fs::write(&p1, content).unwrap();
        std::fs::write(&p2, content).unwrap();

        let meta2 = p2.symlink_metadata().unwrap();
        let fake_candidate = DuplicateCandidate {
            path: p2.clone(),
            size_bytes: meta2.size(), // Same size so it groups.
            identity: DeviceInode {
                device: meta2.dev(),
                inode: meta2.ino() + 99999, // Wrong inode simulates replacement.
            },
            modified: meta2.modified().unwrap(),
        };

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), fake_candidate];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.groups.is_empty());
        assert!(result.issues.iter().any(|i| i.path == p2));
    }

    #[test]
    fn zero_byte_files_ignored() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("empty1");
        let p2 = tmp.path().join("empty2");
        std::fs::write(&p1, b"").unwrap();
        std::fs::write(&p2, b"").unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), make_candidate(&p2)];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.groups.is_empty());
    }

    #[test]
    fn multiple_size_groups_processed_independently() {
        let tmp = TempDir::new().unwrap();
        let a1 = tmp.path().join("a1.bin");
        let a2 = tmp.path().join("a2.bin");
        std::fs::write(&a1, b"0123456789").unwrap();
        std::fs::write(&a2, b"0123456789").unwrap();

        let b1 = tmp.path().join("b1.bin");
        let b2 = tmp.path().join("b2.bin");
        std::fs::write(&b1, b"abcde").unwrap();
        std::fs::write(&b2, b"abcde").unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![
            make_candidate(&a1),
            make_candidate(&a2),
            make_candidate(&b1),
            make_candidate(&b2),
        ];
        let result = scanner.scan(candidates).unwrap();

        assert_eq!(result.groups.len(), 2);
    }

    #[test]
    fn progress_reports_emitted() {
        use std::sync::{Arc, Mutex};

        struct RecordingProgress {
            messages: Arc<Mutex<Vec<String>>>,
        }

        impl ProgressSink for RecordingProgress {
            fn report(&self, progress: &JobProgress) {
                self.messages.lock().unwrap().push(progress.message.clone());
            }
        }

        let tmp = TempDir::new().unwrap();
        let content = b"progress test";
        let p1 = tmp.path().join("a.dat");
        let p2 = tmp.path().join("b.dat");
        std::fs::write(&p1, content).unwrap();
        std::fs::write(&p2, content).unwrap();

        let cancel = no_cancel();
        let messages = Arc::new(Mutex::new(Vec::new()));
        let progress = RecordingProgress {
            messages: Arc::clone(&messages),
        };
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&p1), make_candidate(&p2)];
        let _result = scanner.scan(candidates).unwrap();

        let msgs = messages.lock().unwrap();
        assert!(!msgs.is_empty());
        assert!(msgs.iter().any(|m| m.contains("Grouping")));
        assert!(msgs.iter().any(|m| m.contains("complete")));
    }

    #[test]
    fn empty_candidates_produces_empty_result() {
        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let result = scanner.scan(Vec::new()).unwrap();
        assert!(result.groups.is_empty());
        assert!(result.issues.is_empty());
        assert!(!result.was_cancelled);
        assert_eq!(result.files_considered, 0);
    }

    #[test]
    fn nonexistent_file_recorded_as_issue() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("real.dat");
        std::fs::write(&existing, b"real content").unwrap();

        let vanished = tmp.path().join("vanished.dat");
        std::fs::write(&vanished, b"real content").unwrap();
        let candidate_vanished = make_candidate(&vanished);
        std::fs::remove_file(&vanished).unwrap();

        let cancel = no_cancel();
        let progress = NullProgress;
        let scanner = DuplicateScanner::new(&cancel, &progress);

        let candidates = vec![make_candidate(&existing), candidate_vanished];
        let result = scanner.scan(candidates).unwrap();

        assert!(result.issues.iter().any(|i| i.path == vanished));
        assert!(result.groups.is_empty());
    }
}
