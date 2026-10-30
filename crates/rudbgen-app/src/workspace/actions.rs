//! Actions.

use super::*;

impl Workspace {
    /// Whether a shortcut meant for the window has to stand down, because a
    /// dialog is on screen.
    ///
    /// The bindings live on the shell's root, and every dialog is drawn inside
    /// it, so without this a chord pressed in a dialog's field reaches past
    /// the dialog to the window behind: `Ctrl+N` reopened the connection
    /// dialog over its own draft, `Ctrl+S` saved the template under the
    /// backdrop, `Ctrl+B` folded a panel nobody could see. A dialog that has a
    /// meaning for one of those chords binds it in its own key context, which
    /// gpui matches ahead of this one (see `bind_shortcuts`).
    pub(super) fn shortcut_blocked(&self, cx: &App) -> bool {
        self.dialog_open(cx)
    }

    // --- the template actions ---------------------------------------------

    /// Creates a template file through the platform's save dialog.
    pub(super) fn new_template_action(
        &mut self,
        _: &NewTemplate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.create_template(cx);
    }

    /// Opens a template file through the platform picker.
    pub(super) fn open_template_action(
        &mut self,
        _: &OpenTemplate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.choose_template(cx);
    }

    /// Writes the template on top back to its file.
    pub(super) fn save_template_action(
        &mut self,
        _: &SaveTemplate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let pane = self.templates[index].clone();
        pane.update(cx, |pane, cx| pane.save(cx));
        self.retitle_templates(cx);
    }

    /// Shows and hides the live preview beside the editor.
    pub(super) fn toggle_live_preview_action(
        &mut self,
        _: &ToggleLivePreview,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let pane = self.templates[index].clone();
        pane.update(cx, |pane, cx| pane.toggle_preview(cx));
    }

    /// Offers what may be written where the caret is.
    pub(super) fn trigger_completion_action(
        &mut self,
        _: &TriggerCompletion,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let pane = self.templates[index].clone();
        pane.update(cx, |pane, cx| pane.trigger_completion(cx));
    }

    // --- actions ----------------------------------------------------------

    /// Opens the connection dialog on a new, empty profile.
    pub(super) fn new_connection_action(
        &mut self,
        _: &NewConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.open_connection_dialog(ConnectionDoor::New, window, cx);
    }

    /// Opens the connection dialog on the saved profiles.
    pub(super) fn manage_connections_action(
        &mut self,
        _: &ManageConnections,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.open_connection_dialog(ConnectionDoor::Manage, window, cx);
    }

