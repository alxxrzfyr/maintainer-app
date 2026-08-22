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

/// Detailed hardware and system specifications.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeviceInfo {
    /// Laptop / Hardware model (e.g. "LENOVO ThinkPad T460p (20FXS06A2K)").
    pub model: String,
    /// Operating system distribution (e.g. "Linux Mint 22.3 (64-bit)").
    pub os: String,
    /// Linux kernel version (e.g. "Linux 6.8.0-52-generic").
    pub kernel: String,
    /// Processor model and core configuration (e.g. "Intel Core i5-6440HQ @ 2.60GHz x 4").
    pub cpu: String,
    /// Graphics card / GPU (e.g. "Intel Corporation HD Graphics 530").
    pub gpu: String,
    /// Total RAM installed (e.g. "7.9 GB").
    pub memory: String,
    /// Storage / Hard Drives (e.g. "120.3 GB (ext4)").
    pub storage: String,
    /// Desktop environment (e.g. "XFCE", "GNOME", "Cinnamon").
    pub desktop_env: String,
    /// Display server (e.g. "X11", "Wayland").
    pub display_server: String,
}

impl Default for DeviceInfo {
    fn default() -> Self {
        collect_device_info()
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
    /// Hardware and system specifications.
    pub device_info: DeviceInfo,
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
            device_info: DeviceInfo::default(),
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

/// Collect hardware and system specifications.
pub fn collect_device_info() -> DeviceInfo {
    use std::fs;
    use std::path::Path;

    let vendor = fs::read_to_string("/sys/devices/virtual/dmi/id/sys_vendor")
        .or_else(|_| fs::read_to_string("/sys/class/dmi/id/sys_vendor"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let product_name = fs::read_to_string("/sys/devices/virtual/dmi/id/product_name")
        .or_else(|_| fs::read_to_string("/sys/class/dmi/id/product_name"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let product_version = fs::read_to_string("/sys/devices/virtual/dmi/id/product_version")
        .or_else(|_| fs::read_to_string("/sys/class/dmi/id/product_version"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let arm_model = fs::read_to_string("/proc/device-tree/model")
        .unwrap_or_default()
        .trim_matches('\0')
        .trim()
        .to_string();

    let model = if !product_version.is_empty()
        && product_version != "None"
        && product_version != product_name
    {
        if !vendor.is_empty() && !product_name.is_empty() {
            format!("{vendor} {product_version} ({product_name})")
        } else if !vendor.is_empty() {
            format!("{vendor} {product_version}")
        } else {
            product_version
        }
    } else if !product_name.is_empty()
        && product_name != "System Product Name"
        && product_name != "To be filled by O.E.M."
    {
        if !vendor.is_empty()
            && !product_name
                .to_lowercase()
                .starts_with(&vendor.to_lowercase())
        {
            format!("{vendor} {product_name}")
        } else {
            product_name
        }
    } else if !arm_model.is_empty() {
        arm_model
    } else if !vendor.is_empty() && vendor != "System manufacturer" {
        format!("{vendor} PC")
    } else {
        "Linux Device".to_string()
    };

    let os_release = fs::read_to_string("/etc/os-release").unwrap_or_default();
    let pretty_name = os_release
        .lines()
        .find(|l| l.starts_with("PRETTY_NAME="))
        .map(|l| {
            l.trim_start_matches("PRETTY_NAME=")
                .trim_matches('"')
                .to_string()
        })
        .unwrap_or_else(|| "Linux".to_string());
    let arch = std::env::consts::ARCH;
    let arch_label = match arch {
        "x86_64" => "64-bit",
        "aarch64" => "ARM64",
        "x86" => "32-bit",
        other => other,
    };
    let os = format!("{pretty_name} ({arch_label})");

    let kernel_ver = fs::read_to_string("/proc/sys/kernel/osrelease")
        .unwrap_or_else(|_| {
            fs::read_to_string("/proc/version")
                .unwrap_or_default()
                .split_whitespace()
                .nth(2)
                .unwrap_or("unknown")
                .to_string()
        })
        .trim()
        .to_string();
    let kernel = format!("Linux {kernel_ver}");

    let cpuinfo = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let cpu_name = cpuinfo
        .lines()
        .find(|l| {
            l.starts_with("model name") || l.starts_with("Processor") || l.starts_with("Hardware")
        })
        .and_then(|l| l.split(':').nth(1))
        .map(|name| {
            name.replace("(R)", "")
                .replace("(TM)", "")
                .split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
        })
        .unwrap_or_else(|| "CPU".to_string());
    let thread_count = cpuinfo
        .lines()
        .filter(|l| l.starts_with("processor\t") || l.starts_with("processor :"))
        .count();
    let cpu = if thread_count > 0 {
        format!("{cpu_name} x {thread_count}")
    } else {
        cpu_name
    };

    let gpu = detect_gpu();

    let meminfo = fs::read_to_string("/proc/meminfo").unwrap_or_default();
    let mem_total_kb = meminfo
        .lines()
        .find(|l| l.starts_with("MemTotal:"))
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0);
    let memory = if mem_total_kb > 0 {
        let gb = mem_total_kb as f64 / 1_000_000.0;
        format!("{gb:.1} GB")
    } else {
        "—".to_string()
    };

    let storage =
        match maintainer_core::metrics::disk::read_mounts_with_capacity(Path::new("/proc")) {
            Ok(mounts) => {
                if let Some(root) = mounts.iter().find(|m| m.mount_point == "/") {
                    let total_gb = root.total_bytes as f64 / 1_000_000_000.0;
                    format!("{total_gb:.1} GB (ext4)")
                } else {
                    let total: u64 = mounts.iter().map(|m| m.total_bytes).sum();
                    let total_gb = total as f64 / 1_000_000_000.0;
                    format!("{total_gb:.1} GB")
                }
            }
            Err(_) => "Storage available".to_string(),
        };

    let desktop_env = std::env::var("XDG_CURRENT_DESKTOP")
        .or_else(|_| std::env::var("DESKTOP_SESSION"))
        .unwrap_or_else(|_| "Desktop".to_string());

    let display_server = std::env::var("XDG_SESSION_TYPE").unwrap_or_else(|_| {
        if std::env::var("WAYLAND_DISPLAY").is_ok() {
            "Wayland".to_string()
        } else if std::env::var("DISPLAY").is_ok() {
            "X11".to_string()
        } else {
            "Unknown".to_string()
        }
    });

    DeviceInfo {
        model,
        os,
        kernel,
        cpu,
        gpu,
        memory,
        storage,
        desktop_env,
        display_server,
    }
}

/// Detect graphics card model name via lspci or sysfs.
fn detect_gpu() -> String {
    use std::fs;
    use std::process::Command;

    if let Ok(output) = Command::new("lspci").args(["-mm"]).output() {
        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let mut gpus = Vec::new();
            for line in stdout.lines() {
                if line.contains("VGA compatible controller")
                    || line.contains("3D controller")
                    || line.contains("Display controller")
                {
                    let parts: Vec<&str> = line
                        .split('"')
                        .enumerate()
                        .filter_map(|(idx, s)| if idx % 2 == 1 { Some(s.trim()) } else { None })
                        .collect();
                    if parts.len() >= 3 {
                        let vendor = parts[1];
                        let device = parts[2];
                        let name = if device.to_lowercase().starts_with(&vendor.to_lowercase()) {
                            device.to_string()
                        } else {
                            format!("{vendor} {device}")
                        };
                        if !gpus.contains(&name) {
                            gpus.push(name);
                        }
                    }
                }
            }
            if !gpus.is_empty() {
                return gpus.join(" / ");
            }
        }
    }

    if let Ok(entries) = fs::read_dir("/sys/bus/pci/devices") {
        let mut gpus = Vec::new();
        for entry in entries.flatten() {
            let path = entry.path();
            if let Ok(class) = fs::read_to_string(path.join("class")) {
                let class_trimmed = class.trim();
                if class_trimmed.starts_with("0x0300")
                    || class_trimmed.starts_with("0x0302")
                    || class_trimmed.starts_with("0x0380")
                {
                    let vendor_id = fs::read_to_string(path.join("vendor"))
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let driver = fs::read_link(path.join("driver"))
                        .ok()
                        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
                        .unwrap_or_default();

                    let vendor_name = match vendor_id.to_lowercase().as_str() {
                        "0x8086" => "Intel Corporation",
                        "0x10de" => "NVIDIA Corporation",
                        "0x1002" => "AMD",
                        _ => "Integrated",
                    };

                    let entry_desc = if !driver.is_empty() {
                        format!("{vendor_name} Graphics ({driver})")
                    } else {
                        format!("{vendor_name} Graphics")
                    };
                    if !gpus.contains(&entry_desc) {
                        gpus.push(entry_desc);
                    }
                }
            }
        }
        if !gpus.is_empty() {
            return gpus.join(" / ");
        }
    }

    "Integrated Graphics".to_string()
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
