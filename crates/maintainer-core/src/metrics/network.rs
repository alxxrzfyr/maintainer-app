//! Network metric collector that reads `/proc/net/dev`.
//!
//! Excludes loopback from default summary totals but retains it in the per-interface
//! details list.

use std::collections::HashMap;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::MaintainerError;

/// Snapshot of a single network interface.
#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
    /// Interface name (e.g., "eth0", "wlan0", "lo").
    pub interface: String,
    /// Whether this is a loopback interface.
    pub is_loopback: bool,
    /// Total bytes received since boot.
    pub rx_bytes: u64,
    /// Total bytes transmitted since boot.
    pub tx_bytes: u64,
    /// Total packets received since boot.
    pub rx_packets: u64,
    /// Total packets transmitted since boot.
    pub tx_packets: u64,
    /// Receive errors since boot.
    pub rx_errors: u64,
    /// Transmit errors since boot.
    pub tx_errors: u64,
    /// Receive rate in bytes per second (None on first sample).
    pub rx_bytes_per_sec: Option<f64>,
    /// Transmit rate in bytes per second (None on first sample).
    pub tx_bytes_per_sec: Option<f64>,
}

/// Raw network counters from a single `/proc/net/dev` read.
#[derive(Debug, Clone, Default)]
pub struct NetCounters {
    pub rx_bytes: u64,
    pub rx_packets: u64,
    pub rx_errors: u64,
    pub tx_bytes: u64,
    pub tx_packets: u64,
    pub tx_errors: u64,
}

/// Stored baseline for rate computation.
#[derive(Debug, Clone)]
pub struct NetBaseline {
    pub counters: HashMap<String, NetCounters>,
    pub timestamp: Instant,
}

/// Read and parse `/proc/net/dev`.
pub fn read_net_dev(proc_path: &Path) -> Result<HashMap<String, NetCounters>, MaintainerError> {
    let path = proc_path.join("net/dev");
    let content = crate::metrics::read_proc_text(&path)?;

    parse_net_dev(&content)
}

/// Parse `/proc/net/dev` content.
pub(crate) fn parse_net_dev(
    content: &str,
) -> Result<HashMap<String, NetCounters>, MaintainerError> {
    let mut interfaces = HashMap::new();

    for line in content.lines().skip(2) {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let (iface_name, rest) = match line.split_once(':') {
            Some((name, rest)) => (name.trim(), rest),
            None => continue,
        };

        let fields: Vec<&str> = rest.split_whitespace().collect();
        if fields.len() < 16 {
            continue;
        }

        let parse_u64 =
            |idx: usize| -> u64 { fields.get(idx).and_then(|s| s.parse().ok()).unwrap_or(0) };

        interfaces.insert(
            iface_name.to_string(),
            NetCounters {
                rx_bytes: parse_u64(0),
                rx_packets: parse_u64(1),
                rx_errors: parse_u64(2),
                tx_bytes: parse_u64(8),
                tx_packets: parse_u64(9),
                tx_errors: parse_u64(10),
            },
        );
    }

    Ok(interfaces)
}

/// Build network snapshots from current counters and an optional baseline.
pub fn build_network_snapshots(
    current: &HashMap<String, NetCounters>,
    baseline: Option<&NetBaseline>,
    elapsed: Duration,
) -> Vec<NetworkSnapshot> {
    let mut snapshots: Vec<NetworkSnapshot> = current
        .iter()
        .map(|(name, counters)| {
            let is_loopback = name == "lo";
            let (rx_rate, tx_rate) = compute_net_rates(name, counters, baseline, elapsed);

            NetworkSnapshot {
                interface: name.clone(),
                is_loopback,
                rx_bytes: counters.rx_bytes,
                tx_bytes: counters.tx_bytes,
                rx_packets: counters.rx_packets,
                tx_packets: counters.tx_packets,
                rx_errors: counters.rx_errors,
                tx_errors: counters.tx_errors,
                rx_bytes_per_sec: rx_rate,
                tx_bytes_per_sec: tx_rate,
            }
        })
        .collect();

    snapshots.sort_by(|a, b| a.interface.cmp(&b.interface));
    snapshots
}

/// Compute network rates from baseline to current.
fn compute_net_rates(
    name: &str,
    current: &NetCounters,
    baseline: Option<&NetBaseline>,
    elapsed: Duration,
) -> (Option<f64>, Option<f64>) {
    if elapsed.as_secs_f64() <= 0.0 {
        return (None, None);
    }

    let prev = baseline.and_then(|b| b.counters.get(name));
    match prev {
        Some(p) => {
            let rx_delta = current.rx_bytes.saturating_sub(p.rx_bytes);
            let tx_delta = current.tx_bytes.saturating_sub(p.tx_bytes);
            let secs = elapsed.as_secs_f64();
            (Some(rx_delta as f64 / secs), Some(tx_delta as f64 / secs))
        }
        None => (None, None),
    }
}

