//! Reports page: preview, copy, and save redacted system reports.
//!
//! Preview is mandatory before copy/save. Save writes through a temporary
//! sibling file with mode 0600, flush, then atomic rename.
//! No upload/share endpoint is provided.

use std::cell::RefCell;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::OpenOptionsExt;
use std::path::Path;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, FileChooserAction, FileChooserDialog, Label, Orientation,
    ResponseType, ScrolledWindow, TextView, WrapMode,
};

use maintainer_core::reports::model::{
    CapabilityNote, DiagnosticsSection, HardwareSection, HealthSection, MaintenanceSection,
    OsSection, RedactionPolicy, SoftwareUpdatesSection, SystemReport,
};
use maintainer_core::reports::{redact_report, render_text};

/// State for the reports page.
struct ReportsState {
    /// The rendered report text, available after preview.
    preview_text: Option<String>,
}

/// Build the reports page widget tree.
pub fn build_reports_page() -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 12);
    container.set_margin_top(16);
    container.set_margin_bottom(16);
    container.set_margin_start(16);
    container.set_margin_end(16);

    let state = Rc::new(RefCell::new(ReportsState { preview_text: None }));

    let header = Label::new(Some("System Report"));
    header.add_css_class("heading");
    header.set_halign(Align::Start);
    container.append(&header);

    let description = Label::new(Some(
        "Generate a privacy-conscious snapshot of your system state. \
         Sensitive data is redacted by default.",
    ));
    description.set_halign(Align::Start);
    description.set_wrap(true);
    container.append(&description);

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(WrapMode::Word);
    text_view.set_vexpand(true);

    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&text_view));
    scrolled.set_vexpand(true);
    scrolled.set_min_content_height(300);
    container.append(&scrolled);

    let button_bar = GtkBox::new(Orientation::Horizontal, 8);
    button_bar.set_halign(Align::End);

    let generate_btn = Button::with_label("Generate Preview");
    let copy_btn = Button::with_label("Copy to Clipboard");
    let save_btn = Button::with_label("Save to File");

    copy_btn.set_sensitive(false);
    save_btn.set_sensitive(false);

    button_bar.append(&generate_btn);
    button_bar.append(&copy_btn);
    button_bar.append(&save_btn);
    container.append(&button_bar);

    {
        let state_clone = state.clone();
        let text_view_clone = text_view.clone();
        let copy_btn_clone = copy_btn.clone();
        let save_btn_clone = save_btn.clone();
        generate_btn.connect_clicked(move |_| {
            let report = assemble_placeholder_report();
            let policy = RedactionPolicy::default();
            let redacted = redact_report(report, &policy);
            let text = render_text(&redacted);

            let buffer = text_view_clone.buffer();
            buffer.set_text(&text);

            let mut st = state_clone.borrow_mut();
            st.preview_text = Some(text);

            copy_btn_clone.set_sensitive(true);
            save_btn_clone.set_sensitive(true);
        });
    }

    {
        let state_clone = state.clone();
        let text_view_clone = text_view.clone();
        copy_btn.connect_clicked(move |btn| {
            let st = state_clone.borrow();
            if let Some(ref text) = st.preview_text {
                let clipboard = gtk4::prelude::WidgetExt::clipboard(&text_view_clone);
                clipboard.set_text(text);
                btn.set_label("Copied!");
                let btn_clone = btn.clone();
                glib::timeout_add_local_once(std::time::Duration::from_secs(2), move || {
                    btn_clone.set_label("Copy to Clipboard");
                });
            }
        });
    }

    {
        let state_clone = state.clone();
        save_btn.connect_clicked(move |btn| {
            let st = state_clone.borrow();
            if let Some(ref text) = st.preview_text {
                let text_owned = text.clone();
                let btn_clone = btn.clone();
                let dialog = FileChooserDialog::new(
                    Some("Save Report"),
                    gtk4::Window::NONE,
                    FileChooserAction::Save,
                    &[
                        ("Cancel", ResponseType::Cancel),
                        ("Save", ResponseType::Accept),
                    ],
                );
                dialog.set_current_name("maintainer-report.txt");

                dialog.connect_response(move |dialog, response| {
                    if response == ResponseType::Accept {
                        if let Some(file) = dialog.file() {
                            if let Some(path) = file.path() {
                                match atomic_save(&path, &text_owned) {
                                    Ok(()) => {
                                        btn_clone.set_label("Saved!");
                                        let btn_restore = btn_clone.clone();
                                        glib::timeout_add_local_once(
                                            std::time::Duration::from_secs(2),
                                            move || {
                                                btn_restore.set_label("Save to File");
                                            },
                                        );
                                    }
                                    Err(e) => {
                                        eprintln!("Failed to save report: {e}");
                                    }
                                }
                            }
                        }
                    }
                    dialog.close();
                });

                dialog.show();
            }
        });
    }

    container
}

/// Write report to file atomically: temp sibling with mode 0600, flush, rename.
fn atomic_save(path: &Path, content: &str) -> std::io::Result<()> {
    let parent = path.parent().unwrap_or(Path::new("."));
    let file_name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| "report".to_string());
    let tmp_name = format!(".{file_name}.tmp");
    let tmp_path = parent.join(&tmp_name);

    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp_path)?;

    file.write_all(content.as_bytes())?;
    file.flush()?;
    file.sync_all()?;

    fs::rename(&tmp_path, path)?;
    Ok(())
}

