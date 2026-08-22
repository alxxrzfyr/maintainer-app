//! Overview page: system health summary, hardware specifications, and attention items.
//!
//! Shows CPU, memory, disk, and network at a glance with sparklines for the
//! first two, detailed hardware & system specifications, and interactive attention items.
//!
//! Metric cards and hardware specifications use FlowBox for fluid wrapping.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, FlowBox, Image, Label, Orientation, ScrolledWindow, SelectionMode,
};

use crate::monitoring::{AttentionItem, AttentionSeverity, DeviceInfo, OverviewViewModel};
use crate::state::Page;
use crate::ui::widgets::metric_card::MetricCard;

/// Callback for navigating to a page from attention item clicks.
type NavigateCallback = Rc<RefCell<Option<Box<dyn Fn(Page)>>>>;

/// The overview page widget tree.
pub struct OverviewPage {
    root: ScrolledWindow,
    cpu_card: MetricCard,
    memory_card: MetricCard,
    disk_card: MetricCard,
    network_card: MetricCard,
    model_value: Label,
    os_value: Label,
    kernel_value: Label,
    cpu_value: Label,
    gpu_value: Label,
    memory_value: Label,
    storage_value: Label,
    desktop_env_value: Label,
    display_server_value: Label,
    attention_box: GtkBox,
    on_navigate: NavigateCallback,
    /// Last rendered device specs. These are collected once at startup and do
    /// not change, so rewriting the labels on every tick only dirties widgets.
    last_device_info: RefCell<Option<DeviceInfo>>,
    /// Last rendered attention items. Rebuilding this list tears down and
    /// recreates every row, so it is skipped when the items are unchanged.
    last_attention: RefCell<Vec<AttentionItem>>,
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
        content.add_css_class("maintainer_overview_page");
        content.add_css_class("maintainer_page_content");

        let title = Label::new(Some("Overview"));
        title.add_css_class("maintainer_page_title");
        title.set_halign(Align::Start);
        content.append(&title);

        let subtitle = Label::new(Some(
            "System health summary, live resource metrics, hardware specifications, and items requiring attention.",
        ));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(Align::Start);
        subtitle.set_wrap(true);
        content.append(&subtitle);

        let cards_flow = FlowBox::new();
        cards_flow.set_selection_mode(SelectionMode::None);
        cards_flow.set_homogeneous(true);
        cards_flow.set_row_spacing(12);
        cards_flow.set_column_spacing(12);
        cards_flow.set_min_children_per_line(2);
        cards_flow.set_max_children_per_line(4);
        cards_flow.add_css_class("maintainer_cards_flow");

        let cpu_card = MetricCard::new("CPU", true);
        cards_flow.insert(cpu_card.widget(), -1);

        let memory_card = MetricCard::new("Memory", true);
        cards_flow.insert(memory_card.widget(), -1);

        let disk_card = MetricCard::new("Storage", false);
        cards_flow.insert(disk_card.widget(), -1);

        let network_card = MetricCard::new("Network", false);
        cards_flow.insert(network_card.widget(), -1);

        content.append(&cards_flow);

        let spec_card = GtkBox::new(Orientation::Vertical, 14);
        spec_card.add_css_class("maintainer_spec_card");
        spec_card.add_css_class("maintainer_surface");
        spec_card.set_margin_top(12);

        let spec_title = Label::new(Some("System Specifications"));
        spec_title.add_css_class("maintainer_spec_title");
        spec_title.set_halign(Align::Start);
        spec_card.append(&spec_title);

        let spec_grid = FlowBox::new();
        spec_grid.set_selection_mode(SelectionMode::None);
        spec_grid.set_row_spacing(12);
        spec_grid.set_column_spacing(16);
        spec_grid.set_min_children_per_line(2);
        spec_grid.set_max_children_per_line(3);
        spec_grid.set_homogeneous(true);

        let model_value = Label::new(None);
        let os_value = Label::new(None);
        let kernel_value = Label::new(None);
        let cpu_value = Label::new(None);
        let gpu_value = Label::new(None);
        let memory_value = Label::new(None);
        let storage_value = Label::new(None);
        let desktop_env_value = Label::new(None);
        let display_server_value = Label::new(None);

