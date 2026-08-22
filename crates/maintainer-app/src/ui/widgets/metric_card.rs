//! Metric card widget: a titled card with a value label and optional sparkline.
//!
//! Used on both Overview and System Monitor pages. Displays one metric with
//! its current value, an optional sparkline for history, and handles the
//! unavailable state by showing a reason instead of zero values.

use std::collections::VecDeque;

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Image, Label, Orientation};

use crate::ui::widgets::sparkline::Sparkline;

/// A metric card with title, value, and optional sparkline.
pub struct MetricCard {
    container: GtkBox,
    _title_label: Label,
    value_label: Label,
    sparkline: Option<Sparkline>,
}

impl MetricCard {
    /// Create a new metric card.
    ///
    /// If `with_sparkline` is true, a sparkline widget is included below the value.
    pub fn new(title: &str, with_sparkline: bool) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.add_css_class("maintainer_metric_card");
        container.set_hexpand(true);

        let header_row = GtkBox::new(Orientation::Horizontal, 8);
        header_row.set_valign(Align::Center);

        let icon_name = match title {
            "CPU" => "utilities-system-monitor-symbolic",
            "Memory" => "media-memory-symbolic",
            "Storage" => "drive-harddisk-symbolic",
            "Network" => "network-wired-symbolic",
            _ => "dialog-information-symbolic",
        };
        let icon = Image::from_icon_name(icon_name);
        icon.set_pixel_size(14);
        icon.add_css_class("maintainer_text_muted");
        header_row.append(&icon);

        let title_label = Label::new(Some(title));
        title_label.add_css_class("maintainer_metric_card_title");
        title_label.set_halign(Align::Start);
        title_label.set_hexpand(true);
        header_row.append(&title_label);

        container.append(&header_row);

        let value_label = Label::new(Some("—"));
        value_label.add_css_class("maintainer_metric_card_value");
        value_label.set_halign(Align::Start);
        container.append(&value_label);

        let sparkline = if with_sparkline {
            let sl = Sparkline::new(0.0, 100.0);
            sl.widget().set_hexpand(true);
            container.append(sl.widget());
            Some(sl)
        } else {
            None
        };

        Self {
            container,
            _title_label: title_label,
            value_label,
            sparkline,
        }
    }

    /// Update the displayed value text.
    pub fn set_value(&self, text: &str) {
        self.value_label.set_text(text);
        self.update_accessible_description();
    }

    /// Update the sparkline with new ring buffer data.
    pub fn update_sparkline(&self, samples: &VecDeque<f64>, accessible_text: &str) {
        if let Some(ref sparkline) = self.sparkline {
            sparkline.update_from_ring(samples, accessible_text);
        }
    }

    /// Show an unavailable state with the given reason.
    pub fn set_unavailable(&self, reason: &str) {
        self.value_label.set_text(reason);
        self.value_label.add_css_class("maintainer_text_muted");
        if let Some(ref sparkline) = self.sparkline {
            sparkline.widget().set_visible(false);
        }
        self.update_accessible_description();
    }

    /// Restore available state.
    pub fn set_available(&self) {
        self.value_label.remove_css_class("maintainer_text_muted");
        if let Some(ref sparkline) = self.sparkline {
            sparkline.widget().set_visible(true);
        }
    }

    /// Get the underlying GTK widget.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Announce title and current value together so screen readers treat the
    /// card as one grouped metric (e.g. "CPU: 42%") rather than two unrelated labels.
    fn update_accessible_description(&self) {
        let title = self._title_label.text();
        let value = self.value_label.text();
        self.container
            .update_property(&[gtk4::accessible::Property::Label(&format!(
                "{title}: {value}"
            ))]);
    }
}
