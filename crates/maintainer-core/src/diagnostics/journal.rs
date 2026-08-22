//! Journal event collector.
//!
//! Invokes `journalctl` with explicit arguments, `LC_ALL=C`, JSON output,
//! priority/time filters, line and byte caps. Never invokes a shell.

use std::time::Duration;

use crate::capability::{Capability, UnavailableReason};
use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::error::MaintainerError;
use crate::jobs::Cancellation;

/// Maximum journal lines to collect in one invocation.
const DEFAULT_LINE_CAP: u32 = 2000;

/// Maximum output bytes from journalctl.
const DEFAULT_BYTE_CAP: usize = 5 * 1024 * 1024; // 5 MiB

/// Syslog priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Priority {
    Emergency = 0,
    Alert = 1,
    Critical = 2,
    Error = 3,
    Warning = 4,
    Notice = 5,
    Info = 6,
    Debug = 7,
}

impl Priority {
    /// Format as journalctl priority argument value.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Emergency => "0",
            Self::Alert => "1",
            Self::Critical => "2",
            Self::Error => "3",
            Self::Warning => "4",
            Self::Notice => "5",
            Self::Info => "6",
            Self::Debug => "7",
        }
    }
}

/// Filter parameters for journal collection.
#[derive(Debug, Clone)]
pub struct JournalFilter {
    /// Maximum priority (inclusive). Events with higher numeric priority
    /// (lower severity) are excluded.
    pub max_priority: Priority,
    /// Time window (e.g., "24h", "7d", "1h").
    pub since: String,
    /// Maximum number of lines to collect.
    pub line_cap: u32,
    /// Maximum bytes to capture.
    pub byte_cap: usize,
    /// Optional unit filter (e.g., "nginx.service").
    pub unit: Option<String>,
}

impl Default for JournalFilter {
    fn default() -> Self {
        Self {
            max_priority: Priority::Warning,
            since: "24h ago".into(),
            line_cap: DEFAULT_LINE_CAP,
            byte_cap: DEFAULT_BYTE_CAP,
            unit: None,
        }
    }
}

/// A parsed journal entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JournalEntry {
    /// The message text.
    pub message: String,
    /// Systemd unit name (e.g., "nginx.service").
    pub unit: Option<String>,
    /// Syslog identifier (e.g., "kernel", "systemd").
    pub syslog_identifier: Option<String>,
    /// Numeric priority (0–7).
    pub priority: u8,
    /// Process ID (volatile, used for normalization only).
    pub pid: Option<u32>,
    /// Monotonic timestamp in microseconds (for ordering).
    pub monotonic_usec: Option<u64>,
    /// Realtime timestamp in microseconds.
    pub realtime_usec: Option<u64>,
}

/// Journal collector.
pub struct JournalCollector;

impl JournalCollector {
    /// Collect journal entries matching the given filter.
    ///
    /// Returns `Unavailable` when journalctl is not present.
    pub fn collect(
        filter: &JournalFilter,
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<Vec<JournalEntry>> {
        let mut args: Vec<String> = vec![
            "--output=json".into(),
            "--no-pager".into(),
            format!("--priority={}", filter.max_priority.as_str()),
            format!("--since={}", filter.since),
            format!("--lines={}", filter.line_cap),
        ];

        if let Some(ref unit) = filter.unit {
            args.push(format!("--unit={unit}"));
        }

        let spec = match CommandSpec::new("journalctl", args) {
            Ok(s) => s
                .with_output_cap(filter.byte_cap)
                .with_timeout(Duration::from_secs(30)),
            Err(e) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "journalctl-unavailable",
                    message: format!("Cannot build journalctl command: {e}"),
                    remediation: None,
                });
            }
        };

        let output = match runner.run(&spec, cancel) {
            Ok(o) => o,
            Err(MaintainerError::Cancelled) => return Capability::Available(Vec::new()),
            Err(MaintainerError::Io { .. }) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "journalctl-absent",
                    message: "journalctl is not available on this system".into(),
                    remediation: Some(
                        "This system may not use systemd-journald for logging".into(),
                    ),
                });
            }
            Err(e) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "journalctl-error",
                    message: format!("journalctl failed: {e}"),
                    remediation: None,
                });
            }
        };

        let entries = parse_journal_output(&output);
        Capability::Available(entries)
    }
}

