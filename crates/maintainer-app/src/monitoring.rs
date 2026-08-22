//! Monitoring service state: view models, sample ring buffers, and attention logic.
//!
//! All types here are pure data — no GTK objects. They drive the Overview and
//! System Monitor pages without requiring a display server for testing.

use std::collections::VecDeque;
use std::time::Duration;

use maintainer_core::capability::Capability;
use maintainer_core::metrics::{
    CpuSnapshot, DiskSnapshot, MemorySnapshot, NetworkSnapshot, ProcessSnapshot, SystemSnapshot,
    TemperatureReading,
};

/// Maximum samples retained in ring buffers for sparkline rendering.
const MAX_SAMPLES: usize = 60;

/// Default sampling interval when the window is visible.
pub const VISIBLE_INTERVAL: Duration = Duration::from_secs(2);

/// Sampling interval when the window is hidden (power-saving).
pub const HIDDEN_INTERVAL: Duration = Duration::from_secs(10);

/// Sort order for the process table.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ProcessSort {
    #[default]
    Cpu,
    Memory,
    Name,
    Pid,
}

/// Severity level for overview attention items.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AttentionSeverity {
    /// Informational — no action required.
    Info,
    /// Warning — user should be aware.
    Warning,
    /// Critical — action recommended.
    Critical,
}

/// A single attention item shown on the Overview page.
#[derive(Debug, Clone, PartialEq)]
pub struct AttentionItem {
    /// Severity classification.
    pub severity: AttentionSeverity,
    /// Human-readable evidence text.
    pub evidence: String,
    /// Which page to navigate to for details.
    pub destination: crate::state::Page,
}

/// Ring buffer of f64 samples for sparkline rendering.
#[derive(Debug, Clone)]
pub struct SampleRing {
    samples: VecDeque<f64>,
}

impl SampleRing {
    /// Create a new empty ring buffer.
    pub fn new() -> Self {
        Self {
            samples: VecDeque::with_capacity(MAX_SAMPLES),
        }
    }

    /// Push a new sample, evicting the oldest if at capacity.
    pub fn push(&mut self, value: f64) {
        if self.samples.len() >= MAX_SAMPLES {
            self.samples.pop_front();
        }
        self.samples.push_back(value);
    }

    /// Get all samples as a slice (oldest first).
    pub fn samples(&self) -> &VecDeque<f64> {
        &self.samples
    }

    /// Number of samples currently stored.
    pub fn len(&self) -> usize {
        self.samples.len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }

    /// Most recent sample, if any.
    pub fn latest(&self) -> Option<f64> {
        self.samples.back().copied()
    }

    /// Clear all samples.
    pub fn clear(&mut self) {
        self.samples.clear();
    }
}

impl Default for SampleRing {
    fn default() -> Self {
        Self::new()
    }
}

/// Presentation state for the Overview page.
#[derive(Debug, Clone)]
pub struct OverviewViewModel {
    /// CPU usage sparkline (percentage 0–100).
    pub cpu_ring: SampleRing,
    /// Memory usage sparkline (percentage 0–100).
    pub memory_ring: SampleRing,
    /// Current CPU percentage text (e.g., "23%").
    pub cpu_text: String,
    /// Current memory percentage text (e.g., "4.2 / 16.0 GiB").
    pub memory_text: String,
    /// Current disk summary text.
    pub disk_text: String,
    /// Current network summary text.
    pub network_text: String,
    /// Attention items derived from explicit evidence.
    pub attention_items: Vec<AttentionItem>,
    /// Whether CPU data is available.
    pub cpu_available: bool,
    /// Whether memory data is available.
    pub memory_available: bool,
    /// Reason CPU is unavailable, if applicable.
    pub cpu_unavailable_reason: Option<String>,
    /// Reason memory is unavailable, if applicable.
    pub memory_unavailable_reason: Option<String>,
}

impl Default for OverviewViewModel {
    fn default() -> Self {
        Self {
            cpu_ring: SampleRing::new(),
            memory_ring: SampleRing::new(),
            cpu_text: "—".into(),
            memory_text: "—".into(),
            disk_text: "—".into(),
            network_text: "—".into(),
            attention_items: Vec::new(),
            cpu_available: false,
            memory_available: false,
            cpu_unavailable_reason: None,
            memory_unavailable_reason: None,
        }
    }
}

