//! Evidence-first interpretation of diagnostic findings.
//!
//! Rules are allowlisted and fixture-tested. Unknown events receive
//! NO invented cause. Possible causes use cautious language.

use super::group::EventGroup;
use super::services::FailedService;

/// A piece of evidence supporting a finding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evidence {
    /// What was observed.
    pub observation: String,
    /// Source of the evidence (e.g., "journal", "systemctl").
    pub source: &'static str,
    /// How many times this was observed, if applicable.
    pub count: Option<u32>,
}

/// A diagnostic finding with evidence-first structure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiagnosticFinding {
    /// What was detected (factual statement).
    pub detected: String,
    /// Supporting evidence.
    pub evidence: Vec<Evidence>,
    /// Possible causes (cautious language, never speculative).
    pub possible_causes: Vec<String>,
    /// Recommended action, if any.
    pub recommendation: Option<String>,
}

/// An interpretation rule that matches a pattern and produces a finding.
pub trait InterpretationRule: Send + Sync {
    /// Attempt to match this rule against an event group.
    /// Returns `None` if the rule does not apply.
    fn match_group(&self, group: &EventGroup) -> Option<DiagnosticFinding>;

    /// Attempt to match this rule against a failed service.
    /// Returns `None` if the rule does not apply.
    fn match_service(&self, _service: &FailedService) -> Option<DiagnosticFinding> {
        None
    }
}

// ---------------------------------------------------------------------------
// Built-in rules
// ---------------------------------------------------------------------------

/// Rule: OOM killer activity.
struct OomKillerRule;

impl InterpretationRule for OomKillerRule {
    fn match_group(&self, group: &EventGroup) -> Option<DiagnosticFinding> {
        let template = &group.key.template;
        if !template.contains("Out of memory")
            && !template.contains("oom-kill")
            && !template.contains("OOM")
            && !template.contains("Killed process")
        {
            return None;
        }

        Some(DiagnosticFinding {
            detected: "OOM killer was invoked".into(),
            evidence: vec![Evidence {
                observation: group.samples.first().cloned().unwrap_or_default(),
                source: "journal",
                count: Some(group.count),
            }],
            possible_causes: vec![
                "A process may have consumed more memory than the system can provide".into(),
                "Available swap space may have been exhausted".into(),
            ],
            recommendation: Some(
                "Review memory usage of running processes. Consider adding swap \
                 or reducing memory-intensive workloads."
                    .into(),
            ),
        })
    }
}

/// Rule: Disk space exhaustion.
struct DiskFullRule;

impl InterpretationRule for DiskFullRule {
    fn match_group(&self, group: &EventGroup) -> Option<DiagnosticFinding> {
        let template = &group.key.template;
        if !template.contains("No space left on device")
            && !template.contains("disk full")
            && !template.contains("ENOSPC")
        {
            return None;
        }

        Some(DiagnosticFinding {
            detected: "Disk space exhaustion detected".into(),
            evidence: vec![Evidence {
                observation: group.samples.first().cloned().unwrap_or_default(),
                source: "journal",
                count: Some(group.count),
            }],
            possible_causes: vec![
                "One or more partitions may have reached capacity".into(),
                "Log files or temporary data may have accumulated beyond available space".into(),
            ],
            recommendation: Some(
                "Check available space with the Storage page. Remove unnecessary \
                 files or expand the partition."
                    .into(),
            ),
        })
    }
}

/// Rule: Segmentation fault.
struct SegfaultRule;

impl InterpretationRule for SegfaultRule {
    fn match_group(&self, group: &EventGroup) -> Option<DiagnosticFinding> {
        let template = &group.key.template;
        if !template.contains("segfault") && !template.contains("SIGSEGV") {
            return None;
        }

        Some(DiagnosticFinding {
            detected: "Process segmentation fault occurred".into(),
            evidence: vec![Evidence {
                observation: group.samples.first().cloned().unwrap_or_default(),
                source: "journal",
                count: Some(group.count),
            }],
            possible_causes: vec![
                "The affected process may have a memory corruption bug".into(),
                "A shared library may be incompatible with the running version".into(),
            ],
            recommendation: Some(
                "Check for updates to the affected software. If the issue persists, \
                 file a bug report with the crash details."
                    .into(),
            ),
        })
    }
}

