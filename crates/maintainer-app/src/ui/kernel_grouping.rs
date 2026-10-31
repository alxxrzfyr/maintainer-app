//! Kernel update grouping for the Updates page.
//!
//! Provides a pure, unit-testable function that groups kernel `UpdateCandidate`s
//! by version so the UI can render one row per kernel bump rather than 6-8
//! visually duplicated rows. Grouping is display-only: the full ungrouped
//! candidate list is always submitted unchanged to the privileged execution
//! path after the normal plan revalidation.

use maintainer_core::updates::{UpdateCandidate, UpdateClassification};
use std::collections::BTreeMap;

/// A display-only grouping of kernel update candidates sharing a version.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelGroup {
    /// The kernel version string (e.g. "7.0.0-30").
    pub version: String,
    /// Architecture-qualified selection keys belonging to this group.
    pub package_names: Vec<String>,
    /// Number of packages in the group.
    pub package_count: usize,
    /// Whether any package in the group is a security update.
    pub is_security: bool,
    /// Representative version transition string for display (e.g. "6.8.0-39 → 7.0.0-30").
    /// `None` if no installed version is known.
    pub version_transition: Option<String>,
}

/// Items produced by the grouping pass, ready for UI rendering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DisplayRow {
    /// A single non-kernel candidate rendered as a normal row.
    Single(usize),
    /// A grouped kernel row representing multiple packages.
    KernelGroup(KernelGroup),
}

/// Group kernel candidates by version, leaving non-kernel candidates as-is.
///
/// Returns a list of `DisplayRow` items in a stable order: non-kernel candidates
/// appear in their original order, and kernel groups appear at the position of
/// the first kernel candidate in the original slice (preserving relative order
/// among kernel groups as well).
///
/// The `usize` index in `DisplayRow::Single` refers to the position in the
/// input `candidates` slice.
pub fn group_kernel_candidates(candidates: &[UpdateCandidate]) -> Vec<DisplayRow> {
    // First pass: identify kernel candidates and extract their versions.
    // Use BTreeMap for deterministic ordering of groups by version string.
    let mut kernel_groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
    let mut first_kernel_index: BTreeMap<String, usize> = BTreeMap::new();
    let mut is_kernel_index = vec![false; candidates.len()];

    for (i, candidate) in candidates.iter().enumerate() {
        if candidate.is_kernel {
            is_kernel_index[i] = true;
            let version = extract_kernel_version(candidate);
            first_kernel_index.entry(version.clone()).or_insert(i);
            kernel_groups.entry(version).or_default().push(i);
        }
    }

    // Build display rows preserving original order.
    let mut rows = Vec::new();
    let mut emitted_groups: std::collections::HashSet<String> = std::collections::HashSet::new();

    for (i, candidate) in candidates.iter().enumerate() {
        if !is_kernel_index[i] {
            rows.push(DisplayRow::Single(i));
        } else {
            let version = extract_kernel_version(candidate);
            if emitted_groups.contains(&version) {
                continue; // Already emitted this group's row.
            }
            emitted_groups.insert(version.clone());

            let indices = &kernel_groups[&version];
            let package_names: Vec<String> = indices
                .iter()
                .map(|&idx| candidates[idx].selection_key())
                .collect();
            let package_count = package_names.len();
            let is_security = indices
                .iter()
                .any(|&idx| candidates[idx].classification == UpdateClassification::Security);

            // Derive version transition from the first candidate that has an
            // installed_version (prefer versioned packages over meta-packages).
            let version_transition = derive_version_transition(indices, candidates, &version);

            rows.push(DisplayRow::KernelGroup(KernelGroup {
                version,
                package_names,
                package_count,
                is_security,
                version_transition,
            }));
        }
    }

    rows
}

/// Extract the kernel version from a candidate.
///
/// Strategy:
/// 1. For versioned packages like `linux-image-7.0.0-30-generic`, extract the
///    version from the package name using a regex-like pattern match.
/// 2. For meta-packages like `linux-image-generic-hwe-24.04`, use the
///    `available_version` field which contains the actual kernel version.
/// 3. Fallback: use the `available_version` field directly.
fn extract_kernel_version(candidate: &UpdateCandidate) -> String {
    // Try to extract version from the package name first.
    if let Some(v) = extract_version_from_name(&candidate.name) {
        return v;
    }

    // For meta-packages (linux-image-generic, linux-headers-generic-hwe-24.04, etc.)
    // and HWE tool packages (linux-hwe-7.0-tools-7.0.0-30), use available_version.
    // The available_version typically looks like "7.0.0-30.31" or "7.0.0-30.31~22.04.1".
    // Extract just the kernel ABI version (e.g. "7.0.0-30").
    extract_abi_version(&candidate.available_version)
}

