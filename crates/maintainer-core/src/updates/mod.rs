//! System update subsystem: APT discovery, PackageKit execution,
//! and optional Flatpak/fwupd adapters.
//!
//! Design invariants:
//! - Discovery is always read-only and never mutates package state.
//! - An `UpdatePlan` is immutable; package-set changes invalidate it.
//! - PackageKit owns authorization through Polkit; we never store credentials.
//! - Flatpak and fwupd are optional capabilities; their absence is normal.

pub mod apt;
pub mod flatpak;
pub mod fwupd;
pub mod model;
pub mod packagekit;

pub use apt::{AptDiscovery, parse_apt_simulate, reboot_required};
pub use flatpak::FlatpakAdapter;
pub use fwupd::FwupdAdapter;
pub use model::{
    DiscoveryError, DiscoveryResult, HoldState, RemovalCandidate, TransactionStatus, UpdateBackend,
    UpdateCandidate, UpdateClassification, UpdatePlan, UpdateProgress, UpdateResult,
};
pub use packagekit::{
    PackageKitClient, TransactionState, check_packagekit_available, execute_update,
};