/// Rule: Permission denial.
struct PermissionDeniedRule;

impl InterpretationRule for PermissionDeniedRule {
    fn match_group(&self, group: &EventGroup) -> Option<DiagnosticFinding> {
        let template = &group.key.template;
        if !template.contains("Permission denied")
            && !template.contains("access denied")
            && !template.contains("EACCES")
        {
            return None;
        }

        Some(DiagnosticFinding {
            detected: "Permission denial detected".into(),
            evidence: vec![Evidence {
                observation: group.samples.first().cloned().unwrap_or_default(),
                source: "journal",
                count: Some(group.count),
            }],
            possible_causes: vec![
                "A service may be attempting to access a resource without sufficient permissions"
                    .into(),
                "File or directory permissions may have changed unexpectedly".into(),
            ],
            recommendation: Some(
                "Review the affected file permissions and service configuration. \
                 Check if recent updates modified access controls."
                    .into(),
            ),
        })
    }
}

/// Rule: Service start failure (from failed services).
struct ServiceFailedRule;

impl InterpretationRule for ServiceFailedRule {
    fn match_group(&self, _group: &EventGroup) -> Option<DiagnosticFinding> {
        None
    }

    fn match_service(&self, service: &FailedService) -> Option<DiagnosticFinding> {
        if service.active_state != "failed" {
            return None;
        }

        Some(DiagnosticFinding {
            detected: format!("Service {} is in failed state", service.unit),
            evidence: vec![Evidence {
                observation: format!(
                    "Unit {} is {}/{}: {}",
                    service.unit, service.active_state, service.sub_state, service.description
                ),
                source: "systemctl",
                count: Some(1),
            }],
            possible_causes: vec![
                "The service may have exited with an error during startup or operation".into(),
                "A required dependency or resource may be unavailable".into(),
            ],
            recommendation: Some(format!(
                "Run `systemctl status {}` for detailed error output. \
                 Check journal entries for this unit.",
                service.unit
            )),
        })
    }
}

/// Rule: Rate-limited messages.
struct RateLimitRule;

impl InterpretationRule for RateLimitRule {
    fn match_group(&self, group: &EventGroup) -> Option<DiagnosticFinding> {
        let template = &group.key.template;
        if !template.contains("rate limit")
            && !template.contains("suppressed")
            && !template.contains("messages lost")
        {
            return None;
        }

        Some(DiagnosticFinding {
            detected: "Journal rate limiting is active".into(),
            evidence: vec![Evidence {
                observation: group.samples.first().cloned().unwrap_or_default(),
                source: "journal",
                count: Some(group.count),
            }],
            possible_causes: vec![
                "A service may be producing log messages faster than the configured rate limit"
                    .into(),
            ],
            recommendation: Some(
                "Investigate the source of excessive logging. Some messages \
                 may have been lost due to rate limiting."
                    .into(),
            ),
        })
    }
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// All built-in interpretation rules.
fn built_in_rules() -> Vec<Box<dyn InterpretationRule>> {
    vec![
        Box::new(ServiceFailedRule),
        Box::new(OomKillerRule),
        Box::new(DiskFullRule),
        Box::new(SegfaultRule),
        Box::new(PermissionDeniedRule),
        Box::new(RateLimitRule),
    ]
}

/// Interpret grouped events and failed services into diagnostic findings.
///
/// Unknown events that do not match any rule are NOT given an invented cause.
/// They are omitted from findings entirely.
pub fn interpret_groups(
    groups: &[EventGroup],
    services: &[FailedService],
) -> Vec<DiagnosticFinding> {
    let rules = built_in_rules();
    let mut findings = Vec::new();

    // Interpret failed services.
    for service in services {
        for rule in &rules {
            if let Some(finding) = rule.match_service(service) {
                findings.push(finding);
                break; // First matching rule wins per service.
            }
        }
    }

    // Interpret grouped journal events.
    for group in groups {
        for rule in &rules {
            if let Some(finding) = rule.match_group(group) {
                findings.push(finding);
                break; // First matching rule wins per group.
            }
        }
    }

    findings
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::diagnostics::group::{EventGroup, GroupKey};

    fn make_group(template: &str, message: &str, count: u32) -> EventGroup {
        EventGroup {
            key: GroupKey {
                template: template.to_string(),
                unit: Some("test.service".to_string()),
                syslog_identifier: Some("test".to_string()),
                priority: 3,
            },
            count,
            first_seen: Some(1700000000000000),
            last_seen: Some(1700000001000000),
            samples: vec![message.to_string()],
        }
    }

    #[test]
    fn oom_killer_detected() {
        let group = make_group(
            "Out of memory: Killed process [<PID>]",
            "Out of memory: Killed process [1234]",
            3,
        );
        let findings = interpret_groups(&[group], &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].detected.contains("OOM killer"));
        assert!(!findings[0].possible_causes.is_empty());
        assert!(findings[0].recommendation.is_some());
    }

