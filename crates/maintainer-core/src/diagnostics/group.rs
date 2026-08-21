//! Stable grouping of journal entries.
//!
//! Normalizes only documented volatile fields (PID, standalone timestamps)
//! while preserving unit, identifier, priority, and message template.
//! Unrelated messages must NEVER merge due to aggressive digit stripping.

use std::collections::HashMap;

use super::journal::JournalEntry;

/// Maximum evidence samples stored per group.
const MAX_EVIDENCE_SAMPLES: usize = 5;

/// Key used to group related journal entries.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GroupKey {
    /// Normalized message template.
    pub template: String,
    /// Unit name (preserved as-is).
    pub unit: Option<String>,
    /// Syslog identifier (preserved as-is).
    pub syslog_identifier: Option<String>,
    /// Priority level.
    pub priority: u8,
}

/// A group of related journal events.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EventGroup {
    /// The grouping key.
    pub key: GroupKey,
    /// Number of occurrences.
    pub count: u32,
    /// First occurrence timestamp (realtime microseconds).
    pub first_seen: Option<u64>,
    /// Last occurrence timestamp (realtime microseconds).
    pub last_seen: Option<u64>,
    /// Bounded evidence samples (original messages).
    pub samples: Vec<String>,
}

/// Group journal entries by stable message template.
///
/// The grouping preserves the structural identity of each message class
/// while normalizing only truly volatile noise (PIDs, standalone timestamps).
pub fn group_journal_entries(entries: &[JournalEntry]) -> Vec<EventGroup> {
    let mut groups: HashMap<GroupKey, EventGroup> = HashMap::new();

    for entry in entries {
        let key = GroupKey {
            template: normalize_message(&entry.message),
            unit: entry.unit.clone(),
            syslog_identifier: entry.syslog_identifier.clone(),
            priority: entry.priority,
        };

        let group = groups.entry(key.clone()).or_insert_with(|| EventGroup {
            key,
            count: 0,
            first_seen: None,
            last_seen: None,
            samples: Vec::new(),
        });

        group.count += 1;

        // Update time bounds.
        if let Some(ts) = entry.realtime_usec {
            match group.first_seen {
                None => group.first_seen = Some(ts),
                Some(first) if ts < first => group.first_seen = Some(ts),
                _ => {}
            }
            match group.last_seen {
                None => group.last_seen = Some(ts),
                Some(last) if ts > last => group.last_seen = Some(ts),
                _ => {}
            }
        }

        // Store bounded evidence samples.
        if group.samples.len() < MAX_EVIDENCE_SAMPLES {
            group.samples.push(entry.message.clone());
        }
    }

    let mut result: Vec<EventGroup> = groups.into_values().collect();
    // Sort by count descending, then by template for stability.
    result.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| a.key.template.cmp(&b.key.template))
    });
    result
}