/// Presentation state for the System Monitor page.
#[derive(Debug, Clone)]
pub struct SystemViewModel {
    /// CPU data (latest snapshot).
    pub cpu: Option<CpuSnapshot>,
    /// Memory data (latest snapshot).
    pub memory: Option<MemorySnapshot>,
    /// Per-core CPU sparklines.
    pub per_core_rings: Vec<SampleRing>,
    /// Overall CPU ring.
    pub cpu_ring: SampleRing,
    /// Memory usage ring.
    pub memory_ring: SampleRing,
    /// Disk snapshots.
    pub disks: Vec<DiskSnapshot>,
    /// Network snapshots.
    pub networks: Vec<NetworkSnapshot>,
    /// Temperature readings.
    pub temperatures: Vec<TemperatureReading>,
    /// Process list (sorted).
    pub processes: Vec<ProcessSnapshot>,
    /// Current sort mode.
    pub process_sort: ProcessSort,
    /// Whether process data is available.
    pub processes_available: bool,
}

impl Default for SystemViewModel {
    fn default() -> Self {
        Self {
            cpu: None,
            memory: None,
            per_core_rings: Vec::new(),
            cpu_ring: SampleRing::new(),
            memory_ring: SampleRing::new(),
            disks: Vec::new(),
            networks: Vec::new(),
            temperatures: Vec::new(),
            processes: Vec::new(),
            process_sort: ProcessSort::default(),
            processes_available: false,
        }
    }
}

/// Combined monitoring state managed by the monitoring service.
#[derive(Debug, Clone)]
pub struct MonitoringState {
    pub overview: OverviewViewModel,
    pub system: SystemViewModel,
    /// Whether the window is currently visible (affects sampling interval).
    pub window_visible: bool,
    /// Current sampling interval.
    pub interval: Duration,
    /// Whether an immediate sample is pending (e.g., after becoming visible).
    pub immediate_sample_pending: bool,
}

impl Default for MonitoringState {
    fn default() -> Self {
        Self {
            overview: OverviewViewModel::default(),
            system: SystemViewModel::default(),
            window_visible: true,
            interval: VISIBLE_INTERVAL,
            immediate_sample_pending: false,
        }
    }
}

/// Typed messages sent from collectors to the main thread.
#[derive(Debug)]
pub enum MonitoringMessage {
    /// New system snapshot arrived.
    SnapshotReady(Box<SystemSnapshot>),
    /// Window visibility changed.
    VisibilityChanged(bool),
    /// User changed process sort order.
    SetProcessSort(ProcessSort),
}

impl MonitoringState {
    /// Apply a monitoring message and update state.
    /// Returns true if the overview changed, false otherwise.
    pub fn reduce(&mut self, msg: MonitoringMessage) -> bool {
        match msg {
            MonitoringMessage::SnapshotReady(snapshot) => {
                self.apply_snapshot(*snapshot);
                true
            }
            MonitoringMessage::VisibilityChanged(visible) => {
                self.window_visible = visible;
                if visible {
                    self.interval = VISIBLE_INTERVAL;
                    self.immediate_sample_pending = true;
                } else {
                    self.interval = HIDDEN_INTERVAL;
                    self.immediate_sample_pending = false;
                }
                false
            }
            MonitoringMessage::SetProcessSort(sort) => {
                if self.system.process_sort != sort {
                    self.system.process_sort = sort;
                    self.sort_processes();
                }
                false
            }
        }
    }

    fn apply_snapshot(&mut self, snapshot: SystemSnapshot) {
        self.apply_cpu(&snapshot.cpu);
        self.apply_memory(&snapshot.memory);
        self.apply_disks(&snapshot.disks);
        self.apply_network(&snapshot.network);
        self.apply_temperatures(&snapshot.temperatures);
        self.apply_processes(&snapshot.processes);
        self.derive_attention(&snapshot);
    }

