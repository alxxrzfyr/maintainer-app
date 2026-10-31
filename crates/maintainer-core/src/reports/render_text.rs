//! Deterministic plain-text rendering of a redacted report.
//!
//! Section order is fixed: OS, hardware, health, software updates,
//! diagnostics, maintenance, capability notes, redaction notice.

use super::model::{RedactedReport, RedactionPolicy};
use super::redact::scrub_residuals;

const MAX_RENDERED_REPORT_BYTES: usize = 4 * 1024 * 1024;
const REPORT_TRUNCATION_NOTICE: &str =
    "\n[Report truncated at 4 MiB; additional content omitted.]\n";

/// Render a redacted report to plain text with stable section order.
pub fn render_text(report: &RedactedReport) -> String {
    let mut out = String::with_capacity(4096);

    write_header(&mut out, &report.generated_at);
    write_os_section(&mut out, report);
    write_hardware_section(&mut out, report);
    write_health_section(&mut out, report);
    write_software_updates_section(&mut out, report);
    write_diagnostics_section(&mut out, report);
    write_maintenance_section(&mut out, report);
    write_capability_notes(&mut out, report);
    write_redaction_notice(&mut out, &report.policy);

    cap_rendered_report(scrub_residuals(&out, &report.policy))
}

fn cap_rendered_report(mut text: String) -> String {
    if text.len() <= MAX_RENDERED_REPORT_BYTES {
        return text;
    }

    let mut end = MAX_RENDERED_REPORT_BYTES - REPORT_TRUNCATION_NOTICE.len();
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(REPORT_TRUNCATION_NOTICE);
    text
}

fn write_header(out: &mut String, generated_at: &str) {
    out.push_str("=== Maintainer System Report ===\n");
    out.push_str(&format!("Generated: {generated_at}\n"));
    out.push('\n');
}

fn write_os_section(out: &mut String, report: &RedactedReport) {
    out.push_str("--- Operating System ---\n");
    out.push_str(&format!("Distribution: {}\n", report.os.distro));
    out.push_str(&format!("Kernel: {}\n", report.os.kernel));
    out.push_str(&format!(
        "Uptime: {}\n",
        format_uptime(report.os.uptime_seconds)
    ));
    out.push_str(&format!("Hostname: {}\n", report.os.hostname));
    out.push_str(&format!("User: {}\n", report.os.username));
    if let Some(ref desktop) = report.os.desktop {
        out.push_str(&format!("Desktop: {desktop}\n"));
    }
    out.push('\n');
}

fn write_hardware_section(out: &mut String, report: &RedactedReport) {
    out.push_str("--- Hardware ---\n");

    if let Some(ref cpu) = report.hardware.cpu_summary {
        out.push_str(&format!("{cpu}\n"));
    }
    if let Some(ref mem) = report.hardware.memory_summary {
        out.push_str(&format!("{mem}\n"));
    }

    if !report.hardware.disk_summaries.is_empty() {
        out.push_str("Disks:\n");
        for disk in &report.hardware.disk_summaries {
            out.push_str(&format!("  {disk}\n"));
        }
    }

    if !report.hardware.network_summaries.is_empty() {
        out.push_str("Network:\n");
        for net in &report.hardware.network_summaries {
            out.push_str(&format!("  {net}\n"));
        }
    }

    if !report.hardware.temperature_summaries.is_empty() {
        out.push_str("Temperatures:\n");
        for temp in &report.hardware.temperature_summaries {
            out.push_str(&format!("  {temp}\n"));
        }
    }

    for note in &report.hardware.unavailable_notes {
        out.push_str(&format!("  [unavailable] {note}\n"));
    }

    out.push('\n');
}

