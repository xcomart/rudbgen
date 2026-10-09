//! Database.

use super::*;

impl Workspace {
    /// Empties both panels and retires whatever fetch was in flight.
    ///
    /// Called by every connect and every disconnect. The ticks go with the
    /// rest: a tick names a table of *that* database, and carrying one across
    /// to another server would be a set of names that happen to match.
    pub(super) fn reset_panels(&mut self, cx: &mut Context<Self>) {
        self.connection_epoch = self.connection_epoch.wrapping_add(1);
        self.explorer.update(cx, |explorer, cx| explorer.reset(cx));
        self.inspector.update(cx, |panel, cx| panel.reset(cx));
        self.run_tables.clear();
        self.preview.update(cx, |pane, cx| pane.reset(cx));
        // The tab strip goes back to the permanent tab and the templates: a
        // preview belongs to the database it was rendered against, and keeping
        // it open over a different one would be a file name from a schema that
        // is no longer there — where a template is a *file*, belongs to no
        // connection, and is never closed by one changing (§4.2).
        let templates: Vec<PaneItem> = self
            .pane
            .items()
            .iter()
            .filter(|item| matches!(item, PaneItem::Template { .. }))
            .cloned()
            .collect();
        self.pane = Pane::new();
        self.pane.push(PaneItem::Generate);
        for template in templates {
            self.pane.push(template);
        }
        self.pane.activate(0);
        // The panel edits whatever connection is now open — and nothing at all
        // while none is, which is what keeps a debounced save from writing into
        // a profile the window has already left.
        match self.connection.profile() {
            Some(profile) => {
                let id = profile.id;
                let generation = profile.generation.clone();
                self.generate
                    .update(cx, |pane, cx| pane.load(id, &generation, cx));
            }
            None => self.generate.update(cx, |pane, cx| pane.reset(cx)),
        }
        // The tables a live preview may be rendered against are this
        // connection's, and so is everything the palette shows an example of.
        self.refresh_templates(cx);
        for pane in self.templates.clone() {
            pane.update(cx, |pane, cx| pane.refresh(cx));
        }
    }

    /// The session, the driver definition and the epoch a metadata read needs.
    ///
    /// `None` while nothing is open, which is what makes every call site below
    /// a no-op rather than a panic when the session went away between the click
    /// and the frame.
    pub(super) fn meta_context(&self) -> Option<(SessionHandle, DriverDef, u64)> {
        let ConnectionState::Open {
            driver, session, ..
        } = &self.connection
        else {
            return None;
        };
        Some((session.handle(), (**driver).clone(), self.connection_epoch))
    }

