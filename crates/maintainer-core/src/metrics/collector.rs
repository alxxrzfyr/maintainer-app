//! Stateful metric sampling, owned by a bounded application worker.

use std::collections::HashMap;
use std::path::Path;
use std::time::Instant;

use crate::capability::{Capability, UnavailableReason};
use crate::metrics::{
    CpuSample, NetCounters, NetworkSnapshot, ProcessSnapshot, SystemSnapshot, TemperatureCollector,
    compute_cpu_snapshot, read_cpu_sample, read_memory, read_mounts_with_capacity, read_net_dev,
    read_processes,
};

const MAX_PROCESS_CACHE_BYTES: usize = 4 * 1024 * 1024;

/// Keeps metric delta baselines between samples.
#[derive(Default)]
pub struct SystemCollector {
    prev_cpu: Option<CpuSample>,
    prev_net: Option<(HashMap<String, NetCounters>, Instant)>,
    temperatures: TemperatureCollector,
}

impl SystemCollector {
    pub fn new() -> Self {
        Self::default()
    }

    /// Blocking Linux reads: call only from a background worker.
    pub fn collect(&mut self, needs_processes: bool) -> SystemSnapshot {
        self.collect_at(
            Path::new("/proc"),
            Path::new("/sys"),
            Instant::now(),
            needs_processes,
        )
    }

