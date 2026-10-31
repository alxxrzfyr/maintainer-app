//! Content-addressable hashing for duplicate confirmation.
//!
//! Uses BLAKE3 for fast, secure content hashing with two modes:
//! - Partial hash: reads a fixed prefix and suffix for quick discrimination.
//! - Full hash: reads the entire file for confirmation.

use std::fmt;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::error::MaintainerError;
use crate::jobs::Cancellation;
use crate::storage::mounts::DeviceInode;

/// Size of the prefix and suffix chunks read for partial hashing.
const PARTIAL_CHUNK_BYTES: u64 = 4096;

/// Buffer size for streaming full-file hashing.
const HASH_BUFFER_BYTES: usize = 64 * 1024;

/// A BLAKE3 content digest, used as the identity for duplicate groups.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct ContentDigest {
    bytes: [u8; 32],
}

impl ContentDigest {
    /// Create a digest from raw bytes.
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self { bytes }
    }

    /// Return the digest as a byte slice.
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.bytes
    }

    /// Return the digest as a lowercase hex string.
    pub fn to_hex(&self) -> String {
        use std::fmt::Write;
        self.bytes
            .iter()
            .fold(String::with_capacity(64), |mut s, b| {
                let _ = write!(s, "{b:02x}");
                s
            })
    }
}

impl fmt::Debug for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ContentDigest({})", &self.to_hex()[..16])
    }
}

impl fmt::Display for ContentDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// Compute a partial BLAKE3 hash of a file by reading a prefix and suffix chunk.
///
/// For files smaller than or equal to `PARTIAL_CHUNK_BYTES * 2`, the entire file
/// is read. This gives quick discrimination without reading the full file.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read.
pub fn partial_hash(
    path: &Path,
    file_size_bytes: u64,
    cancel: &dyn Cancellation,
) -> crate::error::Result<ContentDigest> {
    partial_hash_inner(path, file_size_bytes, None, cancel)
}

pub(crate) fn partial_hash_expected(
    path: &Path,
    file_size_bytes: u64,
    identity: DeviceInode,
    cancel: &dyn Cancellation,
) -> crate::error::Result<ContentDigest> {
    partial_hash_inner(path, file_size_bytes, Some(identity), cancel)
}

fn partial_hash_inner(
    path: &Path,
    file_size_bytes: u64,
    expected_identity: Option<DeviceInode>,
    cancel: &dyn Cancellation,
) -> crate::error::Result<ContentDigest> {
    if cancel.is_cancelled() {
        return Err(MaintainerError::Cancelled);
    }

    let context = || format!("partial hash of {}", path.display());
    let mut file = open_hash_file(path, expected_identity, Some(file_size_bytes))
        .map_err(|e| MaintainerError::io(context(), e))?;
    let before = HashSnapshot::capture(&file).map_err(|e| MaintainerError::io(context(), e))?;

    let mut hasher = blake3::Hasher::new();

    if file_size_bytes <= PARTIAL_CHUNK_BYTES * 2 {
        let mut buf = vec![0u8; file_size_bytes as usize];
        file.read_exact(&mut buf)
            .map_err(|e| MaintainerError::io(context(), e))?;
        hasher.update(&buf);
    } else {
        let mut prefix = vec![0u8; PARTIAL_CHUNK_BYTES as usize];
        file.read_exact(&mut prefix)
            .map_err(|e| MaintainerError::io(context(), e))?;
        hasher.update(&prefix);

        if cancel.is_cancelled() {
            return Err(MaintainerError::Cancelled);
        }

        let mut suffix = vec![0u8; PARTIAL_CHUNK_BYTES as usize];
        file.seek(SeekFrom::End(-(PARTIAL_CHUNK_BYTES as i64)))
            .map_err(|e| MaintainerError::io(context(), e))?;
        file.read_exact(&mut suffix)
            .map_err(|e| MaintainerError::io(context(), e))?;
        hasher.update(&suffix);
    }

    ensure_hash_file_unchanged(&file, &before)?;
    let hash = hasher.finalize();
    Ok(ContentDigest::from_bytes(*hash.as_bytes()))
}

/// Compute the full BLAKE3 hash of a file, streaming in fixed-size buffers.
///
/// Checks cancellation between buffer reads. Returns the total bytes read
/// alongside the digest for progress accounting.
///
/// # Errors
///
/// Returns an error if the file cannot be opened or read, or if cancelled.
pub fn full_hash(
    path: &Path,
    cancel: &dyn Cancellation,
) -> crate::error::Result<(ContentDigest, u64)> {
    full_hash_inner(path, None, None, cancel)
}

pub(crate) fn full_hash_expected(
    path: &Path,
    identity: DeviceInode,
    size_bytes: u64,
    cancel: &dyn Cancellation,
) -> crate::error::Result<(ContentDigest, u64)> {
    full_hash_inner(path, Some(identity), Some(size_bytes), cancel)
}

