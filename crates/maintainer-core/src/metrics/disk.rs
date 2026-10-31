//! Disk metric collector that reads `/proc/mounts` and `/proc/diskstats`.
//!
//! Excludes pseudo filesystems from mount enumeration. Computes read/write rates
//! from sector deltas between samples.

use std::collections::HashMap;
use std::fs;
use std::path::Path;
use std::time::{Duration, Instant};

use crate::error::MaintainerError;

/// Snapshot of a single mounted disk.
#[derive(Debug, Clone)]
pub struct DiskSnapshot {
    /// Block device name (e.g., "sda1").
    pub device: String,
    /// Mount point path.
    pub mount_point: String,
    /// Filesystem type (e.g., "ext4", "xfs").
    pub fs_type: String,
    /// Total space in bytes (from statvfs when available, otherwise 0).
    pub total_bytes: u64,
    /// Used space in bytes.
    pub used_bytes: u64,
    /// Available space in bytes.
    pub available_bytes: u64,
    /// Read bytes per second since last sample (None on first sample).
    pub read_bytes_per_sec: Option<f64>,
    /// Write bytes per second since last sample (None on first sample).
    pub write_bytes_per_sec: Option<f64>,
}

/// Pseudo filesystem types that should be excluded from disk enumeration.
const PSEUDO_FS_TYPES: &[&str] = &[
    "sysfs",
    "proc",
    "devtmpfs",
    "devpts",
    "tmpfs",
    "securityfs",
    "cgroup",
    "cgroup2",
    "pstore",
    "debugfs",
    "hugetlbfs",
    "mqueue",
    "configfs",
    "fusectl",
    "tracefs",
    "bpf",
    "autofs",
    "efivarfs",
    "binfmt_misc",
    "rpc_pipefs",
    "nsfs",
    "ramfs",
    "overlay",
];

/// Raw disk I/O counters from `/proc/diskstats`.
#[derive(Debug, Clone, Default)]
pub struct DiskIoCounters {
    /// Sectors read (field 6 in diskstats, 0-indexed from device fields).
    pub sectors_read: u64,
    /// Sectors written (field 10 in diskstats).
    pub sectors_written: u64,
}

/// Stored baseline for rate computation.
#[derive(Debug, Clone)]
pub struct DiskIoBaseline {
    pub counters: HashMap<String, DiskIoCounters>,
    pub timestamp: Instant,
}

/// A parsed mount entry from `/proc/mounts`.
#[derive(Debug, Clone)]
pub struct MountEntry {
    pub device: String,
    pub mount_point: String,
    pub fs_type: String,
}

/// Sector size in bytes (standard Linux block layer).
const SECTOR_SIZE_BYTES: u64 = 512;

/// Parse `/proc/mounts` and return only real filesystem mounts.
pub fn read_mounts(proc_path: &Path) -> Result<Vec<MountEntry>, MaintainerError> {
    let mounts_path = proc_path.join("mounts");
    let content = crate::metrics::read_proc_text(&mounts_path)?;

    parse_mounts(&content)
}

/// Capacity information for a single mount point from `statvfs`.
#[derive(Debug, Clone)]
pub struct MountCapacity {
    /// Total space in bytes.
    pub total_bytes: u64,
    /// Used space in bytes.
    pub used_bytes: u64,
    /// Available space in bytes (for unprivileged users).
    pub available_bytes: u64,
}

/// Query filesystem capacity for a mount point using `statvfs(2)`.
///
/// Returns `None` if the syscall fails (e.g., permission denied or stale NFS
/// mount). The caller treats this as an unavailable entry with a reason rather
/// than zeros.
pub fn query_mount_capacity(mount_point: &str) -> Option<MountCapacity> {
    use std::ffi::CString;

    let c_path = match CString::new(mount_point) {
        Ok(p) => p,
        Err(_) => return None,
    };

    let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };

    // Safety: `c_path` is a valid NUL-terminated C string pointing to an
    // existing mount path; `stat` is a zeroed statvfs struct with adequate
    let ret = unsafe { libc::statvfs(c_path.as_ptr(), &mut stat) };
    if ret != 0 {
        return None;
    }

    #[allow(clippy::unnecessary_cast)]
    let block_size: u64 = stat.f_frsize as u64;
    let total_bytes = stat.f_blocks * block_size;
    let available_bytes = stat.f_bavail * block_size;
    let free_bytes = stat.f_bfree * block_size;
    let used_bytes = total_bytes.saturating_sub(free_bytes);

    Some(MountCapacity {
        total_bytes,
        used_bytes,
        available_bytes,
    })
}

