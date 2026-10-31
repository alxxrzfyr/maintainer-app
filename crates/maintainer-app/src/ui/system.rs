//! System Monitor page: live CPU, memory, disk, and temperature metrics.
//!
//! Network and process tables live on their dedicated pages (Network, Processes).
//! System retains only a compact network summary row showing total rates, and
//! uses grouped temperature readings (one chip per card) to avoid duplicates.

use std::cell::RefCell;
use std::collections::HashMap;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, FlowBox, Grid, Label, Orientation, ScrolledWindow, SelectionMode,
};

use crate::monitoring::SystemViewModel;
use crate::ui::icons;
use crate::ui::widgets::metric_card::MetricCard;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TemperatureSeverity {
    Normal,
    Warning,
    Critical,
}

fn temperature_severity(celsius: f64) -> TemperatureSeverity {
    if celsius >= 85.0 {
        TemperatureSeverity::Critical
    } else if celsius >= 70.0 {
        TemperatureSeverity::Warning
    } else {
        TemperatureSeverity::Normal
    }
}

struct DiskRow {
    row: GtkBox,
    detail: Label,
    fill: GtkBox,
}

struct TemperatureChip {
    wrapper: gtk4::FlowBoxChild,
    chip: GtkBox,
    name: Label,
    value: Label,
    summary: Label,
}

/// The system monitor page widget tree.
pub struct SystemPage {
    root: ScrolledWindow,
    cpu_card: MetricCard,
    memory_card: MetricCard,
    cpu_detail_box: GtkBox,
    memory_detail_box: GtkBox,
    disk_detail_box: GtkBox,
    network_summary_label: Label,
    temperature_flow: FlowBox,
    disk_rows: RefCell<HashMap<(String, String), DiskRow>>,
    disk_empty: Label,
    temperature_chips: RefCell<HashMap<(String, String), TemperatureChip>>,
    temperature_empty: Label,
}

impl Default for SystemPage {
    fn default() -> Self {
        Self::new()
    }
}

