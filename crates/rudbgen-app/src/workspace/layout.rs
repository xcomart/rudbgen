//! Layout.

use super::*;

impl Workspace {
    /// Writes the panel widths and visibilities into the settings global.
    ///
    /// [`app_settings::save`] takes them to disk with everything else when the
    /// last window closes, so this costs a struct copy and no file write.
    pub(super) fn remember_layout(&mut self, cx: &mut Context<Self>) {
        let mut settings = app_settings::current(cx);
        let same = settings.explorer_visible == self.explorer_visible
            && settings.inspector_visible == self.inspector_visible
            && (settings.explorer_width - self.explorer_width).abs() <= f32::EPSILON
            && (settings.inspector_width - self.inspector_width).abs() <= f32::EPSILON;
        if same {
            return;
        }
        settings.explorer_visible = self.explorer_visible;
        settings.inspector_visible = self.inspector_visible;
        settings.explorer_width = self.explorer_width;
        settings.inspector_width = self.inspector_width;
        app_settings::replace(settings, cx);
    }

    /// Whether the explorer is on screen, which needs a session as well as the
    /// switch.
    pub(super) fn explorer_showing(&self) -> bool {
        self.explorer_visible && matches!(self.connection, ConnectionState::Open { .. })
    }

    /// Whether the inspector is on screen.
    pub(super) fn inspector_showing(&self) -> bool {
        self.inspector_visible && matches!(self.connection, ConnectionState::Open { .. })
    }

    /// Moves the explorer's edge to wherever the pointer has dragged it.
    ///
    /// Measured against the row's own box rather than tracked as a delta, so
    /// the edge sits under the pointer however far the gesture wandered —
    /// including outside the window, which a delta would have to keep
    /// integrating.
    pub(super) fn drag_explorer(
        &mut self,
        event: &DragMoveEvent<DraggedExplorer>,
        cx: &mut Context<Self>,
    ) {
        let width = f32::from(event.event.position.x - event.bounds.left());
        if !width.is_finite() {
            return;
        }
        let width = width.clamp(MIN_EXPLORER_WIDTH, MAX_EXPLORER_WIDTH);
        if (self.explorer_width - width).abs() > f32::EPSILON {
            self.explorer_width = width;
            cx.notify();
        }
    }

    /// The same, from the other edge of the row.
    pub(super) fn drag_inspector(
        &mut self,
        event: &DragMoveEvent<DraggedInspector>,
        cx: &mut Context<Self>,
    ) {
        let width = f32::from(event.bounds.right() - event.event.position.x);
        if !width.is_finite() {
            return;
        }
        let width = width.clamp(MIN_INSPECTOR_WIDTH, MAX_INSPECTOR_WIDTH);
        if (self.inspector_width - width).abs() > f32::EPSILON {
            self.inspector_width = width;
            cx.notify();
        }
    }

    // --- the scroll bar over the welcome screen ---------------------------

    /// The welcome screen's overlay scroll indicator, as it now stands.
    ///
    /// Rebuilt on demand rather than kept, because everything it is made of —
    /// the box, how far it overflows, where it sits — is measured afresh by
    /// gpui on every layout pass.
    pub(super) fn scrollbar(&self) -> Scrollbar {
        Scrollbar::for_handle(
            WELCOME_SCROLLBAR,
            ScrollbarAxis::Vertical,
            &self.welcome_scroll,
        )
        .fade(self.welcome_scrollbar.fade())
    }

    /// The same bar, listening for the pointer reaching the edge it rides.
    pub(super) fn hovering_scrollbar(&self, cx: &mut Context<Self>) -> Scrollbar {
        self.scrollbar()
            .on_hover(cx.listener(move |workspace, hovered: &bool, _window, cx| {
                workspace.hover_scrollbar(*hovered, cx);
            }))
    }

