use std::path::PathBuf;

use maintainer_core::storage::{DuplicateGroup, DuplicateIssue, LargeFile, ScanIssue, ScanPolicy};

/// Application page enumeration.
///
/// Each variant maps to one top-level view in the navigation sidebar.
/// There is intentionally no Activity page in v1.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Page {
    #[default]
    Overview,
    System,
    Storage,
    Cleanup,
    Updates,
    Diagnostics,
    Reports,
    Settings,
}

impl Page {
    /// All pages in navigation order.
    pub const ALL: &'static [Page] = &[
        Page::Overview,
        Page::System,
        Page::Storage,
        Page::Cleanup,
        Page::Updates,
        Page::Diagnostics,
        Page::Reports,
        Page::Settings,
    ];

    /// Human-readable label for the page.
    pub fn label(self) -> &'static str {
        match self {
            Page::Overview => "Overview",
            Page::System => "System",
            Page::Storage => "Storage",
            Page::Cleanup => "Cleanup",
            Page::Updates => "Updates",
            Page::Diagnostics => "Diagnostics",
            Page::Reports => "Reports",
            Page::Settings => "Settings",
        }
    }

    /// Icon name from the system icon theme.
    pub fn icon_name(self) -> &'static str {
        match self {
            Page::Overview => "go-home-symbolic",
            Page::System => "utilities-system-monitor-symbolic",
            Page::Storage => "drive-harddisk-symbolic",
            Page::Cleanup => "edit-clear-all-symbolic",
            Page::Updates => "software-update-available-symbolic",
            Page::Diagnostics => "dialog-warning-symbolic",
            Page::Reports => "document-open-symbolic",
            Page::Settings => "preferences-system-symbolic",
        }
    }
}

/// Typed user intents that drive state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppIntent {
    /// Navigate to a specific page.
    Navigate(Page),
    /// Window visibility changed (e.g., hidden/shown).
    WindowVisibilityChanged(bool),
    /// Retry a capability by its identifier.
    RetryCapability(&'static str),
}

/// Presentation state of the application shell.
///
/// Pure data — no GTK types. Testable without a display server.
#[derive(Debug, Clone)]
pub struct AppState {
    /// Currently active page.
    pub active_page: Page,
    /// Whether the window is currently visible.
    pub window_visible: bool,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            active_page: Page::Overview,
            window_visible: true,
        }
    }
}

impl AppState {
    /// Apply an intent and update state.
    pub fn reduce(&mut self, intent: AppIntent) {
        match intent {
            AppIntent::Navigate(page) => {
                self.active_page = page;
            }
            AppIntent::WindowVisibilityChanged(visible) => {
                self.window_visible = visible;
            }
            AppIntent::RetryCapability(_id) => {
                // Retry intents are handled by the service layer;
                // state remains unchanged here.
            }
        }
    }
}

/// Navigation group definition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NavigationGroup {
    /// Group label.
    pub label: &'static str,
    /// Pages in this group.
    pub pages: &'static [Page],
}

/// Navigation groups in sidebar order.
pub const NAVIGATION_GROUPS: &[NavigationGroup] = &[
    NavigationGroup {
        label: "General",
        pages: &[Page::Overview],
    },
    NavigationGroup {
        label: "Monitor",
        pages: &[Page::System],
    },
    NavigationGroup {
        label: "Storage",
        pages: &[Page::Storage],
    },
    NavigationGroup {
        label: "Maintenance",
        pages: &[Page::Cleanup, Page::Updates],
    },
    NavigationGroup {
        label: "Diagnostics",
        pages: &[Page::Diagnostics, Page::Reports],
    },
];

/// Footer pages (displayed separately at bottom of sidebar).
pub const FOOTER_PAGES: &[Page] = &[Page::Settings];

// ---------------------------------------------------------------------------
// Storage state
// ---------------------------------------------------------------------------

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
                // If a scan is already running, deny.
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
                // Clear previous duplicate results.
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
                    // Selections default empty.
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
                // Go up one level or clear.
                let current = self.drill_down_path.take().unwrap();
                self.drill_down_path = current.parent().map(|p| p.to_path_buf());
                true
            }
            StorageIntent::ToggleDuplicateSelection {
                group_idx,
                file_idx,
            } => {
                // Validate indices.
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
        // Files under /home are generally Trash-eligible via freedesktop Trash spec.
        path.starts_with("/home")
    }
}

