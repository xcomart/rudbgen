//! Shortcuts.
//!
//! The application menu's *Keyboard shortcuts* sheet, and its *User guide*
//! row. The sheet is written from the same constants the bindings and the menu
//! hints are wherever there is one, so the three cannot drift apart, and its
//! descriptions are the menus' own words wherever those have a row for the
//! command.

use super::*;
use rugpui::{ScrollbarExt as _, TextSize, modal};

/// Where *User guide* goes: the interface guide, as the repository publishes
/// it — the page whose shortcut table this sheet mirrors.
const USER_GUIDE_URL: &str = "https://github.com/xcomart/rudbgen/blob/main/docs/ui-guide.md";

/// Width of the sheet.
const SHEET_WIDTH: f32 = 560.;

/// Height at which the sheet's list starts scrolling.
const SHEET_MAX_HEIGHT: f32 = 460.;

/// Width of the key column.
const KEY_COLUMN: f32 = 200.;

/// One heading of the sheet and its rows of key and command.
pub(super) type ShortcutGroup = (SharedString, Vec<(SharedString, SharedString)>);

/// The sheet's groups, in the order the guide's own table lists them.
///
/// A chord that only exists on some platforms — `F10`, the second generate
/// chord — is left out where it is not bound, rather than listed with a note.
/// The tab chords and the completion chord are `Ctrl` on every platform, macOS
/// included, which is why they are not written with the modifier.
pub(super) fn shortcut_groups() -> Vec<ShortcutGroup> {
    let key = |text: &str| SharedString::from(text.to_owned());
    let chord = |text: &str| key(&format!("{}+{text}", menu_rows::SHORTCUT_MODIFIER));
    let mac = cfg!(target_os = "macos");

    let mut window = vec![
        (chord("N"), ts!("menu.new_connection")),
        (chord(","), ts!("menu.settings")),
        (chord("B"), ts!("menu.toggle_explorer")),
        (chord("I"), ts!("menu.toggle_inspector")),
    ];
    if !mac {
        window.push((key("F10"), ts!("shortcuts.open_menu")));
    }
    window.extend([
        (key("Shift+F10"), ts!("shortcuts.context_menu")),
        (key("Enter"), ts!("shortcuts.default_answer")),
        (key("Esc"), ts!("shortcuts.dismiss")),
        (chord("Q"), ts!("menu.quit")),
    ]);

    let tabs = vec![
        (key("Ctrl+PgDn"), ts!("menu.next_tab")),
        (key("Ctrl+PgUp"), ts!("menu.prev_tab")),
        (key("Ctrl+Tab"), ts!("menu.next_tab")),
        (key("Ctrl+Shift+Tab"), ts!("menu.prev_tab")),
        (chord("1\u{2026}9"), ts!("shortcuts.select_tab")),
        (chord("W"), ts!("menu.close_tab")),
        (key("F6"), ts!("shortcuts.focus_tabs")),
    ];

    let template = vec![
        (chord("S"), ts!("menu.save_template")),
        (chord("Shift+P"), ts!("menu.toggle_live_preview")),
        (key("Ctrl+Space"), ts!("menu.trigger_completion")),
        (key(COMPLETION_HINT), ts!("menu.trigger_completion")),
        (key("F8"), ts!("menu.next_diagnostic")),
        (key("Shift+F8"), ts!("menu.prev_diagnostic")),
        (chord("F"), ts!("template.menu_find")),
        (chord("H"), ts!("template.menu_replace")),
    ];

    let mut generate = vec![(key(GENERATE_HINT), ts!("menu.generate"))];
    if !mac {
        generate.push((key("Ctrl+Shift+G"), ts!("menu.generate")));
    }

    vec![
        (ts!("shortcuts.group_window"), window),
        (ts!("shortcuts.group_tabs"), tabs),
        (ts!("menu.template"), template),
        (ts!("menu.generate_menu"), generate),
    ]
}

impl Workspace {
    /// Shows the shortcut sheet, closing whatever else was showing.
    pub(super) fn show_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.overlays_closed(window, cx) {
            return;
        }
        self.shortcuts_open = true;
        cx.notify();
    }

    /// Puts the shortcut sheet away, and the keyboard back where it was.
    pub(super) fn close_shortcuts(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if std::mem::take(&mut self.shortcuts_open) {
            self.return_focus(window, cx);
        }
    }

    /// Opens the user guide in the browser.
    pub(super) fn open_user_guide(&mut self, cx: &mut Context<Self>) {
        cx.open_url(USER_GUIDE_URL);
    }

    /// Renders the shortcut sheet, while it is open.
    pub(super) fn render_shortcuts(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        if !self.shortcuts_open {
            return None;
        }
        let theme = theme(cx);
        let this = cx.entity();
        let groups = shortcut_groups()
            .into_iter()
            .enumerate()
            .map(|(index, (heading, rows))| {
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .when(index > 0, |group| group.pt(px(10.)))
                    .child(
                        div()
                            .pb(px(2.))
                            .text_size(TextSize::Xs.size(cx))
                            .text_color(theme.text_muted)
                            .child(heading),
                    )
                    .children(rows.into_iter().map(|(key, action)| {
                        div()
                            .flex()
                            .flex_row()
                            .items_center()
                            .gap(px(12.))
                            .text_size(TextSize::Sm.size(cx))
                            .child(
                                div()
                                    .flex_none()
                                    .w(rugpui::scaled(KEY_COLUMN, cx))
                                    .text_color(theme.accent)
                                    .child(key),
                            )
                            .child(div().flex_1().min_w_0().child(action))
                    }))
            });
        // Shrinks below its cap in a short window, so the buttons under it
        // stay inside the dialog and the sheet scrolls instead.
        let body = div()
            .flex()
            .flex_col()
            .min_h_0()
            .gap(px(12.))
            .child(
                div()
                    .relative()
                    .flex()
                    .flex_col()
                    .min_h_0()
                    .child(
                        div()
                            .id("shortcut-sheet")
                            .track_scroll(&self.shortcuts_scroll)
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .max_h(rugpui::scaled(SHEET_MAX_HEIGHT, cx))
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .children(groups),
                    )
                    .with_scrollbar("shortcut-sheet-scrollbar", &self.shortcuts_scroll),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("shortcut-sheet-guide", ts!("menu.user_guide"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(cx.listener(|workspace, _: &gpui::ClickEvent, _, cx| {
                                workspace.open_user_guide(cx);
                            })),
                    )
                    .child(
                        Button::new("shortcut-sheet-close", ts!("common.close"))
                            .variant(ButtonVariant::Primary)
                            .focus_handle(&self.prompt_focus)
                            .on_click(cx.listener(
                                |workspace, _: &gpui::ClickEvent, window, cx| {
                                    workspace.close_shortcuts(window, cx);
                                },
                            )),
                    ),
            );
        Some(
            div()
                .absolute()
                .inset_0()
                .child(
                    modal(
                        "shortcut-sheet-modal",
                        ts!("menu.shortcuts"),
                        rugpui::scaled(SHEET_WIDTH, cx),
                        body,
                        move |window, cx| {
                            this.update(cx, |workspace, cx| workspace.close_shortcuts(window, cx));
                        },
                    )
                    .initial_focus(&self.prompt_focus)
                    .escape_dismisses(true),
                )
                .into_any_element(),
        )
    }
}
