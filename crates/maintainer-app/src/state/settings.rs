/// Theme selection for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Theme {
    /// Follow the system preference.
    #[default]
    System,
    /// Force light theme.
    Light,
    /// Force dark theme.
    Dark,
}

impl Theme {
    /// Convert to the string ID used in UI controls.
    pub fn as_str(self) -> &'static str {
        match self {
            Theme::System => "system",
            Theme::Light => "light",
            Theme::Dark => "dark",
        }
    }

    /// Parse from a string ID.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Theme::System),
            "light" => Some(Theme::Light),
            "dark" => Some(Theme::Dark),
            _ => None,
        }
    }
}

/// Immutable settings snapshot published to consumers.
///
/// Monitoring applies refresh changes at the next scheduling boundary;
/// active scans retain their original policy snapshot.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingsSnapshot {
    /// Theme preference.
    pub theme: Theme,
    /// Visible-window refresh interval in seconds (1–60).
    pub refresh_interval_secs: u32,
    /// Hidden-window minimum interval in seconds (10–60).
    pub hidden_interval_secs: u32,
    /// Large file threshold in bytes (>= 1 MiB).
    pub large_file_threshold_bytes: u64,
    /// Directories excluded from scans.
    pub scan_exclusions: Vec<String>,
}

impl Default for SettingsSnapshot {
    fn default() -> Self {
        Self {
            theme: Theme::System,
            refresh_interval_secs: 2,
            hidden_interval_secs: 10,
            large_file_threshold_bytes: 100 * 1024 * 1024,
            scan_exclusions: Vec::new(),
        }
    }
}

/// Intents that drive settings state transitions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SettingsIntent {
    /// Change the theme.
    SetTheme(Theme),
    /// Change the visible-window refresh interval.
    SetRefreshInterval(u32),
    /// Change the hidden-window refresh interval.
    SetHiddenInterval(u32),
    /// Change the large file threshold (in bytes).
    SetLargeFileThreshold(u64),
    /// Add a scan exclusion path.
    AddExclusion(String),
    /// Remove a scan exclusion by index.
    RemoveExclusion(usize),
    /// Request deletion of all history (triggers confirmation flow).
    RequestDeleteAllHistory,
    /// Request deletion of all application data (triggers confirmation flow).
    RequestDeleteAllData,
}

/// State managing the current settings and pending confirmations.
#[derive(Debug, Clone, Default)]
pub struct SettingsState {
    /// Current settings snapshot.
    pub snapshot: SettingsSnapshot,
    /// Whether a "delete all history" confirmation is pending.
    pub confirm_delete_history: bool,
    /// Whether a "delete all data" confirmation is pending.
    pub confirm_delete_all_data: bool,
    /// Whether settings have been modified since last save.
    pub dirty: bool,
}

impl SettingsState {
    /// Create from a loaded snapshot.
    pub fn from_snapshot(snapshot: SettingsSnapshot) -> Self {
        Self {
            snapshot,
            confirm_delete_history: false,
            confirm_delete_all_data: false,
            dirty: false,
        }
    }

    /// Apply a settings intent. Returns `true` if state changed.
    pub fn reduce(&mut self, intent: SettingsIntent) -> bool {
        match intent {
            SettingsIntent::SetTheme(theme) => {
                if self.snapshot.theme == theme {
                    return false;
                }
                self.snapshot.theme = theme;
                self.dirty = true;
                true
            }
            SettingsIntent::SetRefreshInterval(secs) => {
                let clamped = secs.clamp(1, 60);
                if self.snapshot.refresh_interval_secs == clamped {
                    return false;
                }
                self.snapshot.refresh_interval_secs = clamped;
                self.dirty = true;
                true
            }
            SettingsIntent::SetHiddenInterval(secs) => {
                let clamped = secs.clamp(10, 60);
                if self.snapshot.hidden_interval_secs == clamped {
                    return false;
                }
                self.snapshot.hidden_interval_secs = clamped;
                self.dirty = true;
                true
            }
            SettingsIntent::SetLargeFileThreshold(bytes) => {
                let min_bytes = 1024 * 1024;
                let validated = if bytes < min_bytes { min_bytes } else { bytes };
                if self.snapshot.large_file_threshold_bytes == validated {
                    return false;
                }
                self.snapshot.large_file_threshold_bytes = validated;
                self.dirty = true;
                true
            }
            SettingsIntent::AddExclusion(path) => {
                let canonical = path.trim().to_string();
                if canonical.is_empty() {
                    return false;
                }
                if self.snapshot.scan_exclusions.contains(&canonical) {
                    return false;
                }
                self.snapshot.scan_exclusions.push(canonical);
                self.dirty = true;
                true
            }
            SettingsIntent::RemoveExclusion(idx) => {
                if idx >= self.snapshot.scan_exclusions.len() {
                    return false;
                }
                self.snapshot.scan_exclusions.remove(idx);
                self.dirty = true;
                true
            }
            SettingsIntent::RequestDeleteAllHistory => {
                self.confirm_delete_history = true;
                true
            }
            SettingsIntent::RequestDeleteAllData => {
                self.confirm_delete_all_data = true;
                true
            }
        }
    }
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn default_snapshot_values() {
        let s = SettingsSnapshot::default();
        assert_eq!(s.theme, Theme::System);
        assert_eq!(s.refresh_interval_secs, 2);
        assert_eq!(s.hidden_interval_secs, 10);
        assert_eq!(s.large_file_threshold_bytes, 100 * 1024 * 1024);
        assert!(s.scan_exclusions.is_empty());
    }