// ---------------------------------------------------------------------------
// Cleanup state
// ---------------------------------------------------------------------------

use crate::ui::cleanup::CleanupPhase;

/// State of the cleanup workflow page.
#[derive(Debug, Clone, Default)]
pub struct CleanupState {
    /// Current phase of the cleanup workflow.
    pub phase: CleanupPhase,
    /// Number of selected items (for enabling/disabling execute button).
    pub selected_count: usize,
    /// Total reclaimable bytes from selected items.
    pub selected_bytes: u64,
    /// Whether there are items requiring permanent deletion in the selection.
    pub has_permanent_items: bool,
}

/// Intents that drive cleanup state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CleanupIntent {
    /// Start scanning for cleanup targets.
    StartScan,
    /// Update scan progress.
    ScanProgress { message: String },
    /// Scan completed with items found.
    ScanCompleted { item_count: usize, total_bytes: u64 },
    /// Scan failed.
    ScanFailed { message: String },
    /// Update selected item count.
    SelectionChanged {
        selected_count: usize,
        selected_bytes: u64,
        has_permanent: bool,
    },
    /// Start executing cleanup.
    StartExecution,
    /// Update execution progress.
    ExecutionProgress { message: String },
    /// Execution completed.
    ExecutionCompleted { summary: String },
    /// Reset to idle state.
    Reset,
}

impl CleanupState {
    /// Apply a cleanup intent and update state.
    pub fn reduce(&mut self, intent: CleanupIntent) -> bool {
        match intent {
            CleanupIntent::StartScan => {
                self.phase = CleanupPhase::Scanning {
                    message: "Starting scan…".into(),
                };
                self.selected_count = 0;
                self.selected_bytes = 0;
                self.has_permanent_items = false;
                true
            }
            CleanupIntent::ScanProgress { message } => {
                self.phase = CleanupPhase::Scanning { message };
                true
            }
            CleanupIntent::ScanCompleted {
                item_count,
                total_bytes,
            } => {
                self.phase = CleanupPhase::Reviewing {
                    item_count,
                    total_bytes,
                };
                true
            }
            CleanupIntent::ScanFailed { message } => {
                self.phase = CleanupPhase::Completed {
                    summary: format!("Scan failed: {message}"),
                };
                true
            }
            CleanupIntent::SelectionChanged {
                selected_count,
                selected_bytes,
                has_permanent,
            } => {
                self.selected_count = selected_count;
                self.selected_bytes = selected_bytes;
                self.has_permanent_items = has_permanent;
                true
            }
            CleanupIntent::StartExecution => {
                self.phase = CleanupPhase::Executing {
                    message: "Starting cleanup…".into(),
                };
                true
            }
            CleanupIntent::ExecutionProgress { message } => {
                self.phase = CleanupPhase::Executing { message };
                true
            }
            CleanupIntent::ExecutionCompleted { summary } => {
                self.phase = CleanupPhase::Completed { summary };
                self.selected_count = 0;
                self.selected_bytes = 0;
                true
            }
            CleanupIntent::Reset => {
                *self = Self::default();
                true
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Settings state
// ---------------------------------------------------------------------------

/// Theme selection for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Follow the system preference.
    #[default]
    System,
    /// Force light theme.
    Light,
    /// Force dark theme.
    Dark,
}

impl Theme {
    /// Convert to the string ID used in UI controls.
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// Parse from a string ID.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

/// Immutable settings snapshot published to consumers.
///
/// Monitoring applies refresh changes at the next scheduling boundary;
/// active scans retain their original policy snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSnapshot {
    /// Theme preference.
    pub theme: Theme,
    /// Visible-window refresh interval in seconds (1–60).
    pub refresh_interval_secs: u32,
    /// Hidden-window minimum interval in seconds (10–60).
    pub hidden_interval_secs: u32,
    /// Large file threshold in bytes (>= 1 MiB).
    pub large_file_threshold_bytes: u64,
    /// Directories excluded from scans.
    pub scan_exclusions: Vec<String>,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            refresh_interval_secs: 2,
            hidden_interval_secs: 10,
            large_file_threshold_bytes: 100 * 1024 * 1024,
            scan_exclusions: Vec::new(),
        }
    }
}

/// Intents that drive settings state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIntent {
    /// Change the theme.
    SetTheme(Theme),
    /// Change the visible-window refresh interval.
    SetRefreshInterval(u32),
    /// Change the hidden-window refresh interval.
    SetHiddenInterval(u32),
    /// Change the large file threshold (in bytes).
    SetLargeFileThreshold(u64),
    /// Add a scan exclusion path.
    AddExclusion(String),
    /// Remove a scan exclusion by index.
    RemoveExclusion(usize),
    /// Request deletion of all history (triggers confirmation flow).
    RequestDeleteAllHistory,
    /// Request deletion of all application data (triggers confirmation flow).
    RequestDeleteAllData,
}

