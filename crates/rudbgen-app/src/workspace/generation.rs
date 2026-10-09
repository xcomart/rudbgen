//! Generation.

use super::*;

impl Workspace {
    /// Starts a run of `kind`, reading whatever tables are not cached first.
    ///
    /// The panel is flushed before anything else: it is the only editor of the
    /// generation profile, and a run has to use what is on screen rather than
    /// what the debounce has got round to writing.
    pub(super) fn start_run(&mut self, kind: RunKind, cx: &mut Context<Self>) {
        self.generate.update(cx, |pane, cx| pane.flush(cx));

        // A preview and a dry run write nothing, so a missing output directory
        // is not in their way — the plan still needs somewhere to resolve names
        // against, and [`Workspace::plan`] stands a temporary directory in.
        let blocker = self
            .readiness(cx)
            .blocker
            .filter(|reason| kind == RunKind::Generate || *reason != Blocker::NoOutputDir);
        if blocker.is_some() {
            return;
        }

        let keys: Vec<TableKey> = self.explorer.read(cx).selection(cx).into_iter().collect();
        // Every ticked table, with the ones the inspector has already described
        // filled in. Walking the tree is what usually fills them, so a generate
        // after a read costs no round trip at all.
        let mut cached: Vec<Option<Table>> = Vec::with_capacity(keys.len());
        let mut references: Vec<rudbgen_meta::TableRef> = Vec::with_capacity(keys.len());
        for key in &keys {
            cached.push(
                self.inspector
                    .read(cx)
                    .cached(key)
                    .map(|table| (*table).clone()),
            );
            references.push(self.explorer.read(cx).table_ref(key, cx).unwrap_or(
                rudbgen_meta::TableRef {
                    catalog: key.catalog.clone(),
                    schema: key.schema.clone(),
                    name: key.name.clone(),
                    ..rudbgen_meta::TableRef::default()
                },
            ));
        }

        let missing = cached.iter().filter(|table| table.is_none()).count();
        if missing == 0 {
            let tables: Vec<Table> = cached.into_iter().flatten().collect();
            self.run_with(kind, tables, cx);
            return;
        }

        let Some((handle, driver, epoch)) = self.meta_context() else {
            return;
        };
        self.job
            .update(cx, |job, cx| job.begin_loading(missing, cx));
        cx.spawn(async move |this, cx| {
            let mut tables: Vec<Table> = Vec::with_capacity(cached.len());
            for (index, slot) in cached.into_iter().enumerate() {
                if let Some(table) = slot {
                    tables.push(table);
                    continue;
                }
                let reference = references[index].clone();
                let key = keys[index].clone();
                let handle = handle.clone();
                let driver = driver.clone();
                let read = cx
                    .background_executor()
                    .spawn(async move {
                        MetaReader::new(handle.session(), &driver)
                            .table(&reference)
                            .map_err(|error| error.to_string())
                    })
                    .await;
                match read {
                    Ok(table) => {
                        let name = table.name.clone();
                        this.update(cx, |workspace, cx| {
                            workspace
                                .inspector
                                .update(cx, |panel, _cx| panel.remember(key, table.clone()));
                            workspace.job.update(cx, |job, cx| job.loaded(&name, cx));
                        })
                        .ok();
                        tables.push(table);
                    }
                    Err(message) => {
                        let table = keys[index].qualified();
                        this.update(cx, |workspace, cx| {
                            workspace.job.update(cx, |job, cx| {
                                job.refuse(
                                    ts!("generate.read_failed", table = table, reason = message),
                                    cx,
                                );
                            });
                        })
                        .ok();
                        return;
                    }
                }
            }
            this.update(cx, |workspace, cx| {
                // The session went away while the tables were being read; the
                // answers describe a database the window has already left.
                if workspace.connection_epoch != epoch {
                    workspace.job.update(cx, |job, cx| job.finish_loading(cx));
                    return;
                }
                workspace.run_with(kind, tables, cx);
            })
            .ok();
        })
        .detach();
    }

    /// Runs `kind` over tables that are all in hand.
    pub(super) fn run_with(&mut self, kind: RunKind, tables: Vec<Table>, cx: &mut Context<Self>) {
        self.run_tables = tables;
        let plan = match self.plan(cx) {
            Ok(plan) => plan,
            Err(message) => {
                self.job.update(cx, |job, cx| job.refuse(message, cx));
                return;
            }
        };

        match kind {
            RunKind::Generate => {
                let policy = app_settings::current(cx).overwrite_policy;
                self.job.update(cx, |job, cx| job.start(plan, policy, cx));
            }
            RunKind::DryRun => {
                self.job.update(cx, |job, cx| job.finish_loading(cx));
                self.dry_run(plan, cx);
            }
            RunKind::Preview => {
                self.job.update(cx, |job, cx| job.finish_loading(cx));
                let (table, template) = self.preview.read(cx).choice();
                self.render_preview(table, template, cx);
            }
        }
        cx.notify();
    }

