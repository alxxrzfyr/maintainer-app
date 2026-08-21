//! Report data model: typed sections carrying sensitive fields that are
//! redacted before rendering rather than stripped post-hoc from text.

use crate::capability::Capability;
use crate::diagnostics::{EventGroup, FailedService, SmartDrive};
use crate::metrics::{
    CpuSnapshot, DiskSnapshot, MemorySnapshot, NetworkSnapshot, TemperatureReading,
};
use crate::persistence::history::HistoryEntry;

/// Categories of sensitive data that the default policy redacts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum RedactionCategory {
    Username,
    Hostname,
    Ipv4,
    Ipv6,
    Mac,
    Serial,
    HomePath,
    MountedPath,
    ProcessArgs,
    Email,
}

impl RedactionCategory {
    /// All categories the PRD mandates for default redaction.
    pub const ALL: &'static [Self] = &[
        Self::Username,
        Self::Hostname,
        Self::Ipv4,
        Self::Ipv6,
        Self::Mac,
        Self::Serial,
        Self::HomePath,
        Self::MountedPath,
        Self::ProcessArgs,
        Self::Email,
    ];
}

/// Policy controlling which categories are redacted.
#[derive(Debug, Clone)]
pub struct RedactionPolicy {
    /// Set of categories to redact. Default includes all.
    pub categories: Vec<RedactionCategory>,
}

impl Default for RedactionPolicy {
    fn default() -> Self {
        Self {
            categories: RedactionCategory::ALL.to_vec(),
        }
    }
}

impl RedactionPolicy {
    /// Whether the given category should be redacted.
    pub fn should_redact(&self, cat: RedactionCategory) -> bool {
        self.categories.contains(&cat)
    }
}

/// Metadata about the system and report generation.
#[derive(Debug, Clone)]
pub struct OsSection {
    /// Distribution name and version (e.g. "Ubuntu 24.04.1 LTS").
    pub distro: String,
    /// Kernel version string.
    pub kernel: String,
    /// System uptime in seconds.
    pub uptime_seconds: u64,
    /// Desktop environment, if detectable.
    pub desktop: Option<String>,
    /// Hostname — sensitive, redacted by default.
    pub hostname: String,
    /// Username — sensitive, redacted by default.
    pub username: String,
}

/// Hardware summary section.
#[derive(Debug, Clone)]
pub struct HardwareSection {
    pub cpu: Capability<CpuSnapshot>,
    pub memory: Capability<MemorySnapshot>,
    pub disks: Capability<Vec<DiskSnapshot>>,
    pub network: Capability<Vec<NetworkSnapshot>>,
    pub temperatures: Capability<Vec<TemperatureReading>>,
}

/// Storage health section.
#[derive(Debug, Clone)]
pub struct HealthSection {
    pub smart_drives: Capability<Vec<SmartDrive>>,
}

/// Software update information.
#[derive(Debug, Clone)]
pub struct UpdateInfo {
    pub package_name: String,
    pub current_version: String,
    pub available_version: String,
}

/// Software updates section.
#[derive(Debug, Clone)]
pub struct SoftwareUpdatesSection {
    pub pending_updates: Capability<Vec<UpdateInfo>>,
}

/// Diagnostics section: failed services and grouped events.
#[derive(Debug, Clone)]
pub struct DiagnosticsSection {
    pub failed_services: Capability<Vec<FailedService>>,
    pub event_groups: Capability<Vec<EventGroup>>,
}

/// Recent maintenance history.
#[derive(Debug, Clone)]
pub struct MaintenanceSection {
    pub recent_history: Vec<HistoryEntry>,
}

/// A note about a capability's availability.
#[derive(Debug, Clone)]
pub struct CapabilityNote {
    pub name: String,
    pub status: String,
    pub reason: Option<String>,
}

/// The full structured system report before redaction.
pub struct SystemReport {
    pub generated_at: String,
    pub os: OsSection,
    pub hardware: HardwareSection,
    pub health: HealthSection,
    pub software_updates: SoftwareUpdatesSection,
    pub diagnostics: DiagnosticsSection,
    pub maintenance: MaintenanceSection,
    pub capability_notes: Vec<CapabilityNote>,
}

/// A report after redaction has been applied, ready for rendering.
pub struct RedactedReport {
    pub generated_at: String,
    pub os: RedactedOsSection,
    pub hardware: RedactedHardwareSection,
    pub health: RedactedHealthSection,
    pub software_updates: SoftwareUpdatesSection,
    pub diagnostics: RedactedDiagnosticsSection,
    pub maintenance: MaintenanceSection,
    pub capability_notes: Vec<CapabilityNote>,
    pub policy: RedactionPolicy,
}

/// OS section after redaction.
#[derive(Debug, Clone)]
pub struct RedactedOsSection {
    pub distro: String,
    pub kernel: String,
    pub uptime_seconds: u64,
    pub desktop: Option<String>,
    /// Replaced with "[hostname]" when redacted.
    pub hostname: String,
    /// Replaced with "[username]" when redacted.
    pub username: String,
}

/// Hardware section after redaction (paths and MACs removed).
#[derive(Debug, Clone)]
pub struct RedactedHardwareSection {
    pub cpu_summary: Option<String>,
    pub memory_summary: Option<String>,
    pub disk_summaries: Vec<String>,
    pub network_summaries: Vec<String>,
    pub temperature_summaries: Vec<String>,
    pub unavailable_notes: Vec<String>,
}

/// Health section after redaction (serials stripped).
#[derive(Debug, Clone)]
pub struct RedactedHealthSection {
    pub drive_summaries: Vec<RedactedDriveSummary>,
    pub unavailable_note: Option<String>,
}

/// A single drive health summary post-redaction.
#[derive(Debug, Clone)]
pub struct RedactedDriveSummary {
    pub model: String,
    pub assessment: String,
    pub temperature_celsius: Option<f64>,
    pub power_on_hours: Option<u64>,
}

/// Diagnostics section after redaction (process args removed).
#[derive(Debug, Clone)]
pub struct RedactedDiagnosticsSection {
    pub failed_services: Vec<RedactedServiceSummary>,
    pub event_groups: Vec<RedactedEventSummary>,
    pub unavailable_notes: Vec<String>,
}

/// A failed service entry post-redaction.
#[derive(Debug, Clone)]
pub struct RedactedServiceSummary {
    pub unit: String,
    pub description: String,
    pub state: String,
}

/// A grouped event summary post-redaction.
#[derive(Debug, Clone)]
pub struct RedactedEventSummary {
    pub template: String,
    pub count: u32,
    pub unit: Option<String>,
    pub priority: u8,
}
