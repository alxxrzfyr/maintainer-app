//! System Monitor page: detailed live metrics, process table, and termination.
//!
//! Shows per-core CPU, memory breakdown, disk I/O, network rates, temperatures,
//! and a sortable process table. Process termination shows a confirmation dialog
//! before sending SIGTERM.

use std::cell::RefCell;
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, Dialog, DialogFlags, Label, Orientation, ResponseType,
    ScrolledWindow,
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
    temperature_box: GtkBox,
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
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);
        content.add_css_class("maintainer_system_page");

        // Title
        let title = Label::new(Some("System Monitor"));
        title.add_css_class("maintainer_page_title");
        title.set_halign(Align::Start);
        content.append(&title);

        // Top metrics row
        let metrics_row = GtkBox::new(Orientation::Horizontal, 12);
        metrics_row.set_homogeneous(true);

        let cpu_card = MetricCard::new("CPU", true);
        metrics_row.append(cpu_card.widget());

        let memory_card = MetricCard::new("Memory", true);
        metrics_row.append(memory_card.widget());

        content.append(&metrics_row);

        // Temperature section
        let temp_label = Label::new(Some("Temperatures"));
        temp_label.add_css_class("maintainer_section_title");
        temp_label.set_halign(Align::Start);
        temp_label.set_margin_top(12);
        content.append(&temp_label);

        let temperature_box = GtkBox::new(Orientation::Horizontal, 12);
        temperature_box.add_css_class("maintainer_temperature_box");
        content.append(&temperature_box);

        // Process section
        let process_header = GtkBox::new(Orientation::Horizontal, 8);
        process_header.set_margin_top(16);

        let process_label = Label::new(Some("Processes"));
        process_label.add_css_class("maintainer_section_title");
        process_label.set_halign(Align::Start);
        process_label.set_hexpand(true);
        process_header.append(&process_label);

        // Sort buttons
        let on_sort_changed: SortCallback = Rc::new(RefCell::new(None));

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

        process_header.append(&cpu_btn);
        process_header.append(&memory_btn);
        process_header.append(&name_btn);
        process_header.append(&pid_btn);

        content.append(&process_header);

        // Process list
        let process_list_box = GtkBox::new(Orientation::Vertical, 2);
        process_list_box.add_css_class("maintainer_process_list");
        content.append(&process_list_box);

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
            temperature_box,
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

    /// Update the page from the view model.
    pub fn update(&self, vm: &SystemViewModel) {
        // CPU
        if let Some(ref cpu) = vm.cpu {
            self.cpu_card.set_available();
            self.cpu_card
                .set_value(&format!("{:.0}%", cpu.total_percent));
            self.cpu_card.update_sparkline(
                vm.cpu_ring.samples(),
                &format!("CPU: {:.0}%", cpu.total_percent),
            );
        }

        // Memory
        if let Some(ref mem) = vm.memory {
            self.memory_card.set_available();
            let used_gib = mem.used_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let total_gib = mem.total_bytes as f64 / (1024.0 * 1024.0 * 1024.0);
            let text = format!("{used_gib:.1} / {total_gib:.1} GiB");
            self.memory_card.set_value(&text);
            self.memory_card
                .update_sparkline(vm.memory_ring.samples(), &format!("Memory: {text}"));
        }

        // Temperatures
        self.update_temperatures(vm);

        // Process list
        self.update_processes(vm);

        // Sort button active state
        self.update_sort_buttons(vm.process_sort);
    }

    fn update_temperatures(&self, vm: &SystemViewModel) {
        // Clear and rebuild temperature labels
        while let Some(child) = self.temperature_box.first_child() {
            self.temperature_box.remove(&child);
        }

        if vm.temperatures.is_empty() {
            let empty = Label::new(Some("No sensors detected"));
            empty.add_css_class("maintainer_text_muted");
            self.temperature_box.append(&empty);
            return;
        }

        for reading in &vm.temperatures {
            if let maintainer_core::capability::Capability::Available(celsius) = &reading.celsius {
                let name = reading.label.as_deref().unwrap_or(&reading.hwmon_name);
                let label = Label::new(Some(&format!("{name}: {celsius:.0}°C")));
                label.add_css_class("maintainer_temperature_reading");
                self.temperature_box.append(&label);
            }
        }
    }

    fn update_processes(&self, vm: &SystemViewModel) {
        // Clear existing rows
        while let Some(child) = self.process_list_box.first_child() {
            self.process_list_box.remove(&child);
        }

        if !vm.processes_available {
            let label = Label::new(Some("Process information unavailable"));
            label.add_css_class("maintainer_text_muted");
            self.process_list_box.append(&label);
            return;
        }

        if vm.processes.is_empty() {
            let label = Label::new(Some("No processes"));
            label.add_css_class("maintainer_text_muted");
            self.process_list_box.append(&label);
            return;
        }

        // Header row
        let header = GtkBox::new(Orientation::Horizontal, 8);
        header.add_css_class("maintainer_process_header");
        header.set_margin_bottom(4);

        let pid_h = Label::new(Some("PID"));
        pid_h.set_width_request(60);
        pid_h.set_halign(Align::Start);
        header.append(&pid_h);

        let name_h = Label::new(Some("Name"));
        name_h.set_hexpand(true);
        name_h.set_halign(Align::Start);
        header.append(&name_h);

        let mem_h = Label::new(Some("Memory"));
        mem_h.set_width_request(80);
        mem_h.set_halign(Align::End);
        header.append(&mem_h);

        let state_h = Label::new(Some("State"));
        state_h.set_width_request(50);
        state_h.set_halign(Align::Center);
        header.append(&state_h);

        let action_h = Label::new(Some(""));
        action_h.set_width_request(60);
        header.append(&action_h);

        self.process_list_box.append(&header);

        // Show top 50 processes
        let display_count = vm.processes.len().min(50);
        for proc in &vm.processes[..display_count] {
            let row = self.build_process_row(proc);
            self.process_list_box.append(&row);
        }
    }

    fn build_process_row(&self, proc: &maintainer_core::metrics::ProcessSnapshot) -> GtkBox {
        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.add_css_class("maintainer_process_row");
        row.set_margin_top(2);
        row.set_margin_bottom(2);

        let pid_label = Label::new(Some(&proc.identity.pid.to_string()));
        pid_label.set_width_request(60);
        pid_label.set_halign(Align::Start);
        pid_label.add_css_class("dim-label");
        row.append(&pid_label);

        let name_label = Label::new(Some(&proc.name));
        name_label.set_hexpand(true);
        name_label.set_halign(Align::Start);
        name_label.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        row.append(&name_label);

        let mem_text = format_memory_bytes(proc.rss_bytes);
        let mem_label = Label::new(Some(&mem_text));
        mem_label.set_width_request(80);
        mem_label.set_halign(Align::End);
        row.append(&mem_label);

        let state_label = Label::new(Some(&proc.state.to_string()));
        state_label.set_width_request(50);
        state_label.set_halign(Align::Center);
        row.append(&state_label);

        // Terminate button
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

        row.append(&terminate_btn);
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
        format!("{:.1} MiB", bytes as f64 / 1_048_576.0)
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}
