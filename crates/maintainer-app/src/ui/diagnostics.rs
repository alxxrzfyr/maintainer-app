//! Diagnostics page UI.
//!
//! Renders diagnostic findings in evidence-first order:
//! Detected, Evidence, Possible cause, Recommendation.
//! Group rows expand to bounded examples.
//! Also displays SMART storage health from UDisks2.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{self, Box as GtkBox, Button, Label, ListBox, Orientation, ScrolledWindow, Separator};

use maintainer_core::diagnostics::{DiagnosticFinding, Evidence, SmartAssessment, SmartDrive};

type ToggleCallback = Rc<RefCell<Option<Box<dyn Fn(usize)>>>>;

/// Presentation state for the diagnostics page.
#[derive(Debug, Clone, PartialEq)]
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
    /// Hardware specifications snapshot (GPU, USB, PCI, BIOS).
    pub hardware: HardwareState,
}

impl Default for DiagnosticsState {
    fn default() -> Self {
        Self {
            findings: Vec::new(),
            scanning: false,
            error: None,
            expanded: Vec::new(),
            smart: SmartHealthState::default(),
            hardware: HardwareState::Loading,
        }
    }
}

/// The initial page contains no system data until its bounded worker finishes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HardwareState {
    Loading,
    Loaded(Rc<maintainer_core::hardware::HardwareSnapshot>),
    Failed(String),
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
    /// Hardware collection completed or failed.
    HardwareUpdated(HardwareState),
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
            DiagnosticsIntent::HardwareUpdated(hardware) => self.hardware = hardware,
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
#[derive(Clone)]
pub struct DiagnosticsPage {
    container: GtkBox,
    scan_button: Button,
    status_label: Label,
    findings_list: ListBox,
    smart_refresh_button: Button,
    smart_status_label: Label,
    smart_drives_list: ListBox,
    on_toggle_expanded: ToggleCallback,
    hardware_hosts: [GtkBox; 4],
    rendered_hardware: Rc<RefCell<HardwareState>>,
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
        container.set_margin_top(16);
        container.set_margin_bottom(24);
        container.set_margin_start(24);
        container.set_margin_end(24);

        let stack = gtk4::Stack::new();
        stack.set_vexpand(true);
        stack.set_hexpand(true);

        let health_box = GtkBox::new(Orientation::Vertical, 12);

        let header = Label::new(Some("System Diagnostics"));
        header.add_css_class("title-1");
        header.set_halign(gtk4::Align::Start);
        health_box.append(&header);

        let description = Label::new(Some(
            "Scan for failed services and recurring error patterns in the system journal. \
             Findings are presented with evidence first — no speculation.",
        ));
        description.set_wrap(true);
        description.set_halign(gtk4::Align::Start);
        description.add_css_class("dim-label");
        health_box.append(&description);

        let scan_button = Button::with_label("Run Diagnostics");
        scan_button.add_css_class("suggested-action");
        scan_button.set_halign(gtk4::Align::Start);
        health_box.append(&scan_button);

        let status_label = Label::new(None);
        status_label.set_halign(gtk4::Align::Start);
        status_label.set_visible(false);
        health_box.append(&status_label);

        health_box.append(&Separator::new(Orientation::Horizontal));

        let scrolled = ScrolledWindow::new();
        scrolled.set_vexpand(true);
        scrolled.set_hscrollbar_policy(gtk4::PolicyType::Never);

        let findings_list = ListBox::new();
        findings_list.set_selection_mode(gtk4::SelectionMode::None);
        findings_list.set_activate_on_single_click(true);
        findings_list.add_css_class("boxed-list");
        let on_toggle_expanded: ToggleCallback = Rc::new(RefCell::new(None));
        let on_toggle_for_rows = on_toggle_expanded.clone();
        findings_list.connect_row_activated(move |_, row| {
            if let Some(callback) = on_toggle_for_rows.borrow().as_ref() {
                callback(row.index() as usize);
            }
        });
        scrolled.set_child(Some(&findings_list));
        health_box.append(&scrolled);

        health_box.append(&Separator::new(Orientation::Horizontal));

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

        health_box.append(&smart_section);

        let health_scrolled = ScrolledWindow::new();
        health_scrolled.set_vexpand(true);
        health_scrolled.set_hexpand(true);
        health_scrolled.set_child(Some(&health_box));

