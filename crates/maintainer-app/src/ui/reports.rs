//! Reports page: preview, copy, and save redacted system reports.
//!
//! Preview is mandatory before copy/save. Save writes through a temporary
//! anonymous file with mode 0600, flush, then publish without overwriting.
//! No upload/share endpoint is provided.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, FileChooserAction, FileChooserDialog, Label, Orientation,
    ResponseType, ScrolledWindow, TextView, WrapMode,
};

use crate::runtime::RuntimeHandle;
use maintainer_core::reports::export::{ReportExportOutcome, save_report_revision};

/// State for the reports page.
struct ReportsState {
    /// The rendered report text, available after preview.
    preview_text: Option<String>,
    revision: Arc<AtomicU64>,
    commit_lock: Arc<Mutex<()>>,
}

/// Build the reports page widget tree.
pub fn build_reports_page(runtime: RuntimeHandle) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 12);
    container.add_css_class("reports-page");
    container.set_margin_top(24);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let state = Rc::new(RefCell::new(ReportsState {
        preview_text: None,
        revision: Arc::new(AtomicU64::new(0)),
        commit_lock: Arc::new(Mutex::new(())),
    }));

    let header = Label::new(Some("System Report"));
    header.add_css_class("title-1");
    header.set_halign(Align::Start);
    container.append(&header);

    let description = Label::new(Some(
        "Generate a privacy-conscious snapshot of your system state. \
         Sensitive data is redacted by default.",
    ));
    description.set_halign(Align::Start);
    description.set_wrap(true);
    description.add_css_class("dim-label");
    container.append(&description);

    let text_view = TextView::new();
    text_view.set_editable(false);
    text_view.set_monospace(true);
    text_view.set_wrap_mode(WrapMode::Word);
    text_view.set_vexpand(true);
    text_view.set_left_margin(16);
    text_view.set_right_margin(16);
    text_view.set_top_margin(16);
    text_view.set_bottom_margin(16);
    text_view.add_css_class("maintainer_report_view");
    text_view.buffer().set_text(
        "Click \"Generate Preview\" below to create a redacted snapshot of your system state.",
    );

    let scrolled = ScrolledWindow::new();
    scrolled.set_child(Some(&text_view));
    scrolled.set_vexpand(true);
    scrolled.set_min_content_height(300);
    container.append(&scrolled);

    let button_bar = GtkBox::new(Orientation::Horizontal, 8);
    button_bar.set_halign(Align::End);
    button_bar.set_vexpand(false);

    let generate_btn = Button::with_label("Generate Preview");
    generate_btn.add_css_class("suggested-action");
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
        let runtime_clone = runtime.clone();
        generate_btn.connect_clicked(move |button| {
            let commit_lock = state_clone.borrow().commit_lock.clone();
            let Ok(commit_guard) = commit_lock.try_lock() else {
                button.set_label("Save finishing; retry preview");
                return;
            };
            let revision = {
                let mut state = state_clone.borrow_mut();
                let revision = state
                    .revision
                    .fetch_add(1, Ordering::AcqRel)
                    .wrapping_add(1);
                state.preview_text = None;
                revision
            };
            drop(commit_guard);
            button.set_sensitive(false);
            button.set_label("Generating…");
            copy_btn_clone.set_sensitive(false);
            save_btn_clone.set_sensitive(false);

            let rt = runtime_clone.borrow();
            let executor = rt.executor.clone();
            let database = rt.db.clone();
            drop(rt);

            let state = state_clone.clone();
            let text_view = text_view_clone.clone();
            let copy_button = copy_btn_clone.clone();
            let save_button = save_btn_clone.clone();
            let generate_button = button.clone();
            crate::dispatch::dispatch_report_generation(&executor, database, move |outcome| {
                if state.borrow().revision.load(Ordering::Acquire) != revision {
                    return;
                }
                let preview_text = report_preview_text(outcome);
                let preview_ready = preview_text.is_some();
                let display_text = preview_text
                    .as_deref()
                    .unwrap_or("Could not generate report. Try again later.");
                text_view.buffer().set_text(display_text);
                state.borrow_mut().preview_text = preview_text;
                copy_button.set_sensitive(preview_ready);
                save_button.set_sensitive(preview_ready);
                generate_button.set_label("Generate Preview");
                generate_button.set_sensitive(true);
            });
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
        let executor = runtime.borrow().executor.clone();
        save_btn.connect_clicked(move |btn| {
            let st = state_clone.borrow();
            if let Some(ref text) = st.preview_text {
                let text_owned = text.clone();
                let revision = st.revision.load(Ordering::Acquire);
                let current_preview = state_clone.clone();
                let executor = executor.clone();
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
                                if current_preview.borrow().revision.load(Ordering::Acquire)
                                    != revision
                                {
                                    btn_clone.set_label("Preview changed; save again");
                                    dialog.close();
                                    return;
                                }
                                let text = text_owned.clone();
                                let result_button = btn_clone.clone();
                                let revision_guard = current_preview.borrow().revision.clone();
                                let commit_lock = current_preview.borrow().commit_lock.clone();
                                let preview_for_result = current_preview.clone();
                                btn_clone.set_sensitive(false);
                                crate::dispatch::dispatch_local_action(
                                    &executor,
                                    move || {
                                        save_report_revision(
                                            &path,
                                            &text,
                                            &revision_guard,
                                            revision,
                                            &commit_lock,
                                        )
                                    },
                                    move |result| {
                                        let current = preview_for_result.borrow();
                                        if current.revision.load(Ordering::Acquire) != revision {
                                            result_button
                                                .set_sensitive(current.preview_text.is_some());
                                            return;
                                        }
                                        result_button.set_label(match result {
                                            Ok(ReportExportOutcome::Saved) => "Saved!",
                                            Ok(ReportExportOutcome::DurabilityUnknown) => {
                                                "File saved; disk durability unconfirmed"
                                            }
                                            Err(_) => "Save failed; choose a new filename",
                                        });
                                        result_button.set_sensitive(true);
                                        glib::timeout_add_local_once(
                                            std::time::Duration::from_secs(2),
                                            move || {
                                                result_button.set_label("Save to File");
                                            },
                                        );
                                    },
                                );
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

fn report_preview_text(outcome: crate::dispatch::ReportOutcome) -> Option<String> {
    match outcome {
        crate::dispatch::ReportOutcome::Completed { text } => Some(text),
        crate::dispatch::ReportOutcome::Failed { .. } => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn failed_report_does_not_create_a_copyable_preview() {
        let outcome = crate::dispatch::ReportOutcome::Failed {
            message: "/home/alice/private-report.log".into(),
        };
        assert!(super::report_preview_text(outcome).is_none());
    }
}
