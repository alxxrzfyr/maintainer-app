//! Optional fwupd firmware update adapter.
//!
//! Detects fwupd availability and lists pending firmware updates.
//! Returns `Capability::Unavailable` with an explanation when fwupd is absent.
//! This is not a required dependency for core APT updates.

use crate::capability::{Capability, UnavailableReason};
use crate::command::{CommandRunner, CommandSpec};
use crate::jobs::Cancellation;

use super::model::{HoldState, UpdateBackend, UpdateCandidate, UpdateClassification};

/// fwupd firmware update adapter.
pub struct FwupdAdapter;

impl FwupdAdapter {
    /// Check if fwupd is available on this system.
    pub fn check_available(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<()> {
        let spec = match CommandSpec::new("fwupdmgr", ["--version"]) {
            Ok(s) => s,
            Err(_) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "fwupd-not-installed",
                    message: "fwupd is not installed on this system".to_string(),
                    remediation: Some("Install fwupd: sudo apt install fwupd".to_string()),
                });
            }
        };

        match runner.run(&spec, cancel) {
            Ok(output) if output.success() => Capability::Available(()),
            Ok(_) => Capability::Unavailable(UnavailableReason {
                code: "fwupd-not-working",
                message: "fwupd is installed but not functioning correctly".to_string(),
                remediation: Some("Try restarting the fwupd service".to_string()),
            }),
            Err(_) => Capability::Unavailable(UnavailableReason {
                code: "fwupd-not-installed",
                message: "fwupd is not available".to_string(),
                remediation: Some("Install fwupd: sudo apt install fwupd".to_string()),
            }),
        }
    }

    /// List pending firmware updates.
    ///
    /// Uses `fwupdmgr get-updates` which is read-only and does not flash anything.
    pub fn list_updates(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<Vec<UpdateCandidate>> {
        let spec = match CommandSpec::new("fwupdmgr", ["get-updates", "--no-unreported-check"]) {
            Ok(s) => s,
            Err(_) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "fwupd-not-installed",
                    message: "fwupd is not available".to_string(),
                    remediation: Some("Install fwupd: sudo apt install fwupd".to_string()),
                });
            }
        };

        let output = match runner.run(&spec, cancel) {
            Ok(o) => o,
            Err(crate::error::MaintainerError::Cancelled) => {
                return Capability::Available(Vec::new());
            }
            Err(_) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "fwupd-check-failed",
                    message: "Failed to check for firmware updates".to_string(),
                    remediation: None,
                });
            }
        };

        if output.exit_code == Some(2) {
            return Capability::Available(Vec::new());
        }

        if !output.success() {
            return Capability::Unavailable(UnavailableReason {
                code: "fwupd-check-failed",
                message: "fwupd update check failed".to_string(),
                remediation: None,
            });
        }

        let candidates = parse_fwupd_updates(&output.stdout);
        Capability::Available(candidates)
    }
}

