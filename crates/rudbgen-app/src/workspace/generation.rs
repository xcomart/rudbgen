//! Generation.

use super::*;

impl Workspace {
    /// Starts a run of `kind`, reading whatever tables are not cached first.
    ///
    /// The panel is flushed before anything else: it is the only editor of the
    /// generation profile, and a run has to use what is on screen rather than
    /// what the debounce has got round to writing.
    pub(super) fn start_run(&mut self, kind: RunKind, cx: &mut Context<Self>) {
        // The buttons are disabled while a run is going, but the shortcut is
        // not a button: a second press would start a second run over the first
        // one's dialog, and Cancel would then reach only the second.
        if self.job.read(cx).is_busy() {
            return;
        }
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

        // A run reads its templates from their files, and the live preview
        // beside an edited one renders its buffer: running over a ticked
        // template with unsaved edits would write something other than what
        // the user has just been looking at. So the question comes first.
        if !self.unsaved_run_templates(cx).is_empty() {
            self.unsaved_run = Some(kind);
            self.focus_prompt = true;
            cx.notify();
            return;
        }
        self.read_and_run(kind, cx);
    }

    /// The open template tabs a run would read the saved file of instead of
    /// what is in them: ticked in the Generate tab, and with unsaved edits.
    pub(super) fn unsaved_run_templates(&self, cx: &App) -> Vec<usize> {
        let ticked: Vec<PathBuf> = self
            .generate
            .read(cx)
            .collect(cx)
            .templates
            .iter()
            .filter(|template| template.selected)
            .map(|template| generate_pane::resolve_template(&template.file))
            .collect();
        self.templates
            .iter()
            .enumerate()
            .filter(|(_, pane)| {
                let pane = pane.read(cx);
                pane.is_dirty(cx) && ticked.iter().any(|file| file == pane.path())
            })
            .map(|(index, _)| index)
            .collect()
    }

    /// Answers "save the templates first?": `Some(true)` saves them and runs,
    /// `Some(false)` runs from the files as they are, `None` runs nothing.
    ///
    /// A save that fails stops the run there, with the tab showing why — the
    /// same rule a failed save before closing a tab follows.
    pub(super) fn answer_unsaved_run(
        &mut self,
        save: Option<bool>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let Some(kind) = self.unsaved_run.take() else {
            return;
        };
        self.return_focus(window, cx);
        cx.notify();
        let Some(save) = save else {
            return;
        };
        if save {
            for index in self.unsaved_run_templates(cx) {
                let pane = self.templates[index].clone();
                if !pane.update(cx, |pane, cx| pane.save(cx)) {
                    self.retitle_templates(cx);
                    return;
                }
            }
            self.retitle_templates(cx);
        }
        self.read_and_run(kind, cx);
    }

