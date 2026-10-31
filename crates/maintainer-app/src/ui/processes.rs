//! Processes detail page: sortable process table with terminate action.
//!
//! Shows a scrollable, sortable process list with PID, name, CPU ticks,
//! memory, and state. Process termination triggers Polkit authorization.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow};

use crate::monitoring::{ProcessSort, ProcessesViewModel};
use maintainer_core::metrics::ProcessIdentity;
use maintainer_core::metrics::ProcessSnapshot;

/// Callback type for sort changes.
type SortCallback = Rc<RefCell<Option<Box<dyn Fn(ProcessSort)>>>>;

/// Callback type for terminate requests (pid, start_time_ticks).
type TerminateCallback = Rc<RefCell<Option<Box<dyn Fn(u32, u64)>>>>;

/// The processes detail page.
pub struct ProcessesPage {
    root: ScrolledWindow,
    process_list_box: GtkBox,
    process_count_label: Label,
    overflow_label: Label,
    rows: RefCell<HashMap<ProcessIdentity, ProcessRow>>,
    operation_status: Label,
    unavailable_box: GtkBox,
    unavailable_label: Label,
    on_sort_changed: SortCallback,
    on_terminate: TerminateCallback,
}

struct ProcessRow {
    root: GtkBox,
    name: Label,
    state: Label,
    memory: Label,
}

impl ProcessRow {
    fn new(
        process: &ProcessSnapshot,
        on_terminate: &TerminateCallback,
        operation_status: &Label,
    ) -> Self {
        let row = GtkBox::new(Orientation::Horizontal, 8);
        row.set_margin_top(4);
        row.set_margin_bottom(4);
        row.set_margin_start(12);
        row.set_margin_end(12);
        row.add_css_class("maintainer_process_row");

        let pid_label = Label::new(Some(&process.identity.pid.to_string()));
        pid_label.set_width_chars(7);
        pid_label.set_halign(Align::Start);
        pid_label.add_css_class("maintainer_text_muted");
        row.append(&pid_label);

        let name = Label::new(None);
        name.set_halign(Align::Start);
        name.set_hexpand(true);
        name.set_ellipsize(gtk4::pango::EllipsizeMode::End);
        name.set_max_width_chars(30);
        row.append(&name);

        let state = Label::new(None);
        state.set_width_chars(3);
        row.append(&state);

        let memory = Label::new(None);
        memory.set_width_chars(10);
        memory.set_halign(Align::End);
        row.append(&memory);

        let terminate_btn = Button::with_label("End");
        terminate_btn.add_css_class("maintainer_terminate_button");
        terminate_btn.set_tooltip_text(Some("Send SIGTERM to this process"));
        let pid = process.identity.pid;
        let start_time = process.identity.start_time_ticks;
        let terminate_cb = on_terminate.clone();
        let status_label = operation_status.clone();
        terminate_btn.connect_clicked(move |_| {
            status_label.remove_css_class("error");
            status_label.set_text("Preparing process review…");
            status_label.set_visible(true);
            if let Some(cb) = terminate_cb.borrow().as_ref() {
                cb(pid, start_time);
            }
        });
        row.append(&terminate_btn);

        let result = Self {
            root: row,
            name,
            state,
            memory,
        };
        result.update(process);
        result
    }

    fn update(&self, process: &ProcessSnapshot) {
        self.name.set_text(&process.name);
        self.memory.set_text(&format_memory(process.rss_bytes));
        self.state.set_text(&String::from(process.state));
        self.state
            .remove_css_class("maintainer_process_state_running");
        self.state
            .remove_css_class("maintainer_process_state_zombie");
        self.state.remove_css_class("maintainer_text_muted");
        match process.state {
            'R' => self.state.add_css_class("maintainer_process_state_running"),
            'Z' => self.state.add_css_class("maintainer_process_state_zombie"),
            _ => self.state.add_css_class("maintainer_text_muted"),
        }
    }
}

impl Default for ProcessesPage {
    fn default() -> Self {
        Self::new()
    }
}