impl SystemPage {
    /// Create a new system monitor page.
    pub fn new() -> Self {
        let content = GtkBox::new(Orientation::Vertical, 16);
        content.add_css_class("maintainer_system_page");
        content.add_css_class("maintainer_page_content");

        let title = Label::new(Some("System Monitor"));
        title.add_css_class("maintainer_page_title");
        title.set_halign(Align::Start);
        content.append(&title);

        let subtitle = Label::new(Some(
            "Live resource metrics and hardware temperature sensors.",
        ));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(Align::Start);
        subtitle.set_wrap(true);
        content.append(&subtitle);

        let metrics_flow = FlowBox::new();
        metrics_flow.set_selection_mode(SelectionMode::None);
        metrics_flow.set_homogeneous(true);
        metrics_flow.set_row_spacing(12);
        metrics_flow.set_column_spacing(12);
        metrics_flow.set_min_children_per_line(1);
        metrics_flow.set_max_children_per_line(2);
        metrics_flow.add_css_class("maintainer_cards_flow");

        let cpu_card = MetricCard::new("CPU", true);
        metrics_flow.insert(cpu_card.widget(), -1);

        let memory_card = MetricCard::new("Memory", true);
        metrics_flow.insert(memory_card.widget(), -1);

        content.append(&metrics_flow);

        // CPU and memory detail grid.
        let detail_grid = Grid::new();
        detail_grid.set_column_homogeneous(true);
        detail_grid.set_column_spacing(12);
        detail_grid.add_css_class("maintainer_resource_grid");

        let cpu_detail_box = GtkBox::new(Orientation::Vertical, 4);
        cpu_detail_box.add_css_class("maintainer_detail_card");
        cpu_detail_box.set_hexpand(true);

        let cpu_detail_title = Label::new(Some("Per-Core Usage"));
        cpu_detail_title.add_css_class("maintainer_detail_card_title");
        cpu_detail_title.set_halign(Align::Start);
        cpu_detail_box.append(&cpu_detail_title);

        let memory_detail_box = GtkBox::new(Orientation::Vertical, 4);
        memory_detail_box.add_css_class("maintainer_detail_card");
        memory_detail_box.set_hexpand(true);

        let mem_detail_title = Label::new(Some("Memory Composition"));
        mem_detail_title.add_css_class("maintainer_detail_card_title");
        mem_detail_title.set_halign(Align::Start);
        memory_detail_box.append(&mem_detail_title);

        detail_grid.attach(&cpu_detail_box, 0, 0, 1, 1);
        detail_grid.attach(&memory_detail_box, 1, 0, 1, 1);
        content.append(&detail_grid);

        // Disk detail card.
        let disk_card = GtkBox::new(Orientation::Vertical, 4);
        disk_card.add_css_class("maintainer_detail_card");
        disk_card.set_hexpand(true);
        let disk_title = Label::new(Some("Disks"));
        disk_title.add_css_class("maintainer_detail_card_title");
        disk_title.set_halign(Align::Start);
        disk_card.append(&disk_title);
        let disk_detail_box = GtkBox::new(Orientation::Vertical, 2);
        let disk_empty = Label::new(Some("No disk information available"));
        disk_empty.add_css_class("maintainer_text_muted");
        disk_detail_box.append(&disk_empty);
        disk_card.append(&disk_detail_box);
        content.append(&disk_card);

        // Compact network summary (no per-interface table).
        let network_row = GtkBox::new(Orientation::Horizontal, 8);
        network_row.add_css_class("maintainer_detail_card");
        network_row.set_margin_top(4);

        let net_icon = icons::icon_with_fallback(icons::chains::NETWORK_ACTIVITY, 16);
        network_row.append(&net_icon);

        let network_summary_label = Label::new(Some("Network: —"));
        network_summary_label.set_halign(Align::Start);
        network_summary_label.set_hexpand(true);
        network_row.append(&network_summary_label);

        content.append(&network_row);

        // Temperature panel using grouped readings.
        let temperature_panel = GtkBox::new(Orientation::Vertical, 4);
        temperature_panel.add_css_class("maintainer_detail_card");
        temperature_panel.add_css_class("maintainer_temperature_panel");

        let temp_label = Label::new(Some("Temperatures"));
        temp_label.add_css_class("maintainer_detail_card_title");
        temp_label.set_halign(Align::Start);
        temperature_panel.append(&temp_label);

        let temperature_flow = FlowBox::new();
        temperature_flow.set_selection_mode(SelectionMode::None);
        temperature_flow.set_homogeneous(true);
        temperature_flow.set_row_spacing(6);
        temperature_flow.set_column_spacing(6);
        temperature_flow.set_min_children_per_line(1);
        temperature_flow.set_max_children_per_line(8);
        temperature_flow.add_css_class("maintainer_temperature_flow");
        let temperature_empty = Label::new(Some("No sensors detected"));
        temperature_empty.add_css_class("maintainer_text_muted");
        temperature_flow.insert(&temperature_empty, -1);
        temperature_panel.append(&temperature_flow);
        content.append(&temperature_panel);

        let root = ScrolledWindow::new();
        root.set_child(Some(&content));
        root.set_hexpand(true);
        root.set_vexpand(true);

        Self {
            root,
            cpu_card,
            memory_card,
            cpu_detail_box,
            memory_detail_box,
            disk_detail_box,
            network_summary_label,
            temperature_flow,
            disk_rows: RefCell::new(HashMap::new()),
            disk_empty,
            temperature_chips: RefCell::new(HashMap::new()),
            temperature_empty,
        }
    }

