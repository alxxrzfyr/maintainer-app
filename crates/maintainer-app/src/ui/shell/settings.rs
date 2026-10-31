//! Settings page controller and dispatch wiring.

use super::*;

impl Shell {
    pub(super) fn refresh_mounted_settings(
        runtime: &RuntimeHandle,
        window: &ApplicationWindow,
        stack: &Stack,
    ) {
        let Some(host) = stack.child_by_name("Settings").and_downcast::<GtkBox>() else {
            return;
        };
        if host.first_child().is_none() {
            return;
        }
        while let Some(child) = host.first_child() {
            host.remove(&child);
        }
        host.append(&Self::build_runtime_settings_page(runtime, window));
    }

    pub(super) fn wire_settings_close_drain(
        window: &ApplicationWindow,
        runtime: &RuntimeHandle,
        pages: &Stack,
    ) {
        use crate::runtime::SettingsCloseState;
        let allow_close = Rc::new(Cell::new(false));
        let draining = Rc::new(Cell::new(false));
        let runtime = runtime.clone();
        let pages = pages.clone();
        window.connect_close_request(move |window| {
            if allow_close.get() {
                return glib::Propagation::Proceed;
            }
            if draining.get() {
                return glib::Propagation::Stop;
            }
            if runtime.borrow().settings_close_state(false) == SettingsCloseState::Ready {
                return glib::Propagation::Proceed;
            }
            draining.set(true);
            window.set_sensitive(false);
            let progress = gtk4::MessageDialog::builder().transient_for(window).modal(true)
                .text("Saving local changes before closing…")
                .secondary_text("Please wait for settings and confirmed data clearing to finish.")
                .build();
            let allow_close = allow_close.clone();
            let draining = draining.clone();
            let runtime = runtime.clone();
            let pages = pages.clone();
            let window = window.clone();
            let deadline = Instant::now() + Duration::from_secs(5);
            progress.present();
            glib::timeout_add_local(Duration::from_millis(50), move || {
                let mut rt = runtime.borrow_mut();
                let reset = rt.poll_settings_writes().is_some_and(|result| result.reset_defaults);
                let state = rt.settings_close_state(Instant::now() >= deadline);
                let theme = rt.settings.theme;
                drop(rt);
                if reset {
                    crate::application::apply_theme(theme);
                    Self::refresh_mounted_settings(&runtime, &window, &pages);
                }
                match state {
                    SettingsCloseState::Pending => ControlFlow::Continue,
                    SettingsCloseState::Ready => {
                        progress.close();
                        allow_close.set(true);
                        window.close();
                        ControlFlow::Break
                    }
                    SettingsCloseState::Failed(error) => {
                        progress.close();
                        let failure = gtk4::MessageDialog::builder().transient_for(&window).modal(true)
                            .message_type(gtk4::MessageType::Warning)
                            .text("Local changes could not be confirmed saved")
                            .secondary_text(format!("{error}\n\nStay open to keep working, or exit with unsaved or incomplete changes."))
                            .build();
                        failure.add_button("Stay Open", gtk4::ResponseType::Cancel);
                        failure.add_button("Exit Anyway", gtk4::ResponseType::Accept);
                        failure.set_default_response(gtk4::ResponseType::Cancel);
                        let window = window.clone();
                        let allow_close = allow_close.clone();
                        let draining = draining.clone();
                        failure.connect_response(move |dialog, response| {
                            dialog.close();
                            window.set_sensitive(true);
                            draining.set(false);
                            if response == gtk4::ResponseType::Accept {
                                allow_close.set(true);
                                window.close();
                            }
                        });
                        failure.present();
                        ControlFlow::Break
                    }
                }
            });
            glib::Propagation::Stop
        });
    }

    pub(super) fn build_runtime_settings_page(
        runtime: &RuntimeHandle,
        window: &ApplicationWindow,
    ) -> gtk4::ScrolledWindow {
        let settings_snapshot = runtime.borrow().settings.clone();
        let runtime_for_settings = runtime.clone();
        let window_for_settings = window.clone();
        let settings_widget = build_settings_page(&settings_snapshot, move |intent| match intent {
            crate::state::SettingsIntent::RequestDeleteAllHistory => {
                Self::confirm_settings_deletion(&window_for_settings, &runtime_for_settings, false);
            }
            crate::state::SettingsIntent::RequestDeleteAllData => {
                Self::confirm_settings_deletion(&window_for_settings, &runtime_for_settings, true);
            }
            intent => {
                let requested_theme = match &intent {
                    crate::state::SettingsIntent::SetTheme(theme) => Some(*theme),
                    _ => None,
                };
                let mut rt = runtime_for_settings.borrow_mut();
                if rt.db_loading || rt.clearing_settings {
                    return;
                }
                let mut settings_state =
                    crate::state::SettingsState::from_snapshot(rt.settings.clone());
                if settings_state.reduce(intent) {
                    rt.settings = settings_state.snapshot.clone();
                    rt.monitoring.set_sampling_intervals(
                        settings_state.snapshot.refresh_interval_secs,
                        settings_state.snapshot.hidden_interval_secs,
                    );
                    let saved = rt.save_settings(to_core_settings(&settings_state.snapshot));
                    drop(rt);
                    if let Err(message) = saved {
                        Self::show_settings_result(&window_for_settings, false, &message);
                    }
                    if let Some(theme) = requested_theme {
                        crate::application::apply_theme(theme);
                    }
                }
            }
        });

        settings_widget.set_sensitive({
            let rt = runtime.borrow();
            !rt.db_loading && !rt.clearing_settings
        });
        settings_widget
    }