    fn apply_cpu(&mut self, cpu: &Capability<CpuSnapshot>) {
        match cpu {
            Capability::Available(snap) => {
                self.overview.cpu_available = true;
                self.overview.cpu_unavailable_reason = None;
                self.overview.cpu_ring.push(snap.total_percent);
                self.overview.cpu_text = format!("{:.0}%", snap.total_percent);
                self.system.cpu_ring.push(snap.total_percent);

                while self.system.per_core_rings.len() < snap.per_core_percent.len() {
                    self.system.per_core_rings.push(SampleRing::new());
                }
                for (i, pct) in snap.per_core_percent.iter().enumerate() {
                    if let Some(ring) = self.system.per_core_rings.get_mut(i) {
                        ring.push(*pct);
                    }
                }

                self.system.cpu = Some(snap.clone());
            }
            Capability::Unavailable(reason) => {
                self.overview.cpu_available = false;
                self.overview.cpu_unavailable_reason = Some(reason.message.clone());
                self.overview.cpu_text = "Unavailable".into();
            }
            Capability::PermissionRequired(_) => {
                self.overview.cpu_available = false;
                self.overview.cpu_text = "Permission required".into();
            }
        }
    }

    fn apply_memory(&mut self, memory: &Capability<MemorySnapshot>) {
        match memory {
            Capability::Available(snap) => {
                self.overview.memory_available = true;
                self.overview.memory_unavailable_reason = None;
                let pct = if snap.total_bytes > 0 {
                    (snap.used_bytes as f64 / snap.total_bytes as f64) * 100.0
                } else {
                    0.0
                };
                self.overview.memory_ring.push(pct);
                self.system.memory_ring.push(pct);

                let used_gib = snap.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                let total_gib = snap.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
                self.overview.memory_text = format!("{used_gib:.1} / {total_gib:.1} GiB");

                self.system.memory = Some(snap.clone());
            }
            Capability::Unavailable(reason) => {
                self.overview.memory_available = false;
                self.overview.memory_unavailable_reason = Some(reason.message.clone());
                self.overview.memory_text = "Unavailable".into();
            }
            Capability::PermissionRequired(_) => {
                self.overview.memory_available = false;
                self.overview.memory_text = "Permission required".into();
            }
        }
    }

    fn apply_disks(&mut self, disks: &Capability<Vec<DiskSnapshot>>) {
        if let Capability::Available(snaps) = disks {
            self.system.disks = snaps.clone();
            let total: u64 = snaps.iter().map(|d| d.total_bytes).sum();
            let used: u64 = snaps.iter().map(|d| d.used_bytes).sum();
            if total > 0 {
                let used_gib = used as f64 / (1024.0 * 1024.0 * 1024.0);
                let total_gib = total as f64 / (1024.0 * 1024.0 * 1024.0);
                self.overview.disk_text = format!("{used_gib:.1} / {total_gib:.1} GiB used");
            } else {
                self.overview.disk_text = "No disks".into();
            }
        } else {
            self.overview.disk_text = "Unavailable".into();
            self.system.disks.clear();
        }
    }

    fn apply_network(&mut self, network: &Capability<Vec<NetworkSnapshot>>) {
        if let Capability::Available(snaps) = network {
            self.system.networks = snaps.clone();
            let total_rx: f64 = snaps.iter().filter_map(|n| n.rx_bytes_per_sec).sum();
            let total_tx: f64 = snaps.iter().filter_map(|n| n.tx_bytes_per_sec).sum();
            self.overview.network_text = format!(
                "↓ {}/s ↑ {}/s",
                format_bytes_rate(total_rx),
                format_bytes_rate(total_tx)
            );
        } else {
            self.overview.network_text = "Unavailable".into();
            self.system.networks.clear();
        }
    }

    fn apply_temperatures(&mut self, temps: &Capability<Vec<TemperatureReading>>) {
        if let Capability::Available(readings) = temps {
            self.system.temperatures = readings.clone();
        } else {
            self.system.temperatures.clear();
        }
    }

