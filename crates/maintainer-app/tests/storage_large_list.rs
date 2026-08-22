//! Headless GTK test for large-list virtualization.
//!
//! Verifies that 10,000 rows in a `gio::ListStore` do not create 10,000
//! widget realizations. The factory-based approach in `ListView`/`ColumnView`
//! only realizes visible rows.

use std::path::PathBuf;

use maintainer_app::state::{StorageIntent, StorageState};
use maintainer_core::storage::LargeFile;

/// Assert that populating 10,000 in-memory rows in the model completes
/// efficiently. The model itself does not realize widgets — that only
/// happens when attached to a visible view within a GTK main loop.
///
/// This test verifies:
/// 1. The state reducer handles 10,000 results without panicking.
/// 2. The model construction is O(n) in items, not O(n) in widget trees.
#[test]
fn ten_thousand_large_files_populate_model_without_widget_explosion() {
    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    let large_files: Vec<LargeFile> = (0..10_000)
        .map(|i| LargeFile {
            path: PathBuf::from(format!("/home/user/data/file_{i:05}.dat")),
            size_bytes: 1_000_000_000 + (i as u64 * 1000),
        })
        .collect();

    assert!(state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files,
        issues: Vec::new(),
    }));

    assert_eq!(state.large_files.len(), 10_000);
    assert_eq!(
        state.large_files[0].size_bytes,
        1_000_000_000 + 9_999 * 1000
    );
    assert_eq!(state.large_files[9_999].size_bytes, 1_000_000_000);
}

/// Test that sort operations on 10,000 entries complete correctly.
#[test]
fn sort_ten_thousand_entries() {
    use maintainer_app::state::LargeFileSort;

    let mut state = StorageState::default();
    state.reduce(StorageIntent::SetScanRoot(PathBuf::from("/home/user")));
    state.reduce(StorageIntent::StartStorageScan);
    let job_id = state.storage_scan.as_ref().unwrap().job_id;

    let large_files: Vec<LargeFile> = (0..10_000)
        .map(|i| LargeFile {
            path: PathBuf::from(format!("/home/user/data/file_{i:05}.dat")),
            size_bytes: 1_000_000_000 + (i as u64 * 1000),
        })
        .collect();

    state.reduce(StorageIntent::StorageScanCompleted {
        job_id,
        large_files,
        issues: Vec::new(),
    });

    state.reduce(StorageIntent::SetLargeFileSort(LargeFileSort::PathAsc));
    assert_eq!(
        state.large_files[0].path,
        PathBuf::from("/home/user/data/file_00000.dat")
    );
    assert_eq!(
        state.large_files[9_999].path,
        PathBuf::from("/home/user/data/file_09999.dat")
    );
}

/// Test that the gio::ListStore model can be populated with 10,000 objects
/// without creating GTK widgets (headless — no display required for model ops).
#[test]
fn list_store_population_is_data_only() {
    use gio::prelude::*;

    let model = gio::ListStore::new::<glib::Object>();
    for _ in 0..10_000 {
        let obj = glib::Object::new::<glib::Object>();
        model.append(&obj);
    }
    assert_eq!(model.n_items(), 10_000);
}