        spec_grid.insert(
            &build_spec_item("laptop-symbolic", "Computer", &model_value),
            -1,
        );
        spec_grid.insert(
            &build_spec_item(
                "software-update-available-symbolic",
                "Operating System",
                &os_value,
            ),
            -1,
        );
        spec_grid.insert(
            &build_spec_item("utilities-terminal-symbolic", "Linux Kernel", &kernel_value),
            -1,
        );
        spec_grid.insert(
            &build_spec_item("utilities-system-monitor-symbolic", "Processor", &cpu_value),
            -1,
        );
        spec_grid.insert(
            &build_spec_item("video-display-symbolic", "Graphics Card", &gpu_value),
            -1,
        );
        spec_grid.insert(
            &build_spec_item("media-memory-symbolic", "Memory", &memory_value),
            -1,
        );
        spec_grid.insert(
            &build_spec_item("drive-harddisk-symbolic", "Hard Drives", &storage_value),
            -1,
        );
        spec_grid.insert(
            &build_spec_item(
                "preferences-desktop-display-symbolic",
                "Desktop Environment",
                &desktop_env_value,
            ),
            -1,
        );
        spec_grid.insert(
            &build_spec_item(
                "display-projector-symbolic",
                "Display Server",
                &display_server_value,
            ),
            -1,
        );

        spec_card.append(&spec_grid);
        content.append(&spec_card);

        let attention_label = Label::new(Some("Attention"));
        attention_label.add_css_class("maintainer_section_title");
        attention_label.set_halign(Align::Start);
        attention_label.set_margin_top(16);
        content.append(&attention_label);

        let attention_box = GtkBox::new(Orientation::Vertical, 0);
        attention_box.add_css_class("maintainer_attention_list");
        content.append(&attention_box);

        let empty_row = GtkBox::new(Orientation::Horizontal, 12);
        empty_row.set_margin_top(12);
        empty_row.set_margin_bottom(12);
        empty_row.set_margin_start(16);
        empty_row.set_margin_end(16);
        empty_row.set_valign(Align::Center);

        let check_icon = Image::from_icon_name("object-select-symbolic");
        check_icon.set_pixel_size(16);
        check_icon.add_css_class("success");
        empty_row.append(&check_icon);

        let empty_label = Label::new(Some(
            "All systems operating within normal parameters. No issues detected.",
        ));
        empty_label.add_css_class("dim-label");
        empty_label.set_halign(Align::Start);
        empty_row.append(&empty_label);
        attention_box.append(&empty_row);

        let root = ScrolledWindow::new();
        root.set_child(Some(&content));
        root.set_hexpand(true);
        root.set_vexpand(true);

        let on_navigate: NavigateCallback = Rc::new(RefCell::new(None));

