//! Verify that Polkit XML action IDs match code constants in `api::actions`.
//!
//! This test reads the shipped policy XML and asserts that every action defined
//! in the code has a corresponding `<action id="...">` entry, and vice versa.

use std::collections::HashSet;
use std::path::PathBuf;

use maintainer_helper::api::actions;

fn policy_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../data/polkit/io.github.maintainerapp.policy")
}

fn extract_action_ids(xml: &str) -> HashSet<String> {
    let mut ids = HashSet::new();
    for line in xml.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("<action id=\"") {
            if let Some(id) = rest.strip_suffix("\">") {
                ids.insert(id.to_string());
            }
        }
    }
    ids
}

fn extract_exec_path_annotation(xml: &str, action_id: &str) -> Option<String> {
    let mut in_action = false;
    for line in xml.lines() {
        let trimmed = line.trim();
        if trimmed.contains(&format!("<action id=\"{action_id}\">")) {
            in_action = true;
        }
        if in_action && trimmed.contains("</action>") {
            break;
        }
        if in_action && trimmed.contains("org.freedesktop.policykit.exec.path") {
            // Extract value between > and <
            if let Some(start) = trimmed.find('>') {
                if let Some(end) = trimmed[start + 1..].find('<') {
                    return Some(trimmed[start + 1..start + 1 + end].to_string());
                }
            }
        }
    }
    None
}

#[test]
fn policy_xml_action_ids_match_code_constants() {
    let xml = std::fs::read_to_string(policy_path()).expect("failed to read policy XML");

    let xml_ids = extract_action_ids(&xml);

    let code_ids: HashSet<String> = [
        actions::TERMINATE_PROCESS,
        actions::CLEAN_PACKAGE_CACHE,
        actions::VACUUM_JOURNAL,
        actions::REFRESH_SMART,
        actions::REMOVE_KERNEL,
        actions::INSTALL_UPDATES,
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let missing_in_xml: Vec<_> = code_ids.difference(&xml_ids).collect();
    let missing_in_code: Vec<_> = xml_ids.difference(&code_ids).collect();

    assert!(
        missing_in_xml.is_empty(),
        "Code constants not found in policy XML: {missing_in_xml:?}"
    );
    assert!(
        missing_in_code.is_empty(),
        "Policy XML actions not found in code constants: {missing_in_code:?}"
    );
    assert_eq!(xml_ids.len(), 6, "Expected exactly 6 policy actions");
}

#[test]
fn install_updates_has_exec_path_annotation() {
    let xml = std::fs::read_to_string(policy_path()).expect("failed to read policy XML");

    let exec_path = extract_exec_path_annotation(&xml, actions::INSTALL_UPDATES)
        .expect("install-updates action must have exec.path annotation");

    assert_eq!(
        exec_path, "/usr/libexec/maintainer-helper",
        "exec-path annotation must point to the helper binary"
    );
}
