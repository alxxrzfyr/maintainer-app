//! System Monitor page: detailed live metrics, process table, and termination.
//!
//! Shows per-core CPU, memory breakdown, disk I/O, network rates, temperatures,
//! and a sortable process table. Process termination shows a confirmation dialog
//! before sending SIGTERM.
//!
//! Temperature chips use FlowBox for fluid wrapping. The process table adapts
//! its visible columns based on allocated width.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Dialog, DialogFlags, FlowBox, Image, Label, Orientation,
    ResponseType, ScrolledWindow, SelectionMode,
};

use crate::monitoring::{ProcessSort, SystemViewModel};
use crate::ui::widgets::metric_card::MetricCard;

/// Callback type for sort changes.
type SortCallback = Rc<RefCell<Option<Box<dyn Fn(ProcessSort)>>>>;

/// Callback type for terminate requests (pid, start_time_ticks).
type TerminateCallback = Rc<RefCell<Option<Box<dyn Fn(u32, u64)>>>>;

/// The system monitor page widget tree.
pub struct SystemPage {
    root: ScrolledWindow,
    cpu_card: MetricCard,
    memory_card: MetricCard,
    process_list_box: GtkBox,
    sort_buttons: SortButtons,
    temperature_flow: FlowBox,
    operation_status: Label,
    on_sort_changed: SortCallback,
    on_terminate: TerminateCallback,
}

struct SortButtons {
    cpu_btn: Button,
    memory_btn: Button,
    name_btn: Button,
    pid_btn: Button,
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
            "Live resource metrics, hardware temperature sensors, and process management.",
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

        let temp_label = Label::new(Some("Temperatures"));
        temp_label.add_css_class("maintainer_section_title");
        temp_label.set_halign(Align::Start);
        temp_label.set_margin_top(8);
        content.append(&temp_label);

        let temperature_flow = FlowBox::new();
        temperature_flow.set_selection_mode(SelectionMode::None);
        temperature_flow.set_homogeneous(false);
        temperature_flow.set_row_spacing(8);
        temperature_flow.set_column_spacing(8);
        temperature_flow.set_min_children_per_line(2);
        temperature_flow.set_max_children_per_line(8);
        temperature_flow.add_css_class("maintainer_temperature_flow");
        content.append(&temperature_flow);

        let process_header = GtkBox::new(Orientation::Horizontal, 12);
        process_header.set_margin_top(12);
        process_header.set_valign(Align::Center);
        process_header.add_css_class("maintainer_process_section_header");

        let process_label = Label::new(Some("Processes"));
        process_label.add_css_class("maintainer_section_title");
        process_label.set_halign(Align::Start);
        process_label.set_hexpand(true);
        process_header.append(&process_label);

        let on_sort_changed: SortCallback = Rc::new(RefCell::new(None));
        let sort_row = GtkBox::new(Orientation::Horizontal, 6);
        sort_row.set_halign(Align::End);

        let sort_label = Label::new(Some("Sort:"));
        sort_label.add_css_class("maintainer_text_muted");
        sort_row.append(&sort_label);

