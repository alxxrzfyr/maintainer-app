use std::path::PathBuf;

use maintainer_core::storage::{DuplicateGroup, DuplicateIssue, LargeFile, ScanIssue, ScanPolicy};

/// Unique identifier for a scan job. Prevents stale messages from overwriting
/// current state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScanJobId(pub u64);

/// Sort order for large-file results.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LargeFileSort {
    /// Largest files first.
    #[default]
    SizeDesc,
    /// Smallest qualifying files first.
    SizeAsc,
    /// Alphabetical by path.
    PathAsc,
}

/// Sub-tab within the Storage page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StorageSubPage {
    #[default]
    Overview,
    LargeFiles,
    Duplicates,
}

/// Phase of an active scan.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScanPhase {
    /// Scan is running. Progress values: (completed, total_if_known).
    Running { completed: u64, total: Option<u64> },
    /// Scan completed.
    Completed,
    /// Scan was cancelled.
    Cancelled,
}

/// Active scan metadata.
#[derive(Debug, Clone)]
pub struct ActiveScan {
    /// Immutable job identifier.
    pub job_id: ScanJobId,
    /// Policy snapshot used for this scan.
    pub policy: ScanPolicy,
    /// Current phase.
    pub phase: ScanPhase,
}

/// A single item in the removal plan review.
#[derive(Debug, Clone)]
pub struct RemovalTarget {
    /// Full canonical path.
    pub path: PathBuf,
    /// Size in bytes.
    pub size_bytes: u64,
    /// Whether the file can be sent to Trash.
    pub trash_eligible: bool,
}

/// Request to plan removal of selected files. Handed off to cleanup (Task 10).
#[derive(Debug, Clone)]
pub struct RemovalPlanRequest {
    /// Target files for removal.
    pub targets: Vec<RemovalTarget>,
    /// Total bytes across all targets.
    pub total_bytes: u64,
}

/// State of the storage analysis pages.
#[derive(Debug, Clone)]
pub struct StorageState {
    /// Current sub-page.
    pub sub_page: StorageSubPage,
    /// Selected scan root directory. Must be non-empty before a scan starts.
    pub scan_root: Option<PathBuf>,
    /// Currently active or last-completed storage scan.
    pub storage_scan: Option<ActiveScan>,
    /// Currently active or last-completed duplicate scan.
    pub duplicate_scan: Option<ActiveScan>,
    /// Large file results from the last completed storage scan.
    pub large_files: Vec<LargeFile>,
    /// Sort order for large files.
    pub large_file_sort: LargeFileSort,
    /// Recoverable issues from the last storage scan.
    pub scan_issues: Vec<ScanIssue>,
    /// Duplicate groups from the last completed duplicate scan.
    pub duplicate_groups: Vec<DuplicateGroup>,
    /// Recoverable issues from the last duplicate scan.
    pub duplicate_issues: Vec<DuplicateIssue>,
    /// Indices of selected duplicate files for removal (group_idx, file_idx).
    /// Defaults empty — user must explicitly select each file.
    pub duplicate_selections: Vec<(usize, usize)>,
    /// Current directory drill-down path within results.
    pub drill_down_path: Option<PathBuf>,
    /// Pending removal plan for review before handoff.
    pub removal_review: Option<RemovalPlanRequest>,
    /// Next job ID counter.
    next_job_id: u64,
}

impl Default for StorageState {
    fn default() -> Self {
        Self {
            sub_page: StorageSubPage::Overview,
            scan_root: None,
            storage_scan: None,
            duplicate_scan: None,
            large_files: Vec::new(),
            large_file_sort: LargeFileSort::default(),
            scan_issues: Vec::new(),
            duplicate_groups: Vec::new(),
            duplicate_issues: Vec::new(),
            duplicate_selections: Vec::new(),
            drill_down_path: None,
            removal_review: None,
            next_job_id: 1,
        }
    }
}

