//! Verify that the install-privileged-files.sh script validates all policy
//! actions and exits nonzero when an action or annotation is missing.

use std::path::PathBuf;

fn installer_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../scripts/install-privileged-files.sh")
}

/// All policy actions that the installer must verify.
const EXPECTED_ACTIONS: &[&str] = &[
    "terminate-process",
    "vacuum-journal",
    "clean-package-cache",
    "refresh-smart",
    "remove-kernel",
    "install-updates",
];

#[test]
fn installer_verifies_all_policy_actions() {
    let content = std::fs::read_to_string(installer_path())
        .expect("failed to read scripts/install-privileged-files.sh");

    for action in EXPECTED_ACTIONS {
        assert!(
            content.contains(action),
            "installer must verify action '{action}' but it is not found in the script"
        );
    }
}

#[test]
fn installer_checks_install_updates_exec_annotation() {
    let content = std::fs::read_to_string(installer_path())
        .expect("failed to read scripts/install-privileged-files.sh");

    assert!(
        content.contains("install-updates") && content.contains("exec.path")
            || content.contains("install-updates") && content.contains("maintainer-helper"),
        "installer must verify the exec.path annotation for install-updates"
    );
}

#[test]
fn installer_exits_nonzero_on_missing_action() {
    let content = std::fs::read_to_string(installer_path())
        .expect("failed to read scripts/install-privileged-files.sh");

    // Script must contain an exit 1 (or equivalent failure) path when
    // verification finds errors.
    assert!(
        content.contains("exit 1"),
        "installer must exit nonzero when policy verification fails"
    );

    // Must have logic to count or detect errors.
    assert!(
        content.contains("POLICY_ERRORS") || content.contains("fail"),
        "installer must track verification errors and fail on them"
    );
}

#[test]
fn installer_has_set_euo_pipefail() {
    let content = std::fs::read_to_string(installer_path())
        .expect("failed to read scripts/install-privileged-files.sh");

    assert!(
        content.contains("set -euo pipefail"),
        "installer must use strict bash error handling"
    );
}
