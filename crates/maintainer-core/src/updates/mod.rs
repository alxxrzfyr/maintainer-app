//! System update subsystem: APT discovery and execution progress parsing,
//! plus optional Flatpak/fwupd adapters.
//!
//! Design invariants:
//! - Discovery is always read-only and never mutates package state.
//! - An `UpdatePlan` is immutable; package-set changes invalidate it.
//! - Package mutation remains unavailable without a proven authoritative owner.
//! - Flatpak and fwupd are optional capabilities; their absence is normal.

/// APT execution requires one independent owner holding authoritative resolver
/// state across review and mutation. Legacy helpers do not provide that contract.
pub const PACKAGE_EXECUTION_UNAVAILABLE_REASON: &str = "Package changes unavailable: reviewed-plan binding and independent transaction ownership are not implemented. Update and kernel inspection remain available.";

pub mod apt;
pub mod apt_parser;
pub mod desktop;
pub mod flatpak;
pub mod fwupd;
pub mod model;
pub mod packagekit;
pub mod pkcon_parser;

pub use apt::{AptDiscovery, parse_apt_simulate, reboot_required};
pub use apt_parser::{AptProgressStatus, parse_apt_progress_line};
pub use desktop::{DesktopEnvironment, PackageCategory, classify_package};
pub use flatpak::FlatpakAdapter;
pub use fwupd::FwupdAdapter;
pub use model::{
    DiscoveryError, DiscoveryResult, HoldState, RemovalCandidate, TransactionStatus, UpdateBackend,
    UpdateCandidate, UpdateClassification, UpdateIntent, UpdatePlan, UpdateProgress, UpdateRequest,
    UpdateResult,
};
pub use packagekit::{
    PackageKitClient, TransactionState, check_packagekit_available, execute_update,
};
