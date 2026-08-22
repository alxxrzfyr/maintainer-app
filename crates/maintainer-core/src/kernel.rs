//! Kernel enumeration, running-kernel detection, and removability determination.
//!
//! Design invariants:
//! - The currently running kernel is **structurally** never marked removable.
//!   Even if `apt-get --simulate autoremove` erroneously lists it, this module
//!   overrides that to `is_removable: false`.
//! - All external commands go through `CommandRunner` (no shell strings).
//! - Removability is determined solely by what apt reports as autoremovable —
//!   no fabricated EOL or unsupported status.

use crate::command::{CommandRunner, CommandSpec};
use crate::jobs::Cancellation;

/// Information about a single installed kernel package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelInfo {
    /// The dpkg package name (e.g. "linux-image-6.8.0-40-generic").
    pub package_name: String,
    /// The version string from dpkg (e.g. "6.8.0-40.40").
    pub version: String,
    /// Whether this kernel matches the currently running kernel (uname -r).
    pub is_running: bool,
    /// Whether apt considers this kernel safely removable.
    /// Structurally guaranteed `false` when `is_running` is `true`.
    pub is_removable: bool,
}

/// Errors from kernel enumeration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KernelError {
    /// A command failed to execute or returned unexpected output.
    CommandFailed { message: String },
    /// The operation was cancelled.
    Cancelled,
}

/// Read the running kernel release string.
///
/// Uses `uname -r` via the provided runner for testability.
pub fn running_kernel_release(
    runner: &dyn CommandRunner,
    cancel: &dyn Cancellation,
) -> Result<String, KernelError> {
    let spec = CommandSpec::new("uname", ["-r"]).map_err(|e| KernelError::CommandFailed {
        message: format!("cannot construct uname command: {e}"),
    })?;

    let output = runner.run(&spec, cancel).map_err(|e| {
        if matches!(e, crate::error::MaintainerError::Cancelled) {
            KernelError::Cancelled
        } else {
            KernelError::CommandFailed {
                message: format!("uname -r failed: {e}"),
            }
        }
    })?;

    if !output.success() {
        return Err(KernelError::CommandFailed {
            message: "uname -r returned non-zero".to_string(),
        });
    }

    let release = output.stdout.trim().to_string();
    if release.is_empty() {
        return Err(KernelError::CommandFailed {
            message: "uname -r returned empty output".to_string(),
        });
    }

    Ok(release)
}

/// Determine the running kernel release from a provided string source.
///
/// This is a pure helper used by the helper-side validation to re-derive
/// the running kernel from a pre-read `/proc/version` or uname output,
/// without needing a full `CommandRunner`.
pub fn extract_release_from_uname(uname_r: &str) -> &str {
    uname_r.trim()
}

/// Convert a kernel release string (e.g. "6.8.0-40-generic") into the
/// corresponding `linux-image-` package name.
pub fn release_to_image_package(release: &str) -> String {
    format!("linux-image-{release}")
}

/// Check whether a package name matches the kernel package naming convention.
///
/// Matches: linux-image-<version>, linux-headers-<version>, linux-modules-<version>,
/// linux-modules-extra-<version>.
/// Rejects meta-packages (linux-image-generic, etc.) and non-kernel packages.
pub fn is_versioned_kernel_package(name: &str) -> bool {
    let suffix = if let Some(s) = name.strip_prefix("linux-image-") {
        s
    } else if let Some(s) = name.strip_prefix("linux-headers-") {
        s
    } else if let Some(s) = name.strip_prefix("linux-modules-extra-") {
        s
    } else if let Some(s) = name.strip_prefix("linux-modules-") {
        s
    } else {
        return false;
    };

    // Must have a version suffix starting with a digit (not a meta-package
    // like "generic" or "lowlatency").
    suffix.chars().next().is_some_and(|c| c.is_ascii_digit())
}