    /// Opens the settings dialog.
    pub(super) fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.open_settings(window, cx);
    }

    /// Opens the abbreviation rules editor.
    pub(super) fn edit_abbreviations_action(
        &mut self,
        _: &EditAbbreviations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.open_abbreviations(window, cx);
    }

    /// Opens the jdbgen import wizard.
    pub(super) fn import_jdbgen_action(
        &mut self,
        _: &ImportJdbgen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.open_import(window, cx);
    }

    /// Opens the about box.
    pub(super) fn show_about_action(
        &mut self,
        _: &ShowAbout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.open_about(window, cx);
    }

    /// Asks GitHub for the latest release.
    pub(super) fn check_updates_action(
        &mut self,
        _: &CheckUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.check_updates(window, cx);
    }

    /// Shows and hides the explorer sidebar.
    pub(super) fn toggle_explorer_action(
        &mut self,
        _: &ToggleExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.explorer_visible = !self.explorer_visible;
        self.hid_a_panel(window, cx);
    }

    /// Shows and hides the inspector panel.
    pub(super) fn toggle_inspector_action(
        &mut self,
        _: &ToggleInspector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.inspector_visible = !self.inspector_visible;
        self.hid_a_panel(window, cx);
    }

    /// Runs the generator.
    pub(super) fn generate_action(
        &mut self,
        _: &Generate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.start_run(RunKind::Generate, cx);
    }

    /// Renders one pair into the Preview tab.
    pub(super) fn preview_action(
        &mut self,
        _: &Preview,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.start_run(RunKind::Preview, cx);
    }

    /// Renders every pair into memory.
    pub(super) fn dry_run_action(
        &mut self,
        _: &DryRun,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.start_run(RunKind::DryRun, cx);
    }

    /// Closes whatever overlay is on top, in the order they are stacked.
    pub(super) fn dismiss_dialog_action(
        &mut self,
        _: &DismissDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The dropdown menu paints above everything else, so it goes first.
        if self.menu_open {
            self.set_menu_open(false, cx);
            return;
        }
        if self.close_template_menus(cx) {
            return;
        }
        if self.close_context_menus(cx) {
            return;
        }
        // The Generate tab's own two questions — the name of a set, and
        // whether to delete one — are drawn inside the tab, and so are not
        // anywhere on this list unless the tab is asked.
        if self.generate.update(cx, |pane, cx| pane.escape(cx)) {
            self.return_focus(window, cx);
            return;
        }
        // Then the "save before closing?" question, which is the innermost
        // thing on screen whenever it is up: it was opened by a press on a
        // tab's close button, over whatever else was already there.
        if self.closing.is_some() {
            self.cancel_close(window, cx);
            return;
        }
        // The questions the shell asks about a run and a connection, each of
        // which has a Cancel that `Escape` stands for.
        if self.unsaved_run.is_some() {
            self.answer_unsaved_run(None, window, cx);
            return;
        }
        if self.deleting_profile.is_some() {
            self.deleting_profile = None;
            self.return_focus(window, cx);
            return;
        }
        if self.password_prompt.is_some() {
            self.cancel_password(window, cx);
            return;
        }
        if self.update.read(cx).is_open() {
            // Swallowed rather than propagated while an install runs: the key
            // must not reach anything else, but nothing may take the screen
            // from a swap either, so `Escape` simply does nothing until it is
            // over.
            if !self.update.read(cx).is_busy() {
                self.update.update(cx, |dialog, cx| dialog.close(cx));
                self.return_focus(window, cx);
            }
            return;
        }
        if self.job.read(cx).is_open() {
            // The overwrite question is the one stage `Escape` answers: it
            // has a Cancel of its own, and "never mind" is exactly what that
            // answer means. Cancel stops the run there, so the files already
            // written stay written and the summary lists them.
            if self.job.read(cx).is_asking() {
                self.job.update(cx, |job, cx| job.cancel(cx));
                return;
            }
            // Swallowed rather than propagated while a run is going: the key
            // must not reach anything else, and a run is stopped with Cancel —
            // which says what it does — rather than by a key that elsewhere
            // means "never mind".
            if !self.job.read(cx).is_busy() {
                self.job.update(cx, |job, cx| job.close(cx));
            }
            return;
        }
        if self.about.read(cx).is_open() {
            self.about.update(cx, |dialog, cx| dialog.close(cx));
            self.return_focus(window, cx);
            return;
        }
        if self.abbreviations.read(cx).is_open() {
            // Routed through the dialog: it stacks a table-name dropdown of its
            // own, and that has to be able to take `Escape` for itself before
            // the whole draft is thrown away.
            self.abbreviations
                .update(cx, |dialog, cx| dialog.escape(cx));
            return;
        }
        if self.import.read(cx).is_open() {
            // Routed through the wizard, which refuses the key while it is
            // decrypting or writing: half an import is not a state it can be
            // left in.
            self.import.update(cx, |dialog, cx| dialog.escape(cx));
            return;
        }
        if self.connection_dialog.read(cx).is_open() {
            // Routed through the dialog for the reason the settings dialog is:
            // it stacks the driver editor and a delete confirmation of its own,
            // and each has to be able to take `Escape` for itself before the
            // whole form is thrown away.
            self.connection_dialog
                .update(cx, |dialog, cx| dialog.escape(cx));
            return;
        }
        if self.settings.read(cx).is_open() {
            // Routed through the dialog rather than closed from here: it stacks
            // a color editor, two dropdowns and a delete confirmation of its
            // own, and each of those has to be able to take `Escape` for itself
            // before the whole form is thrown away. gpui matches key bindings
            // ahead of key listeners, so this handler — not the dialog's own —
            // is where the key actually lands.
            self.settings.update(cx, |dialog, cx| dialog.escape(cx));
            return;
        }
        cx.propagate();
    }
}

