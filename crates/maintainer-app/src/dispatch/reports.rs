//! Redacted report generation.

use std::sync::Arc;
use std::time::Duration;

use maintainer_core::jobs::{CancellationToken, JobExecutor};

use super::install_result_poller;

/// Reports generation result.
pub enum ReportOutcome {
    Completed { text: String },
    Failed { message: String },
}

/// Launch report generation through the shared `JobExecutor`.
pub fn dispatch_report_generation(
    executor: &Arc<JobExecutor>,
    database: Option<Arc<maintainer_core::persistence::database::Database>>,
    on_done: impl FnOnce(ReportOutcome) + 'static,
) -> CancellationToken {
    let token = CancellationToken::new();
    let worker_token = token.clone();

    let job = move || -> maintainer_core::error::Result<ReportOutcome> {
        use std::path::Path;
        use std::time::Instant;

        use maintainer_core::capability::{Capability, UnavailableReason};
        use maintainer_core::command::SystemCommandRunner;
        use maintainer_core::diagnostics::{
            JournalCollector, JournalFilter, RealUDisksClient, ServiceDiagnostics,
            group_journal_entries, query_smart_drives,
        };
        use maintainer_core::metrics::disk::read_mounts_with_capacity;
        use maintainer_core::metrics::{
            NetworkSnapshot, compute_cpu_snapshot, read_cpu_sample, read_memory, read_net_dev,
            read_temperatures,
        };
        use maintainer_core::persistence::history::{HistoryRepository, SqliteHistoryRepository};
        use maintainer_core::reports::model::{
            DiagnosticsSection, HardwareSection, HealthSection, MaintenanceSection,
            RedactionPolicy, SoftwareUpdatesSection, SystemReport, UpdateInfo,
        };
        use maintainer_core::reports::{redact_report, render_text};
        use maintainer_core::updates::{AptDiscovery, DiscoveryResult};

        fn unavailable<T>(code: &'static str, message: String) -> Capability<T> {
            Capability::Unavailable(UnavailableReason {
                code,
                message,
                remediation: None,
            })
        }

        let proc_path = Path::new("/proc");
        let sys_path = Path::new("/sys");
        let first_cpu = read_cpu_sample(proc_path, Instant::now());
        std::thread::sleep(Duration::from_millis(100));
        let cpu = match (first_cpu, read_cpu_sample(proc_path, Instant::now())) {
            (Ok(first), Ok(second)) => Capability::Available(compute_cpu_snapshot(&first, &second)),
            (Err(error), _) | (_, Err(error)) => unavailable("proc-stat-read", error.to_string()),
        };
        let memory = read_memory(proc_path)
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("meminfo-read", error.to_string()));
        let disks = read_mounts_with_capacity(proc_path)
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("mounts-read", error.to_string()));
        let network = read_net_dev(proc_path)
            .map(|counters| {
                counters
                    .into_iter()
                    .map(|(interface, value)| NetworkSnapshot {
                        is_loopback: interface == "lo",
                        interface,
                        rx_bytes: value.rx_bytes,
                        tx_bytes: value.tx_bytes,
                        rx_packets: value.rx_packets,
                        tx_packets: value.tx_packets,
                        rx_errors: value.rx_errors,
                        tx_errors: value.tx_errors,
                        rx_bytes_per_sec: None,
                        tx_bytes_per_sec: None,
                    })
                    .collect()
            })
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("net-dev-read", error.to_string()));
        let temperatures = read_temperatures(sys_path)
            .map(Capability::Available)
            .unwrap_or_else(|error| unavailable("hwmon-read", error.to_string()));
        let smart_drives = query_smart_drives(&RealUDisksClient::new());

        let runner = SystemCommandRunner;
        let failed_services = ServiceDiagnostics::list_failed(&runner, &worker_token);
        let event_groups =
            match JournalCollector::collect(&JournalFilter::default(), &runner, &worker_token) {
                Capability::Available(entries) => {
                    Capability::Available(group_journal_entries(&entries))
                }
                Capability::Unavailable(reason) => Capability::Unavailable(reason),
                Capability::PermissionRequired(request) => Capability::PermissionRequired(request),
            };
        // Same discovery mode as the Updates page, so the count shown here
        // matches what a user would see there rather than under-reporting.
        let pending_updates = match AptDiscovery::check_full_upgrade(&runner, &worker_token) {
            Capability::Available(DiscoveryResult::Available(plan)) => Capability::Available(
                plan.candidates
                    .into_iter()
                    .map(|candidate| UpdateInfo {
                        package_name: candidate.name,
                        current_version: candidate
                            .installed_version
                            .unwrap_or_else(|| "not installed".to_string()),
                        available_version: candidate.available_version,
                    })
                    .collect(),
            ),
            Capability::Available(DiscoveryResult::UpToDate) => Capability::Available(Vec::new()),
            Capability::Available(DiscoveryResult::Failed(error)) => {
                unavailable("apt-check-failed", error.to_string())
            }
            Capability::Unavailable(reason) => Capability::Unavailable(reason),
            Capability::PermissionRequired(request) => Capability::PermissionRequired(request),
        };
        let recent_history = database
            .as_deref()
            .and_then(|database| SqliteHistoryRepository::new(database).recent(20).ok())
            .unwrap_or_default();

        let report = SystemReport {
            generated_at: chrono_like_now(),
            os: build_os_section(),
            hardware: HardwareSection {
                cpu,
                memory,
                disks,
                network,
                temperatures,
            },
            health: HealthSection { smart_drives },
            software_updates: SoftwareUpdatesSection { pending_updates },
            diagnostics: DiagnosticsSection {
                failed_services,
                event_groups,
            },
            maintenance: MaintenanceSection { recent_history },
            capability_notes: Vec::new(),
        };
        let text = render_text(&redact_report(report, &RedactionPolicy::default()));
        Ok(ReportOutcome::Completed { text })
    };

    let receiver = executor.execute_sized(vec![job], token.clone(), report_bytes);
    install_result_poller(receiver, on_done, |message| ReportOutcome::Failed {
        message,
    });
    token
}

