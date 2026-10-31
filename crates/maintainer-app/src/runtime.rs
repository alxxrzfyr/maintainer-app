//! Process-wide services: database, settings, and metric sampling.
//!
//! Database failure is surfaced as a capability state — the app still starts.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError, TrySendError};
use std::thread;
#[cfg(test)]
use std::time::Instant;

use maintainer_core::error::MaintainerError;
use maintainer_core::jobs::JobExecutor;
use maintainer_core::metrics::{SystemCollector, SystemSnapshot};
use maintainer_core::persistence::database::Database;
use maintainer_core::settings::{Settings, SettingsRepository, SqliteSettingsRepository};

use crate::helper_client::{SharedHelperClient, create_helper_client};
use crate::monitoring::MonitoringState;
use crate::state::SettingsSnapshot;

type DatabaseInitResult = Result<(Arc<Database>, SettingsSnapshot), MaintainerError>;

enum SettingsWrite {
    Save { revision: u64, settings: Settings },
    Clear { revision: u64 },
}

pub struct SettingsWriteResult {
    pub cleared: bool,
    pub reset_defaults: bool,
    pub result: Result<u64, String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SettingsCloseState {
    Pending,
    Ready,
    Failed(String),
}

struct SettingsCompletion {
    revision: u64,
    outcome: SettingsWriteResult,
}

struct SettingsWriter {
    sender: SyncSender<SettingsWrite>,
    receiver: Receiver<SettingsCompletion>,
    pending: Option<SettingsWrite>,
    revision: u64,
    clearing: bool,
    disconnected_reported: bool,
    completed_revision: u64,
    latest_failure: Option<String>,
}

impl SettingsWriter {
    fn start(database: Arc<Database>) -> Result<Self, String> {
        Self::start_with(move |write| match write {
            SettingsWrite::Save { settings, .. } => (
                false,
                SqliteSettingsRepository::new(&database)
                    .save(settings)
                    .map(|()| 0)
                    .map_err(|_| "Settings could not be saved to the local database.".into()),
            ),
            SettingsWrite::Clear { .. } => match database.clear_user_data() {
                Ok(deleted) => (
                    true,
                    SqliteSettingsRepository::new(&database)
                        .save(&Settings::default())
                        .map(|()| deleted)
                        .map_err(|_| {
                            "Data was cleared, but default settings could not be saved.".into()
                        }),
                ),
                Err(_) => (false, Err("Local data could not be cleared.".into())),
            },
        })
    }

    fn start_with(
        mut persist: impl FnMut(&SettingsWrite) -> (bool, Result<u64, String>) + Send + 'static,
    ) -> Result<Self, String> {
        let (sender, requests) = mpsc::sync_channel::<SettingsWrite>(1);
        let (results, receiver) = mpsc::sync_channel(1);
        thread::Builder::new()
            .name("settings-writer".into())
            .spawn(move || {
                while let Ok(write) = requests.recv() {
                    let (revision, cleared) = match &write {
                        SettingsWrite::Save { revision, .. } => (*revision, false),
                        SettingsWrite::Clear { revision } => (*revision, true),
                    };
                    let (reset_defaults, result) = persist(&write);
                    if results
                        .send(SettingsCompletion {
                            revision,
                            outcome: SettingsWriteResult {
                                cleared,
                                reset_defaults,
                                result,
                            },
                        })
                        .is_err()
                    {
                        break;
                    }
                }
            })
            .map_err(|_| "Settings persistence worker could not start.".to_string())?;
        Ok(Self {
            sender,
            receiver,
            pending: None,
            revision: 0,
            clearing: false,
            disconnected_reported: false,
            completed_revision: 0,
            latest_failure: None,
        })
    }

    fn submit(&mut self, settings: Option<Settings>) -> Result<(), String> {
        self.revision += 1;
        self.latest_failure = None;
        self.clearing = settings.is_none();
        self.pending = Some(match settings {
            Some(settings) => SettingsWrite::Save {
                revision: self.revision,
                settings,
            },
            None => SettingsWrite::Clear {
                revision: self.revision,
            },
        });
        self.flush()
    }

    fn flush(&mut self) -> Result<(), String> {
        if let Some(write) = self.pending.take() {
            match self.sender.try_send(write) {
                Ok(()) => {}
                Err(TrySendError::Full(write)) => self.pending = Some(write),
                Err(TrySendError::Disconnected(_)) => {
                    let error = "Settings persistence worker stopped.".to_string();
                    self.latest_failure = Some(error.clone());
                    return Err(error);
                }
            }
        }
        Ok(())
    }

