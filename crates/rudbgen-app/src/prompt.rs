//! The question a dialog with edits in it asks before it lets them go.
//!
//! The keyboard half of every question the window asks is rugpui's [`Modal`]
//! now: `initial_focus` puts the keyboard on the button that is the safe or the
//! expected answer when the question goes up, `on_confirm` is what `Enter` on
//! the panel does, and the focus goes back where it was when the question goes.
//! A focused button keeps `Enter` for itself, so a user who tabbed to Cancel
//! gets Cancel.
//!
//! `Escape` is not here: the shell binds it once, to [`crate::DismissDialog`],
//! and walks its own stack of overlays (see `Workspace::dismiss_dialog_action`),
//! so each question says what `Escape` means for it there.

use std::rc::Rc;

use gpui::{AnyElement, App, ElementId, FocusHandle, SharedString, Window, div, prelude::*, px};
use rugpui::{Button, ButtonVariant, TextSize, modal, theme};

use crate::i18n::ts;

/// "Discard your edits?", drawn by a dialog over its own form whenever
/// `Escape` or its Cancel would otherwise throw a changed draft away.
///
/// Both answers are the dialog's: *Keep editing* takes the question down and
/// leaves the form exactly as it was, *Discard* closes the dialog the way the
/// gesture that asked would have. *Keep editing* holds the keyboard and is what
/// `Enter` and the backdrop answer — the one answer that loses nothing is the
/// one a stray key or click must land on. `keep` is that button's focus handle,
/// the asker's to keep from frame to frame.
pub(crate) fn discard(
    id: impl Into<ElementId>,
    keep: &FocusHandle,
    question: SharedString,
    on_keep: impl Fn(&mut Window, &mut App) + 'static,
    on_discard: impl Fn(&mut Window, &mut App) + 'static,
    cx: &App,
) -> AnyElement {
    let theme = theme(cx);
    let id = id.into();
    let on_keep = Rc::new(on_keep);
    let (enter, backdrop, button) = (on_keep.clone(), on_keep.clone(), on_keep);
    modal(
        id,
        ts!("prompt.discard_title"),
        px(380.),
        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(
                div()
                    .text_size(TextSize::Sm.size(cx))
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
                            .focus_handle(keep)
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
    .initial_focus(keep)
    .on_confirm(move |window, cx| enter(window, cx))
    .into_any_element()
}