/// Read mounts and populate capacity using `statvfs` for each real mount.
///
/// Mounts where `statvfs` fails become entries with zero capacity rather than
/// being omitted — the caller can inspect `total_bytes == 0` to identify
/// unavailable entries.
pub fn read_mounts_with_capacity(proc_path: &Path) -> Result<Vec<DiskSnapshot>, MaintainerError> {
    let mounts = read_mounts(proc_path)?;
    let mut snapshots = Vec::with_capacity(mounts.len());

    for mount in mounts {
        let capacity = query_mount_capacity(&mount.mount_point);
        let (total, used, available) = match capacity {
            Some(c) => (c.total_bytes, c.used_bytes, c.available_bytes),
            None => (0, 0, 0),
        };
        snapshots.push(DiskSnapshot {
            device: mount.device,
            mount_point: mount.mount_point,
            fs_type: mount.fs_type,
            total_bytes: total,
            used_bytes: used,
            available_bytes: available,
            read_bytes_per_sec: None,
            write_bytes_per_sec: None,
        });
    }

    Ok(snapshots)
}

/// Parse mount content, filtering pseudo filesystems.
pub(crate) fn parse_mounts(content: &str) -> Result<Vec<MountEntry>, MaintainerError> {
    let mut mounts = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 3 {
            continue;
        }

        let fs_type = fields[2];
        if PSEUDO_FS_TYPES.contains(&fs_type) {
            continue;
        }

        let mount_point = decode_mount_octal(fields[1]);

        mounts.push(MountEntry {
            device: fields[0].to_string(),
            mount_point,
            fs_type: fs_type.to_string(),
        });
    }

    Ok(mounts)
}

