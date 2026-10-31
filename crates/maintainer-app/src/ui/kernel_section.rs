//! Installed kernel inspection and requests for a bound native removal review.
//! Running kernels never expose an action, even with stale removability metadata.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Image, Label, Orientation, Spinner};

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

        self.status_label.set_text("Review Removal resolves the complete plan before any change. The running kernel is protected.");
        self.status_label.set_visible(true);
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
            let badge = Label::new(Some("Not Running"));
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

        if kernel.is_removable && !kernel.is_running {
            let delete_btn = Button::with_label("Review Removal");
            delete_btn.add_css_class("maintainer_destructive");
            delete_btn.set_valign(Align::Center);
            let on_remove = self.on_remove.clone();
            let package = kernel.package_name.clone();
            delete_btn.connect_clicked(move |_| {
                if let Some(callback) = on_remove.borrow().as_ref() {
                    callback(package.clone());
                }
            });
            row.append(&delete_btn);
        }

        row
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn running_kernel_cannot_request_removal_even_with_stale_backend_metadata() {
        assert!(!gtk4::is_initialized() || gtk4::is_initialized_main_thread());
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        let mut section = KernelSection::new();
        let calls = Rc::new(std::cell::Cell::new(0));
        let recorded = calls.clone();
        section.connect_remove(move |_| recorded.set(recorded.get() + 1));
        for is_running in [false, true] {
            section.set_loading();
            section.set_loaded(&[KernelInfo {
                package_name: "linux-image-6.5.0-35-generic".into(),
                version: "6.5.0-35".into(),
                is_running,
                is_removable: true,
            }]);
            assert!(section.status_label.text().contains("protected"));
            let row = section
                .list_container
                .first_child()
                .unwrap()
                .downcast::<GtkBox>()
                .unwrap();
            let button = find_delete_button_in_row(&row);
            if is_running {
                assert!(button.is_none());
            } else {
                button.unwrap().emit_clicked();
            }
            assert_eq!(calls.get(), 1);
        }
    }
}
