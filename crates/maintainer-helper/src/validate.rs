//! Parameter validation for helper operation requests.
//!
//! All validation happens before any privileged action is taken.
//! Bounds are chosen to prevent abuse while remaining useful.

use crate::api::{
    HelperError, JournalVacuumRequest, PackageCacheRequest, SmartRefreshRequest,
    TerminateProcessRequest,
};

/// Minimum journal vacuum size in MiB.
pub const JOURNAL_MIN_SIZE_MIB: u32 = 100;
/// Maximum journal vacuum size in MiB.
pub const JOURNAL_MAX_SIZE_MIB: u32 = 4096;
/// Minimum journal vacuum age in days.
pub const JOURNAL_MIN_AGE_DAYS: u32 = 1;
/// Maximum journal vacuum age in days.
pub const JOURNAL_MAX_AGE_DAYS: u32 = 365;

/// Minimum PID value (PID 1 is init, not terminable by this helper).
pub const MIN_TERMINABLE_PID: u32 = 2;

/// Maximum reasonable request ID length.
const MAX_REQUEST_ID_LEN: usize = 128;

/// Maximum drive ID length.
const MAX_DRIVE_ID_LEN: usize = 256;
/// Maximum packages accepted in one selected update request.
const MAX_UPDATE_PACKAGES: usize = 4096;
/// Maximum length of one `name=version` package specification.
const MAX_PACKAGE_SPEC_LEN: usize = 512;

/// Parse the privileged helper's fixed update CLI payload.
///
/// The caller may choose only `full` or `selected`. Selected entries must be
/// exact `name=version` arguments. Character validation prevents APT option
/// injection while still supporting Debian epochs, revisions, architecture
/// qualifiers, and common version punctuation.
pub fn parse_install_updates_cli(
    args: &[String],
) -> Result<crate::api::InstallUpdatesRequest, HelperError> {
    match args {
        [mode] if mode == "full" => Ok(crate::api::InstallUpdatesRequest::Full),
        [mode, specs @ ..] if mode == "selected" => {
            if specs.is_empty() || specs.len() > MAX_UPDATE_PACKAGES {
                return Err(HelperError::ValidationFailed {
                    reason: format!(
                        "selected update count must be between 1 and {MAX_UPDATE_PACKAGES}"
                    ),
                });
            }
            for spec in specs {
                validate_package_spec(spec)?;
            }
            Ok(crate::api::InstallUpdatesRequest::Selected(specs.to_vec()))
        }
        _ => Err(HelperError::ValidationFailed {
            reason: "update mode must be exactly 'full' or 'selected'".to_string(),
        }),
    }
}

fn validate_package_spec(spec: &str) -> Result<(), HelperError> {
    if spec.is_empty() || spec.len() > MAX_PACKAGE_SPEC_LEN {
        return Err(HelperError::ValidationFailed {
            reason: format!("package specification length must be 1..={MAX_PACKAGE_SPEC_LEN}"),
        });
    }

    let Some((name, version)) = spec.split_once('=') else {
        return Err(HelperError::ValidationFailed {
            reason: "package specification must use name=version".to_string(),
        });
    };
    if name.is_empty()
        || !name
            .chars()
            .next()
            .is_some_and(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit())
        || !name.chars().all(|ch| {
            ch.is_ascii_lowercase() || ch.is_ascii_digit() || matches!(ch, '+' | '-' | '.' | ':')
        })
    {
        return Err(HelperError::ValidationFailed {
            reason: "package name contains invalid characters".to_string(),
        });
    }
    if version.is_empty()
        || !version
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '+' | '-' | '.' | ':' | '~'))
    {
        return Err(HelperError::ValidationFailed {
            reason: "package version contains invalid characters".to_string(),
        });
    }

    Ok(())
}

/// Validate a process termination request.
pub fn validate_terminate_process(request: &TerminateProcessRequest) -> Result<(), HelperError> {
    if request.pid < MIN_TERMINABLE_PID || request.pid > libc::pid_t::MAX as u32 {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "pid {} is outside the terminable range {}..={}",
                request.pid,
                MIN_TERMINABLE_PID,
                libc::pid_t::MAX
            ),
        });
    }

    if request.start_time_ticks == 0 {
        return Err(HelperError::ValidationFailed {
            reason: "start_time_ticks must be non-zero".to_string(),
        });
    }

    Ok(())
}

/// Validate a package cache clean request.
pub fn validate_package_cache(request: &PackageCacheRequest) -> Result<(), HelperError> {
    if request.request_id.is_empty() {
        return Err(HelperError::ValidationFailed {
            reason: "request_id must not be empty".to_string(),
        });
    }

    if request.request_id.len() > MAX_REQUEST_ID_LEN {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "request_id length {} exceeds maximum {}",
                request.request_id.len(),
                MAX_REQUEST_ID_LEN
            ),
        });
    }

    if !request
        .request_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return Err(HelperError::ValidationFailed {
            reason: "request_id contains invalid characters; only [a-zA-Z0-9_-] allowed"
                .to_string(),
        });
    }

    Ok(())
}

