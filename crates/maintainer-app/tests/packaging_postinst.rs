//! Verify that the Debian postinst script reloads PolicyKit after installation.
//!
//! When a new `.policy` file is installed, Polkit must be reloaded for the new
//! action definitions to take effect without a reboot.

use std::path::PathBuf;

fn postinst_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../packaging/debian/postinst")
}

#[test]
fn postinst_reloads_polkit_after_policy_install() {
    let content =
        std::fs::read_to_string(postinst_path()).expect("failed to read packaging/debian/postinst");

    // The postinst must contain a systemctl restart or reload of polkit.
    let has_polkit_reload = content.lines().any(|line| {
        let trimmed = line.trim();
        // Skip comments
        if trimmed.starts_with('#') {
            return false;
        }
        trimmed.contains("systemctl")
            && (trimmed.contains("restart") || trimmed.contains("reload"))
            && trimmed.contains("polkit")
    });

    assert!(
        has_polkit_reload,
        "postinst must reload/restart polkit after policy installation"
    );
}