/// Enumerate installed kernel image packages via `dpkg-query`.
///
/// Returns only `linux-image-*` packages that are fully installed (status `ii`).
pub fn enumerate_installed_kernels(
    runner: &dyn CommandRunner,
    cancel: &dyn Cancellation,
) -> Result<Vec<(String, String)>, KernelError> {
    let spec = CommandSpec::new(
        "dpkg-query",
        [
            "--showformat=${Status}\\t${Package}\\t${Version}\\n",
            "--show",
            "linux-image-*",
        ],
    )
    .map_err(|e| KernelError::CommandFailed {
        message: format!("cannot construct dpkg-query command: {e}"),
    })?;

    let output = runner.run(&spec, cancel).map_err(|e| {
        if matches!(e, crate::error::MaintainerError::Cancelled) {
            KernelError::Cancelled
        } else {
            KernelError::CommandFailed {
                message: format!("dpkg-query failed: {e}"),
            }
        }
    })?;

    // dpkg-query may return exit code 1 if no packages match the pattern,
    // which is a valid "zero kernels" result.
    if !output.success() && !output.stdout.is_empty() {
        return Err(KernelError::CommandFailed {
            message: format!("dpkg-query returned error: {}", output.stderr.trim()),
        });
    }

    Ok(parse_dpkg_query_output(&output.stdout))
}

/// Parse dpkg-query output into (package_name, version) pairs.
///
/// Only includes packages with status "install ok installed" and that
/// match the versioned kernel image pattern.
fn parse_dpkg_query_output(stdout: &str) -> Vec<(String, String)> {
    let mut results = Vec::new();

    for line in stdout.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() < 3 {
            continue;
        }

        let status = parts[0];
        let package = parts[1];
        let version = parts[2];

        // Only fully-installed packages
        if status != "install ok installed" {
            continue;
        }

        // Only versioned kernel image packages (not meta-packages)
        if !is_versioned_kernel_package(package) {
            continue;
        }

        results.push((package.to_string(), version.to_string()));
    }

    results
}

/// Determine which kernel packages apt considers autoremovable.
///
/// Invokes `apt-get --simulate autoremove` and returns the set of package
/// names that appear in `Remv` lines.
fn get_autoremovable_packages(
    runner: &dyn CommandRunner,
    cancel: &dyn Cancellation,
) -> Result<Vec<String>, KernelError> {
    let spec = CommandSpec::new(
        "apt-get",
        ["--simulate", "--quiet", "--quiet", "autoremove"],
    )
    .map_err(|e| KernelError::CommandFailed {
        message: format!("cannot construct apt-get command: {e}"),
    })?;

    let output = runner.run(&spec, cancel).map_err(|e| {
        if matches!(e, crate::error::MaintainerError::Cancelled) {
            KernelError::Cancelled
        } else {
            KernelError::CommandFailed {
                message: format!("apt-get autoremove simulate failed: {e}"),
            }
        }
    })?;

    // Parse Remv lines from the simulation output
    Ok(parse_remv_packages(&output.stdout))
}

/// Extract package names from `Remv <name> [<version>]` lines.
fn parse_remv_packages(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| {
            let rest = line.strip_prefix("Remv ")?;
            let name = rest.split_whitespace().next()?;
            Some(name.to_string())
        })
        .collect()
}