    #[test]
    fn disk_full_detected() {
        let group = make_group(
            "No space left on device",
            "write failed: No space left on device",
            5,
        );
        let findings = interpret_groups(&[group], &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].detected.contains("Disk space"));
    }

    #[test]
    fn segfault_detected() {
        let group = make_group(
            "segfault at 0x0 ip 0x7fff",
            "segfault at 0x0 ip 0x7fff rip 0x0",
            1,
        );
        let findings = interpret_groups(&[group], &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].detected.contains("segmentation fault"));
    }

    #[test]
    fn permission_denied_detected() {
        let group = make_group(
            "Permission denied opening /var/log/secure",
            "Permission denied opening /var/log/secure",
            2,
        );
        let findings = interpret_groups(&[group], &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].detected.contains("Permission denial"));
    }

    #[test]
    fn failed_service_detected() {
        let service = FailedService {
            unit: "nginx.service".into(),
            load_state: "loaded".into(),
            active_state: "failed".into(),
            sub_state: "failed".into(),
            description: "The NGINX HTTP Server".into(),
        };
        let findings = interpret_groups(&[], &[service]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].detected.contains("nginx.service"));
        assert!(findings[0].detected.contains("failed"));
    }

    #[test]
    fn unknown_event_gets_no_cause() {
        let group = make_group(
            "custom application message xyz",
            "custom application message xyz",
            10,
        );
        let findings = interpret_groups(&[group], &[]);
        // Unknown events must NOT get an invented interpretation.
        assert!(findings.is_empty());
    }

    #[test]
    fn rate_limit_detected() {
        let group = make_group(
            "rate limit exceeded, suppressed 1000 messages",
            "rate limit exceeded, suppressed 1000 messages",
            1,
        );
        let findings = interpret_groups(&[group], &[]);
        assert_eq!(findings.len(), 1);
        assert!(findings[0].detected.contains("rate limiting"));
    }

    #[test]
    fn cautious_language_in_causes() {
        let group = make_group("Out of memory: Killed process [<PID>]", "OOM kill", 1);
        let findings = interpret_groups(&[group], &[]);
        for cause in &findings[0].possible_causes {
            // All causes should use cautious language.
            assert!(
                cause.contains("may")
                    || cause.contains("might")
                    || cause.contains("could")
                    || cause.contains("possible"),
                "Cause should use cautious language: {cause}"
            );
        }
    }

    #[test]
    fn empty_inputs_produce_empty_findings() {
        let findings = interpret_groups(&[], &[]);
        assert!(findings.is_empty());
    }

    #[test]
    fn multiple_findings_from_mixed_input() {
        let service = FailedService {
            unit: "docker.service".into(),
            load_state: "loaded".into(),
            active_state: "failed".into(),
            sub_state: "failed".into(),
            description: "Docker".into(),
        };
        let group = make_group("No space left on device", "ENOSPC", 2);
        let findings = interpret_groups(&[group], &[service]);
        assert_eq!(findings.len(), 2);
    }
}
