//! Settings page UI with grouped controls.
//!
//! Groups: General, Monitoring, Storage, Maintenance, Privacy.
//! Activity tracking is intentionally absent from v1.
//! Each setting has an accessible label and concise impact text.
//! Invalid input cannot be saved.

use gtk4::prelude::*;
use gtk4::{
    Align, Box as GtkBox, Button, ComboBoxText, Label, Orientation, ScrolledWindow, Separator,
    SpinButton,
};

use crate::state::{SettingsIntent, SettingsSnapshot, Theme};

/// Build the settings page widget.
///
/// The `on_intent` callback is invoked when the user modifies a setting.
/// The snapshot provides initial values for all controls.
pub fn build_settings_page(
    snapshot: &SettingsSnapshot,
    on_intent: impl Fn(SettingsIntent) + 'static,
) -> ScrolledWindow {
    let scrolled = ScrolledWindow::new();
    scrolled.set_hexpand(true);
    scrolled.set_vexpand(true);
    scrolled.add_css_class("maintainer_settings_page");

    let container = GtkBox::new(Orientation::Vertical, 24);
    container.set_margin_top(24);
    container.set_margin_bottom(24);
    container.set_margin_start(24);
    container.set_margin_end(24);
    container.set_halign(Align::Fill);
    container.set_hexpand(true);

    let on_intent = std::rc::Rc::new(on_intent);

    let title = Label::new(Some("Settings"));
    title.add_css_class("maintainer_page_title");
    title.set_halign(Align::Start);
    container.append(&title);

    let subtitle = Label::new(Some(
        "Manage application appearance, monitoring frequencies, storage thresholds, and local privacy.",
    ));
    subtitle.add_css_class("dim-label");
    subtitle.set_halign(Align::Start);
    subtitle.set_wrap(true);
    container.append(&subtitle);

    container.append(&build_group_header("General"));
    container.append(&build_theme_row(snapshot, on_intent.clone()));
    container.append(&Separator::new(Orientation::Horizontal));

    container.append(&build_group_header("Monitoring"));
    container.append(&build_refresh_interval_row(snapshot, on_intent.clone()));
    container.append(&build_hidden_interval_row(snapshot, on_intent.clone()));
    container.append(&Separator::new(Orientation::Horizontal));

    container.append(&build_group_header("Storage"));
    container.append(&build_large_file_threshold_row(snapshot, on_intent.clone()));
    container.append(&Separator::new(Orientation::Horizontal));

    container.append(&build_group_header("Maintenance"));
    container.append(&build_maintenance_info_row());
    container.append(&Separator::new(Orientation::Horizontal));

    container.append(&build_group_header("Privacy"));
    container.append(&build_privacy_rows(on_intent.clone()));

    scrolled.set_child(Some(&container));
    scrolled
}

fn build_group_header(title: &str) -> Label {
    let label = Label::new(Some(title));
    label.set_halign(Align::Start);
    label.add_css_class("maintainer_settings_group_header");
    label
}

fn build_setting_row(label_text: &str, description: &str) -> (GtkBox, GtkBox) {
    let row = GtkBox::new(Orientation::Horizontal, 16);
    row.set_halign(Align::Fill);
    row.set_hexpand(true);
    row.set_margin_bottom(8);
    row.add_css_class("maintainer_settings_row");

    let text_box = GtkBox::new(Orientation::Vertical, 4);
    text_box.set_hexpand(true);
    text_box.set_halign(Align::Start);
    text_box.set_valign(Align::Center);

    let label = Label::new(Some(label_text));
    label.set_halign(Align::Start);
    label.add_css_class("maintainer_settings_label");
    text_box.append(&label);

    let desc = Label::new(Some(description));
    desc.set_halign(Align::Start);
    desc.add_css_class("maintainer_text_muted");
    desc.set_wrap(true);
    text_box.append(&desc);

    let control_box = GtkBox::new(Orientation::Horizontal, 8);
    control_box.set_halign(Align::End);
    control_box.set_valign(Align::Center);

    row.append(&text_box);
    row.append(&control_box);

    (row, control_box)
}

fn build_theme_row(
    snapshot: &SettingsSnapshot,
    on_intent: std::rc::Rc<dyn Fn(SettingsIntent)>,
) -> GtkBox {
    let (row, control_box) =
        build_setting_row("Theme", "Choose light, dark, or follow system preference.");

    let combo = ComboBoxText::new();
    combo.append(Some("system"), "System");
    combo.append(Some("light"), "Light");
    combo.append(Some("dark"), "Dark");
    combo.set_active_id(Some(snapshot.theme.as_str()));
    combo.update_property(&[gtk4::accessible::Property::Label("Theme selection")]);

    combo.connect_changed(move |c| {
        if let Some(id) = c.active_id() {
            if let Some(theme) = Theme::parse(id.as_str()) {
                on_intent(SettingsIntent::SetTheme(theme));
            }
        }
    });

    control_box.append(&combo);
    row
}