/// Compute summary totals excluding loopback interfaces.
pub fn summary_totals(snapshots: &[NetworkSnapshot]) -> (u64, u64) {
    let mut rx_total = 0u64;
    let mut tx_total = 0u64;
    for snap in snapshots {
        if !snap.is_loopback {
            rx_total += snap.rx_bytes;
            tx_total += snap.tx_bytes;
        }
    }
    (rx_total, tx_total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/proc")
    }

    #[test]
    fn parse_net_dev_from_fixture() {
        let counters = read_net_dev(&fixtures_path()).unwrap();
        assert!(counters.contains_key("lo"));
        assert!(counters.contains_key("eth0"));
        assert!(counters.contains_key("wlan0"));
        assert_eq!(counters["lo"].rx_bytes, 1_234_567_890);
        assert_eq!(counters["eth0"].rx_bytes, 9_876_543_210);
        assert_eq!(counters["eth0"].rx_errors, 5);
        assert_eq!(counters["eth0"].tx_bytes, 5_432_109_876);
        assert_eq!(counters["wlan0"].tx_packets, 1_234_567);
    }

    #[test]
    fn loopback_excluded_from_summary() {
        let counters = read_net_dev(&fixtures_path()).unwrap();
        let snapshots = build_network_snapshots(&counters, None, Duration::ZERO);
        let (rx, tx) = summary_totals(&snapshots);
        assert_eq!(rx, 9_876_543_210 + 3_456_789_012);
        assert_eq!(tx, 5_432_109_876 + 1_234_567_890);
    }

    #[test]
    fn loopback_retained_in_details() {
        let counters = read_net_dev(&fixtures_path()).unwrap();
        let snapshots = build_network_snapshots(&counters, None, Duration::ZERO);
        let lo = snapshots.iter().find(|s| s.interface == "lo").unwrap();
        assert!(lo.is_loopback);
        assert_eq!(lo.rx_bytes, 1_234_567_890);
    }

    #[test]
    fn rate_computation_with_baseline() {
        let prev = HashMap::from([(
            "eth0".to_string(),
            NetCounters {
                rx_bytes: 1000,
                tx_bytes: 2000,
                rx_packets: 10,
                tx_packets: 20,
                rx_errors: 0,
                tx_errors: 0,
            },
        )]);
        let curr = HashMap::from([(
            "eth0".to_string(),
            NetCounters {
                rx_bytes: 5000,
                tx_bytes: 6000,
                rx_packets: 50,
                tx_packets: 60,
                rx_errors: 0,
                tx_errors: 0,
            },
        )]);
        let baseline = NetBaseline {
            counters: prev,
            timestamp: Instant::now(),
        };
        let elapsed = Duration::from_secs(2);

        let snapshots = build_network_snapshots(&curr, Some(&baseline), elapsed);
        let eth0 = snapshots.iter().find(|s| s.interface == "eth0").unwrap();
        assert_eq!(eth0.rx_bytes_per_sec, Some(2000.0));
        assert_eq!(eth0.tx_bytes_per_sec, Some(2000.0));
    }

    #[test]
    fn counter_wrap_handled_with_saturating_sub() {
        let prev = HashMap::from([(
            "eth0".to_string(),
            NetCounters {
                rx_bytes: 10000,
                tx_bytes: 20000,
                rx_packets: 100,
                tx_packets: 200,
                rx_errors: 0,
                tx_errors: 0,
            },
        )]);
        let curr = HashMap::from([(
            "eth0".to_string(),
            NetCounters {
                rx_bytes: 5000, // "wrapped"
                tx_bytes: 8000,
                rx_packets: 50,
                tx_packets: 80,
                rx_errors: 0,
                tx_errors: 0,
            },
        )]);
        let baseline = NetBaseline {
            counters: prev,
            timestamp: Instant::now(),
        };
        let elapsed = Duration::from_secs(1);

        let snapshots = build_network_snapshots(&curr, Some(&baseline), elapsed);
        let eth0 = snapshots.iter().find(|s| s.interface == "eth0").unwrap();
        assert_eq!(eth0.rx_bytes_per_sec, Some(0.0));
        assert_eq!(eth0.tx_bytes_per_sec, Some(0.0));
    }

    #[test]
    fn first_sample_rates_are_none() {
        let counters = read_net_dev(&fixtures_path()).unwrap();
        let snapshots = build_network_snapshots(&counters, None, Duration::ZERO);
        for snap in &snapshots {
            assert_eq!(snap.rx_bytes_per_sec, None);
            assert_eq!(snap.tx_bytes_per_sec, None);
        }
    }
}