/// Validate a journal vacuum request against bounded size and age policy.
pub fn validate_journal_vacuum(request: &JournalVacuumRequest) -> Result<(), HelperError> {
    if request.max_size_mib < JOURNAL_MIN_SIZE_MIB || request.max_size_mib > JOURNAL_MAX_SIZE_MIB {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "max_size_mib {} outside bounds [{}, {}]",
                request.max_size_mib, JOURNAL_MIN_SIZE_MIB, JOURNAL_MAX_SIZE_MIB
            ),
        });
    }

    if request.max_age_days < JOURNAL_MIN_AGE_DAYS || request.max_age_days > JOURNAL_MAX_AGE_DAYS {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "max_age_days {} outside bounds [{}, {}]",
                request.max_age_days, JOURNAL_MIN_AGE_DAYS, JOURNAL_MAX_AGE_DAYS
            ),
        });
    }

    Ok(())
}

/// Validate a SMART refresh request.
///
/// Drive ID must match the UDisks2 drive identifier pattern:
/// alphanumeric characters and underscores only.
pub fn validate_smart_refresh(request: &SmartRefreshRequest) -> Result<(), HelperError> {
    if request.drive_id.is_empty() {
        return Err(HelperError::ValidationFailed {
            reason: "drive_id must not be empty".to_string(),
        });
    }

    if request.drive_id.len() > MAX_DRIVE_ID_LEN {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "drive_id length {} exceeds maximum {}",
                request.drive_id.len(),
                MAX_DRIVE_ID_LEN
            ),
        });
    }

    if !request
        .drive_id
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Err(HelperError::ValidationFailed {
            reason: "drive_id contains invalid characters; only [a-zA-Z0-9_] allowed".to_string(),
        });
    }

    Ok(())
}

/// Maximum kernel package name length.
const MAX_KERNEL_PACKAGE_LEN: usize = 128;

/// Validate a kernel removal request.
///
/// Defense-in-depth: independently re-derives the running kernel and rejects
/// the request if the package matches it. This check does NOT trust the caller's
/// assertion that a package is safe to remove.
///
/// Validation steps:
/// 1. Package name is non-empty and within length bounds.
/// 2. Package name matches the versioned kernel package naming convention.
/// 3. Package name characters are within the dpkg-safe set.
/// 4. The package is NOT the currently running kernel (re-derived from /proc/version).
pub fn validate_remove_kernel(
    request: &crate::api::RemoveKernelRequest,
) -> Result<(), HelperError> {
    let running_release = read_running_kernel_release()?;
    validate_remove_kernel_against_release(request, &running_release)
}

/// Core validation logic, parameterized by the running kernel release.
///
/// Split out from `validate_remove_kernel` so tests exercise the exact
/// production validation path against an injected release string, rather
/// than a separate copy that could silently drift from the real one.
fn validate_remove_kernel_against_release(
    request: &crate::api::RemoveKernelRequest,
    running_release: &str,
) -> Result<(), HelperError> {
    if request.package_name.is_empty() {
        return Err(HelperError::ValidationFailed {
            reason: "package_name must not be empty".to_string(),
        });
    }

    if request.package_name.len() > MAX_KERNEL_PACKAGE_LEN {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "package_name length {} exceeds maximum {}",
                request.package_name.len(),
                MAX_KERNEL_PACKAGE_LEN
            ),
        });
    }

    // Only allow characters valid in dpkg package names: lowercase alphanumeric,
    // digits, plus, dash, dot.
    if !request
        .package_name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '.' || c == '+')
    {
        return Err(HelperError::ValidationFailed {
            reason: "package_name contains invalid characters; only [a-z0-9.+-] allowed"
                .to_string(),
        });
    }

    // Must match the versioned kernel package pattern
    if !maintainer_core::kernel::is_versioned_kernel_package(&request.package_name) {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "package_name '{}' does not match kernel package naming convention",
                request.package_name
            ),
        });
    }

    // DEFENSE IN DEPTH: reject if this package matches the running kernel.
    // This is the second independent check (the first is in
    // maintainer-core's list_kernels), and running_release is re-derived
    // from /proc/version by the caller, never trusted from the request.
    let running_image = maintainer_core::kernel::release_to_image_package(running_release);

    if request.package_name == running_image {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "cannot remove '{}': it is the currently running kernel",
                request.package_name
            ),
        });
    }

    // Also check headers/modules variants of the running kernel
    let running_headers = format!("linux-headers-{running_release}");
    let running_modules = format!("linux-modules-{running_release}");
    let running_modules_extra = format!("linux-modules-extra-{running_release}");

    if request.package_name == running_headers
        || request.package_name == running_modules
        || request.package_name == running_modules_extra
    {
        return Err(HelperError::ValidationFailed {
            reason: format!(
                "cannot remove '{}': it belongs to the currently running kernel",
                request.package_name
            ),
        });
    }

    Ok(())
}

