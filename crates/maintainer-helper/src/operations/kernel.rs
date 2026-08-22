//! Kernel package removal via `apt-get purge`.
//!
//! Removes a validated kernel image package and its associated headers/modules
//! packages in a single apt-get invocation. Uses `purge` (not just `remove`)
//! to also clear configuration files under /boot and avoid broken half-removed
//! kernel entries.
//!
//! # Approach: apt-get purge with explicit package list
//!
//! A single `linux-image-X` removal often leaves orphaned `linux-headers-X`
//! and `linux-modules-X` packages. Rather than relying on a subsequent
//! `autoremove` (which may not run), we explicitly purge the image, headers,
//! and modules packages in one invocation. Using `--yes` is safe because:
//! - The package name was validated against the kernel naming pattern
//! - The running kernel was independently verified as not matching
//! - Polkit already authenticated the user for this irreversible action
//!
//! The `--yes` flag is necessary because apt-get purge prompts interactively
//! by default, and this runs non-interactively in the helper.

use std::process::Command;

use crate::api::{HelperError, OperationResponse, RemoveKernelRequest};

/// Fixed path to apt-get binary.
const APT_GET_PATH: &str = "/usr/bin/apt-get";

/// Execute kernel package removal via `apt-get purge`.
///
/// Purges the requested kernel image package plus its matching headers and
/// modules packages (if installed). Packages that are not installed are
/// harmlessly ignored by apt-get.
pub fn execute_remove_kernel(
    request: &RemoveKernelRequest,
) -> Result<OperationResponse, HelperError> {
    // Derive the version suffix from the image package name to find related packages.
    // e.g. "linux-image-6.5.0-35-generic" -> "6.5.0-35-generic"
    let version_suffix = request
        .package_name
        .strip_prefix("linux-image-")
        .or_else(|| request.package_name.strip_prefix("linux-headers-"))
        .or_else(|| request.package_name.strip_prefix("linux-modules-extra-"))
        .or_else(|| request.package_name.strip_prefix("linux-modules-"))
        .unwrap_or(&request.package_name);

    // Build the full set of related kernel packages to purge.
    let image_pkg = format!("linux-image-{version_suffix}");
    let headers_pkg = format!("linux-headers-{version_suffix}");
    let modules_pkg = format!("linux-modules-{version_suffix}");
    let modules_extra_pkg = format!("linux-modules-extra-{version_suffix}");

    let output = Command::new(APT_GET_PATH)
        .args([
            "purge",
            "--yes",
            "--quiet",
            "--quiet",
            &image_pkg,
            &headers_pkg,
            &modules_pkg,
            &modules_extra_pkg,
        ])
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
        .map_err(|e| HelperError::OperationFailed {
            reason: format!("failed to execute apt-get purge: {e}"),
        })?;

    if output.status.success() {
        Ok(OperationResponse {
            success: true,
            message: format!(
                "kernel {version_suffix} purged successfully (image, headers, modules)"
            ),
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(HelperError::OperationFailed {
            reason: format!("apt-get purge failed: {stderr}"),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apt_get_path_is_absolute() {
        assert!(APT_GET_PATH.starts_with('/'));
    }
}
