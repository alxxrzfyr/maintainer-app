//! Reusable progress indicator for async scan jobs.
//!
//! Displays a progress bar with status label and an optional cancel button.
//! Updates only the affected widgets when progress changes.

use gtk4::prelude::*;
use gtk4::{self, Align, Box as GtkBox, Button, Label, Orientation, ProgressBar};

/// A composite widget showing scan progress with cancel control.
pub struct JobProgress {
    container: GtkBox,
    label: Label,
    progress_bar: ProgressBar,
    cancel_button: Button,
}

impl JobProgress {
    /// Create a new progress widget with the given status label.
    pub fn new(status_text: &str) -> Self {
        let container = GtkBox::new(Orientation::Vertical, 8);
        container.add_css_class("job-progress");

        let header_row = GtkBox::new(Orientation::Horizontal, 8);
        header_row.set_hexpand(true);

        let label = Label::new(Some(status_text));
        label.set_halign(Align::Start);
        label.set_hexpand(true);
        label.add_css_class("job-progress-label");

        let cancel_button = Button::with_label("Cancel");
        cancel_button.add_css_class("destructive-action");
        cancel_button.set_halign(Align::End);

        header_row.append(&label);
        header_row.append(&cancel_button);

        let progress_bar = ProgressBar::new();
        progress_bar.set_hexpand(true);
        progress_bar.set_show_text(true);

        container.append(&header_row);
        container.append(&progress_bar);

        Self {
            container,
            label,
            progress_bar,
            cancel_button,
        }
    }

    /// Get the root widget for embedding in a parent container.
    pub fn widget(&self) -> &GtkBox {
        &self.container
    }

    /// Update progress fraction and text.
    pub fn set_progress(&self, completed: u64, total: Option<u64>) {
        match total {
            Some(t) if t > 0 => {
                let fraction = completed as f64 / t as f64;
                self.progress_bar.set_fraction(fraction.clamp(0.0, 1.0));
                self.progress_bar
                    .set_text(Some(&format!("{completed} / {t}")));
            }
            _ => {
                self.progress_bar.pulse();
                self.progress_bar
                    .set_text(Some(&format!("{completed} items processed")));
            }
        }
    }

    /// Update the status label text.
    pub fn set_status(&self, text: &str) {
        self.label.set_text(text);
    }

    /// Show or hide the cancel button.
    pub fn set_cancel_visible(&self, visible: bool) {
        self.cancel_button.set_visible(visible);
    }

    /// Connect a callback to the cancel button click.
    pub fn connect_cancel<F: Fn() + 'static>(&self, callback: F) {
        self.cancel_button.connect_clicked(move |_| callback());
    }

    /// Set whether the entire widget is visible.
    pub fn set_visible(&self, visible: bool) {
        self.container.set_visible(visible);
    }
}
