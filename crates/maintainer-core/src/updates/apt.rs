//! Read-only APT update discovery through `apt-get` simulation.
//!
//! Uses `apt-get --simulate upgrade` with `LC_ALL=C` to parse available
//! updates without mutating package state. Never runs through a shell.

use crate::capability::{Capability, UnavailableReason};
use crate::command::{CommandOutput, CommandRunner, CommandSpec};
use crate::jobs::Cancellation;

use std::path::Path;

use super::model::{
    DiscoveryError, DiscoveryResult, HoldState, RemovalCandidate, UpdateBackend, UpdateCandidate,
    UpdateClassification, UpdateIntent, UpdatePlan, UpdateRequest,
};

/// Detect whether a package name refers to a Linux kernel package.
///
/// Matches package names that start with `linux-image-`, `linux-headers-`,
/// `linux-modules-`, or equal `linux-generic`, `linux-image-generic`,
/// `linux-headers-generic`, or `linux-modules-generic`.
///
/// `linux-libc-dev` is intentionally excluded: it is a userspace C library
/// header package derived from kernel sources but does not install a kernel
/// or kernel modules, and does not require a reboot.
fn is_kernel_package(name: &str) -> bool {
    // Meta-packages that always track the latest kernel.
    const KERNEL_META: &[&str] = &[
        "linux-generic",
        "linux-image-generic",
        "linux-headers-generic",
        "linux-modules-generic",
    ];

    if KERNEL_META.contains(&name) {
        return true;
    }

    name.starts_with("linux-image-")
        || name.starts_with("linux-headers-")
        || name.starts_with("linux-modules-")
}

/// Check whether the system currently requires a reboot.
///
/// On Debian/Ubuntu/Mint, the file `/var/run/reboot-required` is created by
/// package postinst scripts (typically after kernel or libc upgrades) to signal
/// that the running kernel or critical libraries are stale. The file's content
/// is irrelevant; only its existence matters.
///
/// The `run_path` parameter allows tests to inject a temporary directory instead
/// of reading from the real `/var/run`.
pub fn reboot_required(run_path: &Path) -> bool {
    run_path.join("reboot-required").exists()
}

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

impl AptDiscovery {
    /// Re-plan for a user-selected subset of update candidates.
    ///
    /// Instead of running `apt-get upgrade` (which upgrades everything), this
    /// invokes `apt-get --simulate install pkg1=ver1 pkg2=ver2 ...` to ask apt's
    /// own dependency resolver what would happen if only the selected packages
    /// were requested. This is the ONLY safe way to implement per-package
    /// deselection: we must never fabricate a filtered plan client-side, because
    /// the remaining packages may have version constraints or dependencies that
    /// involved deselected ones.
    ///
    /// Each candidate is pinned to its `available_version` so apt cannot silently
    /// resolve to a newer version that appeared between the original discovery
    /// and this re-plan call.
    ///
    /// Returns the same `Capability<DiscoveryResult>` shape as `check_updates`,
    /// reusing the identical simulation-output parser.
    pub fn plan_for_selection(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
        selected: &[UpdateCandidate],
    ) -> Capability<DiscoveryResult> {
        let requests = selected
            .iter()
            .map(|candidate| UpdateRequest {
                architecture: candidate.architecture.clone(),
                name: candidate.name.clone(),
                version: candidate.available_version.clone(),
            })
            .collect::<Vec<_>>();
        Self::resolve_selection(runner, cancel, &requests)
    }

    /// Revalidate with the same resolver and original requests used for review.
    pub fn revalidate_plan(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
        plan: &UpdatePlan,
    ) -> Capability<DiscoveryResult> {
        match &plan.intent {
            UpdateIntent::FullUpgrade => Self::check_full_upgrade(runner, cancel),
            UpdateIntent::Selected(requests) => Self::resolve_selection(runner, cancel, requests),
        }
    }

