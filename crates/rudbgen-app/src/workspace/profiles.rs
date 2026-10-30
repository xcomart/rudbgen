//! The saved connections, as the main window handles them: the welcome list's
//! menu, the password question, and what a failed attempt offers to do next.
//!
//! The connection dialog is where a profile is *edited*; everything here acts
//! on a profile as it is saved, from outside the dialog — opening it, asking
//! for the secret it does not keep, copying it, forgetting it.

use super::*;

impl Workspace {
    // --- opening ----------------------------------------------------------

    /// The welcome row and the title bar's list: opens the profile `id`.
    ///
    /// Not again, though, while it is already open or opening: a second
    /// press on the same row would start the handshake over, and reselecting
    /// the open connection in the title bar used to reconnect and drop every
    /// tick in the explorer.
    pub(super) fn open_profile(
        &mut self,
        id: uuid::Uuid,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let busy = matches!(
            self.connection,
            ConnectionState::Open { .. } | ConnectionState::Connecting { .. }
        );
        if busy && self.connection.profile().is_some_and(|open| open.id == id) {
            return;
        }
        let Some(profile) = self.profiles.get(id).cloned() else {
            return;
        };
        self.connect_to(profile, None, window, cx);
    }

    /// The failure banner's *Retry*: the same profile, as it is saved now.
    pub(super) fn retry_connection(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.connection.profile().map(|profile| profile.id) else {
            return;
        };
        self.profiles = load_profiles();
        self.open_profile(id, window, cx);
    }

    /// The title bar's *Cancel connecting* and the welcome row's Cancel.
    ///
    /// The attempt is abandoned rather than stopped — the handshake is on a
    /// thread of its own and cannot be interrupted half way — and a session
    /// that opens after all is closed again by its own `Drop`, the way it is
    /// when another connection is picked over it.
    pub(super) fn cancel_connecting(&mut self, cx: &mut Context<Self>) {
        if matches!(self.connection, ConnectionState::Connecting { .. }) {
            self.connection = ConnectionState::Idle;
            self.connection_list_open = false;
            cx.notify();
        }
    }

    /// The failure banner's *Trust this key*: writes the bastion's key to
    /// `known_hosts`.
    ///
    /// Not followed by a retry, for the reason the dialog's button is not: the
    /// press is the user saying the fingerprint matches what they were told,
    /// and *Retry* is them saying they want to go on.
    pub(super) fn trust_failed_host_key(&mut self, cx: &mut Context<Self>) {
        let ConnectionState::Failed { host_key, hint, .. } = &mut self.connection else {
            return;
        };
        let Some(key) = host_key.take() else {
            return;
        };
        *hint = Some(match key.trust() {
            Ok(()) => ts!("connect.host_key_trusted"),
            Err(error) => ts!("connect.save_failed", error = format!("{error:#}")),
        });
        cx.notify();
    }

    // --- the password -----------------------------------------------------

    /// Puts the password question up for `profile`.
    ///
    /// Asked before connecting when the profile keeps no password, and after
    /// a database refused the one it had — `reason` says so then. The
    /// session that is open, if any, stays open until the question is
    /// answered: Cancel must not have cost it.
    pub(super) fn ask_password(
        &mut self,
        profile: ConnectionProfile,
        driver: DriverDef,
        reason: Option<SharedString>,
        cx: &mut Context<Self>,
    ) {
        let weak = cx.weak_entity();
        let input = cx.new(|cx| {
            rugpui::TextInput::new(cx)
                .masked(true)
                .on_submit(move |_, window, cx| {
                    let weak = weak.clone();
                    // Deferred: the field is still leased while its own
                    // submit runs, and the answer reads it back.
                    window.defer(cx, move |window, cx| {
                        weak.update(cx, |workspace, cx| workspace.submit_password(window, cx))
                            .ok();
                    });
                })
        });
        self.password_prompt = Some(PasswordPrompt {
            remember: profile.save_secret,
            profile: Box::new(profile),
            driver: Box::new(driver),
            input,
            focus: true,
            reason,
        });
        self.connection_list_open = false;
        cx.notify();
    }