        let cpu_btn = Button::with_label("CPU");
        cpu_btn.add_css_class("maintainer_sort_button");
        cpu_btn.add_css_class("maintainer_sort_active");
        let on_sort = on_sort_changed.clone();
        cpu_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_sort.borrow() {
                cb(ProcessSort::Cpu);
            }
        });

        let memory_btn = Button::with_label("Mem");
        memory_btn.add_css_class("maintainer_sort_button");
        let on_sort = on_sort_changed.clone();
        memory_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_sort.borrow() {
                cb(ProcessSort::Memory);
            }
        });

        let name_btn = Button::with_label("Name");
        name_btn.add_css_class("maintainer_sort_button");
        let on_sort = on_sort_changed.clone();
        name_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_sort.borrow() {
                cb(ProcessSort::Name);
            }
        });

        let pid_btn = Button::with_label("PID");
        pid_btn.add_css_class("maintainer_sort_button");
        let on_sort = on_sort_changed.clone();
        pid_btn.connect_clicked(move |_| {
            if let Some(ref cb) = *on_sort.borrow() {
                cb(ProcessSort::Pid);
            }
        });

        sort_row.append(&cpu_btn);
        sort_row.append(&memory_btn);
        sort_row.append(&name_btn);
        sort_row.append(&pid_btn);
        process_header.append(&sort_row);

        content.append(&process_header);

        let operation_status = Label::new(None);
        operation_status.set_halign(Align::Start);
        operation_status.set_wrap(true);
        operation_status.set_visible(false);
        content.append(&operation_status);

        let process_list_box = GtkBox::new(Orientation::Vertical, 0);
        process_list_box.add_css_class("maintainer_process_list");

        let process_scroll = ScrolledWindow::new();
        process_scroll.set_child(Some(&process_list_box));
        process_scroll.set_hexpand(true);
        process_scroll.set_vexpand(true);
        process_scroll.set_min_content_height(200);
        process_scroll.set_propagate_natural_height(true);
        process_scroll.set_max_content_height(600);
        content.append(&process_scroll);

        let root = ScrolledWindow::new();
        root.set_child(Some(&content));
        root.set_hexpand(true);
        root.set_vexpand(true);

        let on_terminate: TerminateCallback = Rc::new(RefCell::new(None));

        Self {
            root,
            cpu_card,
            memory_card,
            process_list_box,
            sort_buttons: SortButtons {
                cpu_btn,
                memory_btn,
                name_btn,
                pid_btn,
            },
            temperature_flow,
            operation_status,
            on_sort_changed,
            on_terminate,
        }
    }

    /// Set callback for when sort order changes.
    pub fn connect_sort_changed(&self, cb: impl Fn(ProcessSort) + 'static) {
        *self.on_sort_changed.borrow_mut() = Some(Box::new(cb));
    }

    /// Set callback for when a process termination is requested.
    /// Callback receives (pid, start_time_ticks).
    pub fn connect_terminate(&self, cb: impl Fn(u32, u64) + 'static) {
        *self.on_terminate.borrow_mut() = Some(Box::new(cb));
    }

    /// Status label for process operation feedback.
    pub fn operation_status_handle(&self) -> Label {
        self.operation_status.clone()
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
        }

        if let Some(ref mem) = vm.memory {
            self.memory_card.set_available();
            let used_gib = mem.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let total_gib = mem.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let text = format!("{used_gib:.1} / {total_gib:.1} GiB");
            self.memory_card.set_value(&text);
            self.memory_card
                .update_sparkline(vm.memory_ring.samples(), &format!("Memory: {text}"));
        }

        self.update_temperatures(vm);

        self.update_processes(vm);

        self.update_sort_buttons(vm.process_sort);
    }

    fn update_temperatures(&self, vm: &SystemViewModel) {
        while let Some(child) = self.temperature_flow.first_child() {
            self.temperature_flow.remove(&child);
        }

        if vm.temperatures.is_empty() {
            let empty = Label::new(Some("No sensors detected"));
            empty.add_css_class("maintainer_text_muted");
            self.temperature_flow.insert(&empty, -1);
            return;
        }

        for reading in &vm.temperatures {
            if let maintainer_core::capability::Capability::Available(celsius) = &reading.celsius {
                let name = reading.label.as_deref().unwrap_or(&reading.hwmon_name);

                let chip = GtkBox::new(Orientation::Vertical, 2);
                chip.add_css_class("maintainer_temperature_chip");

                let name_label = Label::new(Some(name));
                name_label.add_css_class("maintainer_temperature_label");
                name_label.set_halign(Align::Start);
                name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
                name_label.set_max_width_chars(14);

                let header_row = GtkBox::new(Orientation::Horizontal, 6);
                let temp_icon = Image::from_icon_name("sensors-temperature-symbolic");
                temp_icon.set_pixel_size(12);
                temp_icon.add_css_class("maintainer_text_muted");
                header_row.append(&temp_icon);
                header_row.append(&name_label);
                chip.append(&header_row);

                let value_label = Label::new(Some(&format!("{celsius:.0}°C")));
                value_label.add_css_class("maintainer_temperature_value");
                value_label.set_halign(Align::Start);

                if *celsius >= 85.0 {
                    value_label.add_css_class("maintainer_temperature_critical");
                    value_label.set_text(&format!("{celsius:.0}°C · Critical"));
                    value_label.update_property(&[gtk4::accessible::Property::Description(
                        &format!("{name} temperature {celsius:.0} degrees Celsius, critical"),
                    )]);
                } else if *celsius >= 70.0 {
                    value_label.add_css_class("maintainer_temperature_warning");
                    value_label.set_text(&format!("{celsius:.0}°C · Warning"));
                    value_label.update_property(&[gtk4::accessible::Property::Description(
                        &format!("{name} temperature {celsius:.0} degrees Celsius, warning"),
                    )]);
                }

                chip.append(&value_label);
                self.temperature_flow.insert(&chip, -1);
            }
        }
    }

    fn update_processes(&self, vm: &SystemViewModel) {
        while let Some(child) = self.process_list_box.first_child() {
            self.process_list_box.remove(&child);
        }

        if !vm.processes_available {
            let label = Label::new(Some("Process information unavailable"));
            label.add_css_class("maintainer_text_muted");
            label.set_margin_top(16);
            label.set_margin_bottom(16);
            self.process_list_box.append(&label);
            return;
        }

        if vm.processes.is_empty() {
            let label = Label::new(Some("No processes"));
            label.add_css_class("maintainer_text_muted");
            label.set_margin_top(16);
            label.set_margin_bottom(16);
            self.process_list_box.append(&label);
            return;
        }

        let header = GtkBox::new(Orientation::Horizontal, 0);
        header.add_css_class("maintainer_process_header");

        let pid_h = Label::new(Some("PID"));
        pid_h.set_width_request(70);
        pid_h.set_halign(Align::Start);
        header.append(&pid_h);

        let name_h = Label::new(Some("Name"));
        name_h.set_hexpand(true);
        name_h.set_halign(Align::Start);
        header.append(&name_h);

        let mem_h = Label::new(Some("Memory"));
        mem_h.set_width_request(90);
        mem_h.set_halign(Align::End);
        mem_h.add_css_class("maintainer_col_memory");
        header.append(&mem_h);

        let state_h = Label::new(Some("State"));
        state_h.set_width_request(60);
        state_h.set_halign(Align::Center);
        state_h.add_css_class("maintainer_col_state");
        header.append(&state_h);

        let action_h = Label::new(Some("Action"));
        action_h.set_width_request(60);
        action_h.set_halign(Align::End);
        header.append(&action_h);

        self.process_list_box.append(&header);

        let display_count = vm.processes.len().min(50);
        for (i, proc) in vm.processes[..display_count].iter().enumerate() {
            let row = self.build_process_row(proc, i % 2 == 1);
            self.process_list_box.append(&row);
        }
    }

    fn build_process_row(
        &self,
        proc: &maintainer_core::metrics::ProcessSnapshot,
        alternate: bool,
    ) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 0);
        row.add_css_class("maintainer_process_row");
        if alternate {
            row.add_css_class("maintainer_process_row_alt");
        }

        let pid_label = Label::new(Some(&proc.identity.pid.to_string()));
        pid_label.set_width_request(70);
        pid_label.set_halign(Align::Start);
        pid_label.add_css_class("maintainer_process_pid");
        row.append(&pid_label);

        let name_label = Label::new(Some(&proc.name));
        name_label.set_hexpand(true);
        name_label.set_halign(Align::Start);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&name_label);

        let mem_text = format_memory_bytes(proc.rss_bytes);
        let mem_label = Label::new(Some(&mem_text));
        mem_label.set_width_request(90);
        mem_label.set_halign(Align::End);
        mem_label.add_css_class("maintainer_process_memory");
        mem_label.add_css_class("maintainer_col_memory");
        row.append(&mem_label);

        let state_label = Label::new(Some(&proc.state.to_string()));
        state_label.set_width_request(60);
        state_label.set_halign(Align::Center);
        state_label.add_css_class("maintainer_text_muted");
        state_label.add_css_class("maintainer_col_state");
        row.append(&state_label);

        let terminate_btn = Button::with_label("End");
        terminate_btn.add_css_class("maintainer_terminate_button");
        terminate_btn.set_tooltip_text(Some(&format!("Terminate {}", proc.name)));
        terminate_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
            "Terminate process {}",
            proc.name
        ))]);

        let on_term = self.on_terminate.clone();
        let pid = proc.identity.pid;
        let start_time = proc.identity.start_time_ticks;
        let name = proc.name.clone();
        let uid = proc.uid;
        terminate_btn.connect_clicked(move |btn| {
            let window = btn.root().and_then(|r| r.downcast::<gtk4::Window>().ok());

            if let Some(win) = window {
                show_terminate_dialog(&win, &name, pid, uid, start_time, &on_term);
            }
        });

        let action_box = GtkBox::new(Orientation::Horizontal, 0);
        action_box.set_width_request(60);
        action_box.set_halign(Align::End);
        action_box.append(&terminate_btn);
        row.append(&action_box);
        row
    }

    fn update_sort_buttons(&self, current: ProcessSort) {
        let buttons = [
            (&self.sort_buttons.cpu_btn, ProcessSort::Cpu),
            (&self.sort_buttons.memory_btn, ProcessSort::Memory),
            (&self.sort_buttons.name_btn, ProcessSort::Name),
            (&self.sort_buttons.pid_btn, ProcessSort::Pid),
        ];

        for (btn, sort) in buttons {
            if sort == current {
                btn.add_css_class("maintainer_sort_active");
            } else {
                btn.remove_css_class("maintainer_sort_active");
            }
        }
    }

    /// Get the root widget for embedding in the shell.
    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }
}