/// Try to extract a kernel ABI version from a package name.
///
/// Matches patterns like:
/// - `linux-image-7.0.0-30-generic` → "7.0.0-30"
/// - `linux-headers-7.0.0-30-generic` → "7.0.0-30"
/// - `linux-modules-extra-7.0.0-30-generic` → "7.0.0-30"
/// - `linux-hwe-7.0-headers-7.0.0-30` → "7.0.0-30"
/// - `linux-hwe-7.0-tools-7.0.0-30` → "7.0.0-30"
/// - `linux-tools-7.0.0-30-generic` → "7.0.0-30"
///
/// Returns `None` for meta-packages without an embedded version.
fn extract_version_from_name(name: &str) -> Option<String> {
    // Look for a segment matching the kernel ABI pattern: digits.digits.digits-digits
    // This pattern: X.Y.Z-N where X,Y,Z,N are numeric.
    for segment in name.split('-') {
        if let Some(version) = try_parse_version_start(segment, name) {
            return Some(version);
        }
    }
    None
}

/// Given a segment that starts a version number (e.g. "7" from "7.0.0-30-generic"),
/// attempt to reconstruct the full ABI version from the original name.
fn try_parse_version_start(segment: &str, full_name: &str) -> Option<String> {
    // We need to find "X.Y.Z-N" within the name.
    // Strategy: find all occurrences of patterns like "D.D.D-D" in the full name.
    let _ = segment; // Not used in this approach; we scan the full name.
    find_kernel_abi_version(full_name)
}

/// Scan a string for the first occurrence of a kernel ABI version pattern.
///
/// Pattern: `major.minor.patch-abi` where each component is numeric.
/// Examples: "7.0.0-30", "6.8.0-40", "5.15.0-101"
fn find_kernel_abi_version(s: &str) -> Option<String> {
    let bytes = s.as_bytes();
    let len = bytes.len();
    let mut i = 0;

    while i < len {
        // Look for a digit that starts a version.
        if bytes[i].is_ascii_digit() {
            if let Some((version, end)) = try_parse_abi_at(s, i) {
                // Verify it's at a word boundary (preceded by '-' or start).
                let at_boundary = i == 0 || bytes[i - 1] == b'-' || bytes[i - 1] == b'/';
                if at_boundary {
                    // Verify the character after is '-', end of string, or '~'.
                    let after_ok = end >= len
                        || bytes[end] == b'-'
                        || bytes[end] == b'~'
                        || bytes[end] == b'.'
                        || bytes[end] == b'+';
                    if after_ok {
                        return Some(version);
                    }
                }
            }
            // Skip past this digit sequence.
            while i < len && bytes[i] != b'-' && bytes[i] != b'.' {
                i += 1;
            }
        }
        i += 1;
    }
    None
}

/// Try to parse "major.minor.patch-abi" starting at position `start`.
/// Returns the parsed version string and the byte index past the last character.
fn try_parse_abi_at(s: &str, start: usize) -> Option<(String, usize)> {
    let remainder = &s[start..];

    // Parse major: one or more digits.
    let (major, rest) = consume_digits(remainder)?;
    let rest = strip_prefix_char(rest, '.')?;

    // Parse minor: one or more digits.
    let (minor, rest) = consume_digits(rest)?;
    let rest = strip_prefix_char(rest, '.')?;

    // Parse patch: one or more digits.
    let (patch, rest) = consume_digits(rest)?;
    let rest = strip_prefix_char(rest, '-')?;

    // Parse ABI: one or more digits.
    let (abi, rest) = consume_digits(rest)?;

    let version = format!("{major}.{minor}.{patch}-{abi}");
    let consumed = remainder.len() - rest.len();
    Some((version, start + consumed))
}

