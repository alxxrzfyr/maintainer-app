//! Overview page: system health summary with metric cards and attention items.
//!
//! Shows CPU, memory, disk, and network at a glance with sparklines for the
//! first two. Attention items are derived from explicit evidence (disk thresholds,
//! temperature critical states) — never from transient CPU load.

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Image, Label, Orientation, ScrolledWindow};

use crate::monitoring::{AttentionItem, AttentionSeverity, OverviewViewModel};
use crate::ui::widgets::metric_card::MetricCard;

/// The overview page widget tree.
pub struct OverviewPage {
    root: ScrolledWindow,
    cpu_card: MetricCard,
    memory_card: MetricCard,
    disk_card: MetricCard,
    network_card: MetricCard,
    attention_box: GtkBox,
}

impl Default for OverviewPage {
    fn default() -> Self {
        Self::new()
    }
}

impl OverviewPage {
    /// Create a new overview page.
    pub fn new() -> Self {
        let content = GtkBox::new(Orientation::Vertical, 16);
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.add_css_class("maintainer_overview_page");

        // Title
        let title = Label::new(Some("Overview"));
        title.add_css_class("maintainer_page_title");
        title.set_halign(Align::Start);
        content.append(&title);

        // Metric cards row
        let cards_row = GtkBox::new(Orientation::Horizontal, 12);
        cards_row.set_homogeneous(true);

        let cpu_card = MetricCard::new("CPU", true);
        cards_row.append(cpu_card.widget());

        let memory_card = MetricCard::new("Memory", true);
        cards_row.append(memory_card.widget());

        let disk_card = MetricCard::new("Storage", false);
        cards_row.append(disk_card.widget());

        let network_card = MetricCard::new("Network", false);
        cards_row.append(network_card.widget());

        content.append(&cards_row);

        // Attention section
        let attention_label = Label::new(Some("Attention"));
        attention_label.add_css_class("maintainer_section_title");
        attention_label.set_halign(Align::Start);
        attention_label.set_margin_top(16);
        content.append(&attention_label);

        let attention_box = GtkBox::new(Orientation::Vertical, 4);
        attention_box.add_css_class("maintainer_attention_list");
        content.append(&attention_box);

        // Empty state for attention
        let empty_label = Label::new(Some("No issues detected"));
        empty_label.add_css_class("maintainer_text_muted");
        empty_label.set_halign(Align::Start);
        attention_box.append(&empty_label);

        let root = ScrolledWindow::new();
        root.set_child(Some(&content));
        root.set_hexpand(true);
        root.set_vexpand(true);

        Self {
            root,
            cpu_card,
            memory_card,
            disk_card,
            network_card,
            attention_box,
        }
    }

    /// Update the page from the view model. Only touches widgets with changed data.
    pub fn update(&self, vm: &OverviewViewModel) {
        // CPU card
        if vm.cpu_available {
            self.cpu_card.set_available();
            self.cpu_card.set_value(&vm.cpu_text);
            self.cpu_card.update_sparkline(
                vm.cpu_ring.samples(),
                &format!("CPU usage: {}", vm.cpu_text),
            );
        } else {
            let reason = vm
                .cpu_unavailable_reason
                .as_deref()
                .unwrap_or("Unavailable");
            self.cpu_card.set_unavailable(reason);
        }

        // Memory card
        if vm.memory_available {
            self.memory_card.set_available();
            self.memory_card.set_value(&vm.memory_text);
            self.memory_card.update_sparkline(
                vm.memory_ring.samples(),
                &format!("Memory usage: {}", vm.memory_text),
            );
        } else {
            let reason = vm
                .memory_unavailable_reason
                .as_deref()
                .unwrap_or("Unavailable");
            self.memory_card.set_unavailable(reason);
        }

        // Disk card
        self.disk_card.set_value(&vm.disk_text);

        // Network card
        self.network_card.set_value(&vm.network_text);

        // Attention items
        self.update_attention(&vm.attention_items);
    }

    fn update_attention(&self, items: &[AttentionItem]) {
        // Clear existing children
        while let Some(child) = self.attention_box.first_child() {
            self.attention_box.remove(&child);
        }

        if items.is_empty() {
            let empty_label = Label::new(Some("No issues detected"));
            empty_label.add_css_class("maintainer_text_muted");
            empty_label.set_halign(Align::Start);
            self.attention_box.append(&empty_label);
            return;
        }

        for item in items {
            let row = GtkBox::new(Orientation::Horizontal, 8);
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.add_css_class("maintainer_attention_item");

            let icon_name = match item.severity {
                AttentionSeverity::Critical => "dialog-error-symbolic",
                AttentionSeverity::Warning => "dialog-warning-symbolic",
                AttentionSeverity::Info => "dialog-information-symbolic",
            };

            let icon = Image::from_icon_name(icon_name);
            icon.set_pixel_size(16);
            match item.severity {
                AttentionSeverity::Critical => icon.add_css_class("maintainer_icon_error"),
                AttentionSeverity::Warning => icon.add_css_class("maintainer_icon_warning"),
                AttentionSeverity::Info => {}
            }
            row.append(&icon);

            let label = Label::new(Some(&item.evidence));
            label.set_halign(Align::Start);
            row.append(&label);

            self.attention_box.append(&row);
        }
    }

    /// Get the root widget for embedding in the shell.
    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }
}
