//! D-Bus server exposing the four fixed privileged operations.
//!
//! Methods are routed through validation and authorization before dispatch.
//! Caller credentials are resolved from the D-Bus daemon (kernel credentials),
//! never from self-reported message payload.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Semaphore;
use zbus::interface;

use crate::api::{
    HelperError, JournalVacuumRequest, MAX_CONCURRENT_REQUESTS, PackageCacheRequest,
    RemoveKernelRequest, SmartRefreshRequest, TerminateProcessRequest, actions,
};
use crate::authorize::{AuthorizePolicy, CallerCredentials};
use crate::validate;

/// The D-Bus interface object implementing `io.github.maintainerapp.Helper`.
pub struct HelperInterface {
    policy: Arc<dyn AuthorizePolicy>,
    concurrency: Arc<Semaphore>,
    inflight: Arc<AtomicUsize>,
    cleanup_batches: Mutex<crate::cleanup_batch::CleanupBatches>,
    #[cfg(test)]
    cleanup_test_dispatch: bool,
}

impl HelperInterface {
    /// Create a new interface with the given authorization policy.
    pub fn new(policy: Arc<dyn AuthorizePolicy>) -> Self {
        Self {
            policy,
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            inflight: Arc::new(AtomicUsize::new(0)),
            cleanup_batches: Mutex::new(crate::cleanup_batch::CleanupBatches::default()),
            #[cfg(test)]
            cleanup_test_dispatch: false,
        }
    }

    /// Resolve caller credentials from the D-Bus connection.
    fn authorize(&self, sender: &str, action_id: &str) -> Result<CallerCredentials, HelperError> {
        crate::authorize::authorize_request(self.policy.as_ref(), sender, action_id)
    }

    /// Acquire a concurrency permit or return Busy.
    fn try_acquire(&self) -> Result<(), HelperError> {
        match self.concurrency.try_acquire() {
            Ok(permit) => {
                permit.forget();
                self.inflight.fetch_add(1, Ordering::SeqCst);
                Ok(())
            }
            Err(_) => Err(HelperError::Busy),
        }
    }

    /// Release a concurrency slot.
    fn release(&self) {
        self.inflight.fetch_sub(1, Ordering::SeqCst);
        self.concurrency.add_permits(1);
    }

    /// Current number of in-flight requests.
    pub fn inflight_count(&self) -> usize {
        self.inflight.load(Ordering::SeqCst)
    }
}

/// Encode a `HelperError` as a D-Bus error.
fn to_dbus_error(err: HelperError) -> zbus::fdo::Error {
    match &err {
        HelperError::AuthorizationDenied { .. } => zbus::fdo::Error::AccessDenied(err.to_string()),
        HelperError::CallerIdentityFailed => zbus::fdo::Error::AuthFailed(err.to_string()),
        HelperError::ValidationFailed { .. } => zbus::fdo::Error::InvalidArgs(err.to_string()),
        HelperError::Busy => zbus::fdo::Error::LimitsExceeded(err.to_string()),
        HelperError::ProcessVanished => zbus::fdo::Error::UnixProcessIdUnknown(err.to_string()),
        HelperError::ProcessUnavailable => zbus::fdo::Error::NotSupported(err.to_string()),
        HelperError::PidReuse => zbus::fdo::Error::Failed(err.to_string()),
        HelperError::OperationFailed { .. } => zbus::fdo::Error::Failed(err.to_string()),
    }
}