    /// Get the root widget for embedding in the shell.
    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }

    /// Update the page from the view model.
    pub fn update(&self, vm: &SystemViewModel) {
        if let Some(ref cpu) = vm.cpu {
            self.cpu_card.set_available();
            self.cpu_card
                .set_value(&format!("{:.0}%", cpu.total_percent));
            self.cpu_card.update_sparkline(
                vm.cpu_ring.samples(),
                &format!("CPU: {:.0}%", cpu.total_percent),
            );
            self.update_cpu_detail(cpu);
        }

        if let Some(ref mem) = vm.memory {
            self.memory_card.set_available();
            let used_gib = mem.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let total_gib = mem.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let text = format!("{used_gib:.1} / {total_gib:.1} GiB");
            self.memory_card.set_value(&text);
            self.memory_card
                .update_sparkline(vm.memory_ring.samples(), &format!("Memory: {text}"));
            self.update_memory_detail(mem);
        }

        self.update_disk_detail(&vm.disks);
        self.update_network_summary(&vm.networks);
        self.update_temperatures(vm);
    }

    /// Render per-core CPU breakdown bars.
    fn update_cpu_detail(&self, cpu: &maintainer_core::metrics::CpuSnapshot) {
        let current_children = count_children_of_class(&self.cpu_detail_box, "maintainer_core_row");
        if current_children == cpu.per_core_percent.len() {
            update_core_bars_inplace(&self.cpu_detail_box, &cpu.per_core_percent);
            return;
        }

        remove_children_except_first(&self.cpu_detail_box);

        for (i, &pct) in cpu.per_core_percent.iter().enumerate() {
            let row = GtkBox::new(Orientation::Horizontal, 8);
            row.add_css_class("maintainer_core_row");

            let label = Label::new(Some(&format!("Core {}", i + 1)));
            label.add_css_class("maintainer_core_label");
            label.set_halign(Align::Start);
            row.append(&label);

            let bar_trough = GtkBox::new(Orientation::Horizontal, 0);
            bar_trough.add_css_class("maintainer_core_bar_trough");
            bar_trough.set_hexpand(true);

            let bar_fill = GtkBox::new(Orientation::Horizontal, 0);
            bar_fill.add_css_class("maintainer_core_bar_fill");
            let fill_width = ((pct / 100.0) * 200.0) as i32;
            bar_fill.set_size_request(fill_width.max(1), -1);
            bar_trough.append(&bar_fill);
            row.append(&bar_trough);

            let pct_label = Label::new(Some(&format!("{pct:.0}%")));
            pct_label.add_css_class("maintainer_core_percent");
            pct_label.set_halign(Align::End);
            row.append(&pct_label);

            self.cpu_detail_box.append(&row);
        }
    }

    /// Render memory composition (used, buffers/cache, available).
    fn update_memory_detail(&self, mem: &maintainer_core::metrics::MemorySnapshot) {
        remove_children_except_first(&self.memory_detail_box);

        let segments = [
            ("Used", mem.used_bytes),
            ("Buffers/Cache", mem.buffers_bytes + mem.cached_bytes),
            ("Available", mem.available_bytes),
            ("Total", mem.total_bytes),
        ];

        for (name, bytes) in segments {
            let row = GtkBox::new(Orientation::Horizontal, 12);
            row.set_margin_top(2);
            row.set_margin_bottom(2);

            let label = Label::new(Some(name));
            label.add_css_class("maintainer_mem_segment_label");
            label.set_halign(Align::Start);
            label.set_hexpand(true);
            row.append(&label);

            let value = Label::new(Some(&format_gib(bytes)));
            value.add_css_class("maintainer_mem_segment_value");
            value.set_halign(Align::End);
            row.append(&value);

            self.memory_detail_box.append(&row);
        }
    }

    /// Render per-disk breakdown with usage bars.
    fn update_disk_detail(&self, disks: &[maintainer_core::metrics::DiskSnapshot]) {
        let mut rows = self.disk_rows.borrow_mut();
        rows.retain(|(device, mount), cached| {
            let present = disks
                .iter()
                .any(|disk| &disk.device == device && &disk.mount_point == mount);
            if !present {
                self.disk_detail_box.remove(&cached.row);
            }
            present
        });
        self.disk_empty.set_visible(disks.is_empty());
        let mut previous: Option<GtkBox> = None;

        for disk in disks {
            let cached = rows
                .entry((disk.device.clone(), disk.mount_point.clone()))
                .or_insert_with(|| {
                    let row = GtkBox::new(Orientation::Vertical, 2);
                    row.add_css_class("maintainer_disk_row");

                    let header = GtkBox::new(Orientation::Horizontal, 8);
                    let name = Label::new(Some(&disk.mount_point));
                    name.add_css_class("maintainer_disk_name");
                    name.set_halign(Align::Start);
                    name.set_hexpand(true);
                    name.set_ellipsize(gtk4::pango::EllipsizeMode::Middle);
                    header.append(&name);

                    let detail = Label::new(None);
                    detail.add_css_class("maintainer_disk_detail");
                    detail.set_halign(Align::End);
                    header.append(&detail);

                    row.append(&header);

                    let bar_trough = GtkBox::new(Orientation::Horizontal, 0);
                    bar_trough.add_css_class("maintainer_core_bar_trough");
                    bar_trough.set_hexpand(true);

                    let bar_fill = GtkBox::new(Orientation::Horizontal, 0);
                    bar_fill.add_css_class("maintainer_core_bar_fill");
                    bar_trough.append(&bar_fill);
                    row.append(&bar_trough);

                    self.disk_detail_box.append(&row);
                    DiskRow {
                        row,
                        detail,
                        fill: bar_fill,
                    }
                });
            let pct = if disk.total_bytes > 0 {
                disk.used_bytes as f64 / disk.total_bytes as f64 * 100.0
            } else {
                0.0
            };
            cached.detail.set_text(&format!(
                "{} / {} ({pct:.0}%)",
                format_gib(disk.used_bytes),
                format_gib(disk.total_bytes)
            ));
            cached
                .fill
                .set_size_request(((pct / 100.0 * 200.0) as i32).max(1), -1);
            cached
                .fill
                .remove_css_class("maintainer_capacity_bar_fill_critical");
            cached
                .fill
                .remove_css_class("maintainer_capacity_bar_fill_warning");
            if pct >= 90.0 {
                cached
                    .fill
                    .add_css_class("maintainer_capacity_bar_fill_critical");
            } else if pct >= 80.0 {
                cached
                    .fill
                    .add_css_class("maintainer_capacity_bar_fill_warning");
            }
            self.disk_detail_box
                .reorder_child_after(&cached.row, previous.as_ref());
            previous = Some(cached.row.clone());
        }
    }

    /// Compact network summary (total rates only; detail is on Network page).
    fn update_network_summary(&self, networks: &[maintainer_core::metrics::NetworkSnapshot]) {
        let non_loopback: Vec<_> = networks.iter().filter(|n| !n.is_loopback).collect();
        if non_loopback.is_empty() {
            self.network_summary_label
                .set_text("Network: no active interfaces");
            self.network_summary_label
                .add_css_class("maintainer_text_muted");
            return;
        }

        self.network_summary_label
            .remove_css_class("maintainer_text_muted");
        let total_rx: f64 = non_loopback.iter().filter_map(|n| n.rx_bytes_per_sec).sum();
        let total_tx: f64 = non_loopback.iter().filter_map(|n| n.tx_bytes_per_sec).sum();
        self.network_summary_label.set_text(&format!(
            "Network: ↓ {}/s  ↑ {}/s ({} interface{})",
            format_rate_bytes(total_rx),
            format_rate_bytes(total_tx),
            non_loopback.len(),
            if non_loopback.len() == 1 { "" } else { "s" }
        ));
    }

    /// Render temperatures using grouped readings (one chip per card).
    ///
    /// Filters invalid 0°C phantom readings (non-reporting sensors in Linux hwmon),
    /// deduplicates identical label/source channels, humanizes labels, and limits
    /// sub-sensor expansion to avoid tall chips. Details are exposed via tooltip.
    fn update_temperatures(&self, vm: &SystemViewModel) {
        let mut chips = self.temperature_chips.borrow_mut();
        chips.retain(|(chip_name, source), cached| {
            let present = vm.temperature_groups.iter().any(|group| {
                &group.chip_name == chip_name && &group.representative.hwmon_name == source
                    && matches!(group.representative.celsius, maintainer_core::capability::Capability::Available(c) if c > 0.0)
            });
            if !present {
                self.temperature_flow.remove(&cached.wrapper);
            }
            present
        });
        let mut displayed = 0_usize;
        for group in &vm.temperature_groups {
            if let maintainer_core::capability::Capability::Available(celsius) =
                &group.representative.celsius
            {
                // Filter invalid/unavailable phantom 0°C readings.
                // 0°C indicates a non-reporting sensor in Linux hwmon — omit
                // rather than display.
                if *celsius <= 0.0 {
                    continue;
                }

                displayed += 1;
                let chip_label = humanize_sensor_label(
                    group
                        .representative
                        .label
                        .as_deref()
                        .unwrap_or(&group.chip_name),
                );

                let cached = chips
                    .entry((
                        group.chip_name.clone(),
                        group.representative.hwmon_name.clone(),
                    ))
                    .or_insert_with(|| {
                        let chip = GtkBox::new(Orientation::Vertical, 2);
                        chip.add_css_class("maintainer_temperature_chip");

                        let header_row = GtkBox::new(Orientation::Horizontal, 6);
                        let temp_icon = icons::icon_with_fallback(icons::chains::TEMPERATURE, 12);
                        temp_icon.add_css_class("maintainer_text_muted");
                        header_row.append(&temp_icon);

                        let name_label = Label::new(Some(&chip_label));
                        name_label.add_css_class("maintainer_temperature_label");
                        name_label.set_halign(Align::Start);
                        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                        name_label.set_max_width_chars(18);
                        header_row.append(&name_label);
                        chip.append(&header_row);

                        let value_label = Label::new(Some(&format!("{celsius:.0}°C")));
                        value_label.add_css_class("maintainer_temperature_value");
                        value_label.set_halign(Align::Start);

                        chip.append(&value_label);
                        let summary = Label::new(None);
                        summary.add_css_class("maintainer_text_muted");
                        summary.set_halign(Align::Start);
                        chip.append(&summary);
                        let wrapper = gtk4::FlowBoxChild::new();
                        wrapper.set_child(Some(&chip));
                        self.temperature_flow.insert(&wrapper, -1);
                        TemperatureChip {
                            wrapper,
                            chip,
                            name: name_label,
                            value: value_label,
                            summary,
                        }
                    });
                let chip = &cached.chip;
                let value_label = &cached.value;
                cached.name.set_text(&chip_label);
                value_label.set_text(&format!("{celsius:.0}°C"));
                value_label.remove_css_class("maintainer_temperature_critical");
                value_label.remove_css_class("maintainer_temperature_warning");
                match temperature_severity(*celsius) {
                    TemperatureSeverity::Critical => {
                        value_label.add_css_class("maintainer_temperature_critical");
                        value_label.set_text(&format!("{celsius:.0}°C · Critical"));
                    }
                    TemperatureSeverity::Warning => {
                        value_label.add_css_class("maintainer_temperature_warning");
                        value_label.set_text(&format!("{celsius:.0}°C · Warm"));
                    }
                    TemperatureSeverity::Normal => {}
                }

                value_label.update_property(&[gtk4::accessible::Property::Description(&format!(
                    "{chip_label} temperature {celsius:.0} degrees Celsius"
                ))]);

                // Build tooltip with sub-sensor details instead of expanding chip height.
                let mut tooltip_parts = vec![format!("{chip_label}: {celsius:.0}°C")];
                let valid_details: Vec<_> = group
                    .details
                    .iter()
                    .filter(|d| {
                        if let maintainer_core::capability::Capability::Available(c) = &d.celsius {
                            // Skip 0°C phantom readings and deduplicate representative.
                            if *c <= 0.0 {
                                return false;
                            }
                            // Skip exact duplicate of representative.
                            if d.label == group.representative.label
                                && d.hwmon_name == group.representative.hwmon_name
                            {
                                if let maintainer_core::capability::Capability::Available(rep_c) =
                                    &group.representative.celsius
                                {
                                    if (c - rep_c).abs() < 0.01 {
                                        return false;
                                    }
                                }
                            }
                            true
                        } else {
                            false
                        }
                    })
                    .collect();

                if !valid_details.is_empty() {
                    for detail in &valid_details {
                        if let maintainer_core::capability::Capability::Available(sub_c) =
                            &detail.celsius
                        {
                            let sub_name = humanize_sensor_label(
                                detail.label.as_deref().unwrap_or(&detail.hwmon_name),
                            );
                            tooltip_parts.push(format!("  {sub_name}: {sub_c:.0}°C"));
                        }
                    }
                }

                chip.set_tooltip_text(Some(&tooltip_parts.join("\n")));

                let summary_text = if valid_details.len() <= 3 {
                    valid_details
                        .iter()
                        .filter_map(|d| {
                            if let maintainer_core::capability::Capability::Available(c) =
                                &d.celsius
                            {
                                Some(format!("{c:.0}°C"))
                            } else {
                                None
                            }
                        })
                        .collect::<Vec<_>>()
                        .join(" · ")
                } else {
                    format!("+{} sensors", valid_details.len())
                };
                cached.summary.set_text(&summary_text);
                cached.summary.set_visible(!summary_text.is_empty());
                let index = (displayed - 1) as i32;
                if cached.wrapper.index() != index {
                    self.temperature_flow.remove(&cached.wrapper);
                    self.temperature_flow.insert(&cached.wrapper, index);
                }
            }
        }

        self.temperature_empty.set_visible(displayed == 0);
        if displayed == 0 {
            let no_sensors = vm.temperature_groups.is_empty();
            self.temperature_empty.set_text(if no_sensors {
                "No sensors detected"
            } else {
                "Sensor readings unavailable"
            });
            self.temperature_empty.set_tooltip_text(if no_sensors {
                None
            } else {
                Some("Sensors were detected, but Linux could not read their current values.")
            });
        }
    }
}