/// Full kernel enumeration: installed kernels with running and removability status.
///
/// # Safety invariant
///
/// The running kernel is **never** marked as removable, regardless of what apt
/// reports. This is enforced structurally: `is_removable` is set to
/// `autoremovable.contains(name) && !is_running`.
pub fn list_kernels(
    runner: &dyn CommandRunner,
    cancel: &dyn Cancellation,
) -> Result<Vec<KernelInfo>, KernelError> {
    let release = running_kernel_release(runner, cancel)?;
    let running_image = release_to_image_package(&release);

    let installed = enumerate_installed_kernels(runner, cancel)?;
    let autoremovable = get_autoremovable_packages(runner, cancel)?;

    let kernels = installed
        .into_iter()
        .map(|(package_name, version)| {
            let is_running = package_name == running_image;
            // SAFETY INVARIANT: running kernel is never removable.
            let is_removable = !is_running && autoremovable.contains(&package_name);

            KernelInfo {
                package_name,
                version,
                is_running,
                is_removable,
            }
        })
        .collect();

    Ok(kernels)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::CommandOutput;
    use crate::jobs::{Cancellation, CancellationToken};
    use std::sync::Mutex;

    fn make_output(stdout: &str, stderr: &str, exit_code: i32) -> CommandOutput {
        CommandOutput {
            exit_code: Some(exit_code),
            stdout: stdout.to_string(),
            stderr: stderr.to_string(),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    /// Sequencing mock runner: returns responses in order regardless of command.
    struct SequenceMockRunner {
        responses: Mutex<Vec<CommandOutput>>,
    }

    impl SequenceMockRunner {
        fn new(responses: Vec<CommandOutput>) -> Self {
            Self {
                responses: Mutex::new(responses),
            }
        }
    }

    impl CommandRunner for SequenceMockRunner {
        fn run(
            &self,
            _spec: &CommandSpec,
            _cancel: &dyn Cancellation,
        ) -> crate::error::Result<CommandOutput> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                Ok(make_output("", "", 0))
            } else {
                Ok(responses.remove(0))
            }
        }
    }

    #[test]
    fn running_kernel_identified_from_uname() {
        let runner = SequenceMockRunner::new(vec![make_output("6.8.0-40-generic\n", "", 0)]);
        let cancel = CancellationToken::new();
        let release = running_kernel_release(&runner, &cancel).unwrap();
        assert_eq!(release, "6.8.0-40-generic");
    }

    #[test]
    fn running_kernel_never_removable_even_if_apt_claims_it() {
        // Simulate: uname says 6.8.0-40-generic is running
        let uname_output = make_output("6.8.0-40-generic\n", "", 0);

        // dpkg-query shows two kernels installed
        let dpkg_output = make_output(
            "install ok installed\tlinux-image-6.8.0-40-generic\t6.8.0-40.40\n\
             install ok installed\tlinux-image-6.5.0-35-generic\t6.5.0-35.35\n",
            "",
            0,
        );

        // ADVERSARIAL: apt-get autoremove simulation erroneously claims the
        // running kernel is removable
        let apt_output = make_output(
            "Remv linux-image-6.8.0-40-generic [6.8.0-40.40]\n\
             Remv linux-image-6.5.0-35-generic [6.5.0-35.35]\n",
            "",
            0,
        );

        let runner = SequenceMockRunner::new(vec![uname_output, dpkg_output, apt_output]);
        let cancel = CancellationToken::new();

        let kernels = list_kernels(&runner, &cancel).unwrap();

        // The running kernel MUST NOT be removable despite apt suggesting it
        let running = kernels
            .iter()
            .find(|k| k.package_name == "linux-image-6.8.0-40-generic")
            .unwrap();
        assert!(running.is_running);
        assert!(
            !running.is_removable,
            "running kernel must never be removable"
        );

        // The non-running kernel IS removable since apt suggested it
        let old = kernels
            .iter()
            .find(|k| k.package_name == "linux-image-6.5.0-35-generic")
            .unwrap();
        assert!(!old.is_running);
        assert!(old.is_removable);
    }

    #[test]
    fn multiple_kernels_mixed_removability() {
        let uname_output = make_output("6.8.0-40-generic\n", "", 0);

        let dpkg_output = make_output(
            "install ok installed\tlinux-image-6.8.0-40-generic\t6.8.0-40.40\n\
             install ok installed\tlinux-image-6.5.0-35-generic\t6.5.0-35.35\n\
             install ok installed\tlinux-image-6.2.0-20-generic\t6.2.0-20.20\n",
            "",
            0,
        );

        // Only the oldest kernel is autoremovable
        let apt_output = make_output("Remv linux-image-6.2.0-20-generic [6.2.0-20.20]\n", "", 0);

        let runner = SequenceMockRunner::new(vec![uname_output, dpkg_output, apt_output]);
        let cancel = CancellationToken::new();

        let kernels = list_kernels(&runner, &cancel).unwrap();
        assert_eq!(kernels.len(), 3);

        let running = kernels
            .iter()
            .find(|k| k.package_name == "linux-image-6.8.0-40-generic")
            .unwrap();
        assert!(running.is_running);
        assert!(!running.is_removable);

        let middle = kernels
            .iter()
            .find(|k| k.package_name == "linux-image-6.5.0-35-generic")
            .unwrap();
        assert!(!middle.is_running);
        assert!(!middle.is_removable); // apt didn't suggest it

        let oldest = kernels
            .iter()
            .find(|k| k.package_name == "linux-image-6.2.0-20-generic")
            .unwrap();
        assert!(!oldest.is_running);
        assert!(oldest.is_removable);
    }

    #[test]
    fn zero_removable_kernels() {
        let uname_output = make_output("6.8.0-40-generic\n", "", 0);

        let dpkg_output = make_output(
            "install ok installed\tlinux-image-6.8.0-40-generic\t6.8.0-40.40\n\
             install ok installed\tlinux-image-6.5.0-35-generic\t6.5.0-35.35\n",
            "",
            0,
        );

        // apt suggests nothing for autoremoval
        let apt_output = make_output("", "", 0);

        let runner = SequenceMockRunner::new(vec![uname_output, dpkg_output, apt_output]);
        let cancel = CancellationToken::new();

        let kernels = list_kernels(&runner, &cancel).unwrap();
        assert_eq!(kernels.len(), 2);
        assert!(kernels.iter().all(|k| !k.is_removable));
    }

    #[test]
    fn is_versioned_kernel_package_accepts_valid() {
        assert!(is_versioned_kernel_package("linux-image-6.8.0-40-generic"));
        assert!(is_versioned_kernel_package(
            "linux-headers-6.8.0-40-generic"
        ));
        assert!(is_versioned_kernel_package(
            "linux-modules-6.8.0-40-generic"
        ));
        assert!(is_versioned_kernel_package(
            "linux-modules-extra-6.8.0-40-generic"
        ));
        assert!(is_versioned_kernel_package(
            "linux-image-5.15.0-101-generic"
        ));
    }

    #[test]
    fn is_versioned_kernel_package_rejects_invalid() {
        // Meta-packages
        assert!(!is_versioned_kernel_package("linux-image-generic"));
        assert!(!is_versioned_kernel_package("linux-headers-generic"));
        assert!(!is_versioned_kernel_package("linux-modules-generic"));
        // Non-kernel
        assert!(!is_versioned_kernel_package("curl"));
        assert!(!is_versioned_kernel_package("linux-libc-dev"));
        assert!(!is_versioned_kernel_package("linux-firmware"));
        // Empty
        assert!(!is_versioned_kernel_package(""));
    }

    #[test]
    fn parse_dpkg_query_filters_correctly() {
        let input = "install ok installed\tlinux-image-6.8.0-40-generic\t6.8.0-40.40\n\
                     install ok installed\tlinux-image-generic\t6.8.0-40.40\n\
                     deinstall ok config-files\tlinux-image-6.2.0-20-generic\t6.2.0-20.20\n\
                     install ok installed\tlinux-image-5.15.0-101-generic\t5.15.0-101.111\n";

        let results = parse_dpkg_query_output(input);
        assert_eq!(results.len(), 2);
        assert_eq!(results[0].0, "linux-image-6.8.0-40-generic");
        assert_eq!(results[1].0, "linux-image-5.15.0-101-generic");
    }

    #[test]
    fn parse_remv_packages_extracts_names() {
        let input = "Remv linux-image-6.2.0-20-generic [6.2.0-20.20]\n\
                     Remv linux-modules-6.2.0-20-generic [6.2.0-20.20]\n\
                     Conf some-other-line\n";

        let names = parse_remv_packages(input);
        assert_eq!(names.len(), 2);
        assert_eq!(names[0], "linux-image-6.2.0-20-generic");
        assert_eq!(names[1], "linux-modules-6.2.0-20-generic");
    }

    #[test]
    fn release_to_image_package_formats_correctly() {
        assert_eq!(
            release_to_image_package("6.8.0-40-generic"),
            "linux-image-6.8.0-40-generic"
        );
    }

    #[test]
    fn running_kernel_release_rejects_empty_output() {
        // If uname -r returns empty/whitespace, the function must fail (not return "")
        // to prevent list_kernels from marking the running kernel as non-running.
        let runner = SequenceMockRunner::new(vec![make_output("\n", "", 0)]);
        let cancel = CancellationToken::new();
        let result = running_kernel_release(&runner, &cancel);
        assert!(result.is_err(), "empty uname output must fail closed");
    }

    #[test]
    fn list_kernels_fails_closed_when_uname_fails() {
        // If uname -r returns non-zero, list_kernels should propagate the error
        // rather than continuing with an empty release (which would fail open).
        let runner = SequenceMockRunner::new(vec![make_output("", "", 1)]);
        let cancel = CancellationToken::new();
        let result = list_kernels(&runner, &cancel);
        assert!(result.is_err(), "uname failure must propagate");
    }
}
