//! Structured redaction: sensitive fields are typed and removed/replaced
//! before rendering. A final conservative regex scan catches residuals.

use crate::capability::Capability;

use super::model::{
    DiagnosticsSection, HardwareSection, HealthSection, OsSection, RedactedDiagnosticsSection,
    RedactedDriveSummary, RedactedEventSummary, RedactedHardwareSection, RedactedHealthSection,
    RedactedOsSection, RedactedReport, RedactedServiceSummary, RedactionCategory, RedactionPolicy,
    SystemReport,
};

/// Placeholder for redacted usernames.
const REDACTED_USERNAME: &str = "[username]";
/// Placeholder for redacted hostnames.
const REDACTED_HOSTNAME: &str = "[hostname]";
/// Placeholder for redacted IP addresses.
const REDACTED_IP: &str = "[ip-address]";
/// Placeholder for redacted MAC addresses.
const REDACTED_MAC: &str = "[mac-address]";
/// Placeholder for redacted paths.
const REDACTED_PATH: &str = "[path]";
/// Placeholder for redacted email.
const REDACTED_EMAIL: &str = "[email]";
/// Placeholder for redacted arguments.
const REDACTED_ARGS: &str = "[args]";

/// Apply the redaction policy to a full system report, producing a
/// `RedactedReport` where sensitive fields have been typed-removed.
pub fn redact_report(report: SystemReport, policy: &RedactionPolicy) -> RedactedReport {
    let os = redact_os(&report.os, policy);
    let hardware = redact_hardware(&report.hardware, policy);
    let health = redact_health(&report.health, policy);
    let diagnostics = redact_diagnostics(&report.diagnostics, policy);

    RedactedReport {
        generated_at: report.generated_at,
        os,
        hardware,
        health,
        software_updates: report.software_updates,
        diagnostics,
        maintenance: report.maintenance,
        capability_notes: report.capability_notes,
        policy: policy.clone(),
    }
}

fn redact_os(os: &OsSection, policy: &RedactionPolicy) -> RedactedOsSection {
    RedactedOsSection {
        distro: os.distro.clone(),
        kernel: os.kernel.clone(),
        uptime_seconds: os.uptime_seconds,
        desktop: os.desktop.clone(),
        hostname: if policy.should_redact(RedactionCategory::Hostname) {
            REDACTED_HOSTNAME.to_string()
        } else {
            os.hostname.clone()
        },
        username: if policy.should_redact(RedactionCategory::Username) {
            REDACTED_USERNAME.to_string()
        } else {
            os.username.clone()
        },
    }
}

fn redact_hardware(hw: &HardwareSection, policy: &RedactionPolicy) -> RedactedHardwareSection {
    let mut unavailable_notes = Vec::new();

    let cpu_summary = match &hw.cpu {
        Capability::Available(cpu) => Some(format!(
            "CPU: {:.0}% usage across {} cores",
            cpu.total_percent,
            cpu.per_core_percent.len()
        )),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("CPU: {}", r.message));
            None
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("CPU: {}", p.explanation));
            None
        }
    };

    let memory_summary = match &hw.memory {
        Capability::Available(mem) => Some(format!(
            "Memory: {} MB used / {} MB total",
            mem.used_bytes / 1_048_576,
            mem.total_bytes / 1_048_576
        )),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("Memory: {}", r.message));
            None
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("Memory: {}", p.explanation));
            None
        }
    };

    let disk_summaries = match &hw.disks {
        Capability::Available(disks) => disks
            .iter()
            .map(|d| {
                let mount = if policy.should_redact(RedactionCategory::MountedPath) {
                    REDACTED_PATH.to_string()
                } else {
                    d.mount_point.clone()
                };
                format!(
                    "{}: {} MB used / {} MB total",
                    mount,
                    d.used_bytes / 1_048_576,
                    d.total_bytes / 1_048_576
                )
            })
            .collect(),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("Disks: {}", r.message));
            Vec::new()
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("Disks: {}", p.explanation));
            Vec::new()
        }
    };

    let network_summaries = match &hw.network {
        Capability::Available(nets) => nets
            .iter()
            .map(|n| {
                format!(
                    "{}: {} bytes rx / {} bytes tx",
                    n.interface, n.rx_bytes, n.tx_bytes
                )
            })
            .collect(),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("Network: {}", r.message));
            Vec::new()
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("Network: {}", p.explanation));
            Vec::new()
        }
    };

    let temperature_summaries = match &hw.temperatures {
        Capability::Available(temps) => temps
            .iter()
            .filter_map(|t| {
                let label = t.label.as_deref().unwrap_or(&t.hwmon_name);
                match &t.celsius {
                    Capability::Available(c) => Some(format!("{}: {:.1}°C", label, c)),
                    _ => None,
                }
            })
            .collect(),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("Temperatures: {}", r.message));
            Vec::new()
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("Temperatures: {}", p.explanation));
            Vec::new()
        }
    };

    RedactedHardwareSection {
        cpu_summary,
        memory_summary,
        disk_summaries,
        network_summaries,
        temperature_summaries,
        unavailable_notes,
    }
}

