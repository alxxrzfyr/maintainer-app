//! Descriptor-bound regular-file removal. The quarantine directory must be
//! inaccessible to the caller; caller-writable staging cannot protect unlink.
use std::ffi::{CStr, CString, OsStr};
use std::fs::{File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::Path;

const LIMIT: usize = 128;
const MAX_RECEIPT: u64 = 16_384;

/// Root-private, durable staging area. Never automatically purges staged data.
pub struct Quarantine {
    directory: File,
}

/// A failed restoration retains data under this opaque receipt identifier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Removal {
    Deleted,
    Trashed,
    /// The namespace mutation committed, but its durability could not be confirmed.
    DeletedDurabilityUnknown,
    TrashedDurabilityUnknown,
    Restored,
    RestoredDurabilityUnknown,
    RecoveryRequired(String),
}

/// Restoration has committed once the no-overwrite rename succeeds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restoration {
    Restored,
    RestoredDurabilityUnknown,
}

/// Private receipt visible only to the owning caller after authorization.
#[derive(Debug, Clone)]
pub struct Recovery {
    pub id: String,
    pub basename: String,
    pub parent_path: String,
}

struct Receipt {
    uid: u32,
    parent_device: u64,
    parent_inode: u64,
    basename: String,
    parent_path: String,
}

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, message)
}

pub fn component(name: &str) -> io::Result<CString> {
    if name.is_empty() || name.len() > 255 || matches!(name, "." | "..") || name.contains('/') {
        return Err(invalid("expected one bounded filename component"));
    }
    CString::new(name).map_err(|_| invalid("filename contains NUL"))
}

