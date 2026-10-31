//! Local system report generator with structured redaction.
//!
//! Reports are assembled from already-collected snapshots and do NOT
//! trigger expensive operations or privileged helper calls.

pub mod export;
pub mod model;
pub mod redact;
pub mod render_text;

pub use model::{
    CapabilityNote, DiagnosticsSection, HardwareSection, HealthSection, MaintenanceSection,
    OsSection, RedactedReport, RedactionCategory, RedactionPolicy, SoftwareUpdatesSection,
    SystemReport, UpdateInfo,
};
pub use redact::redact_report;
pub use render_text::render_text;
