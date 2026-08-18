//! Application lifecycle management.

use std::path::PathBuf;

use gtk4::gdk::Display;
use gtk4::prelude::*;
use gtk4::{Application, CssProvider};

use crate::runtime::Runtime;
use crate::ui::shell::Shell;

/// Application ID following reverse-DNS convention matching packaging metadata.
pub const APP_ID: &str = "io.github.maintainerapp.Maintainer";

/// Create and configure the GTK Application.
pub fn build_application() -> Application {
    Application::builder().application_id(APP_ID).build()
}

/// Load CSS design tokens at runtime from the data/styles directory.
pub fn load_css() {
    let display = match Display::default() {
        Some(d) => d,
        None => {
            eprintln!("warning: no display connection, skipping CSS load");
            return;
        }
    };

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
    let dev_light = PathBuf::from("data/styles/light.css");
    if dev_light.exists() {
        return dev_light;
    }

    if let Ok(exe_path) = std::env::current_exe() {
        if let Some(parent) = exe_path.parent() {
            let installed = parent.join("../share/maintainer/styles/light.css");
            if installed.exists() {
                return installed;
            }
        }
    }

    dev_light
}

/// Setup the application: connect activation signal and run.
pub fn run_application(app: &Application) {
    app.connect_activate(|app| {
        load_css();
        let runtime = Runtime::init();
        let shell = Shell::new(app, runtime);
        shell.present();
    });
}
