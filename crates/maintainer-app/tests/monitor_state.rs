//! Integration tests for monitoring state, process sorting, visibility, and attention.
//!
//! These tests exercise the pure-data monitoring state machine without requiring
//! a display server. They verify incremental updates, sort stability, and
//! interval changes based on window visibility.

use maintainer_app::monitoring::{
    AttentionSeverity, HIDDEN_INTERVAL, MonitoringMessage, MonitoringState, ProcessSort,
    SampleRing, VISIBLE_INTERVAL,
};
use maintainer_core::capability::{Capability, UnavailableReason};
use maintainer_core::metrics::disk::DiskSnapshot;
use maintainer_core::metrics::processes::{ProcessIdentity, ProcessSnapshot};
use maintainer_core::metrics::{CpuSnapshot, MemorySnapshot, SystemSnapshot};

fn make_snapshot_with_cpu(pct: f64) -> SystemSnapshot {
    SystemSnapshot {
        cpu: Capability::Available(CpuSnapshot {
            total_percent: pct,
            per_core_percent: vec![pct],
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 8 * 1024 * 1024 * 1024,
            used_bytes: 8 * 1024 * 1024 * 1024,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Capability::Available(vec![]),
        network: Capability::Available(vec![]),
        temperatures: Capability::Available(vec![]),
        processes: Capability::Available(vec![]),
    }
}

#[test]
fn cpu_update_only_affects_cpu_fields() {
    let mut state = MonitoringState::default();
    let snapshot = make_snapshot_with_cpu(42.0);

    state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

    assert_eq!(state.overview.cpu_text, "42%");
    assert_eq!(state.overview.cpu_ring.len(), 1);
    assert!(state.overview.cpu_available);
}

#[test]
fn multiple_snapshots_accumulate_in_ring() {
    let mut state = MonitoringState::default();

    for i in 0..5 {
        let snapshot = make_snapshot_with_cpu(20.0 + i as f64);
        state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));
    }

    assert_eq!(state.overview.cpu_ring.len(), 5);
    assert_eq!(state.system.cpu_ring.len(), 5);
}

#[test]
fn hidden_window_sets_10s_interval() {
    let mut state = MonitoringState::default();
    assert_eq!(state.interval, VISIBLE_INTERVAL);

    state.reduce(MonitoringMessage::VisibilityChanged(false));

    assert_eq!(state.interval, HIDDEN_INTERVAL);
    assert!(!state.window_visible);
}

#[test]
fn returning_visible_triggers_immediate_sample() {
    let mut state = MonitoringState::default();
    state.reduce(MonitoringMessage::VisibilityChanged(false));
    assert!(!state.immediate_sample_pending);

    state.reduce(MonitoringMessage::VisibilityChanged(true));

    assert!(state.immediate_sample_pending);
    assert_eq!(state.interval, VISIBLE_INTERVAL);
}

