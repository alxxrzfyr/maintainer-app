//! Hardware, diagnostics and SMART jobs.

use std::sync::Arc;

use maintainer_core::jobs::{CancellationToken, JobExecutor};

use super::{install_progress_poller, install_result_poller, vector_bytes};
use crate::progress::SharedProgress;

/// Hardware collection is scheduled by the lazy page controller, never by widgets.
pub(crate) fn dispatch_hardware_collection(
    executor: &Arc<JobExecutor>,
    collect: impl FnOnce()
        -> maintainer_core::error::Result<maintainer_core::hardware::HardwareSnapshot>
    + Send
    + 'static,
    on_done: impl FnOnce(Result<maintainer_core::hardware::HardwareSnapshot, String>) + 'static,
) {
    let job = move || Ok(collect().map_err(|error| error.to_string()));
    let receiver = executor.execute_sized(vec![job], CancellationToken::new(), |outcome| {
        std::mem::size_of_val(outcome)
            + match outcome {
                Ok(snapshot) => {
                    let gpu = &snapshot.gpu;
                    let bios = &snapshot.bios;
                    [
                        &gpu.brand,
                        &gpu.name,
                        &gpu.pci_id,
                        &gpu.driver,
                        &gpu.opengl,
                        &gpu.opengl_es,
                        &gpu.vulkan,
                        &gpu.video_playback,
                        &bios.vendor,
                        &bios.version,
                        &bios.release_date,
                        &bios.motherboard,
                    ]
                    .into_iter()
                    .map(String::capacity)
                    .sum::<usize>()
                        + vector_bytes(&snapshot.usb_devices, |device| {
                            [
                                &device.name,
                                &device.id,
                                &device.speed,
                                &device.power,
                                &device.bus,
                            ]
                            .into_iter()
                            .map(String::capacity)
                            .sum::<usize>()
                        })
                        + vector_bytes(&snapshot.pci_devices, |device| {
                            [
                                &device.slot,
                                &device.dev_type,
                                &device.brand,
                                &device.device,
                                &device.id,
                                &device.driver,
                            ]
                            .into_iter()
                            .map(String::capacity)
                            .sum::<usize>()
                        })
                        + vector_bytes(&snapshot.collection_issues, String::capacity)
                }
                Err(message) => message.capacity(),
            }
    });
    install_result_poller(receiver, on_done, Err);
}

/// Diagnostics scan result.
pub enum DiagnosticsScanOutcome {
    Completed {
        findings: Vec<maintainer_core::diagnostics::DiagnosticFinding>,
    },
    Failed {
        message: String,
    },
}

/// Launch a diagnostics scan through the shared `JobExecutor`.
pub fn dispatch_diagnostics_scan(
    executor: &Arc<JobExecutor>,
    on_progress: impl Fn(&maintainer_core::jobs::JobProgress) + 'static,
    on_done: impl FnOnce(DiagnosticsScanOutcome) + 'static,
) -> (CancellationToken, SharedProgress) {
    let token = CancellationToken::new();
    let token_for_worker = token.clone();
    let progress = SharedProgress::new();
    let _progress_for_worker = progress.clone();

    let job = move || -> maintainer_core::error::Result<DiagnosticsScanOutcome> {
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::diagnostics::{
            JournalCollector, JournalFilter, ServiceDiagnostics, group_journal_entries,
            interpret_groups,
        };

        let runner = SystemCommandRunner;
        let failed_services = match ServiceDiagnostics::list_failed(&runner, &token_for_worker) {
            maintainer_core::capability::Capability::Available(services) => services,
            _ => Vec::new(),
        };
        let filter = JournalFilter::default();
        let journal = JournalCollector::collect(&filter, &runner, &token_for_worker);

        let findings = match journal {
            maintainer_core::capability::Capability::Available(entries) => {
                let groups = group_journal_entries(&entries);
                interpret_groups(&groups, &failed_services)
            }
            _ => interpret_groups(&[], &failed_services),
        };

        Ok(DiagnosticsScanOutcome::Completed { findings })
    };

    let rx = executor.execute_sized(vec![job], token.clone(), diagnostics_bytes);
    install_progress_poller(rx, progress.clone(), on_progress, on_done, |message| {
        DiagnosticsScanOutcome::Failed { message }
    });

    (token, progress)
}

/// Result of refreshing SMART data through the privileged helper.
pub enum SmartRefreshOutcome {
    Loaded(Vec<maintainer_core::diagnostics::SmartDrive>),
    PermissionRequired,
    Unavailable(String),
}

/// Discover drives, refresh each fixed UDisks object through the helper, then reload data.
pub fn dispatch_smart_refresh(
    executor: &Arc<JobExecutor>,
    helper: crate::helper_client::SharedHelperClient,
    on_done: impl FnOnce(SmartRefreshOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let job = move || -> maintainer_core::error::Result<SmartRefreshOutcome> {
        use maintainer_core::capability::Capability;
        use maintainer_core::diagnostics::{RealUDisksClient, query_smart_drives};

        const DRIVE_PREFIX: &str = "/org/freedesktop/UDisks2/drives/";
        let client = RealUDisksClient::new();
        let drives = match query_smart_drives(&client) {
            Capability::Available(drives) => drives,
            Capability::Unavailable(reason) => {
                return Ok(SmartRefreshOutcome::Unavailable(reason.message));
            }
            Capability::PermissionRequired(_) => {
                return Ok(SmartRefreshOutcome::PermissionRequired);
            }
        };

        for drive in &drives {
            if drive.removable {
                continue;
            }
            let Some(drive_id) = drive.object_path.strip_prefix(DRIVE_PREFIX) else {
                continue;
            };
            match helper.refresh_smart(drive_id) {
                Capability::Available(_) => {}
                Capability::Unavailable(reason) => {
                    return Ok(SmartRefreshOutcome::Unavailable(reason.message));
                }
                Capability::PermissionRequired(_) => {
                    return Ok(SmartRefreshOutcome::PermissionRequired);
                }
            }
        }

        match query_smart_drives(&client) {
            Capability::Available(drives) => Ok(SmartRefreshOutcome::Loaded(drives)),
            Capability::Unavailable(reason) => Ok(SmartRefreshOutcome::Unavailable(reason.message)),
            Capability::PermissionRequired(_) => Ok(SmartRefreshOutcome::PermissionRequired),
        }
    };
    let receiver = executor.execute_sized(vec![job], token.clone(), smart_bytes);
    install_result_poller(receiver, on_done, SmartRefreshOutcome::Unavailable);
    token
}

fn diagnostics_bytes(value: &DiagnosticsScanOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            DiagnosticsScanOutcome::Completed { findings } => vector_bytes(findings, |f| {
                f.detected.capacity()
                    + vector_bytes(&f.evidence, |e| e.observation.capacity())
                    + vector_bytes(&f.possible_causes, String::capacity)
                    + f.recommendation.as_ref().map_or(0, String::capacity)
            }),
            DiagnosticsScanOutcome::Failed { message } => message.capacity(),
        }
}

fn smart_bytes(value: &SmartRefreshOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            SmartRefreshOutcome::Loaded(drives) => vector_bytes(drives, |d| {
                d.object_path.capacity()
                    + d.model.capacity()
                    + d.firmware.capacity()
                    + d.serial.capacity()
                    + vector_bytes(&d.attributes, |a| a.name.capacity())
            }),
            SmartRefreshOutcome::Unavailable(message) => message.capacity(),
            SmartRefreshOutcome::PermissionRequired => 0,
        }
}