    fn collect_at(
        &mut self,
        proc_path: &Path,
        sys_path: &Path,
        now: Instant,
        needs_processes: bool,
    ) -> SystemSnapshot {
        let cpu = match read_cpu_sample(proc_path, now) {
            Ok(sample) => {
                let snapshot = match &self.prev_cpu {
                    Some(prev) => compute_cpu_snapshot(prev, &sample),
                    None => crate::metrics::CpuSnapshot {
                        total_percent: 0.0,
                        per_core_percent: vec![0.0; sample.cores.len()],
                    },
                };
                self.prev_cpu = Some(sample);
                Capability::Available(snapshot)
            }
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "proc-stat-read",
                message: e.to_string(),
                remediation: None,
            }),
        };

        let memory = match read_memory(proc_path) {
            Ok(m) => Capability::Available(m),
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "meminfo-read",
                message: e.to_string(),
                remediation: None,
            }),
        };

        let disks = match read_mounts_with_capacity(proc_path) {
            Ok(d) => Capability::Available(d),
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "mounts-read",
                message: e.to_string(),
                remediation: None,
            }),
        };

        let network = match read_net_dev(proc_path) {
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
        };

        let temperatures = match self.temperatures.read(sys_path) {
            Ok(t) => Capability::Available(t),
            Err(e) => Capability::Unavailable(UnavailableReason {
                code: "hwmon-read",
                message: e.to_string(),
                remediation: None,
            }),
        };

        let processes = if needs_processes {
            let result = match read_processes(proc_path) {
                Ok(p) => {
                    let bytes = p
                        .capacity()
                        .saturating_mul(std::mem::size_of::<ProcessSnapshot>())
                        .saturating_add(
                            p.iter()
                                .map(|process| process.name.capacity())
                                .fold(0usize, usize::saturating_add),
                        );
                    if bytes <= MAX_PROCESS_CACHE_BYTES {
                        Capability::Available(p)
                    } else {
                        Capability::Unavailable(UnavailableReason {
                            code: "proc-cache-limit",
                            message: "Process list exceeds the 4 MiB monitoring cache budget"
                                .into(),
                            remediation: None,
                        })
                    }
                }
                Err(e) => Capability::Unavailable(UnavailableReason {
                    code: "proc-read",
                    message: e.to_string(),
                    remediation: None,
                }),
            };
            Some(result)
        } else {
            None
        };

        SystemSnapshot {
            cpu,
            memory,
            disks,
            network,
            temperatures,
            processes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::time::Duration;

    fn fixtures() -> std::path::PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../fixtures")
    }

    fn net_sample(rx: u64, tx: u64, added: bool) -> String {
        format!(
            "header\nheader\neth0: {rx} 1 0 0 0 0 0 0 {tx} 1 0 0 0 0 0 0\n{}",
            if added {
                "wlan0: 9000 1 0 0 0 0 0 0 8000 1 0 0 0 0 0 0\n"
            } else {
                ""
            }
        )
    }

    #[test]
    fn consecutive_samples_keep_cpu_deltas_and_interface_rate_baselines() {
        let temp = tempfile::tempdir().unwrap();
        fs::create_dir(temp.path().join("net")).unwrap();
        fs::copy(fixtures().join("proc/stat"), temp.path().join("stat")).unwrap();
        fs::write(temp.path().join("net/dev"), net_sample(100, 200, false)).unwrap();
        let mut collector = SystemCollector::new();
        let start = Instant::now();
        let first = collector.collect_at(temp.path(), &fixtures().join("sys"), start, false);
        assert_eq!(first.cpu.as_value().unwrap().total_percent, 0.0);
        assert!(
            first.network.as_value().unwrap()[0]
                .rx_bytes_per_sec
                .is_none()
        );
        fs::copy(
            fixtures().join("proc/stat_sample2"),
            temp.path().join("stat"),
        )
        .unwrap();
        fs::write(temp.path().join("net/dev"), net_sample(300, 300, true)).unwrap();
        let second = collector.collect_at(
            temp.path(),
            &fixtures().join("sys"),
            start + Duration::from_secs(2),
            false,
        );
        assert!(second.cpu.as_value().unwrap().total_percent > 0.0);
        let network = second.network.as_value().unwrap();
        let existing = network
            .iter()
            .find(|sample| sample.interface == "eth0")
            .unwrap();
        assert_eq!(existing.rx_bytes_per_sec, Some(100.0));
        assert_eq!(existing.tx_bytes_per_sec, Some(50.0));
        assert!(
            network
                .iter()
                .find(|sample| sample.interface == "wlan0")
                .unwrap()
                .rx_bytes_per_sec
                .is_none()
        );
    }

    #[test]
    fn unrequested_process_samples_omit_data_instead_of_cloning_cached_results() {
        let mut collector = SystemCollector::new();
        let start = Instant::now();
        let absent = Path::new("/nonexistent-maintainer-collector-fixture");
        assert!(
            collector
                .collect_at(absent, absent, start, false)
                .processes
                .is_none()
        );
        let requested = collector.collect_at(
            &fixtures().join("proc"),
            &fixtures().join("sys"),
            start,
            true,
        );
        assert!(
            !requested
                .processes
                .as_ref()
                .unwrap()
                .as_value()
                .unwrap()
                .is_empty()
        );
        assert!(
            collector
                .collect_at(absent, absent, start + Duration::from_secs(2), false)
                .processes
                .is_none()
        );
        let failed = collector.collect_at(absent, absent, start + Duration::from_secs(4), true);
        assert!(
            matches!(failed.processes, Some(Capability::Unavailable(ref reason)) if reason.code == "proc-read")
        );
        assert!(
            collector
                .collect_at(absent, absent, start + Duration::from_secs(6), false)
                .processes
                .is_none()
        );
    }

    #[test]
    fn oversized_process_names_are_not_retained_in_the_monitoring_cache() {
        let temp = tempfile::tempdir().unwrap();
        let process = temp.path().join("1234");
        fs::create_dir(&process).unwrap();
        let stat = fs::read_to_string(fixtures().join("proc/1234/stat")).unwrap();
        fs::write(
            process.join("stat"),
            stat.replace("firefox", &"x".repeat(MAX_PROCESS_CACHE_BYTES)),
        )
        .unwrap();
        fs::copy(fixtures().join("proc/1234/status"), process.join("status")).unwrap();
        let mut collector = SystemCollector::new();
        let sample = collector.collect_at(temp.path(), temp.path(), Instant::now(), true);
        assert!(
            matches!(sample.processes, Some(Capability::Unavailable(ref reason)) if reason.code == "proc-cache-limit")
        );
        assert!(
            collector
                .collect_at(temp.path(), temp.path(), Instant::now(), false)
                .processes
                .is_none()
        );
    }
}
