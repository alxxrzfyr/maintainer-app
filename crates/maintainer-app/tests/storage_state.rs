//! Integration tests for storage state machine.
//!
//! Tests exercise the pure-data storage state reducer without requiring
//! a display server. They verify root selection, empty denial, scan lifecycle,
//! stale job rejection, sort stability, duplicate selections, and removal review.

use std::path::PathBuf;
use std::time::SystemTime;

use maintainer_app::state::{
    LargeFileSort, ScanJobId, ScanPhase, StorageIntent, StorageState, StorageSubPage,
};
use maintainer_core::storage::mounts::DeviceInode;
use maintainer_core::storage::{
    ContentDigest, DuplicateFile, DuplicateGroup, LargeFile, ScanIssue,
};

// ---------------------------------------------------------------------------
// Root selection
// ---------------------------------------------------------------------------

#[test]
fn set_scan_root_stores_path() {
    let mut state = StorageState::default();
    assert!(state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user"))));
    assert_eq!(state.scan_root, Some(PathBuf::from("/home/user")));
}

#[test]
fn scan_denied_without_root() {
    let mut state = StorageState::default();
    assert!(!state.reduce(StorageIntent::StartStorageScan));
    assert!(state.storage_scan.is_none());
}

#[test]
fn duplicate_scan_denied_without_root() {
    let mut state = StorageState::default();
    assert!(!state.reduce(StorageIntent::StartDuplicateScan));
    assert!(state.duplicate_scan.is_none());
}

// ---------------------------------------------------------------------------
// Start / Progress / Cancel / Result lifecycle
// ---------------------------------------------------------------------------

#[test]
fn start_storage_scan_creates_running_state() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    assert!(state.reduce(StorageIntent::StartStorageScan));

    let scan = state.storage_scan.as_ref().unwrap();
    assert!(matches!(
        scan.phase,
        ScanPhase::Running {
            completed: 0,
            total: None
        }
    ));
    assert_eq!(scan.job_id, ScanJobId(1));
}

#[test]
fn cannot_start_scan_while_running() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    // Second start denied.
    assert!(!state.reduce(StorageIntent::StartStorageScan));
}

#[test]
fn progress_updates_running_scan() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    assert!(state.reduce(StorageIntent::StorageScanProgress {
        job_id,
        completed: 50,
        total: Some(100),
    }));
    let scan = state.storage_scan.as_ref().unwrap();
    assert_eq!(
        scan.phase,
        ScanPhase::Running {
            completed: 50,
            total: Some(100)
        }
    );
}

#[test]
fn cancel_transitions_to_cancelled() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    assert!(state.reduce(StorageIntent::CancelStorageScan { job_id }));
    assert_eq!(
        state.storage_scan.as_ref().unwrap().phase,
        ScanPhase::Cancelled
    );
}

#[test]
fn completed_scan_stores_results() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    let files = vec![
        LargeFile {
            path: PathBuf::from("/tmp/big.bin"),
            size_bytes: 2_000_000_000,
        },
        LargeFile {
            path: PathBuf::from("/tmp/medium.bin"),
            size_bytes: 1_500_000_000,
        },
    ];
    let issues = vec![ScanIssue {
        path: PathBuf::from("/tmp/locked"),
        message: "permission denied".into(),
    }];

    assert!(state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files: files,
        issues,
    }));
    assert_eq!(state.large_files.len(), 2);
    assert_eq!(state.scan_issues.len(), 1);
    assert_eq!(
        state.storage_scan.as_ref().unwrap().phase,
        ScanPhase::Completed
    );
}

// ---------------------------------------------------------------------------
// Stale job rejection
// ---------------------------------------------------------------------------

#[test]
fn stale_progress_rejected() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let _job_id = state.storage_scan.as_ref().unwrap().job_id;

    // Stale ID from a hypothetical previous scan.
    let stale_id = ScanJobId(999);
    assert!(!state.reduce(StorageIntent::StorageScanProgress {
        job_id: stale_id,
        completed: 10,
        total: Some(100),
    }));
}

#[test]
fn stale_completion_rejected() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);

    let stale_id = ScanJobId(999);
    assert!(!state.reduce(StorageIntent::StorageScanCompleted {
        job_id: stale_id,
        large_files: Vec::new(),
        issues: Vec::new(),
    }));
}

#[test]
fn stale_cancel_rejected() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);

    let stale_id = ScanJobId(999);
    assert!(!state.reduce(StorageIntent::CancelStorageScan { job_id: stale_id }));
}

// ---------------------------------------------------------------------------
// Sort stability
// ---------------------------------------------------------------------------