fn redact_health(health: &HealthSection, policy: &RedactionPolicy) -> RedactedHealthSection {
    match &health.smart_drives {
        Capability::Available(drives) => {
            let drive_summaries = drives
                .iter()
                .map(|d| {
                    // Serial is always sensitive — redact structurally.
                    let _ = policy.should_redact(RedactionCategory::Serial);
                    RedactedDriveSummary {
                        model: d.model.clone(),
                        assessment: d.assessment.label().to_string(),
                        temperature_celsius: d.temperature_celsius.map(|t| t.value()),
                        power_on_hours: d.power_on_hours,
                    }
                })
                .collect();
            RedactedHealthSection {
                drive_summaries,
                unavailable_note: None,
            }
        }
        Capability::Unavailable(r) => RedactedHealthSection {
            drive_summaries: Vec::new(),
            unavailable_note: Some(r.message.clone()),
        },
        Capability::PermissionRequired(p) => RedactedHealthSection {
            drive_summaries: Vec::new(),
            unavailable_note: Some(p.explanation.clone()),
        },
    }
}

fn redact_diagnostics(
    diag: &DiagnosticsSection,
    policy: &RedactionPolicy,
) -> RedactedDiagnosticsSection {
    let mut unavailable_notes = Vec::new();

    let failed_services = match &diag.failed_services {
        Capability::Available(services) => services
            .iter()
            .map(|s| RedactedServiceSummary {
                unit: s.unit.clone(),
                description: s.description.clone(),
                state: format!("{}/{}", s.active_state, s.sub_state),
            })
            .collect(),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("Failed services: {}", r.message));
            Vec::new()
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("Failed services: {}", p.explanation));
            Vec::new()
        }
    };

    let event_groups = match &diag.event_groups {
        Capability::Available(groups) => groups
            .iter()
            .map(|g| {
                let template = if policy.should_redact(RedactionCategory::ProcessArgs) {
                    scrub_process_args(&g.key.template)
                } else {
                    g.key.template.clone()
                };
                RedactedEventSummary {
                    template,
                    count: g.count,
                    unit: g.key.unit.clone(),
                    priority: g.key.priority,
                }
            })
            .collect(),
        Capability::Unavailable(r) => {
            unavailable_notes.push(format!("Journal events: {}", r.message));
            Vec::new()
        }
        Capability::PermissionRequired(p) => {
            unavailable_notes.push(format!("Journal events: {}", p.explanation));
            Vec::new()
        }
    };

    RedactedDiagnosticsSection {
        failed_services,
        event_groups,
        unavailable_notes,
    }
}