fn write_health_section(out: &mut String, report: &RedactedReport) {
    out.push_str("--- Storage Health ---\n");

    if let Some(ref note) = report.health.unavailable_note {
        out.push_str(&format!("  [unavailable] {note}\n"));
    } else if report.health.drive_summaries.is_empty() {
        out.push_str("  No drives detected.\n");
    } else {
        for drive in &report.health.drive_summaries {
            let temp_str = drive
                .temperature_celsius
                .map(|t| format!(", {t:.0}°C"))
                .unwrap_or_default();
            let hours_str = drive
                .power_on_hours
                .map(|h| format!(", {h} hours"))
                .unwrap_or_default();
            out.push_str(&format!(
                "  {} — {}{}{}\n",
                drive.model, drive.assessment, temp_str, hours_str
            ));
        }
    }

    out.push('\n');
}

fn write_software_updates_section(out: &mut String, report: &RedactedReport) {
    out.push_str("--- Software Updates ---\n");

    match &report.software_updates.pending_updates {
        crate::capability::Capability::Available(updates) => {
            if updates.is_empty() {
                out.push_str("  System is up to date.\n");
            } else {
                out.push_str(&format!("  {} update(s) available:\n", updates.len()));
                for u in updates {
                    out.push_str(&format!(
                        "    {} ({} -> {})\n",
                        u.package_name, u.current_version, u.available_version
                    ));
                }
            }
        }
        crate::capability::Capability::Unavailable(r) => {
            out.push_str(&format!("  [unavailable] {}\n", r.message));
        }
        crate::capability::Capability::PermissionRequired(p) => {
            out.push_str(&format!("  [permission required] {}\n", p.explanation));
        }
    }

    out.push('\n');
}

fn write_diagnostics_section(out: &mut String, report: &RedactedReport) {
    out.push_str("--- Diagnostics ---\n");

    if !report.diagnostics.failed_services.is_empty() {
        out.push_str("  Failed services:\n");
        for svc in &report.diagnostics.failed_services {
            out.push_str(&format!(
                "    {} ({}) — {}\n",
                svc.unit, svc.state, svc.description
            ));
        }
    }

    if !report.diagnostics.event_groups.is_empty() {
        out.push_str("  Recurring events:\n");
        for evt in &report.diagnostics.event_groups {
            let unit_str = evt
                .unit
                .as_deref()
                .map(|u| format!(" [{u}]"))
                .unwrap_or_default();
            out.push_str(&format!(
                "    {}x{} — {}\n",
                evt.count, unit_str, evt.template
            ));
        }
    }

    for note in &report.diagnostics.unavailable_notes {
        out.push_str(&format!("  [unavailable] {note}\n"));
    }

    if report.diagnostics.failed_services.is_empty()
        && report.diagnostics.event_groups.is_empty()
        && report.diagnostics.unavailable_notes.is_empty()
    {
        out.push_str("  No issues detected.\n");
    }

    out.push('\n');
}

fn write_maintenance_section(out: &mut String, report: &RedactedReport) {
    out.push_str("--- Maintenance History ---\n");

    if report.maintenance.recent_history.is_empty() {
        out.push_str("  No recent maintenance recorded.\n");
    } else {
        for entry in &report.maintenance.recent_history {
            let status_str = entry
                .completed_at
                .map(|_| "completed")
                .unwrap_or("in progress");
            out.push_str(&format!(
                "  {} — {} ({})\n",
                entry.operation, status_str, entry.id
            ));
        }
    }

    out.push('\n');
}

fn write_capability_notes(out: &mut String, report: &RedactedReport) {
    if report.capability_notes.is_empty() {
        return;
    }

    out.push_str("--- Capability Notes ---\n");
    for note in &report.capability_notes {
        let reason_str = note
            .reason
            .as_deref()
            .map(|r| format!(": {r}"))
            .unwrap_or_default();
        out.push_str(&format!(
            "  {} — {}{}\n",
            note.name, note.status, reason_str
        ));
    }
    out.push('\n');
}