/// Parse JSON-lines output from journalctl.
///
/// Malformed lines are silently skipped — the journal may contain
/// binary fields or corrupt entries.
pub fn parse_journal_output(output: &CommandOutput) -> Vec<JournalEntry> {
    output
        .stdout
        .lines()
        .filter_map(parse_journal_line)
        .collect()
}

/// Parse a single JSON line from journalctl output.
fn parse_journal_line(line: &str) -> Option<JournalEntry> {
    let line = line.trim();
    if line.is_empty() || !line.starts_with('{') {
        return None;
    }

    let message = extract_json_string(line, "MESSAGE")?;
    let unit = extract_json_string(line, "_SYSTEMD_UNIT");
    let syslog_identifier = extract_json_string(line, "SYSLOG_IDENTIFIER");
    let priority = extract_json_string(line, "PRIORITY")
        .and_then(|s| s.parse::<u8>().ok())
        .unwrap_or(6);
    let pid = extract_json_string(line, "_PID").and_then(|s| s.parse::<u32>().ok());
    let monotonic_usec =
        extract_json_string(line, "__MONOTONIC_TIMESTAMP").and_then(|s| s.parse::<u64>().ok());
    let realtime_usec =
        extract_json_string(line, "__REALTIME_TIMESTAMP").and_then(|s| s.parse::<u64>().ok());

    Some(JournalEntry {
        message,
        unit,
        syslog_identifier,
        priority,
        pid,
        monotonic_usec,
        realtime_usec,
    })
}