/// Remove likely process arguments from a log template.
/// Replaces content after common command patterns with [args].
fn scrub_process_args(template: &str) -> String {
    // Simple heuristic: if line contains an absolute path followed by arguments,
    // keep the path but redact arguments.
    if let Some(idx) = template.find(" -- ") {
        return format!("{} -- {REDACTED_ARGS}", &template[..idx]);
    }
    template.to_string()
}

/// Final residual scan: catches any sensitive patterns that might have
/// leaked through structural redaction (e.g., from free-text fields).
pub fn scrub_residuals(text: &str, policy: &RedactionPolicy) -> String {
    let mut result = text.to_string();

    if policy.should_redact(RedactionCategory::Ipv4) {
        result = scrub_ipv4(&result);
    }
    if policy.should_redact(RedactionCategory::Ipv6) {
        result = scrub_ipv6(&result);
    }
    if policy.should_redact(RedactionCategory::Mac) {
        result = scrub_mac(&result);
    }
    if policy.should_redact(RedactionCategory::Email) {
        result = scrub_email(&result);
    }
    if policy.should_redact(RedactionCategory::HomePath) {
        result = scrub_home_paths(&result);
    }

    result
}

/// Replace IPv4 addresses (but not version numbers like 1.2.3).
fn scrub_ipv4(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut chars = text.char_indices().peekable();

    while let Some(&(i, c)) = chars.peek() {
        if c.is_ascii_digit() {
            if let Some((end, is_ip)) = try_parse_ipv4_str(text, i) {
                if is_ip {
                    result.push_str(REDACTED_IP);
                } else {
                    result.push_str(&text[i..end]);
                }
                // Advance past the matched sequence.
                while let Some(&(j, _)) = chars.peek() {
                    if j >= end {
                        break;
                    }
                    chars.next();
                }
                continue;
            }
        }
        result.push(c);
        chars.next();
    }

    result
}

/// Try to parse an IPv4-like pattern starting at byte index `start`.
/// Returns `(end_byte_index, is_real_ip)` or `None`.
fn try_parse_ipv4_str(text: &str, start: usize) -> Option<(usize, bool)> {
    let slice = &text[start..];
    let mut octets = 0;
    let mut i = 0;

    loop {
        // Parse one octet (1-3 ASCII digits).
        let octet_start = i;
        while i < slice.len() && slice.as_bytes()[i].is_ascii_digit() {
            i += 1;
        }
        let octet_len = i - octet_start;
        if octet_len == 0 || octet_len > 3 {
            return None;
        }
        octets += 1;

        if octets == 4 {
            break;
        }

        // Expect a dot.
        if i >= slice.len() || slice.as_bytes()[i] != b'.' {
            return None;
        }
        i += 1; // consume dot
    }

    // Validate octets are in 0-255 range.
    let candidate = &slice[..i];
    let parts: Vec<&str> = candidate.split('.').collect();
    if parts.len() != 4 {
        return None;
    }

    for part in &parts {
        if let Ok(n) = part.parse::<u16>() {
            if n > 255 {
                return None;
            }
        } else {
            return None;
        }
    }

    let end = start + i;

    // Check it's not preceded by a letter (version prefix like "v").
    if start > 0 {
        let prev_byte = text.as_bytes()[start - 1];
        if prev_byte.is_ascii_alphabetic() {
            return Some((end, false));
        }
    }

    // Check it's not followed by another dot (more than 4 octets = version).
    if end < text.len() && text.as_bytes()[end] == b'.' {
        return Some((end, false));
    }

    Some((end, true))
}