fn write_redaction_notice(out: &mut String, policy: &RedactionPolicy) {
    out.push_str("--- Redaction Notice ---\n");
    if policy.categories.is_empty() {
        out.push_str("  No redaction applied.\n");
    } else {
        out.push_str("  The following categories were redacted from this report:\n");
        for cat in &policy.categories {
            let label = match cat {
                super::model::RedactionCategory::Username => "Usernames",
                super::model::RedactionCategory::Hostname => "Hostnames",
                super::model::RedactionCategory::Ipv4 => "IPv4 addresses",
                super::model::RedactionCategory::Ipv6 => "IPv6 addresses",
                super::model::RedactionCategory::Mac => "MAC addresses",
                super::model::RedactionCategory::Serial => "Serial numbers",
                super::model::RedactionCategory::HomePath => "Home directory paths",
                super::model::RedactionCategory::MountedPath => "Mounted paths",
                super::model::RedactionCategory::ProcessArgs => "Process arguments",
                super::model::RedactionCategory::Email => "Email addresses",
            };
            out.push_str(&format!("  - {label}\n"));
        }
    }
}

/// Format uptime seconds into human-readable "Xd Yh Zm".
fn format_uptime(seconds: u64) -> String {
    let days = seconds / 86400;
    let hours = (seconds % 86400) / 3600;
    let minutes = (seconds % 3600) / 60;

    if days > 0 {
        format!("{days}d {hours}h {minutes}m")
    } else if hours > 0 {
        format!("{hours}h {minutes}m")
    } else {
        format!("{minutes}m")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::capability::{Capability, UnavailableReason};
    use crate::reports::model::*;

    fn minimal_report() -> RedactedReport {
        RedactedReport {
            generated_at: "2026-01-15 10:30:00 UTC".into(),
            os: RedactedOsSection {
                distro: "Ubuntu 24.04.1 LTS".into(),
                kernel: "6.8.0-45-generic".into(),
                uptime_seconds: 90061,
                desktop: Some("GNOME 46".into()),
                hostname: "[hostname]".into(),
                username: "[username]".into(),
            },
            hardware: RedactedHardwareSection {
                cpu_summary: Some("CPU: 12% usage across 8 cores".into()),
                memory_summary: Some("Memory: 4096 MB used / 16384 MB total".into()),
                disk_summaries: vec!["[path]: 100000 MB used / 500000 MB total".into()],
                network_summaries: vec!["eth0: 1048576 bytes rx / 524288 bytes tx".into()],
                temperature_summaries: vec!["CPU Package: 45.0°C".into()],
                unavailable_notes: Vec::new(),
            },
            health: RedactedHealthSection {
                drive_summaries: vec![RedactedDriveSummary {
                    model: "Samsung SSD 970 EVO".into(),
                    assessment: "Passed".into(),
                    temperature_celsius: Some(35.0),
                    power_on_hours: Some(12345),
                }],
                unavailable_note: None,
            },
            software_updates: SoftwareUpdatesSection {
                pending_updates: Capability::Available(vec![UpdateInfo {
                    package_name: "linux-image".into(),
                    current_version: "6.8.0-44".into(),
                    available_version: "6.8.0-45".into(),
                }]),
            },
            diagnostics: RedactedDiagnosticsSection {
                failed_services: vec![RedactedServiceSummary {
                    unit: "nginx.service".into(),
                    description: "A high performance web server".into(),
                    state: "failed/failed".into(),
                }],
                event_groups: vec![RedactedEventSummary {
                    template: "Failed to start".into(),
                    count: 5,
                    unit: Some("nginx.service".into()),
                    priority: 3,
                }],
                unavailable_notes: Vec::new(),
            },
            maintenance: MaintenanceSection {
                recent_history: Vec::new(),
            },
            capability_notes: vec![CapabilityNote {
                name: "Flatpak".into(),
                status: "unavailable".into(),
                reason: Some("flatpak not installed".into()),
            }],
            policy: RedactionPolicy::default(),
        }
    }

    #[test]
    fn rendered_report_is_capped_with_utf8_safe_notice() {
        let long_report = format!("{}🦀tail", "x".repeat(MAX_RENDERED_REPORT_BYTES));
        let capped = cap_rendered_report(long_report);

        assert!(capped.len() <= MAX_RENDERED_REPORT_BYTES);
        assert!(capped.is_char_boundary(capped.len()));
        assert!(capped.ends_with(REPORT_TRUNCATION_NOTICE));
    }

    #[test]
    fn rendered_report_below_cap_is_unchanged() {
        let report = "short report".to_string();
        assert_eq!(cap_rendered_report(report.clone()), report);
    }

    #[test]
    fn golden_report_structure() {
        let report = minimal_report();
        let text = render_text(&report);

        let os_pos = text.find("--- Operating System ---").unwrap();
        let hw_pos = text.find("--- Hardware ---").unwrap();
        let health_pos = text.find("--- Storage Health ---").unwrap();
        let updates_pos = text.find("--- Software Updates ---").unwrap();
        let diag_pos = text.find("--- Diagnostics ---").unwrap();
        let maint_pos = text.find("--- Maintenance History ---").unwrap();
        let cap_pos = text.find("--- Capability Notes ---").unwrap();
        let redact_pos = text.find("--- Redaction Notice ---").unwrap();

        assert!(os_pos < hw_pos);
        assert!(hw_pos < health_pos);
        assert!(health_pos < updates_pos);
        assert!(updates_pos < diag_pos);
        assert!(diag_pos < maint_pos);
        assert!(maint_pos < cap_pos);
        assert!(cap_pos < redact_pos);
    }

    #[test]
    fn golden_report_header() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.starts_with("=== Maintainer System Report ===\n"));
        assert!(text.contains("Generated: 2026-01-15 10:30:00 UTC\n"));
    }

    #[test]
    fn golden_report_os_section() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.contains("Distribution: Ubuntu 24.04.1 LTS\n"));
        assert!(text.contains("Kernel: 6.8.0-45-generic\n"));
        assert!(text.contains("Uptime: 1d 1h 1m\n"));
        assert!(text.contains("Hostname: [hostname]\n"));
        assert!(text.contains("User: [username]\n"));
        assert!(text.contains("Desktop: GNOME 46\n"));
    }

    #[test]
    fn golden_report_hardware_section() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.contains("CPU: 12% usage across 8 cores\n"));
        assert!(text.contains("Memory: 4096 MB used / 16384 MB total\n"));
        assert!(text.contains("  [path]: 100000 MB used / 500000 MB total\n"));
        assert!(text.contains("  eth0: 1048576 bytes rx / 524288 bytes tx\n"));
        assert!(
            text.contains("  CPU Package: 45.0\u{b0}C\n"),
            "temperature not found in: {text}"
        );
    }

    #[test]
    fn golden_report_health_section() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(
            text.contains("Samsung SSD 970 EVO \u{2014} Passed, 35\u{b0}C, 12345 hours\n"),
            "health section not found in: {text}"
        );
    }

    #[test]
    fn golden_report_updates_section() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.contains("1 update(s) available:\n"));
        assert!(text.contains("linux-image (6.8.0-44 -> 6.8.0-45)\n"));
    }

    #[test]
    fn golden_report_diagnostics_section() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.contains("Failed services:\n"));
        assert!(
            text.contains("nginx.service (failed/failed) \u{2014} A high performance web server"),
            "failed service not found in: {text}"
        );
        assert!(text.contains("Recurring events:\n"));
        assert!(
            text.contains("5x [nginx.service] \u{2014} Failed to start\n"),
            "event group not found in: {text}"
        );
    }

    #[test]
    fn golden_report_maintenance_empty() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.contains("No recent maintenance recorded.\n"));
    }

    #[test]
    fn golden_report_capability_notes() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(
            text.contains("Flatpak \u{2014} unavailable: flatpak not installed\n"),
            "capability note not found in: {text}"
        );
    }

    #[test]
    fn golden_report_redaction_notice() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(text.contains("The following categories were redacted"));
        assert!(text.contains("- Usernames\n"));
        assert!(text.contains("- Hostnames\n"));
        assert!(text.contains("- IPv4 addresses\n"));
        assert!(text.contains("- IPv6 addresses\n"));
        assert!(text.contains("- MAC addresses\n"));
        assert!(text.contains("- Serial numbers\n"));
        assert!(text.contains("- Home directory paths\n"));
        assert!(text.contains("- Mounted paths\n"));
        assert!(text.contains("- Process arguments\n"));
        assert!(text.contains("- Email addresses\n"));
    }

    #[test]
    fn render_unavailable_sections() {
        let report = RedactedReport {
            generated_at: "2026-01-15 10:30:00 UTC".into(),
            os: RedactedOsSection {
                distro: "Ubuntu 22.04".into(),
                kernel: "5.15.0".into(),
                uptime_seconds: 60,
                desktop: None,
                hostname: "[hostname]".into(),
                username: "[username]".into(),
            },
            hardware: RedactedHardwareSection {
                cpu_summary: None,
                memory_summary: None,
                disk_summaries: Vec::new(),
                network_summaries: Vec::new(),
                temperature_summaries: Vec::new(),
                unavailable_notes: vec!["CPU: no /proc/stat available".into()],
            },
            health: RedactedHealthSection {
                drive_summaries: Vec::new(),
                unavailable_note: Some("SMART not supported".into()),
            },
            software_updates: SoftwareUpdatesSection {
                pending_updates: Capability::Unavailable(UnavailableReason {
                    code: "apt-unavailable",
                    message: "APT not found".into(),
                    remediation: None,
                }),
            },
            diagnostics: RedactedDiagnosticsSection {
                failed_services: Vec::new(),
                event_groups: Vec::new(),
                unavailable_notes: vec!["Failed services: systemd not running".into()],
            },
            maintenance: MaintenanceSection {
                recent_history: Vec::new(),
            },
            capability_notes: Vec::new(),
            policy: RedactionPolicy::default(),
        };

        let text = render_text(&report);
        assert!(text.contains("[unavailable] CPU: no /proc/stat available"));
        assert!(text.contains("[unavailable] SMART not supported"));
        assert!(text.contains("[unavailable] APT not found"));
        assert!(text.contains("[unavailable] Failed services: systemd not running"));
    }

    #[test]
    fn format_uptime_days() {
        assert_eq!(format_uptime(90061), "1d 1h 1m");
    }

    #[test]
    fn format_uptime_hours_only() {
        assert_eq!(format_uptime(7260), "2h 1m");
    }

    #[test]
    fn format_uptime_minutes_only() {
        assert_eq!(format_uptime(300), "5m");
    }

    #[test]
    fn format_uptime_zero() {
        assert_eq!(format_uptime(0), "0m");
    }

    #[test]
    fn no_redaction_notice_says_none_applied() {
        let mut report = minimal_report();
        report.policy = RedactionPolicy {
            categories: Vec::new(),
        };
        let text = render_text(&report);
        assert!(text.contains("No redaction applied.\n"));
    }

    #[test]
    fn report_uses_unix_line_endings() {
        let report = minimal_report();
        let text = render_text(&report);
        assert!(!text.contains("\r\n"), "report must use unix line endings");
    }

    #[test]
    fn residual_ip_in_free_text_gets_caught() {
        let mut report = minimal_report();
        report.diagnostics.failed_services[0].description = "bind to 10.0.0.5 failed".into();
        let text = render_text(&report);
        assert!(!text.contains("10.0.0.5"), "residual IP not caught: {text}");
    }
}
