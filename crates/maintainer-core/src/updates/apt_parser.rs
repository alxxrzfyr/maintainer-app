//! Parser for streaming output from `apt-get` commands.
//!
//! Extracts human-readable progress from line-by-line stdout during package
//! installation/upgrade (e.g. downloads, unpacking, setting up, trigger processing).

/// Status extracted from an `apt-get` output line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AptProgressStatus {
    /// A package is being fetched/downloaded.
    Downloading { package: String },
    /// Downloading completed.
    DownloadComplete,
    /// A package is being unpacked.
    Unpacking { package: String },
    /// A package is being installed/configured.
    Installing { package: String },
    /// System triggers or services are being processed.
    Configuring { target: String },
    /// Structured percentage emitted through APT::Status-Fd.
    Progress { percent: u64, message: String },
    /// System processes / services are being checked.
    CheckingServices,
}

impl AptProgressStatus {
    /// User-facing message string.
    pub fn display_message(&self) -> String {
        match self {
            Self::Downloading { package } => format!("Downloading {package}…"),
            Self::DownloadComplete => "Download complete. Preparing installation…".to_string(),
            Self::Unpacking { package } => format!("Unpacking {package}…"),
            Self::Installing { package } => format!("Installing {package}…"),
            Self::Configuring { target } => format!("Configuring {target}…"),
            Self::Progress { message, .. } => message.clone(),
            Self::CheckingServices => "Checking system services…".to_string(),
        }
    }

    /// Return APT's authoritative total percentage when Status-Fd supplied it.
    pub fn percentage(&self) -> Option<u64> {
        match self {
            Self::Progress { percent, .. } => Some(*percent),
            _ => None,
        }
    }
}

/// Parse a single line of `apt-get` stdout during an upgrade or installation.
pub fn parse_apt_progress_line(line: &str) -> Option<AptProgressStatus> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }

    // APT::Status-Fd emits colon-separated, machine-readable total progress:
    // `<kind>:<subject>:<percentage>:<description>`. `splitn(4)` preserves any
    // colons in the translated description.
    if trimmed.starts_with("pmstatus:") || trimmed.starts_with("dlstatus:") {
        let mut fields = trimmed.splitn(4, ':');
        let kind = fields.next()?;
        let _subject = fields.next()?;
        let raw_percentage = fields.next()?.trim().parse::<f64>().ok()?;
        if !raw_percentage.is_finite() {
            return None;
        }
        let percent = raw_percentage.round().clamp(0.0, 100.0) as u64;
        let description = fields.next()?.trim();
        let message = if description.is_empty() {
            if kind == "dlstatus" {
                "Downloading updates…"
            } else {
                "Installing updates…"
            }
        } else {
            description
        };
        return Some(AptProgressStatus::Progress {
            percent,
            message: message.to_string(),
        });
    }

    // 1. Download lines: "Get:1 http://... noble-updates/main amd64 curl amd64 ..."
    if trimmed.starts_with("Get:") {
        let parts: Vec<&str> = trimmed.split_whitespace().collect();
        // Package name is typically around index 4 or 5
        let package = if parts.len() >= 5 {
            // Find first part after http URL that looks like a package name
            let mut found_pkg = None;
            for (idx, &part) in parts.iter().enumerate().skip(2) {
                if !part.starts_with("http")
                    && !part.contains('/')
                    && !part.ends_with(']')
                    && idx + 1 < parts.len()
                    && (parts[idx + 1] == "amd64"
                        || parts[idx + 1] == "all"
                        || parts[idx + 1] == "i386")
                {
                    found_pkg = Some(part);
                    break;
                }
            }

            found_pkg.unwrap_or(parts[4])
        } else {
            "packages"
        };
        return Some(AptProgressStatus::Downloading {
            package: package.to_string(),
        });
    }

    // 2. Download finished
    if trimmed.starts_with("Fetched ") {
        return Some(AptProgressStatus::DownloadComplete);
    }

    // 3. Preparing to unpack: "Preparing to unpack .../curl_7.81.0_amd64.deb ..."
    if trimmed.starts_with("Preparing to unpack ") {
        let pkg = extract_deb_package_name(trimmed.strip_prefix("Preparing to unpack ")?);
        return Some(AptProgressStatus::Unpacking { package: pkg });
    }

    // 4. Unpacking: "Unpacking curl (7.81.0) over (7.80.0) ..."
    if trimmed.starts_with("Unpacking ") {
        let after = trimmed.strip_prefix("Unpacking ")?;
        let pkg = after.split_whitespace().next().unwrap_or("package");
        return Some(AptProgressStatus::Unpacking {
            package: pkg.to_string(),
        });
    }

    // 5. Setting up: "Setting up curl (7.81.0) ..."
    if trimmed.starts_with("Setting up ") {
        let after = trimmed.strip_prefix("Setting up ")?;
        let pkg = after.split_whitespace().next().unwrap_or("package");
        return Some(AptProgressStatus::Installing {
            package: pkg.to_string(),
        });
    }

    // 6. Processing triggers: "Processing triggers for man-db (2.12.0) ..."
    if trimmed.starts_with("Processing triggers for ") {
        let after = trimmed.strip_prefix("Processing triggers for ")?;
        let target = after.split_whitespace().next().unwrap_or("triggers");
        return Some(AptProgressStatus::Configuring {
            target: target.to_string(),
        });
    }

    // 7. Scanning processes
    if trimmed.starts_with("Scanning processes") || trimmed.starts_with("Scanning candidates") {
        return Some(AptProgressStatus::CheckingServices);
    }

    None
}