    /// The question's Connect: opens the profile with what was typed.
    ///
    /// The *Remember password* answer is the profile's from now on and is
    /// saved straight away, being a preference rather than a secret; the
    /// password itself goes into the keychain only once the database has
    /// accepted it (see [`Workspace::connected`]), so a typo is never kept.
    pub(super) fn submit_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(prompt) = self.password_prompt.take() else {
            return;
        };
        let password = prompt.input.read(cx).content().to_owned();
        let mut profile = *prompt.profile;
        let changed = profile.save_secret != prompt.remember;
        profile.save_secret = prompt.remember;
        // Only for a profile that is saved: one that is not has no file entry
        // to carry the answer, and rewriting the file would change nothing.
        if changed && self.profiles.get(profile.id).is_some() {
            let id = profile.id;
            let remember = prompt.remember;
            let written = generate_pane::read_modify_write(
                ConnectionStore::load,
                |store| {
                    if let Some(mut saved) = store.get(id).cloned() {
                        saved.save_secret = remember;
                        store.upsert(saved);
                    }
                },
                ConnectionStore::save,
            );
            match written {
                Ok(store) => self.profiles = store,
                Err(error) => log::error!("connections.json was not changed: {error:#}"),
            }
        }
        let keep = prompt.remember.then(|| password.clone());
        // The tunnel's secret is left for the attempt to read, off this
        // thread: the question was only ever about the database's.
        let credentials = Credentials::typed(Some(password), None);
        self.focus_shell(window, cx);
        self.start_connect(profile, *prompt.driver, Some(credentials), keep, cx);
    }

    /// Takes the password question down; nothing is opened.
    pub(super) fn cancel_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.password_prompt.take().is_some() {
            self.return_focus(window, cx);
            cx.notify();
        }
    }

    /// The password question.
    pub(super) fn render_password_prompt(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let prompt = self.password_prompt.as_ref()?;
        let theme = theme(cx);
        let (submit, cancel, dismissed, toggle) =
            (cx.entity(), cx.entity(), cx.entity(), cx.entity());
        let name = label_of(&prompt.profile);
        Some(
            rugpui::modal(
                "password-prompt",
                ts!("password.title"),
                px(400.),
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.text)
                            .child(ts!("password.question", name = name)),
                    )
                    .children(prompt.reason.clone().map(|reason| {
                        div()
                            .text_size(px(11.))
                            .text_color(theme.danger)
                            .child(reason)
                    }))
                    .child(rugpui::form_row(
                        ts!("connect.password"),
                        prompt.input.clone(),
                    ))
                    .child(rugpui::form_row(
                        SharedString::default(),
                        rugpui::Checkbox::new(
                            "password-remember",
                            ts!("connect.remember_password"),
                        )
                        .checked(prompt.remember)
                        .on_toggle(move |checked, _window, cx| {
                            toggle.update(cx, |workspace, cx| {
                                if let Some(prompt) = workspace.password_prompt.as_mut() {
                                    prompt.remember = checked;
                                }
                                cx.notify();
                            });
                        }),
                    ))
                    .child(
                        div()
                            .flex()
                            .flex_row()
                            .justify_end()
                            .gap(px(8.))
                            .child(
                                Button::new("password-cancel", ts!("common.cancel"))
                                    .variant(ButtonVariant::Secondary)
                                    .on_click(move |_, window, cx| {
                                        cancel.update(cx, |workspace, cx| {
                                            workspace.cancel_password(window, cx);
                                        });
                                    }),
                            )
                            .child(
                                Button::new("password-connect", ts!("connect.connect"))
                                    .variant(ButtonVariant::Primary)
                                    .on_click(move |_, window, cx| {
                                        submit.update(cx, |workspace, cx| {
                                            workspace.submit_password(window, cx);
                                        });
                                    }),
                            ),
                    ),
                move |window, cx| {
                    dismissed.update(cx, |workspace, cx| workspace.cancel_password(window, cx));
                },
            )
            .into_any_element(),
        )
    }

    // --- the welcome list's menu ------------------------------------------

    /// Opens the right-click menu of the welcome row `id` at `position`.
    pub(super) fn open_profile_menu(
        &mut self,
        id: uuid::Uuid,
        position: Point<Pixels>,
        cx: &mut Context<Self>,
    ) {
        self.close_template_menus(cx);
        self.menu_open = false;
        self.tab_menu = None;
        self.profile_menu = Some((id, position));
        cx.notify();
    }

    /// Takes down whichever of the shell's own right-click menus is open.
    /// Answers whether one was.
    pub(super) fn close_context_menus(&mut self, cx: &mut Context<Self>) -> bool {
        let open = self.tab_menu.take().is_some() | self.profile_menu.take().is_some();
        if open {
            cx.notify();
        }
        open
    }

    /// The welcome row's menu: the four things there are to do with a saved
    /// connection, as rulogman's start screen offers them.
    ///
    /// *Connect* is what a press on the row already does; *Edit…* is the
    /// dialog over this profile, rather than over whichever one it would pick;
    /// the copy keeps no password, as the dialog's Duplicate does not; and
    /// *Delete…* asks first, as the dialog's Delete does.
    pub(super) fn render_profile_menu(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let (id, position) = self.profile_menu?;
        self.profiles.get(id)?;
        let this = cx.entity();
        let row = |label: SharedString,
                   run: fn(&mut Workspace, uuid::Uuid, &mut Window, &mut Context<Workspace>)| {
            let this = this.clone();
            MenuEntry::new(label).on_activate(move |window, cx| {
                this.update(cx, |workspace, cx| {
                    workspace.profile_menu = None;
                    run(workspace, id, window, cx);
                });
            })
        };
        let entries = vec![
            row(ts!("context.connect"), |workspace, id, window, cx| {
                workspace.open_profile(id, window, cx);
            }),
            MenuEntry::separator(),
            row(ts!("context.edit"), |workspace, id, window, cx| {
                workspace.open_connection_dialog(ConnectionDoor::Edit(id), window, cx);
            }),
            row(ts!("connect.duplicate"), |workspace, id, _window, cx| {
                workspace.duplicate_profile(id, cx);
            }),
            MenuEntry::separator(),
            row(ts!("context.delete"), |workspace, id, window, cx| {
                workspace.deleting_profile = Some(id);
                window.focus(&workspace.prompt_focus, cx);
                cx.notify();
            }),
        ];
        Some(
            rugpui::ContextMenu::new("profile-context")
                .position(position)
                .entries(entries)
                .on_dismiss(move |_window, cx| {
                    this.update(cx, |workspace, cx| {
                        workspace.profile_menu = None;
                        cx.notify();
                    });
                })
                .into_any_element(),
        )
    }

    /// Saves a copy of the profile `id` beside it, under a new name.
    pub(super) fn duplicate_profile(&mut self, id: uuid::Uuid, cx: &mut Context<Self>) {
        let written = generate_pane::read_modify_write(
            ConnectionStore::load,
            |store| {
                if let Some(mut copy) = store.get(id).cloned() {
                    copy.id = uuid::Uuid::new_v4();
                    copy.name = ts!("connect.copy_name", name = copy.name).to_string();
                    store.upsert(copy);
                }
            },
            ConnectionStore::save,
        );
        match written {
            Ok(store) => self.profiles = store,
            Err(error) => log::error!("connections.json was not changed: {error:#}"),
        }
        cx.notify();
    }

    /// The confirmation's Delete: forgets the profile and its secrets.
    pub(super) fn delete_profile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = self.deleting_profile.take() else {
            return;
        };
        let written = generate_pane::read_modify_write(
            ConnectionStore::load,
            |store| {
                store.remove(id);
            },
            ConnectionStore::save,
        );
        match written {
            Ok(store) => {
                self.profiles = store;
                if let Err(error) = SecretStore::delete_all(id) {
                    log::warn!("could not remove the secrets of {id}: {error:#}");
                }
                // A failure about a profile that is no longer there would be
                // a banner nothing on screen belongs to.
                if matches!(self.connection, ConnectionState::Failed { .. })
                    && self
                        .connection
                        .profile()
                        .is_some_and(|failed| failed.id == id)
                {
                    self.connection = ConnectionState::Idle;
                }
            }
            Err(error) => log::error!("connections.json was not changed: {error:#}"),
        }
        self.return_focus(window, cx);
        cx.notify();
    }

    /// "Delete this connection?", for the welcome row's menu.
    pub(super) fn render_delete_profile_prompt(
        &self,
        cx: &mut Context<Self>,
    ) -> Option<AnyElement> {
        let id = self.deleting_profile?;
        let name = self.profiles.get(id).map(label_of).unwrap_or_default();
        let theme = theme(cx);
        let (cancel, delete, dismissed, default) =
            (cx.entity(), cx.entity(), cx.entity(), cx.entity());
        let keep = |workspace: &mut Workspace, window: &mut Window, cx: &mut Context<Workspace>| {
            workspace.deleting_profile = None;
            workspace.return_focus(window, cx);
        };
        Some(
            rugpui::modal(
                "profile-delete",
                ts!("connect.delete"),
                px(400.),
                // `Enter` keeps the connection: deleting it is the answer
                // that cannot be taken back.
                prompt::body(&self.prompt_focus, move |window, cx| {
                    default.update(cx, |workspace, cx| keep(workspace, window, cx));
                })
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text)
                        .child(ts!("connect.delete_confirm", name = name)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("profile-delete-cancel", ts!("common.cancel"))
                                .variant(ButtonVariant::Secondary)
                                .on_click(move |_, window, cx| {
                                    cancel.update(cx, |workspace, cx| keep(workspace, window, cx));
                                }),
                        )
                        .child(
                            Button::new("profile-delete-confirm", ts!("connect.delete"))
                                .variant(ButtonVariant::Danger)
                                .on_click(move |_, window, cx| {
                                    delete.update(cx, |workspace, cx| {
                                        workspace.delete_profile(window, cx);
                                    });
                                }),
                        ),
                ),
                move |window, cx| {
                    dismissed.update(cx, |workspace, cx| keep(workspace, window, cx));
                },
            )
            .into_any_element(),
        )
    }
}
