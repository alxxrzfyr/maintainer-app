//! Read-only APT update discovery through `apt-get` simulation.
//!
//! Uses `apt-get --simulate upgrade` with `LC_ALL=C` to parse available
//! updates without mutating package state. Never runs through a shell.

use crate::capability::{Capability, UnavailableReason};
use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::jobs::Cancellation;

use super::model::{
    DiscoveryError, DiscoveryResult, HoldState, RemovalCandidate, UpdateBackend, UpdateCandidate,
    UpdateClassification, UpdatePlan,
};

/// APT update discovery adapter.
pub struct AptDiscovery;

impl AptDiscovery {
    /// Check for available APT updates using simulation mode.
    ///
    /// This is a read-only operation: `apt-get --simulate` never downloads
    /// or installs packages.
    pub fn check_updates(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<DiscoveryResult> {
        let spec =
            match CommandSpec::new("apt-get", ["--simulate", "--quiet", "--quiet", "upgrade"]) {
                Ok(s) => s,
                Err(e) => {
                    return Capability::Unavailable(UnavailableReason {
                        code: "apt-unavailable",
                        message: format!("Cannot construct apt-get command: {e}"),
                        remediation: Some("Ensure apt-get is installed and accessible".to_string()),
                    });
                }
            };

        let output = match runner.run(&spec, cancel) {
            Ok(o) => o,
            Err(crate::error::MaintainerError::Cancelled) => {
                return Capability::Available(DiscoveryResult::Failed(DiscoveryError::Cancelled));
            }
            Err(e) => {
                return Capability::Available(DiscoveryResult::Failed(
                    DiscoveryError::CommandFailed {
                        message: e.to_string(),
                    },
                ));
            }
        };

        // Check for dpkg lock errors in stderr.
        if output.stderr.contains("Could not get lock") {
            return Capability::Available(DiscoveryResult::Failed(DiscoveryError::DpkgLocked));
        }

        // Check for repository offline errors.
        if output.stderr.contains("Could not resolve") || output.stderr.contains("Failed to fetch")
        {
            return Capability::Available(DiscoveryResult::Failed(
                DiscoveryError::RepositoryOffline {
                    details: first_error_line(&output.stderr),
                },
            ));
        }

        // Non-zero exit without known lock/network issue.
        if !output.success() && output.exit_code != Some(0) {
            return Capability::Available(DiscoveryResult::Failed(DiscoveryError::CommandFailed {
                message: first_error_line(&output.stderr),
            }));
        }

        let parse_result = parse_apt_simulate(&output);
        Capability::Available(parse_result)
    }

    /// Discover available updates using dist-upgrade mode (includes held packages).
    pub fn check_full_upgrade(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
    ) -> Capability<DiscoveryResult> {
        let spec = match CommandSpec::new(
            "apt-get",
            ["--simulate", "--quiet", "--quiet", "dist-upgrade"],
        ) {
            Ok(s) => s,
            Err(e) => {
                return Capability::Unavailable(UnavailableReason {
                    code: "apt-unavailable",
                    message: format!("Cannot construct apt-get command: {e}"),
                    remediation: Some("Ensure apt-get is installed and accessible".to_string()),
                });
            }
        };

        let output = match runner.run(&spec, cancel) {
            Ok(o) => o,
            Err(crate::error::MaintainerError::Cancelled) => {
                return Capability::Available(DiscoveryResult::Failed(DiscoveryError::Cancelled));
            }
            Err(e) => {
                return Capability::Available(DiscoveryResult::Failed(
                    DiscoveryError::CommandFailed {
                        message: e.to_string(),
                    },
                ));
            }
        };

        if output.stderr.contains("Could not get lock") {
            return Capability::Available(DiscoveryResult::Failed(DiscoveryError::DpkgLocked));
        }

        if output.stderr.contains("Could not resolve") || output.stderr.contains("Failed to fetch")
        {
            return Capability::Available(DiscoveryResult::Failed(
                DiscoveryError::RepositoryOffline {
                    details: first_error_line(&output.stderr),
                },
            ));
        }

        if !output.success() && output.exit_code != Some(0) {
            return Capability::Available(DiscoveryResult::Failed(DiscoveryError::CommandFailed {
                message: first_error_line(&output.stderr),
            }));
        }

        let parse_result = parse_apt_simulate(&output);
        Capability::Available(parse_result)
    }
}

/// Parse `apt-get --simulate` output into a `DiscoveryResult`.
pub fn parse_apt_simulate(output: &CommandOutput) -> DiscoveryResult {
    let mut candidates = Vec::new();
    let mut removals = Vec::new();
    let mut held_names: Vec<String> = Vec::new();
    let mut download_bytes: Option<u64> = None;
    let new_packages: Vec<String> = Vec::new();

    for line in output.stdout.lines() {
        // "The following packages have been kept back:" section.
        if line.starts_with("The following packages have been kept back:") {
            // Next lines until an empty line or other section are held packages.
            continue;
        }

        // Parse "Need to get X of archives." line.
        if line.starts_with("Need to get ") {
            download_bytes = parse_download_size(line);
            continue;
        }

        // Track new package names from "The following NEW packages will be installed:" section.
        if line.starts_with("The following NEW packages will be installed:") {
            continue;
        }

        // Inst lines: "Inst <name> [<installed>] (<available> <origin> [<arch>])"
        if line.starts_with("Inst ") {
            if let Some(candidate) = parse_inst_line(line, &new_packages) {
                candidates.push(candidate);
            }
            continue;
        }

        // Remv lines: "Remv <name> [<version>]"
        if line.starts_with("Remv ") {
            if let Some(removal) = parse_remv_line(line) {
                removals.push(removal);
            }
            continue;
        }

        // Lines that look like indented package lists (after "kept back" or "NEW").
        let trimmed = line.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("Reading")
            && !trimmed.starts_with("Building")
            && !trimmed.starts_with("The following")
            && !trimmed.contains(" upgraded,")
            && !trimmed.starts_with("Conf ")
            && !trimmed.starts_with("Need to get")
        {
            // Could be continuation of "kept back" or "NEW" list.
            // Check if previous significant line was "kept back".
            // We use a heuristic: if these look like package names (no special chars).
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.iter().all(|p| is_package_name(p)) {
                // We'll collect these as potential held/new names.
                // The actual classification happens when we see Inst/Remv lines.
                for part in &parts {
                    held_names.push((*part).to_string());
                }
            }
        }
    }

    // Parse held packages from the summary line "X not upgraded".
    let held: Vec<UpdateCandidate> = held_names
        .iter()
        .filter(|name| !candidates.iter().any(|c| &c.name == *name))
        .map(|name| UpdateCandidate {
            name: name.clone(),
            installed_version: None,
            available_version: String::new(),
            classification: UpdateClassification::Unknown,
            download_bytes: None,
            hold_state: HoldState::Held,
            backend: UpdateBackend::Apt,
            is_new: false,
        })
        .collect();

    // Mark new packages.
    for candidate in &mut candidates {
        if new_packages.contains(&candidate.name) {
            candidate.is_new = true;
        }
    }

    if candidates.is_empty() && removals.is_empty() && held.is_empty() {
        return DiscoveryResult::UpToDate;
    }

    let content_hash = compute_plan_hash(&candidates, &removals);
    let plan = UpdatePlan {
        plan_id: uuid::Uuid::new_v4().to_string(),
        created_at: std::time::SystemTime::now(),
        candidates,
        removals,
        held,
        total_download_bytes: download_bytes,
        content_hash,
    };

    DiscoveryResult::Available(plan)
}

/// Parse an Inst line into an `UpdateCandidate`.
///
/// Format: `Inst <name> [<installed>] (<available> <origin> [<arch>])`
/// Or new: `Inst <name> (<available> <origin> [<arch>])`
fn parse_inst_line(line: &str, _new_packages: &[String]) -> Option<UpdateCandidate> {
    // Skip "Inst "
    let rest = line.strip_prefix("Inst ")?;

    // Split at the first space to get package name.
    let (name, remainder) = rest.split_once(' ')?;
    let name = name.trim();

    if name.is_empty() {
        return None;
    }

    // Parse installed version (in square brackets).
    let (installed_version, after_installed) = if remainder.starts_with('[') {
        if let Some(end) = remainder.find(']') {
            let ver = remainder[1..end].to_string();
            (Some(ver), remainder[end + 1..].trim())
        } else {
            // Malformed: opening bracket without close.
            return None;
        }
    } else {
        (None, remainder)
    };

    // Parse available version and origin (in parentheses).
    let paren_content = if after_installed.starts_with('(') {
        if let Some(end) = after_installed.rfind(')') {
            &after_installed[1..end]
        } else {
            return None;
        }
    } else {
        return None;
    };

    // Paren content: "<version> <origin> [<arch>]"
    let mut parts = paren_content.split_whitespace();
    let available_version = parts.next()?.to_string();
    let origin = parts.next().unwrap_or("");

    // Classify based on origin containing "security".
    let classification = classify_origin(origin);

    let is_new = installed_version.is_none();

    Some(UpdateCandidate {
        name: name.to_string(),
        installed_version,
        available_version,
        classification,
        download_bytes: None,
        hold_state: HoldState::None,
        backend: UpdateBackend::Apt,
        is_new,
    })
}

/// Parse a Remv line into a `RemovalCandidate`.
///
/// Format: `Remv <name> [<version>]`
fn parse_remv_line(line: &str) -> Option<RemovalCandidate> {
    let rest = line.strip_prefix("Remv ")?;
    let (name, remainder) = rest.split_once(' ')?;

    let installed_version = if remainder.starts_with('[') {
        if let Some(end) = remainder.find(']') {
            remainder[1..end].to_string()
        } else {
            remainder.trim().to_string()
        }
    } else {
        remainder.trim().to_string()
    };

    Some(RemovalCandidate {
        name: name.to_string(),
        installed_version,
    })
}

/// Classify an update based on its repository origin string.
fn classify_origin(origin: &str) -> UpdateClassification {
    if origin.contains("-security") || origin.contains("/security") {
        UpdateClassification::Security
    } else if origin.contains("Ubuntu:")
        || origin.contains("Debian:")
        || origin.contains("-updates")
    {
        UpdateClassification::Standard
    } else if origin.is_empty() {
        UpdateClassification::Unknown
    } else {
        UpdateClassification::Standard
    }
}

/// Parse download size from "Need to get X of archives." line.
///
/// Handles: "85.3 MB", "2,456 kB", "1.2 GB", etc.
fn parse_download_size(line: &str) -> Option<u64> {
    // Extract the size portion after "Need to get ".
    let after = line.strip_prefix("Need to get ")?;
    let before_of = after.split(" of ").next()?;

    // Split into number and unit.
    let trimmed = before_of.trim();
    let (number_str, unit) = trimmed.rsplit_once(' ')?;

    // Remove thousands separators.
    let clean_number: String = number_str.chars().filter(|c| *c != ',').collect();
    let number: f64 = clean_number.parse().ok()?;

    let multiplier: f64 = match unit {
        "B" | "bytes" => 1.0,
        "kB" => 1_000.0,
        "MB" => 1_000_000.0,
        "GB" => 1_000_000_000.0,
        _ => return None,
    };

    Some((number * multiplier) as u64)
}

/// Compute a deterministic hash of the plan contents for staleness detection.
fn compute_plan_hash(candidates: &[UpdateCandidate], removals: &[RemovalCandidate]) -> String {
    use std::fmt::Write;
    let mut data = String::new();
    for c in candidates {
        let _ = write!(data, "{}:{}:{};", c.name, c.available_version, c.is_new);
    }
    for r in removals {
        let _ = write!(data, "-{}:{};", r.name, r.installed_version);
    }
    let hash = blake3::hash(data.as_bytes());
    hash.to_hex().to_string()
}

/// Extract the first meaningful error line from stderr.
fn first_error_line(stderr: &str) -> String {
    stderr
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("Unknown error")
        .to_string()
}

/// Simple heuristic: a package name contains only alphanumeric, dash, dot, plus.
fn is_package_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '.' || c == '+')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_output(stdout: &str, stderr: &str, exit_code: i32) -> CommandOutput {
        CommandOutput {
            exit_code: Some(exit_code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    #[test]
    fn parse_no_updates() {
        let stdout = include_str!("../../../../fixtures/apt/no_updates.txt");
        let output = make_output(stdout, "", 0);
        let result = parse_apt_simulate(&output);
        assert!(matches!(result, DiscoveryResult::UpToDate));
    }

    #[test]
    fn parse_security_updates() {
        let stdout = include_str!("../../../../fixtures/apt/security_updates.txt");
        let output = make_output(stdout, "", 0);
        let result = parse_apt_simulate(&output);

        let plan = match result {
            DiscoveryResult::Available(p) => p,
            other => panic!("Expected Available, got {other:?}"),
        };

        assert_eq!(plan.candidates.len(), 5);
        assert!(plan.total_download_bytes.is_some());
        assert_eq!(plan.total_download_bytes.unwrap(), 85_300_000);

        // Verify security classification for curl.
        let curl = plan.candidates.iter().find(|c| c.name == "curl").unwrap();
        assert_eq!(curl.classification, UpdateClassification::Security);
        assert_eq!(
            curl.installed_version.as_deref(),
            Some("7.81.0-1ubuntu1.15")
        );
        assert_eq!(curl.available_version, "7.81.0-1ubuntu1.16");

        // linux-image-generic is from jammy-updates (standard).
        let kernel = plan
            .candidates
            .iter()
            .find(|c| c.name == "linux-image-generic")
            .unwrap();
        assert_eq!(kernel.classification, UpdateClassification::Standard);
    }

    #[test]
    fn parse_held_packages() {
        let stdout = include_str!("../../../../fixtures/apt/held_packages.txt");
        let output = make_output(stdout, "", 0);
        let result = parse_apt_simulate(&output);

        let plan = match result {
            DiscoveryResult::Available(p) => p,
            other => panic!("Expected Available, got {other:?}"),
        };

        // 2 upgradable packages.
        assert_eq!(plan.candidates.len(), 2);
        // firefox and snapd are held.
        assert_eq!(plan.held.len(), 2);
        assert!(plan.held.iter().any(|h| h.name == "firefox"));
        assert!(plan.held.iter().any(|h| h.name == "snapd"));
    }

    #[test]
    fn parse_with_removals() {
        let stdout = include_str!("../../../../fixtures/apt/with_removals.txt");
        let output = make_output(stdout, "", 0);
        let result = parse_apt_simulate(&output);

        let plan = match result {
            DiscoveryResult::Available(p) => p,
            other => panic!("Expected Available, got {other:?}"),
        };

        assert!(plan.has_removals());
        assert_eq!(plan.removals.len(), 2);
        assert!(plan.removals.iter().any(|r| r.name == "old-kernel-headers"));
        assert!(plan.removals.iter().any(|r| r.name == "old-kernel-image"));

        // new-dep-lib is newly installed.
        let new_dep = plan
            .candidates
            .iter()
            .find(|c| c.name == "new-dep-lib")
            .unwrap();
        assert!(new_dep.is_new);
        assert!(new_dep.installed_version.is_none());
    }

    #[test]
    fn parse_dpkg_lock_error() {
        let stderr = include_str!("../../../../fixtures/apt/dpkg_lock.txt");
        let output = make_output("", stderr, 100);

        // Simulate full check path.
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

        let runner = MockRunner { output };
        let cancel = crate::jobs::CancellationToken::new();
        let result = AptDiscovery::check_updates(&runner, &cancel);

        match result {
            Capability::Available(DiscoveryResult::Failed(DiscoveryError::DpkgLocked)) => {}
            other => panic!("Expected DpkgLocked, got {other:?}"),
        }
    }

    #[test]
    fn parse_offline_repository() {
        let stderr = include_str!("../../../../fixtures/apt/offline_repo.txt");
        let output = make_output("", stderr, 100);

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

        let runner = MockRunner { output };
        let cancel = crate::jobs::CancellationToken::new();
        let result = AptDiscovery::check_updates(&runner, &cancel);

        match result {
            Capability::Available(DiscoveryResult::Failed(DiscoveryError::RepositoryOffline {
                ..
            })) => {}
            other => panic!("Expected RepositoryOffline, got {other:?}"),
        }
    }

    #[test]
    fn parse_malformed_output_graceful() {
        let stdout = include_str!("../../../../fixtures/apt/malformed.txt");
        let output = make_output(stdout, "", 0);
        let result = parse_apt_simulate(&output);

        // Should still parse the valid line without crashing.
        match result {
            DiscoveryResult::Available(plan) => {
                assert_eq!(plan.candidates.len(), 1);
                assert_eq!(plan.candidates[0].name, "valid-pkg");
            }
            other => panic!("Expected Available with partial parse, got {other:?}"),
        }
    }

    #[test]
    fn parse_download_size_megabytes() {
        let size = parse_download_size("Need to get 85.3 MB of archives.");
        assert_eq!(size, Some(85_300_000));
    }

    #[test]
    fn parse_download_size_kilobytes_with_comma() {
        let size = parse_download_size("Need to get 2,456 kB of archives.");
        assert_eq!(size, Some(2_456_000));
    }

    #[test]
    fn parse_download_size_gigabytes() {
        let size = parse_download_size("Need to get 1.2 GB of archives.");
        assert_eq!(size, Some(1_200_000_000));
    }

    #[test]
    fn classify_security_origin() {
        assert_eq!(
            classify_origin("Ubuntu:22.04/jammy-security"),
            UpdateClassification::Security
        );
    }

    #[test]
    fn classify_standard_origin() {
        assert_eq!(
            classify_origin("Ubuntu:22.04/jammy-updates"),
            UpdateClassification::Standard
        );
    }

    #[test]
    fn classify_unknown_origin() {
        assert_eq!(classify_origin(""), UpdateClassification::Unknown);
    }

    #[test]
    fn cancellation_before_command() {
        struct NeverRunner;
        impl CommandRunner for NeverRunner {
            fn run(
                &self,
                _spec: &CommandSpec,
                _cancel: &dyn Cancellation,
            ) -> crate::error::Result<CommandOutput> {
                Err(crate::error::MaintainerError::Cancelled)
            }
        }

        let runner = NeverRunner;
        let cancel = crate::jobs::CancellationToken::new();
        cancel.cancel();
        let result = AptDiscovery::check_updates(&runner, &cancel);

        match result {
            Capability::Available(DiscoveryResult::Failed(DiscoveryError::Cancelled)) => {}
            other => panic!("Expected Cancelled, got {other:?}"),
        }
    }

    #[test]
    fn plan_immutability_content_hash() {
        let stdout = include_str!("../../../../fixtures/apt/security_updates.txt");
        let output = make_output(stdout, "", 0);
        let result1 = parse_apt_simulate(&output);
        let result2 = parse_apt_simulate(&output);

        let plan1 = match result1 {
            DiscoveryResult::Available(p) => p,
            _ => panic!("Expected Available"),
        };
        let plan2 = match result2 {
            DiscoveryResult::Available(p) => p,
            _ => panic!("Expected Available"),
        };

        // Content hash should be identical for the same input.
        assert_eq!(plan1.content_hash, plan2.content_hash);
        // Plan IDs should differ (UUID).
        assert_ne!(plan1.plan_id, plan2.plan_id);
    }
}
