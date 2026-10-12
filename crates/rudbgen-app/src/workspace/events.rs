//! Routes child events to workspace operations.
use super::*;

impl Workspace {
    pub(super) fn on_about_event(
        &mut self,
        dialog: &Entity<AboutDialog>,
        event: &AboutDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            AboutDialogEvent::Dismissed => {
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                self.focus_shell(window, cx);
            }
        }
    }

    pub(super) fn on_settings_event(
        &mut self,
        dialog: &Entity<SettingsDialog>,
        event: &SettingsDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The dialog has already replaced and persisted the settings
            // global by the time it emits this; the shell re-applies the
            // parts that touch the live window.
            SettingsDialogEvent::Applied => {
                self.apply_settings(window, cx);
                // The dialog closes itself after applying; without a refocus
                // the window focus dangles on its unrendered controls and
                // macOS disables every menu item validated through it.
                self.focus_shell(window, cx);
            }
            // The user is still in the dialog and nothing has been saved, so
            // only the palettes and the fonts follow — and the focus stays
            // where it is, since taking it back now would pull it out from
            // under whoever is typing.
            SettingsDialogEvent::Previewed => self.apply_preview(window, cx),
            // Closing dropped the preview, so re-applying now resolves back
            // to the settings on disk. That is the whole of the undo.
            SettingsDialogEvent::Dismissed => {
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                self.apply_preview(window, cx);
                self.focus_shell(window, cx);
            }
        }
    }

    pub(super) fn on_connection_dialog_event(
        &mut self,
        dialog: &Entity<ConnectionDialog>,
        event: &ConnectionDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The dialog has already written `connections.json`; the shell
            // re-reads it and opens the session, because the session is the
            // shell's to own — a dialog that held one could not be closed
            // without closing the database with it.
            ConnectionDialogEvent::Connect(profile) => {
                self.profiles = load_profiles();
                let profile = (**profile).clone();
                self.focus_shell(window, cx);
                self.connect_to(profile, cx);
            }
            ConnectionDialogEvent::Dismissed => {
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                // Saved, renamed or deleted while it was up, so the welcome
                // list and the selector both have to be re-read.
                self.profiles = load_profiles();
                self.focus_shell(window, cx);
            }
        }
    }

    pub(super) fn on_abbreviations_event(
        &mut self,
        dialog: &Entity<AbbreviationDialog>,
        event: &AbbreviationDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The dialog has already written `abbreviations.json`. What is
            // left is the copy the Generate tab holds — the panel is what a
            // run reads the rules from, and re-reading the file here would
            // be a second answer to a question already answered.
            AbbreviationDialogEvent::Saved(store) => {
                let store = (**store).clone();
                self.generate.update(cx, |pane, cx| {
                    pane.adopt_abbreviations(store, cx);
                });
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                self.focus_shell(window, cx);
            }
            AbbreviationDialogEvent::Dismissed => {
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                self.focus_shell(window, cx);
            }
        }
    }

    pub(super) fn on_import_event(
        &mut self,
        dialog: &Entity<ImportDialog>,
        event: &ImportDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            // The wizard has already written the four stores and the
            // keychain; the shell re-reads the connection list the welcome
            // screen draws, and applies jdbgen's language and theme when
            // the user ticked that box — settings are the shell's, never a
            // dialog's.
            ImportDialogEvent::Imported(hint) => {
                self.profiles = load_profiles();
                // The wizard wrote `abbreviations.json`; the Generate
                // tab holds the copy a run reads the rules from, and it
                // only re-reads the file when a connection opens.
                match rudbgen_core::AbbreviationStore::load() {
                    Ok(store) => self.generate.update(cx, |pane, cx| {
                        pane.adopt_abbreviations(store, cx);
                    }),
                    Err(error) => {
                        log::error!("could not re-read abbreviations.json: {error:#}");
                    }
                }
                if let Some(hint) = hint {
                    self.adopt_jdbgen_settings(hint, window, cx);
                }
                cx.notify();
            }
            ImportDialogEvent::Dismissed => {
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                self.profiles = load_profiles();
                self.focus_shell(window, cx);
            }
        }
    }

    pub(super) fn on_update_event(
        &mut self,
        dialog: &Entity<UpdateDialog>,
        event: &UpdateDialogEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            UpdateDialogEvent::Ignored { tag } => {
                // The dialog has already closed itself; writing the file is
                // the shell's job because the shell is what owns settings.
                update::remember_ignored(tag, cx);
                self.focus_shell(window, cx);
            }
            UpdateDialogEvent::Installed(_) => {
                // The new build is on disk and the restart is the
                // application's to perform: `cx.restart()` spawns a watcher
                // that waits for this pid to exit and starts rudbgen again
                // from the same path, which by then holds what was
                // installed. The dialog stays on screen — the process is
                // about to go, and closing it first would flash the window
                // back into view for a fraction of a second.
                //
                // The path has to be pointed at explicitly: `install`
                // renames the running image aside and writes the new build
                // under the old name, and on Linux `current_exe()` follows
                // the *inode* into the renamed-away copy, so a bare
                // `cx.restart()` would come back up on the build that was
                // just replaced. `restart_path()` is the path as it stood
                // before anything moved, taken by `init_process_identity`
                // at the top of `main`; where the platform never answered
                // one, leaving `set_restart_path` uncalled is gpui's own
                // default and behaves the same as before this build.
                if let Some(path) = rugpui_shell::restart_path() {
                    cx.set_restart_path(path);
                }
                cx.restart();
            }
            UpdateDialogEvent::Dismissed => {
                dialog.update(cx, |dialog, cx| dialog.close(cx));
                self.focus_shell(window, cx);
            }
        }
    }

    pub(super) fn on_explorer_event(
        &mut self,
        _explorer: Entity<Explorer>,
        event: &ExplorerEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            // Both of these need the session, which is the workspace's.
            ExplorerEvent::LoadSchemas => self.load_schemas(cx),
            ExplorerEvent::LoadTables(schema) => self.load_tables(schema.clone(), cx),
            // The status bar counts the ticks, and so does the list of
            // tables a live preview may be rendered against.
            ExplorerEvent::SelectionChanged => {
                self.refresh_templates(cx);
                cx.notify();
            }
            ExplorerEvent::Inspect { table, reveal } => {
                // Only the menu row asks for the panel to be shown. Moving
                // the tree's cursor must not: an arrow key that reopened a
                // panel the user had put away would be the shell arguing.
                if *reveal {
                    self.inspector_visible = true;
                    self.remember_layout(cx);
                }
                let table = table.clone();
                self.inspector.update(cx, |panel, cx| panel.show(table, cx));
                // A cache hit draws without asking anybody, so no Load
                // event follows to refresh the palette on the way back —
                // refresh it here, for the table now in hand.
                self.refresh_palette(cx);
                cx.notify();
            }
        }
    }

    pub(super) fn on_inspector_event(
        &mut self,
        _panel: Entity<Inspector>,
        event: &InspectorEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            InspectorEvent::Load(table) => self.load_table(table.clone(), cx),
        }
    }

    pub(super) fn on_generate_event(
        &mut self,
        _pane: &Entity<GeneratePane>,
        event: &GeneratePaneEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            GeneratePaneEvent::Changed => {
                // The palette lists the custom variables, so a variable
                // typed into the Generate tab has to reach it.
                self.refresh_palette(cx);
                cx.notify();
            }
            GeneratePaneEvent::EditTemplate(file) => {
                let file = generate_pane::resolve_template(file);
                self.open_template(file, cx);
            }
            GeneratePaneEvent::EditAbbreviations => {
                self.open_abbreviations(window, cx);
            }
        }
    }

    pub(super) fn on_preview_event(
        &mut self,
        _pane: Entity<PreviewPane>,
        event: &PreviewEvent,
        cx: &mut Context<Self>,
    ) {
        match event {
            PreviewEvent::Reselect { table, template } => {
                self.render_preview(*table, *template, cx);
            }
        }
    }

    pub(super) fn on_palette_event(
        &mut self,
        _panel: &Entity<VariablePalette>,
        event: &PaletteEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            PaletteEvent::Insert(text) => self.insert_into_template(text, window, cx),
        }
    }

    pub(super) fn on_job_event(
        &mut self,
        _job: &Entity<GenerationJob>,
        event: &JobEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            JobEvent::Closed => self.focus_shell(window, cx),
        }
    }
}