    fn close_state(&self, deadline_reached: bool) -> SettingsCloseState {
        if let Some(error) = &self.latest_failure {
            SettingsCloseState::Failed(error.clone())
        } else if self.pending.is_none() && self.completed_revision == self.revision {
            SettingsCloseState::Ready
        } else if deadline_reached {
            SettingsCloseState::Failed("Local writes have not finished. Exiting may lose recent settings or leave data clearing incomplete.".into())
        } else {
            SettingsCloseState::Pending
        }
    }

    fn poll(&mut self) -> Option<SettingsWriteResult> {
        let completion = match self.receiver.try_recv() {
            Ok(completion) => {
                self.completed_revision = self.completed_revision.max(completion.revision);
                if completion.revision == self.revision {
                    self.latest_failure = completion.outcome.result.as_ref().err().cloned();
                    Some(completion.outcome)
                } else {
                    None
                }
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) if !self.disconnected_reported => {
                self.disconnected_reported = true;
                self.latest_failure = Some("Settings persistence worker stopped.".into());
                return Some(SettingsWriteResult {
                    cleared: self.clearing,
                    reset_defaults: false,
                    result: Err("Settings persistence worker stopped.".into()),
                });
            }
            Err(TryRecvError::Disconnected) => return None,
        };
        if let Err(error) = self.flush() {
            self.disconnected_reported = true;
            return Some(SettingsWriteResult {
                cleared: self.clearing,
                reset_defaults: false,
                result: Err(error),
            });
        }
        completion
    }
}

/// Shared handle to the runtime for use in GTK callbacks.
pub type RuntimeHandle = Rc<RefCell<Runtime>>;

/// Process-wide runtime services.
pub struct Runtime {
    /// Database handle. `None` if open failed at startup.
    pub db: Option<Arc<Database>>,
    /// Database failure reason, if applicable.
    pub db_failure: Option<String>,
    /// Whether database initialization is still running.
    pub db_loading: bool,
    /// Current settings snapshot.
    pub settings: SettingsSnapshot,
    /// Live monitoring state.
    pub monitoring: MonitoringState,
    /// Shared job executor for on-demand work (bounded concurrency).
    pub executor: Arc<JobExecutor>,
    /// D-Bus client for the privileged helper.
    pub helper: SharedHelperClient,
    collector: SystemCollector,
    database_init_rx: Option<Receiver<DatabaseInitResult>>,
    settings_writer: Option<SettingsWriter>,
    pub settings_write_failure: Option<String>,
    pub clearing_settings: bool,
    unsaved_settings: Option<String>,
}

impl Runtime {
    /// Initialize the runtime: open database, load settings, prepare collectors.
    /// Never panics — database failures are recorded as capability states.
    pub fn init() -> RuntimeHandle {
        let (database_tx, database_rx) = mpsc::sync_channel(1);
        let database_worker_started = thread::Builder::new()
            .name("database-init-worker".into())
            .spawn(move || {
                let result = Database::open_default().map(|database| {
                    let database = Arc::new(database);
                    let settings = load_settings_from_db(Some(&database));
                    (database, settings)
                });
                let _ = database_tx.send(result);
            })
            .is_ok();
        let database_init_rx = database_worker_started.then_some(database_rx);
        let db_failure = (!database_worker_started)
            .then(|| "Database initialization worker could not start".into());
        let settings = SettingsSnapshot::default();

        let executor = Arc::new(JobExecutor::new(4).unwrap_or_else(|_| JobExecutor::unavailable()));

        let helper = create_helper_client();

        Rc::new(RefCell::new(Runtime {
            db: None,
            db_failure,
            db_loading: database_worker_started,
            settings,
            monitoring: MonitoringState::default(),
            executor,
            helper,
            collector: SystemCollector::new(),
            database_init_rx,
            settings_writer: None,
            settings_write_failure: None,
            clearing_settings: false,
            unsaved_settings: None,
        }))
    }

