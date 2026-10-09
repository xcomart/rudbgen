//! Templates.

use super::*;

impl Workspace {
    /// Where the buffer for `file` is, if it is open.
    pub(super) fn template_index(&self, file: &Path, cx: &App) -> Option<usize> {
        self.templates
            .iter()
            .position(|pane| pane.read(cx).path() == file)
    }

    /// The buffer of the tab on top, when the tab on top is a template.
    pub(super) fn active_template(&self, cx: &App) -> Option<usize> {
        match self.pane.active() {
            Some(PaneItem::Template { file, .. }) => self.template_index(file, cx),
            _ => None,
        }
    }

    /// Opens `file` in a tab, or brings the tab that already has it to the top.
    ///
    /// One tab per file, deliberately: two buffers over one path are two
    /// answers to what is in it, and the second save would silently undo the
    /// first.
    pub(super) fn open_template(&mut self, file: PathBuf, cx: &mut Context<Self>) {
        if let Some(index) = self.pane.template_of(&file) {
            self.pane.activate(index);
            if let Some(at) = self.template_index(&file, cx) {
                self.templates[at]
                    .clone()
                    .update(cx, |pane, cx| pane.request_focus(cx));
            }
            self.refresh_templates(cx);
            cx.notify();
            return;
        }

        let opened = cx.new(|cx| TemplatePane::open(file.clone(), cx));
        let events = cx.subscribe(&opened, |workspace, pane, event, cx| match event {
            TemplatePaneEvent::DirtyChanged => workspace.retitle_templates(cx),
            TemplatePaneEvent::Render { source, table } => {
                let file = pane.read(cx).path().to_path_buf();
                workspace.render_template_preview(file, source.clone(), *table, cx);
            }
        });
        let title = opened.read(cx).title();
        self.templates.push(opened);
        self.template_events.push(events);
        self.pane.push(PaneItem::Template {
            file,
            title,
            dirty: false,
        });
        self.refresh_templates(cx);
        // Nothing has been typed yet, so there is nothing to wait for: the tab
        // opens with its diagnostics and its preview already in it.
        if let Some(index) = self.active_template(cx) {
            self.templates[index].update(cx, |pane, cx| pane.refresh(cx));
        }
        cx.notify();
    }

