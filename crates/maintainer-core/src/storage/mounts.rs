use std::io;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// Identifies a file's device and inode, used to detect hard links and
/// filesystem boundary crossings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceInode {
    pub device: u64,
    pub inode: u64,
}

impl DeviceInode {
    /// Extract device and inode from metadata. Uses lstat semantics (does not
    /// follow symlinks).
    pub fn from_metadata(meta: &std::fs::Metadata) -> Self {
        Self {
            device: meta.dev(),
            inode: meta.ino(),
        }
    }

    /// Get the device and inode for a path without following symlinks.
    pub fn for_path(path: &Path) -> io::Result<Self> {
        let meta = path.symlink_metadata()?;
        Ok(Self::from_metadata(&meta))
    }
}

/// Returns `true` if `child_device` is on a different filesystem than
/// `root_device`.
pub fn crosses_filesystem(root_device: u64, child_device: u64) -> bool {
    root_device != child_device
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn device_inode_same_file_matches() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("test.txt");
        std::fs::write(&file_path, b"content").unwrap();

        let di1 = DeviceInode::for_path(&file_path).unwrap();
        let di2 = DeviceInode::for_path(&file_path).unwrap();
        assert_eq!(di1, di2);
    }

    #[test]
    fn device_inode_different_files_differ_in_inode() {
        let tmp = TempDir::new().unwrap();
        let f1 = tmp.path().join("a.txt");
        let f2 = tmp.path().join("b.txt");
        std::fs::write(&f1, b"aaa").unwrap();
        std::fs::write(&f2, b"bbb").unwrap();

        let di1 = DeviceInode::for_path(&f1).unwrap();
        let di2 = DeviceInode::for_path(&f2).unwrap();
        assert_eq!(di1.device, di2.device);
        assert_ne!(di1.inode, di2.inode);
    }

    #[test]
    fn crosses_filesystem_detects_boundary() {
        assert!(crosses_filesystem(1, 2));
        assert!(!crosses_filesystem(5, 5));
    }

    #[test]
    fn for_path_nonexistent_returns_error() {
        let result = DeviceInode::for_path(Path::new("/nonexistent_path_xyz_12345"));
        assert!(result.is_err());
    }
}
