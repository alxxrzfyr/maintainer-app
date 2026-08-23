//! Parser for `pkcon --plain` stdout lines.
//!
//! Extracts structured status information from pkcon's tab-delimited plain
//! output format without exposing raw tool text to the user interface.
//! Lines that don't match a known pattern are ignored (caller keeps showing
//! the last known good status).

/// Parsed status from a single pkcon `--plain` stdout line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PkconLineStatus {
    /// A `Status:` line indicating the current phase.
    Status(PkconPhase),
    /// A `Percentage:` line with a value 0–100.
    Percentage(u32),
    /// A `Package:` line with the package identifier.
    Package(String),
    /// The `Results:` header, indicating all work is done and results follow.
    Results,
}

/// Known phases reported by pkcon's `Status:` lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PkconPhase {
    WaitingInQueue,
    WaitingForAuthentication,
    Starting,
    LoadingCache,
    Querying,
    Resolving,
    DepResolving,
    Downloading,
    Installing,
    Updating,
    Removing,
    Cleanup,
    Finished,
    /// A status word we don't recognize. Not surfaced to the user, but
    /// allows the caller to know a status line was present.
    Other,
}

impl PkconPhase {
    /// User-facing display string for this phase.
    pub fn display_message(&self) -> &'static str {
        match self {
            Self::WaitingInQueue => "Waiting in queue…",
            Self::WaitingForAuthentication => "Waiting for authentication…",
            Self::Starting => "Starting…",
            Self::LoadingCache => "Loading package cache…",
            Self::Querying => "Querying packages…",
            Self::Resolving => "Resolving dependencies…",
            Self::DepResolving => "Resolving dependencies…",
            Self::Downloading => "Downloading packages…",
            Self::Installing => "Installing packages…",
            Self::Updating => "Installing updates…",
            Self::Removing => "Removing packages…",
            Self::Cleanup => "Cleaning up…",
            Self::Finished => "Finished",
            Self::Other => "Processing…",
        }
    }
}

/// Parse a single line of `pkcon --plain` stdout.
///
/// Returns `Some(PkconLineStatus)` if the line matches a known pattern,
/// or `None` if it should be ignored (unknown format, results detail lines,
/// transaction header, etc.).
///
/// # Format
///
/// `pkcon --plain` emits tab-separated `Key:\tValue` lines:
/// - `Transaction:\t<description>` — ignored (header)
/// - `Status: \t<phase>` — current phase
/// - `Percentage:\t<0–100>` — progress percentage
/// - `Package:\t<name-version.arch>` — package being processed
/// - `Results:` — marks the start of results (no tab)
///
/// Results detail lines (`Bug fix`, `Installed`, etc.) are ignored.
pub fn parse_pkcon_plain_line(line: &str) -> Option<PkconLineStatus> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // Results header (no tab, just "Results:")
    if trimmed == "Results:" {
        return Some(PkconLineStatus::Results);
    }

    // Tab-separated key:value lines
    // pkcon uses "Key:\t" or "Key: \t" formats
    let (key, value) = split_pkcon_line(trimmed)?;

    match key {
        "Status" => {
            let phase = parse_phase(value);
            Some(PkconLineStatus::Status(phase))
        }
        "Percentage" => {
            let pct: u32 = value.parse().ok()?;
            if pct <= 100 {
                Some(PkconLineStatus::Percentage(pct))
            } else {
                None
            }
        }
        "Package" => {
            // Package ID format: "name-version.arch" or "name;version;arch;repo"
            // Extract just the package name for display.
            let name = extract_package_name(value);
            if name.is_empty() {
                None
            } else {
                Some(PkconLineStatus::Package(name.to_string()))
            }
        }
        // Transaction header, unknown keys — ignore
        _ => None,
    }
}

/// Split a pkcon plain line into (key, value) at the first tab.
///
/// Handles both `Key:\tValue` and `Key: \tValue` formats.
fn split_pkcon_line(line: &str) -> Option<(&str, &str)> {
    let tab_pos = line.find('\t')?;
    let key_part = line[..tab_pos].trim_end();
    let key = key_part.strip_suffix(':')?;
    let value = line[tab_pos + 1..].trim();
    Some((key, value))
}

/// Map a status string to a known phase.
fn parse_phase(status: &str) -> PkconPhase {
    match status {
        "Waiting in queue" => PkconPhase::WaitingInQueue,
        "Waiting for authentication" => PkconPhase::WaitingForAuthentication,
        "Starting" => PkconPhase::Starting,

        "Loading cache" => PkconPhase::LoadingCache,
        "Querying" => PkconPhase::Querying,
        "Resolving" => PkconPhase::Resolving,
        "Resolving dependencies" => PkconPhase::DepResolving,
        "Downloading" | "Downloading packages" => PkconPhase::Downloading,
        "Installing" | "Installing packages" => PkconPhase::Installing,
        "Updating" | "Updating packages" => PkconPhase::Updating,
        "Removing" | "Removing packages" => PkconPhase::Removing,
        "Cleaning up" => PkconPhase::Cleanup,
        "Finished" => PkconPhase::Finished,
        _ => PkconPhase::Other,
    }
}