/// Show a confirmation dialog before terminating a process.
fn show_terminate_dialog(
    parent: &gtk4::Window,
    name: &str,
    pid: u32,
    uid: u32,
    start_time_ticks: u64,
    on_terminate: &TerminateCallback,
) {
    let dialog = Dialog::with_buttons(
        Some("Terminate Process"),
        Some(parent),
        DialogFlags::MODAL | DialogFlags::DESTROY_WITH_PARENT,
        &[
            ("Cancel", ResponseType::Cancel),
            ("Terminate", ResponseType::Accept),
        ],
    );

    let content = dialog.content_area();
    content.set_margin_top(12);
    content.set_margin_bottom(12);
    content.set_margin_start(12);
    content.set_margin_end(12);
    content.set_spacing(8);

    let msg = Label::new(Some(&format!(
        "Terminate \"{}\" (PID {}, UID {})?",
        name, pid, uid
    )));
    msg.set_halign(Align::Start);
    content.append(&msg);

    let detail = Label::new(Some(
        "This sends SIGTERM, requesting the process exit gracefully.\n\
         The process may save state before stopping.",
    ));
    detail.add_css_class("maintainer_text_muted");
    detail.set_halign(Align::Start);
    detail.set_wrap(true);
    content.append(&detail);

    let on_term = on_terminate.clone();
    dialog.connect_response(move |dlg, response| {
        if response == ResponseType::Accept {
            if let Some(ref cb) = *on_term.borrow() {
                cb(pid, start_time_ticks);
            }
        }
        dlg.close();
    });

    dialog.present();
}

/// Format bytes as a human-readable memory string.
fn format_memory_bytes(bytes: u64) -> String {
    if bytes >= 1_073_741_824 {
        format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
    } else if bytes >= 1_048_576 {
        format!("{:.0} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.0} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
