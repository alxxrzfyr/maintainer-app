//! Retained Linux process references for review and SIGTERM delivery.

use std::fs;
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd};

use crate::metrics::processes::ProcessIdentity;

#[derive(Debug, thiserror::Error)]
pub enum ProcessHandleError {
    #[error("invalid process identity")]
    InvalidIdentity,
    #[error("the process has exited")]
    Vanished,
    #[error("the process identity changed; select it again")]
    IdentityChanged,
    #[error("permission is required to access this process")]
    PermissionRequired,
    #[error("stable process references are unavailable on this system")]
    Unavailable,
    #[error("process operation failed: {0}")]
    Io(#[source] io::Error),
}

fn classify(error: io::Error) -> ProcessHandleError {
    match error.raw_os_error() {
        Some(libc::ESRCH | libc::ENOENT) => ProcessHandleError::Vanished,
        Some(libc::EPERM | libc::EACCES) => ProcessHandleError::PermissionRequired,
        Some(libc::ENOSYS | libc::ENODEV) => ProcessHandleError::Unavailable,
        _ => ProcessHandleError::Io(error),
    }
}

/// The descriptor is acquired before review and remains owned until cancellation
/// or signal delivery. Dropping it closes it, including every failure path.
#[derive(Debug)]
pub struct ProcessHandle {
    fd: OwnedFd,
    identity: ProcessIdentity,
    name: String,
}

impl ProcessHandle {
    pub fn acquire(identity: ProcessIdentity) -> Result<Self, ProcessHandleError> {
        validate_identity(&identity)?;
        // SAFETY: the PID fits pid_t, flags zero are supported by the platform floor.
        let raw = unsafe { libc::syscall(libc::SYS_pidfd_open, identity.pid as libc::pid_t, 0) };
        if raw < 0 {
            return Err(classify(io::Error::last_os_error()));
        }
        // SAFETY: successful pidfd_open returns a new, uniquely owned descriptor.
        let fd = unsafe { OwnedFd::from_raw_fd(raw as libc::c_int) };
        let name = validate_reference(fd.as_fd(), &identity)?;
        Ok(Self { fd, identity, name })
    }

    pub fn identity(&self) -> &ProcessIdentity {
        &self.identity
    }

    pub fn name(&self) -> &str {
        &self.name
    }
}

impl AsFd for ProcessHandle {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.fd.as_fd()
    }
}

pub fn validate_identity(identity: &ProcessIdentity) -> Result<(), ProcessHandleError> {
    if identity.pid < 2 || identity.pid > libc::pid_t::MAX as u32 || identity.start_time_ticks == 0
    {
        return Err(ProcessHandleError::InvalidIdentity);
    }
    Ok(())
}

/// Validate a received descriptor before authorization. fdinfo comes from the
/// kernel and binds it to this PID in the helper's namespace. The final liveness
/// check rejects exit/reuse during the numeric /proc metadata read.
pub fn validate_reference(
    fd: BorrowedFd<'_>,
    identity: &ProcessIdentity,
) -> Result<String, ProcessHandleError> {
    validate_reference_with(fd, identity, || {
        fs::read_to_string(format!("/proc/{}/stat", identity.pid))
    })
}

fn validate_reference_with(
    fd: BorrowedFd<'_>,
    identity: &ProcessIdentity,
    read_stat: impl FnOnce() -> io::Result<String>,
) -> Result<String, ProcessHandleError> {
    validate_identity(identity)?;
    // Reject other descriptor kinds before fdinfo: epoll watch lists can be unbounded.
    let origin = fs::read_link(format!("/proc/self/fd/{}", fd.as_raw_fd())).map_err(classify)?;
    if origin != std::path::Path::new("anon_inode:[pidfd]") {
        return Err(ProcessHandleError::InvalidIdentity);
    }
    let info =
        fs::read_to_string(format!("/proc/self/fdinfo/{}", fd.as_raw_fd())).map_err(classify)?;
    let pid = info
        .lines()
        .find_map(|line| line.strip_prefix("Pid:\t"))
        .and_then(|pid| pid.trim().parse::<i64>().ok())
        .ok_or(ProcessHandleError::InvalidIdentity)?;
    if pid == -1 {
        return Err(ProcessHandleError::Vanished);
    }
    if pid != i64::from(identity.pid) {
        return Err(ProcessHandleError::IdentityChanged);
    }
    check_alive(fd)?;
    let stat = read_stat().map_err(classify)?;
    let open = stat.find('(').ok_or(ProcessHandleError::InvalidIdentity)?;
    let close = stat
        .rfind(')')
        .filter(|close| *close > open)
        .ok_or(ProcessHandleError::InvalidIdentity)?;
    let start = stat[close + 1..]
        .split_whitespace()
        .nth(19)
        .and_then(|value| value.parse::<u64>().ok())
        .ok_or(ProcessHandleError::InvalidIdentity)?;
    check_alive(fd)?;
    if start != identity.start_time_ticks {
        return Err(ProcessHandleError::IdentityChanged);
    }
    Ok(stat[open + 1..close].to_string())
}

