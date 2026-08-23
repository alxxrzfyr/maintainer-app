//! Kernel management section: lists installed kernels with status badges
//! and safe removal via the privileged helper.
//!
//! Embedded in the Updates page. Handles loading, empty, unavailable, and
//! error states explicitly. The running kernel row never exposes a delete
//! action — this is a UI-layer defense-in-depth check independent of the
//! two backend layers (core and helper) that also enforce this invariant.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Dialog, DialogFlags, Image, Label, Orientation, ResponseType,
    Spinner,
};

use maintainer_core::kernel::KernelInfo;

/// State of the kernel section.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KernelSectionState {
    /// Initial state, not yet loaded.
    Idle,
    /// Loading kernel list.
    Loading,
    /// Kernel list loaded and displayed.
    Loaded,
    /// An error occurred during enumeration.
    Failed,
    /// Kernel enumeration is unavailable on this system.
    Unavailable,
}

/// Callback for requesting a kernel removal (package_name).
type RemoveCallback = Rc<RefCell<Option<Box<dyn Fn(String)>>>>;

/// The kernel management section widget.
pub struct KernelSection {
    container: GtkBox,
    list_container: GtkBox,
    status_label: Label,
    refresh_button: Button,
    spinner: Spinner,
    state: KernelSectionState,
    on_remove: RemoveCallback,
}

impl Default for KernelSection {
    fn default() -> Self {
        Self::new()
    }
}

impl KernelSection {
    /// Build the kernel section widget tree.
    pub fn new() -> Self {
        let container = GtkBox::new(Orientation::Vertical, 10);
        container.add_css_class("maintainer_kernel_section");
        container.set_margin_top(20);

        // Section header
        let header_row = GtkBox::new(Orientation::Horizontal, 8);
        header_row.set_valign(Align::Center);

        let icon = Image::from_icon_name("system-run-symbolic");
        icon.set_pixel_size(16);
        icon.add_css_class("kernel-icon");
        header_row.append(&icon);

        let title = Label::new(Some("Installed Kernels"));
        title.add_css_class("maintainer_section_title");
        title.set_halign(Align::Start);
        title.set_hexpand(true);
        header_row.append(&title);

        let refresh_button = Button::from_icon_name("view-refresh-symbolic");
        refresh_button.set_tooltip_text(Some("Refresh kernel list"));
        refresh_button.add_css_class("flat");
        header_row.append(&refresh_button);

        let spinner = Spinner::new();
        spinner.set_visible(false);
        header_row.append(&spinner);

        container.append(&header_row);

        // Status label (for loading/error/unavailable messages)
        let status_label = Label::new(None);
        status_label.set_halign(Align::Start);
        status_label.set_wrap(true);
        status_label.set_visible(false);
        container.append(&status_label);

        // Kernel list container
        let list_container = GtkBox::new(Orientation::Vertical, 0);
        list_container.add_css_class("maintainer_kernel_list");
        container.append(&list_container);

        let on_remove: RemoveCallback = Rc::new(RefCell::new(None));

        Self {
            container,
            list_container,
            status_label,
            refresh_button,
            spinner,
            state: KernelSectionState::Idle,
            on_remove,
        }
    }

    /// Get the root widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Get the refresh button for signal connections.
    pub fn refresh_button(&self) -> &Button {
        &self.refresh_button
    }

    /// Connect a callback for kernel removal requests.
    pub fn connect_remove(&self, cb: impl Fn(String) + 'static) {
        *self.on_remove.borrow_mut() = Some(Box::new(cb));
    }

    /// Transition to loading state.
    pub fn set_loading(&mut self) {
        self.state = KernelSectionState::Loading;
        self.spinner.set_visible(true);
        self.spinner.set_spinning(true);
        self.refresh_button.set_sensitive(false);
        self.status_label.set_text("Loading kernel information…");
        self.status_label.set_visible(true);
        self.status_label.remove_css_class("error");
        self.status_label.remove_css_class("maintainer_text_muted");
        self.status_label.add_css_class("maintainer_text_muted");
        self.clear_list();
    }

    /// Display the loaded kernel list.
    pub fn set_loaded(&mut self, kernels: &[KernelInfo]) {
        self.state = KernelSectionState::Loaded;
        self.spinner.set_visible(false);
        self.spinner.set_spinning(false);
        self.refresh_button.set_sensitive(true);

        self.clear_list();

        if kernels.is_empty() {
            self.status_label
                .set_text("No kernel packages detected on this system.");
            self.status_label.set_visible(true);
            self.status_label.remove_css_class("error");
            self.status_label.add_css_class("maintainer_text_muted");
            return;
        }

        self.status_label.set_visible(false);
        for kernel in kernels {
            let row = self.build_kernel_row(kernel);
            self.list_container.append(&row);
        }
    }