/// Extract package name from ".../curl_7.81.0-1ubuntu1_amd64.deb ..."
fn extract_deb_package_name(s: &str) -> String {
    let s = s.trim().trim_end_matches('.').trim();
    let filename = s.rsplit('/').next().unwrap_or(s);
    if let Some(pkg) = filename.split('_').next() {
        pkg.to_string()
    } else {
        filename.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_download_line() {
        let line = "Get:1 http://archive.ubuntu.com/ubuntu noble-updates/main amd64 curl amd64 7.81.0-1ubuntu1.16 [194 kB]";
        let parsed = parse_apt_progress_line(line);
        assert_eq!(
            parsed,
            Some(AptProgressStatus::Downloading {
                package: "curl".to_string()
            })
        );
        assert_eq!(parsed.unwrap().display_message(), "Downloading curl…");
    }

    #[test]
    fn parse_fetched_line() {
        let line = "Fetched 194 kB in 1s (194 kB/s)";
        let parsed = parse_apt_progress_line(line);
        assert_eq!(parsed, Some(AptProgressStatus::DownloadComplete));
    }

    #[test]
    fn parse_unpacking_line() {
        let line = "Unpacking curl (7.81.0-1ubuntu1.16) over (7.81.0-1ubuntu1.15) ...";
        let parsed = parse_apt_progress_line(line);
        assert_eq!(
            parsed,
            Some(AptProgressStatus::Unpacking {
                package: "curl".to_string()
            })
        );
        assert_eq!(parsed.unwrap().display_message(), "Unpacking curl…");
    }

    #[test]
    fn parse_setting_up_line() {
        let line = "Setting up curl (7.81.0-1ubuntu1.16) ...";
        let parsed = parse_apt_progress_line(line);
        assert_eq!(
            parsed,
            Some(AptProgressStatus::Installing {
                package: "curl".to_string()
            })
        );
        assert_eq!(parsed.unwrap().display_message(), "Installing curl…");
    }

    #[test]
    fn parse_processing_triggers_line() {
        let line = "Processing triggers for man-db (2.12.0-4build2) ...";
        let parsed = parse_apt_progress_line(line);
        assert_eq!(
            parsed,
            Some(AptProgressStatus::Configuring {
                target: "man-db".to_string()
            })
        );
        assert_eq!(parsed.unwrap().display_message(), "Configuring man-db…");
    }
}

#[cfg(test)]
mod structured_progress_tests {
    use super::*;

    #[test]
    fn parses_package_manager_percentage() {
        assert_eq!(
            parse_apt_progress_line("pmstatus:curl:40.0:Unpacking curl"),
            Some(AptProgressStatus::Progress {
                percent: 40,
                message: "Unpacking curl".to_string(),
            })
        );
    }

    #[test]
    fn parses_download_percentage_and_rounds() {
        assert_eq!(
            parse_apt_progress_line("dlstatus:1:9.61022:Downloading file 1 of 3"),
            Some(AptProgressStatus::Progress {
                percent: 10,
                message: "Downloading file 1 of 3".to_string(),
            })
        );
    }

    #[test]
    fn rejects_malformed_structured_percentage() {
        assert_eq!(
            parse_apt_progress_line("pmstatus:curl:not-a-number:Unpacking curl"),
            None
        );
    }
}