/// Humanize hardware sensor labels for display.
///
/// Converts kernel-style identifiers (e.g., "thinkpad_acpi", "pch_cannonlake",
/// "Package id 0") into friendlier forms.
fn humanize_sensor_label(raw: &str) -> String {
    match raw {
        "coretemp" => "CPU Package".to_string(),
        "acpitz" => "ACPI Thermal".to_string(),
        "iwlwifi_1" | "iwlwifi" => "Wi-Fi Adapter".to_string(),
        "pch_cannonlake" | "pch_skylake" | "pch_haswell" => "PCH".to_string(),
        "thinkpad" | "thinkpad_acpi" => "ThinkPad EC".to_string(),
        "nvme" | "nvme0" => "NVMe SSD".to_string(),
        _ if raw.starts_with("Package id") => "CPU Package".to_string(),
        _ if raw.starts_with("Core ") => raw.to_string(),
        _ => {
            // Replace underscores with spaces and title-case first letter.
            let cleaned = raw.replace('_', " ");
            let mut chars = cleaned.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().to_string() + chars.as_str(),
            }
        }
    }
}

/// Format bytes as GiB for disk/memory display.
fn format_gib(bytes: u64) -> String {
    let gib = bytes as f64 / (1024.0 * 1024.0 * 1024.0);
    if gib >= 1.0 {
        format!("{gib:.1} GiB")
    } else {
        let mib = bytes as f64 / (1024.0 * 1024.0);
        format!("{mib:.0} MiB")
    }
}