    pub(super) fn confirm_settings_deletion(
        parent: &ApplicationWindow,
        runtime: &RuntimeHandle,
        delete_all_data: bool,
    ) {
        let (title, body, confirm_label) = if delete_all_data {
            (
                "Delete all Maintainer data?",
                "This permanently removes local settings and maintenance history. The database schema is preserved and default settings are recreated.",
                "Delete All Data",
            )
        } else {
            (
                "Delete all history?",
                "This permanently removes operation, diagnostic, and scan history. Settings are preserved.",
                "Delete History",
            )
        };

        let dialog = gtk4::MessageDialog::builder()
            .transient_for(parent)
            .modal(true)
            .message_type(gtk4::MessageType::Warning)
            .text(title)
            .secondary_text(body)
            .build();
        dialog.add_button("Cancel", gtk4::ResponseType::Cancel);
        dialog.add_button(confirm_label, gtk4::ResponseType::Accept);

        let runtime_for_response = runtime.clone();
        let parent_for_response = parent.clone();
        dialog.connect_response(move |dialog, response| {
            dialog.close();
            if response != gtk4::ResponseType::Accept {
                return;
            }

            if delete_all_data {
                let result = runtime_for_response.borrow_mut().clear_settings_data();
                if let Err(message) = result {
                    Self::show_settings_result(&parent_for_response, false, &message);
                }
                return;
            }

            let rt = runtime_for_response.borrow();
            let Some(database) = rt.db.clone() else {
                Self::show_settings_result(
                    &parent_for_response,
                    false,
                    "Local database is unavailable; no data was changed.",
                );
                return;
            };
            let executor = rt.executor.clone();
            drop(rt);

            let parent_for_done = parent_for_response.clone();
            crate::dispatch::dispatch_local_action(
                &executor,
                move || {
                    let result = database.clear_history();
                    result.map_err(|error| error.to_string())
                },
                move |result| match result {
                    Ok(deleted) => {
                        Self::show_settings_result(
                            &parent_for_done,
                            true,
                            &format!("Deleted {deleted} local record(s)."),
                        );
                    }
                    Err(error) => Self::show_settings_result(
                        &parent_for_done,
                        false,
                        &format!("Nothing was deleted: {error}"),
                    ),
                },
            );
        });
        dialog.present();
    }

    pub(super) fn show_settings_result(parent: &ApplicationWindow, success: bool, message: &str) {
        let dialog = gtk4::MessageDialog::builder()
            .transient_for(parent)
            .modal(true)
            .message_type(if success {
                gtk4::MessageType::Info
            } else {
                gtk4::MessageType::Error
            })
            .text(if success {
                "Completed"
            } else {
                "Action failed"
            })
            .secondary_text(message)
            .build();
        dialog.add_button("Close", gtk4::ResponseType::Close);
        dialog.connect_response(|dialog, _| dialog.close());
        dialog.present();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[ignore = "requires a GTK display; run this exact test under Xvfb"]
    fn selecting_theme_changes_native_gtk_controls_immediately() {
        gtk4::init().unwrap();
        crate::application::apply_theme(crate::state::Theme::Light);
        let runtime = crate::runtime::Runtime::isolated_for_test();
        let app = gtk4::Application::new(None::<&str>, gio::ApplicationFlags::NON_UNIQUE);
        app.register(None::<&gio::Cancellable>).unwrap();
        let window = ApplicationWindow::new(&app);
        let page = Shell::build_runtime_settings_page(&runtime, &window);
        let mut widgets = vec![page.upcast::<gtk4::Widget>()];
        let mut theme_combo = None;
        while let Some(widget) = widgets.pop() {
            if let Some(combo) = widget.downcast_ref::<gtk4::ComboBoxText>() {
                theme_combo = Some(combo.clone());
                break;
            }
            let mut child = widget.first_child();
            while let Some(current) = child {
                widgets.push(current.clone());
                child = current.next_sibling();
            }
        }
        let combo = theme_combo.expect("Theme selection control");
        let settings = gtk4::Settings::default().unwrap();
        for (selection, theme, dark) in [
            ("dark", crate::state::Theme::Dark, true),
            ("light", crate::state::Theme::Light, false),
        ] {
            combo.set_active_id(Some(selection));
            assert_eq!(runtime.borrow().settings.theme, theme);
            assert_eq!(
                settings.is_gtk_application_prefer_dark_theme(),
                dark,
                "settings selection must theme native controls before returning"
            );
        }
    }
}
