//! Application settings with validation and persistence.
//!
//! Settings are stored as key-value pairs in SQLite. Values are validated
//! on read and write; invalid stored values fall back to defaults with a
//! warning logged to stderr.

use crate::error::MaintainerError;
use crate::persistence::database::Database;

/// Minimum allowed refresh interval in seconds.
const MIN_REFRESH_INTERVAL_SECS: u32 = 1;
/// Maximum allowed refresh interval in seconds.
const MAX_REFRESH_INTERVAL_SECS: u32 = 60;
/// Default refresh interval in seconds.
const DEFAULT_REFRESH_INTERVAL_SECS: u32 = 2;
/// Default hidden-window interval in seconds.
const DEFAULT_HIDDEN_INTERVAL_SECS: u32 = 10;
/// Minimum hidden-window interval in seconds.
const MIN_HIDDEN_INTERVAL_SECS: u32 = 10;
/// Default large-file threshold in bytes (100 MiB).
const DEFAULT_LARGE_FILE_THRESHOLD_BYTES: u64 = 100 * 1024 * 1024;

// Setting keys.
const KEY_REFRESH_INTERVAL: &str = "refresh_interval_secs";
const KEY_HIDDEN_INTERVAL: &str = "hidden_interval_secs";
const KEY_SCAN_EXCLUSIONS: &str = "scan_exclusions";
const KEY_LARGE_FILE_THRESHOLD: &str = "large_file_threshold_bytes";
const KEY_THEME: &str = "theme";

/// Theme selection for the application.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ThemePreference {
    /// Follow the system preference.
    #[default]
    System,
    /// Force light theme.
    Light,
    /// Force dark theme.
    Dark,
}

impl ThemePreference {
    /// Convert to the storage string.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::System => "system",
            Self::Light => "light",
            Self::Dark => "dark",
        }
    }

    /// Parse from a storage string.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "system" => Some(Self::System),
            "light" => Some(Self::Light),
            "dark" => Some(Self::Dark),
            _ => None,
        }
    }
}

/// Application settings with validated fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Settings {
    /// Theme preference.
    pub theme: ThemePreference,
    /// Monitoring refresh interval, clamped to 1–60 seconds.
    pub refresh_interval_secs: u32,
    /// Hidden-window minimum interval, clamped to 10–60 seconds.
    pub hidden_interval_secs: u32,
    /// Directories excluded from scans, stored as newline-separated paths.
    pub scan_exclusions: Vec<String>,
    /// Byte threshold above which a file is considered "large".
    pub large_file_threshold_bytes: u64,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::System,
            refresh_interval_secs: DEFAULT_REFRESH_INTERVAL_SECS,
            hidden_interval_secs: DEFAULT_HIDDEN_INTERVAL_SECS,
            scan_exclusions: Vec::new(),
            large_file_threshold_bytes: DEFAULT_LARGE_FILE_THRESHOLD_BYTES,
        }
    }
}

/// Trait for settings persistence operations.
pub trait SettingsRepository: Send + Sync {
    /// Load all settings, falling back to defaults for missing or invalid values.
    fn load(&self) -> Result<Settings, MaintainerError>;
    /// Save all settings, validating before persisting.
    fn save(&self, settings: &Settings) -> Result<(), MaintainerError>;
}

/// SQLite-backed implementation of [`SettingsRepository`].
pub struct SqliteSettingsRepository<'a> {
    db: &'a Database,
}

impl<'a> SqliteSettingsRepository<'a> {
    /// Create a new repository backed by the given database.
    pub fn new(db: &'a Database) -> Self {
        Self { db }
    }

    /// Read a raw setting value from the database.
    fn get_raw(&self, key: &str) -> Result<Option<String>, MaintainerError> {
        self.db.with_connection(|conn| {
            let mut stmt = conn
                .prepare("SELECT value FROM settings WHERE key = ?1")
                .map_err(|e| MaintainerError::Internal {
                    context: "preparing settings query".into(),
                    message: e.to_string(),
                })?;

            let result = stmt
                .query_row(rusqlite::params![key], |row| row.get(0))
                .optional()
                .map_err(|e| MaintainerError::Internal {
                    context: format!("reading setting '{key}'"),
                    message: e.to_string(),
                })?;

            Ok(result)
        })
    }

    /// Write a setting value, inserting or replacing.
    fn set_raw(&self, key: &str, value: &str) -> Result<(), MaintainerError> {
        self.db.with_connection(|conn| {
            conn.execute(
                "INSERT OR REPLACE INTO settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .map_err(|e| MaintainerError::Internal {
                context: format!("saving setting '{key}'"),
                message: e.to_string(),
            })?;
            Ok(())
        })
    }
}