#[interface(name = "io.github.maintainerapp.Helper")]
impl HelperInterface {
    /// Send SIGTERM using the process descriptor retained before UI review.
    async fn terminate_process(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        pid: u32,
        start_time_ticks: u64,
        process_fd: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("missing sender in message header".to_string())
            })?
            .to_string();

        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_terminate_process(&sender, pid, start_time_ticks, process_fd);
        self.release();
        result
    }

    async fn begin_cleanup_batch(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        manifest: Vec<crate::cleanup_batch::ManifestEntry>,
    ) -> Result<String, zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        self.do_begin_cleanup_batch(sender.as_str(), manifest)
            .map_err(to_dbus_error)
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Per-file descriptor transfer stays bounded instead of sending a whole manifest of FDs"
    )]
    async fn remove_reviewed_batch_file(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        grant: &str,
        index: u32,
        parent: zbus::zvariant::OwnedFd,
        reviewed: zbus::zvariant::OwnedFd,
        basename: &str,
        trash: bool,
        files: zbus::zvariant::OwnedFd,
        info: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        self.try_acquire().map_err(to_dbus_error)?;
        let result = (|| {
            let caller = self.policy.resolve_caller(sender.as_str())?;
            let parent = std::fs::File::from(std::os::fd::OwnedFd::from(parent));
            let reviewed = std::fs::File::from(std::os::fd::OwnedFd::from(reviewed));
            self.cleanup_batches
                .lock()
                .map_err(|_| HelperError::Busy)?
                .consume(
                    sender.as_str(),
                    caller.uid,
                    grant,
                    index,
                    &parent,
                    &reviewed,
                    basename,
                    trash,
                )?;
            #[cfg(test)]
            if self.cleanup_test_dispatch {
                return Ok((true, String::new()));
            }
            crate::operations::files::remove(
                caller.uid,
                parent.into(),
                reviewed.into(),
                basename,
                trash,
                files.into(),
                info.into(),
            )
        })();
        self.release();
        result.map_err(to_dbus_error)
    }

    async fn end_cleanup_batch(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        grant: &str,
    ) -> Result<(), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        let caller = self
            .policy
            .resolve_caller(sender.as_str())
            .map_err(to_dbus_error)?;
        self.cleanup_batches
            .lock()
            .map_err(|_| to_dbus_error(HelperError::Busy))?
            .end(sender.as_str(), caller.uid, grant)
            .map_err(to_dbus_error)
    }

    /// Atomically isolate and remove only the descriptor-pinned reviewed file.
    #[allow(
        clippy::too_many_arguments,
        reason = "D-Bus uses separate Unix descriptors for source and Trash capabilities"
    )]
    async fn remove_reviewed_file(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        parent: zbus::zvariant::OwnedFd,
        reviewed: zbus::zvariant::OwnedFd,
        basename: &str,
        trash: bool,
        files: zbus::zvariant::OwnedFd,
        info: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_remove_reviewed_file(
            sender.as_str(),
            parent,
            reviewed,
            basename,
            trash,
            files,
            info,
        );
        self.release();
        result
    }

    async fn list_cleanup_recovery(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
    ) -> Result<Vec<(String, String, String)>, zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        let caller = self
            .authorize(sender.as_str(), actions::CLEANUP_FILES)
            .map_err(to_dbus_error)?;
        crate::operations::files::list(caller.uid).map_err(to_dbus_error)
    }

    async fn restore_cleanup_recovery(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        id: &str,
        parent: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        let caller = self
            .authorize(sender.as_str(), actions::CLEANUP_FILES)
            .map_err(to_dbus_error)?;
        crate::operations::files::restore(caller.uid, id, parent.into(), false)
            .map_err(to_dbus_error)
    }

    async fn restore_cleanup_elsewhere(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        id: &str,
        parent: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| zbus::fdo::Error::AuthFailed("missing sender".into()))?;
        let caller = self
            .authorize(sender.as_str(), actions::CLEANUP_FILES)
            .map_err(to_dbus_error)?;
        crate::operations::files::restore(caller.uid, id, parent.into(), true)
            .map_err(to_dbus_error)
    }

    /// Clean the APT package cache.
    async fn clean_package_cache(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        request_id: &str,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("missing sender in message header".to_string())
            })?
            .to_string();

        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_clean_package_cache(&sender, request_id);
        self.release();
        result
    }

    /// Vacuum the systemd journal.
    async fn vacuum_journal(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        max_size_mib: u32,
        max_age_days: u32,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("missing sender in message header".to_string())
            })?
            .to_string();

        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_vacuum_journal(&sender, max_size_mib, max_age_days);
        self.release();
        result
    }

    /// Refresh SMART data for a drive.
    async fn refresh_smart(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        drive_id: &str,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("missing sender in message header".to_string())
            })?
            .to_string();

        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_refresh_smart(&sender, drive_id);
        self.release();
        result
    }

    /// Remove an old kernel package via apt-get purge.
    async fn remove_kernel(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        package_name: &str,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("missing sender in message header".to_string())
            })?
            .to_string();

        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_remove_kernel(&sender, package_name);
        self.release();
        result
    }
}

