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

// Execute the installed-policy verification section with a controlled pkaction.
// Installation and service reloads are deliberately outside this test boundary.
fn run_policy_verification(missing_action: &str, missing_annotation: bool) -> std::process::Output {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let content = std::fs::read_to_string(installer_path()).unwrap();
    let (_, verification) = content
        .split_once("echo \"Authorization required for each action:\"")
        .expect("installer policy verification section");
    let stub = r#"
set -euo pipefail
pkaction() {
    if [[ "$2" == "io.github.maintainerapp.${MISSING_ACTION}" ]]; then
        return 0
    fi
    echo '  implicit active: auth_admin_keep'
    if [[ "$2" == 'io.github.maintainerapp.install-updates' && "$MISSING_ANNOTATION" == 'false' ]]; then
        echo '  org.freedesktop.policykit.exec.path -> /usr/libexec/maintainer-helper'
    fi
}
"#;
    let mut child = Command::new("bash")
        .arg("-s")
        .env("MISSING_ACTION", missing_action)
        .env("MISSING_ANNOTATION", missing_annotation.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start policy verification");
    let mut stdin = child.stdin.take().unwrap();
    stdin.write_all(stub.as_bytes()).unwrap();
    stdin.write_all(verification.as_bytes()).unwrap();
    drop(stdin);
    child.wait_with_output().unwrap()
}

#[test]
fn installer_exits_nonzero_on_missing_action() {
    for action in EXPECTED_ACTIONS {
        let output = run_policy_verification(action, false);
        let error = String::from_utf8_lossy(&output.stderr);
        assert!(!output.status.success(), "missing {action} was accepted");
        assert!(
            error.contains(&format!(
                "action io.github.maintainerapp.{action} not found"
            )),
            "{error}"
        );
        assert!(
            error.contains("FATAL:"),
            "verification did not reach its failure decision: {error}"
        );
    }
}

#[test]
fn installer_accepts_complete_installed_policy() {
    let output = run_policy_verification("", false);
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn installer_rejects_missing_exec_annotation() {
    let output = run_policy_verification("", true);
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("missing org.freedesktop.policykit.exec.path annotation")
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
