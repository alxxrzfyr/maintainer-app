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

use crate::runtime::RuntimeHandle;

/// State for the reports page.
struct ReportsState {
    /// The rendered report text, available after preview.
    preview_text: Option<String>,
}

/// Build the reports page widget tree.
pub fn build_reports_page(runtime: RuntimeHandle) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 12);
    container.add_css_class("reports-page");
    container.set_margin_top(24);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let state = Rc::new(RefCell::new(ReportsState { preview_text: None }));

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
                let text = match outcome {
                    crate::dispatch::ReportOutcome::Completed { text } => text,
                    crate::dispatch::ReportOutcome::Failed { message } => {
                        format!("Could not generate report: {message}")
                    }
                };
                text_view.buffer().set_text(&text);
                state.borrow_mut().preview_text = Some(text);
                copy_button.set_sensitive(true);
                save_button.set_sensitive(true);
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
}
