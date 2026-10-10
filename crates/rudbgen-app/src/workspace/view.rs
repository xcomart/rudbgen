//! View.

use super::*;

/// One row of the welcome screen's saved list, which opens the session it
/// names.
///
/// A press connects rather than opening the editor: the list is the way *in*,
/// and a saved connection that asks to be confirmed before it opens is a
/// dialog nobody wanted. Editing one is the connection dialog's job, which the
/// title bar and `Ctrl+N` both reach.
pub(super) fn profile_row(
    index: usize,
    name: &str,
    driver: &str,
    theme: &Theme,
    on_click: impl Fn(&gpui::ClickEvent, &mut Window, &mut App) + 'static,
) -> AnyElement {
    div()
        .id(("profile-row", index))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .py(px(5.))
        .rounded_md()
        .cursor_pointer()
        .hover(|style| style.bg(theme.surface_hover))
        .on_click(on_click)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(px(12.))
                .text_color(theme.text)
                .child(SharedString::from(name.to_owned())),
        )
        .child(
            div()
                .flex_none()
                .truncate()
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(SharedString::from(driver.to_owned())),
        )
        .into_any_element()
}

/// A box that centres its content while it fits and scrolls it when it does not.
///
/// The two halves of that are what the shape is for: `my_auto` on a `flex_none`
/// column centres it in a box with room to spare, and `overflow_y_scroll` on
/// the box takes over once there is not — so the column reads as centred until
/// there is more of it than the window, and is scrolled from the top once there
/// is not.
pub(super) fn centered_scroll(
    id: &'static str,
    scroll: &ScrollHandle,
    bar: Scrollbar,
    theme: &Theme,
    content: impl IntoElement,
) -> Div {
    div()
        .relative()
        .flex()
        .flex_col()
        .flex_grow_1()
        .min_h_0()
        .child(
            div()
                .id(id)
                .track_scroll(scroll)
                .flex()
                .flex_col()
                .flex_grow_1()
                .min_h_0()
                .items_center()
                .overflow_y_scroll()
                .restrict_scroll_to_axis()
                .child(
                    // `flex_none` so that a column taller than the box overflows
                    // it — and is scrolled to — rather than being squeezed into
                    // it, which is what a flex item does by default.
                    div()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .items_center()
                        .my_auto()
                        .py(px(SCROLL_MARGIN))
                        .child(content),
                ),
        )
        .children(bar.render(theme))
}

impl Render for Workspace {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        // The one place the interface font size is read: everything below
        // inherits it unless it sets a size of its own, which is what makes the
        // setting — and the settings dialog's live preview of it — visible.
        let ui_font_size = app_settings::effective(cx).ui_font_size;
        self.watch_scroll(cx);
        let toolbar = self.render_toolbar(window, cx);
        let body = self.render_body(cx);
        let status_bar = self.render_status_bar(cx);
        let about = self
            .about
            .read(cx)
            .is_open()
            .then(|| div().absolute().inset_0().child(self.about.clone()));
        let settings = self
            .settings
            .read(cx)
            .is_open()
            .then(|| div().absolute().inset_0().child(self.settings.clone()));
        let update = self
            .update
            .read(cx)
            .is_open()
            .then(|| div().absolute().inset_0().child(self.update.clone()));
        let connection = self.connection_dialog.read(cx).is_open().then(|| {
            div()
                .absolute()
                .inset_0()
                .child(self.connection_dialog.clone())
        });
        let abbreviations = self
            .abbreviations
            .read(cx)
            .is_open()
            .then(|| div().absolute().inset_0().child(self.abbreviations.clone()));
        let import = self
            .import
            .read(cx)
            .is_open()
            .then(|| div().absolute().inset_0().child(self.import.clone()));
        let job = self
            .job
            .read(cx)
            .is_open()
            .then(|| div().absolute().inset_0().child(self.job.clone()));
        let closing = self.closing.map(|index| {
            div()
                .absolute()
                .inset_0()
                .child(self.render_close_prompt(index, cx))
        });

        // With client-side decorations the compositor stops drawing the drop
        // shadow along with the frame, so the window has to bring its own: the
        // surface grows a transparent band all round, the content is inset by
        // it, and the shadow is painted into it. The inset call keeps
        // `_GTK_FRAME_EXTENTS` in step so the compositor treats the content
        // edge, not the surface edge, as the window.
        let tiling = chrome::client_tiling(window);
        if tiling.is_some() {
            window.set_client_inset(px(chrome::SHADOW_BAND));
        } else {
            // Clears the extents a client-side frame may have left behind when
            // the setting switches back to the system title bar on a live
            // window; a no-op under decorations that never set any.
            window.set_client_inset(px(0.));
        }

