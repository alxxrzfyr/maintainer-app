//! D-Bus server exposing the four fixed privileged operations.
//!
//! Methods are routed through validation and authorization before dispatch.
//! Caller credentials are resolved from the D-Bus daemon (kernel credentials),
//! never from self-reported message payload.

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use tokio::sync::Semaphore;
use zbus::interface;

use crate::api::{
    HelperError, JournalVacuumRequest, MAX_CONCURRENT_REQUESTS, PackageCacheRequest,
    SmartRefreshRequest, TerminateProcessRequest, actions,
};
use crate::authorize::{AuthorizePolicy, CallerCredentials};
use crate::validate;

/// The D-Bus interface object implementing `io.github.maintainerapp.Helper`.
pub struct HelperInterface {
    policy: Arc<dyn AuthorizePolicy>,
    concurrency: Arc<Semaphore>,
    inflight: Arc<AtomicUsize>,
}

impl HelperInterface {
    /// Create a new interface with the given authorization policy.
    pub fn new(policy: Arc<dyn AuthorizePolicy>) -> Self {
        Self {
            policy,
            concurrency: Arc::new(Semaphore::new(MAX_CONCURRENT_REQUESTS)),
            inflight: Arc::new(AtomicUsize::new(0)),
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
                // Leak the permit — we release it manually via inflight counter.
                // Actually, hold the permit by incrementing and forgetting.
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
        HelperError::PidReuse => zbus::fdo::Error::Failed(err.to_string()),
        HelperError::OperationFailed { .. } => zbus::fdo::Error::Failed(err.to_string()),
    }
}

#[interface(name = "io.github.maintainerapp.Helper")]
impl HelperInterface {
    /// Terminate a process by PID with start-time revalidation.
    async fn terminate_process(
        &self,
        #[zbus(header)] header: zbus::message::Header<'_>,
        pid: u32,
        start_time_ticks: u64,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let sender = header
            .sender()
            .ok_or_else(|| {
                zbus::fdo::Error::Failed("missing sender in message header".to_string())
            })?
            .to_string();

        self.try_acquire().map_err(to_dbus_error)?;
        let result = self.do_terminate_process(&sender, pid, start_time_ticks);
        self.release();
        result
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
}

impl HelperInterface {
    fn do_terminate_process(
        &self,
        sender: &str,
        pid: u32,
        start_time_ticks: u64,
    ) -> Result<(bool, String), zbus::fdo::Error> {
        let request = TerminateProcessRequest {
            pid,
            start_time_ticks,
        };

        validate::validate_terminate_process(&request).map_err(to_dbus_error)?;
        self.authorize(sender, actions::TERMINATE_PROCESS)
            .map_err(to_dbus_error)?;

        let response =
            crate::operations::process::execute_terminate(&request).map_err(to_dbus_error)?;
        Ok((response.success, response.message))
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

    // Idle timeout: exit after IDLE_TIMEOUT_SECS of no activity.
    let idle_timeout = std::time::Duration::from_secs(crate::api::IDLE_TIMEOUT_SECS);

    let mut sigterm =
        signal(SignalKind::terminate()).map_err(|e| format!("signal handler: {e}"))?;
    let mut sigint = signal(SignalKind::interrupt()).map_err(|e| format!("signal handler: {e}"))?;

    // The connection keeps the server alive. We wait for signal or idle timeout.
    loop {
        tokio::select! {
            _ = sigterm.recv() => {
                break;
            }
            _ = sigint.recv() => {
                break;
            }
            _ = tokio::time::sleep(idle_timeout) => {
                // Check if there are in-flight requests. If so, wait more.
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

        // Fill all slots.
        for _ in 0..MAX_CONCURRENT_REQUESTS {
            assert!(iface.try_acquire().is_ok());
        }

        // Next one should be rejected.
        let result = iface.try_acquire();
        assert!(matches!(result, Err(HelperError::Busy)));

        // Release one and try again.
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
        let result = iface.do_terminate_process(":1.10", 0, 12345);
        // Validation failure becomes InvalidArgs.
        assert!(result.is_err());
    }

    #[test]
    fn do_package_cache_validates_before_authorize() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        // Empty request_id should fail validation.
        let result = iface.do_clean_package_cache(":1.10", "");
        assert!(result.is_err());
    }

    #[test]
    fn do_vacuum_journal_validates_before_authorize() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        // Size below minimum should fail.
        let result = iface.do_vacuum_journal(":1.10", 10, 30);
        assert!(result.is_err());
    }

    #[test]
    fn do_refresh_smart_validates_before_authorize() {
        let policy = Arc::new(MockPolicy::allowing(1000, 1234, ":1.10"));
        let iface = HelperInterface::new(policy);
        // Path traversal in drive_id should fail.
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
