//! Tests for application ID consistency and stale-ID guard.

use std::path::Path;
use std::process::Command;

use maintainer_app::application::APP_ID;

#[test]
fn app_id_matches_packaging_convention() {
    assert_eq!(APP_ID, "io.github.maintainerapp.Maintainer");
}

/// Guard test: the old application ID must not appear anywhere in workspace sources or data,
/// excluding this test file itself.
#[test]
fn no_stale_old_app_id_in_workspace() {
    let workspace_root = Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("workspace root");

    let stale_id = ["com", "maintainer", "app"].join(".");

    let output = Command::new("grep")
        .args([
            "-rn",
            "--include=*.rs",
            "--include=*.xml",
            "--include=*.toml",
            "--include=*.desktop",
            "--include=*.service",
            "--include=*.policy",
            &stale_id,
        ])
        .arg(workspace_root.join("crates"))
        .arg(workspace_root.join("data"))
        .output()
        .expect("grep command failed");

    let stdout = String::from_utf8_lossy(&output.stdout);

    let matches: Vec<&str> = stdout
        .lines()
        .filter(|line| !line.contains("app_id.rs"))
        .collect();

    assert!(
        matches.is_empty(),
        "Found stale ID references:\n{}",
        matches.join("\n")
    );
}
