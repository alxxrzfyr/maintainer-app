//! Diagnostics page UI.
//!
//! Renders diagnostic findings in evidence-first order:
//! Detected, Evidence, Possible cause, Recommendation.
//! Group rows expand to bounded examples.
//! Also displays SMART storage health from UDisks2.

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Button, Label, ListBox, Orientation, ScrolledWindow, Separator};

use maintainer_core::diagnostics::{DiagnosticFinding, Evidence, SmartAssessment, SmartDrive};

/// Presentation state for the diagnostics page.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct DiagnosticsState {
    /// Current findings.
    pub findings: Vec<DiagnosticFinding>,
    /// Whether a scan is in progress.
    pub scanning: bool,
    /// Error message if the scan failed.
    pub error: Option<String>,
    /// Set of expanded finding indices (for evidence details).
    pub expanded: Vec<bool>,
    /// SMART storage health state.
    pub smart: SmartHealthState,
}

/// SMART storage health presentation state.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SmartHealthState {
    /// Discovered drives with SMART data.
    pub drives: Vec<SmartDriveView>,
    /// Whether a refresh is in progress.
    pub refreshing: bool,
    /// Unavailability reason, if SMART cannot be queried.
    pub unavailable_reason: Option<String>,
    /// Whether permission is required.
    pub permission_required: bool,
}

/// View model for a single drive's SMART status.
#[derive(Debug, Clone, PartialEq)]
pub struct SmartDriveView {
    /// Drive model name.
    pub model: String,
    /// Health assessment label.
    pub assessment_label: String,
    /// CSS class for the assessment (for color coding).
    pub assessment_class: &'static str,
    /// Temperature in Celsius, if available.
    pub temperature_celsius: Option<f64>,
    /// Power-on hours.
    pub power_on_hours: Option<u64>,
    /// Data source (e.g., "UDisks2").
    pub source: &'static str,
    /// Whether this is a removable drive.
    pub removable: bool,
}

impl SmartDriveView {
    /// Create a view model from a core SmartDrive.
    pub fn from_smart_drive(drive: &SmartDrive) -> Self {
        let (assessment_label, assessment_class) = match &drive.assessment {
            SmartAssessment::Passed => ("Passed".to_string(), "smart-passed"),
            SmartAssessment::WarningReportedByDrive => {
                ("Warning reported by drive".to_string(), "smart-warning")
            }
            SmartAssessment::FailureReportedByDrive => {
                ("Failure reported by drive".to_string(), "smart-failure")
            }
            SmartAssessment::DataStale => ("Data stale".to_string(), "smart-stale"),
            SmartAssessment::Unavailable => ("Unavailable".to_string(), "smart-unavailable"),
        };

        Self {
            model: drive.model.clone(),
            assessment_label,
            assessment_class,
            temperature_celsius: drive.temperature_celsius.map(|t| t.value()),
            power_on_hours: drive.power_on_hours,
            source: drive.source,
            removable: drive.removable,
        }
    }
}

/// Intents that drive diagnostics state transitions.
#[derive(Debug, Clone, PartialEq)]
pub enum DiagnosticsIntent {
    /// Start a diagnostics scan.
    StartScan,
    /// Scan completed with findings.
    ScanCompleted(Vec<DiagnosticFinding>),
    /// Scan failed.
    ScanFailed(String),
    /// Toggle expansion of a finding row.
    ToggleExpanded(usize),
    /// Request SMART data refresh.
    RefreshSmart,
    /// SMART data loaded successfully.
    SmartLoaded(Vec<SmartDriveView>),
    /// SMART data unavailable with reason.
    SmartUnavailable(String),
    /// SMART requires permission.
    SmartPermissionRequired,
}

