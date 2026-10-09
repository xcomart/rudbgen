//! Actions.

use super::*;

impl Workspace {
    // --- the template actions ---------------------------------------------

    /// Opens a template file through the platform picker.
    pub(super) fn open_template_action(
        &mut self,
        _: &OpenTemplate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.choose_template(cx);
    }

    /// Writes the template on top back to its file.
    pub(super) fn save_template_action(
        &mut self,
        _: &SaveTemplate,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let pane = self.templates[index].clone();
        pane.update(cx, |pane, cx| pane.trigger_completion(cx));
    }

    // --- actions ----------------------------------------------------------

    /// Opens the connection dialog.
    pub(super) fn new_connection_action(
        &mut self,
        _: &NewConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_dialog(window, cx);
    }

    /// Opens the settings dialog.
    pub(super) fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
    }

    /// Opens the abbreviation rules editor.
    pub(super) fn edit_abbreviations_action(
        &mut self,
        _: &EditAbbreviations,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_abbreviations(window, cx);
    }

    /// Opens the jdbgen import wizard.
    pub(super) fn import_jdbgen_action(
        &mut self,
        _: &ImportJdbgen,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_import(window, cx);
    }

    /// Opens the about box.
    pub(super) fn show_about_action(
        &mut self,
        _: &ShowAbout,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_about(window, cx);
    }

    /// Asks GitHub for the latest release.
    pub(super) fn check_updates_action(
        &mut self,
        _: &CheckUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.check_updates(window, cx);
    }

    /// Shows and hides the explorer sidebar.
    pub(super) fn toggle_explorer_action(
        &mut self,
        _: &ToggleExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        self.start_run(RunKind::Generate, cx);
    }

    /// Renders one pair into the Preview tab.
    pub(super) fn preview_action(
        &mut self,
        _: &Preview,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.start_run(RunKind::Preview, cx);
    }

    /// Renders every pair into memory.
    pub(super) fn dry_run_action(
        &mut self,
        _: &DryRun,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
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
        // Then the "save before closing?" question, which is the innermost
        // thing on screen whenever it is up: it was opened by a press on a
        // tab's close button, over whatever else was already there.
        if self.closing.is_some() {
            self.closing = None;
            self.focus_shell(window, cx);
            return;
        }
        if self.update.read(cx).is_open() {
            // Swallowed rather than propagated while an install runs: the key
            // must not reach anything else, but nothing may take the screen
            // from a swap either, so `Escape` simply does nothing until it is
            // over.
            if !self.update.read(cx).is_busy() {
                self.update.update(cx, |dialog, cx| dialog.close(cx));
                self.focus_shell(window, cx);
            }
            return;
        }
        if self.job.read(cx).is_open() {
            // Swallowed rather than propagated while a run is going: the key
            // must not reach anything else, and a run is stopped with Cancel —
            // which says what it does — rather than by a key that elsewhere
            // means "never mind".
            if !self.job.read(cx).is_busy() {
                self.job.update(cx, |job, cx| job.close(cx));
                self.focus_shell(window, cx);
            }
            return;
        }
        if self.about.read(cx).is_open() {
            self.about.update(cx, |dialog, cx| dialog.close(cx));
            self.focus_shell(window, cx);
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
            // a colour editor, two dropdowns and a delete confirmation of its
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