    /// Show an error state.
    pub fn set_failed(&mut self, message: &str) {
        self.state = KernelSectionState::Failed;
        self.spinner.set_visible(false);
        self.spinner.set_spinning(false);
        self.refresh_button.set_sensitive(true);
        self.status_label.set_text(message);
        self.status_label.set_visible(true);
        self.status_label.remove_css_class("maintainer_text_muted");
        self.status_label.add_css_class("error");
        self.clear_list();
    }

    /// Show an unavailable state.
    pub fn set_unavailable(&mut self, reason: &str) {
        self.state = KernelSectionState::Unavailable;
        self.spinner.set_visible(false);
        self.spinner.set_spinning(false);
        self.refresh_button.set_sensitive(true);
        self.status_label.set_text(reason);
        self.status_label.set_visible(true);
        self.status_label.remove_css_class("error");
        self.status_label.add_css_class("maintainer_text_muted");
        self.clear_list();
    }

    /// Update a single row to show operation result without rebuilding the
    /// entire list. `success` controls whether the row shows a success or
    /// error indicator.
    pub fn set_row_result(&self, package_name: &str, success: bool, message: &str) {
        // Walk children looking for the row matching this package
        let mut child = self.list_container.first_child();
        while let Some(widget) = child {
            if let Some(row) = widget.downcast_ref::<GtkBox>() {
                // Check if the row's accessible label contains the package name
                if let Some(name_label) = find_label_in_row(row) {
                    if name_label.text().as_str() == package_name {
                        // Find the status area and update it
                        if let Some(status) = find_status_label_in_row(row) {
                            if success {
                                status.set_text("Removed");
                                status.remove_css_class("error");
                                status.add_css_class("success");
                            } else {
                                status.set_text(message);
                                status.remove_css_class("success");
                                status.add_css_class("error");
                            }
                            status.set_visible(true);
                        }
                        // Disable the delete button after successful removal
                        if success {
                            if let Some(btn) = find_delete_button_in_row(row) {
                                btn.set_sensitive(false);
                                btn.set_label("Removed");
                            }
                        }
                        return;
                    }
                }
            }
            child = widget.next_sibling();
        }
    }

    /// Current state.
    pub fn state(&self) -> KernelSectionState {
        self.state
    }

    fn clear_list(&self) {
        while let Some(child) = self.list_container.first_child() {
            self.list_container.remove(&child);
        }
    }

    fn build_kernel_row(&self, kernel: &KernelInfo) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 12);
        row.add_css_class("maintainer_kernel_row");
        row.set_margin_top(4);
        row.set_margin_bottom(4);

        // Status badge: Running gets a distinct accent pill, not just an icon.
        if kernel.is_running {
            let badge = Label::new(Some("Running"));
            badge.add_css_class("maintainer_kernel_badge_running");
            badge.set_valign(Align::Center);
            row.append(&badge);
        } else if kernel.is_removable {
            let badge = Label::new(Some("Removable"));
            badge.add_css_class("maintainer_kernel_badge_removable");
            badge.set_valign(Align::Center);
            row.append(&badge);
        } else {
            let badge = Label::new(Some("Installed"));
            badge.add_css_class("maintainer_kernel_badge_installed");
            badge.set_valign(Align::Center);
            row.append(&badge);
        }

        // Package name and version info
        let info_box = GtkBox::new(Orientation::Vertical, 2);
        info_box.set_hexpand(true);

        let name_label = Label::new(Some(&kernel.package_name));
        name_label.set_halign(Align::Start);
        name_label.add_css_class("heading");
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        info_box.append(&name_label);

        let version_label = Label::new(Some(&format!("Version: {}", kernel.version)));
        version_label.set_halign(Align::Start);
        version_label.add_css_class("dim-label");
        info_box.append(&version_label);

        row.append(&info_box);

        // Operation result label (hidden by default, shown after remove attempt)
        let op_status = Label::new(None);
        op_status.set_visible(false);
        op_status.set_halign(Align::End);
        op_status.set_valign(Align::Center);
        op_status.add_css_class("maintainer_kernel_op_status");
        row.append(&op_status);

        // Delete button: only for removable kernels.
        // DEFENSE-IN-DEPTH: Even if is_removable is true, the button is DISABLED
        // when is_running is true. This ensures a bug in the backend claiming
        // removability for the running kernel cannot lead to accidental removal.
        if kernel.is_removable {
            let delete_btn = Button::with_label("Remove");
            delete_btn.add_css_class("maintainer_destructive");
            delete_btn.set_valign(Align::Center);
            delete_btn.set_tooltip_text(Some(&format!("Remove {}", kernel.package_name)));

            // UI-LAYER DEFENSE-IN-DEPTH: disable for running kernel regardless
            // of what is_removable claims.
            if kernel.is_running {
                delete_btn.set_sensitive(false);
                delete_btn.set_tooltip_text(Some("Cannot remove the running kernel"));
            }

            let pkg_name = kernel.package_name.clone();
            let on_remove = self.on_remove.clone();
            delete_btn.connect_clicked(move |btn| {
                let window = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());
                if let Some(win) = window {
                    show_kernel_removal_dialog(&win, &pkg_name, &on_remove);
                }
            });

            row.append(&delete_btn);
        }

        row
    }
}