/// Format bytes/s rate.
fn format_rate_bytes(rate: f64) -> String {
    if rate >= 1_000_000_000.0 {
        format!("{:.1} GB", rate / 1_000_000_000.0)
    } else if rate >= 1_000_000.0 {
        format!("{:.1} MB", rate / 1_000_000.0)
    } else if rate >= 1_000.0 {
        format!("{:.1} KB", rate / 1_000.0)
    } else {
        format!("{:.0} B", rate)
    }
}

/// Count children with a given CSS class.
fn count_children_of_class(container: &GtkBox, class: &str) -> usize {
    let mut count = 0;
    let mut child = container.first_child();
    while let Some(widget) = child {
        if widget.has_css_class(class) {
            count += 1;
        }
        child = widget.next_sibling();
    }
    count
}

/// Remove all children except the first (the title label).
fn remove_children_except_first(container: &GtkBox) {
    let first = container.first_child();
    if let Some(first_widget) = first {
        let mut sibling = first_widget.next_sibling();
        while let Some(widget) = sibling {
            let next = widget.next_sibling();
            container.remove(&widget);
            sibling = next;
        }
    }
}

/// Update existing per-core bars in-place without rebuilding.
fn update_core_bars_inplace(container: &GtkBox, percentages: &[f64]) {
    let mut child = container.first_child();
    if let Some(w) = child {
        child = w.next_sibling();
    }

    for &pct in percentages {
        let Some(widget) = child else { break };
        if let Some(row) = widget.downcast_ref::<GtkBox>() {
            let mut row_child = row.first_child();
            if let Some(w) = row_child {
                row_child = w.next_sibling();
            }
            if let Some(trough) = row_child.as_ref().and_then(|w| w.downcast_ref::<GtkBox>()) {
                if let Some(fill) = trough
                    .first_child()
                    .and_then(|w| w.downcast::<GtkBox>().ok())
                {
                    let fill_width = ((pct / 100.0) * 200.0) as i32;
                    fill.set_size_request(fill_width.max(1), -1);
                }
                row_child = row_child.as_ref().and_then(|w| w.next_sibling());
            }
            if let Some(pct_widget) = row_child {
                if let Some(pct_label) = pct_widget.downcast_ref::<Label>() {
                    pct_label.set_text(&format!("{pct:.0}%"));
                }
            }
        }
        child = widget.next_sibling();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disk_and_sensor_widgets_survive_reorder_and_remove_stale_entries() {
        // Run this GTK fixture alone under xvfb so GTK stays on its owning thread.
        assert!(
            !gtk4::is_initialized() || gtk4::is_initialized_main_thread(),
            "Run each GTK fixture in its own process using scripts/test-native.sh"
        );
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        use maintainer_core::capability::Capability;
        use maintainer_core::metrics::{DiskSnapshot, TemperatureGroup, TemperatureReading};
        let disk = |name: &str, used_bytes| DiskSnapshot {
            device: name.into(),
            mount_point: format!("/{name}"),
            fs_type: "ext4".into(),
            total_bytes: 100,
            used_bytes,
            available_bytes: 100 - used_bytes,
            read_bytes_per_sec: None,
            write_bytes_per_sec: None,
        };
        let group = |name: &str, celsius| TemperatureGroup {
            chip_name: name.into(),
            representative: TemperatureReading {
                hwmon_name: name.into(),
                label: None,
                celsius: Capability::Available(celsius),
            },
            details: Vec::new(),
        };
        let page = SystemPage::new();
        let first = disk("first", 95);
        let second = disk("second", 30);
        page.update_disk_detail(&[first.clone(), second.clone()]);
        let disk_key = ("first".into(), "/first".into());
        let disk_row = page.disk_rows.borrow()[&disk_key].row.clone();
        let disk_fill = page.disk_rows.borrow()[&disk_key].fill.clone();
        assert!(disk_fill.has_css_class("maintainer_capacity_bar_fill_critical"));
        page.update_disk_detail(&[second, disk("first", 20)]);
        assert_eq!(page.disk_rows.borrow()[&disk_key].row, disk_row);
        assert_eq!(page.disk_rows.borrow()[&disk_key].fill, disk_fill);
        assert!(!disk_fill.has_css_class("maintainer_capacity_bar_fill_critical"));
        assert_eq!(disk_fill.width_request(), 40);
        assert_eq!(
            page.disk_detail_box.first_child().unwrap(),
            page.disk_rows.borrow()[&("second".into(), "/second".into())]
                .row
                .clone()
                .upcast::<gtk4::Widget>()
        );
        page.update_disk_detail(&[first]);
        assert_eq!(page.disk_rows.borrow().len(), 1);
        page.update_disk_detail(&[]);
        assert!(disk_row.parent().is_none());
        assert!(page.disk_empty.is_visible());

        let mut vm = SystemViewModel {
            temperature_groups: vec![group("first", 90.0), group("second", 40.0)],
            ..Default::default()
        };
        page.update_temperatures(&vm);
        let key = ("first".into(), "first".into());
        let chip = page.temperature_chips.borrow()[&key].wrapper.clone();
        let value = page.temperature_chips.borrow()[&key].value.clone();
        vm.temperature_groups = vec![group("second", 40.0), group("first", 50.0)];
        page.update_temperatures(&vm);
        assert_eq!(page.temperature_chips.borrow()[&key].wrapper, chip);
        assert_eq!(page.temperature_chips.borrow()[&key].value, value);
        assert_eq!(chip.index(), 1);
        assert_eq!(value.text(), "50°C");
        assert!(!value.has_css_class("maintainer_temperature_critical"));
        vm.temperature_groups.remove(0);
        page.update_temperatures(&vm);
        assert_eq!(page.temperature_chips.borrow().len(), 1);
        assert_eq!(chip.index(), 0);
        vm.temperature_groups.clear();
        page.update_temperatures(&vm);
        assert!(chip.parent().is_none());
        assert!(page.temperature_empty.is_visible());
        assert_eq!(page.temperature_empty.text(), "No sensors detected");
    }

    #[test]
    fn temperature_thresholds_match_ui_policy() {
        assert_eq!(temperature_severity(69.9), TemperatureSeverity::Normal);
        assert_eq!(temperature_severity(70.0), TemperatureSeverity::Warning);
        assert_eq!(temperature_severity(84.9), TemperatureSeverity::Warning);
        assert_eq!(temperature_severity(85.0), TemperatureSeverity::Critical);
    }
}