    #[test]
    fn set_theme_marks_dirty() {
        let mut state = SettingsState::default();
        assert!(!state.dirty);
        state.reduce(SettingsIntent::SetTheme(Theme::Dark));
        assert!(state.dirty);
        assert_eq!(state.snapshot.theme, Theme::Dark);
    }

    #[test]
    fn set_same_theme_returns_false() {
        let mut state = SettingsState::default();
        let changed = state.reduce(SettingsIntent::SetTheme(Theme::System));
        assert!(!changed);
        assert!(!state.dirty);
    }

    #[test]
    fn refresh_interval_clamped_to_range() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::SetRefreshInterval(0));
        assert_eq!(state.snapshot.refresh_interval_secs, 1);

        state.reduce(SettingsIntent::SetRefreshInterval(100));
        assert_eq!(state.snapshot.refresh_interval_secs, 60);

        state.reduce(SettingsIntent::SetRefreshInterval(30));
        assert_eq!(state.snapshot.refresh_interval_secs, 30);
    }

    #[test]
    fn hidden_interval_minimum_is_10() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::SetHiddenInterval(5));
        assert_eq!(state.snapshot.hidden_interval_secs, 10);

        state.reduce(SettingsIntent::SetHiddenInterval(45));
        assert_eq!(state.snapshot.hidden_interval_secs, 45);
    }

    #[test]
    fn large_file_threshold_minimum_1mib() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::SetLargeFileThreshold(100));
        assert_eq!(state.snapshot.large_file_threshold_bytes, 1024 * 1024);

        state.reduce(SettingsIntent::SetLargeFileThreshold(50 * 1024 * 1024));
        assert_eq!(state.snapshot.large_file_threshold_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn add_and_remove_exclusion() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::AddExclusion("/tmp".into()));
        assert_eq!(state.snapshot.scan_exclusions, vec!["/tmp"]);

        let changed = state.reduce(SettingsIntent::AddExclusion("/tmp".into()));
        assert!(!changed);

        state.reduce(SettingsIntent::RemoveExclusion(0));
        assert!(state.snapshot.scan_exclusions.is_empty());
    }

    #[test]
    fn empty_exclusion_rejected() {
        let mut state = SettingsState::default();
        let changed = state.reduce(SettingsIntent::AddExclusion("  ".into()));
        assert!(!changed);
    }

    #[test]
    fn remove_exclusion_out_of_bounds_rejected() {
        let mut state = SettingsState::default();
        let changed = state.reduce(SettingsIntent::RemoveExclusion(0));
        assert!(!changed);
    }

    #[test]
    fn request_delete_history_sets_flag() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::RequestDeleteAllHistory);
        assert!(state.confirm_delete_history);
    }

    #[test]
    fn request_delete_all_data_sets_flag() {
        let mut state = SettingsState::default();
        state.reduce(SettingsIntent::RequestDeleteAllData);
        assert!(state.confirm_delete_all_data);
    }

    #[test]
    fn theme_round_trip() {
        for theme in [Theme::System, Theme::Light, Theme::Dark] {
            let s = theme.as_str();
            assert_eq!(Theme::parse(s), Some(theme));
        }
        assert_eq!(Theme::parse("invalid"), None);
    }
}