impl DiagnosticsState {
    /// Apply an intent to produce the next state.
    pub fn reduce(&mut self, intent: DiagnosticsIntent) {
        match intent {
            DiagnosticsIntent::StartScan => {
                self.scanning = true;
                self.error = None;
            }
            DiagnosticsIntent::ScanCompleted(findings) => {
                self.expanded = vec![false; findings.len()];
                self.findings = findings;
                self.scanning = false;
                self.error = None;
            }
            DiagnosticsIntent::ScanFailed(msg) => {
                self.scanning = false;
                self.error = Some(msg);
            }
            DiagnosticsIntent::ToggleExpanded(idx) => {
                if let Some(expanded) = self.expanded.get_mut(idx) {
                    *expanded = !*expanded;
                }
            }
            DiagnosticsIntent::RefreshSmart => {
                self.smart.refreshing = true;
                self.smart.unavailable_reason = None;
                self.smart.permission_required = false;
            }
            DiagnosticsIntent::SmartLoaded(drives) => {
                self.smart.drives = drives;
                self.smart.refreshing = false;
                self.smart.unavailable_reason = None;
                self.smart.permission_required = false;
            }
            DiagnosticsIntent::SmartUnavailable(reason) => {
                self.smart.refreshing = false;
                self.smart.unavailable_reason = Some(reason);
                self.smart.permission_required = false;
            }
            DiagnosticsIntent::SmartPermissionRequired => {
                self.smart.refreshing = false;
                self.smart.permission_required = true;
            }
        }
    }
}

/// Top-level diagnostics page widget.
pub struct DiagnosticsPage {
    container: GtkBox,
    scan_button: Button,
    status_label: Label,
    #[allow(dead_code)]
    findings_list: ListBox,
    #[allow(dead_code)]
    smart_section: GtkBox,
    smart_refresh_button: Button,
    smart_status_label: Label,
    smart_drives_list: ListBox,
}

impl Default for DiagnosticsPage {
    fn default() -> Self {
        Self::new()
    }
}

impl DiagnosticsPage {
    /// Build the diagnostics page widget tree.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 12);
        container.add_css_class("diagnostics-page");
        container.set_margin_top(24);
        container.set_margin_bottom(24);
        container.set_margin_start(24);
        container.set_margin_end(24);

        let header = Label::new(Some("System Diagnostics"));
        header.add_css_class("title-1");
        header.set_halign(gtk4::Align::Start);
        container.append(&header);

        let description = Label::new(Some(
            "Scan for failed services and recurring error patterns in the system journal. \
             Findings are presented with evidence first — no speculation.",
        ));
        description.set_wrap(true);
        description.set_halign(gtk4::Align::Start);
        description.add_css_class("dim-label");
        container.append(&description);

        let scan_button = Button::with_label("Run Diagnostics");
        scan_button.add_css_class("suggested-action");
        scan_button.set_halign(gtk4::Align::Start);
        container.append(&scan_button);

        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        container.append(&Separator::new(Orientation::Horizontal));

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let findings_list = ListBox::new();
        findings_list.set_selection_mode(gtk4::SelectionMode::None);
        findings_list.add_css_class("boxed-list");
        scrolled.set_child(Some(&findings_list));
        container.append(&scrolled);

        container.append(&Separator::new(Orientation::Horizontal));

        let smart_section = GtkBox::new(Orientation::Vertical, 8);
        smart_section.set_margin_top(12);

        let smart_header = Label::new(Some("Storage Health (SMART)"));
        smart_header.add_css_class("title-2");
        smart_header.set_halign(gtk4::Align::Start);
        smart_section.append(&smart_header);

        let smart_desc = Label::new(Some(
            "On-demand SMART data from UDisks2. Refresh may spin up sleeping disks.",
        ));
        smart_desc.set_wrap(true);
        smart_desc.set_halign(gtk4::Align::Start);
        smart_desc.add_css_class("dim-label");
        smart_section.append(&smart_desc);

        let smart_refresh_button = Button::with_label("Refresh SMART Data");
        smart_refresh_button.set_halign(gtk4::Align::Start);
        smart_section.append(&smart_refresh_button);

        let smart_status_label = Label::new(None);
        smart_status_label.set_halign(gtk4::Align::Start);
        smart_status_label.set_visible(false);
        smart_section.append(&smart_status_label);

        let smart_drives_list = ListBox::new();
        smart_drives_list.set_selection_mode(gtk4::SelectionMode::None);
        smart_drives_list.add_css_class("boxed-list");
        smart_section.append(&smart_drives_list);

        container.append(&smart_section);