    /// "Save the templates first?", with the names it is asking about.
    pub(super) fn render_unsaved_run_prompt(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let names: Vec<String> = self
            .unsaved_run_templates(cx)
            .into_iter()
            .map(|index| self.templates[index].read(cx).title().to_string())
            .collect();
        let (save, plain, cancel, dismissed, default) = (
            cx.entity(),
            cx.entity(),
            cx.entity(),
            cx.entity(),
            cx.entity(),
        );
        rugpui::modal(
            "unsaved-run",
            ts!("unsaved.title"),
            px(440.),
            prompt::body(&self.prompt_focus, move |window, cx| {
                default.update(cx, |workspace, cx| {
                    workspace.answer_unsaved_run(Some(true), window, cx);
                });
            })
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(ts!("unsaved.question", names = names.join(", "))),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_wrap()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("unsaved-cancel", ts!("common.cancel"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(move |_, window, cx| {
                                cancel.update(cx, |workspace, cx| {
                                    workspace.answer_unsaved_run(None, window, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("unsaved-plain", ts!("unsaved.run_without_saving"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(move |_, window, cx| {
                                plain.update(cx, |workspace, cx| {
                                    workspace.answer_unsaved_run(Some(false), window, cx);
                                });
                            }),
                    )
                    .child(
                        Button::new("unsaved-save", ts!("unsaved.save_and_run"))
                            .variant(ButtonVariant::Primary)
                            .on_click(move |_, window, cx| {
                                save.update(cx, |workspace, cx| {
                                    workspace.answer_unsaved_run(Some(true), window, cx);
                                });
                            }),
                    ),
            ),
            move |window, cx| {
                dismissed.update(cx, |workspace, cx| {
                    workspace.answer_unsaved_run(None, window, cx);
                });
            },
        )
        .into_any_element()
    }

    /// The run itself, once nothing stands in its way: reads the tables the
    /// inspector has not described yet, then hands them to [`Self::run_with`].
    pub(super) fn read_and_run(&mut self, kind: RunKind, cx: &mut Context<Self>) {
        if self.job.read(cx).is_busy() {
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
        let ticket = self
            .job
            .update(cx, |job, cx| job.begin_loading(missing, cx));
        cx.spawn(async move |this, cx| {
            let mut tables: Vec<Table> = Vec::with_capacity(cached.len());
            for (index, slot) in cached.into_iter().enumerate() {
                if let Some(table) = slot {
                    tables.push(table);
                    continue;
                }
                // Checked before every read, because a read that has started
                // cannot be stopped: Cancel saves the ones not yet asked for.
                let wanted = this
                    .update(cx, |workspace, cx| workspace.load_wanted(ticket, epoch, cx))
                    .unwrap_or(false);
                if !wanted {
                    return;
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
                        let wanted = this
                            .update(cx, |workspace, cx| {
                                if !workspace.load_wanted(ticket, epoch, cx) {
                                    return false;
                                }
                                workspace
                                    .inspector
                                    .update(cx, |panel, _cx| panel.remember(key, table.clone()));
                                workspace.job.update(cx, |job, cx| job.loaded(&name, cx));
                                true
                            })
                            .unwrap_or(false);
                        if !wanted {
                            return;
                        }
                        tables.push(table);
                    }
                    Err(message) => {
                        let table = keys[index].qualified();
                        this.update(cx, |workspace, cx| {
                            if !workspace.load_wanted(ticket, epoch, cx) {
                                return;
                            }
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
                if workspace.load_wanted(ticket, epoch, cx) {
                    workspace.run_with(kind, tables, cx);
                }
            })
            .ok();
        })
        .detach();
    }

    /// Whether the tables the reading phase `ticket` is fetching are still
    /// wanted.
    ///
    /// They are not once the dialog's Cancel took the phase down or a later
    /// phase replaced it, and then there is nothing to do. Nor are they once
    /// the session went away while they were being read — the answers describe
    /// a database the window has already left — and then the dialog is still
    /// up, so it is taken down here.
    fn load_wanted(&mut self, ticket: u64, epoch: u64, cx: &mut Context<Self>) -> bool {
        if !self.job.read(cx).is_loading(ticket) {
            return false;
        }
        if self.connection_epoch != epoch {
            self.job.update(cx, |job, cx| job.finish_loading(cx));
            return false;
        }
        true
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
                // The Generate tab's choice for this one run, or the default.
                let policy = self.generate.update(cx, |pane, cx| pane.take_overwrite(cx));
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
    ///
    /// The tab says it is rendering meanwhile, with a Cancel that stops the
    /// render at the next pair — a whole schema's worth takes long enough to
    /// look like nothing happened.
    pub(super) fn dry_run(&mut self, plan: Plan, cx: &mut Context<Self>) {
        self.open_preview_tab(cx);
        let output_dir = self.generate.read(cx).output_dir(cx);
        let token = rudbgen_gen::CancelToken::new();
        self.dry_run_cancel = Some(token.handle());
        self.preview.update(cx, |pane, cx| pane.begin_dry_run(cx));
        self.retitle_preview(cx);
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { rudbgen_gen::dry_run(&plan, &token, &|_| {}) })
                .await;
            this.update(cx, |workspace, cx| {
                workspace.dry_run_cancel = None;
                // Canceled, and so not the answer to anything any more.
                if !workspace.preview.read(cx).is_rendering() {
                    return;
                }
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
                    pane.show_dry_run(files, notes, output_dir, cx);
                });
                workspace.retitle_preview(cx);
            })
            .ok();
        })
        .detach();
    }

    /// The dry run tab's Cancel: stops the render and takes the tab's
    /// "rendering" down.
    pub(super) fn cancel_dry_run(&mut self, cx: &mut Context<Self>) {
        if let Some(handle) = self.dry_run_cancel.take() {
            handle.cancel();
        }
        self.preview.update(cx, |pane, cx| pane.end_dry_run(cx));
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
    ///
    /// In place: the tab keeps the gap it was dragged to and the strip keeps
    /// the tab that is on top, where closing and pushing it again would move
    /// it to the end and bring it to the front behind the user's back — in
    /// the middle of a drag, if the render came back during one.
    pub(super) fn retitle_preview(&mut self, cx: &mut Context<Self>) {
        let title = self.preview.read(cx).title();
        if let Some(index) = self.preview_tab()
            && let Some(PaneItem::Preview { title: shown }) = self.pane.get_mut(index)
        {
            *shown = title;
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