    /// The plan the panel and the ticks describe.
    ///
    /// Template paths are resolved here rather than in the profile: §5 stores
    /// them relative to the configuration directory, and the generator opens
    /// files rather than resolving them.
    pub(super) fn plan(&self, cx: &App) -> Result<Plan, SharedString> {
        let pane = self.generate.read(cx);
        let mut profile = pane.collect(cx);
        for template in &mut profile.templates {
            template.file = generate_pane::resolve_template(&template.file);
        }
        // A preview and a dry run never write, so a directory nobody chose is
        // only ever used to resolve a name against — and the temporary
        // directory is the one place a name can be resolved against harmlessly.
        profile.output_dir = pane
            .output_dir(cx)
            .or_else(|| Some(std::env::temp_dir().join("rudbgen-preview")));
        let plan = Plan::from_profile(&profile, self.run_tables.clone())
            .map_err(|error| SharedString::from(error.to_string()))?;
        Ok(plan.with_abbreviations(pane.abbreviations()))
    }

    /// Renders every pair into memory and shows the file list.
    pub(super) fn dry_run(&mut self, plan: Plan, cx: &mut Context<Self>) {
        self.open_preview_tab(cx);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    let cancel = rudbgen_gen::CancelToken::new();
                    rudbgen_gen::dry_run(&plan, &cancel, &|_| {})
                })
                .await;
            this.update(cx, |workspace, cx| {
                let files: Vec<PreviewFile> = outcome
                    .files
                    .iter()
                    .map(|file| PreviewFile {
                        path: file.path.clone(),
                        content: SharedString::from(file.content.clone()),
                        exists: file.exists,
                    })
                    .collect();
                let mut notes: Vec<SharedString> = outcome
                    .diagnostics
                    .iter()
                    .map(|diagnostic| {
                        SharedString::from(format!(
                            "{} × {}: {}",
                            diagnostic.table, diagnostic.template, diagnostic.warning.message
                        ))
                    })
                    .collect();
                notes.extend(outcome.failed.iter().map(|failure| {
                    SharedString::from(format!("{}: {}", failure.template, failure.message))
                }));
                workspace.preview.update(cx, |pane, cx| {
                    pane.show_dry_run(files, notes, cx);
                });
                workspace.retitle_preview(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Renders one pair into the Preview tab.
    pub(super) fn render_preview(&mut self, table: usize, template: usize, cx: &mut Context<Self>) {
        if self.run_tables.is_empty() {
            return;
        }
        self.open_preview_tab(cx);
        let plan = match self.plan(cx) {
            Ok(plan) => plan,
            Err(message) => {
                self.preview
                    .update(cx, |pane, cx| pane.show_error(message, cx));
                return;
            }
        };
        cx.spawn(async move |this, cx| {
            let rendered = cx
                .background_executor()
                .spawn(async move { rudbgen_gen::preview(&plan, table, template) })
                .await;
            this.update(cx, |workspace, cx| {
                match rendered {
                    Ok(preview) => {
                        let notes = preview
                            .diagnostics
                            .iter()
                            .map(|warning| {
                                SharedString::from(format!(
                                    "{}: {}",
                                    ts!("generate.at_line", line = warning.span.line + 1),
                                    warning.message
                                ))
                            })
                            .collect();
                        workspace.preview.update(cx, |pane, cx| {
                            pane.show_preview(
                                PreviewFile {
                                    path: preview.path,
                                    content: SharedString::from(preview.content),
                                    exists: false,
                                },
                                notes,
                                cx,
                            );
                        });
                    }
                    Err(error) => {
                        let message = SharedString::from(error.to_string());
                        workspace
                            .preview
                            .update(cx, |pane, cx| pane.show_error(message, cx));
                    }
                }
                workspace.retitle_preview(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Opens the Preview tab, or brings the one that is open to the front.
    ///
    /// One preview tab, deliberately: it is a *view* of the run being
    /// configured, not a document, and a strip that grew a tab per press would
    /// fill with names of files nobody asked to keep.
    pub(super) fn open_preview_tab(&mut self, cx: &mut Context<Self>) {
        let tables: Vec<SharedString> = self
            .run_tables
            .iter()
            .map(|table| SharedString::from(table.name.clone()))
            .collect();
        let templates = self.generate.read(cx).selected_names(cx);
        self.preview
            .update(cx, |pane, cx| pane.set_choices(tables, templates, cx));

        let title = self.preview.read(cx).title();
        match self.preview_tab() {
            Some(index) => {
                self.pane.activate(index);
            }
            None => {
                self.pane.push(PaneItem::Preview { title });
            }
        }
        cx.notify();
    }

    /// Where the preview tab is on the strip, if it is open.
    pub(super) fn preview_tab(&self) -> Option<usize> {
        self.pane.preview()
    }

    /// Relabels the preview tab with the file it now shows.
    pub(super) fn retitle_preview(&mut self, cx: &mut Context<Self>) {
        let title = self.preview.read(cx).title();
        if let Some(index) = self.preview_tab() {
            self.pane.close(index);
            self.pane.push(PaneItem::Preview { title });
        }
        cx.notify();
    }

    // --- the template tabs ------------------------------------------------

    /// Whether any template is open, connection or no connection.
    ///
    /// What decides between the welcome screen and the work area when nothing
    /// is connected: a template can be edited without a database (§4.3), and
    /// the tab it is in has to be somewhere.
    pub(super) fn has_templates(&self) -> bool {
        !self.templates.is_empty()
    }
}
