//! System metric collectors that read Linux interfaces to produce typed snapshots.
//!
//! Each collector parses `/proc` or `/sys` to produce strongly-typed metric values
//! with explicit units. Parsers accept a base path parameter so tests can point at
//! fixture files instead of real system interfaces.

pub mod connection;
pub mod cpu;
pub mod disk;
pub mod memory;
pub mod network;
pub mod processes;
pub mod speed_test;
pub mod temperature;
pub mod terminate;

use std::time::Instant;

use crate::capability::Capability;
use crate::error::MaintainerError;

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
pub use crate::metrics::terminate::{
    ProcReader, RealProcReader, TerminateResult, terminate_process,
};

/// A source that can produce a complete system snapshot at a point in time.
pub trait MetricSource: Send + Sync {
    /// Sample all metrics at the given instant.
    fn sample(&mut self, now: Instant) -> Result<SystemSnapshot, MaintainerError>;
}

/// A complete system snapshot containing all metric categories.
#[derive(Debug)]
pub struct SystemSnapshot {
    pub cpu: Capability<CpuSnapshot>,
    pub memory: Capability<MemorySnapshot>,
    pub disks: Capability<Vec<DiskSnapshot>>,
    pub network: Capability<Vec<NetworkSnapshot>>,
    pub temperatures: Capability<Vec<TemperatureReading>>,
    pub processes: Capability<Vec<ProcessSnapshot>>,
}
