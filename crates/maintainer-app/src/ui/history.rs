//! History view page.
//!
//! Displays operation history: operation type, timestamp, result status,
//! recovered bytes, and bounded error summary. Provides delete-one and
//! delete-all actions requiring confirmation.

use gtk4::prelude::*;
use gtk4::{Align, Box as GtkBox, Button, Label, Orientation, ScrolledWindow, Separator};

use crate::state::HistoryIntent;

/// Presentation model for a single history entry in the UI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryRow {
    /// Unique ID for deletion targeting.
    pub id: String,
    /// Human-readable operation type (e.g. "Cleanup", "Scan", "Update").
    pub operation: String,
    /// Formatted timestamp string.
    pub time: String,
    /// Human-readable result (e.g. "Success", "Partial failure").
    pub result: String,
    /// Formatted recovered bytes (e.g. "1.2 MiB" or "—").
    pub recovered_bytes: String,
    /// Bounded error summary, or None if no errors.
    pub error_summary: Option<String>,
}

/// Build the history page widget.
///
/// `entries` are pre-formatted presentation rows from core history data.
/// `on_intent` is invoked on user actions (delete one, delete all).
pub fn build_history_page(
    entries: &[HistoryRow],
    on_intent: impl Fn(HistoryIntent) + 'static,
) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(true);
    scrolled.add_css_class("maintainer_history_page");

    let container = GtkBox::new(Orientation::Vertical, 12);
    container.set_margin_top(24);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);

    let on_intent = std::rc::Rc::new(on_intent);
    let selected_ids = std::rc::Rc::new(std::cell::RefCell::new(
        std::collections::HashSet::<String>::new(),
    ));
    let check_widgets = std::rc::Rc::new(std::cell::RefCell::new(Vec::<gtk4::CheckButton>::new()));

    let header = GtkBox::new(Orientation::Horizontal, 12);
    header.set_halign(Align::Fill);
    header.set_hexpand(true);

    let title = Label::new(Some("Operation History"));
    title.set_halign(Align::Start);
    title.set_hexpand(true);
    title.add_css_class("title-1");
    header.append(&title);

    let delete_selected_btn = Button::with_label("Delete selected");
    delete_selected_btn.add_css_class("destructive-action");
    delete_selected_btn.set_sensitive(false);

    if !entries.is_empty() {
        let select_all_btn = Button::with_label("Select All");
        select_all_btn.add_css_class("flat");

        let deselect_all_btn = Button::with_label("Deselect All");
        deselect_all_btn.add_css_class("flat");

        let checks_for_all = check_widgets.clone();
        select_all_btn.connect_clicked(move |_| {
            for check in checks_for_all.borrow().iter() {
                check.set_active(true);
            }
        });

        let checks_for_none = check_widgets.clone();
        deselect_all_btn.connect_clicked(move |_| {
            for check in checks_for_none.borrow().iter() {
                check.set_active(false);
            }
        });

        let on_intent_sel = on_intent.clone();
        let sel_ids_for_delete = selected_ids.clone();
        let del_btn_clone = delete_selected_btn.clone();
        delete_selected_btn.connect_clicked(move |_| {
            let ids: Vec<String> = sel_ids_for_delete.borrow().iter().cloned().collect();
            if !ids.is_empty() {
                del_btn_clone.set_sensitive(false);
                del_btn_clone.set_label("Deleting…");
                on_intent_sel(HistoryIntent::RequestDeleteMany(ids));
            }
        });

        let on_intent_all = on_intent.clone();
        let delete_all_btn = Button::with_label("Delete all");
        delete_all_btn.add_css_class("destructive-action");
        delete_all_btn.set_tooltip_text(Some("Delete all history entries"));
        delete_all_btn.update_property(&[gtk4::accessible::Property::Label("Delete all history")]);
        delete_all_btn.connect_clicked(move |btn| {
            btn.set_sensitive(false);
            btn.set_label("Deleting all…");
            on_intent_all(HistoryIntent::RequestDeleteAll);
        });

        header.append(&select_all_btn);
        header.append(&deselect_all_btn);
        header.append(&delete_selected_btn);
        header.append(&delete_all_btn);
    }

    container.append(&header);
    container.append(&Separator::new(Orientation::Horizontal));

    if entries.is_empty() {
        let empty = Label::new(Some("No operations recorded yet."));
        empty.add_css_class("maintainer_text_muted");
        empty.set_halign(Align::Start);
        container.append(&empty);
    } else {
        for entry in entries {
            let row = build_entry_row(
                entry,
                on_intent.clone(),
                selected_ids.clone(),
                delete_selected_btn.clone(),
                check_widgets.clone(),
            );
            container.append(&row);
        }
    }

    scrolled.set_child(Some(&container));
    scrolled
}