        Self {
            container,
            scan_button,
            status_label,
            findings_list,
            smart_section,
            smart_refresh_button,
            smart_status_label,
            smart_drives_list,
        }
    }

    /// The top-level widget for embedding in the page stack.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Access the scan button for signal connections.
    pub fn scan_button(&self) -> &Button {
        &self.scan_button
    }

    /// Access the SMART refresh button for signal connections.
    pub fn smart_refresh_button(&self) -> &Button {
        &self.smart_refresh_button
    }

    /// Update the view from the current state.
    pub fn update(&self, state: &DiagnosticsState) {
        self.scan_button.set_sensitive(!state.scanning);
        if state.scanning {
            self.status_label.set_text("Scanning…");
            self.status_label.set_visible(true);
        } else if let Some(ref err) = state.error {
            self.status_label.set_text(err);
            self.status_label.set_visible(true);
            self.status_label.add_css_class("error");
        } else if state.findings.is_empty() {
            self.status_label
                .set_text("No issues detected. System appears healthy.");
            self.status_label.set_visible(true);
            self.status_label.remove_css_class("error");
        } else {
            self.status_label.set_visible(false);
        }

        while let Some(child) = self.findings_list.first_child() {
            self.findings_list.remove(&child);
        }
        for (idx, finding) in state.findings.iter().enumerate() {
            let expanded = state.expanded.get(idx).copied().unwrap_or(false);
            let row = build_finding_row(finding, expanded);
            self.findings_list.append(&row);
        }

        self.update_smart(&state.smart);
    }

    /// Update the SMART storage health section.
    fn update_smart(&self, smart: &SmartHealthState) {
        self.smart_refresh_button.set_sensitive(!smart.refreshing);

        if smart.refreshing {
            self.smart_status_label.set_text("Querying SMART data…");
            self.smart_status_label.set_visible(true);
            self.smart_status_label.remove_css_class("error");
        } else if smart.permission_required {
            self.smart_status_label
                .set_text("Permission required to read SMART data.");
            self.smart_status_label.set_visible(true);
            self.smart_status_label.remove_css_class("error");
        } else if let Some(ref reason) = smart.unavailable_reason {
            self.smart_status_label.set_text(reason);
            self.smart_status_label.set_visible(true);
            self.smart_status_label.add_css_class("error");
        } else if smart.drives.is_empty() {
            self.smart_status_label
                .set_text("No drives queried yet. Press Refresh to load SMART data.");
            self.smart_status_label.set_visible(true);
            self.smart_status_label.remove_css_class("error");
        } else {
            self.smart_status_label.set_visible(false);
        }

        while let Some(child) = self.smart_drives_list.first_child() {
            self.smart_drives_list.remove(&child);
        }
        for drive in &smart.drives {
            let row = build_smart_drive_row(drive);
            self.smart_drives_list.append(&row);
        }
    }
}

/// Build a single finding row widget.
fn build_finding_row(finding: &DiagnosticFinding, expanded: bool) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 8);
    row.set_margin_top(12);
    row.set_margin_bottom(12);
    row.set_margin_start(12);
    row.set_margin_end(12);

    let detected_label = Label::new(Some(&format!("Detected: {}", finding.detected)));
    detected_label.add_css_class("heading");
    detected_label.set_halign(gtk4::Align::Start);
    detected_label.set_wrap(true);
    row.append(&detected_label);

    for evidence in &finding.evidence {
        let ev_box = build_evidence_widget(evidence, expanded);
        row.append(&ev_box);
    }

    if !finding.possible_causes.is_empty() {
        let causes_header = Label::new(Some("Possible cause:"));
        causes_header.add_css_class("dim-label");
        causes_header.set_halign(gtk4::Align::Start);
        row.append(&causes_header);

        for cause in &finding.possible_causes {
            let cause_label = Label::new(Some(&format!("• {cause}")));
            cause_label.set_halign(gtk4::Align::Start);
            cause_label.set_wrap(true);
            cause_label.set_margin_start(12);
            row.append(&cause_label);
        }
    }

    if let Some(ref rec) = finding.recommendation {
        let rec_header = Label::new(Some("Recommendation:"));
        rec_header.add_css_class("dim-label");
        rec_header.set_halign(gtk4::Align::Start);
        row.append(&rec_header);

        let rec_label = Label::new(Some(rec));
        rec_label.set_halign(gtk4::Align::Start);
        rec_label.set_wrap(true);
        rec_label.set_margin_start(12);
        row.append(&rec_label);
    }

    row
}

