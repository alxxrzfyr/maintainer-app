//! Network detail page: per-interface rates, sparklines, adapter state,
//! WiFi signal/link details, IP/gateway/DNS, and an explicit speed test action.
//!
//! Shows each network interface with receive/transmit rates and cumulative
//! totals. Loopback is included but visually de-emphasized. When network data
//! is unavailable, a StatusView explains why.
//!
//! The speed test is never automatic — it requires explicit user confirmation
//! with provider/data-use disclosure before running. It uses a fixed HTTPS
//! endpoint and a cancellable worker, producing download/upload/latency results.
//!
//! Architecture: this module consumes typed models from `maintainer_core::metrics`
//! and dispatches cancellable jobs. It does NOT read `/proc`, `/sys`, `/etc`,
//! or spawn external commands directly.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow, Separator};

use crate::monitoring::NetworkViewModel;
use crate::ui::icons;
use maintainer_core::metrics::{SpeedTestResult, SpeedTestStage};

type SpeedTestService = dyn Fn(&Arc<AtomicBool>, &dyn Fn(SpeedTestStage)) -> Result<SpeedTestResult, String>
    + Send
    + Sync;

/// Speed test state phases.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SpeedTestPhase {
    /// Ready to start, no results yet.
    Idle,
    /// Awaiting user confirmation.
    Confirming,
    /// Test in progress.
    Running,
    /// Completed with results.
    Completed {
        download_mbps: String,
        upload_mbps: String,
        latency_ms: String,
    },
    /// Test failed with an error message.
    Failed(String),
    /// Test was cancelled by user.
    Cancelled,
}

/// The network detail page.
pub struct NetworkPage {
    root: ScrolledWindow,
    // Top live summary
    summary_rx_label: Label,
    summary_tx_label: Label,
    summary_link_label: Label,
    // Interface list
    interfaces_card: GtkBox,
    interfaces_box: GtkBox,
    // Connection details
    adapter_info_box: GtkBox,
    // Speed test panel
    speed_test_section: GtkBox,
    // Unavailable state
    unavailable_box: GtkBox,
    unavailable_label: Label,
}

impl NetworkPage {
    /// Create a new network detail page.
    pub fn new(executor: Arc<maintainer_core::jobs::JobExecutor>) -> Self {
        Self::with_speed_test(
            executor,
            Arc::new(|cancel, progress| {
                maintainer_core::metrics::run_speed_test(cancel, progress)
                    .map_err(|error| error.to_string())
            }),
        )
    }

    fn with_speed_test(
        executor: Arc<maintainer_core::jobs::JobExecutor>,
        service: Arc<SpeedTestService>,
    ) -> Self {
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.add_css_class("maintainer_page_content");
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);

        // Page title
        let title = Label::new(Some("Network"));
        title.add_css_class("maintainer_page_title");
        title.set_halign(Align::Start);
        content.append(&title);

        let subtitle = Label::new(Some(
            "Per-interface network activity, adapter state, and connection details.",
        ));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(Align::Start);
        subtitle.set_wrap(true);
        subtitle.set_margin_top(2);
        subtitle.set_margin_bottom(8);
        content.append(&subtitle);

        // --- Compact top live RX/TX/link summary ---
        let summary_row = GtkBox::new(Orientation::Horizontal, 16);
        summary_row.add_css_class("maintainer_detail_card");
        summary_row.set_margin_bottom(4);

        let net_icon = icons::icon_with_fallback(icons::chains::NETWORK_ACTIVITY, 16);
        net_icon.add_css_class("maintainer_text_muted");
        summary_row.append(&net_icon);

        let summary_rx_label = Label::new(Some("↓ —"));
        summary_rx_label.add_css_class("maintainer_net_rx");
        summary_row.append(&summary_rx_label);

        let summary_tx_label = Label::new(Some("↑ —"));
        summary_tx_label.add_css_class("maintainer_net_tx");
        summary_row.append(&summary_tx_label);