/// Consume one or more ASCII digits from the start of `s`.
fn consume_digits(s: &str) -> Option<(&str, &str)> {
    let end = s
        .bytes()
        .position(|b| !b.is_ascii_digit())
        .unwrap_or(s.len());
    if end == 0 {
        return None;
    }
    Some((&s[..end], &s[end..]))
}

/// Strip a single expected character from the start of `s`.
fn strip_prefix_char(s: &str, c: char) -> Option<&str> {
    s.strip_prefix(c)
}

/// Extract ABI version from a version string like "7.0.0-30.31" or "7.0.0-30.31~22.04.1".
fn extract_abi_version(version_str: &str) -> String {
    if let Some(v) = find_kernel_abi_version(version_str) {
        return v;
    }
    // Fallback: use the version string itself (truncated at first '~' or '+').
    version_str
        .split_once('~')
        .map(|(left, _)| left)
        .unwrap_or(version_str)
        .to_string()
}

/// Derive a version transition string for a kernel group.
///
/// Looks for a candidate with both installed_version and available_version to
/// show something like "6.8.0-39 → 7.0.0-30".
fn derive_version_transition(
    indices: &[usize],
    candidates: &[UpdateCandidate],
    target_version: &str,
) -> Option<String> {
    // Prefer a versioned package (one with the version in its name) for the
    // transition display. Fall back to any candidate with an installed version.
    for &idx in indices {
        let c = &candidates[idx];
        if let Some(ref installed) = c.installed_version {
            let from = extract_abi_version(installed);
            if from != target_version {
                return Some(format!("{from} → {target_version}"));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use maintainer_core::updates::{HoldState, UpdateBackend};

    fn make_candidate(
        name: &str,
        installed: Option<&str>,
        available: &str,
        classification: UpdateClassification,
        is_kernel: bool,
    ) -> UpdateCandidate {
        UpdateCandidate {
            architecture: None,
            name: name.to_string(),
            installed_version: installed.map(|s| s.to_string()),
            available_version: available.to_string(),
            classification,
            download_bytes: Some(1_000_000),
            hold_state: HoldState::None,
            backend: UpdateBackend::Apt,
            is_new: false,
            is_kernel,
        }
    }

    fn make_kernel_candidate(name: &str, installed: &str, available: &str) -> UpdateCandidate {
        make_candidate(
            name,
            Some(installed),
            available,
            UpdateClassification::Standard,
            true,
        )
    }

    #[test]
    fn empty_input_returns_empty() {
        let candidates: Vec<UpdateCandidate> = vec![];
        let rows = group_kernel_candidates(&candidates);
        assert!(rows.is_empty());
    }

    #[test]
    fn non_kernel_candidates_ungrouped() {
        let candidates = vec![
            make_candidate(
                "curl",
                Some("7.88.0"),
                "7.88.1",
                UpdateClassification::Standard,
                false,
            ),
            make_candidate(
                "openssl",
                Some("3.0.2"),
                "3.0.3",
                UpdateClassification::Security,
                false,
            ),
            make_candidate(
                "libfoo",
                Some("1.0"),
                "1.1",
                UpdateClassification::Unknown,
                false,
            ),
        ];
        let rows = group_kernel_candidates(&candidates);
        assert_eq!(rows.len(), 3);
        for (i, row) in rows.iter().enumerate() {
            match row {
                DisplayRow::Single(idx) => assert_eq!(*idx, i),
                DisplayRow::KernelGroup(_) => panic!("Expected Single, got KernelGroup"),
            }
        }
    }

    #[test]
    fn full_kernel_chain_grouped_into_one() {
        let candidates = vec![
            make_kernel_candidate(
                "linux-image-generic-hwe-24.04",
                "6.8.0-39.39~22.04.1",
                "7.0.0-30.31~22.04.1",
            ),
            make_kernel_candidate(
                "linux-hwe-7.0-headers-7.0.0-30",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
            make_kernel_candidate(
                "linux-headers-7.0.0-30-generic",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
            make_kernel_candidate(
                "linux-headers-generic-hwe-24.04",
                "6.8.0-39.39~22.04.1",
                "7.0.0-30.31~22.04.1",
            ),
            make_kernel_candidate("linux-hwe-7.0-tools-7.0.0-30", "6.8.0-39.39", "7.0.0-30.31"),
            make_kernel_candidate("linux-tools-7.0.0-30-generic", "6.8.0-39.39", "7.0.0-30.31"),
            make_kernel_candidate("linux-image-7.0.0-30-generic", "6.8.0-39.39", "7.0.0-30.31"),
            make_kernel_candidate(
                "linux-modules-7.0.0-30-generic",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
        ];
        let rows = group_kernel_candidates(&candidates);
        assert_eq!(rows.len(), 1, "Expected 1 kernel group, got {}", rows.len());

        match &rows[0] {
            DisplayRow::KernelGroup(group) => {
                assert_eq!(group.version, "7.0.0-30");
                assert_eq!(group.package_count, 8);
                assert_eq!(group.package_names.len(), 8);
                assert!(!group.is_security);
                // Should have a version transition since installed versions differ.
                assert!(group.version_transition.is_some());
                let transition = group.version_transition.as_ref().unwrap();
                assert!(transition.contains("→"), "transition: {transition}");
                assert!(transition.contains("7.0.0-30"), "transition: {transition}");
            }
            DisplayRow::Single(_) => panic!("Expected KernelGroup"),
        }
    }

    #[test]
    fn security_kernel_package_marks_group_as_security() {
        let candidates = vec![
            make_kernel_candidate("linux-image-7.0.0-30-generic", "6.8.0-39.39", "7.0.0-30.31"),
            make_candidate(
                "linux-headers-7.0.0-30-generic",
                Some("6.8.0-39.39"),
                "7.0.0-30.31",
                UpdateClassification::Security,
                true,
            ),
            make_kernel_candidate(
                "linux-modules-7.0.0-30-generic",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
        ];
        let rows = group_kernel_candidates(&candidates);
        assert_eq!(rows.len(), 1);

        match &rows[0] {
            DisplayRow::KernelGroup(group) => {
                assert!(group.is_security, "Group should be marked as security");
                assert_eq!(group.package_count, 3);
            }
            _ => panic!("Expected KernelGroup"),
        }
    }

    #[test]
    fn mixed_kernel_and_non_kernel_produces_correct_split() {
        let candidates = vec![
            make_candidate(
                "curl",
                Some("7.88.0"),
                "7.88.1",
                UpdateClassification::Standard,
                false,
            ),
            make_kernel_candidate("linux-image-7.0.0-30-generic", "6.8.0-39.39", "7.0.0-30.31"),
            make_kernel_candidate(
                "linux-headers-7.0.0-30-generic",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
            make_candidate(
                "openssl",
                Some("3.0.2"),
                "3.0.3",
                UpdateClassification::Security,
                false,
            ),
            make_kernel_candidate(
                "linux-modules-7.0.0-30-generic",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
        ];
        let rows = group_kernel_candidates(&candidates);
        // Should be: curl (Single), kernel group (1 row), openssl (Single) = 3 rows
        assert_eq!(rows.len(), 3, "Got {} rows: {:?}", rows.len(), rows);

        match &rows[0] {
            DisplayRow::Single(idx) => assert_eq!(candidates[*idx].name, "curl"),
            _ => panic!("Expected Single for curl"),
        }
        match &rows[1] {
            DisplayRow::KernelGroup(group) => {
                assert_eq!(group.version, "7.0.0-30");
                assert_eq!(group.package_count, 3);
            }
            _ => panic!("Expected KernelGroup"),
        }
        match &rows[2] {
            DisplayRow::Single(idx) => assert_eq!(candidates[*idx].name, "openssl"),
            _ => panic!("Expected Single for openssl"),
        }
    }

    #[test]
    fn two_different_kernel_versions_produce_two_groups() {
        let candidates = vec![
            make_kernel_candidate("linux-image-7.0.0-30-generic", "6.8.0-39.39", "7.0.0-30.31"),
            make_kernel_candidate(
                "linux-headers-7.0.0-30-generic",
                "6.8.0-39.39",
                "7.0.0-30.31",
            ),
            make_kernel_candidate("linux-image-7.0.0-31-generic", "7.0.0-30.31", "7.0.0-31.32"),
            make_kernel_candidate(
                "linux-headers-7.0.0-31-generic",
                "7.0.0-30.31",
                "7.0.0-31.32",
            ),
        ];
        let rows = group_kernel_candidates(&candidates);
        assert_eq!(rows.len(), 2);

        match &rows[0] {
            DisplayRow::KernelGroup(g) => {
                assert_eq!(g.version, "7.0.0-30");
                assert_eq!(g.package_count, 2);
            }
            _ => panic!("Expected KernelGroup for 7.0.0-30"),
        }
        match &rows[1] {
            DisplayRow::KernelGroup(g) => {
                assert_eq!(g.version, "7.0.0-31");
                assert_eq!(g.package_count, 2);
            }
            _ => panic!("Expected KernelGroup for 7.0.0-31"),
        }
    }

    #[test]
    fn extract_version_from_versioned_package_names() {
        assert_eq!(
            extract_version_from_name("linux-image-7.0.0-30-generic"),
            Some("7.0.0-30".to_string())
        );
        assert_eq!(
            extract_version_from_name("linux-headers-6.8.0-40-generic"),
            Some("6.8.0-40".to_string())
        );
        assert_eq!(
            extract_version_from_name("linux-modules-extra-5.15.0-101-generic"),
            Some("5.15.0-101".to_string())
        );
        assert_eq!(
            extract_version_from_name("linux-hwe-7.0-headers-7.0.0-30"),
            Some("7.0.0-30".to_string())
        );
        assert_eq!(
            extract_version_from_name("linux-hwe-7.0-tools-7.0.0-30"),
            Some("7.0.0-30".to_string())
        );
        assert_eq!(
            extract_version_from_name("linux-tools-7.0.0-30-generic"),
            Some("7.0.0-30".to_string())
        );
    }

    #[test]
    fn extract_version_from_meta_packages_uses_available_version() {
        // Meta-packages don't have the version in their name.
        assert_eq!(
            extract_version_from_name("linux-image-generic-hwe-24.04"),
            None
        );
        assert_eq!(extract_version_from_name("linux-headers-generic"), None);

        // But extract_kernel_version uses available_version as fallback.
        let candidate = make_kernel_candidate(
            "linux-image-generic-hwe-24.04",
            "6.8.0-39.39~22.04.1",
            "7.0.0-30.31~22.04.1",
        );
        assert_eq!(extract_kernel_version(&candidate), "7.0.0-30");
    }

    #[test]
    fn extract_abi_version_from_version_strings() {
        assert_eq!(extract_abi_version("7.0.0-30.31~22.04.1"), "7.0.0-30");
        assert_eq!(extract_abi_version("7.0.0-30.31"), "7.0.0-30");
        assert_eq!(extract_abi_version("6.8.0-39.39~22.04.1"), "6.8.0-39");
        assert_eq!(extract_abi_version("5.15.0-101.111"), "5.15.0-101");
    }

    #[test]
    fn hwe_package_name_with_short_version_extracts_full_abi() {
        // linux-hwe-7.0-headers-7.0.0-30: the "7.0" is NOT a full ABI version,
        // but "7.0.0-30" IS. The parser should find the full one.
        let candidate = make_kernel_candidate(
            "linux-hwe-7.0-headers-7.0.0-30",
            "6.8.0-39.39",
            "7.0.0-30.31",
        );
        let version = extract_kernel_version(&candidate);
        assert_eq!(version, "7.0.0-30");
    }

    #[test]
    fn adversarial_no_extractable_version_falls_back_to_raw_string() {
        // A kernel candidate whose name and available_version both lack any
        // recognizable "X.Y.Z-N" pattern. extract_abi_version's fallback path
        // (split_once('~') or the string itself) must not panic and must produce
        // *some* deterministic version key, even if not meaningful, so grouping
        // still terminates and doesn't crash the UI.
        let weird = make_candidate(
            "linux-image-totallybroken",
            Some("garbage"),
            "still-garbage",
            UpdateClassification::Standard,
            true,
        );
        let rows = group_kernel_candidates(&[weird]);
        assert_eq!(rows.len(), 1);
        match &rows[0] {
            DisplayRow::KernelGroup(g) => {
                assert_eq!(g.package_count, 1);
                assert!(!g.version.is_empty());
            }
            DisplayRow::Single(_) => panic!("still expected a KernelGroup since is_kernel=true"),
        }
    }
}