        Self {
            root,
            cpu_card,
            memory_card,
            disk_card,
            network_card,
            model_value,
            os_value,
            kernel_value,
            cpu_value,
            gpu_value,
            memory_value,
            storage_value,
            desktop_env_value,
            display_server_value,
            attention_box,
            on_navigate,
            last_device_info: RefCell::new(None),
            last_attention: RefCell::new(Vec::new()),
        }
    }

    /// Connect attention navigation callback.
    pub fn connect_navigate<F: Fn(Page) + 'static>(&self, callback: F) {
        *self.on_navigate.borrow_mut() = Some(Box::new(callback));
    }

    /// Update the page from the view model. Only touches widgets with changed data.
    pub fn update(&self, vm: &OverviewViewModel) {
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

        self.disk_card.set_value(&vm.disk_text);

        self.network_card.set_value(&vm.network_text);

        let specs_changed = self.last_device_info.borrow().as_ref() != Some(&vm.device_info);
        if specs_changed {
            self.model_value.set_text(&vm.device_info.model);
            self.os_value.set_text(&vm.device_info.os);
            self.kernel_value.set_text(&vm.device_info.kernel);
            self.cpu_value.set_text(&vm.device_info.cpu);
            self.gpu_value.set_text(&vm.device_info.gpu);
            self.memory_value.set_text(&vm.device_info.memory);
            self.storage_value.set_text(&vm.device_info.storage);
            self.desktop_env_value.set_text(&vm.device_info.desktop_env);
            self.display_server_value
                .set_text(&vm.device_info.display_server);
            *self.last_device_info.borrow_mut() = Some(vm.device_info.clone());
        }

        self.update_attention(&vm.attention_items);
    }

    fn update_attention(&self, items: &[AttentionItem]) {
        // repaint. Attention items change rarely, so skip identical updates.
        if self.last_attention.borrow().as_slice() == items {
            return;
        }
        self.last_attention.borrow_mut().clear();
        self.last_attention.borrow_mut().extend_from_slice(items);

        while let Some(child) = self.attention_box.first_child() {
            self.attention_box.remove(&child);
        }

        if items.is_empty() {
            let empty_row = GtkBox::new(Orientation::Horizontal, 12);
            empty_row.set_margin_top(12);
            empty_row.set_margin_bottom(12);
            empty_row.set_margin_start(16);
            empty_row.set_margin_end(16);
            empty_row.set_valign(Align::Center);

            let check_icon = Image::from_icon_name("object-select-symbolic");
            check_icon.set_pixel_size(16);
            check_icon.add_css_class("success");
            empty_row.append(&check_icon);

            let empty_label = Label::new(Some(
                "All systems operating within normal parameters. No issues detected.",
            ));
            empty_label.add_css_class("dim-label");
            empty_label.set_halign(Align::Start);
            empty_row.append(&empty_label);

            self.attention_box.append(&empty_row);
            return;
        }

        for item in items {
            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.add_css_class("maintainer_attention_item");
            row.set_margin_top(4);
            row.set_margin_bottom(4);
            row.set_margin_start(12);
            row.set_margin_end(12);

            let icon_name = match item.severity {
                AttentionSeverity::Critical => "dialog-error-symbolic",
                AttentionSeverity::Warning => "dialog-warning-symbolic",
                AttentionSeverity::Info => "dialog-information-symbolic",
            };

            let icon = Image::from_icon_name(icon_name);
            icon.set_pixel_size(16);
            let severity_text = match item.severity {
                AttentionSeverity::Critical => "Error",
                AttentionSeverity::Warning => "Warning",
                AttentionSeverity::Info => "Information",
            };
            icon.update_property(&[gtk4::accessible::Property::Label(severity_text)]);
            match item.severity {
                AttentionSeverity::Critical => icon.add_css_class("maintainer_icon_error"),
                AttentionSeverity::Warning => icon.add_css_class("maintainer_icon_warning"),
                AttentionSeverity::Info => {}
            }
            row.append(&icon);

            let label = Label::new(Some(&item.evidence));
            label.set_halign(Align::Start);
            label.set_hexpand(true);
            label.set_wrap(true);
            row.append(&label);

            let dest = item.destination;
            let action_btn = Button::with_label(dest.label());
            action_btn.add_css_class("maintainer_attention_button");
            let on_navigate = self.on_navigate.clone();
            action_btn.connect_clicked(move |_| {
                if let Some(cb) = on_navigate.borrow().as_ref() {
                    cb(dest);
                }
            });
            row.append(&action_btn);

            self.attention_box.append(&row);
        }
    }

    /// Get the root widget for embedding in the shell.
    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }
}

fn build_spec_item(icon_name: &'static str, label_text: &str, value_label: &Label) -> GtkBox {
    let item_box = GtkBox::new(Orientation::Horizontal, 10);
    item_box.add_css_class("maintainer_spec_item");

    let icon = Image::from_icon_name(icon_name);
    icon.set_pixel_size(18);
    icon.add_css_class("maintainer_text_muted");
    icon.set_valign(Align::Start);
    item_box.append(&icon);

    let text_box = GtkBox::new(Orientation::Vertical, 2);
    let label = Label::new(Some(label_text));
    label.add_css_class("maintainer_spec_label");
    label.set_halign(Align::Start);
    text_box.append(&label);

    value_label.add_css_class("maintainer_spec_value");
    value_label.set_halign(Align::Start);
    value_label.set_wrap(true);
    text_box.append(value_label);

    item_box.append(&text_box);
    item_box
}
