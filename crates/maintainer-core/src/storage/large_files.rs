use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::ffi::OsStr;
use std::path::PathBuf;

/// A file meeting the large-file threshold.
#[derive(Debug, Clone)]
pub struct LargeFile {
    /// Raw path preserving non-UTF-8 names.
    pub path: PathBuf,
    /// Apparent file size in bytes.
    pub size_bytes: u64,
}

impl LargeFile {
    /// Lossy display string for UI presentation.
    pub fn display_path(&self) -> String {
        self.path.to_string_lossy().into_owned()
    }

    /// The raw OS path.
    pub fn raw_path(&self) -> &OsStr {
        self.path.as_os_str()
    }
}

/// Wrapper for BinaryHeap that inverts ordering to create a min-heap by size.
#[derive(Debug, Clone)]
struct MinEntry {
    size_bytes: u64,
    path: PathBuf,
}

impl PartialEq for MinEntry {
    fn eq(&self, other: &Self) -> bool {
        self.size_bytes == other.size_bytes
    }
}

impl Eq for MinEntry {}

impl PartialOrd for MinEntry {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for MinEntry {
    fn cmp(&self, other: &Self) -> Ordering {
        other.size_bytes.cmp(&self.size_bytes)
    }
}

/// Bounded collection that retains only the top-N largest files by size.
///
/// Uses a min-heap so the smallest element is always accessible for eviction.
#[derive(Debug)]
pub struct LargeFileRanking {
    capacity: usize,
    heap: BinaryHeap<MinEntry>,
    threshold_bytes: u64,
}

impl LargeFileRanking {
    /// Create a new ranking that retains at most `capacity` files above
    /// `threshold_bytes`.
    pub fn new(capacity: usize, threshold_bytes: u64) -> Self {
        Self {
            capacity,
            heap: BinaryHeap::with_capacity(capacity + 1),
            threshold_bytes,
        }
    }

    /// Consider a file for inclusion. Returns `true` if it was accepted.
    pub fn offer(&mut self, path: PathBuf, size_bytes: u64) -> bool {
        if size_bytes < self.threshold_bytes {
            return false;
        }

        if self.heap.len() < self.capacity {
            self.heap.push(MinEntry { size_bytes, path });
            return true;
        }

        if let Some(min) = self.heap.peek() {
            if size_bytes > min.size_bytes {
                self.heap.pop();
                self.heap.push(MinEntry { size_bytes, path });
                return true;
            }
        }

        false
    }

    /// Drain the ranking into a sorted vector (largest first).
    pub fn into_sorted(self) -> Vec<LargeFile> {
        let mut entries: Vec<MinEntry> = self.heap.into_vec();
        entries.sort_by(|a, b| b.size_bytes.cmp(&a.size_bytes));
        entries
            .into_iter()
            .map(|e| LargeFile {
                path: e.path,
                size_bytes: e.size_bytes,
            })
            .collect()
    }

    /// Current count of tracked files.
    pub fn len(&self) -> usize {
        self.heap.len()
    }

    /// Returns `true` if no files are tracked.
    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_below_threshold() {
        let mut ranking = LargeFileRanking::new(10, 1000);
        assert!(!ranking.offer(PathBuf::from("/small"), 999));
        assert!(ranking.is_empty());
    }

    #[test]
    fn accepts_at_threshold() {
        let mut ranking = LargeFileRanking::new(10, 1000);
        assert!(ranking.offer(PathBuf::from("/exact"), 1000));
        assert_eq!(ranking.len(), 1);
    }

    #[test]
    fn bounded_evicts_smallest() {
        let mut ranking = LargeFileRanking::new(3, 100);
        ranking.offer(PathBuf::from("/a"), 200);
        ranking.offer(PathBuf::from("/b"), 300);
        ranking.offer(PathBuf::from("/c"), 400);
        assert!(ranking.offer(PathBuf::from("/d"), 500));
        assert_eq!(ranking.len(), 3);

        let result = ranking.into_sorted();
        assert_eq!(result.len(), 3);
        assert_eq!(result[0].size_bytes, 500);
        assert_eq!(result[1].size_bytes, 400);
        assert_eq!(result[2].size_bytes, 300);
    }

    #[test]
    fn does_not_evict_for_smaller() {
        let mut ranking = LargeFileRanking::new(2, 100);
        ranking.offer(PathBuf::from("/a"), 500);
        ranking.offer(PathBuf::from("/b"), 400);
        assert!(!ranking.offer(PathBuf::from("/c"), 300));
        assert_eq!(ranking.len(), 2);
    }

    #[test]
    fn into_sorted_returns_descending() {
        let mut ranking = LargeFileRanking::new(5, 0);
        ranking.offer(PathBuf::from("/1"), 10);
        ranking.offer(PathBuf::from("/2"), 50);
        ranking.offer(PathBuf::from("/3"), 30);

        let sorted = ranking.into_sorted();
        assert_eq!(sorted[0].size_bytes, 50);
        assert_eq!(sorted[1].size_bytes, 30);
        assert_eq!(sorted[2].size_bytes, 10);
    }

    #[test]
    fn large_file_display_path_lossy() {
        let lf = LargeFile {
            path: PathBuf::from("/home/user/bigfile.dat"),
            size_bytes: 1024,
        };
        assert_eq!(lf.display_path(), "/home/user/bigfile.dat");
    }
}
