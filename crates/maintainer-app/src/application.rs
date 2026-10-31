//! Application lifecycle management.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;

use gtk4::gdk::Display;
use gtk4::prelude::*;
use gtk4::{Application, CssProvider};

use crate::runtime::Runtime;
use crate::state::Theme;
use crate::ui::shell::Shell;

/// Application ID following reverse-DNS convention matching packaging metadata.
pub const APP_ID: &str = "io.github.maintainerapp.Maintainer";

/// Create and configure the GTK Application.
pub fn build_application() -> Application {
    Application::builder().application_id(APP_ID).build()
}

thread_local! {
    static CSS_PROVIDER: RefCell<Option<(CssProvider, bool)>> = const { RefCell::new(None) };
    static SYSTEM_DARK_PREFERENCE: Cell<Option<bool>> = const { Cell::new(None) };
}

/// Apply the selected appearance immediately using the shared CSS provider.
pub fn apply_theme(theme: Theme) {
    let Some(display) = Display::default() else {
        eprintln!("warning: no display connection, skipping CSS load");
        return;
    };
    let gtk_settings = gtk4::Settings::default();
    let system_dark = SYSTEM_DARK_PREFERENCE.with(|preference| {
        preference.get().unwrap_or_else(|| {
            let detected = gtk_settings
                .as_ref()
                .map(|settings| settings.is_gtk_application_prefer_dark_theme())
                .unwrap_or(false);
            preference.set(Some(detected));
            detected
        })
    });
    let dark = match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => system_dark,
    };
    if let Some(settings) = gtk_settings {
        settings.set_gtk_application_prefer_dark_theme(dark);
    }
    CSS_PROVIDER.with(|slot| {
        let mut slot = slot.borrow_mut();
        // Database initialization reapplies the saved theme after the initial
        // paint. Avoid invalidating every widget when it resolves to the same CSS.
        if slot
            .as_ref()
            .is_some_and(|(_, applied_dark)| *applied_dark == dark)
        {
            return;
        }
        let (provider, applied_dark) = slot.get_or_insert_with(|| {
            let provider = CssProvider::new();
            gtk4::style_context_add_provider_for_display(
                &display,
                &provider,
                gtk4::STYLE_PROVIDER_PRIORITY_APPLICATION,
            );
            (provider, dark)
        });
        provider.load_from_path(find_css_path(dark));
        *applied_dark = dark;
    });
}

fn find_css_path(dark: bool) -> PathBuf {
    let filename = if dark { "dark.css" } else { "light.css" };
    if let Ok(executable) = std::env::current_exe() {
        if let Some(parent) = executable.parent() {
            let installed = parent.join("../share/maintainer/styles").join(filename);
            if installed.exists() {
                return installed;
            }
            let development = parent.join("../../data/styles").join(filename);
            if development.exists() {
                return development;
            }
        }
    }
    #[cfg(debug_assertions)]
    {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../data/styles")
            .join(filename)
    }
    #[cfg(not(debug_assertions))]
    {
        PathBuf::from("/usr/share/maintainer/styles").join(filename)
    }
}

/// Setup the application: connect activation signal and run.
pub fn run_application(app: &Application) {
    if crate::probe::enabled() {
        app.connect_shutdown(|_| crate::probe::mark("application-shutdown"));
        app.connect_window_added(|_, window| {
            window.connect_close_request(|_| {
                crate::probe::mark("close-requested");
                gtk4::glib::Propagation::Proceed
            });
            window.connect_destroy(|_| crate::probe::mark("window-destroyed"));
            window.connect_map(|window| {
                crate::probe::mark_once("mapped");
                if let Some(renderer) = window.renderer() {
                    crate::probe::mark_once(&format!("renderer:{}", renderer.type_().name()));
                }
            });
        });
    }
    app.connect_activate(|app| {
        let runtime = Runtime::init();
        apply_theme(runtime.borrow().settings.theme);
        let shell = Shell::new(app, runtime);
        shell.present();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unchanged_theme_preserves_loaded_css_and_changed_theme_reloads_it() {
        gtk4::init().expect("GTK display required; use scripts/test-native.sh");
        apply_theme(Theme::Light);
        let provider = CSS_PROVIDER.with(|slot| slot.borrow().as_ref().unwrap().0.clone());
        provider.load_from_data(".theme-cache-canary { color: red; }");
        apply_theme(Theme::Light);
        assert!(provider.to_str().contains("theme-cache-canary"));
        apply_theme(Theme::Dark);
        assert!(!provider.to_str().contains("theme-cache-canary"));
        assert!(CSS_PROVIDER.with(|slot| slot.borrow().as_ref().unwrap().0 == provider));
    }
}
