//! GTK presentation of the independent package owner's reviewed transaction.

use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use gtk4::prelude::*;
use maintainer_core::jobs::{Cancellation, CancellationToken, JobExecutor, JobMessage};
use maintainer_core::package_transaction::{
    MAX_RECEIPT_BYTES, PackagePhase, PackageReceipt, new_transaction_id,
};

use crate::package_client::PackageClient;
use crate::ui::updates::UpdatesPage;

enum Control {
    Approve(String),
    Cancel,
}

type Snapshot = Result<(PackageReceipt, bool), String>;

#[derive(Clone)]
pub struct PackageWorkflow {
    client: PackageClient,
    executor: Arc<JobExecutor>,
    active: Rc<Cell<bool>>,
    recovery_id: Rc<RefCell<Option<String>>>,
    control: Rc<RefCell<Option<mpsc::SyncSender<Control>>>>,
    page: UpdatesPage,
}

impl PackageWorkflow {
    pub fn new(page: &UpdatesPage, executor: Arc<JobExecutor>) -> Self {
        let workflow = Self {
            client: PackageClient::default(),
            executor,
            active: Rc::new(Cell::new(false)),
            recovery_id: Rc::new(RefCell::new(None)),
            control: Rc::new(RefCell::new(None)),
            page: page.clone(),
        };
        let cancel = workflow.clone();
        page.cancel_button().connect_clicked(move |_| {
            cancel.send(Control::Cancel);
            cancel.page.set_progress(
                0,
                None,
                "Cancellation requested; waiting for the package owner…",
                false,
            );
        });
        let resume = workflow.clone();
        page.resume_button()
            .connect_clicked(move |_| resume.start(None, None));
        let recheck = workflow.clone();
        page.recheck_button().connect_clicked(move |_| {
            let id = recheck.recovery_id.borrow().clone();
            if let Some(id) = id {
                recheck.start(None, Some(id));
            }
        });
        workflow.wire_kernels();
        workflow
    }

    fn wire_kernels(&self) {
        let kernels = self.page.kernels();
        let refresh = kernels.borrow().refresh_button().clone();
        let workflow = self.clone();
        refresh.connect_clicked(move |_| {
            let kernels = workflow.page.kernels();
            if kernels.borrow().state() == crate::ui::kernel_section::KernelSectionState::Loading {
                return;
            }
            kernels.borrow_mut().set_loading();
            let cancellation = CancellationToken::new();
            let worker_cancel = cancellation.clone();
            let receiver = workflow.executor.execute_sized(
                vec![move || {
                    maintainer_core::kernel::list_kernels(
                        &maintainer_core::command::SystemCommandRunner,
                        &worker_cancel,
                    )
                    .map_err(|_| {
                        maintainer_core::error::MaintainerError::invalid_data(
                            "kernel inspection",
                            "Installed kernels are unavailable on this system.",
                        )
                    })
                }],
                cancellation,
                |items| {
                    items
                        .iter()
                        .map(|item| {
                            std::mem::size_of_val(item)
                                + item.package_name.len()
                                + item.version.len()
                        })
                        .sum()
                },
            );
            glib::timeout_add_local(Duration::from_millis(150), move || {
                match receiver.try_recv() {
                    Ok(JobMessage::Completed(items)) => kernels.borrow_mut().set_loaded(&items),
                    Ok(JobMessage::Failed(error)) => {
                        kernels.borrow_mut().set_unavailable(&error.to_string())
                    }
                    Ok(JobMessage::Panicked(_)) => kernels
                        .borrow_mut()
                        .set_failed("Kernel inspection stopped."),
                    Err(mpsc::TryRecvError::Empty) => return glib::ControlFlow::Continue,
                    Err(mpsc::TryRecvError::Disconnected) => kernels
                        .borrow_mut()
                        .set_failed("Kernel inspection unavailable."),
                }
                glib::ControlFlow::Break
            });
        });
        let kernels_for_expand = self.page.kernels();
        self.page
            .kernel_expander()
            .connect_expanded_notify(move |expander| {
                if expander.is_expanded()
                    && kernels_for_expand.borrow().state()
                        == crate::ui::kernel_section::KernelSectionState::Idle
                {
                    refresh.emit_clicked();
                }
            });
        let workflow = self.clone();
        self.page
            .kernels()
            .borrow()
            .connect_remove(move |package| workflow.begin("kernel", vec![package]));
    }