/// Decode octal escape sequences in mount point strings (e.g., `\040` → ` `).
fn decode_mount_octal(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut octal = String::with_capacity(3);
            for _ in 0..3 {
                if let Some(&next) = chars.as_str().as_bytes().first() {
                    if (b'0'..=b'7').contains(&next) {
                        octal.push(next as char);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            if octal.len() == 3 {
                if let Ok(byte) = u8::from_str_radix(&octal, 8) {
                    result.push(byte as char);
                } else {
                    result.push('\\');
                    result.push_str(&octal);
                }
            } else {
                result.push('\\');
                result.push_str(&octal);
            }
        } else {
            result.push(c);
        }
    }

    result
}

/// Parse `/proc/diskstats` into a map of device name -> I/O counters.
pub fn read_diskstats(
    proc_path: &Path,
) -> Result<HashMap<String, DiskIoCounters>, MaintainerError> {
    let path = proc_path.join("diskstats");
    let content = fs::read_to_string(&path)
        .map_err(|e| MaintainerError::io(format!("reading {}", path.display()), e))?;

    parse_diskstats(&content)
}

/// Parse diskstats content.
pub(crate) fn parse_diskstats(
    content: &str,
) -> Result<HashMap<String, DiskIoCounters>, MaintainerError> {
    let mut map = HashMap::new();

    for line in content.lines() {
        let fields: Vec<&str> = line.split_whitespace().collect();
        // diskstats format: major minor name rd_ios rd_merges rd_sectors rd_ticks
        //                   wr_ios wr_merges wr_sectors wr_ticks ...
        if fields.len() < 14 {
            continue;
        }

        let name = fields[2].to_string();
        let sectors_read = fields[5].parse::<u64>().unwrap_or(0);
        let sectors_written = fields[9].parse::<u64>().unwrap_or(0);

        map.insert(
            name,
            DiskIoCounters {
                sectors_read,
                sectors_written,
            },
        );
    }

    Ok(map)
}

/// Compute disk I/O rates from baseline and current counters.
pub fn compute_disk_rates(
    device_name: &str,
    baseline: &DiskIoBaseline,
    current_counters: &HashMap<String, DiskIoCounters>,
    elapsed: Duration,
) -> (Option<f64>, Option<f64>) {
    if elapsed.as_secs_f64() <= 0.0 {
        return (None, None);
    }

    let short_name = device_name.strip_prefix("/dev/").unwrap_or(device_name);

    let prev = baseline.counters.get(short_name);
    let curr = current_counters.get(short_name);

    match (prev, curr) {
        (Some(p), Some(c)) => {
            let read_delta = c.sectors_read.saturating_sub(p.sectors_read);
            let write_delta = c.sectors_written.saturating_sub(p.sectors_written);
            let secs = elapsed.as_secs_f64();

            let read_rate = (read_delta * SECTOR_SIZE_BYTES) as f64 / secs;
            let write_rate = (write_delta * SECTOR_SIZE_BYTES) as f64 / secs;
            (Some(read_rate), Some(write_rate))
        }
        _ => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/proc")
    }

    #[test]
    fn parse_mounts_excludes_pseudo_filesystems() {
        let mounts = read_mounts(&fixtures_path()).unwrap();
        let fs_types: Vec<&str> = mounts.iter().map(|m| m.fs_type.as_str()).collect();
        assert!(!fs_types.contains(&"tmpfs"));
        assert!(!fs_types.contains(&"proc"));
        assert!(!fs_types.contains(&"sysfs"));
        assert!(!fs_types.contains(&"devtmpfs"));
        assert!(!fs_types.contains(&"devpts"));
        assert!(!fs_types.contains(&"cgroup2"));
        assert!(!fs_types.contains(&"fusectl"));
        assert!(fs_types.contains(&"ext4"));
        assert!(fs_types.contains(&"xfs"));
    }

    #[test]
    fn mount_with_spaces_decoded() {
        let mounts = read_mounts(&fixtures_path()).unwrap();
        let nvme_mount = mounts
            .iter()
            .find(|m| m.device == "/dev/nvme0n1p1")
            .unwrap();
        assert_eq!(nvme_mount.mount_point, "/mnt/data drive");
    }

    #[test]
    fn parse_diskstats_from_fixture() {
        let stats = read_diskstats(&fixtures_path()).unwrap();
        assert!(stats.contains_key("sda"));
        assert!(stats.contains_key("sda1"));
        assert!(stats.contains_key("nvme0n1p1"));
        assert_eq!(stats["sda1"].sectors_read, 1_800_000);
        assert_eq!(stats["sda1"].sectors_written, 2_800_000);
    }

    #[test]
    fn rate_computation_with_elapsed_time() {
        let prev = HashMap::from([(
            "sda1".to_string(),
            DiskIoCounters {
                sectors_read: 1000,
                sectors_written: 2000,
            },
        )]);
        let curr = HashMap::from([(
            "sda1".to_string(),
            DiskIoCounters {
                sectors_read: 3000,
                sectors_written: 4000,
            },
        )]);
        let baseline = DiskIoBaseline {
            counters: prev,
            timestamp: Instant::now(),
        };
        let elapsed = Duration::from_secs(2);

        let (read_rate, write_rate) = compute_disk_rates("/dev/sda1", &baseline, &curr, elapsed);
        assert_eq!(read_rate, Some(512_000.0));
        assert_eq!(write_rate, Some(512_000.0));
    }

    #[test]
    fn rate_with_zero_elapsed_returns_none() {
        let prev = HashMap::from([(
            "sda1".to_string(),
            DiskIoCounters {
                sectors_read: 1000,
                sectors_written: 2000,
            },
        )]);
        let curr = prev.clone();
        let baseline = DiskIoBaseline {
            counters: prev,
            timestamp: Instant::now(),
        };

        let (read_rate, write_rate) =
            compute_disk_rates("/dev/sda1", &baseline, &curr, Duration::ZERO);
        assert_eq!(read_rate, None);
        assert_eq!(write_rate, None);
    }

    #[test]
    fn counter_reset_uses_saturating_sub() {
        let prev = HashMap::from([(
            "sda1".to_string(),
            DiskIoCounters {
                sectors_read: 5000,
                sectors_written: 8000,
            },
        )]);
        let curr = HashMap::from([(
            "sda1".to_string(),
            DiskIoCounters {
                sectors_read: 1000,
                sectors_written: 2000,
            },
        )]);
        let baseline = DiskIoBaseline {
            counters: prev,
            timestamp: Instant::now(),
        };
        let elapsed = Duration::from_secs(1);

        let (read_rate, write_rate) = compute_disk_rates("/dev/sda1", &baseline, &curr, elapsed);
        assert_eq!(read_rate, Some(0.0));
        assert_eq!(write_rate, Some(0.0));
    }
}
