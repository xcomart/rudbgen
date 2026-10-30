//! The keyboard half of every question the window asks.
//!
//! A question in a modal — save before closing, replace this file, delete this
//! set, throw these edits away — is answered with a button, and a button is
//! only reachable from the keyboard once something has put the keyboard in
//! the dialog. [`body`] is that something: the box the question's text and
//! buttons sit in holds a focus handle of the asker's, wears
//! [`PROMPT_CONTEXT`], and answers `Enter` ([`ConfirmDialog`]) with whatever
//! the asker names as the default.
//!
//! Only while the box *itself* holds the keyboard, though. Once `Tab` has
//! moved it onto one of the buttons, `Enter` is that button's — gpui turns it
//! into a click on the focused element — so the action is handed on rather
//! than answered, and a user who tabbed to Cancel gets Cancel.
//!
//! `Escape` is not here: the shell binds it once, to [`crate::DismissDialog`],
//! and walks its own stack of overlays (see `Workspace::dismiss_dialog_action`),
//! so each question says what `Escape` means for it there.

use std::rc::Rc;

use gpui::{
    AnyElement, App, Div, ElementId, FocusHandle, SharedString, Window, div, prelude::*, px,
};
use rugpui::{Button, ButtonVariant, modal, theme};

use crate::ConfirmDialog;
use crate::i18n::ts;

/// Key context of a question's box; `Enter` is bound to [`ConfirmDialog`] in
/// it (see the workspace's `bind_shortcuts`).
pub(crate) const PROMPT_CONTEXT: &str = "Prompt";

/// The box a question's text and buttons go in.
///
/// `focus` is the asker's handle, which it moves the keyboard onto when the
/// question goes up; `on_confirm` is the default answer, run by `Enter` while
/// the box itself has the keyboard.
pub(crate) fn body(
    focus: &FocusHandle,
    on_confirm: impl Fn(&mut Window, &mut App) + 'static,
) -> Div {
    let own = focus.clone();
    div()
        .track_focus(focus)
        .key_context(PROMPT_CONTEXT)
        .on_action(move |_: &ConfirmDialog, window, cx| {
            if own.is_focused(window) {
                on_confirm(window, cx);
            } else {
                cx.propagate();
            }
        })
}

/// What a dialog with edits in it asks before it lets them go.
///
/// Drawn by the dialog itself, over its own form, whenever `Escape`, a press
/// on the backdrop or its Cancel would otherwise throw a changed draft away.
/// Both answers are the dialog's: *Keep editing* takes the question down and
/// leaves the form exactly as it was, *Discard* closes the dialog the way the
/// gesture that asked would have. `Enter` and the backdrop keep editing — the
/// one answer that loses nothing is the one a stray key or click must land on.
pub(crate) fn discard(
    id: impl Into<ElementId>,
    focus: &FocusHandle,
    question: SharedString,
    on_keep: impl Fn(&mut Window, &mut App) + 'static,
    on_discard: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = theme(cx);
    let id = id.into();
    let keep = Rc::new(on_keep);
    let (enter, backdrop, button) = (keep.clone(), keep.clone(), keep);
    modal(
        id,
        ts!("prompt.discard_title"),
        px(380.),
        body(focus, move |window, cx| enter(window, cx))
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text)
                    .child(question),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .justify_end()
                    .gap(px(8.))
                    .child(
                        Button::new("discard-keep", ts!("prompt.keep_editing"))
                            .variant(ButtonVariant::Secondary)
                            .on_click(move |_, window, cx| button(window, cx)),
                    )
                    .child(
                        Button::new("discard-confirm", ts!("prompt.discard"))
                            .variant(ButtonVariant::Danger)
                            .on_click(move |_, window, cx| on_discard(window, cx)),
                    ),
            ),
        move |window, cx| backdrop(window, cx),
    )
    .into_any_element()
}