    /// Asks the platform for a template file and opens what it hands back.
    ///
    /// The one way in that needs no connection (§4.3). Nothing waits on the
    /// prompt — on X11 that call is what gpui had to be patched around — so the
    /// click returns and the answer is picked up on a task of its own.
    pub(super) fn choose_template(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(gpui::PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(ts!("template.open_select")),
        });
        cx.spawn(async move |this, cx| {
            let chosen = match paths.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    log::warn!("the file picker could not be opened: {error:#}");
                    return;
                }
            };
            this.update(cx, |workspace, cx| {
                for path in chosen {
                    workspace.open_template(path, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Copies every buffer's dirty flag onto its tab.
    pub(super) fn retitle_templates(&mut self, cx: &mut Context<Self>) {
        let mut changed = false;
        for index in 0..self.pane.items().len() {
            let Some(PaneItem::Template { file, .. }) = self.pane.get(index) else {
                continue;
            };
            let file = file.clone();
            let Some(at) = self.template_index(&file, cx) else {
                continue;
            };
            let dirty = self.templates[at].read(cx).is_dirty(cx);
            let title = self.templates[at].read(cx).title();
            if let Some(PaneItem::Template {
                dirty: was,
                title: label,
                ..
            }) = self.pane.get_mut(index)
                && (*was != dirty || *label != title)
            {
                *was = dirty;
                *label = title;
                changed = true;
            }
        }
        if changed {
            cx.notify();
        }
    }

    /// The tables the live preview may be rendered against, in the order the
    /// header's dropdown lists them.
    ///
    /// The ticked tables, which is what the run itself would use; and when
    /// nothing is ticked, whatever the inspector last described, so that a
    /// template opened after a walk through the tree still has something to
    /// render against.
    pub(super) fn preview_tables(&self, cx: &App) -> Vec<(TableKey, SharedString)> {
        if !matches!(self.connection, ConnectionState::Open { .. }) {
            return Vec::new();
        }
        let mut tables: Vec<(TableKey, SharedString)> = self
            .explorer
            .read(cx)
            .selection(cx)
            .into_iter()
            .map(|key| {
                let label = SharedString::from(key.name.clone());
                (key, label)
            })
            .collect();
        if tables.is_empty()
            && let Some(table) = self.inspector.read(cx).table()
        {
            tables.push((
                TableKey {
                    catalog: table.catalog.clone(),
                    schema: table.schema.clone(),
                    name: table.name.clone(),
                },
                SharedString::from(table.name.clone()),
            ));
        }
        tables
    }

    /// Hands every open template the current table list and the current
    /// palette.
    pub(super) fn refresh_templates(&mut self, cx: &mut Context<Self>) {
        let choices: Vec<SharedString> = self
            .preview_tables(cx)
            .into_iter()
            .map(|(_, label)| label)
            .collect();
        for pane in self.templates.clone() {
            pane.update(cx, |pane, cx| pane.set_choices(choices.clone(), cx));
        }
        self.refresh_palette(cx);
    }

    /// Rebuilds the palette from the model and the profile.
    pub(super) fn refresh_palette(&mut self, cx: &mut Context<Self>) {
        let table = self.palette_table(cx);
        let vars = self.generate.read(cx).collect(cx).custom_vars;
        let items = palette::items(table.as_ref(), &vars);
        self.palette
            .update(cx, |panel, cx| panel.set_items(items.clone(), cx));
        for pane in self.templates.clone() {
            pane.update(cx, |pane, cx| pane.set_items(items.clone(), cx));
        }
    }

    /// The table the palette's examples come from.
    ///
    /// The one the tab on top is previewing, so that the examples and the
    /// preview agree; the inspector's otherwise, and nothing at all when
    /// neither has one.
    pub(super) fn palette_table(&self, cx: &App) -> Option<Table> {
        if let Some(index) = self.active_template(cx) {
            let choice = self.templates[index].read(cx).choice();
            if let Some((key, _)) = self.preview_tables(cx).get(choice)
                && let Some(table) = self.inspector.read(cx).cached(key)
            {
                return Some((*table).clone());
            }
        }
        self.inspector.read(cx).table().cloned()
    }

    /// Renders `source` against the chosen table and hands it to the tab.
    ///
    /// The table is read first when nothing has described it yet, exactly as a
    /// run does — the inspector's cache is what usually makes that free.
    pub(super) fn render_template_preview(
        &mut self,
        file: PathBuf,
        source: String,
        choice: usize,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.template_index(&file, cx) else {
            return;
        };
        let tables = self.preview_tables(cx);
        let Some((key, _)) = tables.get(choice).cloned() else {
            let note = if matches!(self.connection, ConnectionState::Open { .. }) {
                ts!("template.preview_no_table")
            } else {
                ts!("template.preview_no_connection")
            };
            self.refuse_preview(index, note, &source, cx);
            return;
        };

        if let Some(table) = self.inspector.read(cx).cached(&key) {
            // The palette's examples come from the same table, and it is the
            // table that has changed here — whether it was read for this
            // preview or was already in hand makes no difference to the rows.
            self.refresh_palette(cx);
            self.render_template_with(index, source, (*table).clone(), cx);
            return;
        }

        let Some((handle, driver, epoch)) = self.meta_context() else {
            let note = ts!("template.preview_no_connection");
            self.refuse_preview(index, note, &source, cx);
            return;
        };
        let reference = self
            .explorer
            .read(cx)
            .table_ref(&key, cx)
            .unwrap_or_else(|| rudbgen_meta::TableRef {
                catalog: key.catalog.clone(),
                schema: key.schema.clone(),
                name: key.name.clone(),
                ..rudbgen_meta::TableRef::default()
            });
        cx.spawn(async move |this, cx| {
            let read = cx
                .background_executor()
                .spawn(async move {
                    MetaReader::new(handle.session(), &driver)
                        .table(&reference)
                        .map_err(|error| error.to_string())
                })
                .await;
            this.update(cx, |workspace, cx| {
                if workspace.connection_epoch != epoch {
                    return;
                }
                match read {
                    Ok(table) => {
                        workspace
                            .inspector
                            .update(cx, |panel, _cx| panel.remember(key, table.clone()));
                        // The palette's examples come from the same table, and
                        // this is the read that put one in reach.
                        workspace.refresh_palette(cx);
                        workspace.render_template_with(index, source, table, cx);
                    }
                    Err(message) => {
                        let note = ts!(
                            "generate.read_failed",
                            table = key.qualified(),
                            reason = message
                        );
                        workspace.refuse_preview(index, note, &source, cx);
                    }
                }
            })
            .ok();
        })
        .detach();
    }

    /// Tells the tab why there is no preview.
    pub(super) fn refuse_preview(
        &mut self,
        index: usize,
        note: SharedString,
        source: &str,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.templates.get(index).cloned() else {
            return;
        };
        pane.update(cx, |pane, cx| {
            pane.deliver_preview(PreviewOutcome::Refused(note), source, cx);
        });
    }

    /// Renders one pair with a table that is already in hand.
    ///
    /// The plan is built here rather than taken from [`Workspace::plan`]: the
    /// template being edited need not be one of the ticked ones, its body comes
    /// from the buffer rather than from the file (`TemplateSpec::source`), and
    /// the output name is deliberately a literal — so that every warning the
    /// render reports is the *body*'s, and its span therefore an offset into
    /// the buffer the marks are drawn on. The name the run would really give
    /// the file is rendered separately, for the header alone.
    pub(super) fn render_template_with(
        &mut self,
        index: usize,
        source: String,
        table: Table,
        cx: &mut Context<Self>,
    ) {
        let Some(pane) = self.templates.get(index).cloned() else {
            return;
        };
        let file = pane.read(cx).path().to_path_buf();
        let profile = self.generate.read(cx).collect(cx);
        let stored = generate_pane::store_template_path(&file);
        let known = profile.templates.iter().find(|template| {
            generate_pane::resolve_template(&template.file) == file || template.file == stored
        });
        let label = file
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "preview".to_owned());
        let name = known.map_or_else(|| label.clone(), |template| template.name.clone());
        let out_template = known.map(|template| template.out_template.clone());
        let output_dir = self
            .generate
            .read(cx)
            .output_dir(cx)
            .unwrap_or_else(|| std::env::temp_dir().join("rudbgen-preview"));

        let mut plan = Plan::new(
            vec![table],
            vec![TemplateSpec::new(&name, &file, &label).with_source(source.clone())],
            output_dir,
        );
        plan.author = profile.author;
        plan.custom_vars = profile.custom_vars;
        let plan = plan.with_abbreviations(self.generate.read(cx).abbreviations());

        cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_executor()
                .spawn(async move {
                    // The real output name, for the header. Its own unknown
                    // fields are not reported: their spans point into the name
                    // template, and a mark on line 3 of the body would be a lie.
                    let named = out_template.and_then(|out| {
                        let ctx = plan.context();
                        let table = plan.tables.first()?;
                        rudbgen_template::Template::parse(&out)
                            .ok()?
                            .render(table, &ctx)
                            .ok()
                    });
                    let path = named.map(|name| plan.output_dir.join(name));
                    (rudbgen_gen::preview(&plan, 0, 0), path)
                })
                .await;
            this.update(cx, |workspace, cx| {
                let (preview, named) = rendered;
                let outcome = match preview {
                    Ok(preview) => PreviewOutcome::Rendered {
                        path: named.unwrap_or(preview.path),
                        content: preview.content,
                        warnings: preview.diagnostics,
                    },
                    Err(error) => PreviewOutcome::Refused(SharedString::from(error.to_string())),
                };
                let Some(pane) = workspace.templates.get(index).cloned() else {
                    return;
                };
                pane.update(cx, |pane, cx| pane.deliver_preview(outcome, &source, cx));
            })
            .ok();
        })
        .detach();
    }

    /// Writes the palette's entry at the caret of the tab on top.
    pub(super) fn insert_into_template(
        &mut self,
        text: &SharedString,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(index) = self.active_template(cx) else {
            return;
        };
        let pane = self.templates[index].clone();
        pane.update(cx, |pane, cx| pane.insert(text, window, cx));
    }

    // --- the tab strip ----------------------------------------------------

    /// Closes the tab at `index`, asking first when it holds unsaved edits.
    pub(super) fn request_close_tab(
        &mut self,
        index: usize,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match self.pane.get(index) {
            // The Generate tab is what the run is configured in and there is no
            // second one to fall back on, so it is the one tab a close cannot
            // take away.
            Some(PaneItem::Generate) | None => (),
            Some(PaneItem::Preview { .. }) => self.close_tab(index, window, cx),
            Some(PaneItem::Template { file, .. }) => {
                // The buffer is asked, not the flag on the tab: the flag is a
                // copy that travels on the effect cycle, and a save followed by
                // a close in the same update would still find it set.
                let file = file.clone();
                let dirty = self
                    .template_index(&file, cx)
                    .is_some_and(|at| self.templates[at].read(cx).is_dirty(cx));
                if dirty {
                    self.closing = Some(index);
                    cx.notify();
                } else {
                    self.close_tab(index, window, cx);
                }
            }
        }
    }

    /// Closes the tab at `index`, whatever is in it.
    pub(super) fn close_tab(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.closing = None;
        let Some(closed) = self.pane.close(index) else {
            return;
        };
        if let PaneItem::Template { file, .. } = closed
            && let Some(at) = self.template_index(&file, cx)
        {
            self.templates.remove(at);
            drop(self.template_events.remove(at));
        }
        // The tab that has gone may have been holding the keyboard, and a
        // focus handle left on an unrendered element takes every shortcut in
        // the window with it (Appendix A).
        self.focus_shell(window, cx);
        self.refresh_templates(cx);
        cx.notify();
    }

    /// Answers the "save before closing?" question.
    pub(super) fn answer_close(&mut self, save: bool, window: &mut Window, cx: &mut Context<Self>) {
        let Some(index) = self.closing else {
            return;
        };
        if save {
            let Some(PaneItem::Template { file, .. }) = self.pane.get(index) else {
                self.closing = None;
                return;
            };
            let file = file.clone();
            let Some(at) = self.template_index(&file, cx) else {
                self.closing = None;
                return;
            };
            let pane = self.templates[at].clone();
            if !pane.update(cx, |pane, cx| pane.save(cx)) {
                // The write failed and the tab is showing why; closing it now
                // would take the message away with the text it is about.
                self.closing = None;
                cx.notify();
                return;
            }
        }
        self.close_tab(index, window, cx);
    }

    /// Records what the connection attempt came back with.
    ///
    /// A failure is reported on the status bar rather than in a dialog: the
    /// user asked for a database, not for a box to dismiss, and the message
    /// has to stay readable while they open the connection dialog to fix
    /// whatever it names.
    pub(super) fn connected(
        &mut self,
        outcome: Result<Connected, ConnectError>,
        cx: &mut Context<Self>,
    ) {
        // Anything but `Connecting` means the attempt was abandoned — the user
        // asked for another connection, or disconnected — and its answer is no
        // longer about the state the window is in.
        let ConnectionState::Connecting {
            profile, driver, ..
        } = std::mem::replace(&mut self.connection, ConnectionState::Idle)
        else {
            if let Ok(session) = outcome
                && let Err(error) = session.close()
            {
                log::warn!("an abandoned session did not close: {error:#}");
            }
            return;
        };

        self.connection = match outcome {
            Ok(session) => {
                log::info!(
                    "connected to {} ({})",
                    profile.name,
                    session.product().unwrap_or_else(|| "unknown".into())
                );
                // A tunnel that breaks takes the session above it with it, and
                // is never repaired silently: a reconnection would hide what
                // was in flight when the socket went away. The watch fires once,
                // with the reason, and the window goes to the failed state
                // wearing it.
                if let Some(lease) = session.lease() {
                    let watch = lease.watch();
                    let id = profile.id;
                    cx.spawn(async move |this, cx| {
                        let Ok(reason) = watch.await else {
                            return;
                        };
                        this.update(cx, |workspace, cx| {
                            workspace.tunnel_died(id, reason, cx);
                        })
                        .ok();
                    })
                    .detach();
                }
                ConnectionState::Open {
                    profile,
                    driver,
                    session: Box::new(session),
                }
            }
            Err(error) => {
                log::warn!("could not connect to {}: {error}", profile.name);
                ConnectionState::Failed {
                    profile,
                    message: error.message().into(),
                }
            }
        };
        // After the state is in place, not before: the tree asks for its root
        // on the first frame it is drawn in, and the request runs through
        // [`Workspace::meta_context`], which reads that state.
        self.reset_panels(cx);
        cx.notify();
    }
}