impl HelperInterface {
    fn do_begin_cleanup_batch(
        &self,
        sender: &str,
        manifest: Vec<crate::cleanup_batch::ManifestEntry>,
    ) -> Result<String, HelperError> {
        let mut batches = self.cleanup_batches.lock().map_err(|_| HelperError::Busy)?;
        batches.validate(&manifest)?;
        let caller = self.authorize(sender, actions::CLEANUP_FILES)?;
        batches.issue(sender, caller.uid, manifest)
    }

    fn do_terminate_process(
        &self,
        sender: &str,
        pid: u32,
        start_time_ticks: u64,
        process_fd: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let request = TerminateProcessRequest {
            pid,
            start_time_ticks,
        };

        validate::validate_terminate_process(&request).map_err(to_dbus_error)?;
        use std::os::fd::AsFd;
        crate::operations::process::validate_handle(&request, process_fd.as_fd())
            .map_err(to_dbus_error)?;
        self.authorize(sender, actions::TERMINATE_PROCESS)
            .map_err(to_dbus_error)?;

        let response = crate::operations::process::execute_terminate(process_fd.as_fd())
            .map_err(to_dbus_error)?;
        Ok((response.success, response.message))
    }

    #[allow(
        clippy::too_many_arguments,
        reason = "Authorization precedes dispatch of the fixed descriptor-only operation"
    )]
    fn do_remove_reviewed_file(
        &self,
        sender: &str,
        parent: zbus::zvariant::OwnedFd,
        reviewed: zbus::zvariant::OwnedFd,
        basename: &str,
        trash: bool,
        files: zbus::zvariant::OwnedFd,
        info: zbus::zvariant::OwnedFd,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        maintainer_core::cleanup::quarantine::component(basename).map_err(|error| {
            to_dbus_error(HelperError::ValidationFailed {
                reason: error.to_string(),
            })
        })?;
        let caller = self
            .authorize(sender, actions::CLEANUP_FILES)
            .map_err(to_dbus_error)?;
        crate::operations::files::remove(
            caller.uid,
            parent.into(),
            reviewed.into(),
            basename,
            trash,
            files.into(),
            info.into(),
        )
        .map_err(to_dbus_error)
    }

    fn do_clean_package_cache(
        &self,
        sender: &str,
        request_id: &str,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let request = PackageCacheRequest {
            request_id: request_id.to_string(),
        };

        validate::validate_package_cache(&request).map_err(to_dbus_error)?;
        self.authorize(sender, actions::CLEAN_PACKAGE_CACHE)
            .map_err(to_dbus_error)?;

        let response = crate::operations::package_cache::execute_clean_package_cache()
            .map_err(to_dbus_error)?;
        Ok((response.success, response.message))
    }

    fn do_vacuum_journal(
        &self,
        sender: &str,
        max_size_mib: u32,
        max_age_days: u32,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let request = JournalVacuumRequest {
            max_size_mib,
            max_age_days,
        };

        validate::validate_journal_vacuum(&request).map_err(to_dbus_error)?;
        self.authorize(sender, actions::VACUUM_JOURNAL)
            .map_err(to_dbus_error)?;

        let response =
            crate::operations::journal::execute_vacuum_journal(&request).map_err(to_dbus_error)?;
        Ok((response.success, response.message))
    }

    fn do_refresh_smart(
        &self,
        sender: &str,
        drive_id: &str,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let request = SmartRefreshRequest {
            drive_id: drive_id.to_string(),
        };

        validate::validate_smart_refresh(&request).map_err(to_dbus_error)?;
        self.authorize(sender, actions::REFRESH_SMART)
            .map_err(to_dbus_error)?;

        let response =
            crate::operations::smart::execute_smart_refresh(&request).map_err(to_dbus_error)?;
        Ok((response.success, response.message))
    }

    fn do_remove_kernel(
        &self,
        sender: &str,
        package_name: &str,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let request = RemoveKernelRequest {
            package_name: package_name.to_string(),
        };

        validate::validate_remove_kernel(&request).map_err(to_dbus_error)?;
        self.authorize(sender, actions::REMOVE_KERNEL)
            .map_err(to_dbus_error)?;

        let response =
            crate::operations::kernel::execute_remove_kernel(&request).map_err(to_dbus_error)?;
        Ok((response.success, response.message))
    }
}

