//! A reusable view that renders capability states: loading, unavailable,
//! permission-required, empty, failure, and cancelled.
//!
//! `StatusView` takes a `CapabilityViewModel` (a presentation model) rather
//! than a raw `Capability<T>` from the core crate, keeping the UI layer
//! decoupled from service internals.

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Image, Label, Orientation};

/// The visual state a capability can be presented in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CapabilityStatus {
    /// Data is being loaded.
    Loading,
    /// The capability is not available on this system.
    Unavailable,
    /// Elevated privilege is required.
    PermissionRequired,
    /// Data loaded successfully but the result set is empty.
    Empty,
    /// An error occurred.
    Failure,
    /// The operation was cancelled by the user.
    Cancelled,
}

/// Presentation model consumed by `StatusView`.
///
/// This is intentionally separate from core types so the UI never
/// depends directly on service-layer enums.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapabilityViewModel {
    /// Current status of the capability.
    pub status: CapabilityStatus,
    /// Primary user-facing title (e.g., "Temperature sensors unavailable").
    pub title: String,
    /// Optional secondary description or remediation advice.
    pub description: Option<String>,
    /// If true, a retry action is meaningful for this state.
    pub can_retry: bool,
    /// Identifier used when emitting a retry intent.
    pub capability_id: &'static str,
}

/// Builds a GTK widget tree representing the current capability state.
///
/// The returned `GtkBox` contains an icon, title label, optional description,
/// and an optional retry button. Accessibility names are set to include
/// the status and any remediation text.
pub struct StatusView;

impl StatusView {
    /// Render a status view from a `CapabilityViewModel`.
    ///
    /// Returns a vertical `GtkBox` ready to be packed into a parent container.
    /// The optional `on_retry` callback is invoked when the retry button is clicked.
    pub fn render(model: &CapabilityViewModel, on_retry: Option<impl Fn() + 'static>) -> GtkBox {
        let container = GtkBox::new(Orientation::Vertical, 12);
        container.set_halign(Align::Center);
        container.set_valign(Align::Center);
        container.add_css_class("maintainer_status_view");

        // Icon
        let icon = Image::from_icon_name(Self::icon_for_status(&model.status));
        icon.set_pixel_size(48);
        icon.add_css_class("maintainer_status_icon");
        icon.add_css_class(Self::css_class_for_status(&model.status));
        container.append(&icon);

        // Title
        let title_label = Label::new(Some(&model.title));
        title_label.add_css_class("maintainer_status_title");
        title_label.set_wrap(true);
        title_label.set_halign(Align::Center);
        container.append(&title_label);

        // Description
        if let Some(ref desc) = model.description {
            let desc_label = Label::new(Some(desc));
            desc_label.add_css_class("maintainer_status_description");
            desc_label.add_css_class("maintainer_text_muted");
            desc_label.set_wrap(true);
            desc_label.set_halign(Align::Center);
            container.append(&desc_label);
        }

        // Retry button (only if retry is meaningful)
        if model.can_retry {
            let retry_btn = Button::with_label("Retry");
            retry_btn.add_css_class("maintainer_status_retry");
            retry_btn.set_halign(Align::Center);

            // Accessible description
            retry_btn.update_property(&[gtk4::accessible::Property::Description(&format!(
                "Retry {}",
                model.capability_id
            ))]);

            if let Some(callback) = on_retry {
                retry_btn.connect_clicked(move |_| callback());
            }

            container.append(&retry_btn);
        }

        // Accessible name for the whole container
        let accessible_desc = match &model.description {
            Some(desc) => format!(
                "{}: {}. {}",
                Self::status_label(&model.status),
                model.title,
                desc
            ),
            None => format!("{}: {}", Self::status_label(&model.status), model.title),
        };
        container.update_property(&[gtk4::accessible::Property::Description(&accessible_desc)]);

        container
    }

    fn icon_for_status(status: &CapabilityStatus) -> &'static str {
        match status {
            CapabilityStatus::Loading => "content-loading-symbolic",
            CapabilityStatus::Unavailable => "action-unavailable-symbolic",
            CapabilityStatus::PermissionRequired => "dialog-password-symbolic",
            CapabilityStatus::Empty => "edit-clear-symbolic",
            CapabilityStatus::Failure => "dialog-error-symbolic",
            CapabilityStatus::Cancelled => "process-stop-symbolic",
        }
    }

    fn css_class_for_status(status: &CapabilityStatus) -> &'static str {
        match status {
            CapabilityStatus::Loading => "maintainer_status_loading",
            CapabilityStatus::Unavailable => "maintainer_status_unavailable",
            CapabilityStatus::PermissionRequired => "maintainer_status_permission",
            CapabilityStatus::Empty => "maintainer_status_empty",
            CapabilityStatus::Failure => "maintainer_status_failure",
            CapabilityStatus::Cancelled => "maintainer_status_cancelled",
        }
    }

    fn status_label(status: &CapabilityStatus) -> &'static str {
        match status {
            CapabilityStatus::Loading => "Loading",
            CapabilityStatus::Unavailable => "Unavailable",
            CapabilityStatus::PermissionRequired => "Permission required",
            CapabilityStatus::Empty => "Empty",
            CapabilityStatus::Failure => "Error",
            CapabilityStatus::Cancelled => "Cancelled",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capability_view_model_roundtrip() {
        let model = CapabilityViewModel {
            status: CapabilityStatus::Unavailable,
            title: "Temperature sensors unavailable".into(),
            description: Some("No hwmon driver detected".into()),
            can_retry: false,
            capability_id: "temperature",
        };
        assert_eq!(model.status, CapabilityStatus::Unavailable);
        assert!(!model.can_retry);
    }

    #[test]
    fn all_statuses_have_icons() {
        let statuses = [
            CapabilityStatus::Loading,
            CapabilityStatus::Unavailable,
            CapabilityStatus::PermissionRequired,
            CapabilityStatus::Empty,
            CapabilityStatus::Failure,
            CapabilityStatus::Cancelled,
        ];
        for status in &statuses {
            assert!(!StatusView::icon_for_status(status).is_empty());
            assert!(!StatusView::css_class_for_status(status).is_empty());
            assert!(!StatusView::status_label(status).is_empty());
        }
    }
}