/// Extract the package name from a pkcon package identifier.
///
/// Handles the `name-version.arch` format used in `Package:` lines,
/// which is actually `name;version;arch;repo` with dashes in the
/// full identifier field. The real format from `--plain` is just
/// the raw package-id string like `curl-7.81.0-1ubuntu1.16.amd64`
/// or the semicolon-delimited `name;version;arch;origin` format.
fn extract_package_name(package_id: &str) -> &str {
    // Try semicolon format first (standard PackageKit ID)
    if let Some(pos) = package_id.find(';') {
        return &package_id[..pos];
    }
    // Hyphenated format: `name-version.arch` — take up to the first
    // character sequence that looks like a version (digit after hyphen).
    // But pkcon --plain Package: lines use the semicolon format based on
    // real output, so this is a fallback only.
    if let Some(pos) = find_version_separator(package_id) {
        return &package_id[..pos];
    }
    package_id
}

/// Find the position of the hyphen that separates name from version.
/// Version starts with a digit after a hyphen.
fn find_version_separator(s: &str) -> Option<usize> {
    let bytes = s.as_bytes();
    for i in 1..bytes.len() {
        if bytes[i - 1] == b'-' && bytes[i].is_ascii_digit() {
            return Some(i - 1);
        }
    }
    None
}