impl ProcessesPage {
    /// Create a new processes page.
    pub fn new() -> Self {
        let content = GtkBox::new(Orientation::Vertical, 12);
        content.add_css_class("maintainer_page_content");
        content.set_margin_top(24);
        content.set_margin_bottom(24);
        content.set_margin_start(24);
        content.set_margin_end(24);

        let title = Label::new(Some("Processes"));
        title.add_css_class("maintainer_page_title");
        title.set_halign(Align::Start);
        content.append(&title);

        let subtitle = Label::new(Some(
            "Running processes with CPU, memory, and state. Terminate sends SIGTERM.",
        ));
        subtitle.add_css_class("dim-label");
        subtitle.set_halign(Align::Start);
        subtitle.set_wrap(true);
        content.append(&subtitle);

        let process_count_label = Label::new(Some("0 processes"));
        process_count_label.add_css_class("maintainer_text_muted");
        process_count_label.set_halign(Align::Start);
        content.append(&process_count_label);

        // Sort buttons
        let sort_bar = GtkBox::new(Orientation::Horizontal, 4);
        sort_bar.set_margin_top(4);

        let sort_label = Label::new(Some("Sort by:"));
        sort_label.add_css_class("maintainer_text_muted");
        sort_bar.append(&sort_label);

        let cpu_btn = Button::with_label("CPU");
        cpu_btn.add_css_class("maintainer_sort_button");
        cpu_btn.add_css_class("maintainer_sort_active");
        sort_bar.append(&cpu_btn);

        let memory_btn = Button::with_label("Memory");
        memory_btn.add_css_class("maintainer_sort_button");
        sort_bar.append(&memory_btn);

        let name_btn = Button::with_label("Name");
        name_btn.add_css_class("maintainer_sort_button");
        sort_bar.append(&name_btn);

        let pid_btn = Button::with_label("PID");
        pid_btn.add_css_class("maintainer_sort_button");
        sort_bar.append(&pid_btn);

        content.append(&sort_bar);

        let operation_status = Label::new(None);
        operation_status.set_halign(Align::Start);
        operation_status.set_visible(false);
        content.append(&operation_status);

        let process_list_box = GtkBox::new(Orientation::Vertical, 0);
        process_list_box.add_css_class("maintainer_surface");
        let overflow_label = Label::new(None);
        overflow_label.add_css_class("maintainer_text_muted");
        overflow_label.set_margin_top(8);
        overflow_label.set_margin_bottom(8);
        overflow_label.set_visible(false);
        process_list_box.append(&overflow_label);

        let process_scroll = ScrolledWindow::new();
        process_scroll.set_child(Some(&process_list_box));
        process_scroll.set_vexpand(true);
        process_scroll.set_min_content_height(300);
        content.append(&process_scroll);

        let unavailable_box = GtkBox::new(Orientation::Vertical, 8);
        unavailable_box.set_halign(Align::Center);
        unavailable_box.set_valign(Align::Center);
        unavailable_box.set_margin_top(32);
        unavailable_box.set_visible(false);

        let unavailable_label = Label::new(Some("Process data unavailable"));
        unavailable_label.add_css_class("maintainer_text_muted");
        unavailable_box.append(&unavailable_label);
        content.append(&unavailable_box);

        let on_sort_changed: SortCallback = Rc::new(RefCell::new(None));
        let on_terminate: TerminateCallback = Rc::new(RefCell::new(None));

        // Wire sort buttons
        let all_buttons = [&cpu_btn, &memory_btn, &name_btn, &pid_btn];
        let sorts = [
            ProcessSort::Cpu,
            ProcessSort::Memory,
            ProcessSort::Name,
            ProcessSort::Pid,
        ];

        for (i, btn) in all_buttons.iter().enumerate() {
            let sort = sorts[i];
            let callback = on_sort_changed.clone();
            let cpu_ref = cpu_btn.downgrade();
            let mem_ref = memory_btn.downgrade();
            let name_ref = name_btn.downgrade();
            let pid_ref = pid_btn.downgrade();
            let btn_clone = (*btn).clone();
            btn_clone.connect_clicked(move |_| {
                let (Some(cpu_ref), Some(mem_ref), Some(name_ref), Some(pid_ref)) = (
                    cpu_ref.upgrade(),
                    mem_ref.upgrade(),
                    name_ref.upgrade(),
                    pid_ref.upgrade(),
                ) else {
                    return;
                };
                for b in [&cpu_ref, &mem_ref, &name_ref, &pid_ref] {
                    b.remove_css_class("maintainer_sort_active");
                }
                match sort {
                    ProcessSort::Cpu => cpu_ref.add_css_class("maintainer_sort_active"),
                    ProcessSort::Memory => mem_ref.add_css_class("maintainer_sort_active"),
                    ProcessSort::Name => name_ref.add_css_class("maintainer_sort_active"),
                    ProcessSort::Pid => pid_ref.add_css_class("maintainer_sort_active"),
                }
                if let Some(cb) = callback.borrow().as_ref() {
                    cb(sort);
                }
            });
        }

        let root = ScrolledWindow::new();
        root.set_child(Some(&content));
        root.set_hexpand(true);
        root.set_vexpand(true);

        Self {
            root,
            process_list_box,
            process_count_label,
            overflow_label,
            rows: RefCell::new(HashMap::new()),
            operation_status,
            unavailable_box,
            unavailable_label,
            on_sort_changed,
            on_terminate,
        }
    }