/// D-Bus server runtime configuration.
pub struct ServerConfig {
    /// Bus name to request on the system bus.
    pub bus_name: &'static str,
    /// Object path to serve the interface.
    pub object_path: &'static str,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bus_name: "io.github.maintainerapp.Helper",
            object_path: "/io/github/maintainerapp/Helper",
        }
    }
}

/// Run the D-Bus server until idle timeout or termination.
///
/// Returns `Ok(())` on clean idle shutdown or signal-based termination.
pub async fn run_server(
    config: ServerConfig,
    policy: Arc<dyn AuthorizePolicy>,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use tokio::signal::unix::{SignalKind, signal};

    let interface = HelperInterface::new(policy);

    let connection = zbus::connection::Builder::system()
        .map_err(|e| format!("failed to configure system bus connection: {e}"))?
        .name(config.bus_name)
        .map_err(|e| format!("failed to request bus name: {e}"))?
        .serve_at(config.object_path, interface)
        .map_err(|e| format!("failed to serve interface: {e}"))?
        .build()
        .await
        .map_err(|e| format!("failed to connect to system bus: {e}"))?;

    let idle_timeout = std::time::Duration::from_secs(crate::api::IDLE_TIMEOUT_SECS);

    let mut sigterm =
        signal(SignalKind::terminate()).map_err(|e| format!("signal handler: {e}"))?;
    let mut sigint = signal(SignalKind::interrupt()).map_err(|e| format!("signal handler: {e}"))?;

    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                break;
            }
            _ = sigint.recv() => {
                break;
            }
            _ = tokio::time::sleep(idle_timeout) => {
                let iface_ref = connection
                    .object_server()
                    .interface::<_, HelperInterface>(config.object_path)
                    .await;
                match iface_ref {
                    Ok(iface) => {
                        let guard = iface.get().await;
                        if guard.inflight_count() == 0 {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorize::mock::MockPolicy;

    #[test]
    fn interface_rejects_when_concurrency_full() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);

        for _ in 0..MAX_CONCURRENT_REQUESTS {
            assert!(iface.try_acquire().is_ok());
        }

        let result = iface.try_acquire();
        assert!(matches!(result, Err(HelperError::Busy)));

        iface.release();
        assert!(iface.try_acquire().is_ok());
    }

    #[test]
    fn interface_authorize_denies_with_bad_caller() {
        let policy = Arc::new(MockPolicy::denying_caller());
        let iface = HelperInterface::new(policy);
        let result = iface.authorize(":1.999", actions::TERMINATE_PROCESS);
        assert!(matches!(result, Err(HelperError::CallerIdentityFailed)));
    }

    #[test]
    fn interface_authorize_denies_polkit() {
        let policy = Arc::new(MockPolicy::denying_polkit(actions::CLEAN_PACKAGE_CACHE));
        let iface = HelperInterface::new(policy);
        let result = iface.authorize(":1.42", actions::CLEAN_PACKAGE_CACHE);
        assert!(matches!(
            result,
            Err(HelperError::AuthorizationDenied { .. })
        ));
    }

    #[test]
    fn do_terminate_validates_before_authorize() {
        // Policy allows everything — validation should reject pid=0.
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        let file = std::fs::File::open("/dev/null").unwrap();
        let result =
            iface.do_terminate_process(":1.10", 0, 12345, std::os::fd::OwnedFd::from(file).into());
        assert!(result.is_err());
    }

    #[test]
    fn file_cleanup_denial_happens_before_any_filesystem_work() {
        for policy in [
            MockPolicy::denying_caller(),
            MockPolicy::denying_polkit(actions::CLEANUP_FILES),
            MockPolicy::dismissed(actions::CLEANUP_FILES),
            MockPolicy::polkit_unavailable(actions::CLEANUP_FILES),
        ] {
            let iface = HelperInterface::new(Arc::new(policy));
            let fd =
                || std::os::fd::OwnedFd::from(std::fs::File::open("/dev/null").unwrap()).into();
            let error = iface
                .do_remove_reviewed_file(":1.10", fd(), fd(), "reviewed", false, fd(), fd())
                .unwrap_err();
            assert!(!error.to_string().contains("quarantine"));
        }
    }

    struct CountCleanupPolicy {
        count: std::sync::atomic::AtomicUsize,
        uid: u32,
    }
    impl AuthorizePolicy for CountCleanupPolicy {
        fn resolve_caller(&self, sender: &str) -> Result<CallerCredentials, HelperError> {
            Ok(CallerCredentials {
                uid: self.uid,
                pid: 1234,
                bus_name: sender.into(),
            })
        }
        fn check_authorization(
            &self,
            _: &CallerCredentials,
            action: &str,
        ) -> Result<(), HelperError> {
            assert_eq!(action, actions::CLEANUP_FILES);
            self.count.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
    }
    #[test]
    fn cleanup_batch_authorizes_once_for_two_files_and_validates_before_prompt() {
        use std::os::unix::fs::MetadataExt;
        let parent = std::fs::File::open(env!("CARGO_MANIFEST_DIR")).unwrap();
        let file = std::fs::File::open(concat!(env!("CARGO_MANIFEST_DIR"), "/Cargo.toml")).unwrap();
        let p = parent.metadata().unwrap();
        let f = file.metadata().unwrap();
        let entry = (
            "Cargo.toml".into(),
            p.dev(),
            p.ino(),
            f.dev(),
            f.ino(),
            false,
        );
        let policy = Arc::new(CountCleanupPolicy {
            count: AtomicUsize::new(0),
            uid: f.uid(),
        });
        let iface = HelperInterface::new(policy.clone());
        assert!(iface.do_begin_cleanup_batch(":1.10", vec![]).is_err());
        assert_eq!(policy.count.load(Ordering::SeqCst), 0);
        let id = iface
            .do_begin_cleanup_batch(":1.10", vec![entry.clone(), entry])
            .unwrap();
        for index in [0, 1] {
            iface
                .cleanup_batches
                .lock()
                .unwrap()
                .consume(
                    ":1.10",
                    f.uid(),
                    &id,
                    index,
                    &parent,
                    &file,
                    "Cargo.toml",
                    false,
                )
                .unwrap();
        }
        assert_eq!(policy.count.load(Ordering::SeqCst), 1);
        let denied =
            HelperInterface::new(Arc::new(MockPolicy::denying_polkit(actions::CLEANUP_FILES)));
        assert!(
            denied
                .do_begin_cleanup_batch(
                    ":1.10",
                    vec![(
                        "Cargo.toml".into(),
                        p.dev(),
                        p.ino(),
                        f.dev(),
                        f.ino(),
                        false
                    )]
                )
                .is_err()
        );
        assert_eq!(file.metadata().unwrap().len(), f.len());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cleanup_batch_private_bus_keeps_one_authorization_and_connection_binding() {
        use std::io::BufRead;
        use std::os::fd::AsFd;
        use std::os::unix::fs::MetadataExt;
        struct PrivateBus(std::process::Child);
        impl Drop for PrivateBus {
            fn drop(&mut self) {
                let _kill = self.0.kill();
                let _wait = self.0.wait();
            }
        }
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
        let roots = [
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")),
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."),
        ];
        let parents = roots
            .iter()
            .map(|root| std::fs::File::open(root).unwrap())
            .collect::<Vec<_>>();
        let files = roots
            .iter()
            .map(|root| std::fs::File::open(root.join("Cargo.toml")).unwrap())
            .collect::<Vec<_>>();
        let manifest = parents
            .iter()
            .zip(&files)
            .map(|(parent, file)| {
                let p = parent.metadata().unwrap();
                let f = file.metadata().unwrap();
                (
                    "Cargo.toml".to_string(),
                    p.dev(),
                    p.ino(),
                    f.dev(),
                    f.ino(),
                    false,
                )
            })
            .collect::<Vec<_>>();
        let policy = Arc::new(CountCleanupPolicy {
            count: AtomicUsize::new(0),
            uid: files[0].metadata().unwrap().uid(),
        });
        let mut interface = HelperInterface::new(policy.clone());
        interface.cleanup_test_dispatch = true;
        let server = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .name("io.github.maintainerapp.Helper")
            .unwrap()
            .serve_at("/io/github/maintainerapp/Helper", interface)
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let other = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let reply = client
            .call_method(
                Some("io.github.maintainerapp.Helper"),
                "/io/github/maintainerapp/Helper",
                Some("io.github.maintainerapp.Helper"),
                "BeginCleanupBatch",
                &(manifest,),
            )
            .await
            .unwrap();
        let grant: String = reply.body().deserialize().unwrap();
        for index in [0u32, 1] {
            let args = (
                grant.as_str(),
                index,
                zbus::zvariant::Fd::from(parents[index as usize].as_fd()),
                zbus::zvariant::Fd::from(files[index as usize].as_fd()),
                "Cargo.toml",
                false,
                zbus::zvariant::Fd::from(parents[index as usize].as_fd()),
                zbus::zvariant::Fd::from(parents[index as usize].as_fd()),
            );
            assert!(
                other
                    .call_method(
                        Some("io.github.maintainerapp.Helper"),
                        "/io/github/maintainerapp/Helper",
                        Some("io.github.maintainerapp.Helper"),
                        "RemoveReviewedBatchFile",
                        &args
                    )
                    .await
                    .is_err()
            );
            let reply = client
                .call_method(
                    Some("io.github.maintainerapp.Helper"),
                    "/io/github/maintainerapp/Helper",
                    Some("io.github.maintainerapp.Helper"),
                    "RemoveReviewedBatchFile",
                    &args,
                )
                .await
                .unwrap();
            assert!(reply.body().deserialize::<(bool, String)>().unwrap().0);
            assert!(
                client
                    .call_method(
                        Some("io.github.maintainerapp.Helper"),
                        "/io/github/maintainerapp/Helper",
                        Some("io.github.maintainerapp.Helper"),
                        "RemoveReviewedBatchFile",
                        &args
                    )
                    .await
                    .is_err()
            );
        }
        assert_eq!(policy.count.load(Ordering::SeqCst), 1);
        client
            .call_method(
                Some("io.github.maintainerapp.Helper"),
                "/io/github/maintainerapp/Helper",
                Some("io.github.maintainerapp.Helper"),
                "EndCleanupBatch",
                &(grant,),
            )
            .await
            .unwrap();
        drop(other);
        drop(client);
        drop(server);
    }

    fn process_target() -> (std::process::Child, maintainer_core::metrics::ProcessHandle) {
        let child = std::process::Command::new("sleep")
            .arg("60")
            .spawn()
            .unwrap();
        let stat = std::fs::read_to_string(format!("/proc/{}/stat", child.id())).unwrap();
        let start_time_ticks = stat[stat.rfind(')').unwrap() + 1..]
            .split_whitespace()
            .nth(19)
            .unwrap()
            .parse()
            .unwrap();
        let handle = maintainer_core::metrics::ProcessHandle::acquire(
            maintainer_core::metrics::ProcessIdentity {
                pid: child.id(),
                start_time_ticks,
            },
        )
        .unwrap();
        (child, handle)
    }

    fn received_fd(handle: &maintainer_core::metrics::ProcessHandle) -> zbus::zvariant::OwnedFd {
        use std::os::fd::AsFd;
        handle.as_fd().try_clone_to_owned().unwrap().into()
    }

    #[test]
    fn process_denial_dismissal_and_unavailable_send_no_signal() {
        for policy in [
            MockPolicy::denying_polkit(actions::TERMINATE_PROCESS),
            MockPolicy::dismissed(actions::TERMINATE_PROCESS),
            MockPolicy::polkit_unavailable(actions::TERMINATE_PROCESS),
            MockPolicy::denying_caller(),
        ] {
            let (mut child, handle) = process_target();
            let iface = HelperInterface::new(Arc::new(policy));
            assert!(
                iface
                    .do_terminate_process(
                        ":1.10",
                        handle.identity().pid,
                        handle.identity().start_time_ticks,
                        received_fd(&handle)
                    )
                    .is_err()
            );
            assert!(child.try_wait().unwrap().is_none());
            child.kill().unwrap();
            child.wait().unwrap();
        }
    }

    struct ExitDuringAuthorization(std::sync::Mutex<std::process::Child>);

    impl AuthorizePolicy for ExitDuringAuthorization {
        fn resolve_caller(&self, sender: &str) -> Result<CallerCredentials, HelperError> {
            Ok(CallerCredentials {
                uid: 1000,
                pid: 1234,
                bus_name: sender.into(),
            })
        }
        fn check_authorization(
            &self,
            _: &CallerCredentials,
            action: &str,
        ) -> Result<(), HelperError> {
            assert_eq!(action, actions::TERMINATE_PROCESS);
            let mut child = self.0.lock().unwrap();
            child.kill().unwrap();
            child.wait().unwrap();
            Ok(())
        }
    }

    #[test]
    fn process_exit_during_authorization_cannot_retarget_signal() {
        let (target, handle) = process_target();
        let (mut unrelated, _) = process_target();
        let iface = HelperInterface::new(Arc::new(ExitDuringAuthorization(std::sync::Mutex::new(
            target,
        ))));
        let error = iface
            .do_terminate_process(
                ":1.10",
                handle.identity().pid,
                handle.identity().start_time_ticks,
                received_fd(&handle),
            )
            .unwrap_err();
        assert!(error.to_string().contains("exited"));
        assert!(unrelated.try_wait().unwrap().is_none());
        unrelated.kill().unwrap();
        unrelated.wait().unwrap();
    }

    #[test]
    fn process_authorization_success_reports_signal_acceptance_only() {
        let (mut child, handle) = process_target();
        let iface = HelperInterface::new(Arc::new(MockPolicy::allowing(1000, 1234, ":1.10")));
        let (success, message) = iface
            .do_terminate_process(
                ":1.10",
                handle.identity().pid,
                handle.identity().start_time_ticks,
                received_fd(&handle),
            )
            .unwrap();
        assert!(success);
        assert!(message.contains("exit is not yet confirmed"));
        assert!(!child.wait().unwrap().success());
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn process_descriptor_survives_real_dbus_transport() {
        use std::io::BufRead;
        use std::os::fd::AsFd;
        struct PrivateBus(std::process::Child);
        impl Drop for PrivateBus {
            fn drop(&mut self) {
                let _kill = self.0.kill();
                let _wait = self.0.wait();
            }
        }
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
        let server = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .name("io.github.maintainerapp.Helper")
            .unwrap()
            .serve_at(
                "/io/github/maintainerapp/Helper",
                HelperInterface::new(Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"))),
            )
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let (mut child, handle) = process_target();
        let fd = zbus::zvariant::Fd::from(handle.as_fd());
        let reply = client
            .call_method(
                Some("io.github.maintainerapp.Helper"),
                "/io/github/maintainerapp/Helper",
                Some("io.github.maintainerapp.Helper"),
                "TerminateProcess",
                &(
                    handle.identity().pid,
                    handle.identity().start_time_ticks,
                    fd,
                ),
            )
            .await
            .unwrap();
        let (accepted, message): (bool, String) = reply.body().deserialize().unwrap();
        assert!(accepted);
        assert!(message.contains("exit is not yet confirmed"));
        assert!(!child.wait().unwrap().success());
        drop(client);
        drop(server);
    }

    #[test]
    fn do_remove_kernel_validates_before_authorize() {
        // Policy allows everything — validation should reject an empty
        // package name before authorization is ever consulted, matching the
        // other four operations.
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        let result = iface.do_remove_kernel(":1.10", "");
        assert!(result.is_err());
    }

    #[test]
    fn do_package_cache_validates_before_authorize() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        let result = iface.do_clean_package_cache(":1.10", "");
        assert!(result.is_err());
    }

    #[test]
    fn do_vacuum_journal_validates_before_authorize() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        let result = iface.do_vacuum_journal(":1.10", 10, 30);
        assert!(result.is_err());
    }

    #[test]
    fn do_refresh_smart_validates_before_authorize() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        let result = iface.do_refresh_smart(":1.10", "../etc/shadow");
        assert!(result.is_err());
    }

    #[test]
    fn to_dbus_error_maps_variants() {
        let err = to_dbus_error(HelperError::Busy);
        assert!(
            format!("{err}").contains("concurrent request limit")
                || format!("{err:?}").contains("LimitsExceeded")
        );

        let err = to_dbus_error(HelperError::PidReuse);
        let msg = format!("{err:?}");
        assert!(msg.contains("Failed") || msg.contains("pid reuse"));
    }
}