/// Format a user-facing progress message from parsed pkcon state.
///
/// Never exposes raw pkcon text. Combines the phase with optional
/// package name context.
pub fn format_progress_message(phase: &PkconPhase, current_package: Option<&str>) -> String {
    let base = phase.display_message();
    match (phase, current_package) {
        (PkconPhase::Downloading, Some(pkg)) => format!("Downloading {pkg}…"),
        (PkconPhase::Installing, Some(pkg)) => format!("Installing {pkg}…"),
        (PkconPhase::Updating, Some(pkg)) => format!("Updating {pkg}…"),
        (PkconPhase::Removing, Some(pkg)) => format!("Removing {pkg}…"),
        _ => base.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Real captured lines from `pkcon --plain --noninteractive get-updates`
    // on this machine (Ubuntu 24.04, PackageKit 1.2.8).

    #[test]
    fn parse_transaction_header_ignored() {
        let line = "Transaction:\tGetting updates";
        assert_eq!(parse_pkcon_plain_line(line), None);
    }

    #[test]
    fn parse_status_waiting_in_queue() {
        let line = "Status: \tWaiting in queue";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(PkconPhase::WaitingInQueue))
        );
    }

    #[test]
    fn parse_status_waiting_for_authentication() {
        let line = "Status: \tWaiting for authentication";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(
                PkconPhase::WaitingForAuthentication
            ))
        );
    }

    #[test]
    fn parse_status_starting() {
        let line = "Status: \tStarting";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(PkconPhase::Starting))
        );
    }

    #[test]
    fn parse_status_loading_cache() {
        let line = "Status: \tLoading cache";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(PkconPhase::LoadingCache))
        );
    }

    #[test]
    fn parse_status_querying() {
        let line = "Status: \tQuerying";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(PkconPhase::Querying))
        );
    }

    #[test]
    fn parse_status_installing_packages() {
        let line = "Status: \tInstalling packages";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(PkconPhase::Installing))
        );
    }

    #[test]
    fn parse_status_finished() {
        let line = "Status: \tFinished";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Status(PkconPhase::Finished))
        );
    }

    #[test]
    fn parse_percentage_zero() {
        let line = "Percentage:\t0";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Percentage(0))
        );
    }

    #[test]
    fn parse_percentage_mid() {
        let line = "Percentage:\t50";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Percentage(50))
        );
    }

    #[test]
    fn parse_percentage_hundred() {
        let line = "Percentage:\t100";
        assert_eq!(
            parse_pkcon_plain_line(line),
            Some(PkconLineStatus::Percentage(100))
        );
    }

    #[test]
    fn parse_percentage_invalid_over_100() {
        let line = "Percentage:\t101";
        assert_eq!(parse_pkcon_plain_line(line), None);
    }

    #[test]
    fn parse_package_semicolon_format() {
        let line = "Package:\tlinux-headers-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64";
        let result = parse_pkcon_plain_line(line);
        assert!(matches!(result, Some(PkconLineStatus::Package(_))));
        if let Some(PkconLineStatus::Package(name)) = result {
            // This is the hyphenated format; name extraction should get the name part
            assert_eq!(name, "linux-headers");
        }
    }

    #[test]
    fn parse_package_with_semicolons() {
        // The real --plain format from Package: lines uses hyphens, but
        // if PackageKit ever emits the standard semicolon-delimited ID:
        let line = "Package:\tcurl;7.81.0-1ubuntu1.16;amd64;Ubuntu";
        let result = parse_pkcon_plain_line(line);
        assert_eq!(result, Some(PkconLineStatus::Package("curl".to_string())));
    }

    #[test]
    fn parse_results_header() {
        let line = "Results:";
        assert_eq!(parse_pkcon_plain_line(line), Some(PkconLineStatus::Results));
    }

    #[test]
    fn parse_results_detail_line_ignored() {
        // These are result summary lines that follow "Results:" — not key:value format
        let line = "Bug fix      console-setup-1.226ubuntu1.1.all (ubuntu-noble-updates-main)";
        assert_eq!(parse_pkcon_plain_line(line), None);
    }

    #[test]
    fn parse_installed_detail_line_ignored() {
        let line = "Installed    linux-headers-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64 (+auto:ubuntu-noble-updates-main)";
        assert_eq!(parse_pkcon_plain_line(line), None);
    }

    #[test]
    fn parse_empty_line_ignored() {
        assert_eq!(parse_pkcon_plain_line(""), None);
        assert_eq!(parse_pkcon_plain_line("   "), None);
    }

    #[test]
    fn format_message_downloading_with_package() {
        let msg = format_progress_message(&PkconPhase::Downloading, Some("curl"));
        assert_eq!(msg, "Downloading curl…");
    }

    #[test]
    fn format_message_installing_with_package() {
        let msg = format_progress_message(&PkconPhase::Installing, Some("linux-image"));
        assert_eq!(msg, "Installing linux-image…");
    }

    #[test]
    fn format_message_loading_cache_no_package() {
        let msg = format_progress_message(&PkconPhase::LoadingCache, None);
        assert_eq!(msg, "Loading package cache…");
    }

    #[test]
    fn format_message_finished() {
        let msg = format_progress_message(&PkconPhase::Finished, None);
        assert_eq!(msg, "Finished");
    }

    #[test]
    fn extract_name_semicolons() {
        assert_eq!(
            extract_package_name("curl;7.81.0-1ubuntu1.16;amd64;Ubuntu"),
            "curl"
        );
    }

    #[test]
    fn extract_name_hyphenated_version() {
        assert_eq!(
            extract_package_name("linux-headers-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64"),
            "linux-headers"
        );
    }

    #[test]
    fn extract_name_simple_hyphenated() {
        assert_eq!(
            extract_package_name("vim-common-2:9.1.0016-1ubuntu7.19.all"),
            "vim-common"
        );
    }

    // Comprehensive test using ALL real captured lines from
    // `pkcon --plain --noninteractive get-updates` on this machine.
    #[test]
    fn parse_full_real_session() {
        let real_output = "\
Transaction:\tGetting updates
Status: \tWaiting in queue
Status: \tStarting
Status: \tLoading cache
Percentage:\t0
Percentage:\t100
Percentage:\t0
Percentage:\t100
Status: \tQuerying
Status: \tLoading cache
Percentage:\t0
Percentage:\t10
Percentage:\t100
Status: \tInstalling packages
Package:\tlinux-headers-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64
Package:\tlinux-hwe-7.0-headers-7.0.0-30-7.0.0-30.30~24.04.1.all
Package:\tlinux-hwe-7.0-tools-7.0.0-30-7.0.0-30.30~24.04.1.amd64
Package:\tlinux-image-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64
Package:\tlinux-modules-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64
Package:\tlinux-tools-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64
Status: \tFinished
Results:
Bug fix      console-setup-1.226ubuntu1.1.all (ubuntu-noble-updates-main)
Installed    linux-headers-7.0.0-30-generic-7.0.0-30.30~24.04.1.amd64 (+auto:ubuntu-noble-updates-main)
Bug fix      wget-1.21.4-1ubuntu4.5.amd64 (ubuntu-noble-updates-main)";

        let mut statuses = Vec::new();
        let mut percentages = Vec::new();
        let mut packages = Vec::new();
        let mut saw_results = false;

        for line in real_output.lines() {
            if let Some(parsed) = parse_pkcon_plain_line(line) {
                match parsed {
                    PkconLineStatus::Status(phase) => statuses.push(phase),
                    PkconLineStatus::Percentage(pct) => percentages.push(pct),
                    PkconLineStatus::Package(name) => packages.push(name),
                    PkconLineStatus::Results => saw_results = true,
                }
            }
        }

        // Verify we got the expected sequence
        assert_eq!(statuses[0], PkconPhase::WaitingInQueue);
        assert_eq!(statuses[1], PkconPhase::Starting);
        assert!(statuses.contains(&PkconPhase::Installing));
        assert!(statuses.contains(&PkconPhase::Finished));

        assert!(percentages.contains(&0));
        assert!(percentages.contains(&100));
        assert!(percentages.contains(&10));

        assert_eq!(packages.len(), 6);
        assert!(saw_results);

        // Verify NO result detail lines leaked through
        let all_parsed: Vec<_> = real_output
            .lines()
            .filter_map(parse_pkcon_plain_line)
            .collect();
        for parsed in &all_parsed {
            // None of the parsed items should contain raw "Bug fix" or
            // "Installed" result text.
            if let PkconLineStatus::Package(name) = parsed {
                assert!(!name.contains("Bug fix"));
                assert!(!name.contains("Installed"));
            }
        }
    }
}