/// Read the running kernel release from /proc/version.
///
/// Format: "Linux version 6.8.0-40-generic (builder@host) ..."
/// We extract the second field.
fn read_running_kernel_release() -> Result<String, HelperError> {
    let contents =
        std::fs::read_to_string("/proc/version").map_err(|e| HelperError::OperationFailed {
            reason: format!("failed to read /proc/version: {e}"),
        })?;

    parse_proc_version(&contents).ok_or_else(|| HelperError::OperationFailed {
        reason: "failed to parse kernel release from /proc/version".to_string(),
    })
}

/// Parse the kernel release from /proc/version content.
///
/// Expected format: "Linux version <release> ..."
fn parse_proc_version(contents: &str) -> Option<String> {
    let mut parts = contents.split_whitespace();
    parts.next()?; // "Linux"
    parts.next()?; // "version"
    Some(parts.next()?.to_string()) // "6.8.0-40-generic"
}

/// Testable entry point: validates against an injected release string
/// instead of reading `/proc/version`, exercising the exact same
/// `validate_remove_kernel_against_release` logic the production path uses.
#[cfg(test)]
pub fn validate_remove_kernel_with_release(
    request: &crate::api::RemoveKernelRequest,
    running_release: &str,
) -> Result<(), HelperError> {
    validate_remove_kernel_against_release(request, running_release)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminate_process_rejects_pid_zero() {
        let req = TerminateProcessRequest {
            pid: 0,
            start_time_ticks: 12345,
        };
        let err = validate_terminate_process(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn terminate_process_rejects_pid_one() {
        let req = TerminateProcessRequest {
            pid: 1,
            start_time_ticks: 12345,
        };
        let err = validate_terminate_process(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn terminate_process_rejects_pid_overflow() {
        let req = TerminateProcessRequest {
            pid: u32::MAX,
            start_time_ticks: 1,
        };
        assert!(matches!(
            validate_terminate_process(&req),
            Err(HelperError::ValidationFailed { .. })
        ));
    }

    #[test]
    fn terminate_process_rejects_zero_start_time() {
        let req = TerminateProcessRequest {
            pid: 100,
            start_time_ticks: 0,
        };
        let err = validate_terminate_process(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn terminate_process_accepts_valid_request() {
        let req = TerminateProcessRequest {
            pid: 1234,
            start_time_ticks: 98765,
        };
        assert!(validate_terminate_process(&req).is_ok());
    }

    #[test]
    fn package_cache_rejects_empty_request_id() {
        let req = PackageCacheRequest {
            request_id: String::new(),
        };
        let err = validate_package_cache(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn package_cache_rejects_oversized_request_id() {
        let req = PackageCacheRequest {
            request_id: "a".repeat(MAX_REQUEST_ID_LEN + 1),
        };
        let err = validate_package_cache(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn package_cache_rejects_special_characters() {
        let req = PackageCacheRequest {
            request_id: "valid-id/../../../etc/passwd".to_string(),
        };
        let err = validate_package_cache(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn package_cache_accepts_valid_id() {
        let req = PackageCacheRequest {
            request_id: "req-123_abc".to_string(),
        };
        assert!(validate_package_cache(&req).is_ok());
    }

    #[test]
    fn journal_vacuum_rejects_size_below_minimum() {
        let req = JournalVacuumRequest {
            max_size_mib: 50,
            max_age_days: 30,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_rejects_size_above_maximum() {
        let req = JournalVacuumRequest {
            max_size_mib: 5000,
            max_age_days: 30,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_rejects_age_below_minimum() {
        let req = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 0,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_rejects_age_above_maximum() {
        let req = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 400,
        };
        let err = validate_journal_vacuum(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn journal_vacuum_accepts_valid_bounds() {
        let req = JournalVacuumRequest {
            max_size_mib: 500,
            max_age_days: 30,
        };
        assert!(validate_journal_vacuum(&req).is_ok());
    }

    #[test]
    fn smart_refresh_rejects_empty_drive_id() {
        let req = SmartRefreshRequest {
            drive_id: String::new(),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_rejects_path_traversal() {
        let req = SmartRefreshRequest {
            drive_id: "../../etc/shadow".to_string(),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_rejects_slashes() {
        let req = SmartRefreshRequest {
            drive_id: "drive/with/slashes".to_string(),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_rejects_oversized_id() {
        let req = SmartRefreshRequest {
            drive_id: "A".repeat(MAX_DRIVE_ID_LEN + 1),
        };
        let err = validate_smart_refresh(&req).unwrap_err();
        assert!(matches!(err, HelperError::ValidationFailed { .. }));
    }

    #[test]
    fn smart_refresh_accepts_valid_udisks_id() {
        let req = SmartRefreshRequest {
            drive_id: "SAMSUNG_MZVL21T0HCLR_0BL2_S64FNX0T506050".to_string(),
        };
        assert!(validate_smart_refresh(&req).is_ok());
    }

    // --- validate_remove_kernel tests ---

    #[test]
    fn remove_kernel_rejects_running_kernel() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "linux-image-6.8.0-40-generic".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
        if let Err(HelperError::ValidationFailed { reason }) = result {
            assert!(reason.contains("currently running kernel"));
        }
    }

    #[test]
    fn remove_kernel_rejects_running_kernel_headers() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "linux-headers-6.8.0-40-generic".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
    }

    #[test]
    fn remove_kernel_rejects_non_kernel_package() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "curl".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
        if let Err(HelperError::ValidationFailed { reason }) = result {
            assert!(reason.contains("naming convention"));
        }
    }

    #[test]
    fn remove_kernel_rejects_meta_package() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "linux-image-generic".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
    }

    #[test]
    fn remove_kernel_rejects_malformed_name_uppercase() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "Linux-Image-6.5.0-35-generic".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
        if let Err(HelperError::ValidationFailed { reason }) = result {
            assert!(reason.contains("invalid characters"));
        }
    }

    #[test]
    fn remove_kernel_rejects_path_traversal() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "../etc/passwd".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
    }

    #[test]
    fn remove_kernel_rejects_empty() {
        let req = crate::api::RemoveKernelRequest {
            package_name: String::new(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(matches!(result, Err(HelperError::ValidationFailed { .. })));
    }

    #[test]
    fn remove_kernel_accepts_non_running_installed_kernel() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "linux-image-6.5.0-35-generic".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(result.is_ok());
    }

    #[test]
    fn remove_kernel_accepts_old_headers() {
        let req = crate::api::RemoveKernelRequest {
            package_name: "linux-headers-6.5.0-35-generic".to_string(),
        };
        let result = validate_remove_kernel_with_release(&req, "6.8.0-40-generic");
        assert!(result.is_ok());
    }

    #[test]
    fn parse_proc_version_extracts_release() {
        let content = "Linux version 6.8.0-40-generic (buildd@lcy02-amd64-116) (x86_64-linux-gnu-gcc-13 (Ubuntu 13.2.0-23ubuntu4) 13.2.0, GNU ld (GNU Binutils for Ubuntu) 2.42) #40-Ubuntu SMP PREEMPT_DYNAMIC Thu Jun  6 19:43:14 UTC 2024";
        let release = super::parse_proc_version(content);
        assert_eq!(release, Some("6.8.0-40-generic".to_string()));
    }
}

#[cfg(test)]
mod update_cli_tests {
    use super::*;
    use crate::api::InstallUpdatesRequest;

    #[test]
    fn parses_full_update_mode_without_packages() {
        let request = parse_install_updates_cli(&["full".to_string()]).unwrap();
        assert_eq!(request, InstallUpdatesRequest::Full);
    }

    #[test]
    fn parses_selected_mode_with_exact_versions() {
        let request = parse_install_updates_cli(&[
            "selected".to_string(),
            "curl=8.5.0-2ubuntu10.6".to_string(),
            "libc6:amd64=2.39-0ubuntu8.4".to_string(),
        ])
        .unwrap();
        assert_eq!(
            request,
            InstallUpdatesRequest::Selected(vec![
                "curl=8.5.0-2ubuntu10.6".to_string(),
                "libc6:amd64=2.39-0ubuntu8.4".to_string(),
            ])
        );
    }

    #[test]
    fn rejects_selected_mode_without_packages() {
        assert!(parse_install_updates_cli(&["selected".to_string()]).is_err());
    }

    #[test]
    fn rejects_option_and_shell_injection_package_specs() {
        for spec in [
            "--option=APT::Update::Pre-Invoke=/tmp/evil",
            "curl=2.0;touch /tmp/evil",
            "curl=$(id)",
            "../curl=2.0",
        ] {
            assert!(
                parse_install_updates_cli(&["selected".to_string(), spec.to_string()]).is_err(),
                "accepted unsafe package spec: {spec}"
            );
        }
    }

    #[test]
    fn rejects_unknown_mode_and_extra_full_arguments() {
        assert!(parse_install_updates_cli(&["unknown".to_string()]).is_err());
        assert!(parse_install_updates_cli(&["full".to_string(), "curl=2.0".to_string()]).is_err());
    }
}