        // No background fill here on purpose. The three bands below — title
        // bar, body and status bar — cover the window between them, and each
        // paints its own. A fill at this level would sit *under* the translucent
        // body fill and compose back to opaque, which is the mistake that makes
        // `window.background_opacity` and `background_blur` look as though they
        // did nothing at all.
        let content = div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .relative()
            .size_full()
            .flex()
            .flex_col()
            .text_color(theme.text)
            .text_size(px(ui_font_size))
            // The overlay bar is answered from here rather than from the
            // surface it rides: gpui hands a drag move to every listener of
            // that type wherever it sits, and the root is the one element that
            // is always mounted while a drag of one is in flight.
            .on_drag_move::<DraggedThumb>(cx.listener(
                move |workspace, event: &DragMoveEvent<DraggedThumb>, _window, cx| {
                    workspace.drag_scrollbar(event, cx);
                },
            ))
            // The panel edges are dragged from here for the same reason: gpui
            // hands a drag move to every listener of the type wherever it sits,
            // and a release outside the handle still has to end the gesture.
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _window, cx| {
                    workspace.remember_layout(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _window, cx| {
                    workspace.remember_layout(cx);
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _window, cx| {
                    workspace.release_scrollbars(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|workspace, _: &MouseUpEvent, _window, cx| {
                    workspace.release_scrollbars(cx);
                }),
            )
            .on_action(cx.listener(Self::new_connection_action))
            .on_action(cx.listener(Self::open_settings_action))
            .on_action(cx.listener(Self::show_about_action))
            .on_action(cx.listener(Self::check_updates_action))
            .on_action(cx.listener(Self::toggle_explorer_action))
            .on_action(cx.listener(Self::toggle_inspector_action))
            .on_action(cx.listener(Self::generate_action))
            .on_action(cx.listener(Self::preview_action))
            .on_action(cx.listener(Self::dry_run_action))
            .on_action(cx.listener(Self::open_template_action))
            .on_action(cx.listener(Self::save_template_action))
            .on_action(cx.listener(Self::toggle_live_preview_action))
            .on_action(cx.listener(Self::trigger_completion_action))
            .on_action(cx.listener(Self::edit_abbreviations_action))
            .on_action(cx.listener(Self::import_jdbgen_action))
            .on_action(cx.listener(Self::dismiss_dialog_action))
            .child(toolbar)
            .child(body)
            .child(status_bar)
            .children(about)
            .children(settings)
            .children(connection)
            .children(abbreviations)
            .children(import)
            .children(update)
            // Last, so a run that is asking about a file is answered before
            // anything else on screen.
            .children(job)
            .children(closing);

        let Some(tiling) = tiling else {
            // A server-decorated window: the compositor frames and shadows it,
            // and the content is the whole surface.
            return content.into_any_element();
        };
        chrome::render_client_frame(
            content,
            tiling,
            theme.surface,
            theme.border,
            window.is_window_active(),
        )
        .into_any_element()
    }
}

impl Workspace {
    /// The connection selector, at the left of the title bar.
    ///
    /// A [`Select`] with a status dot in front of it: connecting, connected or
    /// failed, so a session that is still opening and one that died are told
    /// apart without opening anything. The list is the saved profiles, with
    /// **Disconnect** on the end while a session is open — the one row that is
    /// not a connection, which is why the handler branches on the index rather
    /// than on the text.
    pub(super) fn render_connection_select(
        &self,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let theme = theme(cx);
        let names: Vec<SharedString> = self.profiles.connections().iter().map(label_of).collect();
        let count = names.len();
        let connected = matches!(self.connection, ConnectionState::Open { .. });
        let mut options = names;
        if connected {
            options.push(ts!("titlebar.disconnect"));
        }

        // The label follows the state rather than the store: a profile that is
        // opening says so, and one that failed keeps its name on the trigger so
        // that the message on the status bar has something to belong to.
        let selected = self.connection.profile().map(|profile| {
            let label = label_of(profile);
            match self.connection {
                ConnectionState::Connecting { .. } => ts!("titlebar.connecting", name = label),
                _ => label,
            }
        });

        let this = cx.entity();
        let toggle = cx.entity();
        div()
            // Occluded because it sits inside the window's drag area: without
            // it a press on the control would move the window instead.
            .occlude()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .w(px(CONNECTION_SELECT_WIDTH))
            .child(
                div()
                    .flex_none()
                    .size(px(STATUS_DOT))
                    .rounded_full()
                    .bg(self.connection.dot(&theme)),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Select::new("connection-select")
                        .chevron_icon(icons::CHEVRON_DOWN)
                        .placeholder(ts!("titlebar.no_connection"))
                        .options(options)
                        .selected(selected)
                        .open(self.connection_list_open)
                        .width(px(CONNECTION_SELECT_WIDTH))
                        .on_select(move |index, _text, _window, cx| {
                            this.update(cx, |workspace, cx| {
                                workspace.connection_list_open = false;
                                match workspace.profiles.connections().get(index).cloned() {
                                    Some(profile) => workspace.connect_to(profile, cx),
                                    // Past the end of the list is the
                                    // "Disconnect" row.
                                    None if index == count => workspace.disconnect(cx),
                                    None => {}
                                }
                            });
                        })
                        .on_open_change(move |open, _window, cx| {
                            toggle.update(cx, |workspace, cx| {
                                // Re-read on the way open: the dialog may have
                                // added or renamed a profile since the last
                                // frame that drew this list.
                                if open {
                                    workspace.profiles = load_profiles();
                                }
                                workspace.connection_list_open = open;
                                cx.notify();
                            });
                        }),
                ),
            )
    }

    /// The application menu: every command the shell has, on the platforms
    /// without a native menu bar.
    ///
    /// Every row dispatches the action its keyboard shortcut dispatches, so the
    /// menu adds a way in rather than a second implementation. A row is greyed
    /// exactly when the action behind it would return without doing anything,
    /// and drawn rather than dropped: a command that is missing tells the reader
    /// nothing about what the application can do.
    pub(super) fn render_app_menu(&self, cx: &mut Context<Self>) -> MenuButton {
        let this = cx.entity();
        let entries = vec![
            MenuEntry::new(ts!("menu.new_connection"))
                .shortcut(format!("{}+N", menu_rows::SHORTCUT_MODIFIER))
                .on_activate(|window, cx| window.dispatch_action(Box::new(NewConnection), cx)),
            MenuEntry::new(ts!("menu.settings"))
                .shortcut(format!("{}+,", menu_rows::SHORTCUT_MODIFIER))
                .on_activate(|window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            // Greyed while nothing is connected, which is when the two panels
            // are out of the frame altogether and the command would flip a
            // switch with nothing behind it.
            MenuEntry::new(ts!("menu.toggle_explorer"))
                .shortcut(format!("{}+B", menu_rows::SHORTCUT_MODIFIER))
                .checked(self.explorer_visible)
                .disabled(!matches!(self.connection, ConnectionState::Open { .. }))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ToggleExplorer), cx)),
            MenuEntry::new(ts!("menu.toggle_inspector"))
                .shortcut(format!("{}+I", menu_rows::SHORTCUT_MODIFIER))
                .checked(self.inspector_visible)
                .disabled(!matches!(self.connection, ConnectionState::Open { .. }))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ToggleInspector), cx)),
            MenuEntry::separator(),
            // The one way into a template tab that needs no connection and no
            // template list (§4.3), beside the save that writes it back.
            MenuEntry::new(ts!("menu.open_template"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(OpenTemplate), cx)),
            MenuEntry::new(ts!("menu.save_template"))
                .shortcut(format!("{}+S", menu_rows::SHORTCUT_MODIFIER))
                .disabled(self.active_template(cx).is_none())
                .on_activate(|window, cx| window.dispatch_action(Box::new(SaveTemplate), cx)),
            MenuEntry::new(ts!("menu.toggle_live_preview"))
                .shortcut(format!("{}+Shift+P", menu_rows::SHORTCUT_MODIFIER))
                .disabled(self.active_template(cx).is_none())
                .on_activate(|window, cx| window.dispatch_action(Box::new(ToggleLivePreview), cx)),
            MenuEntry::separator(),
            // The three run commands, in the order the status bar draws them.
            // Each is greyed exactly when the button is, and for the reason the
            // button's tooltip gives.
            MenuEntry::new(ts!("menu.preview"))
                .disabled(!self.can_render(cx))
                .on_activate(|window, cx| window.dispatch_action(Box::new(Preview), cx)),
            MenuEntry::new(ts!("menu.dry_run"))
                .disabled(!self.can_render(cx))
                .on_activate(|window, cx| window.dispatch_action(Box::new(DryRun), cx)),
            MenuEntry::new(ts!("menu.generate"))
                .shortcut(format!("{}+G", menu_rows::SHORTCUT_MODIFIER))
                .disabled(!self.can_generate(cx))
                .on_activate(|window, cx| window.dispatch_action(Box::new(Generate), cx)),
            MenuEntry::new(ts!("menu.abbreviations"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(EditAbbreviations), cx)),
            MenuEntry::separator(),
            // The one-time import (D5). Never greyed: a configuration copied
            // from another machine is chosen from inside the wizard, so the
            // command has something to do even when `locate` finds nothing.
            MenuEntry::new(ts!("menu.import_jdbgen"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ImportJdbgen), cx)),
            MenuEntry::separator(),
            // Next to About, where a Help menu would put it and where users of
            // every other desktop application look for it.
            MenuEntry::new(ts!("menu.check_updates"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(CheckUpdates), cx)),
            MenuEntry::new(ts!("menu.about"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ShowAbout), cx)),
            MenuEntry::separator(),
            MenuEntry::new(ts!("menu.quit"))
                .shortcut(format!("{}+Q", menu_rows::SHORTCUT_MODIFIER))
                .on_activate(|window, cx| window.dispatch_action(Box::new(Quit), cx)),
        ];

        MenuButton::new("app-menu")
            .tooltip(ts!("titlebar.tip_menu"))
            .open(self.menu_open)
            .entries(entries)
            .on_open_change(move |open, _window, cx| {
                this.update(cx, |workspace, cx| workspace.set_menu_open(open, cx));
            })
    }

    /// Whether a preview or a dry run could be started right now.
    pub(super) fn can_render(&self, cx: &App) -> bool {
        !self.job.read(cx).is_busy()
            && self
                .readiness(cx)
                .blocker
                .is_none_or(|reason| reason == Blocker::NoOutputDir)
    }

    /// Whether a generate could be started right now.
    pub(super) fn can_generate(&self, cx: &App) -> bool {
        !self.job.read(cx).is_busy() && self.readiness(cx).blocker.is_none()
    }

    /// Renders the body of the window.
    ///
    /// The welcome screen while no session is open, and the work area once one
    /// is. The explorer and the inspector are out of the frame rather than
    /// empty until then (architecture document, §4.3); the tree that fills the
    /// first of them arrives with the metadata reader, and the tab strip
    /// between them with the Generate tab.
    pub(super) fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        // The work area needs a connection *or* an open template: a template
        // can be edited without a database (§4.3), and the tab it is in has to
        // be somewhere. With neither, the welcome screen is the whole body.
        let body =
            if matches!(self.connection, ConnectionState::Open { .. }) || self.has_templates() {
                self.render_work_area(&theme, cx)
            } else {
                self.render_welcome(&theme, cx)
            };
        div()
            .flex()
            .flex_col()
            .flex_grow_1()
            .min_h_0()
            .bg(app_settings::window_tint(theme.background, cx))
            .child(body)
            .into_any_element()
    }

    /// §4.2's frame: the explorer, the work area, the inspector.
    ///
    /// Both panels are left out of the tree entirely when they are hidden
    /// rather than given zero width — a zero-width flex child would still take
    /// its divider's hit area with it — and the divider goes with the panel,
    /// because each one is a child of the panel it resizes rather than a band
    /// of its own between them.
    ///
    /// The row paints no fill of its own. Its children tile it and each tints
    /// its own share: the panels' surface at the two edges, the background
    /// between them. Side by side rather than stacked is what
    /// [`app_settings::window_tint`] requires, and it is what lets the blur
    /// behind the window carry on under the panels too.
    pub(super) fn render_work_area(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        // `relative`, because the divider is the last child rather than the
        // next sibling: a `ResizeHandle` places itself absolutely against its
        // container, so it lands on the sidebar's own trailing border without
        // being a flex child that the row would have to make room for. The
        // widget brings the press, the resize cursor and the accent bar that
        // fades in under the pointer — the same bar a `Splitter`'s seam shows,
        // which is the point of borrowing it rather than drawing a band here.
        let sidebar = self.explorer_showing().then(|| {
            div()
                .flex()
                .relative()
                .flex_none()
                .w(px(self.explorer_width))
                .min_h_0()
                .child(self.explorer.clone())
                .child(
                    ResizeHandle::new("explorer-divider", Axis::Horizontal, DraggedExplorer)
                        .at_end()
                        .thickness(px(SPLIT_HANDLE)),
                )
        });
        // The same column, and one of two things in it: the variable palette
        // while a template is being edited, the table inspector otherwise
        // (§4.5). The palette needs no connection — a template opened from the
        // welcome screen still gets its statements and its decorators, and only
        // the example values are missing.
        let palette = self.active_template(cx).is_some();
        let panel = (palette || self.inspector_showing()).then(|| {
            div()
                .flex()
                .relative()
                .flex_none()
                .w(px(self.inspector_width))
                .min_h_0()
                .child(if palette {
                    self.palette.clone().into_any_element()
                } else {
                    self.inspector.clone().into_any_element()
                })
                // The mirror of the sidebar's, on the edge this panel is
                // dragged from: its leading one, because the panel sits at the
                // right of the row.
                .child(
                    ResizeHandle::new("inspector-divider", Axis::Horizontal, DraggedInspector)
                        .at_start()
                        .thickness(px(SPLIT_HANDLE)),
                )
        });

        div()
            .flex()
            .flex_row()
            .flex_grow_1()
            .min_w_0()
            .min_h_0()
            .on_drag_move::<DraggedExplorer>(cx.listener(
                |workspace, event: &DragMoveEvent<DraggedExplorer>, _window, cx| {
                    workspace.drag_explorer(event, cx);
                },
            ))
            .on_drag_move::<DraggedInspector>(cx.listener(
                |workspace, event: &DragMoveEvent<DraggedInspector>, _window, cx| {
                    workspace.drag_inspector(event, cx);
                },
            ))
            .children(sidebar)
            .child(
                // A column, and pointedly not the row its parent is: the tab
                // strip is `w_full`, and in a row it would take the whole of
                // the work area and push the tab under it off the frame.
                div()
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    // Everything the row is not already covering with a panel's
                    // own fill, and so the one fill over these pixels; see
                    // [`app_settings::window_tint`].
                    .bg(app_settings::window_tint(theme.background, cx))
                    .child(self.render_tabs(theme, cx))
                    .child(self.render_active_tab(theme, cx)),
            )
            .children(panel)
            .into_any_element()
    }

    /// The work area's tab strip.
    ///
    /// The Generate tab is permanent and carries no close button (§4.2); a
    /// Preview and a template tab both do, because each was asked for and can
    /// be put away — a template with unsaved edits asks first.
    pub(super) fn render_tabs(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let tabs: Vec<TabItem> = self
            .pane
            .items()
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let title = match item {
                    PaneItem::Generate => ts!("generate.tab"),
                    PaneItem::Template { title, .. } => title.clone(),
                    PaneItem::Preview { title } => title.clone(),
                };
                let tab = TabItem::new(("work-tab", index), title);
                // The dirty marker is the strip's own dot rather than a
                // character glued to the title: a title that changes width
                // when a key is pressed makes the whole strip twitch.
                match item {
                    PaneItem::Template { dirty: true, .. } => tab.dot(theme.accent),
                    _ => tab,
                }
            })
            .collect();
        let select = cx.entity();
        let close = cx.entity();
        let menu = cx.entity();

        div()
            .flex()
            .flex_none()
            .w_full()
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .child(
                TabBar::new("work-tabs")
                    .tabs(tabs)
                    .active(self.pane.active_index())
                    .scroll_handle(self.pane.scroll_handle())
                    .menu_open(self.pane_menu_open)
                    .menu_icon(icons::TAB_LIST)
                    .on_select(move |index, window, cx| {
                        select.update(cx, |workspace, cx| {
                            if !workspace.pane.activate(index) {
                                return;
                            }
                            // The tab that was on top is about to stop being
                            // rendered, so whatever inside it held the keyboard
                            // has to give it up in the same update (Appendix A)
                            // — and a template that has just come to the front
                            // asks for it back on the frame that draws it.
                            workspace.focus_shell(window, cx);
                            if let Some(at) = workspace.active_template(cx) {
                                workspace.templates[at]
                                    .clone()
                                    .update(cx, |pane, cx| pane.request_focus(cx));
                            }
                            workspace.refresh_templates(cx);
                            cx.notify();
                        });
                    })
                    .on_close(move |index, window, cx| {
                        close.update(cx, |workspace, cx| {
                            workspace.request_close_tab(index, window, cx);
                        });
                    })
                    .on_menu_open_change(move |open, _window, cx| {
                        menu.update(cx, |workspace, cx| {
                            workspace.pane_menu_open = open;
                            cx.notify();
                        });
                    }),
            )
            .into_any_element()
    }

    /// Whatever tab is on top.
    pub(super) fn render_active_tab(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let body = match self.pane.active() {
            // With no connection the Generate tab has no profile to edit, so it
            // shows the way to one: the welcome screen, inside the strip, which
            // is what a window holding nothing but a template tab looks like.
            Some(PaneItem::Generate) | None => {
                if matches!(self.connection, ConnectionState::Open { .. }) {
                    self.generate.clone().into_any_element()
                } else {
                    return self.render_welcome(theme, cx);
                }
            }
            Some(PaneItem::Preview { .. }) => self.preview.clone().into_any_element(),
            Some(PaneItem::Template { file, .. }) => match self.template_index(file, cx) {
                Some(index) => self.templates[index].clone().into_any_element(),
                // A tab with no buffer behind it cannot happen — the two are
                // pushed together — but drawing nothing is better than a panic
                // if it ever did.
                None => div().into_any_element(),
            },
        };
        div()
            .flex()
            .flex_col()
            .flex_grow_1()
            .min_h_0()
            .child(body)
            .into_any_element()
    }

    /// The "save before closing?" question.
    ///
    /// Three answers rather than two: a close that only offered *save* or
    /// *don't* would be a close nobody could take back, and the tab was
    /// closed by a press on a button four pixels from the one that selects it.
    pub(super) fn render_close_prompt(&self, index: usize, cx: &mut Context<Self>) -> AnyElement {
        let title = match self.pane.get(index) {
            Some(PaneItem::Template { title, .. }) => title.clone(),
            _ => SharedString::default(),
        };
        let theme = theme(cx);
        let save = cx.entity();
        let discard = cx.entity();
        let cancel = cx.entity();
        let dismissed = cx.entity();

        rugpui::modal(
            "template-close",
            ts!("template.close_title"),
            px(400.),
            div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(
                    div()
                        .text_size(px(12.))
                        .text_color(theme.text)
                        .child(ts!("template.close_question", file = title)),
                )
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("template-close-cancel", ts!("common.cancel")).on_click(
                                move |_, window, cx| {
                                    cancel.update(cx, |workspace, cx| {
                                        workspace.closing = None;
                                        workspace.focus_shell(window, cx);
                                    });
                                },
                            ),
                        )
                        .child(
                            Button::new("template-close-discard", ts!("template.discard"))
                                .variant(ButtonVariant::Danger)
                                .on_click(move |_, window, cx| {
                                    discard.update(cx, |workspace, cx| {
                                        workspace.answer_close(false, window, cx);
                                    });
                                }),
                        )
                        .child(
                            Button::new("template-close-save", ts!("common.save"))
                                .variant(ButtonVariant::Primary)
                                .on_click(move |_, window, cx| {
                                    save.update(cx, |workspace, cx| {
                                        workspace.answer_close(true, window, cx);
                                    });
                                }),
                        ),
                ),
            move |window, cx| {
                dismissed.update(cx, |workspace, cx| {
                    workspace.closing = None;
                    workspace.focus_shell(window, cx);
                });
            },
        )
        .into_any_element()
    }

    /// The welcome screen: the name, what the application is for, the three
    /// ways in, and the connections already saved.
    ///
    /// A saved row opens the session it names; the two buttons whose milestone
    /// has not arrived say so on hover rather than by silence.
    pub(super) fn render_welcome(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let profiles = self.profiles.connections();
        // A failed attempt is reported here as well as on the status bar: the
        // welcome screen is what the window goes back to when a connection does
        // not open, and a bar 400 pixels below the button that was pressed is
        // not where the answer is looked for.
        let failure = match &self.connection {
            ConnectionState::Failed { profile, message } => Some(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .w(px(WELCOME_WIDTH))
                    .p(px(8.))
                    .rounded_md()
                    .bg(theme.surface)
                    .border_1()
                    .border_color(theme.danger)
                    .child(
                        div()
                            .text_size(px(12.))
                            .text_color(theme.danger)
                            .child(ts!("connect.could_not_open", name = label_of(profile))),
                    )
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child(message.clone()),
                    ),
            ),
            _ => None,
        };

        // A first run has nothing saved and no habit of the chord yet, so the
        // line under the buttons is left out; once something is saved it
        // carries the shortcut that skips the button instead.
        let hint = (!profiles.is_empty()).then(|| {
            div()
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(ts!(
                    "welcome.hint",
                    shortcut = format!("{}+N", menu_rows::SHORTCUT_MODIFIER)
                ))
        });

        let saved = div()
            .flex()
            .flex_col()
            .gap(px(6.))
            .w(px(WELCOME_WIDTH))
            .child(div().text_size(px(11.)).text_color(theme.text_muted).child(
                if profiles.is_empty() {
                    ts!("welcome.empty")
                } else {
                    ts!("welcome.saved")
                },
            ))
            .when(!profiles.is_empty(), |list| {
                list.child(div().flex().flex_col().gap(px(1.)).children(
                    profiles.iter().enumerate().map(|(index, profile)| {
                        let id = profile.id;
                        profile_row(
                            index,
                            &profile.name,
                            &profile.driver_id,
                            theme,
                            cx.listener(move |workspace, _, _window, cx| {
                                let Some(profile) = workspace.profiles.get(id).cloned() else {
                                    return;
                                };
                                workspace.connect_to(profile, cx);
                            }),
                        )
                    }),
                ))
            });

        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(30.))
                    .text_color(theme.text)
                    .child(APP_NAME),
            )
            .child(
                div()
                    .w(px(WELCOME_WIDTH))
                    .text_size(px(13.))
                    .text_color(theme.text_muted)
                    .child(ts!("welcome.tagline")),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w(px(WELCOME_WIDTH))
                    .child(
                        div()
                            .id(WELCOME_NEW_SELECTOR)
                            .debug_selector(|| WELCOME_NEW_SELECTOR.to_string())
                            .child(
                                Button::new("welcome-new", ts!("welcome.new_connection"))
                                    .variant(ButtonVariant::Primary)
                                    .full_width(true)
                                    .tab_index(WELCOME_FIRST_TAB)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(NewConnection), cx);
                                    }),
                            )
                            .into_any_element(),
                    )
                    // Offered only when there is something to import from
                    // (§4.3): a button that opens a dialog saying "no jdbgen
                    // configuration found" would be worse than no button. The
                    // menu row is there either way, for a file copied from
                    // another machine.
                    .children(self.jdbgen_config.is_some().then(|| {
                        div()
                            .id(WELCOME_IMPORT_SELECTOR)
                            .debug_selector(|| WELCOME_IMPORT_SELECTOR.to_string())
                            .child(
                                Button::new("welcome-import", ts!("welcome.import_jdbgen"))
                                    .full_width(true)
                                    .tab_index(WELCOME_FIRST_TAB + 1)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(ImportJdbgen), cx);
                                    }),
                            )
                            .into_any_element()
                    }))
                    .child(
                        div()
                            .id(WELCOME_TEMPLATE_SELECTOR)
                            .debug_selector(|| WELCOME_TEMPLATE_SELECTOR.to_string())
                            .child(
                                Button::new("welcome-template", ts!("welcome.open_template"))
                                    .full_width(true)
                                    .tab_index(WELCOME_FIRST_TAB + 2)
                                    .on_click(|_, window, cx| {
                                        window.dispatch_action(Box::new(OpenTemplate), cx);
                                    }),
                            )
                            .into_any_element(),
                    ),
            )
            .children(failure)
            .children(hint)
            .child(saved);

        let bar = self.hovering_scrollbar(cx);
        centered_scroll(WELCOME_STATE, &self.welcome_scroll, bar, theme, content).into_any_element()
    }

    /// Renders the bottom status bar.
    ///
    /// The layout is the one the architecture document asks for: the connection
    /// on the left, the arithmetic of the run — tables × templates → files —
    /// filling the middle, and the three run buttons on the right. In M0 there
    /// is neither a connection nor a selection, so both cells say so and the
    /// buttons arrive with the Generate tab in M3.
    pub(super) fn render_status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(14.))
            .h(px(STATUS_BAR_HEIGHT))
            .px(px(10.))
            // The bar is inert, so a press on it must not move the keyboard.
            // Without this the workspace root's `track_focus` would claim the
            // click.
            .on_any_mouse_down(|_, window, _cx| window.prevent_default())
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(11.))
            .text_color(theme.text_muted)
            .child(
                div()
                    .flex_none()
                    .whitespace_nowrap()
                    .when(
                        matches!(self.connection, ConnectionState::Failed { .. }),
                        |cell| cell.text_color(theme.danger),
                    )
                    .child(match &self.connection {
                        ConnectionState::Idle => ts!("statusbar.no_connection"),
                        ConnectionState::Connecting { profile, .. } => {
                            ts!("statusbar.connecting", name = label_of(profile))
                        }
                        ConnectionState::Open {
                            profile, session, ..
                        } => ts!(
                            "statusbar.connected",
                            name = label_of(profile),
                            product = session
                                .product()
                                .map(SharedString::from)
                                .unwrap_or_else(|| ts!("statusbar.unknown_product"))
                        ),
                        ConnectionState::Failed { profile, .. } => {
                            ts!("statusbar.failed", name = label_of(profile))
                        }
                    }),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(match &self.connection {
                        // The reason lives here for as long as the failure
                        // does: a message the user can still read while they
                        // open the dialog to fix what it names.
                        ConnectionState::Failed { message, .. } => message.clone(),
                        // §4.2's arithmetic, in as many words: what will be
                        // read, what it will be read through, how many files
                        // that is, and where they go.
                        _ => self.render_run_summary(cx),
                    }),
            )
            .children(self.render_run_buttons(cx))
            .into_any_element()
    }

    /// The sentence in the middle of the status bar.
    pub(super) fn render_run_summary(&self, cx: &mut Context<Self>) -> SharedString {
        let ready = self.readiness(cx);
        if ready.tables == 0 {
            return ts!("statusbar.no_selection");
        }
        if ready.templates == 0 {
            return ts!("statusbar.tables_selected", count = ready.tables);
        }
        match self.generate.read(cx).output_dir(cx) {
            Some(dir) => ts!(
                "statusbar.run",
                tables = ready.tables,
                templates = ready.templates,
                files = ready.files,
                dir = dir.display().to_string()
            ),
            None => ts!(
                "statusbar.run_nowhere",
                tables = ready.tables,
                templates = ready.templates,
                files = ready.files
            ),
        }
    }

    /// The three buttons at the right end of the status bar.
    ///
    /// Disabled with the reason in a tooltip rather than an error box after the
    /// click (§4.2): the tooltip has to go on a box *around* the button,
    /// because a disabled control takes no pointer events of its own.
    pub(super) fn render_run_buttons(&self, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let ready = self.readiness(cx);
        let running = self.job.read(cx).is_busy();
        // A preview and a dry run write nothing, so the output directory is not
        // in their way; everything else is.
        let renders = ready
            .blocker
            .filter(|reason| *reason != Blocker::NoOutputDir)
            .or(running.then_some(Blocker::Disconnected));
        let writes = ready.blocker.or(running.then_some(Blocker::Disconnected));
        let reason = |blocker: Option<Blocker>| match blocker {
            Some(_) if running => ts!("generate.blocked_running"),
            Some(blocker) => blocker.message(),
            None => SharedString::default(),
        };

        let button = |id: &'static str,
                      label: SharedString,
                      blocker: Option<Blocker>,
                      primary: bool,
                      action: fn() -> Box<dyn gpui::Action>| {
            let tip = reason(blocker);
            div()
                .id(id)
                .flex_none()
                .when(blocker.is_some(), |wrapper| {
                    wrapper.tooltip(tooltip_label(tip))
                })
                .child(
                    Button::new(id, label)
                        .variant(if primary {
                            ButtonVariant::Primary
                        } else {
                            ButtonVariant::Secondary
                        })
                        .disabled(blocker.is_some())
                        .compact()
                        .on_click(move |_, window, cx| window.dispatch_action(action(), cx)),
                )
                .into_any_element()
        };

        vec![
            button(
                "statusbar-preview",
                ts!("generate.preview"),
                renders,
                false,
                || Box::new(Preview),
            ),
            button(
                "statusbar-dry-run",
                ts!("generate.dry_run"),
                renders,
                false,
                || Box::new(DryRun),
            ),
            button(
                "statusbar-generate",
                ts!("generate.run"),
                writes,
                true,
                || Box::new(Generate),
            ),
        ]
    }
}