/// Assemble a placeholder report from available system state.
/// In a full implementation this would consume the latest cached snapshot.
/// Report assembly does NOT trigger expensive operations.
fn assemble_placeholder_report() -> SystemReport {
    use maintainer_core::capability::{Capability, UnavailableReason};

    let hostname = hostname_or_redacted();
    let username = username_or_redacted();

    SystemReport {
        generated_at: chrono_like_now(),
        os: OsSection {
            distro: read_os_release().unwrap_or_else(|| "Unknown".to_string()),
            kernel: read_kernel_version().unwrap_or_else(|| "Unknown".to_string()),
            uptime_seconds: read_uptime().unwrap_or(0),
            desktop: std::env::var("XDG_CURRENT_DESKTOP").ok(),
            hostname,
            username,
        },
        hardware: HardwareSection {
            cpu: Capability::Unavailable(UnavailableReason {
                code: "not-sampled",
                message: "No live metric snapshot available".into(),
                remediation: None,
            }),
            memory: Capability::Unavailable(UnavailableReason {
                code: "not-sampled",
                message: "No live metric snapshot available".into(),
                remediation: None,
            }),
            disks: Capability::Unavailable(UnavailableReason {
                code: "not-sampled",
                message: "No live metric snapshot available".into(),
                remediation: None,
            }),
            network: Capability::Unavailable(UnavailableReason {
                code: "not-sampled",
                message: "No live metric snapshot available".into(),
                remediation: None,
            }),
            temperatures: Capability::Unavailable(UnavailableReason {
                code: "not-sampled",
                message: "No live metric snapshot available".into(),
                remediation: None,
            }),
        },
        health: HealthSection {
            smart_drives: Capability::Unavailable(UnavailableReason {
                code: "not-queried",
                message: "SMART data not queried for report".into(),
                remediation: None,
            }),
        },
        software_updates: SoftwareUpdatesSection {
            pending_updates: Capability::Unavailable(UnavailableReason {
                code: "not-checked",
                message: "Update check not performed for report".into(),
                remediation: None,
            }),
        },
        diagnostics: DiagnosticsSection {
            failed_services: Capability::Unavailable(UnavailableReason {
                code: "not-queried",
                message: "Service diagnostics not queried for report".into(),
                remediation: None,
            }),
            event_groups: Capability::Unavailable(UnavailableReason {
                code: "not-queried",
                message: "Journal events not queried for report".into(),
                remediation: None,
            }),
        },
        maintenance: MaintenanceSection {
            recent_history: Vec::new(),
        },
        capability_notes: vec![CapabilityNote {
            name: "Live metrics".into(),
            status: "not sampled".into(),
            reason: Some("Report does not trigger live sampling".into()),
        }],
    }
}

/// Read hostname from /proc/sys/kernel/hostname.
fn hostname_or_redacted() -> String {
    fs::read_to_string("/proc/sys/kernel/hostname")
        .map(|s| s.trim().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Read username from environment.
fn username_or_redacted() -> String {
    std::env::var("USER").unwrap_or_else(|_| "unknown".to_string())
}

/// Generate a timestamp string.
fn chrono_like_now() -> String {
    use std::time::SystemTime;
    let duration = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap_or_default();
    let secs = duration.as_secs();
    format!("{secs} (Unix timestamp)")
}

/// Read /etc/os-release for PRETTY_NAME.
fn read_os_release() -> Option<String> {
    let content = fs::read_to_string("/etc/os-release").ok()?;
    for line in content.lines() {
        if let Some(value) = line.strip_prefix("PRETTY_NAME=") {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// Read kernel version from /proc/version.
fn read_kernel_version() -> Option<String> {
    let content = fs::read_to_string("/proc/version").ok()?;
    content.split_whitespace().nth(2).map(|s| s.to_string())
}

/// Read uptime from /proc/uptime.
fn read_uptime() -> Option<u64> {
    let content = fs::read_to_string("/proc/uptime").ok()?;
    let seconds_str = content.split_whitespace().next()?;
    seconds_str.parse::<f64>().ok().map(|s| s as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_save_creates_file_with_content() {
        let dir = std::env::temp_dir();
        let path = dir.join("maintainer_test_report.txt");
        let content = "Test report content\nLine 2\n";

        let result = atomic_save(&path, content);
        assert!(result.is_ok(), "atomic_save failed: {:?}", result.err());

        let read_back = fs::read_to_string(&path).unwrap();
        assert_eq!(read_back, content);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn atomic_save_file_permissions() {
        use std::os::unix::fs::MetadataExt;

        let dir = std::env::temp_dir();
        let path = dir.join("maintainer_test_report_perms.txt");
        let content = "secret report";

        let _ = atomic_save(&path, content);

        let metadata = fs::metadata(&path).unwrap();
        let mode = metadata.mode() & 0o777;
        assert_eq!(mode, 0o600, "file should have mode 0600, got {:o}", mode);

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn atomic_save_overwrites_existing() {
        let dir = std::env::temp_dir();
        let path = dir.join("maintainer_test_overwrite.txt");

        let _ = atomic_save(&path, "first");
        let _ = atomic_save(&path, "second");

        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content, "second");

        let _ = fs::remove_file(&path);
    }

    #[test]
    fn placeholder_report_assembles_without_panic() {
        let report = assemble_placeholder_report();
        let policy = RedactionPolicy::default();
        let redacted = redact_report(report, &policy);
        let text = render_text(&redacted);
        assert!(text.contains("Maintainer System Report"));
        assert!(text.contains("[hostname]"));
        assert!(text.contains("[username]"));
    }
}