/// Normalize a journal message for grouping purposes.
///
/// Rules (conservative — avoids merging unrelated messages):
/// - Replace standalone PIDs (`[1234]`, `pid=1234`, `PID 1234`) with placeholders.
/// - Replace standalone timestamps in ISO-like or epoch formats.
/// - Preserve port numbers, error codes, version numbers, and file paths.
/// - Do NOT strip all digits — that would merge unrelated messages.
fn normalize_message(msg: &str) -> String {
    let mut result = String::with_capacity(msg.len());
    let chars: Vec<char> = msg.chars().collect();
    let len = chars.len();
    let mut i = 0;

    while i < len {
        // Pattern: [DIGITS] (PID in square brackets)
        if chars[i] == '[' && i + 1 < len && chars[i + 1].is_ascii_digit() {
            let start = i;
            i += 1;
            let digit_start = i;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            if i < len && chars[i] == ']' && i - digit_start >= 1 {
                // Only normalize if it looks like a PID (1-7 digits).
                if i - digit_start <= 7 {
                    result.push_str("[<PID>]");
                    i += 1; // skip ]
                    continue;
                }
            }
            // Not a PID pattern — emit as-is.
            i = start;
            result.push(chars[i]);
            i += 1;
            continue;
        }

        // Pattern: pid=DIGITS or PID=DIGITS or PID DIGITS
        if i + 4 <= len {
            let slice: String = chars[i..i + 4].iter().collect();
            let lower = slice.to_lowercase();
            if lower == "pid=" || lower == "pid " {
                let prefix_end = i + 4;
                let mut j = prefix_end;
                while j < len && chars[j].is_ascii_digit() {
                    j += 1;
                }
                if j > prefix_end {
                    result.push_str(&chars[i..i + 3].iter().collect::<String>());
                    result.push(chars[i + 3]); // '=' or ' '
                    result.push_str("<PID>");
                    i = j;
                    continue;
                }
            }
        }

        // Pattern: standalone epoch timestamp (10+ digits not part of a path or version)
        if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_alphanumeric()) {
            let start = i;
            while i < len && chars[i].is_ascii_digit() {
                i += 1;
            }
            let digit_len = i - start;

            // Only replace if it looks like a standalone timestamp (10-16 digits)
            // and is not preceded by '.' (version) or '/' (path) or ':' (port).
            let preceded_by_structural = start > 0
                && (chars[start - 1] == '.'
                    || chars[start - 1] == '/'
                    || chars[start - 1] == ':'
                    || chars[start - 1] == '_'
                    || chars[start - 1] == '-');

            if (10..=16).contains(&digit_len) && !preceded_by_structural {
                result.push_str("<TIMESTAMP>");
            } else {
                // Emit digits as-is.
                for c in &chars[start..i] {
                    result.push(*c);
                }
            }
            continue;
        }

        result.push(chars[i]);
        i += 1;
    }

    result
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(msg: &str) -> JournalEntry {
        JournalEntry {
            message: msg.to_string(),
            unit: Some("test.service".to_string()),
            syslog_identifier: Some("test".to_string()),
            priority: 3,
            pid: Some(1234),
            monotonic_usec: None,
            realtime_usec: Some(1700000000000000),
        }
    }

    #[test]
    fn pid_in_brackets_normalized() {
        assert_eq!(
            normalize_message("process[12345] failed"),
            "process[<PID>] failed"
        );
    }

    #[test]
    fn pid_equals_normalized() {
        assert_eq!(
            normalize_message("connection from pid=9876 dropped"),
            "connection from pid=<PID> dropped"
        );
    }

    #[test]
    fn standalone_timestamp_normalized() {
        assert_eq!(
            normalize_message("event at 1700000000 completed"),
            "event at <TIMESTAMP> completed"
        );
    }

    #[test]
    fn port_numbers_preserved() {
        // Port after colon should NOT be normalized.
        assert_eq!(
            normalize_message("listening on :8080"),
            "listening on :8080"
        );
    }

    #[test]
    fn version_numbers_preserved() {
        // Version numbers with dots should NOT be normalized.
        assert_eq!(
            normalize_message("upgraded to 3.14.159"),
            "upgraded to 3.14.159"
        );
    }

    #[test]
    fn path_numbers_preserved() {
        // Numbers in file paths should NOT be normalized.
        assert_eq!(
            normalize_message("reading /proc/12345/status"),
            "reading /proc/12345/status"
        );
    }

    #[test]
    fn unrelated_messages_never_merge() {
        // Critical test: different error codes must NOT collapse.
        let entries = vec![
            entry("exit code 1"),
            entry("exit code 2"),
            entry("error 404 not found"),
            entry("error 500 internal"),
        ];
        let groups = group_journal_entries(&entries);
        // Each distinct message should be its own group.
        assert_eq!(groups.len(), 4, "Unrelated messages must not merge");
    }

    #[test]
    fn same_message_different_pids_groups() {
        let entries = vec![
            JournalEntry {
                message: "worker[1234] crashed".into(),
                unit: Some("app.service".into()),
                syslog_identifier: Some("app".into()),
                priority: 3,
                pid: Some(1234),
                monotonic_usec: None,
                realtime_usec: Some(1700000000000000),
            },
            JournalEntry {
                message: "worker[5678] crashed".into(),
                unit: Some("app.service".into()),
                syslog_identifier: Some("app".into()),
                priority: 3,
                pid: Some(5678),
                monotonic_usec: None,
                realtime_usec: Some(1700000001000000),
            },
        ];
        let groups = group_journal_entries(&entries);
        // Same message template with different PIDs should merge.
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 2);
    }

    #[test]
    fn different_units_do_not_merge() {
        let mut e1 = entry("segfault in module");
        e1.unit = Some("nginx.service".into());
        let mut e2 = entry("segfault in module");
        e2.unit = Some("apache.service".into());

        let groups = group_journal_entries(&[e1, e2]);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn group_tracks_time_bounds() {
        let entries = vec![
            JournalEntry {
                message: "error occurred".into(),
                unit: None,
                syslog_identifier: None,
                priority: 3,
                pid: None,
                monotonic_usec: None,
                realtime_usec: Some(1000),
            },
            JournalEntry {
                message: "error occurred".into(),
                unit: None,
                syslog_identifier: None,
                priority: 3,
                pid: None,
                monotonic_usec: None,
                realtime_usec: Some(5000),
            },
            JournalEntry {
                message: "error occurred".into(),
                unit: None,
                syslog_identifier: None,
                priority: 3,
                pid: None,
                monotonic_usec: None,
                realtime_usec: Some(3000),
            },
        ];
        let groups = group_journal_entries(&entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].first_seen, Some(1000));
        assert_eq!(groups[0].last_seen, Some(5000));
        assert_eq!(groups[0].count, 3);
    }

    #[test]
    fn evidence_samples_bounded() {
        let entries: Vec<JournalEntry> = (0..20)
            .map(|i| JournalEntry {
                message: format!("worker[{}] crashed", i),
                unit: Some("app.service".into()),
                syslog_identifier: None,
                priority: 3,
                pid: Some(i),
                monotonic_usec: None,
                realtime_usec: Some(u64::from(i) * 1000),
            })
            .collect();
        let groups = group_journal_entries(&entries);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].count, 20);
        assert_eq!(groups[0].samples.len(), 5); // MAX_EVIDENCE_SAMPLES
    }

    #[test]
    fn different_priorities_do_not_merge() {
        let mut e1 = entry("disk full");
        e1.priority = 3; // error
        let mut e2 = entry("disk full");
        e2.priority = 4; // warning

        let groups = group_journal_entries(&[e1, e2]);
        assert_eq!(groups.len(), 2);
    }

    #[test]
    fn empty_input_returns_empty() {
        let groups = group_journal_entries(&[]);
        assert!(groups.is_empty());
    }

    #[test]
    fn small_numbers_not_treated_as_timestamps() {
        // Error codes, exit codes, and small numbers must NOT be stripped.
        assert_eq!(normalize_message("exit status 127"), "exit status 127");
        assert_eq!(normalize_message("returned error 42"), "returned error 42");
    }

    #[test]
    fn hyphenated_numbers_preserved() {
        // UUID-like or structured identifiers should not be mangled.
        assert_eq!(
            normalize_message("job-12345-abcdef started"),
            "job-12345-abcdef started"
        );
    }
}