    fn resolve_selection(
        runner: &dyn CommandRunner,
        cancel: &dyn Cancellation,
        selected: &[UpdateRequest],
    ) -> Capability<DiscoveryResult> {
        // Empty selection is a no-op: no packages to install means the system
        // is effectively up-to-date from the user's perspective.
        if selected.is_empty() {
            return Capability::Available(DiscoveryResult::UpToDate);
        }

        // Build version-pinned arguments: "name=available_version" for each
        // selected candidate. The version pin ensures apt resolves against
        // exactly the versions the user reviewed, not newer ones that may
        // have appeared since the original discovery.
        let pinned_args: Vec<String> = selected.iter().map(UpdateRequest::package_spec).collect();

        let mut args = vec![
            "--simulate".to_string(),
            "--quiet".to_string(),
            "--quiet".to_string(),
            "install".to_string(),
        ];
        args.extend(pinned_args);

        let spec = match CommandSpec::new("apt-get", args) {
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

        let mut parse_result = parse_apt_simulate(&output);
        if let DiscoveryResult::Available(plan) = &mut parse_result {
            plan.intent = UpdateIntent::Selected(selected.to_vec());
        }
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
        if line.starts_with("The following packages have been kept back:") {
            continue;
        }

        if line.starts_with("Need to get ") {
            download_bytes = parse_download_size(line);
            continue;
        }

        if line.starts_with("The following NEW packages will be installed:") {
            continue;
        }

        if line.starts_with("Inst ") {
            if let Some(candidate) = parse_inst_line(line, &new_packages) {
                candidates.push(candidate);
            }
            continue;
        }

        if line.starts_with("Remv ") {
            if let Some(removal) = parse_remv_line(line) {
                removals.push(removal);
            }
            continue;
        }

        let trimmed = line.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("Reading")
            && !trimmed.starts_with("Building")
            && !trimmed.starts_with("The following")
            && !trimmed.contains(" upgraded,")
            && !trimmed.starts_with("Conf ")
            && !trimmed.starts_with("Need to get")
        {
            let parts: Vec<&str> = trimmed.split_whitespace().collect();
            if parts.iter().all(|p| is_package_name(p)) {
                for part in &parts {
                    held_names.push((*part).to_string());
                }
            }
        }
    }

    let held: Vec<UpdateCandidate> = held_names
        .iter()
        .filter(|name| !candidates.iter().any(|c| &c.name == *name))
        .map(|name| UpdateCandidate {
            architecture: None,
            name: name.clone(),
            installed_version: None,
            available_version: String::new(),
            classification: UpdateClassification::Unknown,
            download_bytes: None,
            hold_state: HoldState::Held,
            backend: UpdateBackend::Apt,
            is_new: false,
            is_kernel: is_kernel_package(name),
        })
        .collect();

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
        intent: UpdateIntent::FullUpgrade,
        plan_id: uuid::Uuid::new_v4().to_string(),
        created_at: std::time::SystemTime::now(),
        candidates,
        removals,
        held,
        total_download_bytes: download_bytes,
        content_hash,
        reboot_required: reboot_required(Path::new("/var/run")),
    };

    DiscoveryResult::Available(plan)
}

