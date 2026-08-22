//! Host smoke test that reads real /proc and /sys and asserts plausible ranges.
//!
//! Ignored by default; run with: `cargo test -p maintainer-core --test host_metrics -- --ignored`

use std::path::Path;
use std::time::Instant;

use maintainer_core::metrics::cpu::{compute_cpu_snapshot, read_cpu_sample};
use maintainer_core::metrics::disk::{read_diskstats, read_mounts, read_mounts_with_capacity};
use maintainer_core::metrics::memory::read_memory;
use maintainer_core::metrics::network::read_net_dev;
use maintainer_core::metrics::processes::read_processes;
use maintainer_core::metrics::temperature::read_temperatures;

#[test]
#[ignore]
fn host_cpu_produces_plausible_values() {
    let proc_path = Path::new("/proc");
    let now = Instant::now();
    let sample1 = read_cpu_sample(proc_path, now).unwrap();

    std::thread::sleep(std::time::Duration::from_millis(100));

    let sample2 = read_cpu_sample(proc_path, Instant::now()).unwrap();
    let snapshot = compute_cpu_snapshot(&sample1, &sample2);

    assert!(
        snapshot.total_percent >= 0.0 && snapshot.total_percent <= 100.0,
        "total CPU% out of range: {}",
        snapshot.total_percent
    );
    assert!(!snapshot.per_core_percent.is_empty());
    for pct in &snapshot.per_core_percent {
        assert!(
            *pct >= 0.0 && *pct <= 100.0,
            "core CPU% out of range: {pct}"
        );
    }
}

#[test]
#[ignore]
fn host_memory_produces_plausible_values() {
    let snapshot = read_memory(Path::new("/proc")).unwrap();

    assert!(snapshot.total_bytes > 0, "total_bytes should be > 0");
    assert!(
        snapshot.available_bytes <= snapshot.total_bytes,
        "available > total"
    );
    assert_eq!(
        snapshot.used_bytes,
        snapshot.total_bytes - snapshot.available_bytes
    );
    assert!(snapshot.swap_used_bytes <= snapshot.swap_total_bytes);
}

#[test]
#[ignore]
fn host_mounts_finds_root() {
    let mounts = read_mounts(Path::new("/proc")).unwrap();
    assert!(!mounts.is_empty());
    let root = mounts.iter().find(|m| m.mount_point == "/");
    assert!(root.is_some(), "should find root mount");
}

#[test]
#[ignore]
fn host_diskstats_has_entries() {
    let stats = read_diskstats(Path::new("/proc")).unwrap();
    assert!(!stats.is_empty(), "should find at least one disk device");
}

#[test]
#[ignore]
fn host_network_has_loopback() {
    let counters = read_net_dev(Path::new("/proc")).unwrap();
    assert!(
        counters.contains_key("lo"),
        "should find loopback interface"
    );
}

#[test]
#[ignore]
fn host_processes_finds_self() {
    let processes = read_processes(Path::new("/proc")).unwrap();
    assert!(!processes.is_empty(), "should find at least one process");

    let our_pid = std::process::id();
    let us = processes.iter().find(|p| p.identity.pid == our_pid);
    assert!(us.is_some(), "should find our own process (pid {our_pid})");
}

#[test]
#[ignore]
fn host_temperatures_no_panic() {
    let readings = read_temperatures(Path::new("/sys")).unwrap();
    for reading in &readings {
        if let maintainer_core::capability::Capability::Available(c) = &reading.celsius {
            assert!(
                *c >= -40.0 && *c <= 150.0,
                "temperature out of plausible range: {c}°C for {}",
                reading.hwmon_name
            );
        }
    }
}

#[test]
#[ignore]
fn host_disk_capacity_reports_nonzero_for_root() {
    let snapshots = read_mounts_with_capacity(Path::new("/proc")).unwrap();
    assert!(!snapshots.is_empty(), "should find at least one disk");

    let root = snapshots.iter().find(|d| d.mount_point == "/");
    assert!(root.is_some(), "should find root mount");

    let root = root.unwrap();
    assert!(
        root.total_bytes > 0,
        "root total_bytes should be > 0, got {}",
        root.total_bytes
    );
    assert!(
        root.available_bytes > 0,
        "root available_bytes should be > 0, got {}",
        root.available_bytes
    );
    assert!(
        root.used_bytes > 0,
        "root used_bytes should be > 0, got {}",
        root.used_bytes
    );
    assert!(
        root.total_bytes >= root.used_bytes,
        "total >= used: {} >= {}",
        root.total_bytes,
        root.used_bytes
    );

    eprintln!(
        "Root filesystem: total={} used={} available={}",
        root.total_bytes, root.used_bytes, root.available_bytes
    );
}
