//! CPU metric collector that reads `/proc/stat` and computes utilization from deltas.

#[cfg(test)]
use std::fs;
use std::path::Path;
use std::time::Instant;

use crate::error::MaintainerError;

/// Snapshot of CPU utilization derived from delta between two samples.
#[derive(Debug, Clone)]
pub struct CpuSnapshot {
    /// Overall CPU usage percentage (0.0–100.0).
    pub total_percent: f64,
    /// Per-core usage percentages.
    pub per_core_percent: Vec<f64>,
}

/// Raw counters from a single `/proc/stat` read (in jiffies).
#[derive(Debug, Clone)]
pub struct CpuSample {
    pub total: CpuCounters,
    pub cores: Vec<CpuCounters>,
    pub timestamp: Instant,
}

/// Individual CPU time counters parsed from one line of `/proc/stat`.
#[derive(Debug, Clone, Default)]
pub struct CpuCounters {
    pub user: u64,
    pub nice: u64,
    pub system: u64,
    pub idle: u64,
    pub iowait: u64,
    pub irq: u64,
    pub softirq: u64,
    pub steal: u64,
    pub guest: u64,
    pub guest_nice: u64,
}

impl CpuCounters {
    /// Total jiffies (all fields summed).
    fn total(&self) -> u64 {
        self.user
            + self.nice
            + self.system
            + self.idle
            + self.iowait
            + self.irq
            + self.softirq
            + self.steal
            + self.guest
            + self.guest_nice
    }

    /// Idle jiffies (idle + iowait).
    fn idle_total(&self) -> u64 {
        self.idle + self.iowait
    }
}

/// Parse a single cpu line from `/proc/stat` into counters.
fn parse_cpu_line(line: &str) -> Result<CpuCounters, MaintainerError> {
    let fields: Vec<&str> = line.split_whitespace().collect();
    if fields.len() < 8 {
        return Err(MaintainerError::invalid_data(
            "/proc/stat",
            format!("cpu line has fewer than 8 fields: {line}"),
        ));
    }

    let parse_field =
        |idx: usize| -> Result<u64, MaintainerError> {
            fields.get(idx).unwrap_or(&"0").parse::<u64>().map_err(|e| {
                MaintainerError::invalid_data("/proc/stat", format!("field {idx}: {e}"))
            })
        };

    Ok(CpuCounters {
        user: parse_field(1)?,
        nice: parse_field(2)?,
        system: parse_field(3)?,
        idle: parse_field(4)?,
        iowait: parse_field(5)?,
        irq: parse_field(6)?,
        softirq: parse_field(7)?,
        steal: parse_field(8).unwrap_or(0),
        guest: parse_field(9).unwrap_or(0),
        guest_nice: parse_field(10).unwrap_or(0),
    })
}

/// Read and parse `/proc/stat` from the given base path.
pub fn read_cpu_sample(proc_path: &Path, now: Instant) -> Result<CpuSample, MaintainerError> {
    let stat_path = proc_path.join("stat");
    let content = crate::metrics::read_proc_text(&stat_path)?;

    let mut total = None;
    let mut cores = Vec::new();

    for line in content.lines() {
        if line.starts_with("cpu ") {
            total = Some(parse_cpu_line(line)?);
        } else if line.starts_with("cpu") {
            let first_field = line.split_whitespace().next().unwrap_or("");
            if first_field.len() > 3 && first_field[3..].chars().all(|c| c.is_ascii_digit()) {
                cores.push(parse_cpu_line(line)?);
            }
        }
    }

    let total = total
        .ok_or_else(|| MaintainerError::invalid_data("/proc/stat", "missing aggregate cpu line"))?;

    Ok(CpuSample {
        total,
        cores,
        timestamp: now,
    })
}

/// Compute CPU usage percentage from the delta between two counter sets.
/// Returns 0.0 if total delta is zero (prevents division by zero).
fn compute_percent(prev: &CpuCounters, curr: &CpuCounters) -> f64 {
    let total_delta = curr.total().saturating_sub(prev.total());
    if total_delta == 0 {
        return 0.0;
    }
    let idle_delta = curr.idle_total().saturating_sub(prev.idle_total());
    let busy_delta = total_delta.saturating_sub(idle_delta);
    (busy_delta as f64 / total_delta as f64) * 100.0
}

