//! Diagnostics subsystem: failed services, grouped journal events,
//! and evidence-first interpretation.

pub mod group;
pub mod interpret;
pub mod journal;
pub mod services;

pub use group::{EventGroup, GroupKey, group_journal_entries};
pub use interpret::{DiagnosticFinding, Evidence, InterpretationRule, interpret_groups};
pub use journal::{JournalCollector, JournalEntry, JournalFilter};
pub use services::{FailedService, ServiceDiagnostics};
