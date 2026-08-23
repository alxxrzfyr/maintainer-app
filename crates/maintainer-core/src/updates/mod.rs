//! System update subsystem: APT discovery and execution progress parsing,
//! plus optional Flatpak/fwupd adapters.
//!
//! Design invariants:
//! - Discovery is always read-only and never mutates package state.
//! - An `UpdatePlan` is immutable; package-set changes invalidate it.
//! - Maintainer's fixed helper wrapper owns execution authorization through
//!   Polkit; credentials are never stored or handled by the app.
//! - Flatpak and fwupd are optional capabilities; their absence is normal.

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
    UpdateCandidate, UpdateClassification, UpdatePlan, UpdateProgress, UpdateResult,
};
pub use packagekit::{
    PackageKitClient, TransactionState, check_packagekit_available, execute_update,
};