#[test]
fn sort_change_reorders_large_files() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    let files = vec![
        LargeFile {
            path: PathBuf::from("/tmp/a.bin"),
            size_bytes: 100,
        },
        LargeFile {
            path: PathBuf::from("/tmp/b.bin"),
            size_bytes: 300,
        },
        LargeFile {
            path: PathBuf::from("/tmp/c.bin"),
            size_bytes: 200,
        },
    ];
    state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files: files,
        issues: Vec::new(),
    });

    // Default is SizeDesc.
    assert_eq!(state.large_files[0].size_bytes, 300);
    assert_eq!(state.large_files[1].size_bytes, 200);
    assert_eq!(state.large_files[2].size_bytes, 100);

    // Switch to SizeAsc.
    assert!(state.reduce(StorageIntent::SetLargeFileSort(LargeFileSort::SizeAsc)));
    assert_eq!(state.large_files[0].size_bytes, 100);
    assert_eq!(state.large_files[2].size_bytes, 300);

    // Same sort is a no-op.
    assert!(!state.reduce(StorageIntent::SetLargeFileSort(LargeFileSort::SizeAsc)));
}

#[test]
fn sort_by_path_orders_alphabetically() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    let files = vec![
        LargeFile {
            path: PathBuf::from("/tmp/zeta.bin"),
            size_bytes: 100,
        },
        LargeFile {
            path: PathBuf::from("/tmp/alpha.bin"),
            size_bytes: 200,
        },
    ];
    state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files: files,
        issues: Vec::new(),
    });

    state.reduce(StorageIntent::SetLargeFileSort(LargeFileSort::PathAsc));
    assert_eq!(state.large_files[0].path, PathBuf::from("/tmp/alpha.bin"));
    assert_eq!(state.large_files[1].path, PathBuf::from("/tmp/zeta.bin"));
}

// ---------------------------------------------------------------------------
// Directory drill-down
// ---------------------------------------------------------------------------

#[test]
fn drill_down_sets_path() {
    let mut state = StorageState::default();
    assert!(state.reduce(StorageIntent::DrillDown(PathBuf::from("/home/user/docs"))));
    assert_eq!(
        state.drill_down_path,
        Some(PathBuf::from("/home/user/docs"))
    );
}

#[test]
fn drill_up_navigates_parent() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::DrillDown(PathBuf::from("/home/user/docs")));
    assert!(state.reduce(StorageIntent::DrillUp));
    assert_eq!(state.drill_down_path, Some(PathBuf::from("/home/user")));
}

#[test]
fn drill_up_from_root_clears() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::DrillDown(PathBuf::from("/")));
    // Drilling up from "/" yields empty string parent which becomes None-equivalent.
    assert!(state.reduce(StorageIntent::DrillUp));
    // Parent of "/" is None on Unix.
    assert!(state.drill_down_path.is_none());
}

#[test]
fn drill_up_without_path_is_noop() {
    let mut state = StorageState::default();
    assert!(!state.reduce(StorageIntent::DrillUp));
}

// ---------------------------------------------------------------------------
// Duplicate selections default empty
// ---------------------------------------------------------------------------

fn make_duplicate_groups() -> Vec<DuplicateGroup> {
    vec![
        DuplicateGroup {
            digest: ContentDigest::from_bytes([1u8; 32]),
            size_bytes: 1024,
            files: vec![
                DuplicateFile {
                    path: PathBuf::from("/home/user/a.txt"),
                    identity: DeviceInode {
                        device: 1,
                        inode: 100,
                    },
                    modified: SystemTime::UNIX_EPOCH,
                },
                DuplicateFile {
                    path: PathBuf::from("/home/user/b.txt"),
                    identity: DeviceInode {
                        device: 1,
                        inode: 101,
                    },
                    modified: SystemTime::UNIX_EPOCH,
                },
            ],
        },
        DuplicateGroup {
            digest: ContentDigest::from_bytes([2u8; 32]),
            size_bytes: 2048,
            files: vec![
                DuplicateFile {
                    path: PathBuf::from("/home/user/c.dat"),
                    identity: DeviceInode {
                        device: 1,
                        inode: 200,
                    },
                    modified: SystemTime::UNIX_EPOCH,
                },
                DuplicateFile {
                    path: PathBuf::from("/home/user/d.dat"),
                    identity: DeviceInode {
                        device: 1,
                        inode: 201,
                    },
                    modified: SystemTime::UNIX_EPOCH,
                },
            ],
        },
    ]
}

#[test]
fn duplicate_selections_default_empty_after_scan() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::DuplicateScanCompleted {
        job_id,
        groups: make_duplicate_groups(),
        issues: Vec::new(),
    });

    assert!(state.duplicate_selections.is_empty());
}

#[test]
fn toggle_selection_adds_and_removes() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::DuplicateScanCompleted {
        job_id,
        groups: make_duplicate_groups(),
        issues: Vec::new(),
    });

    // Select file 1 from group 0.
    assert!(state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 0,
        file_idx: 1,
    }));
    assert_eq!(state.duplicate_selections.len(), 1);

    // Toggle again to deselect.
    assert!(state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 0,
        file_idx: 1,
    }));
    assert!(state.duplicate_selections.is_empty());
}