/// State managing the current settings and pending confirmations.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    /// Current settings snapshot.
    pub snapshot: SettingsSnapshot,
    /// Whether a "delete all history" confirmation is pending.
    pub confirm_delete_history: bool,
    /// Whether a "delete all data" confirmation is pending.
    pub confirm_delete_all_data: bool,
    /// Whether settings have been modified since last save.
    pub dirty: bool,
}

impl SettingsState {
    /// Create from a loaded snapshot.
    pub fn from_snapshot(snapshot: SettingsSnapshot) -> Self {
        Self {
            snapshot,
            confirm_delete_history: false,
            confirm_delete_all_data: false,
            dirty: false,
        }
    }

    /// Apply a settings intent. Returns `true` if state changed.
    pub fn reduce(&mut self, intent: SettingsIntent) -> bool {
        match intent {
            SettingsIntent::SetTheme(theme) => {
                if self.snapshot.theme == theme {
                    return false;
                }
                self.snapshot.theme = theme;
                self.dirty = true;
                true
            }
            SettingsIntent::SetRefreshInterval(secs) => {
                let clamped = secs.clamp(1, 60);
                if self.snapshot.refresh_interval_secs == clamped {
                    return false;
                }
                self.snapshot.refresh_interval_secs = clamped;
                self.dirty = true;
                true
            }
            SettingsIntent::SetHiddenInterval(secs) => {
                let clamped = secs.clamp(10, 60);
                if self.snapshot.hidden_interval_secs == clamped {
                    return false;
                }
                self.snapshot.hidden_interval_secs = clamped;
                self.dirty = true;
                true
            }
            SettingsIntent::SetLargeFileThreshold(bytes) => {
                // Minimum 1 MiB.
                let min_bytes = 1024 * 1024;
                let validated = if bytes < min_bytes { min_bytes } else { bytes };
                if self.snapshot.large_file_threshold_bytes == validated {
                    return false;
                }
                self.snapshot.large_file_threshold_bytes = validated;
                self.dirty = true;
                true
            }
            SettingsIntent::AddExclusion(path) => {
                let canonical = path.trim().to_string();
                if canonical.is_empty() {
                    return false;
                }
                if self.snapshot.scan_exclusions.contains(&canonical) {
                    return false;
                }
                self.snapshot.scan_exclusions.push(canonical);
                self.dirty = true;
                true
            }
            SettingsIntent::RemoveExclusion(idx) => {
                if idx >= self.snapshot.scan_exclusions.len() {
                    return false;
                }
                self.snapshot.scan_exclusions.remove(idx);
                self.dirty = true;
                true
            }
            SettingsIntent::RequestDeleteAllHistory => {
                self.confirm_delete_history = true;
                true
            }
            SettingsIntent::RequestDeleteAllData => {
                self.confirm_delete_all_data = true;
                true
            }
        }
    }
}

// ---------------------------------------------------------------------------
// History view state
// ---------------------------------------------------------------------------

/// Intents for the history view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HistoryIntent {
    /// Request deletion of a single entry by ID (triggers confirmation).
    RequestDeleteOne(String),
    /// Request deletion of all entries (triggers confirmation).
    RequestDeleteAll,
    /// Confirm the pending deletion.
    ConfirmDelete,
    /// Cancel the pending deletion.
    CancelDelete,
}