fn build_entry_row(
    entry: &HistoryRow,
    on_intent: std::rc::Rc<dyn Fn(HistoryIntent)>,
    selected_ids: std::rc::Rc<std::cell::RefCell<std::collections::HashSet<String>>>,
    delete_selected_btn: Button,
    check_widgets: std::rc::Rc<std::cell::RefCell<Vec<gtk4::CheckButton>>>,
) -> GtkBox {
    let row = GtkBox::new(Orientation::Horizontal, 12);
    row.set_halign(Align::Fill);
    row.set_hexpand(true);
    row.add_css_class("maintainer_history_row");
    row.set_margin_top(4);
    row.set_margin_bottom(4);

    let check = gtk4::CheckButton::new();
    check_widgets.borrow_mut().push(check.clone());

    let entry_id = entry.id.clone();
    let sel_ids = selected_ids;
    let del_btn = delete_selected_btn;
    check.connect_toggled(move |c| {
        let mut ids = sel_ids.borrow_mut();
        if c.is_active() {
            ids.insert(entry_id.clone());
        } else {
            ids.remove(&entry_id);
        }
        let count = ids.len();
        del_btn.set_sensitive(count > 0);
        if count > 0 {
            del_btn.set_label(&format!("Delete selected ({count})"));
        } else {
            del_btn.set_label("Delete selected");
        }
    });

    row.append(&check);

    let info_box = GtkBox::new(Orientation::Vertical, 2);
    info_box.set_hexpand(true);

    let primary = GtkBox::new(Orientation::Horizontal, 8);
    let op_label = Label::new(Some(&entry.operation));
    op_label.set_halign(Align::Start);
    op_label.add_css_class("maintainer_history_operation");
    primary.append(&op_label);

    let result_label = Label::new(Some(&entry.result));
    result_label.set_halign(Align::Start);
    result_label.add_css_class("maintainer_history_result");
    if entry.result.to_lowercase().contains("success") {
        result_label.add_css_class("success");
    } else if entry.result.to_lowercase().contains("fail")
        || entry.result.to_lowercase().contains("error")
    {
        result_label.add_css_class("error");
    }
    primary.append(&result_label);

    if entry.recovered_bytes != "—" {
        let bytes_label = Label::new(Some(&format!("({})", entry.recovered_bytes)));
        bytes_label.set_halign(Align::Start);
        bytes_label.add_css_class("maintainer_text_muted");
        primary.append(&bytes_label);
    }

    info_box.append(&primary);

    let time_label = Label::new(Some(&entry.time));
    time_label.set_halign(Align::Start);
    time_label.add_css_class("maintainer_text_muted");
    info_box.append(&time_label);

    if let Some(ref errors) = entry.error_summary {
        let err_label = Label::new(Some(errors));
        err_label.set_halign(Align::Start);
        err_label.add_css_class("maintainer_history_error");
        err_label.set_wrap(true);
        err_label.set_max_width_chars(80);
        info_box.append(&err_label);
    }

    row.append(&info_box);

    let id = entry.id.clone();
    let delete_btn = Button::with_label("Delete");
    delete_btn.set_valign(Align::Center);
    delete_btn.set_tooltip_text(Some("Delete this history entry"));
    delete_btn.update_property(&[gtk4::accessible::Property::Label(&format!(
        "Delete {} entry",
        entry.operation
    ))]);
    delete_btn.connect_clicked(move |btn| {
        btn.set_sensitive(false);
        btn.set_label("Deleting…");
        on_intent(HistoryIntent::RequestDeleteOne(id.clone()));
    });
    row.append(&delete_btn);

    row
}

