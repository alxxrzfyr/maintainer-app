//! Process metric collector that reads `/proc/[pid]/stat` and `/proc/[pid]/status`.
//!
//! Uses PID + start_time_ticks for unique process identity to prevent
//! PID-reuse confusion. Does not persist command lines or show arguments
//! by default for privacy.

use std::fs;
use std::path::Path;

use crate::error::MaintainerError;

/// Unique identity of a process (PID + start time prevents reuse issues).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ProcessIdentity {
    /// Process ID.
    pub pid: u32,
    /// Process start time in clock ticks since boot (field 22 of `/proc/[pid]/stat`).
    pub start_time_ticks: u64,
}

/// Snapshot of a single process.
#[derive(Debug, Clone)]
pub struct ProcessSnapshot {
    /// Unique identity (pid + start_time).
    pub identity: ProcessIdentity,
    /// Process command name (from comm field in stat, without path or arguments).
    pub name: String,
    /// Process state character (R, S, D, Z, T, etc.).
    pub state: char,
    /// Parent PID.
    pub ppid: u32,
    /// Real user ID.
    pub uid: u32,
    /// Resident set size in bytes.
    pub rss_bytes: u64,
    /// User-mode CPU time in ticks.
    pub utime_ticks: u64,
    /// Kernel-mode CPU time in ticks.
    pub stime_ticks: u64,
    /// Number of threads.
    pub num_threads: u32,
}

/// Read a list of process snapshots from `/proc`.
pub fn read_processes(proc_path: &Path) -> Result<Vec<ProcessSnapshot>, MaintainerError> {
    let mut processes = Vec::new();

    let entries = fs::read_dir(proc_path)
        .map_err(|e| MaintainerError::io(format!("reading {}", proc_path.display()), e))?;

    for entry in entries {
        let entry = match entry {
            Ok(e) => e,
            Err(_) => continue,
        };

        // Only process directories named with digits (PIDs)
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if !name_str.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        let pid_dir = entry.path();

        // Try reading process info; skip silently on permission denied or vanished
        match read_single_process(&pid_dir) {
            Ok(snapshot) => processes.push(snapshot),
            Err(_) => continue, // Process vanished or permission denied
        }
    }

    // Sort by PID for deterministic output
    processes.sort_by_key(|p| p.identity.pid);
    Ok(processes)
}

/// Read a single process from its `/proc/[pid]` directory.
fn read_single_process(pid_dir: &Path) -> Result<ProcessSnapshot, MaintainerError> {
    let stat_content = fs::read_to_string(pid_dir.join("stat"))
        .map_err(|e| MaintainerError::io(format!("reading {}/stat", pid_dir.display()), e))?;

    let status_content = fs::read_to_string(pid_dir.join("status"))
        .map_err(|e| MaintainerError::io(format!("reading {}/status", pid_dir.display()), e))?;

    let stat = parse_proc_stat(&stat_content)?;
    let status = parse_proc_status(&status_content)?;

    Ok(ProcessSnapshot {
        identity: ProcessIdentity {
            pid: stat.pid,
            start_time_ticks: stat.start_time_ticks,
        },
        name: stat.comm,
        state: stat.state,
        ppid: stat.ppid,
        uid: status.uid,
        rss_bytes: stat.rss_pages * page_size(),
        utime_ticks: stat.utime,
        stime_ticks: stat.stime,
        num_threads: stat.num_threads,
    })
}

/// Parsed fields from `/proc/[pid]/stat`.
struct StatFields {
    pid: u32,
    comm: String,
    state: char,
    ppid: u32,
    utime: u64,
    stime: u64,
    num_threads: u32,
    start_time_ticks: u64,
    rss_pages: u64,
}