    /// Reads the catalogs and schemas of the open session.
    ///
    /// On a background task, like every other call into `rudbgen-meta`: the
    /// reader blocks on a round trip through the bridge, and the UI thread
    /// never waits (architecture document, §6).
    pub(super) fn load_schemas(&mut self, cx: &mut Context<Self>) {
        let Some((handle, driver, epoch)) = self.meta_context() else {
            return;
        };
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    MetaReader::new(handle.session(), &driver)
                        .schemas()
                        .map_err(|error| SharedString::from(error.to_string()))
                })
                .await;
            this.update(cx, |workspace, cx| {
                if workspace.connection_epoch != epoch {
                    return;
                }
                workspace
                    .explorer
                    .update(cx, |explorer, cx| explorer.deliver_schemas(outcome, cx));
            })
            .ok();
        })
        .detach();
    }

    /// Reads the table list of one schema.
    ///
    /// Views included whatever the panel's toggle says: the explorer caches the
    /// wider answer and filters it, so flipping the toggle costs no round trip.
    pub(super) fn load_tables(&mut self, key: SchemaKey, cx: &mut Context<Self>) {
        let Some((handle, driver, epoch)) = self.meta_context() else {
            return;
        };
        let schema = key.schema();
        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    MetaReader::new(handle.session(), &driver)
                        .tables(&schema, true)
                        .map_err(|error| SharedString::from(error.to_string()))
                })
                .await;
            this.update(cx, |workspace, cx| {
                if workspace.connection_epoch != epoch {
                    return;
                }
                workspace
                    .explorer
                    .update(cx, |explorer, cx| explorer.deliver_tables(key, outcome, cx));
            })
            .ok();
        })
        .detach();
    }

    /// Reads one whole table for the inspector.
    pub(super) fn load_table(&mut self, key: TableKey, cx: &mut Context<Self>) {
        let Some((handle, driver, epoch)) = self.meta_context() else {
            return;
        };
        // The row the table list produced, not one built from the key: the
        // reader copies the kind, the comment and the position off what it is
        // handed rather than reading them again. A key nothing has listed —
        // which nothing on screen can produce — still gets described, with
        // those three left as the defaults.
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
            let outcome = cx
                .background_executor()
                .spawn(async move {
                    MetaReader::new(handle.session(), &driver)
                        .table(&reference)
                        .map_err(|error| SharedString::from(error.to_string()))
                })
                .await;
            this.update(cx, |workspace, cx| {
                if workspace.connection_epoch != epoch {
                    return;
                }
                workspace
                    .inspector
                    .update(cx, |panel, cx| panel.deliver(key, outcome, cx));
                // The palette's examples come from whatever table is in hand,
                // and one has just arrived.
                workspace.refresh_palette(cx);
            })
            .ok();
        })
        .detach();
    }

    // --- the run ----------------------------------------------------------

    /// §4.2's arithmetic, and the first thing standing in the run's way.
    pub(super) fn readiness(&self, cx: &App) -> generate_pane::Readiness {
        let pane = self.generate.read(cx);
        let output = pane.output_dir(cx);
        generate_pane::readiness(
            matches!(self.connection, ConnectionState::Open { .. }),
            self.explorer.read(cx).selected_count(cx),
            pane.selected_templates(),
            output.as_deref(),
        )
    }

    /// The tunnel under the open session ended; the session goes with it.
    ///
    /// Keyed on the profile's id rather than on its name: by the time this
    /// arrives the user may have connected to something else, and only the
    /// session that was running over *this* tunnel is the one to take down.
    pub(super) fn tunnel_died(&mut self, id: uuid::Uuid, reason: String, cx: &mut Context<Self>) {
        let ConnectionState::Open { profile, .. } = &self.connection else {
            return;
        };
        if profile.id != id {
            return;
        }
        log::warn!("the tunnel under {} ended: {reason}", profile.name);
        let ConnectionState::Open {
            profile, session, ..
        } = std::mem::replace(&mut self.connection, ConnectionState::Idle)
        else {
            return;
        };
        // The session is gone whatever the close says; the tunnel it ran over
        // is already down.
        drop(session);
        self.connection = ConnectionState::Failed {
            profile,
            message: ts!("connect.tunnel_ended", reason = reason),
        };
        self.reset_panels(cx);
        cx.notify();
    }

    /// Closes the session and goes back to the welcome screen.
    pub(super) fn disconnect(&mut self, cx: &mut Context<Self>) {
        self.close_session();
        self.connection_list_open = false;
        self.reset_panels(cx);
        cx.notify();
    }

    /// Closes whatever session is open, without touching anything on screen.
    ///
    /// Shared by [`Workspace::disconnect`], by every reconnection, and by the
    /// quit observer, which is why it takes no context: at quit there is no
    /// frame left to ask for.
    pub(super) fn close_session(&mut self) {
        if let ConnectionState::Open {
            profile, session, ..
        } = std::mem::replace(&mut self.connection, ConnectionState::Idle)
        {
            log::info!("closing the session on {}", profile.name);
            if let Err(error) = session.close() {
                log::warn!("the session did not close cleanly: {error:#}");
            }
        }
    }

    /// A handle a background task can carry, while a session is open.
    ///
    /// Nothing calls it yet — the explorer and the metadata reader are what it
    /// is for — and it is the one thing every one of those call sites needs, so
    /// it arrives with the session rather than after it.
    #[allow(dead_code)]
    pub(super) fn session_handle(&self) -> Option<SessionHandle> {
        self.connection.session().map(Connected::handle)
    }

    /// Opens the settings dialog, closing whatever else was showing.
    pub(super) fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        self.settings.update(cx, |dialog, cx| dialog.open(cx));
        cx.notify();
    }
}
