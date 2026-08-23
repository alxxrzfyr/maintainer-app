//! Fixed system update execution for the privileged helper CLI.

use std::os::unix::process::CommandExt;
use std::process::Command;

use crate::api::InstallUpdatesRequest;

const APT_GET_PATH: &str = "/usr/bin/apt-get";

/// Build the only APT argument shapes this helper permits.
pub fn apt_get_arguments(request: &InstallUpdatesRequest) -> Vec<String> {
    let mut arguments = vec![
        "-y".to_string(),
        "-o".to_string(),
        "APT::Status-Fd=1".to_string(),
        "-o".to_string(),
        "Dpkg::Options::=--force-confdef".to_string(),
        "-o".to_string(),
        "Dpkg::Options::=--force-confold".to_string(),
    ];
    match request {
        InstallUpdatesRequest::Full => arguments.push("dist-upgrade".to_string()),
        InstallUpdatesRequest::Selected(package_specs) => {
            arguments.push("install".to_string());
            // Prevent every following package spec from being interpreted as
            // an APT option even if validation is accidentally weakened later.
            arguments.push("--".to_string());
            arguments.extend(package_specs.iter().cloned());
        }
    }
    arguments
}

/// Replace the authenticated helper process with the fixed APT transaction.
///
/// `exec` preserves stdout/stderr pipes and PID, so the app can stream real APT
/// progress and cancellation terminates APT itself rather than orphaning a
/// privileged child process.
pub fn exec_install_updates(request: &InstallUpdatesRequest) -> std::io::Error {
    let arguments = apt_get_arguments(request);
    Command::new(APT_GET_PATH)
        .args(arguments)
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .exec()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::InstallUpdatesRequest;

    #[test]
    fn full_update_builds_fixed_dist_upgrade_arguments() {
        assert_eq!(
            apt_get_arguments(&InstallUpdatesRequest::Full),
            [
                "-y",
                "-o",
                "APT::Status-Fd=1",
                "-o",
                "Dpkg::Options::=--force-confdef",
                "-o",
                "Dpkg::Options::=--force-confold",
                "dist-upgrade",
            ]
        );
    }

    #[test]
    fn selected_update_uses_option_terminator_and_exact_versions() {
        let request = InstallUpdatesRequest::Selected(vec![
            "curl=8.5.0-2ubuntu10.6".to_string(),
            "libc6:amd64=2.39-0ubuntu8.4".to_string(),
        ]);
        assert_eq!(
            apt_get_arguments(&request),
            [
                "-y",
                "-o",
                "APT::Status-Fd=1",
                "-o",
                "Dpkg::Options::=--force-confdef",
                "-o",
                "Dpkg::Options::=--force-confold",
                "install",
                "--",
                "curl=8.5.0-2ubuntu10.6",
                "libc6:amd64=2.39-0ubuntu8.4",
            ]
        );
    }
}