/// Pending deletion target for the history view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingHistoryDeletion {
    /// Deleting a single entry by ID.
    One(String),
    /// Deleting all entries.
    All,
}

/// State for the history view's confirmation flow.
#[derive(Debug, Clone, Default)]
pub struct HistoryState {
    /// Pending deletion confirmation, if any.
    pub pending_deletion: Option<PendingHistoryDeletion>,
}

impl HistoryState {
    /// Apply a history intent. Returns `true` if state changed.
    pub fn reduce(&mut self, intent: HistoryIntent) -> bool {
        match intent {
            HistoryIntent::RequestDeleteOne(id) => {
                self.pending_deletion = Some(PendingHistoryDeletion::One(id));
                true
            }
            HistoryIntent::RequestDeleteAll => {
                self.pending_deletion = Some(PendingHistoryDeletion::All);
                true
            }
            HistoryIntent::ConfirmDelete => {
                if self.pending_deletion.is_some() {
                    // Confirmed; the service layer handles the actual deletion.
                    // pending_deletion is consumed by the caller.
                    true
                } else {
                    false
                }
            }
            HistoryIntent::CancelDelete => {
                if self.pending_deletion.is_some() {
                    self.pending_deletion = None;
                    true
                } else {
                    false
                }
            }
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn default_snapshot_values() {
        let s = SettingsSnapshot::default();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.refresh_interval_secs, 2);
        assert_eq!(s.hidden_interval_secs, 10);
        assert_eq!(s.large_file_threshold_bytes, 100 * 1024 * 1024);
        assert!(s.scan_exclusions.is_empty());
    }

    #[test]
    fn set_theme_marks_dirty() {
        let mut state = SettingsState::default();
        assert!(!state.dirty);
        state.reduce(SettingsIntent::SetTheme(Theme::Dark));
        assert!(state.dirty);
        assert_eq!(state.snapshot.theme, Theme::Dark);
    }

    #[test]
    fn set_same_theme_returns_false() {
        let mut state = SettingsState::default();
        let changed = state.reduce(SettingsIntent::SetTheme(Theme::System));
        assert!(!changed);
        assert!(!state.dirty);
    }

    #[test]
    fn refresh_interval_clamped_to_range() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::SetRefreshInterval(0));
        assert_eq!(state.snapshot.refresh_interval_secs, 1);

        state.reduce(SettingsIntent::SetRefreshInterval(100));
        assert_eq!(state.snapshot.refresh_interval_secs, 60);

        state.reduce(SettingsIntent::SetRefreshInterval(30));
        assert_eq!(state.snapshot.refresh_interval_secs, 30);
    }

    #[test]
    fn hidden_interval_minimum_is_10() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::SetHiddenInterval(5));
        assert_eq!(state.snapshot.hidden_interval_secs, 10);

        state.reduce(SettingsIntent::SetHiddenInterval(45));
        assert_eq!(state.snapshot.hidden_interval_secs, 45);
    }

    #[test]
    fn large_file_threshold_minimum_1mib() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::SetLargeFileThreshold(100));
        assert_eq!(state.snapshot.large_file_threshold_bytes, 1024 * 1024);

        state.reduce(SettingsIntent::SetLargeFileThreshold(50 * 1024 * 1024));
        assert_eq!(state.snapshot.large_file_threshold_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn add_and_remove_exclusion() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::AddExclusion("/tmp".into()));
        assert_eq!(state.snapshot.scan_exclusions, vec!["/tmp"]);

        // Duplicate is rejected.
        let changed = state.reduce(SettingsIntent::AddExclusion("/tmp".into()));
        assert!(!changed);

        state.reduce(SettingsIntent::RemoveExclusion(0));
        assert!(state.snapshot.scan_exclusions.is_empty());
    }

    #[test]
    fn empty_exclusion_rejected() {
        let mut state = SettingsState::default();
        let changed = state.reduce(SettingsIntent::AddExclusion("  ".into()));
        assert!(!changed);
    }

    #[test]
    fn remove_exclusion_out_of_bounds_rejected() {
        let mut state = SettingsState::default();
        let changed = state.reduce(SettingsIntent::RemoveExclusion(0));
        assert!(!changed);
    }

    #[test]
    fn request_delete_history_sets_flag() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::RequestDeleteAllHistory);
        assert!(state.confirm_delete_history);
    }

    #[test]
    fn request_delete_all_data_sets_flag() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::RequestDeleteAllData);
        assert!(state.confirm_delete_all_data);
    }

    #[test]
    fn theme_round_trip() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            let s = theme.as_str();
            assert_eq!(Theme::parse(s), Some(theme));
        }
        assert_eq!(Theme::parse("invalid"), None);
    }
}