        let summary_link_label = Label::new(Some(""));
        summary_link_label.add_css_class("maintainer_text_muted");
        summary_link_label.set_hexpand(true);
        summary_link_label.set_halign(Align::End);
        summary_row.append(&summary_link_label);

        content.append(&summary_row);

        // --- Interfaces card ---
        let interfaces_card = GtkBox::new(Orientation::Vertical, 0);
        interfaces_card.add_css_class("maintainer_detail_card");

        let iface_title = Label::new(Some("Interfaces"));
        iface_title.add_css_class("maintainer_detail_card_title");
        iface_title.set_halign(Align::Start);
        interfaces_card.append(&iface_title);

        let interfaces_box = GtkBox::new(Orientation::Vertical, 0);
        interfaces_card.append(&interfaces_box);

        content.append(&interfaces_card);

        // --- Connection details card ---
        let adapter_info_box = GtkBox::new(Orientation::Vertical, 4);
        adapter_info_box.add_css_class("maintainer_detail_card");

        let adapter_title = Label::new(Some("Connection Details"));
        adapter_title.add_css_class("maintainer_detail_card_title");
        adapter_title.set_halign(Align::Start);
        adapter_info_box.append(&adapter_title);

        content.append(&adapter_info_box);

        // --- Speed test panel ---
        let speed_test_section = GtkBox::new(Orientation::Vertical, 8);
        speed_test_section.add_css_class("maintainer_detail_card");

        let speed_title = Label::new(Some("Internet Speed Test"));
        speed_title.add_css_class("maintainer_detail_card_title");
        speed_title.set_halign(Align::Start);
        speed_test_section.append(&speed_title);

        let speed_disclosure = Label::new(Some(
            "Provider: Cloudflare (speed.cloudflare.com). \
             Transfers ~10–25 MB. Your public IP is visible to the server.",
        ));
        speed_disclosure.add_css_class("maintainer_text_muted");
        speed_disclosure.set_halign(Align::Start);
        speed_disclosure.set_wrap(true);
        speed_test_section.append(&speed_disclosure);

        let speed_btn_row = GtkBox::new(Orientation::Horizontal, 8);
        speed_btn_row.set_margin_top(4);

        let speed_test_run_btn = Button::with_label("Run Speed Test");
        speed_test_run_btn.add_css_class("suggested-action");
        speed_test_run_btn
            .set_tooltip_text(Some("Download and upload data to measure connection speed"));
        speed_btn_row.append(&speed_test_run_btn);

        let speed_test_cancel_btn = Button::with_label("Cancel");
        speed_test_cancel_btn.set_visible(false);
        speed_test_cancel_btn.set_tooltip_text(Some("Cancel the running speed test"));
        speed_btn_row.append(&speed_test_cancel_btn);

        speed_test_section.append(&speed_btn_row);

        let speed_test_status = Label::new(None);
        speed_test_status.set_halign(Align::Start);
        speed_test_status.set_wrap(true);
        speed_test_status.set_visible(false);
        speed_test_section.append(&speed_test_status);

        // Structured result metrics area
        let speed_test_result_box = GtkBox::new(Orientation::Horizontal, 24);
        speed_test_result_box.set_visible(false);
        speed_test_result_box.set_margin_top(4);
        speed_test_section.append(&speed_test_result_box);

        content.append(&speed_test_section);

        // --- Unavailable state ---
        let unavailable_box = GtkBox::new(Orientation::Vertical, 8);
        unavailable_box.set_halign(Align::Center);
        unavailable_box.set_valign(Align::Center);
        unavailable_box.set_margin_top(32);
        unavailable_box.set_visible(false);

        let unavailable_icon = icons::icon_with_fallback(icons::chains::UNAVAILABLE, 48);
        unavailable_box.append(&unavailable_icon);

