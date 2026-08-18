//! Application lifecycle management.
//!
//! Sets up the GTK Application, loads CSS design tokens, and
//! wires the shell on activation.

use std::path::PathBuf;

use gtk4::gdk::Display;
use gtk4::prelude::*;
use gtk4::{Application, CssProvider};

use crate::ui::shell::Shell;

/// Application ID following reverse-DNS convention.
pub const APP_ID: &str = "com.maintainer.app";

/// Create and configure the GTK Application.
pub fn build_application() -> Application {
    Application::builder().application_id(APP_ID).build()
}

/// Load CSS design tokens at runtime from the data/styles directory.
///
/// Loads the light theme CSS. During development, runtime loading avoids
/// requiring a GResource build step.
pub fn load_css() {
    let display = Display::default().expect("Could not connect to display");

    let css_path = find_css_path();
    let provider = CssProvider::new();
    provider.load_from_path(&css_path);

    gtk4::style_context_add_provider_for_display(
        &display,
        &provider,
        gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

/// Find the CSS file path, checking development and installed locations.
fn find_css_path() -> PathBuf {
    // Development: relative to the workspace root
    let dev_light = PathBuf::from("data/styles/light.css");
    if dev_light.exists() {
        return dev_light;
    }

    // Try relative to the executable
    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let installed = parent.join("../share/maintainer/styles/light.css");
            if installed.exists() {
                return installed;
            }
        }
    }

    // Fallback: just use the dev path (will log a GTK warning if missing)
    dev_light
}

/// Setup the application: connect activation signal and run.
pub fn run_application(app: &Application) {
    app.connect_activate(|app| {
        load_css();
        let shell = Shell::new(app);
        shell.present();
    });
}
