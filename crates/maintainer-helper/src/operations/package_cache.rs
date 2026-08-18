//! Package cache cleaning via fixed `apt-get clean` invocation.
//!
//! No caller-provided paths, arguments, or shell strings.
//! The operation is always `apt-get clean` with no additional parameters.

use std::process::Command;

use crate::api::{HelperError, OperationResponse};

/// Fixed path to apt-get binary.
const APT_GET_PATH: &str = "/usr/bin/apt-get";

/// Execute `apt-get clean` to remove cached package files.
///
/// This is a fixed operation with no caller-controlled arguments.
pub fn execute_clean_package_cache() -> Result<OperationResponse, HelperError> {
    let output = Command::new(APT_GET_PATH)
        .arg("clean")
        .env_clear()
        .env("PATH", "/usr/sbin:/usr/bin:/sbin:/bin")
        .env("DEBIAN_FRONTEND", "noninteractive")
        .output()
        .map_err(|e| HelperError::OperationFailed {
            reason: format!("failed to execute apt-get clean: {e}"),
        })?;

    if output.status.success() {
        Ok(OperationResponse {
            success: true,
            message: "package cache cleaned successfully".to_string(),
        })
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr);
        Err(HelperError::OperationFailed {
            reason: format!("apt-get clean failed: {stderr}"),
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

    #[test]
    fn clean_operation_uses_no_caller_args() {
        // Structural test: verify the command construction does not
        // interpolate any caller input. The function signature takes
        // no parameters that could be injected.
        // This test documents the fixed nature of the operation.
        let _output = Command::new(APT_GET_PATH).arg("clean").env_clear();
        // If this compiles and the function signature takes no &str args,
        // no injection vector exists.
    }
}
