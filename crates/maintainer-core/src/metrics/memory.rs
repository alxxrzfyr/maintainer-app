//! Memory metric collector that reads `/proc/meminfo`.
//!
//! Uses correct Linux semantics: "used" = total - available, not total - free.
//! This properly accounts for reclaimable cache and buffers.

use std::collections::HashMap;
use std::fs;
use std::path::Path;

use crate::error::MaintainerError;

/// Snapshot of system memory state. All values in bytes.
#[derive(Debug, Clone)]
pub struct MemorySnapshot {
    /// Total physical memory in bytes.
    pub total_bytes: u64,
    /// Available memory in bytes (total minus truly-in-use memory).
    pub available_bytes: u64,
    /// Used memory in bytes (total - available).
    pub used_bytes: u64,
    /// Buffer cache in bytes.
    pub buffers_bytes: u64,
    /// Page cache in bytes.
    pub cached_bytes: u64,
    /// Total swap space in bytes.
    pub swap_total_bytes: u64,
    /// Free (unused) swap in bytes.
    pub swap_free_bytes: u64,
    /// Used swap in bytes.
    pub swap_used_bytes: u64,
}

/// Parse `/proc/meminfo` from the given base path.
pub fn read_memory(proc_path: &Path) -> Result<MemorySnapshot, MaintainerError> {
    let meminfo_path = proc_path.join("meminfo");
    let content = fs::read_to_string(&meminfo_path)
        .map_err(|e| MaintainerError::io(format!("reading {}", meminfo_path.display()), e))?;

    parse_meminfo(&content)
}

/// Parse meminfo content into a typed snapshot.
pub(crate) fn parse_meminfo(content: &str) -> Result<MemorySnapshot, MaintainerError> {
    let mut fields: HashMap<&str, u64> = HashMap::new();

    for line in content.lines() {
        let mut parts = line.splitn(2, ':');
        let key = match parts.next() {
            Some(k) => k.trim(),
            None => continue,
        };
        let value_str = match parts.next() {
            Some(v) => v.trim(),
            None => continue,
        };
        // Strip " kB" suffix and parse the number
        let numeric_str = value_str.strip_suffix(" kB").unwrap_or(value_str).trim();
        if let Ok(val) = numeric_str.parse::<u64>() {
            fields.insert(key, val);
        }
    }

    let total_kb = fields
        .get("MemTotal")
        .copied()
        .ok_or_else(|| MaintainerError::invalid_data("/proc/meminfo", "missing MemTotal"))?;

    let available_kb = fields
        .get("MemAvailable")
        .copied()
        .ok_or_else(|| MaintainerError::invalid_data("/proc/meminfo", "missing MemAvailable"))?;

    let buffers_kb = fields.get("Buffers").copied().unwrap_or(0);
    let cached_kb = fields.get("Cached").copied().unwrap_or(0);
    let swap_total_kb = fields.get("SwapTotal").copied().unwrap_or(0);
    let swap_free_kb = fields.get("SwapFree").copied().unwrap_or(0);

    let total_bytes = total_kb * 1024;
    let available_bytes = available_kb * 1024;
    // Linux semantics: used = total - available
    let used_bytes = total_bytes.saturating_sub(available_bytes);

    let swap_total_bytes = swap_total_kb * 1024;
    let swap_free_bytes = swap_free_kb * 1024;
    let swap_used_bytes = swap_total_bytes.saturating_sub(swap_free_bytes);

    Ok(MemorySnapshot {
        total_bytes,
        available_bytes,
        used_bytes,
        buffers_bytes: buffers_kb * 1024,
        cached_bytes: cached_kb * 1024,
        swap_total_bytes,
        swap_free_bytes,
        swap_used_bytes,
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
    fn parse_memory_from_fixture() {
        let snapshot = read_memory(&fixtures_path()).unwrap();

        // MemTotal: 16384000 kB = 16384000 * 1024 bytes
        assert_eq!(snapshot.total_bytes, 16_384_000 * 1024);
        // MemAvailable: 8192000 kB
        assert_eq!(snapshot.available_bytes, 8_192_000 * 1024);
        // used = total - available
        assert_eq!(snapshot.used_bytes, (16_384_000 - 8_192_000) * 1024);
        // Buffers: 512000 kB
        assert_eq!(snapshot.buffers_bytes, 512_000 * 1024);
        // Cached: 4096000 kB
        assert_eq!(snapshot.cached_bytes, 4_096_000 * 1024);
        // SwapTotal: 4096000 kB
        assert_eq!(snapshot.swap_total_bytes, 4_096_000 * 1024);
        // SwapFree: 3072000 kB
        assert_eq!(snapshot.swap_free_bytes, 3_072_000 * 1024);
        // Swap used = total - free
        assert_eq!(snapshot.swap_used_bytes, (4_096_000 - 3_072_000) * 1024);
    }

    #[test]
    fn used_equals_total_minus_available() {
        let content =
            "MemTotal:       8000000 kB\nMemFree:        1000000 kB\nMemAvailable:   5000000 kB\n";
        let snapshot = parse_meminfo(content).unwrap();
        // used = total - available, NOT total - free
        assert_eq!(snapshot.used_bytes, (8_000_000 - 5_000_000) * 1024);
        assert_ne!(snapshot.used_bytes, (8_000_000 - 1_000_000) * 1024);
    }

    #[test]
    fn missing_mem_total_is_error() {
        let content = "MemFree:  1000000 kB\nMemAvailable:  5000000 kB\n";
        let result = parse_meminfo(content);
        assert!(result.is_err());
    }

    #[test]
    fn missing_mem_available_is_error() {
        let content = "MemTotal:  8000000 kB\nMemFree:  1000000 kB\n";
        let result = parse_meminfo(content);
        assert!(result.is_err());
    }

    #[test]
    fn missing_swap_defaults_to_zero() {
        let content = "MemTotal:  8000000 kB\nMemAvailable:  5000000 kB\n";
        let snapshot = parse_meminfo(content).unwrap();
        assert_eq!(snapshot.swap_total_bytes, 0);
        assert_eq!(snapshot.swap_free_bytes, 0);
        assert_eq!(snapshot.swap_used_bytes, 0);
    }
}