        stack.add_titled(&health_scrolled, Some("health"), "System Health");
        let hardware_hosts = std::array::from_fn(|_| {
            let host = GtkBox::new(Orientation::Vertical, 8);
            host.append(&Label::new(Some("Loading hardware information…")));
            host
        });
        for (host, (name, title)) in hardware_hosts.iter().zip([
            ("gpu", "GPU"),
            ("usb", "USB"),
            ("pci", "PCI"),
            ("bios", "BIOS"),
        ]) {
            stack.add_titled(host, Some(name), title);
        }

        let switcher = gtk4::StackSwitcher::new();
        switcher.set_stack(Some(&stack));
        switcher.set_halign(gtk4::Align::Center);
        switcher.set_margin_bottom(8);
        switcher.add_css_class("maintainer_stack_switcher");

        container.append(&switcher);
        container.append(&stack);

        Self {
            container,
            scan_button,
            status_label,
            findings_list,
            smart_refresh_button,
            smart_status_label,
            smart_drives_list,
            on_toggle_expanded,
            hardware_hosts,
            rendered_hardware: Rc::new(RefCell::new(HardwareState::Loading)),
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

    /// Connect finding-row expansion for mouse and keyboard activation.
    pub fn connect_toggle_expanded<F: Fn(usize) + 'static>(&self, callback: F) {
        *self.on_toggle_expanded.borrow_mut() = Some(Box::new(callback));
    }

    /// Show scan progress directly.
    pub fn show_scan_progress(&self, completed: u64, total: Option<u64>, message: &str) {
        self.scan_button.set_sensitive(false);
        let text = match total {
            Some(t) if t > 0 => format!("{message} ({completed}/{t})"),
            _ => format!("{message} ({completed} items)"),
        };
        self.status_label.set_text(&text);
        self.status_label.set_visible(true);
        self.status_label.remove_css_class("error");
    }

    /// Get a clone of the status label for use in progress closures.
    pub fn status_label_handle(&self) -> Label {
        self.status_label.clone()
    }

    /// Access the SMART refresh button for signal connections.
    pub fn smart_refresh_button(&self) -> &Button {
        &self.smart_refresh_button
    }