/// Replace IPv6 addresses.
fn scrub_ipv6(text: &str) -> String {
    // Matches common IPv6 patterns: groups of hex separated by colons.
    // Simplified: look for sequences with at least 2 colon-hex groups.
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;

    while i < chars.len() {
        if let Some(end) = try_match_ipv6(&chars, i) {
            result.push_str(REDACTED_IP);
            i = end;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Try to match an IPv6 address starting at position `start`.
fn try_match_ipv6(chars: &[char], start: usize) -> Option<usize> {
    // IPv6: groups of hex digits separated by colons, with possible ::
    let mut i = start;
    let mut colon_count = 0;
    let mut hex_groups = 0;
    let mut has_double_colon = false;

    // Must start with hex digit or ':' (for ::).
    if i >= chars.len() {
        return None;
    }
    if !chars[i].is_ascii_hexdigit() && chars[i] != ':' {
        return None;
    }

    // Don't match if preceded by alphanumeric (part of a word).
    if start > 0 && (chars[start - 1].is_ascii_alphanumeric() || chars[start - 1] == ':') {
        return None;
    }

    let mut in_hex_group = false;
    let mut group_len = 0;
    let mut prev_was_colon = false;

    while i < chars.len() {
        if chars[i].is_ascii_hexdigit() {
            if !in_hex_group {
                in_hex_group = true;
                hex_groups += 1;
                group_len = 0;
            }
            group_len += 1;
            if group_len > 4 {
                break;
            }
            prev_was_colon = false;
            i += 1;
        } else if chars[i] == ':' {
            if prev_was_colon {
                has_double_colon = true;
            }
            colon_count += 1;
            in_hex_group = false;
            prev_was_colon = true;
            i += 1;
        } else {
            break;
        }
    }

    // A valid IPv6 needs either:
    // - 7+ colons (full form), or
    // - :: with at least 1 hex group and 2+ colons
    let is_valid = (colon_count >= 3 && hex_groups >= 3)
        || (has_double_colon && hex_groups >= 1 && colon_count >= 2);

    if is_valid {
        // Don't match if followed by alphanumeric.
        if i < chars.len() && chars[i].is_ascii_alphanumeric() {
            return None;
        }
        Some(i)
    } else {
        None
    }
}

/// Replace MAC addresses (xx:xx:xx:xx:xx:xx or xx-xx-xx-xx-xx-xx).
fn scrub_mac(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        if let Some(end) = try_match_mac(&chars, i) {
            result.push_str(REDACTED_MAC);
            i = end;
        } else {
            result.push(chars[i]);
            i += 1;
        }
    }

    result
}

/// Try to match a MAC address at position `start`.
fn try_match_mac(chars: &[char], start: usize) -> Option<usize> {
    // MAC: XX:XX:XX:XX:XX:XX or XX-XX-XX-XX-XX-XX
    // Exactly 6 groups of 2 hex digits separated by : or -.
    if start + 17 > chars.len() {
        return None;
    }

    // Don't match if preceded by hex digit or colon.
    if start > 0 && (chars[start - 1].is_ascii_hexdigit() || chars[start - 1] == ':') {
        return None;
    }

    let separator = chars[start + 2];
    if separator != ':' && separator != '-' {
        return None;
    }

    for group in 0..6 {
        let base = start + group * 3;
        if !chars[base].is_ascii_hexdigit() || !chars[base + 1].is_ascii_hexdigit() {
            return None;
        }
        if group < 5 && chars[base + 2] != separator {
            return None;
        }
    }

    let end = start + 17;
    // Don't match if followed by hex or separator.
    if end < chars.len() && (chars[end].is_ascii_hexdigit() || chars[end] == separator) {
        return None;
    }

    Some(end)
}

/// Replace email-like patterns.
fn scrub_email(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut last_end = 0;

    for (i, _) in text.char_indices().filter(|&(_, c)| c == '@') {
        // Look backwards for local part start.
        let local_start = find_email_local_start_str(text, i);
        // Look forwards for domain end.
        if let Some(domain_end) = find_email_domain_end_str(text, i + 1) {
            // Emit everything before the email.
            result.push_str(&text[last_end..local_start]);
            result.push_str(REDACTED_EMAIL);
            last_end = domain_end;
        }
    }

    result.push_str(&text[last_end..]);
    result
}

fn find_email_local_start_str(text: &str, at_pos: usize) -> usize {
    let bytes = text.as_bytes();
    let mut i = at_pos;
    while i > 0 {
        let c = bytes[i - 1];
        if c.is_ascii_alphanumeric() || c == b'.' || c == b'+' || c == b'-' || c == b'_' {
            i -= 1;
        } else {
            break;
        }
    }
    i
}

fn find_email_domain_end_str(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let mut i = start;
    let mut has_dot = false;
    while i < bytes.len() {
        let c = bytes[i];
        if c.is_ascii_alphanumeric() || c == b'-' {
            i += 1;
        } else if c == b'.' {
            has_dot = true;
            i += 1;
        } else {
            break;
        }
    }
    // Must have at least one dot and end after start.
    if has_dot && i > start + 2 {
        Some(i)
    } else {
        None
    }
}

/// Replace /home/username paths.
fn scrub_home_paths(text: &str) -> String {
    let mut result = String::with_capacity(text.len());
    let mut remaining = text;

    while let Some(idx) = remaining.find("/home/") {
        result.push_str(&remaining[..idx]);
        let after_home = &remaining[idx + 6..];
        // Find end of username component.
        let end = after_home
            .find(|c: char| c == '/' || c.is_whitespace())
            .unwrap_or(after_home.len());
        if end > 0 {
            result.push_str(REDACTED_PATH);
            // Skip the rest of the path if it continues.
            let path_rest = &after_home[end..];
            if let Some(path_end) = path_rest.find(|c: char| c.is_whitespace()) {
                // Include the rest of the path in the redaction.
                remaining = &path_rest[path_end..];
            } else {
                remaining = "";
            }
        } else {
            result.push_str("/home/");
            remaining = after_home;
        }
    }

    result.push_str(remaining);
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrub_ipv4_replaces_addresses() {
        let input = "Connection from 192.168.1.100 on port 8080";
        let policy = RedactionPolicy::default();
        let result = scrub_residuals(input, &policy);
        assert!(
            !result.contains("192.168.1.100"),
            "IPv4 not redacted: {result}"
        );
        assert!(result.contains(REDACTED_IP));
    }

    #[test]
    fn scrub_ipv4_preserves_version_with_prefix() {
        let input = "Running version v1.2.3.4 of app";
        let result = scrub_ipv4(input);
        // v1.2.3.4 preceded by 'v' — should not be treated as IP.
        assert!(
            result.contains("v1.2.3.4"),
            "version was incorrectly redacted: {result}"
        );
    }

    #[test]
    fn scrub_ipv4_replaces_loopback() {
        let input = "Listening on 127.0.0.1:3000";
        let result = scrub_ipv4(input);
        assert!(
            !result.contains("127.0.0.1"),
            "loopback not redacted: {result}"
        );
        assert!(result.contains(REDACTED_IP));
    }

    #[test]
    fn scrub_ipv6_replaces_full_address() {
        let input = "peer 2001:0db8:85a3:0000:0000:8a2e:0370:7334 connected";
        let result = scrub_ipv6(input);
        assert!(!result.contains("2001:0db8"), "IPv6 not redacted: {result}");
        assert!(result.contains(REDACTED_IP));
    }

    #[test]
    fn scrub_ipv6_replaces_compressed() {
        let input = "addr fe80::1 link-local";
        let result = scrub_ipv6(input);
        assert!(
            !result.contains("fe80::1"),
            "compressed IPv6 not redacted: {result}"
        );
    }

    #[test]
    fn scrub_mac_replaces_colon_format() {
        let input = "interface aa:bb:cc:dd:ee:ff is up";
        let result = scrub_mac(input);
        assert!(
            !result.contains("aa:bb:cc:dd:ee:ff"),
            "MAC not redacted: {result}"
        );
        assert!(result.contains(REDACTED_MAC));
    }

    #[test]
    fn scrub_mac_replaces_dash_format() {
        let input = "device AA-BB-CC-DD-EE-FF";
        let result = scrub_mac(input);
        assert!(
            !result.contains("AA-BB-CC-DD-EE-FF"),
            "MAC not redacted: {result}"
        );
        assert!(result.contains(REDACTED_MAC));
    }

    #[test]
    fn scrub_email_replaces_addresses() {
        let input = "contact admin@example.com for help";
        let result = scrub_email(input);
        assert!(
            !result.contains("admin@example.com"),
            "email not redacted: {result}"
        );
        assert!(result.contains(REDACTED_EMAIL));
    }

    #[test]
    fn scrub_email_handles_plus_addressing() {
        let input = "user+tag@domain.org sent";
        let result = scrub_email(input);
        assert!(
            !result.contains("user+tag@domain.org"),
            "email not redacted: {result}"
        );
    }

    #[test]
    fn scrub_home_paths_replaces_user_paths() {
        let input = "file at /home/alice/Documents/report.txt";
        let result = scrub_home_paths(input);
        assert!(
            !result.contains("alice"),
            "home path not redacted: {result}"
        );
        assert!(result.contains(REDACTED_PATH));
    }

    #[test]
    fn scrub_home_paths_preserves_non_home() {
        let input = "binary at /usr/bin/ls";
        let result = scrub_home_paths(input);
        assert!(result.contains("/usr/bin/ls"));
    }

    #[test]
    fn default_policy_redacts_all_categories() {
        let policy = RedactionPolicy::default();
        for cat in RedactionCategory::ALL {
            assert!(policy.should_redact(*cat));
        }
    }

    #[test]
    fn empty_policy_redacts_nothing() {
        let policy = RedactionPolicy {
            categories: Vec::new(),
        };
        for cat in RedactionCategory::ALL {
            assert!(!policy.should_redact(*cat));
        }
    }

    #[test]
    fn residual_scan_catches_mixed_sensitive_data() {
        let input = "Host 10.0.0.1 with MAC aa:bb:cc:dd:ee:ff at /home/bob/secrets";
        let policy = RedactionPolicy::default();
        let result = scrub_residuals(input, &policy);
        assert!(!result.contains("10.0.0.1"));
        assert!(!result.contains("aa:bb:cc:dd:ee:ff"));
        assert!(!result.contains("bob"));
    }

    #[test]
    fn scrub_process_args_redacts_after_double_dash() {
        let input = "/usr/bin/app -- --secret-key=abc123";
        let result = scrub_process_args(input);
        assert!(!result.contains("abc123"));
        assert!(result.contains(REDACTED_ARGS));
    }

    #[test]
    fn scrub_process_args_preserves_without_separator() {
        let input = "systemd: started unit";
        let result = scrub_process_args(input);
        assert_eq!(result, input);
    }

    #[test]
    fn os_section_redacts_username_and_hostname() {
        let os = OsSection {
            distro: "Ubuntu 24.04".into(),
            kernel: "6.8.0-generic".into(),
            uptime_seconds: 3600,
            desktop: Some("GNOME".into()),
            hostname: "my-secret-host".into(),
            username: "alice".into(),
        };
        let policy = RedactionPolicy::default();
        let redacted = redact_os(&os, &policy);
        assert_eq!(redacted.hostname, REDACTED_HOSTNAME);
        assert_eq!(redacted.username, REDACTED_USERNAME);
        assert_eq!(redacted.distro, "Ubuntu 24.04");
    }

    #[test]
    fn os_section_preserves_when_policy_allows() {
        let os = OsSection {
            distro: "Ubuntu 24.04".into(),
            kernel: "6.8.0-generic".into(),
            uptime_seconds: 3600,
            desktop: None,
            hostname: "workstation".into(),
            username: "bob".into(),
        };
        let policy = RedactionPolicy {
            categories: Vec::new(),
        };
        let redacted = redact_os(&os, &policy);
        assert_eq!(redacted.hostname, "workstation");
        assert_eq!(redacted.username, "bob");
    }
}