impl SettingsRepository for SqliteSettingsRepository<'_> {
    fn load(&self) -> Result<Settings, MaintainerError> {
        let mut settings = Settings::default();

        // Theme.
        if let Some(raw) = self.get_raw(KEY_THEME)? {
            match ThemePreference::parse(&raw) {
                Some(t) => settings.theme = t,
                None => {
                    eprintln!("warning: unknown theme '{raw}', using default");
                }
            }
        }

        // Refresh interval.
        if let Some(raw) = self.get_raw(KEY_REFRESH_INTERVAL)? {
            match raw.parse::<u32>() {
                Ok(v) if (MIN_REFRESH_INTERVAL_SECS..=MAX_REFRESH_INTERVAL_SECS).contains(&v) => {
                    settings.refresh_interval_secs = v;
                }
                Ok(v) => {
                    eprintln!(
                        "warning: stored refresh_interval_secs {v} out of range [{MIN_REFRESH_INTERVAL_SECS}–{MAX_REFRESH_INTERVAL_SECS}], using default"
                    );
                }
                Err(_) => {
                    eprintln!("warning: malformed refresh_interval_secs '{raw}', using default");
                }
            }
        }

        // Hidden interval.
        if let Some(raw) = self.get_raw(KEY_HIDDEN_INTERVAL)? {
            match raw.parse::<u32>() {
                Ok(v) if (MIN_HIDDEN_INTERVAL_SECS..=MAX_REFRESH_INTERVAL_SECS).contains(&v) => {
                    settings.hidden_interval_secs = v;
                }
                Ok(v) => {
                    eprintln!(
                        "warning: stored hidden_interval_secs {v} out of range [{MIN_HIDDEN_INTERVAL_SECS}–{MAX_REFRESH_INTERVAL_SECS}], using default"
                    );
                }
                Err(_) => {
                    eprintln!("warning: malformed hidden_interval_secs '{raw}', using default");
                }
            }
        }

        // Scan exclusions.
        if let Some(raw) = self.get_raw(KEY_SCAN_EXCLUSIONS)? {
            settings.scan_exclusions = raw
                .lines()
                .filter(|l| !l.is_empty())
                .map(String::from)
                .collect();
        }

        // Large file threshold.
        if let Some(raw) = self.get_raw(KEY_LARGE_FILE_THRESHOLD)? {
            match raw.parse::<u64>() {
                Ok(v) if v > 0 => {
                    settings.large_file_threshold_bytes = v;
                }
                Ok(_) => {
                    eprintln!("warning: stored large_file_threshold_bytes is zero, using default");
                }
                Err(_) => {
                    eprintln!(
                        "warning: malformed large_file_threshold_bytes '{raw}', using default"
                    );
                }
            }
        }

        Ok(settings)
    }

    fn save(&self, settings: &Settings) -> Result<(), MaintainerError> {
        // Theme.
        self.set_raw(KEY_THEME, settings.theme.as_str())?;

        // Clamp refresh interval to valid range before saving.
        let interval = settings
            .refresh_interval_secs
            .clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS);
        self.set_raw(KEY_REFRESH_INTERVAL, &interval.to_string())?;

        // Clamp hidden interval.
        let hidden = settings
            .hidden_interval_secs
            .clamp(MIN_HIDDEN_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS);
        self.set_raw(KEY_HIDDEN_INTERVAL, &hidden.to_string())?;

        let exclusions = settings.scan_exclusions.join("\n");
        self.set_raw(KEY_SCAN_EXCLUSIONS, &exclusions)?;

        let threshold = if settings.large_file_threshold_bytes == 0 {
            DEFAULT_LARGE_FILE_THRESHOLD_BYTES
        } else {
            settings.large_file_threshold_bytes
        };
        self.set_raw(KEY_LARGE_FILE_THRESHOLD, &threshold.to_string())?;

        Ok(())
    }
}

/// Clamp a refresh interval to the valid range.
pub fn clamp_refresh_interval(secs: u32) -> u32 {
    secs.clamp(MIN_REFRESH_INTERVAL_SECS, MAX_REFRESH_INTERVAL_SECS)
}

// Re-export the `optional` extension for rusqlite.
trait OptionalExt<T> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error>;
}

impl<T> OptionalExt<T> for std::result::Result<T, rusqlite::Error> {
    fn optional(self) -> std::result::Result<Option<T>, rusqlite::Error> {
        match self {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_db() -> Database {
        Database::open_in_memory().unwrap()
    }

    #[test]
    fn default_settings_on_empty_database() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);
        let settings = repo.load().unwrap();

        assert_eq!(settings.theme, ThemePreference::System);
        assert_eq!(settings.refresh_interval_secs, 2);
        assert_eq!(settings.hidden_interval_secs, 10);
        assert!(settings.scan_exclusions.is_empty());
        assert_eq!(settings.large_file_threshold_bytes, 100 * 1024 * 1024);
    }

