//! Process-wide services: database, settings, and metric sampling.
//!
//! Database failure is surfaced as a capability state — the app still starts.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use maintainer_core::capability::{Capability, UnavailableReason};
use maintainer_core::jobs::JobExecutor;
use maintainer_core::metrics::disk::{DiskSnapshot, read_mounts_with_capacity};
use maintainer_core::metrics::network::NetCounters;
use maintainer_core::metrics::{
    CpuSample, NetworkSnapshot, SystemSnapshot, compute_cpu_snapshot, read_cpu_sample, read_memory,
    read_net_dev, read_processes, read_temperatures,
};
use maintainer_core::persistence::database::Database;
use maintainer_core::settings::{Settings, SettingsRepository, SqliteSettingsRepository};

use crate::helper_client::{SharedHelperClient, create_helper_client};
use crate::monitoring::MonitoringState;
use crate::state::SettingsSnapshot;

/// Shared handle to the runtime for use in GTK callbacks.
pub type RuntimeHandle = Rc<RefCell<Runtime>>;

/// Process-wide runtime services.
pub struct Runtime {
    /// Database handle. `None` if open failed at startup.
    pub db: Option<Arc<Database>>,
    /// Database failure reason, if applicable.
    pub db_failure: Option<String>,
    /// Current settings snapshot.
    pub settings: SettingsSnapshot,
    /// Live monitoring state.
    pub monitoring: MonitoringState,
    /// Shared job executor for on-demand work (bounded concurrency).
    pub executor: Arc<JobExecutor>,
    /// D-Bus client for the privileged helper.
    pub helper: SharedHelperClient,
    /// Previous CPU sample for delta computation.
    prev_cpu_sample: Option<CpuSample>,
    /// Previous network counters and timestamp for rate computation.
    prev_net: Option<(HashMap<String, NetCounters>, Instant)>,
}

impl Runtime {
    /// Initialize the runtime: open database, load settings, prepare collectors.
    /// Never panics — database failures are recorded as capability states.
    pub fn init() -> RuntimeHandle {
        let (db, db_failure) = match Database::open_default() {
            Ok(d) => (Some(Arc::new(d)), None),
            Err(e) => {
                eprintln!("warning: database unavailable: {e}");
                (None, Some(e.to_string()))
            }
        };

        let settings = load_settings_from_db(db.as_deref());

        let executor =
            Arc::new(JobExecutor::new(4).expect("max_workers > 0 is guaranteed at compile time"));

        let helper = create_helper_client();

        Rc::new(RefCell::new(Runtime {
            db,
            db_failure,
            settings,
            monitoring: MonitoringState::default(),
            executor,
            helper,
            prev_cpu_sample: None,
            prev_net: None,
        }))
    }

    /// Sample all system metrics and return a snapshot.
    pub fn collect_snapshot(&mut self) -> SystemSnapshot {
        let proc_path = Path::new("/proc");
        let sys_path = Path::new("/sys");
        let now = Instant::now();

        let cpu = self.collect_cpu(proc_path, now);
        let memory = collect_memory(proc_path);
        let disks = collect_disks(proc_path);
        let network = self.collect_network(proc_path, now);
        let temperatures = collect_temperatures(sys_path);
        let processes = collect_processes(proc_path);

        SystemSnapshot {
            cpu,
            memory,
            disks,
            network,
            temperatures,
            processes,
        }
    }

    fn collect_cpu(
        &mut self,
        proc_path: &Path,
        now: Instant,
    ) -> Capability<maintainer_core::metrics::CpuSnapshot> {
        match read_cpu_sample(proc_path, now) {
            Ok(sample) => {
                let snapshot = match &self.prev_cpu_sample {
                    Some(prev) => compute_cpu_snapshot(prev, &sample),
                    None => maintainer_core::metrics::CpuSnapshot {
                        total_percent: 0.0,
                        per_core_percent: vec![0.0; sample.cores.len()],
                    },
                };
                self.prev_cpu_sample = Some(sample);
                Capability::Available(snapshot)
            }
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "proc-stat-read",
                message: e.to_string(),
                remediation: None,
            }),
        }
    }

    fn collect_network(
        &mut self,
        proc_path: &Path,
        now: Instant,
    ) -> Capability<Vec<NetworkSnapshot>> {
        match read_net_dev(proc_path) {
            Ok(counters) => {
                let prev = self.prev_net.take();
                let snapshots: Vec<NetworkSnapshot> = counters
                    .iter()
                    .map(|(iface, c)| {
                        let (rx_rate, tx_rate) = match &prev {
                            Some((prev_counters, prev_time)) => {
                                let elapsed = now.duration_since(*prev_time).as_secs_f64();
                                if elapsed > 0.0 {
                                    if let Some(pc) = prev_counters.get(iface) {
                                        let rx = (c.rx_bytes.saturating_sub(pc.rx_bytes)) as f64
                                            / elapsed;
                                        let tx = (c.tx_bytes.saturating_sub(pc.tx_bytes)) as f64
                                            / elapsed;
                                        (Some(rx), Some(tx))
                                    } else {
                                        (None, None)
                                    }
                                } else {
                                    (None, None)
                                }
                            }
                            None => (None, None),
                        };
                        NetworkSnapshot {
                            interface: iface.clone(),
                            is_loopback: iface == "lo",
                            rx_bytes: c.rx_bytes,
                            tx_bytes: c.tx_bytes,
                            rx_packets: c.rx_packets,
                            tx_packets: c.tx_packets,
                            rx_errors: c.rx_errors,
                            tx_errors: c.tx_errors,
                            rx_bytes_per_sec: rx_rate,
                            tx_bytes_per_sec: tx_rate,
                        }
                    })
                    .collect();
                self.prev_net = Some((counters, now));
                Capability::Available(snapshots)
            }
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "net-dev-read",
                message: e.to_string(),
                remediation: None,
            }),
        }
    }

    /// Reload settings from database after a save.
    pub fn reload_settings(&mut self) {
        self.settings = load_settings_from_db(self.db.as_deref());
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

fn collect_memory(proc_path: &Path) -> Capability<maintainer_core::metrics::MemorySnapshot> {
    match read_memory(proc_path) {
        Ok(m) => Capability::Available(m),
        Err(e) => Capability::Unavailable(UnavailableReason {
            code: "meminfo-read",
            message: e.to_string(),
            remediation: None,
        }),
    }
}

fn collect_disks(proc_path: &Path) -> Capability<Vec<DiskSnapshot>> {
    match read_mounts_with_capacity(proc_path) {
        Ok(disks) => Capability::Available(disks),
        Err(e) => Capability::Unavailable(UnavailableReason {
            code: "mounts-read",
            message: e.to_string(),
            remediation: None,
        }),
    }
}

fn collect_temperatures(
    sys_path: &Path,
) -> Capability<Vec<maintainer_core::metrics::TemperatureReading>> {
    match read_temperatures(sys_path) {
        Ok(temps) => Capability::Available(temps),
        Err(e) => Capability::Unavailable(UnavailableReason {
            code: "hwmon-read",
            message: e.to_string(),
            remediation: None,
        }),
    }
}

fn collect_processes(
    proc_path: &Path,
) -> Capability<Vec<maintainer_core::metrics::ProcessSnapshot>> {
    match read_processes(proc_path) {
        Ok(procs) => Capability::Available(procs),
        Err(e) => Capability::Unavailable(UnavailableReason {
            code: "proc-read",
            message: e.to_string(),
            remediation: None,
        }),
    }
}