/// Format bytes into a human-readable string.
pub fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_string();
    }
    let units = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;
    while size >= 1024.0 && unit_idx < units.len() - 1 {
        size /= 1024.0;
        unit_idx += 1;
    }
    if unit_idx == 0 {
        format!("{size:.0} {}", units[unit_idx])
    } else {
        format!("{size:.1} {}", units[unit_idx])
    }
}

/// Format a UTC Unix timestamp into a local display string.
pub fn format_timestamp(unix_secs: i64) -> String {
    let secs_per_min: i64 = 60;
    let secs_per_hour: i64 = 3600;
    let secs_per_day: i64 = 86400;

    let days = unix_secs / secs_per_day;
    let remaining = unix_secs % secs_per_day;
    let hours = remaining / secs_per_hour;
    let minutes = (remaining % secs_per_hour) / secs_per_min;

    let (year, month, day) = days_to_date(days);
    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02} UTC")
}

/// Convert days since Unix epoch to (year, month, day).
fn days_to_date(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = (z - era * 146097) as u32;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = (yoe as i64) + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    (year, m, d)
}

/// Truncate error JSON array to a bounded summary string.
pub fn summarize_errors(json_errors: Option<&str>, max_chars: usize) -> Option<String> {
    let raw = json_errors?;
    if raw.is_empty() {
        return None;
    }
    let trimmed = raw.trim_start_matches('[').trim_end_matches(']');
    let parts: Vec<&str> = trimmed.split("\",\"").collect();
    let cleaned: Vec<String> = parts
        .iter()
        .map(|p| p.trim_matches('"').trim().to_string())
        .filter(|p| !p.is_empty())
        .collect();

    if cleaned.is_empty() {
        return None;
    }

    let joined = cleaned.join("; ");
    if joined.len() <= max_chars {
        Some(joined)
    } else {
        let truncated: String = joined.chars().take(max_chars - 1).collect();
        Some(format!("{truncated}…"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_bytes_zero_shows_dash() {
        assert_eq!(format_bytes(0), "—");
    }

    #[test]
    fn format_bytes_small() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn format_bytes_mib() {
        let result = format_bytes(100 * 1024 * 1024);
        assert!(result.contains("MiB"));
    }

    #[test]
    fn format_timestamp_reasonable() {
        let ts = format_timestamp(1_700_000_000);
        assert!(ts.contains("2023"));
        assert!(ts.contains("UTC"));
    }

    #[test]
    fn summarize_errors_none_input() {
        assert_eq!(summarize_errors(None, 100), None);
    }

    #[test]
    fn summarize_errors_empty_string() {
        assert_eq!(summarize_errors(Some(""), 100), None);
    }

    #[test]
    fn summarize_errors_single_error() {
        let input = r#"["permission denied"]"#;
        let result = summarize_errors(Some(input), 100);
        assert_eq!(result, Some("permission denied".to_string()));
    }

    #[test]
    fn summarize_errors_multiple() {
        let input = r#"["error one","error two"]"#;
        let result = summarize_errors(Some(input), 100);
        assert_eq!(result, Some("error one; error two".to_string()));
    }

    #[test]
    fn summarize_errors_truncated() {
        let input = r#"["a very long error message that goes on and on"]"#;
        let result = summarize_errors(Some(input), 20);
        assert!(result.unwrap().ends_with('…'));
    }
}