#[test]
fn invalid_selection_indices_rejected() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::DuplicateScanCompleted {
        job_id,
        groups: make_duplicate_groups(),
        issues: Vec::new(),
    });

    // Out of bounds group.
    assert!(!state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 99,
        file_idx: 0,
    }));
    // Out of bounds file.
    assert!(!state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 0,
        file_idx: 99,
    }));
}

// ---------------------------------------------------------------------------
// Full-path review before removal
// ---------------------------------------------------------------------------

#[test]
fn build_removal_plan_shows_full_paths() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::DuplicateScanCompleted {
        job_id,
        groups: make_duplicate_groups(),
        issues: Vec::new(),
    });

    // Select one file from each group.
    state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 0,
        file_idx: 1,
    });
    state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 1,
        file_idx: 0,
    });

    assert!(state.reduce(StorageIntent::BuildRemovalPlan));
    let plan = state.removal_review.as_ref().unwrap();
    assert_eq!(plan.targets.len(), 2);
    assert_eq!(plan.targets[0].path, PathBuf::from("/home/user/b.txt"));
    assert_eq!(plan.targets[0].size_bytes, 1024);
    assert!(plan.targets[0].trash_eligible);
    assert_eq!(plan.targets[1].path, PathBuf::from("/home/user/c.dat"));
    assert_eq!(plan.targets[1].size_bytes, 2048);
    assert_eq!(plan.total_bytes, 1024 + 2048);
}

#[test]
fn build_removal_plan_denied_with_empty_selections() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::DuplicateScanCompleted {
        job_id,
        groups: make_duplicate_groups(),
        issues: Vec::new(),
    });

    // No selections -> denied.
    assert!(!state.reduce(StorageIntent::BuildRemovalPlan));
    assert!(state.removal_review.is_none());
}

#[test]
fn dismiss_removal_review_clears() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::DuplicateScanCompleted {
        job_id,
        groups: make_duplicate_groups(),
        issues: Vec::new(),
    });
    state.reduce(StorageIntent::ToggleDuplicateSelection {
        group_idx: 0,
        file_idx: 0,
    });
    state.reduce(StorageIntent::BuildRemovalPlan);
    assert!(state.removal_review.is_some());

    assert!(state.reduce(StorageIntent::DismissRemovalReview));
    assert!(state.removal_review.is_none());
}

// ---------------------------------------------------------------------------
// Duplicate scan lifecycle
// ---------------------------------------------------------------------------

#[test]
fn duplicate_scan_progress_and_cancel() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home")));
    state.reduce(StorageIntent::StartDuplicateScan);
    let job_id = state.duplicate_scan.as_ref().unwrap().job_id;

    assert!(state.reduce(StorageIntent::DuplicateScanProgress {
        job_id,
        completed: 25,
        total: Some(100),
    }));
    assert!(state.reduce(StorageIntent::CancelDuplicateScan { job_id }));
    assert_eq!(
        state.duplicate_scan.as_ref().unwrap().phase,
        ScanPhase::Cancelled
    );
}

#[test]
fn can_start_new_scan_after_completed() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files: Vec::new(),
        issues: Vec::new(),
    });

    // Start a new scan after completion.
    assert!(state.reduce(StorageIntent::StartStorageScan));
    let new_id = state.storage_scan.as_ref().unwrap().job_id;
    assert_ne!(new_id, job_id);
}

#[test]
fn can_start_new_scan_after_cancelled() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;
    state.reduce(StorageIntent::CancelStorageScan { job_id });

    assert!(state.reduce(StorageIntent::StartStorageScan));
}

// ---------------------------------------------------------------------------
// Sub-page navigation
// ---------------------------------------------------------------------------

#[test]
fn navigate_sub_page() {
    let mut state = StorageState::default();
    assert_eq!(state.sub_page, StorageSubPage::Overview);
    assert!(state.reduce(StorageIntent::NavigateSubPage(StorageSubPage::LargeFiles)));
    assert_eq!(state.sub_page, StorageSubPage::LargeFiles);
}

// ---------------------------------------------------------------------------
// Recoverable issue count
// ---------------------------------------------------------------------------

#[test]
fn recoverable_issues_stored_on_completion() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/tmp")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    let issues = vec![
        ScanIssue {
            path: PathBuf::from("/tmp/a"),
            message: "permission denied".into(),
        },
        ScanIssue {
            path: PathBuf::from("/tmp/b"),
            message: "file disappeared".into(),
        },
    ];
    state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files: Vec::new(),
        issues,
    });

    assert_eq!(state.scan_issues.len(), 2);
}