/// Intents that drive storage state transitions.
#[derive(Debug, Clone)]
pub enum StorageIntent {
    /// Navigate to a storage sub-page.
    NavigateSubPage(StorageSubPage),
    /// Set the scan root directory.
    SetScanRoot(PathBuf),
    /// Start a storage scan. Denied if no root is set.
    StartStorageScan,
    /// Start a duplicate scan. Denied if no root is set.
    StartDuplicateScan,
    /// Report progress for a storage scan job.
    StorageScanProgress {
        job_id: ScanJobId,
        completed: u64,
        total: Option<u64>,
    },
    /// Report progress for a duplicate scan job.
    DuplicateScanProgress {
        job_id: ScanJobId,
        completed: u64,
        total: Option<u64>,
    },
    /// Storage scan completed with results.
    StorageScanCompleted {
        job_id: ScanJobId,
        large_files: Vec<LargeFile>,
        issues: Vec<ScanIssue>,
    },
    /// Duplicate scan completed with results.
    DuplicateScanCompleted {
        job_id: ScanJobId,
        groups: Vec<DuplicateGroup>,
        issues: Vec<DuplicateIssue>,
    },
    /// Cancel the active storage scan.
    CancelStorageScan { job_id: ScanJobId },
    /// Cancel the active duplicate scan.
    CancelDuplicateScan { job_id: ScanJobId },
    /// Change sort order for large files.
    SetLargeFileSort(LargeFileSort),
    /// Drill down into a directory within results.
    DrillDown(PathBuf),
    /// Navigate back from drill-down.
    DrillUp,
    /// Toggle duplicate file selection.
    ToggleDuplicateSelection { group_idx: usize, file_idx: usize },
    /// Clear all duplicate selections.
    ClearDuplicateSelections,
    /// Build removal plan from current selections for review.
    BuildRemovalPlan,
    /// Dismiss the removal review.
    DismissRemovalReview,
}

impl StorageState {
    /// Allocate the next unique job ID.
    fn next_id(&mut self) -> ScanJobId {
        let id = ScanJobId(self.next_job_id);
        self.next_job_id += 1;
        id
    }