fn build_refresh_interval_row(
    snapshot: &SettingsSnapshot,
    on_intent: std::rc::Rc<dyn Fn(SettingsIntent)>,
) -> GtkBox {
    let (row, control_box) = build_setting_row(
        "Refresh interval",
        "How often live metrics update when the window is visible (1–60 seconds).",
    );

    let adjustment = gtk4::Adjustment::new(
        f64::from(snapshot.refresh_interval_secs),
        1.0,
        60.0,
        1.0,
        5.0,
        0.0,
    );
    let spin = SpinButton::new(Some(&adjustment), 1.0, 0);
    spin.set_width_chars(4);
    spin.update_property(&[gtk4::accessible::Property::Label(
        "Refresh interval seconds",
    )]);

    spin.connect_value_changed(move |s| {
        let val = s.value_as_int();
        if (1..=60).contains(&val) {
            on_intent(SettingsIntent::SetRefreshInterval(val as u32));
        }
    });

    let unit_label = Label::new(Some("seconds"));
    control_box.append(&spin);
    control_box.append(&unit_label);
    row
}

fn build_hidden_interval_row(
    snapshot: &SettingsSnapshot,
    on_intent: std::rc::Rc<dyn Fn(SettingsIntent)>,
) -> GtkBox {
    let (row, control_box) = build_setting_row(
        "Hidden window interval",
        "Minimum refresh interval when the window is hidden (10–60 seconds).",
    );

    let adjustment = gtk4::Adjustment::new(
        f64::from(snapshot.hidden_interval_secs),
        10.0,
        60.0,
        1.0,
        5.0,
        0.0,
    );
    let spin = SpinButton::new(Some(&adjustment), 1.0, 0);
    spin.set_width_chars(4);
    spin.update_property(&[gtk4::accessible::Property::Label("Hidden interval seconds")]);

    spin.connect_value_changed(move |s| {
        let val = s.value_as_int();
        if (10..=60).contains(&val) {
            on_intent(SettingsIntent::SetHiddenInterval(val as u32));
        }
    });

    let unit_label = Label::new(Some("seconds"));
    control_box.append(&spin);
    control_box.append(&unit_label);
    row
}

fn build_large_file_threshold_row(
    snapshot: &SettingsSnapshot,
    on_intent: std::rc::Rc<dyn Fn(SettingsIntent)>,
) -> GtkBox {
    let (row, control_box) = build_setting_row(
        "Large file threshold",
        "Files above this size appear in storage analysis results (1–10000 MiB).",
    );

    let mib_value = snapshot.large_file_threshold_bytes / (1024 * 1024);
    let adjustment = gtk4::Adjustment::new(mib_value as f64, 1.0, 10000.0, 1.0, 10.0, 0.0);
    let spin = SpinButton::new(Some(&adjustment), 1.0, 0);
    spin.set_width_chars(6);
    spin.update_property(&[gtk4::accessible::Property::Label(
        "Large file threshold in MiB",
    )]);

    spin.connect_value_changed(move |s| {
        let mib = s.value_as_int();
        if (1..=10000).contains(&mib) {
            let bytes = (mib as u64) * 1024 * 1024;
            on_intent(SettingsIntent::SetLargeFileThreshold(bytes));
        }
    });

    let unit_label = Label::new(Some("MiB"));
    control_box.append(&spin);
    control_box.append(&unit_label);
    row
}

fn build_maintenance_info_row() -> GtkBox {
    let row = GtkBox::new(Orientation::Vertical, 4);
    row.set_halign(Align::Fill);

    let info = Label::new(Some(
        "Cleanup operations always require review and confirmation before execution.",
    ));
    info.set_halign(Align::Start);
    info.add_css_class("maintainer_text_muted");
    info.set_wrap(true);
    row.append(&info);
    row
}

fn build_privacy_rows(on_intent: std::rc::Rc<dyn Fn(SettingsIntent)>) -> GtkBox {
    let container = GtkBox::new(Orientation::Vertical, 12);

    let info = Label::new(Some(
        "Maintainer stores settings and operation history locally. No data is uploaded.",
    ));
    info.set_halign(Align::Start);
    info.add_css_class("maintainer_text_muted");
    info.set_wrap(true);
    container.append(&info);

    let on_intent_delete_history = on_intent.clone();
    let delete_history_btn = Button::with_label("Delete all history");
    delete_history_btn.add_css_class("destructive-action");
    delete_history_btn.set_halign(Align::Start);
    delete_history_btn.set_tooltip_text(Some("Remove all operation history records"));
    delete_history_btn.update_property(&[gtk4::accessible::Property::Label("Delete all history")]);
    delete_history_btn.connect_clicked(move |_| {
        on_intent_delete_history(SettingsIntent::RequestDeleteAllHistory);
    });
    container.append(&delete_history_btn);

    let delete_all_btn = Button::with_label("Delete all Maintainer data");
    delete_all_btn.add_css_class("destructive-action");
    delete_all_btn.set_halign(Align::Start);
    delete_all_btn.set_tooltip_text(Some(
        "Remove local settings and maintenance history, then recreate defaults",
    ));
    delete_all_btn.update_property(&[gtk4::accessible::Property::Label(
        "Delete all Maintainer data",
    )]);
    delete_all_btn.connect_clicked(move |_| {
        on_intent(SettingsIntent::RequestDeleteAllData);
    });
    container.append(&delete_all_btn);

    container
}