fn full_hash_inner(
    path: &Path,
    expected_identity: Option<DeviceInode>,
    expected_size: Option<u64>,
    cancel: &dyn Cancellation,
) -> crate::error::Result<(ContentDigest, u64)> {
    if cancel.is_cancelled() {
        return Err(MaintainerError::Cancelled);
    }

    let context = || format!("full hash of {}", path.display());
    let mut file = open_hash_file(path, expected_identity, expected_size)
        .map_err(|e| MaintainerError::io(context(), e))?;
    let before = HashSnapshot::capture(&file).map_err(|e| MaintainerError::io(context(), e))?;

    let mut hasher = blake3::Hasher::new();
    let mut buf = vec![0u8; HASH_BUFFER_BYTES];
    let mut total_bytes: u64 = 0;

    loop {
        if cancel.is_cancelled() {
            return Err(MaintainerError::Cancelled);
        }

        let n = file
            .read(&mut buf)
            .map_err(|e| MaintainerError::io(context(), e))?;
        if n == 0 {
            break;
        }
        total_bytes += n as u64;
        if total_bytes > before.size {
            return Err(MaintainerError::invalid_data(
                "duplicate scan",
                "file grew while hashing",
            ));
        }
        hasher.update(&buf[..n]);
    }

    ensure_hash_file_unchanged(&file, &before)?;
    let hash = hasher.finalize();
    Ok((ContentDigest::from_bytes(*hash.as_bytes()), total_bytes))
}

#[derive(Debug, PartialEq, Eq)]
struct HashSnapshot {
    device: u64,
    inode: u64,
    size: u64,
    modified_secs: i64,
    modified_nsecs: i64,
    changed_secs: i64,
    changed_nsecs: i64,
}

impl HashSnapshot {
    fn capture(file: &File) -> std::io::Result<Self> {
        let metadata = file.metadata()?;
        Ok(Self {
            device: metadata.dev(),
            inode: metadata.ino(),
            size: metadata.size(),
            modified_secs: metadata.mtime(),
            modified_nsecs: metadata.mtime_nsec(),
            changed_secs: metadata.ctime(),
            changed_nsecs: metadata.ctime_nsec(),
        })
    }
}

fn open_hash_file(
    path: &Path,
    expected_identity: Option<DeviceInode>,
    expected_size: Option<u64>,
) -> std::io::Result<File> {
    let file = crate::storage::policy::open_anchored(path, false)?;
    let metadata = file.metadata()?;
    if !metadata.file_type().is_file()
        || expected_size.is_some_and(|size| metadata.len() != size)
        || expected_identity.is_some_and(|identity| {
            metadata.dev() != identity.device || metadata.ino() != identity.inode
        })
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "file identity or size changed before hashing",
        ));
    }
    Ok(file)
}

