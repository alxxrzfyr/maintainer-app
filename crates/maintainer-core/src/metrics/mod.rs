//! System metric collectors that read Linux interfaces to produce typed snapshots.
//!
//! Each collector parses `/proc` or `/sys` to produce strongly-typed metric values
//! with explicit units. Parsers accept a base path parameter so tests can point at
//! fixture files instead of real system interfaces.

pub mod collector;
pub mod connection;
pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod processes;
pub mod speed_test;
pub mod temperature;
pub mod terminate;

use std::io::Read;
use std::path::Path;
use std::time::Instant;

use crate::capability::Capability;
use crate::error::MaintainerError;

pub use crate::metrics::collector::SystemCollector;
pub use crate::metrics::connection::{ConnectionDetails, WifiInfo, read_connection_details};
pub use crate::metrics::cpu::{
    CpuCounters, CpuSample, CpuSnapshot, compute_cpu_snapshot, read_cpu_sample,
};
pub use crate::metrics::disk::{
    DiskIoCounters, DiskSnapshot, MountEntry, read_diskstats, read_mounts,
    read_mounts_with_capacity,
};
pub use crate::metrics::memory::{MemorySnapshot, read_memory};
pub use crate::metrics::network::{NetCounters, NetworkSnapshot, read_net_dev};
pub use crate::metrics::processes::{ProcessIdentity, ProcessSnapshot, read_processes};
pub use crate::metrics::speed_test::{
    SpeedTestResult, SpeedTestStage, check_curl_available, run_speed_test,
};
pub use crate::metrics::temperature::{
    TemperatureCollector, TemperatureGroup, TemperatureReading, group_temperatures,
    read_temperatures,
};
pub use crate::metrics::terminate::{ProcessHandle, ProcessHandleError};

/// A source that can produce a complete system snapshot at a point in time.
pub trait MetricSource: Send + Sync {
    /// Sample all metrics at the given instant.
    fn sample(&mut self, now: Instant) -> Result<SystemSnapshot, MaintainerError>;
}

/// Metric samples, with a process update only when explicitly requested.
#[derive(Debug)]
pub struct SystemSnapshot {
    pub cpu: Capability<CpuSnapshot>,
    pub memory: Capability<MemorySnapshot>,
    pub disks: Capability<Vec<DiskSnapshot>>,
    pub network: Capability<Vec<NetworkSnapshot>>,
    pub temperatures: Capability<Vec<TemperatureReading>>,
    pub processes: Option<Capability<Vec<ProcessSnapshot>>>,
}

// Proc files report no useful length. Reserve enough for typical samples to
// avoid repeated small reads; read_to_string still grows for larger hosts.
fn read_proc_text(path: &Path) -> Result<String, MaintainerError> {
    let read = || -> std::io::Result<String> {
        let mut file = std::fs::File::open(path)?;
        let mut content = String::with_capacity(16 * 1024);
        file.read_to_string(&mut content)?;
        Ok(content)
    };
    read().map_err(|error| MaintainerError::io(format!("reading {}", path.display()), error))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_text_reads_complete_large_samples() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("mounts");
        let content = "/dev/sda1 / ext4 rw 0 0\n".repeat(2048);
        std::fs::write(&path, &content).unwrap();
        assert_eq!(read_proc_text(&path).unwrap(), content);
    }

    #[test]
    fn proc_text_missing_file_preserves_read_context_and_io_kind() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("missing");
        let error = read_proc_text(&path).unwrap_err();
        match error {
            MaintainerError::Io { context, source } => {
                assert_eq!(context, format!("reading {}", path.display()));
                assert_eq!(source.kind(), std::io::ErrorKind::NotFound);
            }
            other => panic!("expected contextual I/O error, got {other:?}"),
        }
    }
}