/// Extract a string value for a given key from a flat JSON object.
///
/// This is a lightweight parser for the specific flat structure that
/// journalctl produces. It avoids adding serde_json as a dependency.
fn extract_json_string(json: &str, key: &str) -> Option<String> {
    let search = format!("\"{}\"", key);
    let key_start = json.find(&search)?;
    let after_key = key_start + search.len();
    let rest = &json[after_key..];

    let rest = rest.trim_start();
    let rest = rest.strip_prefix(':')?;
    let rest = rest.trim_start();

    if let Some(content) = rest.strip_prefix('"') {
        let mut value = String::new();
        let mut chars = content.chars();
        loop {
            match chars.next() {
                Some('\\') => match chars.next() {
                    Some('n') => value.push('\n'),
                    Some('t') => value.push('\t'),
                    Some('\\') => value.push('\\'),
                    Some('"') => value.push('"'),
                    Some('/') => value.push('/'),
                    Some(c) => {
                        value.push('\\');
                        value.push(c);
                    }
                    None => break,
                },
                Some('"') => break,
                Some(c) => value.push(c),
                None => break,
            }
        }
        Some(value)
    } else if rest.starts_with('[') {
        None
    } else {
        // Numeric or other literal — take until comma, brace, or whitespace.
        let end = rest
            .find(|c: char| c == ',' || c == '}' || c == ']' || c.is_whitespace())
            .unwrap_or(rest.len());
        let val = rest[..end].trim();
        if val == "null" {
            None
        } else {
            Some(val.to_string())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandOutput;

    fn output_from(stdout: &str) -> CommandOutput {
        CommandOutput {
            exit_code: Some(0),
            stdout: stdout.to_string(),
            stderr: String::new(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    #[test]
    fn parse_valid_journal_line() {
        let json = r#"{"MESSAGE":"Failed to start nginx","_SYSTEMD_UNIT":"nginx.service","SYSLOG_IDENTIFIER":"systemd","PRIORITY":"3","_PID":"1","__REALTIME_TIMESTAMP":"1700000000000000","__MONOTONIC_TIMESTAMP":"5000000"}"#;
        let out = output_from(json);
        let entries = parse_journal_output(&out);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "Failed to start nginx");
        assert_eq!(entries[0].unit.as_deref(), Some("nginx.service"));
        assert_eq!(entries[0].syslog_identifier.as_deref(), Some("systemd"));
        assert_eq!(entries[0].priority, 3);
        assert_eq!(entries[0].pid, Some(1));
    }

    #[test]
    fn parse_multiple_lines() {
        let json = r#"{"MESSAGE":"error one","PRIORITY":"3"}
{"MESSAGE":"error two","PRIORITY":"4"}"#;
        let out = output_from(json);
        let entries = parse_journal_output(&out);
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].message, "error one");
        assert_eq!(entries[1].message, "error two");
    }

    #[test]
    fn malformed_lines_skipped() {
        let json = "not json\n{\"MESSAGE\":\"ok\",\"PRIORITY\":\"4\"}\n{broken\n";
        let out = output_from(json);
        let entries = parse_journal_output(&out);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "ok");
    }

    #[test]
    fn empty_output_returns_empty_vec() {
        let out = output_from("");
        let entries = parse_journal_output(&out);
        assert!(entries.is_empty());
    }

    #[test]
    fn binary_field_array_skipped() {
        let json = r#"{"MESSAGE":[72,101,108,108,111],"PRIORITY":"3"}"#;
        let out = output_from(json);
        let entries = parse_journal_output(&out);
        assert!(entries.is_empty());
    }

    #[test]
    fn escaped_characters_handled() {
        let json = r#"{"MESSAGE":"line1\nline2\ttab\\slash\"quote","PRIORITY":"4"}"#;
        let out = output_from(json);
        let entries = parse_journal_output(&out);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].message, "line1\nline2\ttab\\slash\"quote");
    }

    #[test]
    fn missing_optional_fields() {
        let json = r#"{"MESSAGE":"hello","PRIORITY":"6"}"#;
        let out = output_from(json);
        let entries = parse_journal_output(&out);
        assert_eq!(entries.len(), 1);
        assert!(entries[0].unit.is_none());
        assert!(entries[0].syslog_identifier.is_none());
        assert!(entries[0].pid.is_none());
    }

    #[test]
    fn returns_unavailable_on_io_error() {
        struct FailRunner;
        impl CommandRunner for FailRunner {
            fn run(
                &self,
                _spec: &CommandSpec,
                _cancel: &dyn Cancellation,
            ) -> crate::error::Result<CommandOutput> {
                Err(MaintainerError::io(
                    "spawning journalctl",
                    std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
                ))
            }
        }
        struct NeverCancel;
        impl Cancellation for NeverCancel {
            fn is_cancelled(&self) -> bool {
                false
            }
        }

        let filter = JournalFilter::default();
        let result = JournalCollector::collect(&filter, &FailRunner, &NeverCancel);
        assert!(matches!(result, Capability::Unavailable(_)));
        let reason = result.reason().unwrap();
        assert_eq!(reason.code, "journalctl-absent");
    }

    #[test]
    fn returns_empty_on_cancel() {
        struct CancelRunner;
        impl CommandRunner for CancelRunner {
            fn run(
                &self,
                _spec: &CommandSpec,
                _cancel: &dyn Cancellation,
            ) -> crate::error::Result<CommandOutput> {
                Err(MaintainerError::Cancelled)
            }
        }
        struct AlwaysCancel;
        impl Cancellation for AlwaysCancel {
            fn is_cancelled(&self) -> bool {
                true
            }
        }

        let filter = JournalFilter::default();
        let result = JournalCollector::collect(&filter, &CancelRunner, &AlwaysCancel);
        match result {
            Capability::Available(v) => assert!(v.is_empty()),
            _ => panic!("Expected Available on cancel"),
        }
    }
}