pub(super) fn report_bytes(value: &ReportOutcome) -> usize {
    std::mem::size_of_val(value)
        + match value {
            ReportOutcome::Completed { text } => text.capacity(),
            ReportOutcome::Failed { message } => message.capacity(),
        }
}

fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    format_unix_time_utc(duration.as_secs())
}

fn format_unix_time_utc(secs: u64) -> String {
    let mut days = (secs / 86400) as i64;
    let rem_secs = secs % 86400;
    let hours = rem_secs / 3600;
    let minutes = (rem_secs % 3600) / 60;
    let seconds = rem_secs % 60;

    days += 719468;
    let era = if days >= 0 { days } else { days - 146096 } / 146097;
    let doe = (days - era * 146097) as u32;
    let yoe = (doe - doe / 1024 + doe / 1461 - doe / 142401) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };

    format!("{y:04}-{m:02}-{d:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}

fn build_os_section() -> maintainer_core::reports::model::OsSection {
    use std::fs;

    let distro = fs::read_to_string("/etc/os-release")
        .ok()
        .and_then(|content| {
            content
                .lines()
                .find(|l| l.starts_with("PRETTY_NAME="))
                .map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
        })
        .unwrap_or_else(|| "Unknown".to_string());

    let kernel = fs::read_to_string("/proc/version")
        .unwrap_or_default()
        .split_whitespace()
        .nth(2)
        .unwrap_or("unknown")
        .to_string();

    let uptime_seconds = fs::read_to_string("/proc/uptime")
        .ok()
        .and_then(|c| {
            c.split_whitespace()
                .next()
                .and_then(|s| s.parse::<f64>().ok())
        })
        .map(|s| s as u64)
        .unwrap_or(0);

    let hostname = fs::read_to_string("/etc/hostname")
        .unwrap_or_default()
        .trim()
        .to_string();

    let username = std::env::var("USER").unwrap_or_else(|_| "unknown".to_string());
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();

    maintainer_core::reports::model::OsSection {
        distro,
        kernel,
        uptime_seconds,
        desktop,
        hostname,
        username,
    }
}