    /// Apply a storage intent and update state. Returns `true` if the intent
    /// was accepted (not denied or stale).
    pub fn reduce(&mut self, intent: StorageIntent) -> bool {
        match intent {
            StorageIntent::NavigateSubPage(page) => {
                self.sub_page = page;
                true
            }
            StorageIntent::SetScanRoot(root) => {
                self.scan_root = Some(root);
                true
            }
            StorageIntent::StartStorageScan => {
                let root = match &self.scan_root {
                    Some(r) => r.clone(),
                    None => return false, // Denied: no root selected.
                };
                if let Some(ref scan) = self.storage_scan {
                    if matches!(scan.phase, ScanPhase::Running { .. }) {
                        return false;
                    }
                }
                let job_id = self.next_id();
                let policy = ScanPolicy::with_roots(vec![root]);
                self.storage_scan = Some(ActiveScan {
                    job_id,
                    policy,
                    phase: ScanPhase::Running {
                        completed: 0,
                        total: None,
                    },
                });
                true
            }
            StorageIntent::StartDuplicateScan => {
                let root = match &self.scan_root {
                    Some(r) => r.clone(),
                    None => return false,
                };
                if let Some(ref scan) = self.duplicate_scan {
                    if matches!(scan.phase, ScanPhase::Running { .. }) {
                        return false;
                    }
                }
                let job_id = self.next_id();
                let policy = ScanPolicy::with_roots(vec![root]);
                self.duplicate_scan = Some(ActiveScan {
                    job_id,
                    policy,
                    phase: ScanPhase::Running {
                        completed: 0,
                        total: None,
                    },
                });
                self.duplicate_groups.clear();
                self.duplicate_issues.clear();
                self.duplicate_selections.clear();
                true
            }
            StorageIntent::StorageScanProgress {
                job_id,
                completed,
                total,
            } => {
                if let Some(ref mut scan) = self.storage_scan {
                    if scan.job_id != job_id {
                        return false; // Stale.
                    }
                    if !matches!(scan.phase, ScanPhase::Running { .. }) {
                        return false;
                    }
                    scan.phase = ScanPhase::Running { completed, total };
                    true
                } else {
                    false
                }
            }
            StorageIntent::DuplicateScanProgress {
                job_id,
                completed,
                total,
            } => {
                if let Some(ref mut scan) = self.duplicate_scan {
                    if scan.job_id != job_id {
                        return false;
                    }
                    if !matches!(scan.phase, ScanPhase::Running { .. }) {
                        return false;
                    }
                    scan.phase = ScanPhase::Running { completed, total };
                    true
                } else {
                    false
                }
            }
            StorageIntent::StorageScanCompleted {
                job_id,
                large_files,
                issues,
            } => {
                if let Some(ref mut scan) = self.storage_scan {
                    if scan.job_id != job_id {
                        return false;
                    }
                    scan.phase = ScanPhase::Completed;
                    self.large_files = large_files;
                    self.scan_issues = issues;
                    self.apply_large_file_sort();
                    true
                } else {
                    false
                }
            }
            StorageIntent::DuplicateScanCompleted {
                job_id,
                groups,
                issues,
            } => {
                if let Some(ref mut scan) = self.duplicate_scan {
                    if scan.job_id != job_id {
                        return false;
                    }
                    scan.phase = ScanPhase::Completed;
                    self.duplicate_groups = groups;
                    self.duplicate_issues = issues;
                    self.duplicate_selections.clear();
                    true
                } else {
                    false
                }
            }
            StorageIntent::CancelStorageScan { job_id } => {
                if let Some(ref mut scan) = self.storage_scan {
                    if scan.job_id != job_id {
                        return false;
                    }
                    if !matches!(scan.phase, ScanPhase::Running { .. }) {
                        return false;
                    }
                    scan.phase = ScanPhase::Cancelled;
                    true
                } else {
                    false
                }
            }
            StorageIntent::CancelDuplicateScan { job_id } => {
                if let Some(ref mut scan) = self.duplicate_scan {
                    if scan.job_id != job_id {
                        return false;
                    }
                    if !matches!(scan.phase, ScanPhase::Running { .. }) {
                        return false;
                    }
                    scan.phase = ScanPhase::Cancelled;
                    true
                } else {
                    false
                }
            }
            StorageIntent::SetLargeFileSort(sort) => {
                if self.large_file_sort == sort {
                    return false;
                }
                self.large_file_sort = sort;
                self.apply_large_file_sort();
                true
            }
            StorageIntent::DrillDown(path) => {
                self.drill_down_path = Some(path);
                true
            }
            StorageIntent::DrillUp => {
                if self.drill_down_path.is_none() {
                    return false;
                }
                let current = self.drill_down_path.take().unwrap();
                self.drill_down_path = current.parent().map(|p| p.to_path_buf());
                true
            }
            StorageIntent::ToggleDuplicateSelection {
                group_idx,
                file_idx,
            } => {
                if group_idx >= self.duplicate_groups.len() {
                    return false;
                }
                if file_idx >= self.duplicate_groups[group_idx].files.len() {
                    return false;
                }
                let entry = (group_idx, file_idx);
                if let Some(pos) = self.duplicate_selections.iter().position(|e| *e == entry) {
                    self.duplicate_selections.remove(pos);
                } else {
                    self.duplicate_selections.push(entry);
                }
                true
            }
            StorageIntent::ClearDuplicateSelections => {
                if self.duplicate_selections.is_empty() {
                    return false;
                }
                self.duplicate_selections.clear();
                true
            }
            StorageIntent::BuildRemovalPlan => {
                if self.duplicate_selections.is_empty() {
                    return false; // Cannot build empty plan.
                }
                let mut targets = Vec::new();
                let mut total_bytes: u64 = 0;
                for &(group_idx, file_idx) in &self.duplicate_selections {
                    if group_idx < self.duplicate_groups.len() {
                        let group = &self.duplicate_groups[group_idx];
                        if file_idx < group.files.len() {
                            let file = &group.files[file_idx];
                            let size = group.size_bytes;
                            total_bytes = total_bytes.saturating_add(size);
                            targets.push(RemovalTarget {
                                path: file.path.clone(),
                                size_bytes: size,
                                trash_eligible: Self::is_trash_eligible(&file.path),
                            });
                        }
                    }
                }
                if targets.is_empty() {
                    return false;
                }
                self.removal_review = Some(RemovalPlanRequest {
                    targets,
                    total_bytes,
                });
                true
            }
            StorageIntent::DismissRemovalReview => {
                if self.removal_review.is_none() {
                    return false;
                }
                self.removal_review = None;
                true
            }
        }
    }

    /// Sort large files according to the current sort order.
    fn apply_large_file_sort(&mut self) {
        match self.large_file_sort {
            LargeFileSort::SizeDesc => {
                self.large_files
                    .sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
            }
            LargeFileSort::SizeAsc => {
                self.large_files
                    .sort_by(|a, b| a.size_bytes.cmp(&b.size_bytes));
            }
            LargeFileSort::PathAsc => {
                self.large_files.sort_by(|a, b| a.path.cmp(&b.path));
            }
        }
    }

    /// Heuristic: files under common home directories are Trash-eligible.
    fn is_trash_eligible(path: &std::path::Path) -> bool {
        path.starts_with("/home")
    }
}
