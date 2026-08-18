//! Systemd failed-service adapter.
//!
//! Uses `systemctl` through `CommandRunner` to enumerate failed services.
//! Returns `Capability::Unavailable` when systemd is absent.

use crate::capability::{Capability, UnavailableReason};
use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::error::MaintainerError;
use crate::jobs::Cancellation;

/// A systemd service in the failed state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FailedService {
    /// Unit name (e.g., "nginx.service").
    pub unit: String,
    /// Load state (e.g., "loaded").
    pub load_state: String,
    /// Active state (e.g., "failed").
    pub active_state: String,
    /// Sub state (e.g., "failed", "dead").
    pub sub_state: String,
    /// Description from systemd.
    pub description: String,
}

/// Service diagnostics adapter.
pub struct ServiceDiagnostics;

impl ServiceDiagnostics {
    /// List all failed services on this system.
    ///
    /// Returns `Unavailable` when systemctl is not present or systemd
    /// is not running.
    pub fn list_failed(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<Vec<FailedService>> {
        let spec = match CommandSpec::new(
            "systemctl",
            [
                "list-units",
                "--type=service",
                "--state=failed",
                "--no-pager",
                "--no-legend",
                "--plain",
            ],
        ) {
            Ok(s) => s,
            Err(e) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "systemctl-unavailable",
                    message: format!("Cannot build systemctl command: {e}"),
                    remediation: None,
                });
            }
        };

        let output = match runner.run(&spec, cancel) {
            Ok(o) => o,
            Err(MaintainerError::Cancelled) => return Capability::Available(Vec::new()),
            Err(MaintainerError::Io { .. }) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "systemd-absent",
                    message: "systemctl is not available on this system".into(),
                    remediation: Some("This system may not use systemd as its init system".into()),
                });
            }
            Err(e) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "systemctl-error",
                    message: format!("systemctl failed: {e}"),
                    remediation: None,
                });
            }
        };

        // systemctl may exit non-zero when there are failed units
        // but still produce valid output.
        let services = parse_systemctl_output(&output);
        Capability::Available(services)
    }
}

/// Parse the plain output from `systemctl list-units --no-legend --plain`.
///
/// Each line has the format:
/// `UNIT LOAD ACTIVE SUB DESCRIPTION...`
pub fn parse_systemctl_output(output: &CommandOutput) -> Vec<FailedService> {
    output
        .stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim();
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(5, char::is_whitespace);
            let unit = parts.next()?.trim().to_string();
            let load_state = skip_whitespace_parts(&mut parts)?;
            let active_state = skip_whitespace_parts(&mut parts)?;
            let sub_state = skip_whitespace_parts(&mut parts)?;
            let description = parts.next().unwrap_or("").trim().to_string();

            if unit.is_empty() {
                return None;
            }

            Some(FailedService {
                unit,
                load_state,
                active_state,
                sub_state,
                description,
            })
        })
        .collect()
}

/// Advance an iterator past whitespace-only items and return the next non-empty value.
fn skip_whitespace_parts<'a>(iter: &mut impl Iterator<Item = &'a str>) -> Option<String> {
    for part in iter.by_ref() {
        let trimmed = part.trim();
        if !trimmed.is_empty() {
            return Some(trimmed.to_string());
        }
    }
    None
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
    fn parse_single_failed_service() {
        let out = output_from(
            "nginx.service loaded failed failed The NGINX HTTP and reverse proxy server\n",
        );
        let services = parse_systemctl_output(&out);
        assert_eq!(services.len(), 1);
        assert_eq!(services[0].unit, "nginx.service");
        assert_eq!(services[0].load_state, "loaded");
        assert_eq!(services[0].active_state, "failed");
        assert_eq!(services[0].sub_state, "failed");
        assert_eq!(
            services[0].description,
            "The NGINX HTTP and reverse proxy server"
        );
    }

    #[test]
    fn parse_multiple_services() {
        let out = output_from(
            "nginx.service loaded failed failed NGINX HTTP Server\n\
             docker.service loaded failed failed Docker Application Container Engine\n",
        );
        let services = parse_systemctl_output(&out);
        assert_eq!(services.len(), 2);
        assert_eq!(services[0].unit, "nginx.service");
        assert_eq!(services[1].unit, "docker.service");
    }

    #[test]
    fn parse_empty_output() {
        let out = output_from("");
        let services = parse_systemctl_output(&out);
        assert!(services.is_empty());
    }

    #[test]
    fn parse_whitespace_only_lines_skipped() {
        let out = output_from("  \n\n  \n");
        let services = parse_systemctl_output(&out);
        assert!(services.is_empty());
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
                    "spawning systemctl",
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

        let result = ServiceDiagnostics::list_failed(&FailRunner, &NeverCancel);
        assert!(matches!(result, Capability::Unavailable(_)));
        let reason = result.reason().unwrap();
        assert_eq!(reason.code, "systemd-absent");
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

        let result = ServiceDiagnostics::list_failed(&CancelRunner, &AlwaysCancel);
        match result {
            Capability::Available(v) => assert!(v.is_empty()),
            _ => panic!("Expected Available on cancel"),
        }
    }
}