impl Workspace {
    // --- the tab strip ----------------------------------------------------

    /// Brings the next tab to the front, wrapping at the end.
    pub(super) fn next_tab_action(
        &mut self,
        _: &NextTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.cycle_tab(true, window, cx);
    }

    /// Brings the previous tab to the front, wrapping at the start.
    pub(super) fn prev_tab_action(
        &mut self,
        _: &PrevTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        self.cycle_tab(false, window, cx);
    }

    /// Brings the tab the chord's digit names to the front.
    pub(super) fn select_tab_action(
        &mut self,
        action: &SelectTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) || !self.work_area_showing() {
            return;
        }
        self.activate_tab(action.0, window, cx);
    }

    /// Closes the tab on top, as its close button would — asking first when
    /// it holds unsaved edits, and not at all for the Generate tab.
    pub(super) fn close_tab_action(
        &mut self,
        _: &CloseTab,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) || !self.work_area_showing() {
            return;
        }
        let index = self.pane.active_index();
        self.request_close_tab(index, window, cx);
    }

    /// Puts the keyboard on the tab strip.
    pub(super) fn focus_tabs_action(
        &mut self,
        _: &FocusTabs,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) || !self.work_area_showing() {
            return;
        }
        self.close_template_menus(cx);
        window.focus(&self.tabs_focus, cx);
        cx.notify();
    }

    /// Moves the caret to the next problem of the template on top.
    pub(super) fn next_diagnostic_action(
        &mut self,
        _: &NextDiagnostic,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_diagnostic(true, window, cx);
    }

    /// Moves the caret to the previous problem of the template on top.
    pub(super) fn prev_diagnostic_action(
        &mut self,
        _: &PrevDiagnostic,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.step_diagnostic(false, window, cx);
    }

    /// The template on top's problems, one step either way.
    fn step_diagnostic(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.shortcut_blocked(cx) {
            return;
        }
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let pane = self.templates[index].clone();
        pane.update(cx, |pane, cx| pane.step_diagnostic(forward, window, cx));
    }

    /// Adds the template on top to the connection's generation list.
    pub(super) fn add_to_generation_action(
        &mut self,
        _: &AddToGeneration,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) {
            return;
        }
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let file = self.templates[index].read(cx).path().to_path_buf();
        self.add_to_generation(file, cx);
    }
}

impl Workspace {
    // --- menus from the keyboard ------------------------------------------

    /// `F10`: the application menu, on its first row.
    ///
    /// Off macOS only, which is where the ☰ menu is: on macOS the commands are
    /// the menu bar's, and the key is left to whatever it means there.
    pub(super) fn open_app_menu_action(
        &mut self,
        _: &rugpui::OpenAppMenu,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if cfg!(target_os = "macos") || self.shortcut_blocked(cx) {
            cx.propagate();
            return;
        }
        self.set_menu_open(true, cx);
        self.menu_by_keyboard = true;
        cx.notify();
    }

    /// `Shift+F10` and the Menu key, for the surfaces the shell draws menus
    /// for itself: the tab strip, while it has the keyboard, opens the menu of
    /// the tab on top. The explorer, the template editor and the welcome rows
    /// answer the key themselves, deeper down.
    pub(super) fn open_context_menu_action(
        &mut self,
        _: &rugpui::OpenContextMenu,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.shortcut_blocked(cx) || !self.tabs_focus.is_focused(window) {
            cx.propagate();
            return;
        }
        let origin = self.tabs_bounds.get().bottom_left();
        self.close_template_menus(cx);
        self.profile_menu = None;
        self.tab_menu = Some((
            self.pane.active_index(),
            origin + gpui::point(px(16.), px(0.)),
        ));
        self.tab_menu_by_keyboard = true;
        cx.notify();
    }
}