fn open_at(directory: BorrowedFd<'_>, name: &CStr, flags: i32) -> io::Result<File> {
    // SAFETY: name is NUL terminated and the borrowed descriptor is live.
    let fd = unsafe {
        libc::openat(
            directory.as_raw_fd(),
            name.as_ptr(),
            flags | libc::O_CLOEXEC | libc::O_NOFOLLOW,
            0o600,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: openat returned a new owned descriptor.
    Ok(unsafe { File::from_raw_fd(fd) })
}

#[cfg(test)]
thread_local! {
    static MOVE_ERROR: std::cell::Cell<Option<i32>> = const { std::cell::Cell::new(None) };
    static COMMIT_SYNC_ERROR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    static FINISH_ERROR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

fn commit_sync(directory: &File) -> io::Result<()> {
    #[cfg(test)]
    if COMMIT_SYNC_ERROR.with(|fault| fault.replace(false)) {
        return Err(io::Error::from_raw_os_error(libc::EIO));
    }
    directory.sync_all()
}

fn rename(
    source: BorrowedFd<'_>,
    name: &CStr,
    destination: BorrowedFd<'_>,
    target: &CStr,
) -> io::Result<()> {
    #[cfg(test)]
    if let Some(errno) = MOVE_ERROR.with(|error| error.take()) {
        return Err(io::Error::from_raw_os_error(errno));
    }
    // SAFETY: both borrowed directory descriptors and C strings live through the syscall.
    let rc = unsafe {
        libc::renameat2(
            source.as_raw_fd(),
            name.as_ptr(),
            destination.as_raw_fd(),
            target.as_ptr(),
            libc::RENAME_NOREPLACE,
        )
    };
    if rc < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn unlink(directory: BorrowedFd<'_>, name: &CStr, flags: i32) -> io::Result<()> {
    // SAFETY: bounded name is interpreted relative to a retained directory.
    if unsafe { libc::unlinkat(directory.as_raw_fd(), name.as_ptr(), flags) } < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

impl Quarantine {
    /// Only the fixed root-owned state directory is accepted in production.
    pub fn system() -> io::Result<Self> {
        let path = Path::new("/var/lib/maintainer");
        let parent = OpenOptions::new()
            .read(true)
            .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(path)?;
        Self::validate_private(&parent, 0)?;
        let name = component("quarantine")?;
        // SAFETY: parent is anchored and name is a fixed component.
        if unsafe { libc::mkdirat(parent.as_raw_fd(), name.as_ptr(), 0o700) } < 0 {
            let error = io::Error::last_os_error();
            if error.kind() != io::ErrorKind::AlreadyExists {
                return Err(error);
            }
        }
        let directory = open_at(parent.as_fd(), &name, libc::O_RDONLY | libc::O_DIRECTORY)?;
        Self::validate_private(&directory, 0)?;
        Ok(Self { directory })
    }

    fn validate_private(directory: &File, owner: u32) -> io::Result<()> {
        let metadata = directory.metadata()?;
        if !metadata.is_dir()
            || metadata.uid() != owner
            || metadata.permissions().mode() & 0o077 != 0
        {
            return Err(invalid(
                "quarantine must be owned by the helper with mode 0700",
            ));
        }
        Ok(())
    }

    fn transaction(&self, id: &str) -> io::Result<File> {
        if id.len() != 32 || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(invalid("invalid recovery identifier"));
        }
        let directory = open_at(
            self.directory.as_fd(),
            &component(id)?,
            libc::O_RDONLY | libc::O_DIRECTORY,
        )?;
        Self::validate_private(&directory, self.directory.metadata()?.uid())?;
        Ok(directory)
    }

    fn receipt(directory: &File) -> io::Result<Receipt> {
        let mut text = String::new();
        open_at(directory.as_fd(), &component("receipt")?, libc::O_RDONLY)?
            .take(MAX_RECEIPT + 1)
            .read_to_string(&mut text)?;
        if text.len() as u64 > MAX_RECEIPT {
            return Err(invalid("oversized recovery receipt"));
        }
        let mut lines = text.lines();
        let mut next = || lines.next().ok_or_else(|| invalid("incomplete receipt"));
        let uid = next()?
            .parse()
            .map_err(|_| invalid("invalid receipt UID"))?;
        let parent_device = next()?
            .parse()
            .map_err(|_| invalid("invalid receipt device"))?;
        let parent_inode = next()?
            .parse()
            .map_err(|_| invalid("invalid receipt inode"))?;
        let basename = decode(next()?)?;
        component(&basename)?;
        let parent_path = decode(next()?)?;
        Ok(Receipt {
            uid,
            parent_device,
            parent_inode,
            basename,
            parent_path,
        })
    }

    /// Returns at most 128 outstanding transactions. Unknown entries consume
    /// capacity rather than being deleted or interpreted as approved data.
    pub fn list(&self, uid: u32) -> io::Result<Vec<Recovery>> {
        let mut recoveries = Vec::new();
        for entry in std::fs::read_dir(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))?
            .take(LIMIT + 1)
        {
            let entry = entry?;
            let id = entry.file_name().to_string_lossy().into_owned();
            let Ok(transaction) = self.transaction(&id) else {
                continue;
            };
            let Ok(receipt) = Self::receipt(&transaction) else {
                continue;
            };
            if receipt.uid == uid
                && open_at(transaction.as_fd(), &component("entry")?, libc::O_PATH).is_ok()
            {
                recoveries.push(Recovery {
                    id,
                    basename: receipt.basename,
                    parent_path: receipt.parent_path,
                });
            }
        }
        Ok(recoveries)
    }

    fn finish(&self, transaction: &File, id: &str) -> io::Result<()> {
        #[cfg(test)]
        if FINISH_ERROR.with(|fault| fault.replace(false)) {
            return Err(io::Error::from_raw_os_error(libc::EIO));
        }
        unlink(transaction.as_fd(), &component("receipt")?, 0)?;
        transaction.sync_all()?;
        unlink(self.directory.as_fd(), &component(id)?, libc::AT_REMOVEDIR)?;
        self.directory.sync_all()
    }

    /// Recovery never overwrites a destination, follows a pathname ancestor,
    /// or deletes the retained entry. The caller must supply the original parent.
    pub fn restore(&self, uid: u32, id: &str, parent: &File) -> io::Result<Restoration> {
        self.restore_with_hook(uid, id, parent, || {})
    }

    fn restore_with_hook(
        &self,
        uid: u32,
        id: &str,
        parent: &File,
        after_commit: impl FnOnce(),
    ) -> io::Result<Restoration> {
        self.restore_inner(uid, id, parent, true, after_commit)
    }

    /// Explicit user-selected recovery destination. The original basename and
    /// caller-owned same-filesystem directory remain mandatory.
    pub fn restore_elsewhere(&self, uid: u32, id: &str, parent: &File) -> io::Result<Restoration> {
        self.restore_inner(uid, id, parent, false, || {})
    }

    fn restore_inner(
        &self,
        uid: u32,
        id: &str,
        parent: &File,
        original_parent: bool,
        after_commit: impl FnOnce(),
    ) -> io::Result<Restoration> {
        let transaction = self.transaction(id)?;
        let receipt = Self::receipt(&transaction)?;
        let metadata = parent.metadata()?;
        if receipt.uid != uid
            || metadata.uid() != uid
            || !metadata.is_dir()
            || metadata.dev() != receipt.parent_device
            || (original_parent && metadata.ino() != receipt.parent_inode)
        {
            return Err(invalid("recovery parent or caller identity changed"));
        }
        rename(
            transaction.as_fd(),
            &component("entry")?,
            parent.as_fd(),
            &component(&receipt.basename)?,
        )?;
        after_commit();
        let synced = commit_sync(parent).is_ok();
        let finished = self.finish(&transaction, id).is_ok();
        Ok(if synced && finished {
            Restoration::Restored
        } else {
            Restoration::RestoredDurabilityUnknown
        })
    }

    /// Pinning the reviewed descriptor prevents inode reuse. Only the entry
    /// atomically moved into the protected directory can subsequently be unlinked.
    pub fn remove(
        &self,
        uid: u32,
        parent: &File,
        reviewed: &File,
        basename: &str,
        cancelled: impl Fn() -> bool,
    ) -> io::Result<Removal> {
        self.remove_with_hook(uid, parent, reviewed, basename, cancelled, || {}, None)
    }

    /// Trash uses caller-owned destination descriptors. No external destination
    /// name is ever overwritten or later unlinked by the privileged helper.
    #[allow(
        clippy::too_many_arguments,
        reason = "The fixed operation carries distinct source, reviewed file and Trash directory capabilities"
    )]
    pub fn trash(
        &self,
        uid: u32,
        parent: &File,
        reviewed: &File,
        basename: &str,
        files: &File,
        info: &File,
    ) -> io::Result<Removal> {
        self.remove_with_hook(
            uid,
            parent,
            reviewed,
            basename,
            || false,
            || {},
            Some((files, info)),
        )
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "The deterministic race hook adds one test control to the descriptor operation"
    )]
    fn remove_with_hook(
        &self,
        uid: u32,
        parent: &File,
        reviewed: &File,
        basename: &str,
        cancelled: impl Fn() -> bool,
        after_move: impl FnOnce(),
        trash: Option<(&File, &File)>,
    ) -> io::Result<Removal> {
        let name = component(basename)?;
        let parent_metadata = parent.metadata()?;
        let reviewed_metadata = reviewed.metadata()?;
        if parent_metadata.uid() != uid
            || !parent_metadata.is_dir()
            || reviewed_metadata.uid() != uid
            || !reviewed_metadata.is_file()
            || reviewed_metadata.nlink() != 1
        {
            return Err(invalid(
                "only caller-owned single-link regular files and parents are supported",
            ));
        }
        if parent_metadata.dev() != self.directory.metadata()?.dev()
            || reviewed_metadata.dev() != parent_metadata.dev()
        {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "quarantine is on another filesystem; no copy or deletion fallback",
            ));
        }
        let mut filesystem = std::mem::MaybeUninit::<libc::statfs>::uninit();
        // SAFETY: fstatfs initializes the buffer on success.
        if unsafe { libc::fstatfs(parent.as_raw_fd(), filesystem.as_mut_ptr()) } != 0 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: fstatfs returned success.
        let magic = unsafe { filesystem.assume_init() }.f_type;
        if ![0xef53, 0x58465342, 0x9123683e, 0x01021994].contains(&magic) {
            return Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "filesystem does not support the reviewed cleanup contract",
            ));
        }
        if let Some((files, info)) = trash {
            for directory in [files, info] {
                let metadata = directory.metadata()?;
                if !metadata.is_dir()
                    || metadata.uid() != uid
                    || metadata.dev() != parent_metadata.dev()
                {
                    return Err(invalid(
                        "Trash directories must be caller-owned and on the same filesystem",
                    ));
                }
            }
        }
        if cancelled() {
            return Ok(Removal::Restored);
        }
        if std::fs::read_dir(format!("/proc/self/fd/{}", self.directory.as_raw_fd()))?
            .take(LIMIT)
            .count()
            >= LIMIT
        {
            return Err(io::Error::other(
                "recovery capacity reached; restore outstanding entries first",
            ));
        }
        let id = uuid::Uuid::new_v4().simple().to_string();
        let id_name = component(&id)?;
        // SAFETY: the new private directory name is generated and parent is anchored.
        if unsafe { libc::mkdirat(self.directory.as_raw_fd(), id_name.as_ptr(), 0o700) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let transaction = self.transaction(&id)?;
        let parent_path = std::fs::read_link(format!("/proc/self/fd/{}", parent.as_raw_fd()))?;
        let receipt = format!(
            "{}\n{}\n{}\n{}\n{}\nreviewed:{}:{}\n",
            uid,
            parent_metadata.dev(),
            parent_metadata.ino(),
            encode(OsStr::new(basename).as_bytes()),
            encode(parent_path.as_os_str().as_bytes()),
            reviewed_metadata.dev(),
            reviewed_metadata.ino()
        );
        let mut file = open_at(
            transaction.as_fd(),
            &component("receipt")?,
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        file.write_all(receipt.as_bytes())?;
        file.sync_all()?;
        transaction.sync_all()?;
        self.directory.sync_all()?;
        if cancelled() {
            self.finish(&transaction, &id)?;
            return Ok(Removal::Restored);
        }
        if let Err(error) = rename(
            parent.as_fd(),
            &name,
            transaction.as_fd(),
            &component("entry")?,
        ) {
            self.finish(&transaction, &id)?;
            return Err(error);
        }
        // The receipt was durable before the move. A crash from here leaves a
        // recoverable entry and never causes an automatic cleanup on restart.
        after_move();
        let inspected = (|| {
            parent.sync_all()?;
            transaction.sync_all()?;
            open_at(transaction.as_fd(), &component("entry")?, libc::O_PATH)?.metadata()
        })();
        let moved = match inspected {
            Ok(metadata) => metadata,
            Err(_) => return Ok(Removal::RecoveryRequired(id)),
        };
        if writeln!(
            file,
            "moved:{}:{}:{}:{}",
            moved.dev(),
            moved.ino(),
            moved.uid(),
            moved.mode()
        )
        .and_then(|()| file.sync_all())
        .is_err()
        {
            return Ok(Removal::RecoveryRequired(id));
        }
        let matches = moved.is_file()
            && moved.dev() == reviewed_metadata.dev()
            && moved.ino() == reviewed_metadata.ino()
            && moved.uid() == uid
            && moved.nlink() == 1;
        if !matches || cancelled() {
            return match self.restore(uid, &id, parent) {
                Ok(Restoration::Restored) => Ok(Removal::Restored),
                Ok(Restoration::RestoredDurabilityUnknown) => {
                    Ok(Removal::RestoredDurabilityUnknown)
                }
                Err(_) => Ok(Removal::RecoveryRequired(id)),
            };
        }
        if let Some((files, info)) = trash {
            let moved = (|| -> io::Result<()> {
                let info_name = component(&format!("{id}.trashinfo"))?;
                let mut sidecar = open_at(
                    info.as_fd(),
                    &info_name,
                    libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
                )?;
                let path = parent_path.join(basename);
                let encoded_path = path
                    .as_os_str()
                    .as_bytes()
                    .iter()
                    .map(|byte| {
                        if byte.is_ascii_alphanumeric() || b"/-._~".contains(byte) {
                            char::from(*byte).to_string()
                        } else {
                            format!("%{byte:02X}")
                        }
                    })
                    .collect::<String>();
                sidecar.write_all(
                    format!(
                        "[Trash Info]\nPath={encoded_path}\nDeletionDate={}\n",
                        deletion_date()?
                    )
                    .as_bytes(),
                )?;
                // SAFETY: this descriptor refers only to the exclusively created sidecar.
                if unsafe { libc::fchown(sidecar.as_raw_fd(), uid, !0) } != 0 {
                    return Err(io::Error::last_os_error());
                }
                sidecar.sync_all()?;
                info.sync_all()?;
                rename(
                    transaction.as_fd(),
                    &component("entry")?,
                    files.as_fd(),
                    &component(&id)?,
                )?;
                Ok(())
            })();
            if moved.is_err() {
                return match self.restore(uid, &id, parent) {
                    Ok(Restoration::Restored) => Ok(Removal::Restored),
                    Ok(Restoration::RestoredDurabilityUnknown) => {
                        Ok(Removal::RestoredDurabilityUnknown)
                    }
                    Err(_) => Ok(Removal::RecoveryRequired(id)),
                };
            }
            // Publication committed. No subsequent failure may trigger source
            // restoration: the retained entry is now in the caller's Trash.
            let synced = commit_sync(files).is_ok();
            let finished = self.finish(&transaction, &id).is_ok();
            return Ok(if synced && finished {
                Removal::Trashed
            } else {
                Removal::TrashedDurabilityUnknown
            });
        }
        if unlink(transaction.as_fd(), &component("entry")?, 0).is_err() {
            return Ok(Removal::RecoveryRequired(id));
        }
        let synced = commit_sync(&transaction).is_ok();
        let finished = self.finish(&transaction, &id).is_ok();
        if !synced || !finished {
            return Ok(Removal::DeletedDurabilityUnknown);
        }
        // Open writers or other retained descriptors can delay physical space
        // reclamation; this result deliberately makes no recovered-byte claim.
        Ok(Removal::Deleted)
    }
}

