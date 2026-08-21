//! Optional Flatpak update adapter.
//!
//! Detects Flatpak availability and lists pending application/runtime updates.
//! Returns `Capability::Unavailable` with an explanation when Flatpak is absent.
//! This is not a required dependency for core APT updates.

use crate::capability::{Capability, UnavailableReason};
use crate::command::{CommandRunner, CommandSpec};
use crate::jobs::Cancellation;

use super::model::{HoldState, UpdateBackend, UpdateCandidate, UpdateClassification};

/// Flatpak update adapter.
pub struct FlatpakAdapter;

impl FlatpakAdapter {
    /// Check if Flatpak is available on this system.
    pub fn check_available(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<()> {
        let spec = match CommandSpec::new("flatpak", ["--version"]) {
            Ok(s) => s,
            Err(_) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "flatpak-not-installed",
                    message: "Flatpak is not installed on this system".to_string(),
                    remediation: Some("Install Flatpak: sudo apt install flatpak".to_string()),
                });
            }
        };

        match runner.run(&spec, cancel) {
            Ok(output) if output.success() => Capability::Available(()),
            Ok(_) => Capability::Unavailable(UnavailableReason {
                code: "flatpak-not-working",
                message: "Flatpak is installed but not functioning correctly".to_string(),
                remediation: Some("Try reinstalling Flatpak".to_string()),
            }),
            Err(_) => Capability::Unavailable(UnavailableReason {
                code: "flatpak-not-installed",
                message: "Flatpak is not available".to_string(),
                remediation: Some("Install Flatpak: sudo apt install flatpak".to_string()),
            }),
        }
    }

    /// List pending Flatpak updates.
    ///
    /// This is a read-only check and does not download or install anything.
    pub fn list_updates(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<Vec<UpdateCandidate>> {
        let spec = match CommandSpec::new(
            "flatpak",
            [
                "remote-ls",
                "--updates",
                "--columns=application,branch,download-size",
            ],
        ) {
            Ok(s) => s,
            Err(_) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "flatpak-not-installed",
                    message: "Flatpak is not available".to_string(),
                    remediation: Some("Install Flatpak: sudo apt install flatpak".to_string()),
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
                    code: "flatpak-check-failed",
                    message: "Failed to check for Flatpak updates".to_string(),
                    remediation: None,
                });
            }
        };

        if !output.success() {
            return Capability::Unavailable(UnavailableReason {
                code: "flatpak-check-failed",
                message: "Flatpak update check returned an error".to_string(),
                remediation: None,
            });
        }

        let candidates = parse_flatpak_updates(&output.stdout);
        Capability::Available(candidates)
    }
}

/// Parse Flatpak remote-ls --updates output into candidates.
fn parse_flatpak_updates(stdout: &str) -> Vec<UpdateCandidate> {
    let mut candidates = Vec::new();

    for line in stdout.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let parts: Vec<&str> = line.split('\t').collect();
        if parts.is_empty() {
            continue;
        }

        let name = parts[0].to_string();
        let available_version = parts.get(1).unwrap_or(&"").to_string();
        let download_bytes = parts.get(2).and_then(|s| parse_flatpak_size(s));

        candidates.push(UpdateCandidate {
            name,
            installed_version: None,
            available_version,
            classification: UpdateClassification::Unknown,
            download_bytes,
            hold_state: HoldState::None,
            backend: UpdateBackend::Flatpak,
            is_new: false,
        });
    }

    candidates
}

/// Parse Flatpak size strings (e.g., "45.2 MB", "1.1 kB").
fn parse_flatpak_size(s: &str) -> Option<u64> {
    let s = s.trim();
    let (number_str, unit) = s.rsplit_once(' ')?;
    let number: f64 = number_str.parse().ok()?;

    let multiplier: f64 = match unit {
        "B" | "bytes" => 1.0,
        "kB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        _ => return None,
    };

    Some((number * multiplier) as u64)
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
    fn flatpak_unavailable_when_not_installed() {
        struct FailRunner;
        impl CommandRunner for FailRunner {
            fn run(
                &self,
                _spec: &CommandSpec,
                _cancel: &dyn Cancellation,
            ) -> crate::error::Result<CommandOutput> {
                Err(crate::error::MaintainerError::io(
                    "spawn flatpak",
                    std::io::Error::new(std::io::ErrorKind::NotFound, "not found"),
                ))
            }
        }

        let cancel = CancellationToken::new();
        let result = FlatpakAdapter::check_available(&FailRunner, &cancel);
        assert!(matches!(result, Capability::Unavailable(_)));
    }

    #[test]
    fn flatpak_available_when_working() {
        let runner = MockRunner {
            output: CommandOutput {
                exit_code: Some(0),
                stdout: "Flatpak 1.14.4\n".to_string(),
                stderr: String::new(),
                stdout_truncated: false,
                stderr_truncated: false,
            },
        };
        let cancel = CancellationToken::new();
        let result = FlatpakAdapter::check_available(&runner, &cancel);
        assert!(result.is_available());
    }

    #[test]
    fn parse_flatpak_update_list() {
        let output = "org.mozilla.firefox\tstable\t45.2 MB\norg.gimp.GIMP\tstable\t120.5 MB\n";
        let candidates = parse_flatpak_updates(output);
        assert_eq!(candidates.len(), 2);
        assert_eq!(candidates[0].name, "org.mozilla.firefox");
        assert_eq!(candidates[0].backend, UpdateBackend::Flatpak);
        assert_eq!(candidates[0].download_bytes, Some(45_200_000));
        assert_eq!(candidates[1].name, "org.gimp.GIMP");
    }

    #[test]
    fn parse_flatpak_empty_output() {
        let candidates = parse_flatpak_updates("");
        assert!(candidates.is_empty());
    }

    #[test]
    fn flatpak_size_parsing() {
        assert_eq!(parse_flatpak_size("45.2 MB"), Some(45_200_000));
        assert_eq!(parse_flatpak_size("1.1 kB"), Some(1_100));
        assert_eq!(parse_flatpak_size("2.5 GB"), Some(2_500_000_000));
        assert_eq!(parse_flatpak_size("invalid"), None);
    }
}