    /// Update the view from the current state.
    pub fn update(&self, state: &DiagnosticsState) {
        self.scan_button.set_sensitive(!state.scanning);
        if state.scanning {
            self.scan_button.set_label("Running Diagnostics…");
            self.status_label.set_text("Scanning…");
            self.status_label.set_visible(true);
        } else {
            self.scan_button.set_label("Run Diagnostics");
            if let Some(ref err) = state.error {
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
        self.update_hardware(&state.hardware);
    }

    /// Render only hardware widgets when the background inventory changes.
    pub(crate) fn update_hardware(&self, hardware: &HardwareState) {
        if *self.rendered_hardware.borrow() == *hardware {
            return;
        }
        *self.rendered_hardware.borrow_mut() = hardware.clone();
        for host in &self.hardware_hosts {
            while let Some(child) = host.first_child() {
                host.remove(&child);
            }
        }
        match hardware {
            HardwareState::Loaded(snapshot) => {
                for host in &self.hardware_hosts {
                    for issue in &snapshot.collection_issues {
                        let label = Label::new(Some(issue));
                        label.set_wrap(true);
                        label.add_css_class("dim-label");
                        host.append(&label);
                    }
                }
                for (host, view) in self.hardware_hosts.iter().zip([
                    build_gpu_view(&snapshot.gpu),
                    build_usb_view(&snapshot.usb_devices),
                    build_pci_view(&snapshot.pci_devices),
                    build_bios_view(&snapshot.bios),
                ]) {
                    host.append(&view);
                }
            }
            HardwareState::Loading | HardwareState::Failed(_) => {
                let message = match hardware {
                    HardwareState::Failed(message) => {
                        format!("Hardware information unavailable: {message}")
                    }
                    _ => "Loading hardware information…".into(),
                };
                for host in &self.hardware_hosts {
                    let label = Label::new(Some(&message));
                    label.set_wrap(true);
                    host.append(&label);
                }
            }
        }
    }

    /// Update the SMART storage health section.
    fn update_smart(&self, smart: &SmartHealthState) {
        self.smart_refresh_button.set_sensitive(!smart.refreshing);

        if smart.refreshing {
            self.smart_refresh_button
                .set_label("Refreshing SMART Data…");
            self.smart_status_label.set_text("Querying SMART data…");
            self.smart_status_label.set_visible(true);
            self.smart_status_label.remove_css_class("error");
        } else {
            self.smart_refresh_button.set_label("Refresh SMART Data");
            if smart.permission_required {
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

fn build_gpu_view(gpu: &maintainer_core::hardware::GpuInfo) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);

    let container = GtkBox::new(Orientation::Vertical, 16);
    container.set_margin_top(16);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let gpu_title = Label::new(Some("Default GPU"));
    gpu_title.add_css_class("title-2");
    gpu_title.set_halign(gtk4::Align::Start);
    container.append(&gpu_title);

    let gpu_card = GtkBox::new(Orientation::Vertical, 0);
    gpu_card.add_css_class("maintainer_surface");
    gpu_card.add_css_class("maintainer_spec_card");

    gpu_card.append(&build_hardware_row("Brand", &gpu.brand));
    gpu_card.append(&build_hardware_row("Name", &gpu.name));
    gpu_card.append(&build_hardware_row("PCI ID", &gpu.pci_id));
    gpu_card.append(&build_hardware_row("Driver", &gpu.driver));
    container.append(&gpu_card);

    let accel_title = Label::new(Some("Hardware Acceleration"));
    accel_title.add_css_class("title-2");
    accel_title.set_halign(gtk4::Align::Start);
    accel_title.set_margin_top(12);
    container.append(&accel_title);

    let accel_card = GtkBox::new(Orientation::Vertical, 0);
    accel_card.add_css_class("maintainer_surface");
    accel_card.add_css_class("maintainer_spec_card");

    accel_card.append(&build_hardware_row("OpenGL", &gpu.opengl));
    accel_card.append(&build_hardware_row("OpenGL ES", &gpu.opengl_es));
    accel_card.append(&build_hardware_row("Vulkan", &gpu.vulkan));
    accel_card.append(&build_hardware_row("Video Playback", &gpu.video_playback));
    container.append(&accel_card);

    scrolled.set_child(Some(&container));
    scrolled
}

fn build_usb_view(devices: &[maintainer_core::hardware::UsbDevice]) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);

    let container = GtkBox::new(Orientation::Vertical, 16);
    container.set_margin_top(16);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let header = GtkBox::new(Orientation::Horizontal, 12);
    header.add_css_class("maintainer_process_header");
    let name_col = Label::new(Some("Name"));
    name_col.set_hexpand(true);
    name_col.set_halign(gtk4::Align::Start);
    let id_col = Label::new(Some("ID"));
    id_col.set_width_chars(12);
    id_col.set_halign(gtk4::Align::Start);
    let speed_col = Label::new(Some("Speed"));
    speed_col.set_width_chars(12);
    speed_col.set_halign(gtk4::Align::Start);
    let power_col = Label::new(Some("Power"));
    power_col.set_width_chars(12);
    power_col.set_halign(gtk4::Align::End);

    header.append(&name_col);
    header.append(&id_col);
    header.append(&speed_col);
    header.append(&power_col);
    container.append(&header);

    let list = ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);

    if devices.is_empty() {
        container.append(&Label::new(Some(
            "No devices reported by the available collectors.",
        )));
    }

    for dev in devices {
        let row = GtkBox::new(Orientation::Horizontal, 12);
        row.set_margin_top(8);
        row.set_margin_bottom(8);
        row.set_margin_start(12);
        row.set_margin_end(12);

        let icon = gtk4::Image::from_icon_name(if dev.is_hub {
            "network-server-symbolic"
        } else if dev.name.to_lowercase().contains("camera") {
            "camera-web-symbolic"
        } else if dev.name.to_lowercase().contains("bluetooth") {
            "bluetooth-active-symbolic"
        } else {
            "media-removable-symbolic"
        });
        icon.set_pixel_size(16);
        icon.add_css_class("maintainer_text_muted");
        row.append(&icon);

        let name_lbl = Label::new(Some(&dev.name));
        name_lbl.set_hexpand(true);
        name_lbl.set_halign(gtk4::Align::Start);
        name_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        row.append(&name_lbl);

        let id_btn = Button::with_label(&dev.id);
        id_btn.add_css_class("flat");
        id_btn.add_css_class("maintainer_process_pid");
        id_btn.set_tooltip_text(Some("Click to copy ID"));
        let id_val = dev.id.clone();
        id_btn.connect_clicked(move |btn| {
            if let Some(display) = gtk4::gdk::Display::default() {
                display.clipboard().set_text(&id_val);
                btn.set_label("Copied!");
                let btn_done = btn.clone();
                let original_id = id_val.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    btn_done.set_label(&original_id);
                });
            }
        });
        row.append(&id_btn);