/// Compute a full `CpuSnapshot` from two successive samples.
/// Returns `None` if this is the first sample (no previous baseline).
pub fn compute_cpu_snapshot(prev: &CpuSample, curr: &CpuSample) -> CpuSnapshot {
    let total_percent = compute_percent(&prev.total, &curr.total);

    let per_core_percent: Vec<f64> = prev
        .cores
        .iter()
        .zip(curr.cores.iter())
        .map(|(p, c)| compute_percent(p, c))
        .collect();

    CpuSnapshot {
        total_percent,
        per_core_percent,
    }
}

/// Parse CPU sample from content string (used for testing with alternate fixtures).
pub fn read_cpu_sample_from_content(
    content: &str,
    now: Instant,
) -> Result<CpuSample, MaintainerError> {
    let mut total = None;
    let mut cores = Vec::new();

    for line in content.lines() {
        if line.starts_with("cpu ") {
            total = Some(parse_cpu_line(line)?);
        } else if line.starts_with("cpu") {
            let first_field = line.split_whitespace().next().unwrap_or("");
            if first_field.len() > 3 && first_field[3..].chars().all(|c| c.is_ascii_digit()) {
                cores.push(parse_cpu_line(line)?);
            }
        }
    }

    let total = total
        .ok_or_else(|| MaintainerError::invalid_data("/proc/stat", "missing aggregate cpu line"))?;

    Ok(CpuSample {
        total,
        cores,
        timestamp: now,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/proc")
    }

    #[test]
    fn parse_cpu_sample_from_fixture() {
        let path = fixtures_path();
        let sample = read_cpu_sample(&path, Instant::now()).unwrap();

        assert_eq!(sample.total.user, 10132153);
        assert_eq!(sample.total.nice, 290696);
        assert_eq!(sample.total.system, 3084719);
        assert_eq!(sample.total.idle, 46828483);
        assert_eq!(sample.total.iowait, 16683);
        assert_eq!(sample.total.irq, 0);
        assert_eq!(sample.total.softirq, 25195);
        assert_eq!(sample.cores.len(), 4);
    }

    #[test]
    fn first_sample_no_snapshot_possible() {
        let path = fixtures_path();
        let sample = read_cpu_sample(&path, Instant::now()).unwrap();
        let snapshot = compute_cpu_snapshot(&sample, &sample);
        assert_eq!(snapshot.total_percent, 0.0);
        for core in &snapshot.per_core_percent {
            assert_eq!(*core, 0.0);
        }
    }

    #[test]
    fn delta_computation_produces_plausible_percent() {
        let path = fixtures_path();
        let sample1 = read_cpu_sample(&path, Instant::now()).unwrap();

        let path2 = fixtures_path().parent().unwrap().join("proc");
        let sample2 = read_cpu_sample_from_content(
            &fs::read_to_string(path2.join("stat_sample2")).unwrap(),
            Instant::now(),
        )
        .unwrap();

        let snapshot = compute_cpu_snapshot(&sample1, &sample2);
        assert!(
            snapshot.total_percent > 0.0,
            "expected >0, got {}",
            snapshot.total_percent
        );
        assert!(snapshot.total_percent <= 100.0);
        assert_eq!(snapshot.per_core_percent.len(), 4);
        for pct in &snapshot.per_core_percent {
            assert!(
                *pct >= 0.0 && *pct <= 100.0,
                "core percent out of range: {pct}"
            );
        }
    }

    #[test]
    fn zero_elapsed_returns_zero_percent() {
        let counters = CpuCounters::default();
        let pct = compute_percent(&counters, &counters);
        assert_eq!(pct, 0.0);
    }

    #[test]
    fn counter_reset_handled_gracefully() {
        let prev = CpuCounters {
            user: 1000,
            nice: 100,
            system: 500,
            idle: 5000,
            iowait: 100,
            irq: 50,
            softirq: 50,
            steal: 0,
            guest: 0,
            guest_nice: 0,
        };
        let curr = CpuCounters {
            user: 500, // "reset" - lower than prev
            nice: 50,
            system: 250,
            idle: 2500,
            iowait: 50,
            irq: 25,
            softirq: 25,
            steal: 0,
            guest: 0,
            guest_nice: 0,
        };
        let pct = compute_percent(&prev, &curr);
        assert_eq!(pct, 0.0);
    }
}
