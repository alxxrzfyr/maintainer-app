//! Diagnostics page UI.
//!
//! Renders diagnostic findings in evidence-first order:
//! Detected, Evidence, Possible cause, Recommendation.
//! Group rows expand to bounded examples.

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Button, Label, ListBox, Orientation, ScrolledWindow, Separator};

use maintainer_core::diagnostics::{DiagnosticFinding, Evidence};

/// Presentation state for the diagnostics page.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DiagnosticsState {
    /// Current findings.
    pub findings: Vec<DiagnosticFinding>,
    /// Whether a scan is in progress.
    pub scanning: bool,
    /// Error message if the scan failed.
    pub error: Option<String>,
    /// Set of expanded finding indices (for evidence details).
    pub expanded: Vec<bool>,
}

/// Intents that drive diagnostics state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DiagnosticsIntent {
    /// Start a diagnostics scan.
    StartScan,
    /// Scan completed with findings.
    ScanCompleted(Vec<DiagnosticFinding>),
    /// Scan failed.
    ScanFailed(String),
    /// Toggle expansion of a finding row.
    ToggleExpanded(usize),
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

        // Header.
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

        // Scan button.
        let scan_button = Button::with_label("Run Diagnostics");
        scan_button.add_css_class("suggested-action");
        scan_button.set_halign(gtk4::Align::Start);
        container.append(&scan_button);

        // Status label.
        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        container.append(&status_label);

        container.append(&Separator::new(Orientation::Horizontal));

        // Scrolled findings list.
        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let findings_list = ListBox::new();
        findings_list.set_selection_mode(gtk4::SelectionMode::None);
        findings_list.add_css_class("boxed-list");
        scrolled.set_child(Some(&findings_list));
        container.append(&scrolled);

        Self {
            container,
            scan_button,
            status_label,
            findings_list,
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

    /// Update the view from the current state.
    pub fn update(&self, state: &DiagnosticsState) {
        // Update scanning state.
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

        // Rebuild findings list.
        while let Some(child) = self.findings_list.first_child() {
            self.findings_list.remove(&child);
        }
        for (idx, finding) in state.findings.iter().enumerate() {
            let expanded = state.expanded.get(idx).copied().unwrap_or(false);
            let row = build_finding_row(finding, expanded);
            self.findings_list.append(&row);
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

    // Detected.
    let detected_label = Label::new(Some(&format!("Detected: {}", finding.detected)));
    detected_label.add_css_class("heading");
    detected_label.set_halign(gtk4::Align::Start);
    detected_label.set_wrap(true);
    row.append(&detected_label);

    // Evidence.
    for evidence in &finding.evidence {
        let ev_box = build_evidence_widget(evidence, expanded);
        row.append(&ev_box);
    }

    // Possible causes.
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

    // Recommendation.
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
        // Should not panic.
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
}