    /// Get the root widget for stack insertion.
    pub fn widget(&self) -> &ScrolledWindow {
        &self.root
    }

    /// Register a callback for sort order changes.
    pub fn connect_sort_changed<F: Fn(ProcessSort) + 'static>(&self, callback: F) {
        *self.on_sort_changed.borrow_mut() = Some(Box::new(callback));
    }

    /// Register a callback for process termination requests.
    pub fn connect_terminate<F: Fn(u32, u64) + 'static>(&self, callback: F) {
        *self.on_terminate.borrow_mut() = Some(Box::new(callback));
    }

    /// Get the operation status label for helper result display.
    pub fn operation_status_handle(&self) -> &Label {
        &self.operation_status
    }

    /// Update the page with new processes view model data.
    pub fn update(&self, vm: &ProcessesViewModel) {
        if !vm.available {
            self.process_list_box.set_visible(false);
            self.unavailable_box.set_visible(true);
            if let Some(reason) = &vm.unavailable_reason {
                self.unavailable_label.set_text(reason);
            }
            self.process_count_label.set_text("—");
            return;
        }

        self.unavailable_box.set_visible(false);
        self.process_list_box.set_visible(true);
        self.process_count_label
            .set_text(&format!("{} processes", vm.total_count));

        // Show at most 100 processes to keep GTK responsive.
        let display_limit = vm.processes.len().min(100);
        let mut wanted = HashSet::with_capacity(display_limit);
        let mut rows = self.rows.borrow_mut();
        for proc in &vm.processes[..display_limit] {
            wanted.insert(proc.identity.clone());
            let row = rows.entry(proc.identity.clone()).or_insert_with(|| {
                ProcessRow::new(proc, &self.on_terminate, &self.operation_status)
            });
            row.update(proc);
            if row.root.parent().is_none() {
                self.process_list_box.append(&row.root);
            }
        }

        let stale: Vec<_> = rows
            .keys()
            .filter(|identity| !wanted.contains(*identity))
            .cloned()
            .collect();
        for identity in stale {
            if let Some(row) = rows.remove(&identity) {
                self.process_list_box.remove(&row.root);
            }
        }

        let mut previous_row: Option<GtkBox> = None;
        for process in &vm.processes[..display_limit] {
            if let Some(row) = rows.get(&process.identity) {
                self.process_list_box
                    .reorder_child_after(&row.root, previous_row.as_ref());
                previous_row = Some(row.root.clone());
            }
        }

        if display_limit < vm.processes.len() {
            self.overflow_label.set_text(&format!(
                "… and {} more processes",
                vm.processes.len() - display_limit
            ));
            self.overflow_label.set_visible(true);
        } else {
            self.overflow_label.set_visible(false);
        }
    }
}

/// Format bytes as a human-readable memory string.
fn format_memory(bytes: u64) -> String {
    const KIB: f64 = 1024.0;
    const MIB: f64 = KIB * 1024.0;
    const GIB: f64 = MIB * 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.1} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{} B", bytes)
    }
}