/// Build evidence display widget.
fn build_evidence_widget(evidence: &Evidence, expanded: bool) -> GtkBox {
    let ev_box = GtkBox::new(Orientation::Vertical, 4);
    ev_box.set_margin_start(12);

    let header_parts = if let Some(count) = evidence.count {
        format!("Evidence ({}×, source: {})", count, evidence.source)
    } else {
        format!("Evidence (source: {})", evidence.source)
    };

    let header = Label::new(Some(&header_parts));
    header.add_css_class("dim-label");
    header.set_halign(gtk4::Align::Start);
    ev_box.append(&header);

    if expanded || evidence.observation.len() <= 120 {
        let obs_label = Label::new(Some(&evidence.observation));
        obs_label.set_halign(gtk4::Align::Start);
        obs_label.set_wrap(true);
        obs_label.add_css_class("monospace");
        ev_box.append(&obs_label);
    } else {
        let truncated = &evidence.observation[..120.min(evidence.observation.len())];
        let obs_label = Label::new(Some(&format!("{truncated}…")));
        obs_label.set_halign(gtk4::Align::Start);
        obs_label.set_wrap(true);
        obs_label.add_css_class("monospace");
        ev_box.append(&obs_label);
    }

    ev_box
}

/// Build a SMART drive row widget.
fn build_smart_drive_row(drive: &SmartDriveView) -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 4);
    row.set_margin_top(8);
    row.set_margin_bottom(8);
    row.set_margin_start(12);
    row.set_margin_end(12);

    let header_box = GtkBox::new(Orientation::Horizontal, 12);

    let model_label = Label::new(Some(&drive.model));
    model_label.add_css_class("heading");
    model_label.set_halign(gtk4::Align::Start);
    model_label.set_hexpand(true);
    header_box.append(&model_label);

    let assessment_label = Label::new(Some(&drive.assessment_label));
    assessment_label.add_css_class(drive.assessment_class);
    assessment_label.set_halign(gtk4::Align::End);
    header_box.append(&assessment_label);

    row.append(&header_box);

    let mut details_parts = Vec::new();
    if let Some(temp) = drive.temperature_celsius {
        details_parts.push(format!("{temp:.0} °C"));
    }
    if let Some(hours) = drive.power_on_hours {
        details_parts.push(format!("{hours} power-on hours"));
    }
    details_parts.push(format!("Source: {}", drive.source));
    if drive.removable {
        details_parts.push("Removable".into());
    }

    let details = details_parts.join(" · ");
    let details_label = Label::new(Some(&details));
    details_label.add_css_class("dim-label");
    details_label.set_halign(gtk4::Align::Start);
    row.append(&details_label);

    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initial_state_is_empty() {
        let state = DiagnosticsState::default();
        assert!(state.findings.is_empty());
        assert!(!state.scanning);
        assert!(state.error.is_none());
        assert!(state.expanded.is_empty());
        assert!(state.smart.drives.is_empty());
        assert!(!state.smart.refreshing);
    }

    #[test]
    fn start_scan_sets_scanning() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::StartScan);
        assert!(state.scanning);
        assert!(state.error.is_none());
    }

    #[test]
    fn scan_completed_updates_findings() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::StartScan);

        let findings = vec![DiagnosticFinding {
            detected: "test finding".into(),
            evidence: vec![Evidence {
                observation: "saw this".into(),
                source: "journal",
                count: Some(3),
            }],
            possible_causes: vec!["maybe this".into()],
            recommendation: Some("do this".into()),
        }];

        state.reduce(DiagnosticsIntent::ScanCompleted(findings.clone()));
        assert!(!state.scanning);
        assert_eq!(state.findings.len(), 1);
        assert_eq!(state.findings[0].detected, "test finding");
        assert_eq!(state.expanded, vec![false]);
    }

    #[test]
    fn scan_failed_sets_error() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::StartScan);
        state.reduce(DiagnosticsIntent::ScanFailed("boom".into()));
        assert!(!state.scanning);
        assert_eq!(state.error.as_deref(), Some("boom"));
    }

    #[test]
    fn toggle_expanded_flips_state() {
        let mut state = DiagnosticsState::default();
        let findings = vec![
            DiagnosticFinding {
                detected: "a".into(),
                evidence: vec![],
                possible_causes: vec![],
                recommendation: None,
            },
            DiagnosticFinding {
                detected: "b".into(),
                evidence: vec![],
                possible_causes: vec![],
                recommendation: None,
            },
        ];
        state.reduce(DiagnosticsIntent::ScanCompleted(findings));

        assert_eq!(state.expanded, vec![false, false]);
        state.reduce(DiagnosticsIntent::ToggleExpanded(1));
        assert_eq!(state.expanded, vec![false, true]);
        state.reduce(DiagnosticsIntent::ToggleExpanded(1));
        assert_eq!(state.expanded, vec![false, false]);
    }

    #[test]
    fn toggle_out_of_bounds_is_safe() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::ScanCompleted(vec![]));
        state.reduce(DiagnosticsIntent::ToggleExpanded(99));
        assert!(state.expanded.is_empty());
    }

    #[test]
    fn new_scan_clears_previous_error() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::ScanFailed("old error".into()));
        state.reduce(DiagnosticsIntent::StartScan);
        assert!(state.error.is_none());
    }

    #[test]
    fn refresh_smart_sets_refreshing() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::RefreshSmart);
        assert!(state.smart.refreshing);
        assert!(state.smart.unavailable_reason.is_none());
    }

    #[test]
    fn smart_loaded_updates_drives() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::RefreshSmart);

        let drives = vec![SmartDriveView {
            model: "WDC WD10EZEX".into(),
            assessment_label: "Passed".into(),
            assessment_class: "smart-passed",
            temperature_celsius: Some(40.0),
            power_on_hours: Some(12345),
            source: "UDisks2",
            removable: false,
        }];

        state.reduce(DiagnosticsIntent::SmartLoaded(drives));
        assert!(!state.smart.refreshing);
        assert_eq!(state.smart.drives.len(), 1);
        assert_eq!(state.smart.drives[0].model, "WDC WD10EZEX");
    }

    #[test]
    fn smart_unavailable_sets_reason() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::RefreshSmart);
        state.reduce(DiagnosticsIntent::SmartUnavailable(
            "UDisks2 not installed".into(),
        ));
        assert!(!state.smart.refreshing);
        assert_eq!(
            state.smart.unavailable_reason.as_deref(),
            Some("UDisks2 not installed")
        );
    }

    #[test]
    fn smart_permission_required_sets_flag() {
        let mut state = DiagnosticsState::default();
        state.reduce(DiagnosticsIntent::RefreshSmart);
        state.reduce(DiagnosticsIntent::SmartPermissionRequired);
        assert!(!state.smart.refreshing);
        assert!(state.smart.permission_required);
    }

    #[test]
    fn smart_drive_view_from_smart_drive_passed() {
        use maintainer_core::diagnostics::{SmartAssessment, SmartDrive, TemperatureCelsius};
        use std::time::SystemTime;

        let drive = SmartDrive {
            object_path: "/org/freedesktop/UDisks2/drives/WDC".into(),
            model: "WDC WD10EZEX-00BN5A0".into(),
            firmware: "01.01A01".into(),
            serial: "WD-WMC4N0123456".into(),
            removable: false,
            assessment: SmartAssessment::Passed,
            temperature_celsius: Some(TemperatureCelsius::from_kelvin(313.15)),
            power_on_hours: Some(12345),
            attributes: vec![],
            sample_time: Some(SystemTime::now()),
            source: "UDisks2",
        };

        let view = SmartDriveView::from_smart_drive(&drive);
        assert_eq!(view.model, "WDC WD10EZEX-00BN5A0");
        assert_eq!(view.assessment_label, "Passed");
        assert_eq!(view.assessment_class, "smart-passed");
        assert!((view.temperature_celsius.unwrap() - 40.0).abs() < 0.01);
        assert_eq!(view.power_on_hours, Some(12345));
        assert!(!view.removable);
    }

    #[test]
    fn smart_drive_view_from_smart_drive_failure() {
        use maintainer_core::diagnostics::{SmartAssessment, SmartDrive};

        let drive = SmartDrive {
            object_path: "/org/freedesktop/UDisks2/drives/FAIL".into(),
            model: "Failing Drive".into(),
            firmware: "1.0".into(),
            serial: "SN123".into(),
            removable: false,
            assessment: SmartAssessment::FailureReportedByDrive,
            temperature_celsius: None,
            power_on_hours: None,
            attributes: vec![],
            sample_time: None,
            source: "UDisks2",
        };

        let view = SmartDriveView::from_smart_drive(&drive);
        assert_eq!(view.assessment_label, "Failure reported by drive");
        assert_eq!(view.assessment_class, "smart-failure");
    }
}