    /// Apply the background database result without blocking the main thread.
    /// Returns the saved theme when initialization succeeds.
    pub fn poll_database_initialization(&mut self) -> Option<crate::state::Theme> {
        let result = self.database_init_rx.as_ref()?.try_recv();
        match result {
            Ok(Ok((database, settings))) => {
                match SettingsWriter::start(database.clone()) {
                    Ok(writer) => self.settings_writer = Some(writer),
                    Err(error) => self.settings_write_failure = Some(error),
                }
                self.db = Some(database);
                self.settings = settings;
                self.monitoring.set_sampling_intervals(
                    self.settings.refresh_interval_secs,
                    self.settings.hidden_interval_secs,
                );
                self.monitoring.immediate_sample_pending = true;
                self.db_failure = None;
                self.db_loading = false;
                self.database_init_rx = None;
                Some(self.settings.theme)
            }
            Ok(Err(_)) | Err(TryRecvError::Disconnected) => {
                eprintln!("warning: local database unavailable");
                self.db_failure = Some("Local database unavailable".into());
                self.db_loading = false;
                self.database_init_rx = None;
                None
            }
            Err(TryRecvError::Empty) => None,
        }
    }

    /// Sample all system metrics and return a snapshot.
    pub fn collect_snapshot(&mut self) -> SystemSnapshot {
        self.collector.collect(true)
    }

    /// Submit a snapshot without waiting for SQLite or an occupied worker.
    pub fn save_settings(&mut self, settings: Settings) -> Result<(), String> {
        if self.clearing_settings {
            return Err("Local data is being cleared. Please wait.".into());
        }
        let result = match self.settings_writer.as_mut() {
            Some(writer) => writer.submit(Some(settings)),
            None => Err(self.settings_write_failure.clone().unwrap_or_else(|| {
                "Local database is unavailable; settings apply only to this session.".into()
            })),
        };
        self.unsaved_settings = result.as_ref().err().cloned();
        result
    }

    /// Inspect drain progress after polling writes, without waiting for the worker.
    pub fn settings_close_state(&self, deadline_reached: bool) -> SettingsCloseState {
        if let Some(error) = &self.unsaved_settings {
            return SettingsCloseState::Failed(error.clone());
        }
        match &self.settings_writer {
            Some(writer) => writer.close_state(deadline_reached),
            None => SettingsCloseState::Ready,
        }
    }

    /// Serialize clearing behind earlier writes and discard superseded pending edits.
    pub fn clear_settings_data(&mut self) -> Result<(), String> {
        if self.clearing_settings {
            return Err("Local data is already being cleared. Please wait.".into());
        }
        let writer = self
            .settings_writer
            .as_mut()
            .ok_or_else(|| "Local database is unavailable; no data was changed.".to_string())?;
        writer.submit(None)?;
        self.clearing_settings = true;
        Ok(())
    }

    pub fn poll_settings_writes(&mut self) -> Option<SettingsWriteResult> {
        let completion = self.settings_writer.as_mut()?.poll()?;
        if completion.cleared {
            self.clearing_settings = false;
            if completion.reset_defaults {
                self.settings = SettingsSnapshot::default();
                self.monitoring.set_sampling_intervals(
                    self.settings.refresh_interval_secs,
                    self.settings.hidden_interval_secs,
                );
            }
        }
        Some(completion)
    }