    fn send(&self, command: Control) {
        if let Some(sender) = self.control.borrow().as_ref() {
            if sender.try_send(command).is_err() {
                self.page.set_progress(
                    0,
                    None,
                    "Control request could not be sent. Resume to check the operation.",
                    false,
                );
            }
        }
    }

    pub fn begin(&self, intent: &str, requests: Vec<String>) {
        self.start(Some((intent.to_owned(), requests)), None);
    }

    fn start(&self, request: Option<(String, Vec<String>)>, reconcile: Option<String>) {
        if self.active.replace(true) {
            return;
        }
        let mut page = self.page.clone();
        page.set_executing();
        page.recheck_button().set_visible(false);
        page.resume_button().set_sensitive(false);
        page.set_progress(
            0,
            None,
            "Contacting the package owner; administrator permission may be required…",
            false,
        );
        let (sender, commands) = mpsc::sync_channel(4);
        *self.control.borrow_mut() = Some(sender);
        let latest: Arc<Mutex<Option<Snapshot>>> = Arc::new(Mutex::new(None));
        let output = latest.clone();
        let client = self.client.clone();
        let token = CancellationToken::new();
        let worker_token = token.clone();
        let finished = Arc::new(AtomicBool::new(false));
        let worker_finished = finished.clone();
        let receiver = self.executor.execute_sized(
            vec![move || {
                let run = || -> Result<(), String> {
                    let id = if let Some(id) = reconcile {
                        client.reconcile(&id)?;
                        id
                    } else {
                        match request {
                            Some((intent, requests)) => {
                                let id = new_transaction_id();
                                let returned = client.begin(&intent, &requests, &id)?;
                                if returned != id {
                                    return Err(
                                        "Package owner returned a different transaction.".into()
                                    );
                                }
                                id
                            }
                            None => choose_recent(&client)?,
                        }
                    };
                    let mut approved = false;
                    loop {
                        if worker_token.is_cancelled() {
                            if !approved {
                                client.cancel(&id).map_err(|_| "The pending review could not be cancelled. Use Resume Package Operation to check its expiry.")?;
                            }
                            break;
                        }
                        match commands.try_recv() {
                            Ok(Control::Approve(hash)) => {
                                client.approve(&id, &hash)?;
                                approved = true;
                            }
                            Ok(Control::Cancel) => {
                                client.cancel(&id)?;
                            }
                            Err(mpsc::TryRecvError::Disconnected) => {
                                if !approved {
                                    client.cancel(&id).map_err(|_| "The pending review could not be cancelled. Use Resume Package Operation to check its expiry.")?;
                                }
                                break;
                            }
                            Err(mpsc::TryRecvError::Empty) => {}
                        }
                        let status = client.status(&id)?;
                        // Resumed fetching/mutation is already owned independently;
                        // stopping this observer must never cancel that work.
                        approved |= matches!(
                            status.0.phase,
                            PackagePhase::Fetching | PackagePhase::Mutating
                        );
                        let terminal = status.0.phase.is_terminal();
                        if let Ok(mut slot) = output.lock() {
                            *slot = Some(Ok(status));
                        }
                        if terminal {
                            break;
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                    Ok(())
                };
                if let Err(error) = run() {
                    if let Ok(mut slot) = output.lock() {
                        *slot = Some(Err(error));
                    }
                }
                worker_finished.store(true, Ordering::Release);
                Ok(())
            }],
            token.clone(),
            |_| MAX_RECEIPT_BYTES,
        );
        let workflow = self.clone();
        let shown = Rc::new(RefCell::new(None::<String>));
        let dialog = Rc::new(RefCell::new(None::<gtk4::Dialog>));
        glib::timeout_add_local(Duration::from_millis(150), move || {
            if workflow.page.widget().root().is_none() {
                token.cancel();
            }
            if let Ok(mut slot) = latest.lock() {
                if let Some(snapshot) = slot.take() {
                    match snapshot {
                        Ok((receipt, can_cancel)) => {
                            let can_cancel = can_cancel && receipt.phase.can_cancel();
                            if receipt.phase == PackagePhase::RecoveryRequired {
                                *workflow.recovery_id.borrow_mut() = Some(receipt.id.clone());
                                workflow.page.recheck_button().set_visible(true);
                            }
                            if receipt.phase == PackagePhase::Review {
                                workflow.page.set_progress(
                                    0,
                                    None,
                                    "Review the complete locked package plan before applying it.",
                                    can_cancel,
                                );
                                if shown.borrow().as_ref() != receipt.fingerprint.as_ref() {
                                    *shown.borrow_mut() = receipt.fingerprint.clone();
                                    *dialog.borrow_mut() =
                                        Some(workflow.review_dialog(&receipt, can_cancel));
                                }
                            } else {
                                if let Some(open) = dialog.borrow_mut().take() {
                                    open.close();
                                }
                                let message = phase_message(&receipt);
                                if receipt.phase.is_terminal() {
                                    let mut page = workflow.page.clone();
                                    page.set_result(
                                        receipt.phase == PackagePhase::Succeeded,
                                        &message,
                                        receipt.phase == PackagePhase::Succeeded
                                            && receipt.changes.iter().any(|change| {
                                                change.name.starts_with("linux-image-")
                                                    && change.new_version.is_some()
                                            }),
                                    );
                                } else {
                                    workflow.page.set_progress(0, None, &message, can_cancel);
                                }
                            }
                        }
                        Err(message) => {
                            let mut page = workflow.page.clone();
                            page.set_result(false, &message, false);
                        }
                    }
                }
            }
            match receiver.try_recv() {
                Ok(JobMessage::Failed(error)) => {
                    let mut page = workflow.page.clone();
                    page.set_result(false, &error.to_string(), false);
                    finished.store(true, Ordering::Release);
                }
                Ok(JobMessage::Panicked(_)) => {
                    let mut page = workflow.page.clone();
                    page.set_result(false, "Package observation stopped. Use Resume Package Operation to check the outcome.", false);
                    finished.store(true, Ordering::Release);
                }
                _ => {}
            }
            if finished.load(Ordering::Acquire) {
                // Process the last coalesced receipt before dropping the observer.
                if latest.lock().is_ok_and(|slot| slot.is_some()) {
                    return glib::ControlFlow::Continue;
                }
                workflow.active.set(false);
                workflow.control.borrow_mut().take();
                workflow.page.resume_button().set_sensitive(true);
                if let Some(open) = dialog.borrow_mut().take() {
                    open.close();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
    }

    fn review_dialog(&self, receipt: &PackageReceipt, can_apply: bool) -> gtk4::Dialog {
        let dialog = gtk4::Dialog::builder()
            .title("Review Package Changes")
            .modal(true)
            .default_width(680)
            .build();
        if let Some(window) = self.page.widget().root().and_downcast::<gtk4::Window>() {
            dialog.set_transient_for(Some(&window));
        }
        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        dialog.add_button("Apply", gtk4::ResponseType::Accept);
        dialog.set_default_response(gtk4::ResponseType::Cancel);
        if !can_apply {
            dialog.set_response_sensitive(gtk4::ResponseType::Accept, false);
        }
        let content = dialog.content_area();
        content.set_spacing(12);
        let summary = gtk4::Label::new(Some(&format!(
            "{} package operations · {} bytes to download\nThe package owner holds this exact plan. Existing local configuration files are preserved.",
            receipt.changes.len(),
            receipt.download_bytes
        )));
        summary.set_wrap(true);
        content.append(&summary);
        if !can_apply {
            let notice = gtk4::Label::new(Some(
                "This pending review belongs to another session and cannot be approved here. Wait for it to finish or expire.",
            ));
            notice.set_wrap(true);
            content.append(&notice);
        }
        let text = gtk4::TextView::builder()
            .editable(false)
            .cursor_visible(false)
            .monospace(true)
            .build();
        text.buffer().set_text(&review_text(receipt));
        let scroll = gtk4::ScrolledWindow::builder()
            .min_content_height(260)
            .max_content_height(360)
            .vexpand(true)
            .child(&text)
            .build();
        content.append(&scroll);
        let acknowledged = Rc::new(Cell::new(can_apply && !stronger_confirmation(receipt)));
        if stronger_confirmation(receipt) {
            let confirmation =
                gtk4::CheckButton::with_label("I reviewed every removal and downgrade above.");
            dialog.set_response_sensitive(gtk4::ResponseType::Accept, false);
            confirmation.set_sensitive(can_apply);
            let target = dialog.downgrade();
            let acknowledged = acknowledged.clone();
            confirmation.connect_toggled(move |check| {
                acknowledged.set(can_apply && check.is_active());
                if let Some(dialog) = target.upgrade() {
                    dialog.set_response_sensitive(
                        gtk4::ResponseType::Accept,
                        can_apply && check.is_active(),
                    );
                }
            });
            content.append(&confirmation);
        }
        let workflow = self.clone();
        let fingerprint = receipt.fingerprint.clone();
        let responded = Cell::new(false);
        dialog.connect_response(move |dialog, response| {
            if response == gtk4::ResponseType::Accept && !acknowledged.get() {
                return;
            }
            if responded.replace(true) {
                return;
            }
            if response == gtk4::ResponseType::Accept {
                if let Some(hash) = fingerprint.as_ref() {
                    workflow.send(Control::Approve(hash.clone()));
                }
            } else {
                workflow.send(Control::Cancel);
            }
            dialog.close();
        });
        dialog.present();
        dialog
    }
}

fn choose_recent(client: &PackageClient) -> Result<String, String> {
    let mut fallback = None;
    let mut recovery = None;
    for id in client.recent()? {
        let (receipt, _) = client.status(&id)?;
        if !receipt.phase.is_terminal() {
            return Ok(id);
        }
        if receipt.phase == PackagePhase::RecoveryRequired && recovery.is_none() {
            recovery = Some(id.clone());
        }
        if fallback.is_none() {
            fallback = Some(id);
        }
    }
    recovery
        .or(fallback)
        .ok_or_else(|| "No package operation history is available for this account.".into())
}

pub(crate) fn stronger_confirmation(receipt: &PackageReceipt) -> bool {
    receipt
        .changes
        .iter()
        .any(|change| matches!(change.operation.as_str(), "remove" | "downgrade"))
}

pub(crate) fn review_text(receipt: &PackageReceipt) -> String {
    let mut text = String::new();
    for change in &receipt.changes {
        text.push_str(&change.operation);
        text.push(' ');
        text.push_str(&change.name);
        text.push(':');
        text.push_str(&change.architecture);
        text.push_str("  ");
        text.push_str(change.old_version.as_deref().unwrap_or("—"));
        text.push_str(" → ");
        text.push_str(change.new_version.as_deref().unwrap_or("—"));
        text.push('\n');
    }
    text
}

fn phase_message(receipt: &PackageReceipt) -> String {
    if receipt.phase == PackagePhase::Failed {
        let reason = match receipt.error_code.as_deref() {
            Some("maintainer_requires_external_update") => {
                Some("Update Maintainer with your normal system package manager, then check again.")
            }
            Some("held_package") => {
                Some("The plan would change a held package. No installation was started.")
            }
            Some("kernel_fallback_required") => {
                Some("Removal would leave no spare installed kernel. Keep a fallback kernel.")
            }
            Some("protected_package") => Some(
                "The plan would remove a protected package or the running kernel. No installation was started.",
            ),
            Some("package_lock_unavailable") => {
                Some("Another package manager is busy. Wait for it to finish, then check again.")
            }
            Some("repaired_outcome_unknown") => Some(
                "Package-manager state is consistent again, but the original operation's outcome could not be verified. Check available updates again.",
            ),
            _ => None,
        };
        if let Some(reason) = reason {
            return reason.into();
        }
    }
    match receipt.phase {
        PackagePhase::Locking => "Waiting for the package-manager lock…",
        PackagePhase::Resolving => "Resolving the complete package plan…",
        PackagePhase::Review => "Awaiting review…",
        PackagePhase::Fetching => "Downloading and verifying packages…",
        PackagePhase::Mutating => "Applying packages. Cancellation is unavailable; the package owner continues independently.",
        PackagePhase::Succeeded => "Package operation completed and installed versions were verified.",
        PackagePhase::Cancelled => "Package operation cancelled before installation.",
        PackagePhase::Failed => "Package operation failed before installation. Check available updates again.",
        PackagePhase::RecoveryRequired => "Package operation requires recovery. Repair the package manager with your normal system tools, then use Recheck recovery for a read-only check.",
    }.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use maintainer_core::package_transaction::PackageChange;

    fn receipt() -> PackageReceipt {
        PackageReceipt {
            id: "a".repeat(32),
            uid: 1000,
            backend_pid: 12,
            backend_start_ticks: 1,
            phase: PackagePhase::Review,
            intent: "full".into(),
            requests: vec![],
            fingerprint: Some("b".repeat(64)),
            download_bytes: 100,
            changes: vec![PackageChange {
                operation: "upgrade".into(),
                name: "example".into(),
                architecture: "amd64".into(),
                old_version: Some("1".into()),
                new_version: Some("2".into()),
            }],
            error_code: None,
        }
    }

    #[test]
    fn complete_review_keeps_all_operations_architectures_and_versions() {
        let mut receipt = receipt();
        receipt.changes = (0..4096)
            .map(|i| PackageChange {
                name: format!("package-{i}"),
                architecture: if i % 2 == 0 { "amd64" } else { "i386" }.into(),
                ..receipt.changes[0].clone()
            })
            .collect();
        let text = review_text(&receipt);
        assert_eq!(text.lines().count(), 4096);
        assert!(text.contains("upgrade package-4095:i386  1 → 2"));
        assert!(!stronger_confirmation(&receipt));
        receipt.changes[0].operation = "remove".into();
        assert!(stronger_confirmation(&receipt));
        receipt.changes[0].operation = "downgrade".into();
        assert!(stronger_confirmation(&receipt));
    }

    #[test]
    fn destructive_review_requires_explicit_acknowledgement_before_apply() {
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        let page = UpdatesPage::new();
        let window = gtk4::Window::builder().child(page.widget()).build();
        window.present();
        let workflow = PackageWorkflow::new(&page, Arc::new(JobExecutor::unavailable()));
        let mut receipt = receipt();
        receipt.changes[0].operation = "remove".into();
        let dialog = workflow.review_dialog(&receipt, true);
        let apply = dialog
            .widget_for_response(gtk4::ResponseType::Accept)
            .unwrap();
        assert!(!apply.is_sensitive());
        dialog.response(gtk4::ResponseType::Accept);
        assert!(dialog.is_visible());
        let mut child = dialog.content_area().first_child();
        let mut found = false;
        while let Some(widget) = child {
            if let Some(check) = widget.downcast_ref::<gtk4::CheckButton>() {
                check.set_active(true);
                found = true;
            }
            child = widget.next_sibling();
        }
        assert!(found);
        assert!(apply.is_sensitive());
        dialog.response(gtk4::ResponseType::Cancel);
        window.close();
    }
}