fn deletion_date() -> io::Result<String> {
    let mut now: libc::time_t = 0;
    let mut calendar = std::mem::MaybeUninit::<libc::tm>::uninit();
    // SAFETY: time and localtime_r receive valid writable pointers.
    unsafe {
        libc::time(&mut now);
    }
    if unsafe { libc::localtime_r(&now, calendar.as_mut_ptr()) }.is_null() {
        return Err(io::Error::other("cannot determine Trash timestamp"));
    }
    // SAFETY: localtime_r initialized calendar and strftime receives a bounded buffer.
    let mut buffer = [0u8; 32];
    let count = unsafe {
        libc::strftime(
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            c"%Y-%m-%dT%H:%M:%S".as_ptr(),
            calendar.as_ptr(),
        )
    };
    if count == 0 {
        return Err(io::Error::other("cannot format Trash timestamp"));
    }
    String::from_utf8(buffer[..count].to_vec()).map_err(io::Error::other)
}

fn encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    bytes
        .iter()
        .flat_map(|byte| {
            [
                char::from(HEX[(byte >> 4) as usize]),
                char::from(HEX[(byte & 15) as usize]),
            ]
        })
        .collect()
}
fn decode(text: &str) -> io::Result<String> {
    if !text.is_ascii() || text.len() % 2 != 0 || text.len() > 8192 {
        return Err(invalid("invalid receipt text"));
    }
    let bytes: Result<Vec<_>, _> = (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16))
        .collect();
    String::from_utf8(bytes.map_err(|_| invalid("invalid receipt encoding"))?)
        .map_err(|_| invalid("unsupported non UTF-8 filename"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        root: tempfile::TempDir,
        source: File,
        store: Quarantine,
        uid: u32,
    }
    impl Fixture {
        fn new() -> Self {
            // /dev/shm is tmpfs, one of the explicitly supported filesystems.
            let root = tempfile::tempdir_in("/dev/shm").unwrap();
            let source_path = root.path().join("source");
            let quarantine_path = root.path().join("quarantine");
            std::fs::create_dir(&source_path).unwrap();
            std::fs::create_dir(&quarantine_path).unwrap();
            std::fs::set_permissions(&quarantine_path, std::fs::Permissions::from_mode(0o700))
                .unwrap();
            let source = File::open(source_path).unwrap();
            let directory = File::open(quarantine_path).unwrap();
            let uid = directory.metadata().unwrap().uid();
            Quarantine::validate_private(&directory, uid).unwrap();
            Self {
                root,
                source,
                store: Quarantine { directory },
                uid,
            }
        }
        fn path(&self, name: &str) -> std::path::PathBuf {
            self.root.path().join("source").join(name)
        }
        fn pin(&self) -> File {
            std::fs::write(self.path("reviewed"), b"reviewed data").unwrap();
            File::open(self.path("reviewed")).unwrap()
        }
        fn remove(&self, pin: &File) -> Removal {
            self.store
                .remove(self.uid, &self.source, pin, "reviewed", || false)
                .unwrap()
        }
    }

    struct MockAuthorizedFiles {
        store: Quarantine,
        uid: u32,
    }
    #[zbus::interface(name = "io.github.maintainerapp.TestFiles")]
    impl MockAuthorizedFiles {
        fn remove(
            &self,
            parent: zbus::zvariant::OwnedFd,
            reviewed: zbus::zvariant::OwnedFd,
            name: &str,
        ) -> zbus::fdo::Result<bool> {
            let result = self
                .store
                .remove(
                    self.uid,
                    &File::from(std::os::fd::OwnedFd::from(parent)),
                    &File::from(std::os::fd::OwnedFd::from(reviewed)),
                    name,
                    || false,
                )
                .map_err(|error| zbus::fdo::Error::Failed(error.to_string()))?;
            Ok(result == Removal::Deleted)
        }
    }

    #[test]
    fn pinned_file_descriptors_survive_private_dbus_transport() {
        use std::io::BufRead;
        struct PrivateBus(std::process::Child);
        impl Drop for PrivateBus {
            fn drop(&mut self) {
                let _kill = self.0.kill();
                let _wait = self.0.wait();
            }
        }
        let f = Fixture::new();
        let pin = f.pin();
        let mut bus = PrivateBus(
            std::process::Command::new("dbus-daemon")
                .args(["--session", "--nofork", "--print-address=1"])
                .stdout(std::process::Stdio::piped())
                .spawn()
                .unwrap(),
        );
        let mut address = String::new();
        std::io::BufReader::new(bus.0.stdout.take().unwrap())
            .read_line(&mut address)
            .unwrap();
        let service = MockAuthorizedFiles {
            store: Quarantine {
                directory: f.store.directory.try_clone().unwrap(),
            },
            uid: f.uid,
        };
        let server = zbus::blocking::connection::Builder::address(address.trim())
            .unwrap()
            .name("io.github.maintainerapp.TestFiles")
            .unwrap()
            .serve_at("/io/github/maintainerapp/TestFiles", service)
            .unwrap()
            .build()
            .unwrap();
        let client = zbus::blocking::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .unwrap();
        let reply = client
            .call_method(
                Some("io.github.maintainerapp.TestFiles"),
                "/io/github/maintainerapp/TestFiles",
                Some("io.github.maintainerapp.TestFiles"),
                "Remove",
                &(
                    zbus::zvariant::Fd::from(f.source.as_fd()),
                    zbus::zvariant::Fd::from(pin.as_fd()),
                    "reviewed",
                ),
            )
            .unwrap();
        assert!(reply.body().deserialize::<bool>().unwrap());
        assert!(!f.path("reviewed").exists());
        drop(client);
        drop(server);
    }

    #[test]
    fn only_reviewed_regular_file_is_deleted() {
        let f = Fixture::new();
        let pin = f.pin();
        assert_eq!(f.remove(&pin), Removal::Deleted);
        assert!(!f.path("reviewed").exists());
        let mut data = String::new();
        (&pin).read_to_string(&mut data).unwrap();
        assert_eq!(data, "reviewed data"); // pinned FD prevents reuse and defers freeing
        assert!(f.store.list(f.uid).unwrap().is_empty());
    }

    #[test]
    fn swapped_leaf_restores_unreviewed_entry_without_deleting_it() {
        let f = Fixture::new();
        let pin = f.pin();
        std::fs::rename(f.path("reviewed"), f.path("original")).unwrap();
        std::fs::write(f.path("reviewed"), b"unreviewed replacement").unwrap();
        assert_eq!(f.remove(&pin), Removal::Restored);
        assert_eq!(
            std::fs::read(f.path("reviewed")).unwrap(),
            b"unreviewed replacement"
        );
        assert_eq!(std::fs::read(f.path("original")).unwrap(), b"reviewed data");
    }

    #[test]
    fn swapped_symlink_is_restored_and_target_is_never_followed() {
        let f = Fixture::new();
        let pin = f.pin();
        std::fs::rename(f.path("reviewed"), f.path("original")).unwrap();
        std::fs::write(f.path("victim"), b"outside").unwrap();
        std::os::unix::fs::symlink(f.path("victim"), f.path("reviewed")).unwrap();
        assert_eq!(f.remove(&pin), Removal::Restored);
        assert!(
            std::fs::symlink_metadata(f.path("reviewed"))
                .unwrap()
                .is_symlink()
        );
        assert_eq!(std::fs::read(f.path("victim")).unwrap(), b"outside");
    }

    #[test]
    fn ancestor_swap_uses_retained_parent_and_preserves_outside_victim() {
        let f = Fixture::new();
        let pin = f.pin();
        let retained = f.root.path().join("retained");
        std::fs::rename(f.root.path().join("source"), &retained).unwrap();
        let outside = f.root.path().join("outside");
        std::fs::create_dir(&outside).unwrap();
        std::fs::write(outside.join("reviewed"), b"unreviewed").unwrap();
        std::os::unix::fs::symlink(&outside, f.root.path().join("source")).unwrap();
        assert_eq!(f.remove(&pin), Removal::Deleted);
        assert_eq!(
            std::fs::read(outside.join("reviewed")).unwrap(),
            b"unreviewed"
        );
        assert!(!retained.join("reviewed").exists());
    }

    #[test]
    fn cancellation_after_move_restores_or_retains_without_overwrite() {
        for collision in [false, true] {
            let f = Fixture::new();
            let pin = f.pin();
            let cancelled = Cell::new(false);
            let outcome = f
                .store
                .remove_with_hook(
                    f.uid,
                    &f.source,
                    &pin,
                    "reviewed",
                    || cancelled.get(),
                    || {
                        if collision {
                            std::fs::write(f.path("reviewed"), b"new occupant").unwrap();
                        }
                        cancelled.set(true);
                    },
                    None,
                )
                .unwrap();
            if collision {
                let Removal::RecoveryRequired(id) = outcome else {
                    panic!("must retain on collision")
                };
                assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"new occupant");
                assert!(f.store.restore(f.uid, &id, &f.source).is_err());
                assert_eq!(f.store.list(f.uid).unwrap().len(), 1);
                std::fs::rename(f.path("reviewed"), f.path("saved-occupant")).unwrap();
                f.store.restore(f.uid, &id, &f.source).unwrap();
                assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"reviewed data");
                assert_eq!(
                    std::fs::read(f.path("saved-occupant")).unwrap(),
                    b"new occupant"
                );
            } else {
                assert_eq!(outcome, Removal::Restored);
            }
        }
    }

    #[test]
    fn mismatched_entry_and_restore_collision_remain_recoverable() {
        let f = Fixture::new();
        let pin = f.pin();
        std::fs::rename(f.path("reviewed"), f.path("original")).unwrap();
        std::fs::write(f.path("reviewed"), b"replacement").unwrap();
        let outcome = f
            .store
            .remove_with_hook(
                f.uid,
                &f.source,
                &pin,
                "reviewed",
                || false,
                || {
                    std::fs::write(f.path("reviewed"), b"collision").unwrap();
                },
                None,
            )
            .unwrap();
        let Removal::RecoveryRequired(id) = outcome else {
            panic!("must retain replacement")
        };
        assert!(
            f.store
                .restore(f.uid.wrapping_add(1), &id, &f.source)
                .is_err()
        );
        assert_eq!(f.store.list(f.uid.wrapping_add(1)).unwrap().len(), 0);
        std::fs::rename(f.path("reviewed"), f.path("collision")).unwrap();
        f.store.restore(f.uid, &id, &f.source).unwrap();
        assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"replacement");
        assert_eq!(std::fs::read(f.path("original")).unwrap(), b"reviewed data");
        assert_eq!(std::fs::read(f.path("collision")).unwrap(), b"collision");
    }

    #[test]
    fn cancellation_before_move_does_not_change_any_entry() {
        let f = Fixture::new();
        let pin = f.pin();
        assert_eq!(
            f.store
                .remove(f.uid, &f.source, &pin, "reviewed", || true)
                .unwrap(),
            Removal::Restored
        );
        assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"reviewed data");
        assert_eq!(f.store.list(f.uid).unwrap().len(), 0);
    }

    #[test]
    fn hardlinks_directories_bad_names_and_wrong_owner_are_rejected() {
        let f = Fixture::new();
        let pin = f.pin();
        std::fs::hard_link(f.path("reviewed"), f.path("link")).unwrap();
        assert!(
            f.store
                .remove(f.uid, &f.source, &pin, "reviewed", || false)
                .is_err()
        );
        assert!(
            f.store
                .remove(f.uid, &f.source, &f.source, "reviewed", || false)
                .is_err()
        );
        assert!(
            f.store
                .remove(f.uid.wrapping_add(1), &f.source, &pin, "reviewed", || false)
                .is_err()
        );
        for name in ["", ".", "..", "a/b", "nul\0"] {
            assert!(component(name).is_err());
        }
        assert!(component(&"a".repeat(256)).is_err());
    }

    #[test]
    fn failed_rename_never_changes_source_or_unreviewed_destination() {
        for errno in [libc::EXDEV, libc::ENOSPC, libc::EEXIST, libc::EACCES] {
            let f = Fixture::new();
            let pin = f.pin();
            MOVE_ERROR.with(|error| error.set(Some(errno)));
            assert!(
                f.store
                    .remove(f.uid, &f.source, &pin, "reviewed", || false)
                    .is_err()
            );
            assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"reviewed data");
            assert!(f.store.list(f.uid).unwrap().is_empty());
        }
    }

    #[test]
    fn interrupted_transaction_is_recoverable_after_reopening_store() {
        let f = Fixture::new();
        let pin = f.pin();
        let interrupted = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.store.remove_with_hook(
                f.uid,
                &f.source,
                &pin,
                "reviewed",
                || false,
                || panic!("simulated crash after rename"),
                None,
            )
        }));
        assert!(interrupted.is_err());
        let reopened = Quarantine {
            directory: f.store.directory.try_clone().unwrap(),
        };
        let retained = reopened.list(f.uid).unwrap();
        assert_eq!(retained.len(), 1);
        reopened.restore(f.uid, &retained[0].id, &f.source).unwrap();
        assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"reviewed data");
    }

    #[test]
    fn trash_move_and_metadata_never_overwrite_existing_entries() {
        for collision in [0, 1, 2] {
            let f = Fixture::new();
            let pin = f.pin();
            let files_path = f.root.path().join("files");
            let info_path = f.root.path().join("info");
            std::fs::create_dir(&files_path).unwrap();
            std::fs::create_dir(&info_path).unwrap();
            let files = File::open(&files_path).unwrap();
            let info = File::open(&info_path).unwrap();
            let id = std::cell::RefCell::new(String::new());
            let result = f
                .store
                .remove_with_hook(
                    f.uid,
                    &f.source,
                    &pin,
                    "reviewed",
                    || false,
                    || {
                        let receipt = f.store.list(f.uid).unwrap().remove(0);
                        *id.borrow_mut() = receipt.id;
                        if collision == 1 {
                            std::fs::write(files_path.join(&*id.borrow()), b"existing file")
                                .unwrap();
                        }
                        if collision == 2 {
                            std::fs::write(
                                info_path.join(format!("{}.trashinfo", id.borrow())),
                                b"existing metadata",
                            )
                            .unwrap();
                        }
                    },
                    Some((&files, &info)),
                )
                .unwrap();
            if collision == 0 {
                assert_eq!(result, Removal::Trashed);
                assert!(!f.path("reviewed").exists());
                assert_eq!(
                    std::fs::read(files_path.join(&*id.borrow())).unwrap(),
                    b"reviewed data"
                );
                let metadata =
                    std::fs::read_to_string(info_path.join(format!("{}.trashinfo", id.borrow())))
                        .unwrap();
                assert!(metadata.contains("[Trash Info]\nPath="));
                assert!(metadata.contains("DeletionDate="));
            } else {
                assert_eq!(result, Removal::Restored);
                assert_eq!(std::fs::read(f.path("reviewed")).unwrap(), b"reviewed data");
                if collision == 1 {
                    assert_eq!(
                        std::fs::read(files_path.join(&*id.borrow())).unwrap(),
                        b"existing file"
                    );
                }
                if collision == 2 {
                    assert_eq!(
                        std::fs::read(info_path.join(format!("{}.trashinfo", id.borrow())))
                            .unwrap(),
                        b"existing metadata"
                    );
                }
            }
        }
    }

    #[test]
    fn postcommit_failures_do_not_rollback_trash_or_retry_deleted_files() {
        for trash in [false, true] {
            for sync_failure in [false, true] {
                let f = Fixture::new();
                let pin = f.pin();
                let files_path = f.root.path().join("files");
                let info_path = f.root.path().join("info");
                std::fs::create_dir(&files_path).unwrap();
                std::fs::create_dir(&info_path).unwrap();
                let files = File::open(&files_path).unwrap();
                let info = File::open(&info_path).unwrap();
                let result = f
                    .store
                    .remove_with_hook(
                        f.uid,
                        &f.source,
                        &pin,
                        "reviewed",
                        || false,
                        || {
                            std::fs::write(f.path("reviewed"), b"new replacement").unwrap();
                            if sync_failure {
                                COMMIT_SYNC_ERROR.with(|fault| fault.set(true));
                            } else {
                                FINISH_ERROR.with(|fault| fault.set(true));
                            }
                        },
                        trash.then_some((&files, &info)),
                    )
                    .unwrap();
                assert_eq!(
                    result,
                    if trash {
                        Removal::TrashedDurabilityUnknown
                    } else {
                        Removal::DeletedDurabilityUnknown
                    }
                );
                assert_eq!(
                    std::fs::read(f.path("reviewed")).unwrap(),
                    b"new replacement"
                );
                assert!(f.store.list(f.uid).unwrap().is_empty());
                let entries = std::fs::read_dir(&files_path)
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                if trash {
                    assert_eq!(entries.len(), 1);
                    assert_eq!(std::fs::read(entries[0].path()).unwrap(), b"reviewed data");
                } else {
                    assert!(entries.is_empty());
                    assert_eq!(pin.metadata().unwrap().nlink(), 0);
                }
            }
        }
    }

    #[test]
    fn restore_postcommit_failures_are_not_retryable_and_preserve_replacements() {
        for sync_failure in [false, true] {
            let f = Fixture::new();
            let pin = f.pin();
            let outcome = f
                .store
                .remove_with_hook(f.uid, &f.source, &pin, "reviewed", || true, || {}, None)
                .unwrap();
            assert_eq!(outcome, Removal::Restored);
            let cancelled = Cell::new(false);
            let outcome = f
                .store
                .remove_with_hook(
                    f.uid,
                    &f.source,
                    &pin,
                    "reviewed",
                    || cancelled.get(),
                    || {
                        cancelled.set(true);
                        std::fs::write(f.path("reviewed"), b"first occupant").unwrap();
                    },
                    None,
                )
                .unwrap();
            let Removal::RecoveryRequired(id) = outcome else {
                panic!("fixture must retain staged data")
            };
            std::fs::rename(f.path("reviewed"), f.path("first-occupant")).unwrap();
            let result = f
                .store
                .restore_with_hook(f.uid, &id, &f.source, || {
                    std::fs::rename(f.path("reviewed"), f.path("restored-file")).unwrap();
                    std::fs::write(f.path("reviewed"), b"new replacement").unwrap();
                    if sync_failure {
                        COMMIT_SYNC_ERROR.with(|fault| fault.set(true));
                    } else {
                        FINISH_ERROR.with(|fault| fault.set(true));
                    }
                })
                .unwrap();
            assert_eq!(result, Restoration::RestoredDurabilityUnknown);
            assert_eq!(
                std::fs::read(f.path("reviewed")).unwrap(),
                b"new replacement"
            );
            assert_eq!(
                std::fs::read(f.path("restored-file")).unwrap(),
                b"reviewed data"
            );
            assert_eq!(
                std::fs::read(f.path("first-occupant")).unwrap(),
                b"first occupant"
            );
            assert!(f.store.list(f.uid).unwrap().is_empty());
        }
    }

    #[test]
    fn explicit_elsewhere_recovery_survives_lost_parent_and_never_overwrites() {
        let f = Fixture::new();
        let pin = f.pin();
        let cancelled = Cell::new(false);
        let outcome = f
            .store
            .remove_with_hook(
                f.uid,
                &f.source,
                &pin,
                "reviewed",
                || cancelled.get(),
                || {
                    cancelled.set(true);
                    std::fs::write(f.path("reviewed"), b"collision").unwrap();
                },
                None,
            )
            .unwrap();
        let Removal::RecoveryRequired(id) = outcome else {
            panic!("fixture must retain data")
        };
        std::fs::remove_file(f.path("reviewed")).unwrap();
        std::fs::remove_dir(f.root.path().join("source")).unwrap();
        let destination = f.root.path().join("new-folder");
        std::fs::create_dir(&destination).unwrap();
        let parent = File::open(&destination).unwrap();
        assert!(f.store.restore(f.uid, &id, &parent).is_err());
        assert!(
            f.store
                .restore_elsewhere(f.uid.wrapping_add(1), &id, &parent)
                .is_err()
        );
        std::fs::write(destination.join("reviewed"), b"new occupant").unwrap();
        assert!(f.store.restore_elsewhere(f.uid, &id, &parent).is_err());
        assert_eq!(
            std::fs::read(destination.join("reviewed")).unwrap(),
            b"new occupant"
        );
        std::fs::rename(
            destination.join("reviewed"),
            destination.join("preserved-occupant"),
        )
        .unwrap();
        assert_eq!(
            f.store.restore_elsewhere(f.uid, &id, &parent).unwrap(),
            Restoration::Restored
        );
        assert_eq!(
            std::fs::read(destination.join("reviewed")).unwrap(),
            b"reviewed data"
        );
        assert_eq!(
            std::fs::read(destination.join("preserved-occupant")).unwrap(),
            b"new occupant"
        );
    }

    #[test]
    fn writable_quarantine_is_rejected() {
        let f = Fixture::new();
        f.store
            .directory
            .set_permissions(std::fs::Permissions::from_mode(0o777))
            .unwrap();
        assert!(Quarantine::validate_private(&f.store.directory, f.uid).is_err());
    }

    #[test]
    fn new_descendants_are_never_included_in_removal() {
        let f = Fixture::new();
        let pin = f.pin();
        std::fs::write(f.path("added-after-review"), b"new").unwrap();
        assert_eq!(f.remove(&pin), Removal::Deleted);
        assert_eq!(std::fs::read(f.path("added-after-review")).unwrap(), b"new");
        assert!(f.root.path().join("source").is_dir());
    }
}