/// Parse `/proc/[pid]/stat` handling process names with parentheses.
///
/// The format is: pid (comm) state ppid ...
/// The comm field can contain spaces and parentheses, so we find the last ')'.
fn parse_proc_stat(content: &str) -> Result<StatFields, MaintainerError> {
    let content = content.trim();

    // Find the opening and closing parentheses for comm
    let open_paren = content.find('(').ok_or_else(|| {
        MaintainerError::invalid_data("/proc/[pid]/stat", "missing opening parenthesis")
    })?;
    let close_paren = content.rfind(')').ok_or_else(|| {
        MaintainerError::invalid_data("/proc/[pid]/stat", "missing closing parenthesis")
    })?;

    let pid_str = &content[..open_paren].trim();
    let pid: u32 = pid_str.parse().map_err(|_| {
        MaintainerError::invalid_data("/proc/[pid]/stat", format!("invalid pid: '{pid_str}'"))
    })?;

    let comm = content[open_paren + 1..close_paren].to_string();

    // Fields after the closing paren
    let rest = &content[close_paren + 2..]; // skip ") "
    let fields: Vec<&str> = rest.split_whitespace().collect();

    if fields.len() < 20 {
        return Err(MaintainerError::invalid_data(
            "/proc/[pid]/stat",
            format!(
                "too few fields after comm: {} (need at least 20)",
                fields.len()
            ),
        ));
    }

    let state = fields[0].chars().next().unwrap_or('?');
    let ppid: u32 = fields[1].parse().unwrap_or(0);
    let utime: u64 = fields[11].parse().unwrap_or(0);
    let stime: u64 = fields[12].parse().unwrap_or(0);
    let num_threads: u32 = fields[17].parse().unwrap_or(1);
    let start_time_ticks: u64 = fields[19].parse().unwrap_or(0);
    let rss_pages: u64 = fields[21].parse().unwrap_or(0);

    Ok(StatFields {
        pid,
        comm,
        state,
        ppid,
        utime,
        stime,
        num_threads,
        start_time_ticks,
        rss_pages,
    })
}

/// Parsed fields from `/proc/[pid]/status`.
struct StatusFields {
    uid: u32,
}

/// Parse `/proc/[pid]/status` for UID.
fn parse_proc_status(content: &str) -> Result<StatusFields, MaintainerError> {
    let mut uid = 0u32;

    for line in content.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            let fields: Vec<&str> = rest.split_whitespace().collect();
            if let Some(real_uid) = fields.first() {
                uid = real_uid.parse().unwrap_or(0);
            }
            break;
        }
    }

    Ok(StatusFields { uid })
}

/// Get the system page size.
fn page_size() -> u64 {
    // Default to 4096 which is by far the most common on Linux x86_64/arm64
    4096
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixtures_path() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../fixtures/proc")
    }

    #[test]
    fn read_processes_from_fixture() {
        let processes = read_processes(&fixtures_path()).unwrap();
        // Should find processes 1, 42, 1234 from fixtures
        assert!(
            processes.len() >= 3,
            "expected at least 3 processes, got {}",
            processes.len()
        );
    }

    #[test]
    fn process_identity_includes_start_time() {
        let processes = read_processes(&fixtures_path()).unwrap();
        let firefox = processes.iter().find(|p| p.identity.pid == 1234).unwrap();
        // From fixture: start_time_ticks is field 22 (0-indexed from after comm) = fields[19]
        assert_eq!(firefox.identity.start_time_ticks, 98765);
        assert_eq!(firefox.name, "firefox");
        assert_eq!(firefox.state, 'S');
        assert_eq!(firefox.ppid, 1100);
        assert_eq!(firefox.uid, 1000);
    }

    #[test]
    fn process_name_with_parentheses() {
        let processes = read_processes(&fixtures_path()).unwrap();
        let kworker = processes.iter().find(|p| p.identity.pid == 42).unwrap();
        assert_eq!(kworker.name, "kworker/0:1-events");
        assert_eq!(kworker.uid, 0);
    }

    #[test]
    fn systemd_process_parsed() {
        let processes = read_processes(&fixtures_path()).unwrap();
        let systemd = processes.iter().find(|p| p.identity.pid == 1).unwrap();
        assert_eq!(systemd.name, "systemd");
        assert_eq!(systemd.ppid, 0);
        assert_eq!(systemd.uid, 0);
        assert_eq!(systemd.identity.start_time_ticks, 1);
    }

    #[test]
    fn rss_bytes_computed_from_pages() {
        let processes = read_processes(&fixtures_path()).unwrap();
        let firefox = processes.iter().find(|p| p.identity.pid == 1234).unwrap();
        // From fixture: rss field (field index 23, which is fields[21] after comm) = 234567 pages
        assert_eq!(firefox.rss_bytes, 234567 * 4096);
    }

    #[test]
    fn vanished_process_directory_skipped() {
        // Non-existent PID dir should be skipped gracefully
        let result = read_single_process(Path::new("/proc/99999999"));
        assert!(result.is_err()); // Would fail, but read_processes catches this
    }

    #[test]
    fn parse_stat_with_spaces_in_name() {
        let content = "12345 (Web Content) S 1000 12345 1000 0 -1 4194304 100 0 0 0 200 50 0 0 20 0 5 0 5000 1234567890 5678 18446744073709551615 0 0 0 0 0 0 0 0 0 0 0 0 17 1 0 0 0 0 0 0 0 0 0 0 0 0 0";
        let stat = parse_proc_stat(content).unwrap();
        assert_eq!(stat.pid, 12345);
        assert_eq!(stat.comm, "Web Content");
        assert_eq!(stat.state, 'S');
        assert_eq!(stat.ppid, 1000);
    }
}