/// Show a strong confirmation dialog before kernel removal.
///
/// This is an irreversible operation — the copy communicates finality and
/// requires explicit confirmation, matching the AGENTS.md requirement for
/// "explicit stronger confirmation" on permanent deletion.
fn show_kernel_removal_dialog(
    parent: &gtk4::Window,
    package_name: &str,
    on_remove: &RemoveCallback,
) {
    let dialog = Dialog::with_buttons(
        Some("Remove Kernel Package"),
        Some(parent),
        DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", ResponseType::Cancel),
            ("Remove Permanently", ResponseType::Accept),
        ],
    );

    // Mark the destructive button
    if let Some(btn) = dialog.widget_for_response(ResponseType::Accept) {
        btn.add_css_class("maintainer_destructive");
    }

    let content = dialog.content_area();
    content.set_margin_top(16);
    content.set_margin_bottom(12);
    content.set_margin_start(16);
    content.set_margin_end(16);
    content.set_spacing(10);

    let warning_icon = Image::from_icon_name("dialog-warning-symbolic");
    warning_icon.set_pixel_size(32);
    warning_icon.add_css_class("maintainer_icon_warning");
    warning_icon.set_halign(Align::Start);
    content.append(&warning_icon);

    let msg = Label::new(Some(&format!("Permanently remove \"{}\"?", package_name)));
    msg.set_halign(Align::Start);
    msg.add_css_class("heading");
    msg.set_wrap(true);
    content.append(&msg);

    let detail = Label::new(Some(
        "This will purge the kernel package from the system. \
         This action cannot be undone — the package must be \
         reinstalled from APT if needed again.\n\n\
         Only proceed if you are certain this kernel version is \
         no longer required.",
    ));
    detail.add_css_class("dim-label");
    detail.set_halign(Align::Start);
    detail.set_wrap(true);
    content.append(&detail);

    let pkg_name = package_name.to_string();
    let on_rm = on_remove.clone();
    dialog.connect_response(move |dlg, response| {
        if response == ResponseType::Accept {
            if let Some(ref cb) = *on_rm.borrow() {
                cb(pkg_name.clone());
            }
        }
        dlg.close();
    });

    dialog.present();
}

/// Helper: find the first Label with `heading` class in a row (the package name).
fn find_label_in_row(row: &GtkBox) -> Option<Label> {
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(info_box) = widget.downcast_ref::<GtkBox>() {
            let mut inner = info_box.first_child();
            while let Some(inner_w) = inner {
                if let Some(label) = inner_w.downcast_ref::<Label>() {
                    if label.has_css_class("heading") {
                        return Some(label.clone());
                    }
                }
                inner = inner_w.next_sibling();
            }
        }
        child = widget.next_sibling();
    }
    None
}

/// Helper: find the operation status label in a row.
fn find_status_label_in_row(row: &GtkBox) -> Option<Label> {
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(label) = widget.downcast_ref::<Label>() {
            if label.has_css_class("maintainer_kernel_op_status") {
                return Some(label.clone());
            }
        }
        child = widget.next_sibling();
    }
    None
}

/// Helper: find the delete button in a row.
fn find_delete_button_in_row(row: &GtkBox) -> Option<Button> {
    let mut child = row.first_child();
    while let Some(widget) = child {
        if let Some(btn) = widget.downcast_ref::<Button>() {
            if btn.has_css_class("maintainer_destructive") {
                return Some(btn.clone());
            }
        }
        child = widget.next_sibling();
    }
    None
}