        let unavailable_label = Label::new(Some("Network data unavailable"));
        unavailable_label.add_css_class("maintainer_text_muted");
        unavailable_box.append(&unavailable_label);
        content.append(&unavailable_box);

        let root = ScrolledWindow::new();
        root.set_child(Some(&content));
        root.set_hexpand(true);
        root.set_vexpand(true);

        let speed_test_phase = Rc::new(RefCell::new(SpeedTestPhase::Idle));
        let speed_test_cancel_flag = Arc::new(AtomicBool::new(false));

        // Wire speed test button.
        {
            let phase = speed_test_phase.clone();
            let status = speed_test_status.clone();
            let result_box = speed_test_result_box.clone();
            let run_btn = speed_test_run_btn.clone();
            let cancel_btn = speed_test_cancel_btn.clone();
            let cancel_flag = speed_test_cancel_flag.clone();

            speed_test_run_btn.connect_clicked(move |_| {
                let current = phase.borrow().clone();
                match current {
                    SpeedTestPhase::Idle
                    | SpeedTestPhase::Completed { .. }
                    | SpeedTestPhase::Failed(_)
                    | SpeedTestPhase::Cancelled => {
                        // Show confirmation.
                        *phase.borrow_mut() = SpeedTestPhase::Confirming;
                        status.set_text("Press again to confirm and start the speed test.");
                        status.set_visible(true);
                        status.remove_css_class("maintainer_danger_text");
                        result_box.set_visible(false);
                        run_btn.set_label("Confirm & Run");
                    }
                    SpeedTestPhase::Confirming => {
                        // User confirmed — start the test.
                        *phase.borrow_mut() = SpeedTestPhase::Running;
                        cancel_flag.store(false, Ordering::SeqCst);
                        status.set_text("Measuring latency…");
                        status.set_visible(true);
                        status.remove_css_class("maintainer_danger_text");
                        status.add_css_class("maintainer_accent");
                        result_box.set_visible(false);
                        run_btn.set_sensitive(false);
                        cancel_btn.set_visible(true);

                        let phase_inner = phase.clone();
                        let status_inner = status.clone();
                        let result_box_inner = result_box.clone();
                        let run_btn_inner = run_btn.clone();
                        let cancel_btn_inner = cancel_btn.clone();
                        let cancel_flag_inner = cancel_flag.clone();

                        let service = service.clone();
                        let (stage_tx, stage_rx) = std::sync::mpsc::sync_channel(4);
                        let cancellation =
                            maintainer_core::jobs::CancellationToken::from_shared_flag(
                                cancel_flag_inner.clone(),
                            );
                        let rx = executor.execute_sized(
                            vec![move || {
                                Ok(service(&cancel_flag_inner, &|stage| {
                                    let _ = stage_tx.try_send(stage);
                                }))
                            }],
                            cancellation,
                            |result| {
                                std::mem::size_of_val(result)
                                    + result.as_ref().err().map_or(0, String::capacity)
                            },
                        );

                        // Poll for progress and result every 150ms from the main thread.
                        gtk4::glib::timeout_add_local(
                            std::time::Duration::from_millis(150),
                            move || {
                                // Update stage progress.
                                while let Ok(stage) = stage_rx.try_recv() {
                                    status_inner.set_text(&format!("{}…", stage.label()));
                                }

                                let result = rx.try_recv().map(|message| match message {
                                    maintainer_core::jobs::JobMessage::Completed(result) => result,
                                    maintainer_core::jobs::JobMessage::Failed(error) => {
                                        Err(error.to_string())
                                    }
                                    maintainer_core::jobs::JobMessage::Panicked(message) => {
                                        Err(message)
                                    }
                                });
                                match result {
                                    Ok(result) => {
                                        status_inner.remove_css_class("maintainer_accent");
                                        cancel_btn_inner.set_visible(false);
                                        run_btn_inner.set_sensitive(true);
                                        run_btn_inner.set_label("Run Speed Test");

                                        match result {
                                            Ok(r) => {
                                                let dl_text =
                                                    format!("{:.1} Mbps", r.download_mbps);
                                                let ul_text = format!("{:.1} Mbps", r.upload_mbps);
                                                let lat_text = format!("{:.0} ms", r.latency_ms);
                                                status_inner.set_visible(false);
                                                show_speed_results(
                                                    &result_box_inner,
                                                    &dl_text,
                                                    &ul_text,
                                                    &lat_text,
                                                );
                                                *phase_inner.borrow_mut() =
                                                    SpeedTestPhase::Completed {
                                                        download_mbps: dl_text,
                                                        upload_mbps: ul_text,
                                                        latency_ms: lat_text,
                                                    };
                                            }
                                            Err(ref e) if e.contains("cancelled") => {
                                                status_inner.set_text("Speed test cancelled.");
                                                status_inner.set_visible(true);
                                                *phase_inner.borrow_mut() =
                                                    SpeedTestPhase::Cancelled;
                                            }
                                            Err(e) => {
                                                status_inner.set_text(&format!("Failed: {e}"));
                                                status_inner
                                                    .add_css_class("maintainer_danger_text");
                                                status_inner.set_visible(true);
                                                *phase_inner.borrow_mut() =
                                                    SpeedTestPhase::Failed(e);
                                            }
                                        }
                                        gtk4::glib::ControlFlow::Break
                                    }
                                    Err(std::sync::mpsc::TryRecvError::Empty) => {
                                        gtk4::glib::ControlFlow::Continue
                                    }
                                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                                        status_inner.remove_css_class("maintainer_accent");
                                        cancel_btn_inner.set_visible(false);
                                        run_btn_inner.set_sensitive(true);
                                        run_btn_inner.set_label("Run Speed Test");
                                        status_inner
                                            .set_text("Speed test failed: worker disconnected");
                                        status_inner.set_visible(true);
                                        *phase_inner.borrow_mut() =
                                            SpeedTestPhase::Failed("worker disconnected".into());
                                        gtk4::glib::ControlFlow::Break
                                    }
                                }
                            },
                        );
                    }
                    _ => {}
                }
            });
        }

        // Wire cancel button — sets flag which terminates the child process.
        {
            let cancel_flag = speed_test_cancel_flag.clone();
            speed_test_cancel_btn.connect_clicked(move |_| {
                cancel_flag.store(true, Ordering::SeqCst);
            });
        }

        Self {
            root,
            summary_rx_label,
            summary_tx_label,
            summary_link_label,
            interfaces_card,
            interfaces_box,
            adapter_info_box,
            unavailable_box,
            unavailable_label,
            speed_test_section,
        }
    }

    /// Get the root widget for stack insertion.
    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }

    /// Update the page with new network view model data.
    pub fn update(&self, vm: &NetworkViewModel) {
        if !vm.available {
            self.interfaces_card.set_visible(false);
            self.adapter_info_box.set_visible(false);
            self.speed_test_section.set_visible(false);
            self.unavailable_box.set_visible(true);
            if let Some(reason) = &vm.unavailable_reason {
                self.unavailable_label.set_text(reason);
            } else {
                self.unavailable_label.set_text("Network data unavailable");
            }
            self.summary_rx_label.set_text("↓ —");
            self.summary_tx_label.set_text("↑ —");
            self.summary_link_label.set_text("");
            return;
        }

        self.unavailable_box.set_visible(false);
        self.interfaces_card.set_visible(true);
        self.adapter_info_box.set_visible(true);
        self.speed_test_section.set_visible(true);

        // Update top live summary
        self.summary_rx_label
            .set_text(&format!("↓ {}/s", format_bytes_rate(vm.total_rx_rate)));
        self.summary_tx_label
            .set_text(&format!("↑ {}/s", format_bytes_rate(vm.total_tx_rate)));

        let active_count = vm.interfaces.iter().filter(|i| !i.is_loopback).count();
        self.summary_link_label.set_text(&format!(
            "{active_count} interface{}",
            if active_count == 1 { "" } else { "s" }
        ));

        // Rebuild interface list
        while let Some(child) = self.interfaces_box.first_child() {
            self.interfaces_box.remove(&child);
        }

        for (idx, iface) in vm.interfaces.iter().enumerate() {
            if idx > 0 {
                let sep = Separator::new(Orientation::Horizontal);
                sep.add_css_class("maintainer_row_separator");
                self.interfaces_box.append(&sep);
            }

            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.set_margin_top(6);
            row.set_margin_bottom(6);
            row.set_margin_start(8);
            row.set_margin_end(8);

            let name_label = Label::new(Some(&iface.interface));
            name_label.set_halign(Align::Start);
            name_label.set_hexpand(true);
            if iface.is_loopback {
                name_label.add_css_class("maintainer_text_muted");
            }
            row.append(&name_label);

            let state_text = if iface.is_loopback { "lo" } else { "up" };
            let state_label = Label::new(Some(state_text));
            state_label.add_css_class("maintainer_text_muted");
            row.append(&state_label);

            let rx_text = format!(
                "↓ {}/s",
                format_bytes_rate(iface.rx_bytes_per_sec.unwrap_or(0.0))
            );
            let rx_label = Label::new(Some(&rx_text));
            rx_label.add_css_class("maintainer_net_rx");
            row.append(&rx_label);

            let tx_text = format!(
                "↑ {}/s",
                format_bytes_rate(iface.tx_bytes_per_sec.unwrap_or(0.0))
            );
            let tx_label = Label::new(Some(&tx_text));
            tx_label.add_css_class("maintainer_net_tx");
            row.append(&tx_label);

            self.interfaces_box.append(&row);
        }

        // Update connection details from the view model (populated by core).
        self.update_adapter_info(vm);
    }

    /// Populate connection details: IP, gateway, DNS, WiFi.
    /// Consumes typed `ConnectionDetails` from the view model — no direct system access.
    fn update_adapter_info(&self, vm: &NetworkViewModel) {
        // Remove all children after the title.
        if let Some(title) = self.adapter_info_box.first_child() {
            let mut sibling = title.next_sibling();
            while let Some(widget) = sibling {
                let next = widget.next_sibling();
                self.adapter_info_box.remove(&widget);
                sibling = next;
            }
        }

        if let Some(reason) = &vm.connection_unavailable_reason {
            let label = Label::new(Some(reason));
            label.set_wrap(true);
            label.set_halign(Align::Start);
            self.adapter_info_box.append(&label);
        }
        let details = &vm.connection_details;

        let add_detail_row = |label: &str, value: &str, parent: &GtkBox| {
            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            let key = Label::new(Some(label));
            key.set_halign(Align::Start);
            key.set_width_chars(12);
            key.add_css_class("maintainer_text_muted");
            row.append(&key);

            let val = Label::new(Some(value));
            val.set_halign(Align::Start);
            val.set_hexpand(true);
            val.set_selectable(true);
            row.append(&val);

            parent.append(&row);
        };

        let ip_text = details.ip_address.as_deref().unwrap_or("Unavailable");
        add_detail_row("IP Address", ip_text, &self.adapter_info_box);

        let gw_text = details.gateway.as_deref().unwrap_or("Unavailable");
        add_detail_row("Gateway", gw_text, &self.adapter_info_box);

        let dns_text = if details.dns_servers.is_empty() {
            "Unavailable".to_string()
        } else {
            details.dns_servers.join(", ")
        };
        add_detail_row("DNS", &dns_text, &self.adapter_info_box);

        if let Some(ref wifi) = details.wifi {
            self.adapter_info_box
                .append(&Separator::new(Orientation::Horizontal));
            let wifi_header = Label::new(Some("Wi-Fi"));
            wifi_header.add_css_class("maintainer_detail_card_title");
            wifi_header.set_halign(Align::Start);
            wifi_header.set_margin_top(4);
            self.adapter_info_box.append(&wifi_header);

            add_detail_row("SSID", &wifi.ssid, &self.adapter_info_box);
            add_detail_row("Signal", &wifi.signal, &self.adapter_info_box);
            add_detail_row("Link Speed", &wifi.link_speed, &self.adapter_info_box);
            add_detail_row("Frequency", &wifi.frequency, &self.adapter_info_box);
        }
    }
}