#[test]
fn process_sorting_stable_by_cpu() {
    let mut state = MonitoringState::default();

    let procs = vec![
        ProcessSnapshot {
            identity: ProcessIdentity {
                pid: 1,
                start_time_ticks: 1,
            },
            name: "low-cpu".into(),
            state: 'S',
            ppid: 0,
            uid: 1000,
            rss_bytes: 1_000_000,
            utime_ticks: 10,
            stime_ticks: 5,
            num_threads: 1,
        },
        ProcessSnapshot {
            identity: ProcessIdentity {
                pid: 2,
                start_time_ticks: 2,
            },
            name: "high-cpu".into(),
            state: 'R',
            ppid: 0,
            uid: 1000,
            rss_bytes: 500_000,
            utime_ticks: 1000,
            stime_ticks: 500,
            num_threads: 4,
        },
    ];

    let snapshot = SystemSnapshot {
        cpu: Capability::Available(CpuSnapshot {
            total_percent: 50.0,
            per_core_percent: vec![50.0],
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 8 * 1024 * 1024 * 1024,
            used_bytes: 8 * 1024 * 1024 * 1024,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Capability::Available(vec![]),
        network: Capability::Available(vec![]),
        temperatures: Capability::Available(vec![]),
        processes: Capability::Available(procs),
    };

    state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

    assert_eq!(state.system.processes[0].name, "high-cpu");
    assert_eq!(state.system.processes[1].name, "low-cpu");

    state.reduce(MonitoringMessage::SetProcessSort(ProcessSort::Memory));
    assert_eq!(state.system.processes[0].name, "low-cpu"); // higher RSS
}

#[test]
fn disk_threshold_attention_not_cpu() {
    let mut state = MonitoringState::default();

    let snapshot = SystemSnapshot {
        cpu: Capability::Available(CpuSnapshot {
            total_percent: 99.0, // High CPU should NOT trigger attention
            per_core_percent: vec![99.0, 99.0],
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 8 * 1024 * 1024 * 1024,
            used_bytes: 8 * 1024 * 1024 * 1024,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Capability::Available(vec![DiskSnapshot {
            device: "nvme0n1p1".into(),
            mount_point: "/".into(),
            fs_type: "ext4".into(),
            total_bytes: 500_000_000_000,
            used_bytes: 460_000_000_000, // 92% full
            available_bytes: 40_000_000_000,
            read_bytes_per_sec: None,
            write_bytes_per_sec: None,
        }]),
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
fn unavailable_metric_shows_reason_text() {
    let mut state = MonitoringState::default();

    let snapshot = SystemSnapshot {
        cpu: Capability::Unavailable(UnavailableReason {
            code: "no-proc-stat",
            message: "Cannot read /proc/stat".into(),
            remediation: None,
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 8 * 1024 * 1024 * 1024,
            used_bytes: 8 * 1024 * 1024 * 1024,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
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
fn ring_buffer_evicts_oldest_at_capacity() {
    let mut ring = SampleRing::new();

    for i in 0..70 {
        ring.push(i as f64);
    }

    assert_eq!(ring.len(), 60);
    assert_eq!(ring.latest(), Some(69.0));
    assert_eq!(*ring.samples().front().unwrap(), 10.0);
}

#[test]
fn network_vm_populated_from_snapshot() {
    use maintainer_core::metrics::NetworkSnapshot;

    let mut state = MonitoringState::default();

    let snapshot = SystemSnapshot {
        cpu: Capability::Available(CpuSnapshot {
            total_percent: 10.0,
            per_core_percent: vec![10.0],
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 16 * 1024 * 1024 * 1024,
            available_bytes: 8 * 1024 * 1024 * 1024,
            used_bytes: 8 * 1024 * 1024 * 1024,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Capability::Available(vec![]),
        network: Capability::Available(vec![
            NetworkSnapshot {
                interface: "eth0".into(),
                is_loopback: false,
                rx_bytes: 1000,
                tx_bytes: 500,
                rx_packets: 10,
                tx_packets: 5,
                rx_errors: 0,
                tx_errors: 0,
                rx_bytes_per_sec: Some(1024.0),
                tx_bytes_per_sec: Some(512.0),
            },
            NetworkSnapshot {
                interface: "lo".into(),
                is_loopback: true,
                rx_bytes: 100,
                tx_bytes: 100,
                rx_packets: 1,
                tx_packets: 1,
                rx_errors: 0,
                tx_errors: 0,
                rx_bytes_per_sec: Some(0.0),
                tx_bytes_per_sec: Some(0.0),
            },
        ]),
        temperatures: Capability::Available(vec![]),
        processes: Capability::Available(vec![]),
    };

    state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

    assert!(state.network_vm.available);
    assert_eq!(state.network_vm.interfaces.len(), 2);
    assert_eq!(state.network_vm.total_rx_rate, 1024.0);
    assert_eq!(state.network_vm.total_tx_rate, 512.0);
    assert_eq!(state.network_vm.rx_rings.len(), 2);
    assert_eq!(state.network_vm.tx_rings.len(), 2);
}

#[test]
fn processes_vm_sorted_by_active_sort() {
    let mut state = MonitoringState::default();

    let procs = vec![
        ProcessSnapshot {
            identity: ProcessIdentity {
                pid: 1,
                start_time_ticks: 1,
            },
            name: "alpha".into(),
            state: 'S',
            ppid: 0,
            uid: 1000,
            rss_bytes: 500_000,
            utime_ticks: 100,
            stime_ticks: 50,
            num_threads: 1,
        },
        ProcessSnapshot {
            identity: ProcessIdentity {
                pid: 2,
                start_time_ticks: 2,
            },
            name: "beta".into(),
            state: 'R',
            ppid: 0,
            uid: 1000,
            rss_bytes: 2_000_000,
            utime_ticks: 10,
            stime_ticks: 5,
            num_threads: 2,
        },
    ];

    let snapshot = SystemSnapshot {
        cpu: Capability::Available(CpuSnapshot {
            total_percent: 10.0,
            per_core_percent: vec![10.0],
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 4096,
            available_bytes: 2048,
            used_bytes: 2048,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Capability::Available(vec![]),
        network: Capability::Available(vec![]),
        temperatures: Capability::Available(vec![]),
        processes: Capability::Available(procs),
    };

    state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

    assert!(state.processes_vm.available);
    assert_eq!(state.processes_vm.total_count, 2);
    // Default sort is CPU (highest ticks first)
    assert_eq!(state.processes_vm.processes[0].name, "alpha");

    state.reduce(MonitoringMessage::SetProcessSort(ProcessSort::Memory));
    // After re-sort, processes_vm should reflect new order on next snapshot
    // (SetProcessSort only re-sorts system.processes, not processes_vm directly,
    // but processes_vm is populated from the sorted system.processes during snapshot)
    assert_eq!(state.system.processes[0].name, "beta");
}

#[test]
fn temperature_groups_populated() {
    use maintainer_core::metrics::TemperatureReading;

    let mut state = MonitoringState::default();

    let temps = vec![
        TemperatureReading {
            hwmon_name: "coretemp".into(),
            label: Some("Package id 0".into()),
            celsius: Capability::Available(50.0),
        },
        TemperatureReading {
            hwmon_name: "coretemp".into(),
            label: Some("Core 0".into()),
            celsius: Capability::Available(47.0),
        },
        TemperatureReading {
            hwmon_name: "acpitz".into(),
            label: None,
            celsius: Capability::Available(38.0),
        },
    ];

    let snapshot = SystemSnapshot {
        cpu: Capability::Available(CpuSnapshot {
            total_percent: 10.0,
            per_core_percent: vec![10.0],
        }),
        memory: Capability::Available(MemorySnapshot {
            total_bytes: 4096,
            available_bytes: 2048,
            used_bytes: 2048,
            buffers_bytes: 0,
            cached_bytes: 0,
            swap_total_bytes: 0,
            swap_free_bytes: 0,
            swap_used_bytes: 0,
        }),
        disks: Capability::Available(vec![]),
        network: Capability::Available(vec![]),
        temperatures: Capability::Available(temps),
        processes: Capability::Available(vec![]),
    };

    state.reduce(MonitoringMessage::SnapshotReady(Box::new(snapshot)));

    assert_eq!(state.system.temperature_groups.len(), 2);
    assert_eq!(state.system.temperature_groups[0].chip_name, "acpitz");
    assert_eq!(state.system.temperature_groups[1].chip_name, "coretemp");
    assert_eq!(
        state.system.temperature_groups[1]
            .representative
            .label
            .as_deref(),
        Some("Package id 0")
    );
}