    /// Get the database reference, if available.
    pub fn database(&self) -> Option<&Arc<Database>> {
        self.db.as_ref()
    }
}

fn load_settings_from_db(db: Option<&Database>) -> SettingsSnapshot {
    let core_settings = match db {
        Some(d) => {
            let repo = SqliteSettingsRepository::new(d);
            repo.load().unwrap_or_default()
        }
        None => Settings::default(),
    };
    to_snapshot(&core_settings)
}

fn to_snapshot(s: &Settings) -> SettingsSnapshot {
    SettingsSnapshot {
        theme: match s.theme {
            maintainer_core::settings::ThemePreference::System => crate::state::Theme::System,
            maintainer_core::settings::ThemePreference::Light => crate::state::Theme::Light,
            maintainer_core::settings::ThemePreference::Dark => crate::state::Theme::Dark,
        },
        refresh_interval_secs: s.refresh_interval_secs,
        hidden_interval_secs: s.hidden_interval_secs,
        large_file_threshold_bytes: s.large_file_threshold_bytes,
        scan_exclusions: s.scan_exclusions.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pending_runtime(receiver: Receiver<DatabaseInitResult>) -> Runtime {
        Runtime {
            db: None,
            db_failure: None,
            db_loading: true,
            settings: SettingsSnapshot::default(),
            monitoring: MonitoringState::default(),
            executor: Arc::new(JobExecutor::new(1).unwrap()),
            helper: create_helper_client(),
            collector: SystemCollector::new(),
            database_init_rx: Some(receiver),
            settings_writer: None,
            settings_write_failure: None,
            clearing_settings: false,
            unsaved_settings: None,
        }
    }

    #[test]
    fn database_initialization_applies_saved_settings_once_without_waiting() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut runtime = pending_runtime(receiver);
        assert!(runtime.poll_database_initialization().is_none());
        assert!(runtime.db_loading);
        let saved = SettingsSnapshot {
            theme: crate::state::Theme::Dark,
            refresh_interval_secs: 7,
            hidden_interval_secs: 20,
            ..Default::default()
        };
        sender
            .send(Ok((Arc::new(Database::open_in_memory().unwrap()), saved)))
            .unwrap();
        assert_eq!(
            runtime.poll_database_initialization(),
            Some(crate::state::Theme::Dark)
        );
        assert!(!runtime.db_loading);
        assert!(runtime.db.is_some());
        assert_eq!(runtime.monitoring.interval.as_secs(), 7);
        assert!(runtime.poll_database_initialization().is_none());
    }

    #[test]
    fn database_worker_failure_preserves_monitoring() {
        let (sender, receiver) = mpsc::sync_channel(1);
        let mut runtime = pending_runtime(receiver);
        runtime.monitoring.overview.cpu_text = "25%".into();
        drop(sender);
        assert!(runtime.poll_database_initialization().is_none());
        assert!(!runtime.db_loading);
        assert!(runtime.db_failure.is_some());
        assert_eq!(runtime.monitoring.overview.cpu_text, "25%");
    }
    fn wait_for_write(writer: &mut SettingsWriter) -> SettingsWriteResult {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(result) = writer.poll() {
                return result;
            }
            assert!(Instant::now() < deadline, "settings worker did not finish");
            thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    #[test]
    fn settings_worker_coalesces_pending_edits_and_keeps_order() {
        let (started, observed) = mpsc::channel();
        let (release, gate) = mpsc::channel();
        let mut writer = SettingsWriter::start_with(move |write| {
            let SettingsWrite::Save { revision, .. } = write else {
                panic!("unexpected clear")
            };
            started.send(*revision).unwrap();
            if *revision == 1 {
                gate.recv().unwrap();
            }
            (false, Ok(0))
        })
        .unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        assert_eq!(
            observed
                .recv_timeout(std::time::Duration::from_secs(3))
                .unwrap(),
            1
        );
        writer.submit(Some(Settings::default())).unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        assert_eq!(writer.close_state(false), SettingsCloseState::Pending);
        release.send(()).unwrap();
        assert!(wait_for_write(&mut writer).result.is_ok());
        assert_eq!(writer.completed_revision, 4);
        assert_eq!(writer.close_state(false), SettingsCloseState::Ready);
        assert_eq!(observed.try_iter().collect::<Vec<_>>(), vec![2, 4]);
    }

    #[test]
    fn stale_settings_failure_does_not_replace_latest_result() {
        let mut writer = SettingsWriter::start_with(|write| {
            let SettingsWrite::Save { revision, .. } = write else {
                panic!("unexpected clear")
            };
            (
                false,
                if *revision == 1 {
                    Err("old failure".into())
                } else {
                    Ok(0)
                },
            )
        })
        .unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        assert!(wait_for_write(&mut writer).result.is_ok());
    }

    #[test]
    fn latest_settings_failure_is_reported() {
        let mut writer = SettingsWriter::start_with(|_| (false, Err("disk full".into()))).unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        assert_eq!(wait_for_write(&mut writer).result, Err("disk full".into()));
        assert_eq!(
            writer.close_state(false),
            SettingsCloseState::Failed("disk full".into())
        );
    }

    #[test]
    fn clear_data_is_serialized_after_saves_and_recreates_defaults() {
        let database = Arc::new(Database::open_in_memory().unwrap());
        let mut writer = SettingsWriter::start(database.clone()).unwrap();
        let settings = Settings {
            theme: maintainer_core::settings::ThemePreference::Dark,
            ..Default::default()
        };
        writer.submit(Some(settings)).unwrap();
        writer.submit(None).unwrap();
        assert_eq!(writer.close_state(false), SettingsCloseState::Pending);
        let completion = wait_for_write(&mut writer);
        assert_eq!(writer.close_state(false), SettingsCloseState::Ready);
        assert!(completion.cleared);
        assert!(completion.reset_defaults);
        assert!(completion.result.is_ok());
        assert_eq!(
            SqliteSettingsRepository::new(&database).load().unwrap(),
            Settings::default()
        );
    }

    #[test]
    fn unavailable_database_and_writer_start_failure_are_visible() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let mut runtime = pending_runtime(receiver);
        assert!(
            runtime
                .save_settings(Settings::default())
                .unwrap_err()
                .contains("unavailable")
        );
        runtime.settings_write_failure =
            Some("Settings persistence worker could not start.".into());
        assert_eq!(
            runtime.save_settings(Settings::default()).unwrap_err(),
            "Settings persistence worker could not start."
        );
    }

    #[test]
    fn disconnected_worker_reports_failure_once() {
        let mut writer =
            SettingsWriter::start_with(|_| panic!("simulated worker failure")).unwrap();
        writer.submit(None).unwrap();
        let result = wait_for_write(&mut writer);
        assert!(result.cleared);
        assert!(!result.reset_defaults);
        assert!(result.result.is_err());
        assert!(matches!(
            writer.close_state(false),
            SettingsCloseState::Failed(_)
        ));
        assert!(writer.poll().is_none());
    }
    #[test]
    fn clear_partial_failure_restores_session_defaults_and_unblocks_edits() {
        let (_sender, receiver) = mpsc::sync_channel(1);
        let mut runtime = pending_runtime(receiver);
        runtime.settings.theme = crate::state::Theme::Dark;
        runtime.settings.refresh_interval_secs = 12;
        runtime.settings_writer = Some(
            SettingsWriter::start_with(|_| {
                (
                    true,
                    Err("Data was cleared, but defaults could not be saved.".into()),
                )
            })
            .unwrap(),
        );
        runtime.clear_settings_data().unwrap();
        assert!(runtime.save_settings(Settings::default()).is_err());
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        loop {
            if let Some(result) = runtime.poll_settings_writes() {
                assert!(result.result.is_err());
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(std::time::Duration::from_millis(1));
        }
        assert!(!runtime.clearing_settings);
        assert_eq!(runtime.settings.theme, crate::state::Theme::System);
        assert_eq!(runtime.monitoring.interval.as_secs(), 2);
    }
    #[test]
    fn stalled_close_has_bounded_outcome_and_can_resume_after_cancelling_exit() {
        let (release, gate) = mpsc::channel();
        let mut writer = SettingsWriter::start_with(move |_| {
            gate.recv().unwrap();
            (false, Ok(0))
        })
        .unwrap();
        writer.submit(Some(Settings::default())).unwrap();
        assert_eq!(writer.close_state(false), SettingsCloseState::Pending);
        assert!(matches!(
            writer.close_state(true),
            SettingsCloseState::Failed(_)
        ));
        release.send(()).unwrap();
        assert!(wait_for_write(&mut writer).result.is_ok());
        assert_eq!(writer.close_state(true), SettingsCloseState::Ready);
    }

    #[test]
    fn stale_completion_advances_drain_without_reporting_latest_as_saved() {
        let (sender, _requests) = mpsc::sync_channel(1);
        let (results, receiver) = mpsc::sync_channel(1);
        let mut writer = SettingsWriter {
            sender,
            receiver,
            pending: None,
            revision: 2,
            clearing: false,
            disconnected_reported: false,
            completed_revision: 0,
            latest_failure: None,
        };
        results
            .send(SettingsCompletion {
                revision: 1,
                outcome: SettingsWriteResult {
                    cleared: false,
                    reset_defaults: false,
                    result: Ok(0),
                },
            })
            .unwrap();
        assert!(writer.poll().is_none());
        assert_eq!(writer.completed_revision, 1);
        assert_eq!(writer.close_state(false), SettingsCloseState::Pending);
        results
            .send(SettingsCompletion {
                revision: 2,
                outcome: SettingsWriteResult {
                    cleared: false,
                    reset_defaults: false,
                    result: Ok(0),
                },
            })
            .unwrap();
        assert!(writer.poll().unwrap().result.is_ok());
        assert_eq!(writer.close_state(false), SettingsCloseState::Ready);
    }
}