    fn apply_processes(&mut self, processes: &Capability<Vec<ProcessSnapshot>>) {
        match processes {
            Capability::Available(procs) => {
                self.system.processes = procs.clone();
                self.system.processes_available = true;
                self.sort_processes();
            }
            _ => {
                self.system.processes_available = false;
                self.system.processes.clear();
            }
        }
    }

    fn sort_processes(&mut self) {
        match self.system.process_sort {
            ProcessSort::Cpu => {
                self.system.processes.sort_by(|a, b| {
                    let a_cpu = a.utime_ticks + a.stime_ticks;
                    let b_cpu = b.utime_ticks + b.stime_ticks;
                    b_cpu.cmp(&a_cpu)
                });
            }
            ProcessSort::Memory => {
                self.system
                    .processes
                    .sort_by(|a, b| b.rss_bytes.cmp(&a.rss_bytes));
            }
            ProcessSort::Name => {
                self.system.processes.sort_by(|a, b| a.name.cmp(&b.name));
            }
            ProcessSort::Pid => {
                self.system.processes.sort_by_key(|p| p.identity.pid);
            }
        }
    }

    /// Derive attention items from explicit evidence.
    ///
    /// Rules:
    /// - Filesystem > 90% used → Critical
    /// - Filesystem > 80% used → Warning
    /// - Temperature critical (placeholder for SMART integration)
    fn derive_attention(&mut self, snapshot: &SystemSnapshot) {
        let mut items = Vec::new();

        if let Capability::Available(disks) = &snapshot.disks {
            for disk in disks {
                if disk.total_bytes == 0 {
                    continue;
                }
                let usage_pct = (disk.used_bytes as f64 / disk.total_bytes as f64) * 100.0;
                if usage_pct >= 90.0 {
                    items.push(AttentionItem {
                        severity: AttentionSeverity::Critical,
                        evidence: format!("{} is {:.0}% full", disk.mount_point, usage_pct),
                        destination: crate::state::Page::Storage,
                    });
                } else if usage_pct >= 80.0 {
                    items.push(AttentionItem {
                        severity: AttentionSeverity::Warning,
                        evidence: format!("{} is {:.0}% full", disk.mount_point, usage_pct),
                        destination: crate::state::Page::Storage,
                    });
                }
            }
        }

        if let Capability::Available(temps) = &snapshot.temperatures {
            for reading in temps {
                if let Capability::Available(celsius) = &reading.celsius {
                    if *celsius >= 90.0 {
                        items.push(AttentionItem {
                            severity: AttentionSeverity::Critical,
                            evidence: format!(
                                "{} at {:.0}°C",
                                reading.label.as_deref().unwrap_or(&reading.hwmon_name),
                                celsius
                            ),
                            destination: crate::state::Page::System,
                        });
                    }
                }
            }
        }

        items.sort_by(|a, b| b.severity.cmp(&a.severity));
        self.overview.attention_items = items;
    }
}