/// Parse fwupdmgr get-updates output.
///
/// The output format varies between fwupd versions but typically looks like:
/// ```text
/// Devices with pending updates:
/// ├─Device Name:
/// │     Current version:    1.2.3
/// │     Update Version:     1.2.4
/// │     Update Description: Bug fixes
/// ```
fn parse_fwupd_updates(stdout: &str) -> Vec<UpdateCandidate> {
    let mut candidates = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_version: Option<String> = None;
    let mut update_version: Option<String> = None;

    for line in stdout.lines() {
        let trimmed = line.trim().trim_start_matches(['├', '─', '│', '└', ' ']);

        if !trimmed.contains(':') && !trimmed.is_empty() && !trimmed.starts_with("Devices") {
            if let (Some(name), Some(ver)) = (&current_name, &update_version) {
                candidates.push(UpdateCandidate {
                    architecture: None,
                    name: name.clone(),
                    installed_version: current_version.clone(),
                    available_version: ver.clone(),
                    classification: UpdateClassification::Unknown,
                    download_bytes: None,
                    hold_state: HoldState::None,
                    backend: UpdateBackend::Fwupd,
                    is_new: false,
                    is_kernel: false,
                });
            }
            current_name = Some(trimmed.trim_end_matches(':').to_string());
            current_version = None;
            update_version = None;
            continue;
        }

        if let Some((key, value)) = trimmed.split_once(':') {
            let key = key.trim().to_lowercase();
            let value = value.trim();

            match key.as_str() {
                "current version" => current_version = Some(value.to_string()),
                "update version" | "new version" | "version" => {
                    if update_version.is_none() && current_version.is_some() {
                        update_version = Some(value.to_string());
                    } else if current_version.is_none() {
                        current_version = Some(value.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    if let (Some(name), Some(ver)) = (&current_name, &update_version) {
        candidates.push(UpdateCandidate {
            architecture: None,
            name: name.clone(),
            installed_version: current_version.clone(),
            available_version: ver.clone(),
            classification: UpdateClassification::Unknown,
            download_bytes: None,
            hold_state: HoldState::None,
            backend: UpdateBackend::Fwupd,
            is_new: false,
            is_kernel: false,
        });
    }

    candidates
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::{CommandOutput, CommandSpec};
    use crate::jobs::CancellationToken;

    struct MockRunner {
        output: CommandOutput,
    }

    impl CommandRunner for MockRunner {
        fn run(
            &self,
            _spec: &CommandSpec,
            _cancel: &dyn Cancellation,
        ) -> crate::error::Result<CommandOutput> {
            Ok(self.output.clone())
        }
    }

    #[test]
    fn fwupd_unavailable_when_not_installed() {
        struct FailRunner;
        impl CommandRunner for FailRunner {
            fn run(
                &self,
                _spec: &CommandSpec,
                _cancel: &dyn Cancellation,
            ) -> crate::error::Result<CommandOutput> {
                Err(crate::error::MaintainerError::io(
                    "spawn fwupdmgr",
                    std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
                ))
            }
        }

        let cancel = CancellationToken::new();
        let result = FwupdAdapter::check_available(&FailRunner, &cancel);
        assert!(matches!(result, Capability::Unavailable(_)));
        let reason = result.reason().unwrap();
        assert_eq!(reason.code, "fwupd-not-installed");
    }

    #[test]
    fn fwupd_available_when_working() {
        let runner = MockRunner {
            output: CommandOutput {
                exit_code: Some(0),
                stdout: "client version:\t1.9.5\n".to_string(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
        };
        let cancel = CancellationToken::new();
        let result = FwupdAdapter::check_available(&runner, &cancel);
        assert!(result.is_available());
    }

    #[test]
    fn fwupd_no_updates() {
        let runner = MockRunner {
            output: CommandOutput {
                exit_code: Some(2),
                stdout: "No updates available\n".to_string(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
        };
        let cancel = CancellationToken::new();
        let result = FwupdAdapter::list_updates(&runner, &cancel);
        match result {
            Capability::Available(updates) => assert!(updates.is_empty()),
            other => panic!("Expected Available([]), got {other:?}"),
        }
    }

    #[test]
    fn parse_fwupd_update_output() {
        let output = "\
Devices with pending updates:
├─UEFI dbx
│     Current version:    77
│     Update Version:     79
├─System Firmware
│     Current version:    1.18.0
│     Update Version:     1.19.0
";
        let candidates = parse_fwupd_updates(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].name, "UEFI dbx");
        assert_eq!(candidates[0].installed_version.as_deref(), Some("77"));
        assert_eq!(candidates[0].available_version, "79");
        assert_eq!(candidates[0].backend, UpdateBackend::Fwupd);
        assert_eq!(candidates[1].name, "System Firmware");
        assert_eq!(candidates[1].available_version, "1.19.0");
    }
}