/// Render structured speed test results in the result box.
fn show_speed_results(result_box: &GtkBox, download: &str, upload: &str, latency: &str) {
    while let Some(child) = result_box.first_child() {
        result_box.remove(&child);
    }

    let metrics = [
        ("Download", download),
        ("Upload", upload),
        ("Latency", latency),
    ];

    for (label, value) in metrics {
        let metric = GtkBox::new(Orientation::Vertical, 2);
        let lbl = Label::new(Some(label));
        lbl.add_css_class("maintainer_text_muted");
        lbl.set_halign(Align::Start);
        metric.append(&lbl);

        let val = Label::new(Some(value));
        val.add_css_class("maintainer_speed_result_value");
        val.set_halign(Align::Start);
        metric.append(&val);

        result_box.append(&metric);
    }
    result_box.set_visible(true);
}

/// Format bytes per second as a human-readable string.
fn format_bytes_rate(bytes_per_sec: f64) -> String {
    if bytes_per_sec >= 1_073_741_824.0 {
        format!("{:.1} GiB", bytes_per_sec / 1_073_741_824.0)
    } else if bytes_per_sec >= 1_048_576.0 {
        format!("{:.1} MiB", bytes_per_sec / 1_048_576.0)
    } else if bytes_per_sec >= 1024.0 {
        format!("{:.1} KiB", bytes_per_sec / 1024.0)
    } else {
        format!("{:.0} B", bytes_per_sec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn descendants<T: IsA<gtk4::Widget> + Clone + 'static>(
        widget: &impl IsA<gtk4::Widget>,
    ) -> Vec<T> {
        let mut found = Vec::new();
        if let Ok(item) = widget.as_ref().clone().downcast::<T>() {
            found.push(item);
        }
        let mut child = widget.as_ref().first_child();
        while let Some(current) = child {
            found.extend(descendants::<T>(&current));
            child = current.next_sibling();
        }
        found
    }

    fn wait_until(mut ready: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while !ready() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for speed-test UI"
            );
            glib::MainContext::default().iteration(false);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }

    #[test]
    fn speed_test_requires_confirmation_runs_off_thread_and_renders_each_stage() {
        assert!(!gtk4::is_initialized() || gtk4::is_initialized_main_thread());
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        let main_thread = std::thread::current().id();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let recorded = calls.clone();
        let (advance, next_stage) = std::sync::mpsc::channel();
        let next_stage = std::sync::Mutex::new(next_stage);
        let executor = Arc::new(maintainer_core::jobs::JobExecutor::new(1).unwrap());
        let page = NetworkPage::with_speed_test(
            executor,
            Arc::new(move |cancel, progress| {
                assert_ne!(std::thread::current().id(), main_thread);
                recorded.fetch_add(1, Ordering::SeqCst);
                for stage in [
                    SpeedTestStage::Latency,
                    SpeedTestStage::Download,
                    SpeedTestStage::Upload,
                ] {
                    assert!(!cancel.load(Ordering::SeqCst));
                    progress(stage);
                    next_stage
                        .lock()
                        .unwrap()
                        .recv_timeout(std::time::Duration::from_secs(3))
                        .unwrap();
                }
                Ok(SpeedTestResult {
                    download_mbps: 125.0,
                    upload_mbps: 25.0,
                    latency_ms: 12.0,
                })
            }),
        );
        page.update(&NetworkViewModel {
            available: true,
            total_rx_rate: 2048.0,
            total_tx_rate: 1024.0,
            connection_unavailable_reason: Some("fixture connection unavailable".into()),
            ..Default::default()
        });
        let labels = descendants::<Label>(page.widget());
        assert!(labels.iter().any(|label| label.text() == "↓ 2.0 KiB/s"));
        assert!(
            labels
                .iter()
                .any(|label| label.text() == "fixture connection unavailable")
        );
        let buttons = descendants::<Button>(page.widget());
        let run = buttons
            .iter()
            .find(|button| button.label().as_deref() == Some("Run Speed Test"))
            .unwrap();
        let cancel = buttons
            .iter()
            .find(|button| button.label().as_deref() == Some("Cancel"))
            .unwrap();
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "construction/model rendering must not invoke a service"
        );
        run.emit_clicked();
        assert_eq!(run.label().as_deref(), Some("Confirm & Run"));
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "review must not start network work"
        );
        run.emit_clicked();
        assert!(!run.is_sensitive());
        assert!(cancel.is_visible());
        wait_until(|| calls.load(Ordering::SeqCst) == 1);
        for stage in [
            SpeedTestStage::Latency,
            SpeedTestStage::Download,
            SpeedTestStage::Upload,
        ] {
            let expected = format!("{}…", stage.label());
            wait_until(|| labels.iter().any(|label| label.text() == expected));
            advance.send(()).unwrap();
        }
        wait_until(|| run.is_sensitive());
        assert!(!cancel.is_visible());
        let labels = descendants::<Label>(page.widget());
        for value in ["125.0 Mbps", "25.0 Mbps", "12 ms"] {
            assert!(
                labels.iter().any(|label| label.text() == value),
                "missing result {value}"
            );
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn speed_test_surfaces_capability_failure_and_cancellation() {
        assert!(!gtk4::is_initialized() || gtk4::is_initialized_main_thread());
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        for cancellation in [false, true] {
            let executor = Arc::new(maintainer_core::jobs::JobExecutor::new(1).unwrap());
            let page = NetworkPage::with_speed_test(
                executor,
                Arc::new(move |cancel, _| {
                    if cancellation {
                        let deadline =
                            std::time::Instant::now() + std::time::Duration::from_secs(3);
                        while !cancel.load(Ordering::SeqCst) {
                            assert!(
                                std::time::Instant::now() < deadline,
                                "cancel never reached worker"
                            );
                            std::thread::sleep(std::time::Duration::from_millis(5));
                        }
                        Err("cancelled".into())
                    } else {
                        Err("curl is not installed".into())
                    }
                }),
            );
            let buttons = descendants::<Button>(page.widget());
            let run = buttons
                .iter()
                .find(|button| button.label().as_deref() == Some("Run Speed Test"))
                .unwrap();
            let cancel = buttons
                .iter()
                .find(|button| button.label().as_deref() == Some("Cancel"))
                .unwrap();
            run.emit_clicked();
            run.emit_clicked();
            if cancellation {
                cancel.emit_clicked();
            }
            wait_until(|| run.is_sensitive());
            assert!(!cancel.is_visible());
            assert_eq!(run.label().as_deref(), Some("Run Speed Test"));
            let expected = if cancellation {
                "Speed test cancelled."
            } else {
                "Failed: curl is not installed"
            };
            let labels = descendants::<Label>(page.widget());
            let status = labels
                .iter()
                .find(|label| label.text() == expected)
                .unwrap();
            assert!(status.is_visible());
            assert_eq!(
                status.has_css_class("maintainer_danger_text"),
                !cancellation
            );
            assert!(
                !labels
                    .iter()
                    .any(|label| label.has_css_class("maintainer_speed_result_value"))
            );
        }
    }
}