fn ensure_hash_file_unchanged(file: &File, before: &HashSnapshot) -> crate::error::Result<()> {
    let after = HashSnapshot::capture(file)
        .map_err(|error| MaintainerError::io("checking hashed file", error))?;
    if &after != before {
        return Err(MaintainerError::invalid_data(
            "duplicate scan",
            "file changed while hashing",
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::CancellationToken;
    use tempfile::TempDir;

    fn no_cancel() -> CancellationToken {
        CancellationToken::new()
    }

    #[test]
    fn content_digest_hex_is_64_chars() {
        let digest = ContentDigest::from_bytes([0xab; 32]);
        assert_eq!(digest.to_hex().len(), 64);
    }

    #[test]
    fn content_digest_equality() {
        let a = ContentDigest::from_bytes([1; 32]);
        let b = ContentDigest::from_bytes([1; 32]);
        let c = ContentDigest::from_bytes([2; 32]);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn partial_hash_small_file_reads_all() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("small.bin");
        std::fs::write(&path, b"hello world").unwrap();

        let cancel = no_cancel();
        let digest = partial_hash(&path, 11, &cancel).unwrap();

        let path2 = tmp.path().join("small2.bin");
        std::fs::write(&path2, b"hello world").unwrap();
        let digest2 = partial_hash(&path2, 11, &cancel).unwrap();
        assert_eq!(digest, digest2);
    }

    #[test]
    fn partial_hash_different_content_differs() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.bin");
        let p2 = tmp.path().join("b.bin");
        std::fs::write(&p1, b"aaaa").unwrap();
        std::fs::write(&p2, b"bbbb").unwrap();

        let cancel = no_cancel();
        let d1 = partial_hash(&p1, 4, &cancel).unwrap();
        let d2 = partial_hash(&p2, 4, &cancel).unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn partial_hash_large_file_reads_prefix_and_suffix() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("large.bin");

        let size = (PARTIAL_CHUNK_BYTES * 3) as usize;
        let mut content = vec![0u8; size];
        content[0] = 0xAA;
        content[size - 1] = 0xBB;
        std::fs::write(&path, &content).unwrap();

        let cancel = no_cancel();
        let digest = partial_hash(&path, size as u64, &cancel).unwrap();

        let mut content2 = content.clone();
        content2[size - 1] = 0xCC;
        let path2 = tmp.path().join("large2.bin");
        std::fs::write(&path2, &content2).unwrap();
        let digest2 = partial_hash(&path2, size as u64, &cancel).unwrap();
        assert_ne!(digest, digest2);
    }

    #[test]
    fn partial_hash_cancellation() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.bin");
        std::fs::write(&path, b"data").unwrap();

        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = partial_hash(&path, 4, &cancel);
        assert!(matches!(result, Err(MaintainerError::Cancelled)));
    }

    #[test]
    fn full_hash_identical_content() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.bin");
        let p2 = tmp.path().join("b.bin");
        let content = b"identical content for hashing test";
        std::fs::write(&p1, content).unwrap();
        std::fs::write(&p2, content).unwrap();

        let cancel = no_cancel();
        let (d1, bytes1) = full_hash(&p1, &cancel).unwrap();
        let (d2, bytes2) = full_hash(&p2, &cancel).unwrap();
        assert_eq!(d1, d2);
        assert_eq!(bytes1, content.len() as u64);
        assert_eq!(bytes2, content.len() as u64);
    }

    #[test]
    fn full_hash_different_content() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.bin");
        let p2 = tmp.path().join("b.bin");
        std::fs::write(&p1, b"content A").unwrap();
        std::fs::write(&p2, b"content B").unwrap();

        let cancel = no_cancel();
        let (d1, _) = full_hash(&p1, &cancel).unwrap();
        let (d2, _) = full_hash(&p2, &cancel).unwrap();
        assert_ne!(d1, d2);
    }

    #[test]
    fn full_hash_cancellation() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("file.bin");
        let data = vec![0u8; HASH_BUFFER_BYTES * 3];
        std::fs::write(&path, &data).unwrap();

        let cancel = CancellationToken::new();
        cancel.cancel();
        let result = full_hash(&path, &cancel);
        assert!(matches!(result, Err(MaintainerError::Cancelled)));
    }

    #[test]
    fn full_hash_nonexistent_file_returns_error() {
        let cancel = no_cancel();
        let result = full_hash(Path::new("/nonexistent_xyz_12345"), &cancel);
        assert!(result.is_err());
        assert!(matches!(result, Err(MaintainerError::Io { .. })));
    }

    #[test]
    fn full_hash_reports_bytes_read() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("measured.bin");
        let size = 12345usize;
        let data = vec![0x42u8; size];
        std::fs::write(&path, &data).unwrap();

        let cancel = no_cancel();
        let (_, bytes) = full_hash(&path, &cancel).unwrap();
        assert_eq!(bytes, size as u64);
    }

    #[test]
    fn partial_and_full_same_for_small_identical_files() {
        let tmp = TempDir::new().unwrap();
        let p1 = tmp.path().join("a.txt");
        let p2 = tmp.path().join("b.txt");
        let content = b"small identical";
        std::fs::write(&p1, content).unwrap();
        std::fs::write(&p2, content).unwrap();

        let cancel = no_cancel();
        let partial1 = partial_hash(&p1, content.len() as u64, &cancel).unwrap();
        let partial2 = partial_hash(&p2, content.len() as u64, &cancel).unwrap();
        assert_eq!(partial1, partial2);

        let (full1, _) = full_hash(&p1, &cancel).unwrap();
        let (full2, _) = full_hash(&p2, &cancel).unwrap();
        assert_eq!(full1, full2);
    }

    #[test]
    fn hash_rejects_fifo_without_waiting_for_a_writer() {
        use std::os::unix::ffi::OsStrExt;
        let temporary = TempDir::new().unwrap();
        let path = temporary.path().join("fifo");
        let name = std::ffi::CString::new(path.as_os_str().as_bytes()).unwrap();
        // SAFETY: name is a valid NUL-terminated path in this test's private directory.
        assert_eq!(unsafe { libc::mkfifo(name.as_ptr(), 0o600) }, 0);
        assert!(full_hash(&path, &no_cancel()).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn candidate_hash_rejects_symlink_and_replaced_identity() {
        use std::os::unix::fs as unix_fs;

        let tmp = TempDir::new().unwrap();
        let target = tmp.path().join("target");
        let link = tmp.path().join("link");
        std::fs::write(&target, b"same").unwrap();
        unix_fs::symlink(&target, &link).unwrap();
        let metadata = std::fs::symlink_metadata(&target).unwrap();
        let identity = DeviceInode::from_metadata(&metadata);
        let cancel = no_cancel();

        assert!(partial_hash_expected(&link, 4, identity, &cancel).is_err());
        assert!(
            partial_hash_expected(
                &target,
                4,
                DeviceInode {
                    device: 0,
                    inode: 0
                },
                &cancel
            )
            .is_err()
        );
    }
}