        let speed_lbl = Label::new(Some(&dev.speed));
        speed_lbl.set_width_chars(12);
        speed_lbl.set_halign(gtk4::Align::Start);
        speed_lbl.add_css_class("dim-label");
        row.append(&speed_lbl);

        let power_lbl = Label::new(Some(&dev.power));
        power_lbl.set_width_chars(12);
        power_lbl.set_halign(gtk4::Align::End);
        power_lbl.add_css_class("dim-label");
        row.append(&power_lbl);

        list.append(&row);
    }
    container.append(&list);

    let note = Label::new(Some(
        "Note: Click a device's ID button to copy it to clipboard.",
    ));
    note.add_css_class("dim-label");
    note.set_halign(gtk4::Align::Center);
    note.set_margin_top(12);
    container.append(&note);

    scrolled.set_child(Some(&container));
    scrolled
}

fn build_pci_view(devices: &[maintainer_core::hardware::PciDevice]) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);

    let container = GtkBox::new(Orientation::Vertical, 16);
    container.set_margin_top(16);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let header = GtkBox::new(Orientation::Horizontal, 12);
    header.add_css_class("maintainer_process_header");
    let bus_col = Label::new(Some("Bus"));
    bus_col.set_width_chars(14);
    bus_col.set_halign(gtk4::Align::Start);
    let type_col = Label::new(Some("Type"));
    type_col.set_width_chars(16);
    type_col.set_halign(gtk4::Align::Start);
    let brand_col = Label::new(Some("Brand"));
    brand_col.set_width_chars(12);
    brand_col.set_halign(gtk4::Align::Start);
    let dev_col = Label::new(Some("Device"));
    dev_col.set_hexpand(true);
    dev_col.set_halign(gtk4::Align::Start);
    let id_col = Label::new(Some("ID"));
    id_col.set_width_chars(12);
    id_col.set_halign(gtk4::Align::Start);
    let driver_col = Label::new(Some("Driver"));
    driver_col.set_width_chars(14);
    driver_col.set_halign(gtk4::Align::Start);

    header.append(&bus_col);
    header.append(&type_col);
    header.append(&brand_col);
    header.append(&dev_col);
    header.append(&id_col);
    header.append(&driver_col);
    container.append(&header);

    let list = ListBox::new();
    list.add_css_class("boxed-list");
    list.set_selection_mode(gtk4::SelectionMode::None);

    if devices.is_empty() {
        container.append(&Label::new(Some(
            "No devices reported by the available collectors.",
        )));
    }

    for dev in devices {
        let row = GtkBox::new(Orientation::Horizontal, 12);
        row.set_margin_top(6);
        row.set_margin_bottom(6);
        row.set_margin_start(12);
        row.set_margin_end(12);

        let bus_lbl = Label::new(Some(&dev.slot));
        bus_lbl.set_width_chars(14);
        bus_lbl.set_halign(gtk4::Align::Start);
        bus_lbl.add_css_class("maintainer_process_pid");
        row.append(&bus_lbl);

        let type_lbl = Label::new(Some(&dev.dev_type));
        type_lbl.set_width_chars(16);
        type_lbl.set_halign(gtk4::Align::Start);
        type_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&type_lbl);

        let brand_lbl = Label::new(Some(&dev.brand));
        brand_lbl.set_width_chars(12);
        brand_lbl.set_halign(gtk4::Align::Start);
        row.append(&brand_lbl);

        let dev_lbl = Label::new(Some(&dev.device));
        dev_lbl.set_hexpand(true);
        dev_lbl.set_halign(gtk4::Align::Start);
        dev_lbl.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
        row.append(&dev_lbl);

        let id_btn = Button::with_label(&dev.id);
        id_btn.add_css_class("flat");
        id_btn.add_css_class("maintainer_process_pid");
        id_btn.set_tooltip_text(Some("Click to copy ID"));
        let id_val = dev.id.clone();
        id_btn.connect_clicked(move |btn| {
            if let Some(display) = gtk4::gdk::Display::default() {
                display.clipboard().set_text(&id_val);
                btn.set_label("Copied!");
                let btn_done = btn.clone();
                let original_id = id_val.clone();
                glib::timeout_add_local_once(std::time::Duration::from_millis(1500), move || {
                    btn_done.set_label(&original_id);
                });
            }
        });
        row.append(&id_btn);

        let driver_lbl = Label::new(Some(&dev.driver));
        driver_lbl.set_width_chars(14);
        driver_lbl.set_halign(gtk4::Align::Start);
        driver_lbl.add_css_class("dim-label");
        row.append(&driver_lbl);

        list.append(&row);
    }
    container.append(&list);

    scrolled.set_child(Some(&container));
    scrolled
}