/// Format bytes per second as a human-readable string.
fn format_bytes_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_073_741_824.0 {
        format!("{:.1} GiB", bytes_per_sec / 1_073_741_824.0)
    } else if bytes_per_sec >= 1_048_576.0 {
        format!("{:.1} MiB", bytes_per_sec / 1_048_576.0)
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KiB", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B", bytes_per_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use maintainer_core::capability::UnavailableReason;
    use maintainer_core::metrics::processes::ProcessIdentity;

    fn make_cpu_snapshot(total: f64, cores: &[f64]) -> CpuSnapshot {
        CpuSnapshot {
            total_percent: total,
            per_core_percent: cores.to_vec(),
        }
    }

    fn make_memory_snapshot(used: u64, total: u64) -> MemorySnapshot {
        MemorySnapshot {
            total_bytes: total,
            available_bytes: total - used,
            used_bytes: used,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }
    }

    fn make_minimal_snapshot() -> SystemSnapshot {
        SystemSnapshot {
            cpu: Capability::Available(make_cpu_snapshot(25.0, &[20.0, 30.0])),
            memory: Capability::Available(make_memory_snapshot(
                4 * 1024 * 1024 * 1024,
                16 * 1024 * 1024 * 1024,
            )),
            disks: Capability::Available(vec![]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(vec![]),
        }
    }

    #[test]
    fn cpu_update_changes_only_cpu_state() {
        let mut state = MonitoringState::default();
        let snapshot = make_minimal_snapshot();

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

        assert_eq!(state.overview.cpu_text, "25%");
        assert!(state.overview.cpu_available);
        assert_eq!(state.overview.cpu_ring.len(), 1);
        assert_eq!(state.system.cpu_ring.len(), 1);
    }

    #[test]
    fn memory_update_produces_gib_text() {
        let mut state = MonitoringState::default();
        let snapshot = make_minimal_snapshot();

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

        assert!(state.overview.memory_text.contains("GiB"));
        assert!(state.overview.memory_available);
        assert_eq!(state.overview.memory_ring.len(), 1);
    }

    #[test]
    fn hidden_window_changes_interval_to_10s() {
        let mut state = MonitoringState::default();
        state.reduce(MonitoringMessage::VisibilityChanged(false));

        assert_eq!(state.interval, HIDDEN_INTERVAL);
        assert!(!state.window_visible);
        assert!(!state.immediate_sample_pending);
    }

    #[test]
    fn visible_window_triggers_immediate_sample() {
        let mut state = MonitoringState::default();
        state.reduce(MonitoringMessage::VisibilityChanged(false));
        state.reduce(MonitoringMessage::VisibilityChanged(true));

        assert_eq!(state.interval, VISIBLE_INTERVAL);
        assert!(state.window_visible);
        assert!(state.immediate_sample_pending);
    }

    #[test]
    fn process_sort_by_memory() {
        let mut state = MonitoringState::default();

        let processes = vec![
            ProcessSnapshot {
                identity: ProcessIdentity {
                    pid: 1,
                    start_time_ticks: 1,
                },
                name: "small".into(),
                state: 'S',
                ppid: 0,
                uid: 1000,
                rss_bytes: 1000,
                utime_ticks: 100,
                stime_ticks: 50,
                num_threads: 1,
            },
            ProcessSnapshot {
                identity: ProcessIdentity {
                    pid: 2,
                    start_time_ticks: 2,
                },
                name: "big".into(),
                state: 'S',
                ppid: 0,
                uid: 1000,
                rss_bytes: 999_999,
                utime_ticks: 10,
                stime_ticks: 5,
                num_threads: 1,
            },
        ];

        let snapshot = SystemSnapshot {
            cpu: Capability::Available(make_cpu_snapshot(10.0, &[10.0])),
            memory: Capability::Available(make_memory_snapshot(1024, 4096)),
            disks: Capability::Available(vec![]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(processes),
        };

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));
        state.reduce(MonitoringMessage::SetProcessSort(ProcessSort::Memory));

        assert_eq!(state.system.processes[0].name, "big");
        assert_eq!(state.system.processes[1].name, "small");
    }

    #[test]
    fn process_sort_by_name() {
        let mut state = MonitoringState::default();

        let processes = vec![
            ProcessSnapshot {
                identity: ProcessIdentity {
                    pid: 1,
                    start_time_ticks: 1,
                },
                name: "zzz".into(),
                state: 'S',
                ppid: 0,
                uid: 1000,
                rss_bytes: 1000,
                utime_ticks: 100,
                stime_ticks: 50,
                num_threads: 1,
            },
            ProcessSnapshot {
                identity: ProcessIdentity {
                    pid: 2,
                    start_time_ticks: 2,
                },
                name: "aaa".into(),
                state: 'S',
                ppid: 0,
                uid: 1000,
                rss_bytes: 500,
                utime_ticks: 10,
                stime_ticks: 5,
                num_threads: 1,
            },
        ];

        let snapshot = SystemSnapshot {
            cpu: Capability::Available(make_cpu_snapshot(10.0, &[10.0])),
            memory: Capability::Available(make_memory_snapshot(1024, 4096)),
            disks: Capability::Available(vec![]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(processes),
        };

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));
        state.reduce(MonitoringMessage::SetProcessSort(ProcessSort::Name));

        assert_eq!(state.system.processes[0].name, "aaa");
        assert_eq!(state.system.processes[1].name, "zzz");
    }

    #[test]
    fn ring_buffer_caps_at_60_samples() {
        let mut ring = SampleRing::new();
        for i in 0..100 {
            ring.push(i as f64);
        }
        assert_eq!(ring.len(), MAX_SAMPLES);
        assert_eq!(*ring.samples().front().unwrap(), 40.0);
        assert_eq!(ring.latest(), Some(99.0));
    }

    #[test]
    fn disk_over_90_percent_triggers_critical_attention() {
        let mut state = MonitoringState::default();

        let disk = DiskSnapshot {
            device: "sda1".into(),
            mount_point: "/".into(),
            fs_type: "ext4".into(),
            total_bytes: 100_000_000_000,
            used_bytes: 95_000_000_000,
            available_bytes: 5_000_000_000,
            read_bytes_per_sec: None,
            write_bytes_per_sec: None,
        };

        let snapshot = SystemSnapshot {
            cpu: Capability::Available(make_cpu_snapshot(10.0, &[10.0])),
            memory: Capability::Available(make_memory_snapshot(1024, 4096)),
            disks: Capability::Available(vec![disk]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(vec![]),
        };

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

        assert_eq!(state.overview.attention_items.len(), 1);
        assert_eq!(
            state.overview.attention_items[0].severity,
            AttentionSeverity::Critical
        );
        assert!(state.overview.attention_items[0].evidence.contains("/"));
    }

    #[test]
    fn disk_between_80_90_triggers_warning() {
        let mut state = MonitoringState::default();

        let disk = DiskSnapshot {
            device: "sda1".into(),
            mount_point: "/home".into(),
            fs_type: "ext4".into(),
            total_bytes: 100_000_000_000,
            used_bytes: 85_000_000_000,
            available_bytes: 15_000_000_000,
            read_bytes_per_sec: None,
            write_bytes_per_sec: None,
        };

        let snapshot = SystemSnapshot {
            cpu: Capability::Available(make_cpu_snapshot(10.0, &[10.0])),
            memory: Capability::Available(make_memory_snapshot(1024, 4096)),
            disks: Capability::Available(vec![disk]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(vec![]),
        };

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

        assert_eq!(state.overview.attention_items.len(), 1);
        assert_eq!(
            state.overview.attention_items[0].severity,
            AttentionSeverity::Warning
        );
    }

    #[test]
    fn cpu_load_does_not_trigger_attention() {
        let mut state = MonitoringState::default();

        let snapshot = SystemSnapshot {
            cpu: Capability::Available(make_cpu_snapshot(99.0, &[99.0, 99.0])),
            memory: Capability::Available(make_memory_snapshot(1024, 4096)),
            disks: Capability::Available(vec![]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(vec![]),
        };

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

        assert!(
            state.overview.attention_items.is_empty(),
            "CPU load should NOT trigger attention"
        );
    }

    #[test]
    fn unavailable_cpu_renders_reason_not_zero() {
        let mut state = MonitoringState::default();

        let snapshot = SystemSnapshot {
            cpu: Capability::Unavailable(UnavailableReason {
                code: "no-proc",
                message: "Cannot read /proc/stat".into(),
                remediation: None,
            }),
            memory: Capability::Available(make_memory_snapshot(1024, 4096)),
            disks: Capability::Available(vec![]),
            network: Capability::Available(vec![]),
            temperatures: Capability::Available(vec![]),
            processes: Capability::Available(vec![]),
        };

        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

        assert!(!state.overview.cpu_available);
        assert_eq!(state.overview.cpu_text, "Unavailable");
        assert!(state.overview.cpu_ring.is_empty());
    }

    #[test]
    fn format_bytes_rate_scales() {
        assert_eq!(format_bytes_rate(500.0), "500 B");
        assert_eq!(format_bytes_rate(1500.0), "1.5 KiB");
        assert_eq!(format_bytes_rate(1_500_000.0), "1.4 MiB");
        assert_eq!(format_bytes_rate(1_500_000_000.0), "1.4 GiB");
    }
}