#[cfg(test)]
mod history_state_tests {
    use super::*;

    #[test]
    fn request_delete_one_sets_pending() {
        let mut state = HistoryState::default();
        state.reduce(HistoryIntent::RequestDeleteOne("abc".into()));
        assert_eq!(
            state.pending_deletion,
            Some(PendingHistoryDeletion::One("abc".into()))
        );
    }

    #[test]
    fn request_delete_all_sets_pending() {
        let mut state = HistoryState::default();
        state.reduce(HistoryIntent::RequestDeleteAll);
        assert_eq!(state.pending_deletion, Some(PendingHistoryDeletion::All));
    }

    #[test]
    fn cancel_clears_pending() {
        let mut state = HistoryState::default();
        state.reduce(HistoryIntent::RequestDeleteAll);
        state.reduce(HistoryIntent::CancelDelete);
        assert_eq!(state.pending_deletion, None);
    }

    #[test]
    fn confirm_without_pending_is_noop() {
        let mut state = HistoryState::default();
        let changed = state.reduce(HistoryIntent::ConfirmDelete);
        assert!(!changed);
    }

    #[test]
    fn cancel_without_pending_is_noop() {
        let mut state = HistoryState::default();
        let changed = state.reduce(HistoryIntent::CancelDelete);
        assert!(!changed);
    }
}

#[cfg(test)]
mod cleanup_tests {
    use super::*;

    #[test]
    fn cleanup_state_default_is_idle() {
        let state = CleanupState::default();
        assert_eq!(state.phase, CleanupPhase::Idle);
        assert_eq!(state.selected_count, 0);
    }

    #[test]
    fn cleanup_start_scan_transitions_to_scanning() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        assert!(matches!(state.phase, CleanupPhase::Scanning { .. }));
    }

    #[test]
    fn cleanup_scan_completed_transitions_to_reviewing() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        state.reduce(CleanupIntent::ScanCompleted {
            item_count: 5,
            total_bytes: 1024,
        });
        assert_eq!(
            state.phase,
            CleanupPhase::Reviewing {
                item_count: 5,
                total_bytes: 1024,
            }
        );
    }

    #[test]
    fn cleanup_selection_updates() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::SelectionChanged {
            selected_count: 3,
            selected_bytes: 512,
            has_permanent: true,
        });
        assert_eq!(state.selected_count, 3);
        assert_eq!(state.selected_bytes, 512);
        assert!(state.has_permanent_items);
    }

    #[test]
    fn cleanup_execution_flow() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartExecution);
        assert!(matches!(state.phase, CleanupPhase::Executing { .. }));

        state.reduce(CleanupIntent::ExecutionCompleted {
            summary: "Recovered 1 MiB".into(),
        });
        assert!(matches!(state.phase, CleanupPhase::Completed { .. }));
        assert_eq!(state.selected_count, 0);
    }

    #[test]
    fn cleanup_reset_returns_to_default() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        state.reduce(CleanupIntent::ScanCompleted {
            item_count: 10,
            total_bytes: 2048,
        });
        state.reduce(CleanupIntent::Reset);
        assert_eq!(state.phase, CleanupPhase::Idle);
    }

    #[test]
    fn cleanup_scan_failed_shows_error() {
        let mut state = CleanupState::default();
        state.reduce(CleanupIntent::StartScan);
        state.reduce(CleanupIntent::ScanFailed {
            message: "permission denied".into(),
        });
        match &state.phase {
            CleanupPhase::Completed { summary } => {
                assert!(summary.contains("permission denied"));
            }
            other => panic!("expected Completed, got {other:?}"),
        }
    }
}