fn build_bios_view(bios: &maintainer_core::hardware::BiosInfo) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_vexpand(true);
    scrolled.set_hexpand(true);

    let container = GtkBox::new(Orientation::Vertical, 16);
    container.set_margin_top(16);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let title = Label::new(Some("BIOS & Motherboard"));
    title.add_css_class("title-2");
    title.set_halign(gtk4::Align::Start);
    container.append(&title);

    let card = GtkBox::new(Orientation::Vertical, 0);
    card.add_css_class("maintainer_surface");
    card.add_css_class("maintainer_spec_card");

    card.append(&build_hardware_row("BIOS Vendor", &bios.vendor));
    card.append(&build_hardware_row("BIOS Version", &bios.version));
    card.append(&build_hardware_row("Release Date", &bios.release_date));
    card.append(&build_hardware_row("Motherboard", &bios.motherboard));
    container.append(&card);

    scrolled.set_child(Some(&container));
    scrolled
}

fn build_hardware_row(label_text: &str, value_text: &str) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    row.set_margin_top(10);
    row.set_margin_bottom(10);
    row.set_margin_start(16);
    row.set_margin_end(16);

    let label = Label::new(Some(label_text));
    label.add_css_class("maintainer_spec_label");
    label.set_halign(gtk4::Align::Start);
    label.set_hexpand(true);
    row.append(&label);

    let value = Label::new(Some(value_text));
    value.add_css_class("maintainer_spec_value");
    value.set_halign(gtk4::Align::End);
    row.append(&value);

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
    fn hardware_collection_state_is_pure_and_records_failure() {
        let mut state = DiagnosticsState::default();
        assert_eq!(state.hardware, HardwareState::Loading);
        state.reduce(DiagnosticsIntent::HardwareUpdated(HardwareState::Failed(
            "queue unavailable".into(),
        )));
        assert_eq!(
            state.hardware,
            HardwareState::Failed("queue unavailable".into())
        );
    }

    #[test]
    #[ignore = "requires a GTK display; run this exact test under Xvfb"]
    fn hardware_views_show_collection_issues_without_rebuilding_unchanged_data() {
        gtk4::init().unwrap();
        let page = DiagnosticsPage::new();
        for host in &page.hardware_hosts {
            let loading = host.first_child().and_downcast::<Label>().unwrap();
            assert!(loading.text().contains("Loading hardware"));
        }
        let snapshot = maintainer_core::hardware::HardwareSnapshot {
            collection_issues: vec!["PCI inventory unavailable: permission denied".into()],
            ..Default::default()
        };
        let hardware = HardwareState::Loaded(Rc::new(snapshot));
        page.update_hardware(&hardware);
        for host in &page.hardware_hosts {
            let issue = host.first_child().and_downcast::<Label>().unwrap();
            assert_eq!(issue.text(), "PCI inventory unavailable: permission denied");
            assert!(issue.next_sibling().is_some());
        }
        let original_view = page.hardware_hosts[0].last_child().unwrap();
        page.update_hardware(&hardware);
        assert_eq!(page.hardware_hosts[0].last_child(), Some(original_view));
        page.update_hardware(&HardwareState::Failed("worker unavailable".into()));
        for host in &page.hardware_hosts {
            let failure = host.first_child().and_downcast::<Label>().unwrap();
            assert!(
                failure
                    .text()
                    .contains("Hardware information unavailable: worker unavailable")
            );
            assert!(failure.next_sibling().is_none());
        }
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