    /// Puts the bar up whenever the welcome screen has moved, and starts the
    /// clock that takes it down again.
    pub(super) fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        let scrolled = scrolled(&self.welcome_scroll, ScrollbarAxis::Vertical);
        if let Some(epoch) = self.welcome_scrollbar.moved(scrolled) {
            hide_later(epoch, cx, move |workspace| {
                Some(&mut workspace.welcome_scrollbar)
            });
        }
    }

    /// Scrolls the welcome screen when its thumb is dragged.
    pub(super) fn drag_scrollbar(
        &mut self,
        event: &DragMoveEvent<DraggedThumb>,
        cx: &mut Context<Self>,
    ) {
        // Every bar in the window answers a drag of this type and all but one
        // of them finds it is about somebody else's thumb; the inspector's is
        // asked here because the panel has no always-mounted element of its own
        // to listen from.
        self.inspector
            .update(cx, |panel, cx| panel.drag_scrollbar(event, cx));
        self.generate
            .update(cx, |pane, cx| pane.drag_scrollbar(event, cx));
        self.preview
            .update(cx, |pane, cx| pane.drag_scrollbar(event, cx));
        self.palette
            .update(cx, |panel, cx| panel.drag_scrollbar(event, cx));
        let Some(progress) = self.scrollbar().dragged(event, cx) else {
            return;
        };
        // Held even when the pointer moved sideways and the box has not budged:
        // the bar has to stay up for as long as it is being held, and a still
        // pointer moves nothing to notice.
        self.welcome_scrollbar.hold();
        scroll_to(&self.welcome_scroll, ScrollbarAxis::Vertical, progress);
        cx.notify();
    }

    /// Lets go of the thumb, and starts its clock again.
    ///
    /// Every mouse release in the window arrives here; all but the one ending a
    /// drag of the bar find nothing to let go of.
    pub(super) fn release_scrollbars(&mut self, cx: &mut Context<Self>) {
        self.inspector
            .update(cx, |panel, cx| panel.release_scrollbar(cx));
        self.generate
            .update(cx, |pane, cx| pane.release_scrollbar(cx));
        self.preview
            .update(cx, |pane, cx| pane.release_scrollbar(cx));
        self.palette
            .update(cx, |panel, cx| panel.release_scrollbar(cx));
        if let Some(epoch) = self.welcome_scrollbar.release() {
            hide_later(epoch, cx, move |workspace| {
                Some(&mut workspace.welcome_scrollbar)
            });
            cx.notify();
        }
    }

    /// Puts the bar up while the pointer rests on the edge it rides, and starts
    /// it going the moment the pointer leaves.
    pub(super) fn hover_scrollbar(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            if self.welcome_scrollbar.hover_enter() {
                cx.notify();
            }
            return;
        }

        let Some(epoch) = self.welcome_scrollbar.hover_leave() else {
            return;
        };
        hide_now(self, epoch, cx, move |workspace| {
            Some(&mut workspace.welcome_scrollbar)
        });
    }

    // --- rendering --------------------------------------------------------

    /// Renders the title bar: the application mark, the application menu
    /// button and the connection selector.
    ///
    /// The button is left out on macOS, where [`app_menus`] puts the same
    /// commands in the system menu bar.
    ///
    /// In the custom title bar style this row *is* the title bar. It then marks
    /// itself as the window's drag area, takes over writing the application's
    /// name at its left end, and — off macOS, which keeps its native traffic
    /// lights — grows a set of caption buttons at its right end. Every *control*
    /// inside it occludes, so the drag area only ever answers for the gaps
    /// between them; see [`rugpui::window_controls`]. The name is not a
    /// control and deliberately does not.
    pub(super) fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let custom = chrome::draws_own_titlebar(chrome_style(self.titlebar), window);
        let titlebar_active = custom && window.is_window_active();
        let titlebar_text = if titlebar_active {
            theme.text
        } else {
            theme.text_muted
        };
        let menu = (!cfg!(target_os = "macos")).then(|| self.render_app_menu(cx));

        // Room for the traffic lights AppKit still draws over the transparent
        // title bar. Fullscreen hides the buttons, and the gap goes with them.
        let traffic_lights = (custom && cfg!(target_os = "macos") && !window.is_fullscreen())
            .then(|| div().flex_none().w(px(TRAFFIC_LIGHT_GAP)));

        // The application's own name, which only the custom style has to write:
        // a system title bar already carries it, and drawing it twice would put
        // it in two places at once.
        //
        // Windows and the GTK/KDE captions set an application icon beside the
        // title and macOS does not, so the mark follows that split.
        //
        // Nothing here is interactive, and — unlike every control in this row —
        // nothing here occludes either. The name and the mark are part of the
        // *empty* title bar as far as the window is concerned, so a press on
        // them has to reach the drag area underneath and move the window.
        let title = custom.then(|| {
            // Use the shipped PNG so the title bar preserves the icon's
            // colours on every platform instead of tinting its alpha mask.
            let icon = (!cfg!(target_os = "macos")).then(|| {
                let icon = img(icons::APP_ICON).size(px(16.)).flex_none();
                if !titlebar_active {
                    div()
                        .size(px(16.))
                        .flex_none()
                        .opacity(0.55)
                        .child(icon)
                        .into_any_element()
                } else {
                    icon.into_any_element()
                }
            });
            div()
                .flex()
                .flex_row()
                .flex_none()
                .items_center()
                .gap(px(6.))
                .px(px(4.))
                // A shade quieter than the connection selector, which is the
                // one control in this row that has to be read.
                .text_size(px(12.))
                .text_color(titlebar_text)
                .children(icon)
                .child(APP_NAME)
        });

        // The caption buttons the other two platforms have to draw themselves,
        // as the two ends a Linux desktop may ask for them at.
        let (leading_controls, trailing_controls) = chrome::window_control_strips(
            &rugpui_shell::window_control_icons(),
            custom,
            window,
            cx,
        );

        div()
            .id("toolbar")
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .w_full()
            .h(px(TOOLBAR_HEIGHT))
            .px(px(6.))
            .bg(theme.surface)
            .border_b_1()
            .border_color(theme.border)
            .when(custom, |this| {
                // Occluding is load-bearing, not just hygiene: the workspace
                // root tracks focus, and gpui's focus transfer marks every
                // mouse down over it `default_prevented` — which the Windows
                // backend reads as "the app took this press", swallowing the
                // `HTCAPTION` down that would have started the system drag.
                // Cutting the root's hitbox out from under the strip keeps the
                // press unclaimed.
                chrome::titlebar_gestures(
                    this.occlude().window_control_area(WindowControlArea::Drag),
                )
            })
            // Ahead of the wordmark, which is where a desktop that asks for
            // left-hand caption buttons expects them: the buttons are the
            // window's, the name is the application's.
            .children(leading_controls)
            .children(traffic_lights)
            .children(title)
            .children(menu)
            .child(self.render_connection_select(cx))
            // The gap the window is dragged by: everything to its left and
            // right is a control, and this is what is left of the caption.
            .child(div().flex_1().min_w_0())
            .children(trailing_controls)
            .into_any_element()
    }
}