    #[test]
    fn save_and_load_round_trip() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);

        let settings = Settings {
            theme: ThemePreference::Dark,
            refresh_interval_secs: 5,
            hidden_interval_secs: 15,
            scan_exclusions: vec!["/tmp".into(), "/proc".into(), "/sys".into()],
            large_file_threshold_bytes: 50 * 1024 * 1024,
        };
        repo.save(&settings).unwrap();

        let loaded = repo.load().unwrap();
        assert_eq!(loaded.theme, ThemePreference::Dark);
        assert_eq!(loaded.refresh_interval_secs, 5);
        assert_eq!(loaded.hidden_interval_secs, 15);
        assert_eq!(loaded.scan_exclusions, vec!["/tmp", "/proc", "/sys"]);
        assert_eq!(loaded.large_file_threshold_bytes, 50 * 1024 * 1024);
    }

    #[test]
    fn refresh_interval_clamped_on_save() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);

        // Too low.
        let settings = Settings {
            refresh_interval_secs: 0,
            ..Settings::default()
        };
        repo.save(&settings).unwrap();
        let loaded = repo.load().unwrap();
        assert_eq!(loaded.refresh_interval_secs, 1);

        // Too high.
        let settings = Settings {
            refresh_interval_secs: 120,
            ..Settings::default()
        };
        repo.save(&settings).unwrap();
        let loaded = repo.load().unwrap();
        assert_eq!(loaded.refresh_interval_secs, 60);
    }

    #[test]
    fn hidden_interval_clamped_on_save() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);

        // Too low — clamped to 10.
        let settings = Settings {
            hidden_interval_secs: 5,
            ..Settings::default()
        };
        repo.save(&settings).unwrap();
        let loaded = repo.load().unwrap();
        assert_eq!(loaded.hidden_interval_secs, 10);

        // Too high — clamped to 60.
        let settings = Settings {
            hidden_interval_secs: 120,
            ..Settings::default()
        };
        repo.save(&settings).unwrap();
        let loaded = repo.load().unwrap();
        assert_eq!(loaded.hidden_interval_secs, 60);
    }

    #[test]
    fn theme_round_trip() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);

        for theme in [
            ThemePreference::System,
            ThemePreference::Light,
            ThemePreference::Dark,
        ] {
            let settings = Settings {
                theme,
                ..Settings::default()
            };
            repo.save(&settings).unwrap();
            let loaded = repo.load().unwrap();
            assert_eq!(loaded.theme, theme);
        }
    }

    #[test]
    fn malformed_stored_value_falls_back_to_default() {
        let db = test_db();
        // Manually insert a garbage value.
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('refresh_interval_secs', 'not_a_number')",
                [],
            )
            .map_err(|e| MaintainerError::Internal {
                context: "test setup".into(),
                message: e.to_string(),
            })
        })
        .unwrap();

        let repo = SqliteSettingsRepository::new(&db);
        let settings = repo.load().unwrap();
        // Should fall back to default without panic.
        assert_eq!(settings.refresh_interval_secs, 2);
    }

    #[test]
    fn out_of_range_stored_value_falls_back_to_default() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('refresh_interval_secs', '999')",
                [],
            )
            .map_err(|e| MaintainerError::Internal {
                context: "test setup".into(),
                message: e.to_string(),
            })
        })
        .unwrap();

        let repo = SqliteSettingsRepository::new(&db);
        let settings = repo.load().unwrap();
        assert_eq!(settings.refresh_interval_secs, 2);
    }

    #[test]
    fn invalid_theme_falls_back_to_default() {
        let db = test_db();
        db.with_connection(|conn| {
            conn.execute(
                "INSERT INTO settings (key, value) VALUES ('theme', 'neon')",
                [],
            )
            .map_err(|e| MaintainerError::Internal {
                context: "test setup".into(),
                message: e.to_string(),
            })
        })
        .unwrap();

        let repo = SqliteSettingsRepository::new(&db);
        let settings = repo.load().unwrap();
        assert_eq!(settings.theme, ThemePreference::System);
    }

    #[test]
    fn empty_exclusions_round_trip() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);

        let settings = Settings {
            scan_exclusions: Vec::new(),
            ..Settings::default()
        };
        repo.save(&settings).unwrap();

        let loaded = repo.load().unwrap();
        assert!(loaded.scan_exclusions.is_empty());
    }

    #[test]
    fn clamp_refresh_interval_function() {
        assert_eq!(clamp_refresh_interval(0), 1);
        assert_eq!(clamp_refresh_interval(1), 1);
        assert_eq!(clamp_refresh_interval(30), 30);
        assert_eq!(clamp_refresh_interval(60), 60);
        assert_eq!(clamp_refresh_interval(61), 60);
    }

    #[test]
    fn restart_round_trip_all_fields() {
        let db = test_db();
        let repo = SqliteSettingsRepository::new(&db);

        let settings = Settings {
            theme: ThemePreference::Light,
            refresh_interval_secs: 10,
            hidden_interval_secs: 30,
            scan_exclusions: vec!["/mnt/external".into()],
            large_file_threshold_bytes: 200 * 1024 * 1024,
        };
        repo.save(&settings).unwrap();

        // Simulate restart by creating a new repository on the same DB.
        let repo2 = SqliteSettingsRepository::new(&db);
        let loaded = repo2.load().unwrap();
        assert_eq!(loaded, settings);
    }
}
