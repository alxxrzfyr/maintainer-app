use gtk4::prelude::*;
use maintainer_app::application;
use std::os::unix::process::CommandExt;

fn main() {
    // `geteuid` takes no pointers or other caller-controlled values.
    if is_root(unsafe { libc::geteuid() }) {
        eprintln!(
            "Maintainer must run as a normal user. Install packages with sudo, then launch Maintainer from your desktop session."
        );
        std::process::exit(1);
    }

    let automatic_vulkan = configure_renderer();
    let fallback = std::rc::Rc::new(std::cell::Cell::new(false));
    // Startup measurements begin at the harness's spawn, including bootstrap.
    maintainer_app::probe::mark_once("process-start");
    let app = application::build_application();
    if automatic_vulkan {
        let fallback = fallback.clone();
        let checked = std::rc::Rc::new(std::cell::Cell::new(false));
        app.connect_window_added(move |app, window| {
            let app = app.downgrade();
            let fallback = fallback.clone();
            let checked = checked.clone();
            window.connect_map(move |window| {
                if !checked.replace(true)
                    && window
                        .renderer()
                        .is_none_or(|renderer| renderer.type_().name() != "GskVulkanRenderer")
                {
                    fallback.set(true);
                    if let Some(app) = app.upgrade() {
                        app.quit();
                    }
                }
            });
        });
    }
    application::run_application(&app);
    let exit_code = app.run();
    if fallback.get() {
        for window in app.windows() {
            window.destroy();
        }
        drop(app);
        eprintln!("Vulkan is unavailable; restarting with GTK's default renderer");
        restart_renderer(false);
        std::process::exit(1);
    }
    drop(app);
    std::process::exit(exit_code.into());
}

fn is_root(effective_uid: libc::uid_t) -> bool {
    effective_uid == 0
}

fn prefer_vulkan(gtk_minor: u32, bootstrapped: bool, explicit_settings: bool) -> bool {
    gtk_minor == 14 && !bootstrapped && !explicit_settings
}

fn configure_renderer() -> bool {
    let stage = std::env::var_os("MAINTAINER_RENDERER_BOOTSTRAP");
    let explicit_settings =
        std::env::var_os("GSK_RENDERER").is_some() || std::env::var_os("GDK_DEBUG").is_some();
    if prefer_vulkan(gtk4::minor_version(), stage.is_some(), explicit_settings) {
        restart_renderer(true);
    }
    stage.as_deref() == Some(std::ffi::OsStr::new("vulkan"))
}

fn restart_renderer(vulkan: bool) {
    let executable = match std::env::current_exe() {
        Ok(path) => path,
        Err(error) => {
            eprintln!("Renderer bootstrap unavailable: {error}");
            return;
        }
    };
    // A self-exec sets child-only environment safely before application services
    // start; it avoids mutating the environment of GTK's worker threads.
    let mut command = std::process::Command::new(executable);
    command.args(std::env::args_os().skip(1));
    if vulkan {
        command
            .env("GSK_RENDERER", "vulkan")
            .env("GDK_DEBUG", "gl-disable")
            .env("MAINTAINER_RENDERER_BOOTSTRAP", "vulkan");
    } else {
        command
            .env_remove("GSK_RENDERER")
            .env_remove("GDK_DEBUG")
            .env("MAINTAINER_RENDERER_BOOTSTRAP", "native");
    }
    let error = command.exec();
    eprintln!("Renderer restart unavailable; continuing with GTK: {error}");
}

#[cfg(test)]
mod tests {
    use super::{is_root, prefer_vulkan};

    #[test]
    fn renderer_preference_respects_the_gtk_floor_user_settings_and_restart_guard() {
        assert!(prefer_vulkan(14, false, false));
        assert!(!prefer_vulkan(6, false, false));
        assert!(!prefer_vulkan(16, false, false));
        assert!(!prefer_vulkan(14, true, false));
        assert!(!prefer_vulkan(14, false, true));
    }

    #[test]
    fn launch_identity_check_rejects_root_and_accepts_regular_users() {
        assert!(is_root(0));
        assert!(!is_root(1));
        assert!(!is_root(1000));
    }
}