fn check_alive(fd: BorrowedFd<'_>) -> Result<(), ProcessHandleError> {
    let mut event = libc::pollfd {
        fd: fd.as_raw_fd(),
        events: libc::POLLIN,
        revents: 0,
    };
    // SAFETY: event is a live writable pollfd, count is one, timeout is zero.
    let result = unsafe { libc::poll(&mut event, 1, 0) };
    if result < 0 {
        return Err(classify(io::Error::last_os_error()));
    }
    if event.revents != 0 {
        return Err(ProcessHandleError::Vanished);
    }
    Ok(())
}

/// Success means the kernel accepted SIGTERM, not that the process has exited.
/// There is deliberately no numeric PID signal path.
pub fn send_sigterm(fd: BorrowedFd<'_>) -> Result<(), ProcessHandleError> {
    // SAFETY: the borrowed descriptor remains live for the call, the signal is
    // fixed, siginfo is null, and flags zero preserve kernel permission checks.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        Err(classify(io::Error::last_os_error()))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command};

    fn child() -> (Child, ProcessIdentity) {
        let child = Command::new("sleep").arg("60").spawn().unwrap();
        let stat = fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        let start_time_ticks = stat[stat.rfind(')').unwrap() + 1..]
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        let identity = ProcessIdentity {
            pid: child.id(),
            start_time_ticks,
        };
        (child, identity)
    }

    #[test]
    fn permission_vanished_and_unavailable_are_distinct() {
        for errno in [libc::EPERM, libc::EACCES] {
            assert!(matches!(
                classify(io::Error::from_raw_os_error(errno)),
                ProcessHandleError::PermissionRequired
            ));
        }
        assert!(matches!(
            classify(io::Error::from_raw_os_error(libc::ESRCH)),
            ProcessHandleError::Vanished
        ));
        assert!(matches!(
            classify(io::Error::from_raw_os_error(libc::ENOSYS)),
            ProcessHandleError::Unavailable
        ));
        assert!(matches!(
            classify(io::Error::from_raw_os_error(libc::EMFILE)),
            ProcessHandleError::Io(_)
        ));
    }

    #[test]
    fn invalid_pids_and_start_times_are_rejected_before_open() {
        for (pid, start_time_ticks) in [(0, 1), (1, 1), (u32::MAX, 1), (2, 0)] {
            assert!(matches!(
                ProcessHandle::acquire(ProcessIdentity {
                    pid,
                    start_time_ticks
                }),
                Err(ProcessHandleError::InvalidIdentity)
            ));
        }
    }

    #[test]
    fn retained_reference_signals_only_its_process() {
        let (mut target, identity) = child();
        let (mut unrelated, _) = child();
        let handle = ProcessHandle::acquire(identity).unwrap();
        send_sigterm(handle.as_fd()).unwrap();
        assert!(!target.wait().unwrap().success());
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }

    #[test]
    fn exit_during_metadata_read_rejects_review() {
        let (mut target, identity) = child();
        let handle = ProcessHandle::acquire(identity.clone()).unwrap();
        let result = validate_reference_with(handle.as_fd(), &identity, || {
            let stat = fs::read_to_string(format!("/proc/{}/stat", identity.pid))?;
            target.kill()?;
            target.wait()?;
            Ok(stat)
        });
        assert!(matches!(result, Err(ProcessHandleError::Vanished)));
    }

    #[test]
    fn exit_after_review_cannot_signal_a_replacement() {
        let (mut target, identity) = child();
        let handle = ProcessHandle::acquire(identity).unwrap();
        target.kill().unwrap();
        target.wait().unwrap();
        let (mut replacement, _) = child();
        assert!(matches!(
            send_sigterm(handle.as_fd()),
            Err(ProcessHandleError::Vanished)
        ));
        assert!(replacement.try_wait().unwrap().is_none());
        replacement.kill().unwrap();
        replacement.wait().unwrap();
    }

    #[test]
    fn mismatched_metadata_or_non_pidfd_is_rejected() {
        let (mut target, mut identity) = child();
        let handle = ProcessHandle::acquire(identity.clone()).unwrap();
        identity.start_time_ticks += 1;
        assert!(matches!(
            validate_reference(handle.as_fd(), &identity),
            Err(ProcessHandleError::IdentityChanged)
        ));
        let file = fs::File::open("/dev/null").unwrap();
        assert!(matches!(
            validate_reference(file.as_fd(), &identity),
            Err(ProcessHandleError::InvalidIdentity)
        ));
        target.kill().unwrap();
        target.wait().unwrap();
    }

    #[test]
    fn cancellation_drops_owned_descriptor() {
        let (mut target, identity) = child();
        let handle = ProcessHandle::acquire(identity).unwrap();
        let raw = handle.as_fd().as_raw_fd();
        drop(handle);
        // Other parallel tests may reuse the descriptor number after close.
        // They cannot recreate a reference to this test's still-live child.
        let info = fs::read_to_string(format!("/proc/self/fdinfo/{raw}")).unwrap_or_default();
        assert!(
            !info
                .lines()
                .any(|line| line == format!("Pid:\t{}", target.id()))
        );
        assert!(target.try_wait().unwrap().is_none());
        target.kill().unwrap();
        target.wait().unwrap();
    }
}