/// Parse an Inst line into an `UpdateCandidate`.
///
/// Format: `Inst <name> [<installed>] (<available> <origin> [<arch>])`
/// Or new: `Inst <name> (<available> <origin> [<arch>])`
fn parse_inst_line(line: &str, _new_packages: &[String]) -> Option<UpdateCandidate> {
    let rest = line.strip_prefix("Inst ")?;

    let (name, remainder) = rest.split_once(' ')?;
    let name = name.trim();

    if name.is_empty() {
        return None;
    }

    let (installed_version, after_installed) = if remainder.starts_with('[') {
        if let Some(end) = remainder.find(']') {
            let ver = remainder[1..end].to_string();
            (Some(ver), remainder[end + 1..].trim())
        } else {
            return None;
        }
    } else {
        (None, remainder)
    };

    let paren_content = if after_installed.starts_with('(') {
        if let Some(end) = after_installed.rfind(')') {
            &after_installed[1..end]
        } else {
            return None;
        }
    } else {
        return None;
    };

    let mut parts = paren_content.split_whitespace();
    let available_version = parts.next()?.to_string();
    let origin = parts.next().unwrap_or("");

    // Classify based on origin containing "security".
    let classification = classify_origin(origin);

    let architecture = paren_content
        .split_whitespace()
        .last()
        .and_then(|value| value.strip_prefix('[')?.strip_suffix(']'))
        .filter(|value| {
            !value.is_empty()
                && value
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'-')
        })
        .or_else(|| name.split_once(':').map(|(_, arch)| arch))
        .map(str::to_string);
    if let (Some((_, qualified)), Some(architecture)) =
        (name.split_once(':'), architecture.as_deref())
    {
        if qualified != architecture {
            return None;
        }
    }
    let is_new = installed_version.is_none();

    Some(UpdateCandidate {
        architecture,
        name: name.to_string(),
        installed_version,
        available_version,
        classification,
        download_bytes: None,
        hold_state: HoldState::None,
        backend: UpdateBackend::Apt,
        is_new,
        is_kernel: is_kernel_package(name),
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
    let lower = origin.to_ascii_lowercase();

    if lower.contains("-security") || lower.contains("/security") {
        UpdateClassification::Security
    } else if origin.contains("Ubuntu:")
        || origin.contains("Debian:")
        || origin.contains("-updates")
        // Linux Mint native packages report origins like "o=linuxmint" or
        // "linuxmint/mint-repository". Classify as Standard explicitly.
        || lower.contains("linuxmint")
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
    let after = line.strip_prefix("Need to get ")?;
    let before_of = after.split(" of ").next()?;

    let trimmed = before_of.trim();
    let (number_str, unit) = trimmed.rsplit_once(' ')?;

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
        let _ = write!(
            data,
            "{}:{}:{}:{};",
            c.name,
            c.architecture.as_deref().unwrap_or("?"),
            c.available_version,
            c.is_new
        );
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
    fn architecture_is_preserved_in_selection_resolution_and_plan_hash() {
        let output = make_output("Inst libfoo [1.0] (2.0 Ubuntu:stable [i386])\n", "", 0);
        let parsed = parse_apt_simulate(&output);
        let DiscoveryResult::Available(plan) = parsed else {
            panic!("missing plan");
        };
        assert_eq!(plan.candidates[0].architecture.as_deref(), Some("i386"));
        let mut other = plan.candidates.clone();
        other[0].architecture = Some("amd64".into());
        assert_ne!(
            compute_plan_hash(&plan.candidates, &[]),
            compute_plan_hash(&other, &[])
        );
        let runner = CapturingMockRunner::new(output);
        let result = AptDiscovery::plan_for_selection(
            &runner,
            &crate::jobs::CancellationToken::new(),
            &plan.candidates,
        );
        assert!(
            runner
                .captured_args()
                .contains(&"libfoo:i386=2.0".to_string())
        );
        let Capability::Available(DiscoveryResult::Available(selected)) = result else {
            panic!("missing selected plan");
        };
        let UpdateIntent::Selected(requests) = selected.intent else {
            panic!("missing intent");
        };
        assert_eq!(requests[0].architecture.as_deref(), Some("i386"));
        assert_eq!(requests[0].package_spec(), "libfoo:i386=2.0");
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

        assert_eq!(plan.candidates.len(), 2);
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
    fn classify_security_origin_case_insensitive() {
        // Debian uses "Debian-Security" with capital S and hyphen.
        assert_eq!(
            classify_origin("Debian-Security:12/stable-security"),
            UpdateClassification::Security
        );
        assert_eq!(
            classify_origin("o=Debian,l=Debian-Security"),
            UpdateClassification::Security
        );
    }

    #[test]
    fn classify_security_origin_lowercase_variants() {
        // Standard lowercase forms must still match.
        assert_eq!(
            classify_origin("Ubuntu:22.04/jammy-security"),
            UpdateClassification::Security
        );
        assert_eq!(
            classify_origin("archive.ubuntu.com/security"),
            UpdateClassification::Security
        );
    }

    #[test]
    fn classify_linuxmint_origin_standard() {
        // Linux Mint native packages use origins like "o=linuxmint".
        assert_eq!(
            classify_origin("o=linuxmint,a=wilma"),
            UpdateClassification::Standard
        );
        assert_eq!(
            classify_origin("linuxmint/mint-repository"),
            UpdateClassification::Standard
        );
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

        assert_eq!(plan1.content_hash, plan2.content_hash);
        assert_ne!(plan1.plan_id, plan2.plan_id);
    }

    #[test]
    fn kernel_package_detection_true_cases() {
        // Versioned kernel image packages.
        assert!(is_kernel_package("linux-image-6.8.0-40-generic"));
        assert!(is_kernel_package("linux-image-5.15.0-101-generic"));
        // Meta-packages.
        assert!(is_kernel_package("linux-image-generic"));
        assert!(is_kernel_package("linux-headers-generic"));
        assert!(is_kernel_package("linux-generic"));
        assert!(is_kernel_package("linux-modules-generic"));
        // Versioned headers and modules.
        assert!(is_kernel_package("linux-headers-6.8.0-40-generic"));
        assert!(is_kernel_package("linux-modules-6.8.0-40-generic"));
        assert!(is_kernel_package("linux-modules-extra-6.8.0-40-generic"));
    }

    #[test]
    fn kernel_package_detection_false_cases() {
        // linux-libc-dev is a userspace header package, not a bootable kernel.
        // It does not require a reboot and should not be flagged as kernel.
        assert!(!is_kernel_package("linux-libc-dev"));
        // Regular packages.
        assert!(!is_kernel_package("curl"));
        assert!(!is_kernel_package("libssl3"));
        assert!(!is_kernel_package("openssl"));
        // Packages that contain "linux" but are not kernel packages.
        assert!(!is_kernel_package("linux-base"));
        assert!(!is_kernel_package("linux-firmware"));
    }

    #[test]
    fn kernel_flag_set_in_parsed_output() {
        let stdout = include_str!("../../../../fixtures/apt/security_updates.txt");
        let output = make_output(stdout, "", 0);
        let result = parse_apt_simulate(&output);

        let plan = match result {
            DiscoveryResult::Available(p) => p,
            other => panic!("Expected Available, got {other:?}"),
        };

        let kernel = plan
            .candidates
            .iter()
            .find(|c| c.name == "linux-image-generic")
            .unwrap();
        assert!(kernel.is_kernel);

        let curl = plan.candidates.iter().find(|c| c.name == "curl").unwrap();
        assert!(!curl.is_kernel);
    }

    #[test]
    fn reboot_required_file_present() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("reboot-required"),
            "*** System restart required ***\n",
        )
        .unwrap();
        assert!(reboot_required(dir.path()));
    }

    #[test]
    fn reboot_required_file_absent() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!reboot_required(dir.path()));
    }

    #[test]
    fn reboot_required_empty_file() {
        // An empty file still signals reboot required (existence is the signal).
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("reboot-required"), "").unwrap();
        assert!(reboot_required(dir.path()));
    }

    // --- check_full_upgrade regression tests ---
    //
    // `dispatch_updates_check` in maintainer-app switched from `check_updates`
    // (apt-get upgrade) to `check_full_upgrade` (apt-get dist-upgrade) because
    // plain upgrade mode refuses anything that requires new packages or
    // dependency changes, which silently hid kernel updates tied to the HWE
    // metapackage chain on a real Mint/Ubuntu 24.04 system: apt-get upgrade
    // showed 7 candidates, apt-get dist-upgrade showed 16, and the missing 9
    // were exactly the kernel image/headers/modules/tools chain. These tests
    // guard the two properties that fix depends on.

    #[test]
    fn check_full_upgrade_invokes_dist_upgrade_not_upgrade() {
        let output = make_output("", "", 0);
        let runner = CapturingMockRunner::new(output);
        let cancel = crate::jobs::CancellationToken::new();

        let _ = AptDiscovery::check_full_upgrade(&runner, &cancel);

        let args = runner.captured_args();
        assert!(
            args.contains(&"dist-upgrade".to_string()),
            "expected dist-upgrade in args, got {args:?}"
        );
        assert!(
            !args.contains(&"upgrade".to_string()) || args.contains(&"dist-upgrade".to_string()),
            "must not silently fall back to plain upgrade"
        );
    }

    #[test]
    fn check_full_upgrade_surfaces_kernel_hidden_by_plain_upgrade() {
        // Fixture captured from a real Ubuntu 24.04 / Mint system: this exact
        // dist-upgrade output was NOT reproducible by running apt-get upgrade
        // on the same machine, because upgrade mode refuses the HWE kernel
        // metapackage chain's dependency changes.
        let stdout = include_str!("../../../../fixtures/apt/dist_upgrade_kernel.txt");
        let output = make_output(stdout, "", 0);
        let runner = CapturingMockRunner::new(output);
        let cancel = crate::jobs::CancellationToken::new();

        let result = AptDiscovery::check_full_upgrade(&runner, &cancel);

        let plan = match result {
            Capability::Available(DiscoveryResult::Available(p)) => p,
            other => panic!("Expected Available plan, got {other:?}"),
        };

        assert!(
            plan.candidates.iter().any(|c| c.is_kernel),
            "dist-upgrade output must surface at least one kernel candidate"
        );
        let kernel_names: Vec<&str> = plan
            .candidates
            .iter()
            .filter(|c| c.is_kernel)
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            kernel_names.contains(&"linux-image-7.0.0-30-generic"),
            "expected linux-image-7.0.0-30-generic among kernel candidates, got {kernel_names:?}"
        );
    }

    /// Mock runner that captures the command spec for assertion and returns
    /// a pre-configured output.
    struct CapturingMockRunner {
        output: CommandOutput,
        captured_spec: std::sync::Mutex<Option<(String, Vec<String>)>>,
    }

    impl CapturingMockRunner {
        fn new(output: CommandOutput) -> Self {
            Self {
                output,
                captured_spec: std::sync::Mutex::new(None),
            }
        }

        fn captured_args(&self) -> Vec<String> {
            self.captured_spec
                .lock()
                .unwrap()
                .as_ref()
                .map(|(_, args)| args.clone())
                .unwrap_or_default()
        }

        fn captured_program(&self) -> String {
            self.captured_spec
                .lock()
                .unwrap()
                .as_ref()
                .map(|(prog, _)| prog.clone())
                .unwrap_or_default()
        }
    }

    impl CommandRunner for CapturingMockRunner {
        fn run(
            &self,
            spec: &CommandSpec,
            _cancel: &dyn Cancellation,
        ) -> crate::error::Result<CommandOutput> {
            *self.captured_spec.lock().unwrap() =
                Some((spec.program().to_string(), spec.args().to_vec()));
            Ok(self.output.clone())
        }
    }

    fn make_test_candidate(name: &str, version: &str) -> UpdateCandidate {
        UpdateCandidate {
            architecture: None,
            name: name.to_string(),
            installed_version: Some("1.0.0".to_string()),
            available_version: version.to_string(),
            classification: UpdateClassification::Security,
            download_bytes: None,
            hold_state: HoldState::None,
            backend: UpdateBackend::Apt,
            is_new: false,
            is_kernel: false,
        }
    }

    #[test]
    fn plan_for_selection_empty_returns_up_to_date() {
        let output = make_output("", "", 0);
        let runner = CapturingMockRunner::new(output);
        let cancel = crate::jobs::CancellationToken::new();

        let result = AptDiscovery::plan_for_selection(&runner, &cancel, &[]);

        match result {
            Capability::Available(DiscoveryResult::UpToDate) => {}
            other => panic!("Expected UpToDate for empty selection, got {other:?}"),
        }
        // Verify runner was never called for empty selection.
        assert!(runner.captured_spec.lock().unwrap().is_none());
    }

    #[test]
    fn plan_for_selection_builds_install_command_with_pinned_versions() {
        let stdout = include_str!("../../../../fixtures/apt/selective_install.txt");
        let output = make_output(stdout, "", 0);
        let runner = CapturingMockRunner::new(output);
        let cancel = crate::jobs::CancellationToken::new();

        let selected = vec![
            make_test_candidate("curl", "7.81.0-1ubuntu1.16"),
            make_test_candidate("libcurl4", "7.81.0-1ubuntu1.16"),
        ];

        let result = AptDiscovery::plan_for_selection(&runner, &cancel, &selected);

        // Verify the command uses `install` (not `upgrade`) with version-pinned args.
        assert_eq!(runner.captured_program(), "apt-get");
        let args = runner.captured_args();
        assert!(args.contains(&"install".to_string()));
        assert!(!args.contains(&"upgrade".to_string()));
        assert!(args.contains(&"curl=7.81.0-1ubuntu1.16".to_string()));
        assert!(args.contains(&"libcurl4=7.81.0-1ubuntu1.16".to_string()));
        // Verify simulation flags are present.
        assert!(args.contains(&"--simulate".to_string()));
        assert_eq!(args.iter().filter(|a| *a == "--quiet").count(), 2);

        // Verify the result parses correctly.
        match result {
            Capability::Available(DiscoveryResult::Available(plan)) => {
                assert_eq!(plan.candidates.len(), 3);
            }
            other => panic!("Expected Available plan, got {other:?}"),
        }
    }

    #[test]
    fn plan_for_selection_discloses_additional_dependencies() {
        // Core correctness property: when apt pulls in additional packages
        // beyond what was selected (a dependency), those appear in the result.
        let stdout = include_str!("../../../../fixtures/apt/selective_install.txt");
        let output = make_output(stdout, "", 0);
        let runner = CapturingMockRunner::new(output);
        let cancel = crate::jobs::CancellationToken::new();

        // User selected only curl and libcurl4, but apt pulls in libgnutls30.
        let selected = vec![
            make_test_candidate("curl", "7.81.0-1ubuntu1.16"),
            make_test_candidate("libcurl4", "7.81.0-1ubuntu1.16"),
        ];

        let result = AptDiscovery::plan_for_selection(&runner, &cancel, &selected);

        let plan = match result {
            Capability::Available(DiscoveryResult::Available(p)) => p,
            other => panic!("Expected Available plan, got {other:?}"),
        };

        let original_args = runner.captured_args();
        let requests = match &plan.intent {
            UpdateIntent::Selected(requests) => requests,
            other => panic!("selection lost its intent: {other:?}"),
        };
        assert_eq!(requests.len(), 2);
        assert!(!requests.iter().any(|request| request.name == "libgnutls30"));
        let revalidated = AptDiscovery::revalidate_plan(&runner, &cancel, &plan);
        assert_eq!(runner.captured_args(), original_args);
        assert!(
            matches!(revalidated, Capability::Available(DiscoveryResult::Available(ref current)) if current.content_hash == plan.content_hash && current.intent == plan.intent)
        );

        // libgnutls30 was NOT in the user's selection but apt decided to
        // install it as a dependency. It must appear in the plan as a new
        // package so the user sees what apt actually decided.
        let gnutls = plan
            .candidates
            .iter()
            .find(|c| c.name == "libgnutls30")
            .expect("Additional dependency libgnutls30 must be disclosed");
        assert!(
            gnutls.is_new,
            "libgnutls30 should be marked is_new since it is newly installed"
        );
        assert_eq!(gnutls.available_version, "3.7.3-4ubuntu1.4");

        // Original selections must also be present.
        assert!(plan.candidates.iter().any(|c| c.name == "curl"));
        assert!(plan.candidates.iter().any(|c| c.name == "libcurl4"));
    }

    #[test]
    fn plan_for_selection_dpkg_locked_error() {
        let stderr = include_str!("../../../../fixtures/apt/dpkg_lock.txt");
        let output = make_output("", stderr, 100);
        let runner = CapturingMockRunner::new(output);
        let cancel = crate::jobs::CancellationToken::new();

        let selected = vec![make_test_candidate("curl", "7.81.0-1ubuntu1.16")];
        let result = AptDiscovery::plan_for_selection(&runner, &cancel, &selected);

        match result {
            Capability::Available(DiscoveryResult::Failed(DiscoveryError::DpkgLocked)) => {}
            other => panic!("Expected DpkgLocked, got {other:?}"),
        }
    }

    #[test]
    fn plan_for_selection_cancelled() {
        struct CancellingRunner;
        impl CommandRunner for CancellingRunner {
            fn run(
                &self,
                _spec: &CommandSpec,
                _cancel: &dyn Cancellation,
            ) -> crate::error::Result<CommandOutput> {
                Err(crate::error::MaintainerError::Cancelled)
            }
        }

        let runner = CancellingRunner;
        let cancel = crate::jobs::CancellationToken::new();
        cancel.cancel();

        let selected = vec![make_test_candidate("curl", "7.81.0-1ubuntu1.16")];
        let result = AptDiscovery::plan_for_selection(&runner, &cancel, &selected);

        match result {
            Capability::Available(DiscoveryResult::Failed(DiscoveryError::Cancelled)) => {}
            other => panic!("Expected Cancelled, got {other:?}"),
        }
    }
}
