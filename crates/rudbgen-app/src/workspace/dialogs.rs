//! Dialogs.

use super::*;

impl Workspace {
    // --- focus ------------------------------------------------------------

    /// Puts the keyboard back on the shell after a dialog closes.
    pub(super) fn focus_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // --- dialogs ----------------------------------------------------------

    /// Whether any modal is on screen.
    ///
    /// Exactly the set [`Workspace::close_overlays`] closes, minus the dropdown
    /// menu: it is transient and dismisses itself on the next press, so a
    /// dialog appearing over one takes nothing away.
    ///
    /// One caller, and the reason this exists at all: the start-up update check
    /// announces itself only into an empty window. It is the one dialog nobody
    /// asked for, and it must not land on top of something the user opened.
    pub(super) fn dialog_open(&self, cx: &App) -> bool {
        self.closing.is_some()
            || self.about.read(cx).is_open()
            || self.settings.read(cx).is_open()
            || self.update.read(cx).is_open()
            || self.connection_dialog.read(cx).is_open()
            || self.abbreviations.read(cx).is_open()
            || self.import.read(cx).is_open()
            || self.job.read(cx).is_open()
    }

    /// Closes every dialog and the dropdown menu.
    ///
    /// Every `open_*` method starts here, which is what keeps the modals
    /// mutually exclusive: only one of them can be on screen at a time, and
    /// opening one always puts the menu away.
    ///
    /// Closing the settings dialog drops its live preview, so the palettes are
    /// re-applied on the way out; without that the window would keep wearing a
    /// theme that nothing in the settings names any more.
    ///
    /// The update dialog is closed here like the rest, so a user who reaches
    /// for a command instead of one of its buttons is not left with a stale
    /// announcement floating over the window — except while it is installing,
    /// when its own `close` refuses and the swap is allowed to finish; see
    /// [`UpdateDialog::close`].
    pub(super) fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_open = false;
        // A question nobody answered is a question withdrawn: the tab stays
        // open with its edits, which is the safe half of both answers.
        self.closing = None;
        if self.about.read(cx).is_open() {
            self.about.update(cx, |dialog, cx| dialog.close(cx));
        }
        if self.settings.read(cx).is_open() {
            self.settings.update(cx, |dialog, cx| dialog.close(cx));
            self.apply_preview(window, cx);
        }
        if self.update.read(cx).is_open() {
            self.update.update(cx, |dialog, cx| dialog.close(cx));
        }
        if self.connection_dialog.read(cx).is_open() {
            self.connection_dialog
                .update(cx, |dialog, cx| dialog.close(cx));
        }
        if self.abbreviations.read(cx).is_open() {
            self.abbreviations.update(cx, |dialog, cx| dialog.close(cx));
        }
        if self.import.read(cx).is_open() {
            self.import.update(cx, |dialog, cx| dialog.close(cx));
        }
        // A run in flight is the one overlay a command may not take the screen
        // from: its own `close` refuses while it is busy, exactly as the
        // update dialog's does mid-install.
        if self.job.read(cx).is_open() {
            self.job.update(cx, |job, cx| job.close(cx));
        }
    }

    /// Opens the about dialog, closing whatever else was showing.
    pub(super) fn open_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        self.about.update(cx, |dialog, cx| dialog.open(cx));
        cx.notify();
    }

    /// Asks GitHub for the latest release and shows the answer.
    ///
    /// Goes through `close_overlays` where the start-up check pointedly does
    /// not: this dialog was asked for, so it is entitled to the screen the way
    /// every other menu command is.
    ///
    /// Refuses while an install is already running, which is the one case where
    /// the update dialog cannot be closed and so must not be reopened into a
    /// different state.
    pub(super) fn check_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update.read(cx).is_busy() {
            return;
        }
        self.close_overlays(window, cx);
        self.update.update(cx, |dialog, cx| dialog.start_check(cx));
        cx.notify();
    }

    /// Opens the abbreviation rules editor over the rules the panel holds.
    ///
    /// The rules and the switch come from the Generate tab rather than from
    /// disk: the panel read the file when the connection opened and owns the
    /// only copy a run will use, so anything else would put two answers on
    /// screen. The table names come from the explorer, which is what makes the
    /// whole-name picker worth having; with nothing connected the list is empty
    /// and the picker is simply not drawn.
    pub(super) fn open_abbreviations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        let store = self.generate.read(cx).abbreviations().clone();
        let tables = self.explorer.read(cx).loaded_table_names(cx);
        self.abbreviations
            .update(cx, |dialog, cx| dialog.open(&store, tables, cx));
        cx.notify();
    }

    /// Opens the jdbgen import wizard over the configuration it can find.
    ///
    /// [`Workspace::jdbgen_config`] answers only when the file is actually
    /// there (§4.3); when it is not — which is how the menu row is reached, the
    /// welcome screen leaving its button out — the wizard still opens, on the
    /// data directory's path, which its first step reports as unreadable and
    /// offers to replace with a file the user chooses. That is the door for a
    /// configuration copied from another machine, and the reason the command is
    /// never greyed out.
    pub(super) fn open_import(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        let path = self.jdbgen_config.clone().unwrap_or_else(|| {
            rudbgen_import::data_dir()
                .unwrap_or_default()
                .join("config.json")
        });
        self.import.update(cx, |dialog, cx| dialog.open(path, cx));
        cx.notify();
    }

    /// Takes on the language and theme jdbgen was last run in.
    ///
    /// The wizard asked; this applies. Settings are the shell's — it is what
    /// owns `settings.json`, the live locale and the palettes — so a dialog
    /// never writes them.
    pub(super) fn adopt_jdbgen_settings(
        &mut self,
        hint: &rudbgen_import::SettingsHint,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut settings = app_settings::current(cx);
        if let Some(language) = &hint.language {
            settings.language = Some(language.clone());
        }
        settings.theme = if hint.dark_ui {
            DARK_THEME.to_string()
        } else {
            LIGHT_THEME.to_string()
        };
        app_settings::replace(settings, cx);
        app_settings::save(cx);
        self.apply_settings(window, cx);
    }

    /// Opens the connection dialog over the profile that is open, if one is.
    pub(super) fn open_connection_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        // What the driver editor's custom-query **Test** runs against when the
        // driver being edited is the one this session was opened through.
        let session = self.connection.session().map(|connected| {
            (
                self.connection
                    .profile()
                    .map(|profile| profile.driver_id.clone())
                    .unwrap_or_default(),
                connected.handle(),
            )
        });
        let at = self.connection.profile().map(|profile| profile.id);
        self.connection_dialog.update(cx, |dialog, cx| {
            dialog.set_open_session(session);
            match at {
                Some(id) => dialog.open_at(id, cx),
                None => dialog.open(cx),
            }
        });
        cx.notify();
    }

    // --- the session ------------------------------------------------------

    /// Opens a session for `profile`, replacing whatever was open.
    ///
    /// Everything that blocks — the keychain read, the tunnel, `JNI_CreateJavaVM`
    /// on the first connection of the run, `OPEN_SESSION` — happens on a
    /// background task, so the window stays live while a database that is not
    /// answering takes its time about saying so.
    pub(super) fn connect_to(&mut self, profile: ConnectionProfile, cx: &mut Context<Self>) {
        self.close_session();

        let drivers = match DriverStore::load() {
            Ok(drivers) => drivers,
            Err(error) => {
                log::error!("could not read drivers.json: {error:#}");
                DriverStore::default()
            }
        };
        let Some(driver) = drivers.get(&profile.driver_id).cloned() else {
            let message = ts!("connect.no_driver", driver = profile.driver_id.clone());
            self.connection = ConnectionState::Failed {
                profile: Box::new(profile),
                message,
            };
            cx.notify();
            return;
        };

        let settings = app_settings::current(cx);
        let opening = profile.clone();
        let definition = driver.clone();
        let run = cx.background_spawn(async move {
            // Read here rather than on the UI thread: a keychain that is locked
            // puts a system prompt up, and waiting for that on the UI thread
            // would freeze the window behind it.
            let credentials = Credentials::read(&opening);
            connection::connect(&opening, &driver, &credentials, &settings)
        });
        let task = cx.spawn(async move |this, cx| {
            let outcome = run.await;
            this.update(cx, |workspace, cx| workspace.connected(outcome, cx))
                .ok();
        });

        self.connection = ConnectionState::Connecting {
            profile: Box::new(profile),
            driver: Box::new(definition),
            _task: task,
        };
        cx.notify();
    }

    /// Shows or hides the help menu.
    pub(super) fn set_menu_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.menu_open != open {
            self.menu_open = open;
            cx.notify();
        }
    }

    /// Re-applies everything a saved settings file changes about the live
    /// window.
    ///
    /// Every platform call in here acts on the window, and one of them —
    /// `request_decorations` on X11 — is the call that used to re-enter gpui's
    /// window callbacks and panic. It is safe from this stack: the settings
    /// dialog emits its event, gpui delivers it after the button's own callback
    /// has returned and released every borrow, and this runs from there. It must
    /// stay that way; calling it from inside a widget callback would put the
    /// borrow back.
    pub(super) fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = app_settings::current(cx);
        // Before the repaint below, so the next frame is already drawn in the
        // newly chosen language.
        i18n::apply(settings.language.as_deref());
        // The native macOS menu bar is built once and owned by the platform, so
        // unlike the in-app menu it does not follow a repaint; it has to be
        // handed over again.
        cx.set_menus(app_menus());
        // The grid borrows its column names from the source it was given, so a
        // language change has to be written into it; everything else on the two
        // panels is built afresh every frame.
        self.inspector.update(cx, |panel, cx| panel.relabel(cx));
        apply_themes(&settings, cx);
        // Ahead of the repaint, so the title bar's next frame already knows
        // whether it has to stand in for a caption; and ahead of the two calls
        // below, which leave the accent policy and the caption colors on the
        // window, so a caption that comes back here comes back already themed.
        //
        // The field follows the call rather than the stored setting: everything
        // that branches on it is asking what the window carries, not what was
        // last saved.
        if settings.window.titlebar != self.titlebar {
            self.titlebar = settings.window.titlebar;
            let custom = self.titlebar == TitlebarStyle::Custom;
            window.set_titlebar_transparent(custom, custom.then_some(TRAFFIC_LIGHT_ORIGIN));
            // The Linux counterpart of the call above, which only the Windows
            // and macOS backends implement: swap the compositor's frame for
            // client-side decorations (or back) on the live window.
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            window.request_decorations(if custom {
                gpui::WindowDecorations::Client
            } else {
                gpui::WindowDecorations::Server
            });
        }
        // Paired with the call below, and never with a preview: the leaf crates
        // read this to decide whether to paint their own background, and the
        // answer is only right once the surface itself permits alpha. Ahead of
        // the repaint, so the next frame already draws under the new answer.
        set_window_tint(settings.window.background_opacity, cx);
        cx.refresh_windows();
        window.set_background_appearance(window_appearance(&settings.window));
        // After the background appearance, never before: on Windows that call
        // re-arms the accent policy that would otherwise repaint the caption out
        // from under us.
        apply_caption_theme(window, &theme(cx), cx);
    }

    /// Re-applies the palettes the settings dialog is currently showing.
    ///
    /// The unsaved half of [`Workspace::apply_settings`], and deliberately much
    /// smaller: only the two palettes and the fonts are previewed, so this
    /// touches no platform state beyond the native caption's colours, which have
    /// to follow the chrome theme or the window would be half repainted.
    ///
    /// Reads [`app_settings::effective`], which answers the preview while one is
    /// installed and the saved settings once it is dropped — so the same call
    /// both applies a preview and undoes it.
    pub(super) fn apply_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        apply_themes(&app_settings::effective(cx), cx);
        cx.refresh_windows();
        apply_caption_theme(window, &theme(cx), cx);
    }

    /// Takes the keyboard back and records what the layout now is.
    ///
    /// The focus half is not optional: the panel that has just gone may have
    /// been holding the keyboard — the filter box, the tree — and a focus
    /// handle left dangling on an unrendered element takes every shortcut in
    /// the window with it (architecture document, Appendix A). It has to happen
    /// in the same update that hides the subtree, which is this one.
    pub(super) fn hid_a_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_shell(window, cx);
        self.remember_layout(cx);
        cx.notify();
    }
}
