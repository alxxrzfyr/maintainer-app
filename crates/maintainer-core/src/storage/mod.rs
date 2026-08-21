//! Cancellable storage scanning and large-file detection.
//!
//! This module provides an iterative filesystem traversal that:
//! - respects explicit scan roots and exclusion lists,
//! - does not follow symlinks or cross filesystem boundaries by default,
//! - reports recoverable issues (permission denied, disappeared files),
//! - supports cooperative cancellation,
//! - emits throttled progress updates, and
//! - ranks large files using a bounded min-heap.

pub mod duplicates;
pub mod hash;
pub mod large_files;
pub mod mounts;
pub mod policy;
pub mod traverse;

pub use duplicates::{
    DuplicateCandidate, DuplicateFile, DuplicateGroup, DuplicateIssue, DuplicateScanResult,
    DuplicateScanner,
};
pub use hash::ContentDigest;
pub use large_files::{LargeFile, LargeFileRanking};
pub use mounts::DeviceInode;
pub use policy::ScanPolicy;
pub use traverse::{DirectoryUsage, ScanIssue, ScanSummary, StorageScanner};
