//! The abbreviation rules editor (architecture document, §4.6 and D7/D10).
//!
//! Four columns and a delete glyph — *apply*, *whole name*, *abbreviation*,
//! *replacement* — over the one global `abbreviations.json`. jdbgen keeps the
//! same table in its Configuration window; what this adds is the trailing empty
//! row, a duplicate check that refuses to save rather than silently keeping the
//! last of two rules, and a table-name picker for the whole-name rows.
//!
//! # A draft, and only a draft
//!
//! D7: nothing here touches [`AbbreviationStore`] until **Save**. The rows are
//! text fields and two booleans; `Save` collects them, drops the blank ones,
//! writes them into the file as it is then (see [`AbbreviationDialog::write_draft`])
//! and reports the store it wrote so that the Generate tab can adopt it without
//! re-reading the disk. A file that could not be read is never saved over: the
//! dialog opens over nothing with the reason showing and **Save** grayed out. `Cancel`, `Escape` and a click on the
//! backdrop throw the draft away, the freshly typed rule included.
//!
//! # The `apply` switch is the Generate tab's
//!
//! [`AbbreviationStore::apply_to_names`] is one value with two controls: the
//! checkbox at the top of this dialog and the one on the Generate tab. The
//! dialog is filled from the panel when it opens and hands the panel the store
//! back when it saves, so the two can never disagree — and because the dialog
//! is modal, they are never both reachable at once.
//!
//! # Why duplicates are refused per kind
//!
//! The engine keys its dictionary by the **lower-cased** abbreviation, with
//! whole names in one map and words in another
//! ([`rudbgen_template::Abbreviations`]). Two enabled rules that agree on both
//! the kind and the lower-cased abbreviation therefore end as one entry, and
//! which of the two survives is a map insertion order the user cannot see. That
//! — and only that — is what [`duplicates`] refuses; a whole-name `EMP` beside
//! a word `EMP` is two entries in two maps and two different, useful rules.

use gpui::{
    App, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, MouseButton, MouseUpEvent, Render, ScrollHandle, SharedString, Subscription,
    Window, div, prelude::*, px,
};
use rudbgen_core::{AbbreviationRule, AbbreviationStore};
use rugpui::SmoothScrollExt;
use rugpui::{
    Button, ButtonVariant, Checkbox, DraggedThumb, Scrollbar, ScrollbarAxis, ScrollbarState,
    Select, TextInput, Theme, hide_later, modal, scroll_to, scrolled, theme, tooltip_label,
};

use crate::i18n::ts;
use crate::icons;

/// Width of the dialog panel.
const DIALOG_WIDTH: f32 = 720.;

/// Height at which the rule table starts scrolling.
const TABLE_MAX_HEIGHT: f32 = 380.;

/// Element id the table's overlay scroll indicator is drawn under.
const TABLE_SCROLLBAR: &str = "abbr-scrollbar";

/// Width of the *apply* column.
const APPLY_WIDTH: f32 = 52.;

/// Width of the *whole name* column.
const WHOLE_WIDTH: f32 = 92.;

/// Width of the *abbreviation* column.
const ABBR_WIDTH: f32 = 170.;

/// Width of the table-name picker beside a whole-name row's abbreviation.
const PICKER_WIDTH: f32 = 92.;

/// What the dialog tells the shell about.
pub enum AbbreviationDialogEvent {
    /// The rules were written; the store as it now stands.
    ///
    /// Carried rather than left to be re-read, so the Generate tab adopts
    /// exactly what was saved instead of whatever the file says a moment later.
    Saved(Box<AbbreviationStore>),
    /// The dialog was dismissed and the draft thrown away.
    Dismissed,
}

/// One editable row of the table.
struct RuleRow {
    /// Whether the rule takes part in a run.
    enabled: bool,
    /// Whether it matches a whole identifier rather than a word inside one.
    whole_name: bool,
    /// What it looks for.
    abbreviation: Entity<TextInput>,
    /// What it puts in its place.
    replacement: Entity<TextInput>,
    /// Keeps the two observers above alive.
    _subs: [Subscription; 2],
}

/// The rules editor.
pub struct AbbreviationDialog {
    /// Whether the dialog is visible.
    open: bool,
    /// Focus of the dialog root; also the anchor for the `Escape` handler.
    focus_handle: FocusHandle,
    /// Whether focus should move into the dialog on the next render.
    pending_focus: bool,
    /// The store the draft was filled from.
    store: AbbreviationStore,
    /// Whether the dialog was opened over a file that could not be read.
    ///
    /// The draft is then empty rather than the user's rules, and saving it
    /// would replace them all, so **Save** stays grayed out until the dialog is
    /// opened again over a file that reads.
    unreadable: bool,
    /// Why the rules could not be read or written, shown above the buttons.
    error: Option<SharedString>,
    /// The draft: one row per rule, plus the trailing blank one.
    rows: Vec<RuleRow>,
    /// The draft's copy of the global switch.
    apply_to_names: bool,
    /// Table names the picker offers, empty when nothing is connected.
    tables: Vec<SharedString>,
    /// Which row's table-name picker is open, if any.
    picker: Option<usize>,
    /// Scroll of the picker's list.
    picker_scroll: ScrollHandle,
    /// Vertical scroll of the rule table.
    scroll: ScrollHandle,
    /// Whether the table's overlay scroll indicator is on screen.
    scrollbar: ScrollbarState,
}

impl AbbreviationDialog {
    /// Builds the dialog, closed and empty.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            open: false,
            focus_handle: cx.focus_handle(),
            pending_focus: false,
            store: AbbreviationStore::default(),
            unreadable: false,
            error: None,
            rows: Vec::new(),
            apply_to_names: false,
            tables: Vec::new(),
            picker: None,
            picker_scroll: ScrollHandle::new(),
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }

    /// Fills the draft from `store` and shows the dialog.
    ///
    /// `tables` is what the picker offers: the names the explorer has loaded,
    /// already sorted and de-duplicated by the caller. An empty list simply
    /// leaves the picker out — a dropdown with nothing in it is worse than no
    /// dropdown.
    pub fn open(
        &mut self,
        store: &AbbreviationStore,
        tables: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.store = store.clone();
        self.unreadable = false;
        self.error = None;
        self.apply_to_names = store.apply_to_names;
        self.tables = tables;
        self.picker = None;
        let rows: Vec<RuleRow> = store
            .rules
            .iter()
            .map(|rule| self.new_row(rule, cx))
            .collect();
        self.rows = rows;
        self.ensure_blank(cx);
        self.open = true;
        self.pending_focus = true;
        cx.notify();
    }

    /// Shows the dialog over a rules file that could not be read.
    ///
    /// Opened empty, with `error` showing and **Save** grayed out: the rules
    /// are still in the file, and the empty draft must never replace them.
    pub fn open_unreadable(
        &mut self,
        error: &anyhow::Error,
        tables: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.open(&AbbreviationStore::default(), tables, cx);
        self.unreadable = true;
        self.error = Some(ts!("abbr.load_failed", error = format!("{error:#}")));
    }

    /// Whether the dialog is visible.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Hides the dialog and drops the draft, without emitting an event.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.pending_focus = false;
        self.picker = None;
        self.rows.clear();
        cx.notify();
    }

    /// Closes the dialog and reports it, so the shell can restore focus.
    pub fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(AbbreviationDialogEvent::Dismissed);
        self.close(cx);
    }

    /// `Escape` from anywhere inside: closes the picker first, the dialog next.
    pub fn escape(&mut self, cx: &mut Context<Self>) {
        if self.picker.take().is_some() {
            cx.notify();
            return;
        }
        self.dismiss(cx);
    }

    /// The table names the whole-name picker offers.
    ///
    /// Test-only: on screen the list is the dropdown, and what a test wants to
    /// know is whether the explorer's answer reached it at all.
    #[cfg(test)]
    pub fn tables(&self) -> &[SharedString] {
        &self.tables
    }

    /// The rules the widgets currently describe, blank rows and all.
    ///
    /// Includes the trailing empty row, and every abbreviation exactly as it
    /// was typed: this is the draft, not what would be saved. [`collect_rules`]
    /// is the step between the two.
    pub fn draft(&self, cx: &App) -> Vec<AbbreviationRule> {
        self.rows
            .iter()
            .map(|row| AbbreviationRule {
                enabled: row.enabled,
                whole_name: row.whole_name,
                abbreviation: row.abbreviation.read(cx).content().to_owned(),
                replacement: row.replacement.read(cx).content().to_owned(),
            })
            .collect()
    }

    /// The store a `Save` would write.
    ///
    /// The draft with the blank rows dropped, over the store it was filled
    /// from — so the schema version and any top-level key this build does not
    /// know survive the round trip.
    pub fn drafted_store(&self, cx: &App) -> AbbreviationStore {
        let mut store = self.store.clone();
        store.apply_to_names = self.apply_to_names;
        store.rules = collect_rules(&self.draft(cx));
        store
    }

    /// Types `rule` into row `index`, as the keyboard would.
    ///
    /// Test-only: the rows are a text field and two boxes, and everything the
    /// window does to them goes through gpui's own input handling. This is the
    /// door a headless test uses to say what a user would have typed.
    #[cfg(test)]
    pub fn write_row(&mut self, index: usize, rule: &AbbreviationRule, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let (abbreviation, replacement) = (row.abbreviation.clone(), row.replacement.clone());
        abbreviation.update(cx, |input, cx| {
            input.set_content(rule.abbreviation.clone(), cx);
        });
        replacement.update(cx, |input, cx| {
            input.set_content(rule.replacement.clone(), cx);
        });
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        row.enabled = rule.enabled;
        row.whole_name = rule.whole_name;
        self.ensure_blank(cx);
        cx.notify();
    }

    /// Writes the draft to `abbreviations.json` and reports what was written.
    ///
    /// Refuses while a duplicate stands or the dialog was opened over a file
    /// that could not be read — the Save button is disabled then, so this is
    /// the belt to that brace rather than a path a click can reach. A failed
    /// read or write leaves the dialog open with the reason showing.
    fn save(&mut self, cx: &mut Context<Self>) {
        if self.unreadable || !duplicates(&self.draft(cx)).is_empty() {
            return;
        }
        match self.write_draft(AbbreviationStore::load, AbbreviationStore::save, cx) {
            Ok(store) => {
                cx.emit(AbbreviationDialogEvent::Saved(Box::new(store)));
                self.close(cx);
            }
            Err(error) => {
                log::error!("abbreviations.json was not changed: {error:#}");
                self.error = Some(ts!("abbr.save_failed", error = format!("{error:#}")));
                cx.notify();
            }
        }
    }

    /// Re-reads the rules file, puts the draft's rules and switch in that copy,
    /// and writes it back.
    ///
    /// The draft replaces the rules — it is the whole table, as the user left
    /// it — but everything else comes from the file as it is now rather than as
    /// it was when the dialog opened: its version and the unknown top-level
    /// keys a save has to round-trip.
    ///
    /// # Errors
    ///
    /// A file that cannot be read is left exactly as it is and nothing is
    /// written. A failed write is the other error.
    fn write_draft(
        &self,
        load: impl FnOnce() -> anyhow::Result<AbbreviationStore>,
        save: impl FnOnce(&AbbreviationStore) -> anyhow::Result<()>,
        cx: &App,
    ) -> anyhow::Result<AbbreviationStore> {
        let draft = self.drafted_store(cx);
        crate::generate_pane::read_modify_write(
            load,
            |store| {
                store.apply_to_names = draft.apply_to_names;
                store.rules = draft.rules;
            },
            save,
        )
    }

    /// Builds one editable row over `rule`.
    fn new_row(&self, rule: &AbbreviationRule, cx: &mut Context<Self>) -> RuleRow {
        let abbreviation = cx.new(|cx| {
            let mut input = TextInput::new(cx).placeholder(ts!("abbr.abbreviation_placeholder"));
            input.set_content(rule.abbreviation.clone(), cx);
            input
        });
        let replacement = cx.new(|cx| {
            let mut input = TextInput::new(cx).placeholder(ts!("abbr.replacement_placeholder"));
            input.set_content(rule.replacement.clone(), cx);
            input
        });
        let subs = [
            cx.observe(&abbreviation, |dialog, _, cx| dialog.row_edited(cx)),
            cx.observe(&replacement, |dialog, _, cx| dialog.row_edited(cx)),
        ];
        RuleRow {
            enabled: rule.enabled,
            whole_name: rule.whole_name,
            abbreviation,
            replacement,
            _subs: subs,
        }
    }

    /// A field was touched: the table grows a blank row if it needs one.
    fn row_edited(&mut self, cx: &mut Context<Self>) {
        self.ensure_blank(cx);
        cx.notify();
    }

    /// Keeps exactly one blank row at the foot of the table.
    fn ensure_blank(&mut self, cx: &mut Context<Self>) {
        if needs_trailing_blank(&self.draft(cx)) {
            let row = self.new_row(&AbbreviationRule::default(), cx);
            self.rows.push(row);
        }
    }

    /// Ticks or unticks one rule.
    fn set_enabled(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        row.enabled = on;
        self.ensure_blank(cx);
        cx.notify();
    }

    /// Switches one rule between a whole name and a word.
    fn set_whole_name(&mut self, index: usize, on: bool, cx: &mut Context<Self>) {
        let Some(row) = self.rows.get_mut(index) else {
            return;
        };
        row.whole_name = on;
        // A row that stops being a whole name loses the picker with it, and a
        // list left open over a row that no longer draws it would never close.
        if !on && self.picker == Some(index) {
            self.picker = None;
        }
        self.ensure_blank(cx);
        cx.notify();
    }

    /// Drops one rule from the draft.
    fn remove(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.rows.len() {
            return;
        }
        self.rows.remove(index);
        self.picker = None;
        self.ensure_blank(cx);
        cx.notify();
    }

    /// Puts a table name into a whole-name row's abbreviation field.
    fn pick_table(&mut self, index: usize, name: &str, cx: &mut Context<Self>) {
        self.picker = None;
        let Some(row) = self.rows.get(index) else {
            return;
        };
        let field = row.abbreviation.clone();
        field.update(cx, |input, cx| input.set_content(name.to_owned(), cx));
        self.ensure_blank(cx);
        cx.notify();
    }

    /// Flips the draft's copy of the global switch.
    fn set_apply(&mut self, on: bool, cx: &mut Context<Self>) {
        self.apply_to_names = on;
        cx.notify();
    }

    /// Moves focus into the dialog when it opens, so `Escape` reaches it.
    fn apply_pending_focus(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.pending_focus {
            return;
        }
        self.pending_focus = false;
        let handle = self.focus_handle.clone();
        window.focus(&handle, cx);
        cx.notify();
    }

    /// `Escape` dismisses the dialog from anywhere inside it.
    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open && event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.escape(cx);
        }
    }

    // --- the scroll bar ---------------------------------------------------

    /// The table's overlay scroll indicator, as it now stands.
    fn bar(&self) -> Scrollbar {
        Scrollbar::for_handle(TABLE_SCROLLBAR, ScrollbarAxis::Vertical, &self.scroll)
            .fade(self.scrollbar.fade())
    }

    /// Puts the bar up whenever the table has moved.
    fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        if let Some(epoch) = self
            .scrollbar
            .moved(scrolled(&self.scroll, ScrollbarAxis::Vertical))
        {
            hide_later(epoch, cx, move |dialog: &mut Self| {
                Some(&mut dialog.scrollbar)
            });
        }
    }

    /// Scrolls the table when its thumb is dragged.
    fn drag_scrollbar(&mut self, event: &DragMoveEvent<DraggedThumb>, cx: &mut Context<Self>) {
        let Some(progress) = self.bar().dragged(event, cx) else {
            return;
        };
        self.scrollbar.hold();
        scroll_to(&self.scroll, ScrollbarAxis::Vertical, progress);
        cx.notify();
    }

    /// Lets go of the thumb, and starts its clock again.
    fn release_scrollbar(&mut self, cx: &mut Context<Self>) {
        if let Some(epoch) = self.scrollbar.release() {
            hide_later(epoch, cx, move |dialog: &mut Self| {
                Some(&mut dialog.scrollbar)
            });
            cx.notify();
        }
    }

    /// Puts the bar up while the pointer rests on the edge it rides.
    fn hover_scrollbar(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            if self.scrollbar.hover_enter() {
                cx.notify();
            }
            return;
        }
        if let Some(epoch) = self.scrollbar.hover_leave() {
            hide_later(epoch, cx, move |dialog: &mut Self| {
                Some(&mut dialog.scrollbar)
            });
            cx.notify();
        }
    }

    // --- rendering --------------------------------------------------------

    /// The column headings of the table.
    fn render_head(&self, chrome: &Theme) -> impl IntoElement + use<> {
        let cell = |width: Option<f32>, label: SharedString| {
            let base = div()
                .flex_none()
                .text_size(px(11.))
                .text_color(chrome.text_muted);
            match width {
                Some(width) => base.w(px(width)).child(label),
                None => base.flex_1().min_w_0().child(label),
            }
        };
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .child(cell(Some(APPLY_WIDTH), ts!("abbr.column_apply")))
            .child(cell(Some(WHOLE_WIDTH), ts!("abbr.column_whole")))
            .child(cell(
                Some(ABBR_WIDTH + PICKER_WIDTH + 6.),
                ts!("abbr.column_abbreviation"),
            ))
            .child(cell(None, ts!("abbr.column_replacement")))
            // The delete glyph's column, which has no heading of its own.
            .child(div().flex_none().size(px(22.)))
    }

    /// One rule row.
    fn render_row(
        &self,
        index: usize,
        row: &RuleRow,
        duplicate: bool,
        chrome: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity();
        let picker = (row.whole_name && !self.tables.is_empty()).then(|| {
            let open = self.picker == Some(index);
            let pick = this.clone();
            let toggle = this.clone();
            Select::new(("abbr-table", index))
                .chevron_icon(icons::CHEVRON_DOWN)
                .options(self.tables.clone())
                .placeholder(ts!("abbr.pick_table"))
                // No width: `Select::width` sizes the trigger as well as the
                // list, and a trigger wider than its column reaches over the
                // replacement field. Left alone, the trigger fills the column
                // and the list drops at Select's default width, which leaves a
                // long table name room the column does not have.
                .open(open)
                .scroll_handle(self.picker_scroll.clone())
                .on_select(move |_, name, _window, cx| {
                    let name = name.to_owned();
                    pick.update(cx, |dialog, cx| dialog.pick_table(index, &name, cx));
                })
                .on_open_change(move |open, _window, cx| {
                    toggle.update(cx, |dialog, cx| {
                        dialog.picker = open.then_some(index);
                        cx.notify();
                    });
                })
        });

        div()
            .id(("abbr-row", index))
            .flex()
            .flex_row()
            .w_full()
            .items_center()
            .gap(px(6.))
            .child(
                div().w(px(APPLY_WIDTH)).flex_none().child(
                    Checkbox::new(("abbr-enabled", index), SharedString::default())
                        .checked(row.enabled)
                        .on_toggle({
                            let this = this.clone();
                            move |on, _window, cx| {
                                this.update(cx, |dialog, cx| dialog.set_enabled(index, on, cx));
                            }
                        }),
                ),
            )
            .child(
                div().w(px(WHOLE_WIDTH)).flex_none().child(
                    Checkbox::new(("abbr-whole", index), SharedString::default())
                        .checked(row.whole_name)
                        .on_toggle({
                            let this = this.clone();
                            move |on, _window, cx| {
                                this.update(cx, |dialog, cx| dialog.set_whole_name(index, on, cx));
                            }
                        }),
                ),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_none()
                    .items_center()
                    .gap(px(6.))
                    .w(px(ABBR_WIDTH + PICKER_WIDTH + 6.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            // The duplicate is marked where it is, not only in
                            // the message under the table: with twenty rules
                            // "EMP is there twice" is not an instruction.
                            .when(duplicate, |field| {
                                field.rounded_md().border_1().border_color(chrome.danger)
                            })
                            .child(row.abbreviation.clone()),
                    )
                    .children(picker.map(|select| {
                        div()
                            .debug_selector(move || format!("abbr-picker-{index}"))
                            .w(px(PICKER_WIDTH))
                            .flex_none()
                            .child(select)
                    })),
            )
            .child(div().flex_1().min_w_0().child(row.replacement.clone()))
            .child(
                div()
                    .id(("abbr-remove", index))
                    .tab_index(0)
                    .focus(|style| style.bg(chrome.surface_active).text_color(chrome.accent))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(chrome.icon)
                    .cursor_pointer()
                    .hover(|style| style.bg(chrome.surface_hover).text_color(chrome.danger))
                    .tooltip(tooltip_label(ts!("abbr.tip_remove")))
                    .on_click(move |_, _window, cx| {
                        this.update(cx, |dialog, cx| dialog.remove(index, cx));
                    })
                    .child("\u{2715}"),
            )
    }
}

impl EventEmitter<AbbreviationDialogEvent> for AbbreviationDialog {}

impl Focusable for AbbreviationDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for AbbreviationDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("abbreviation-dialog");
        }

        self.apply_pending_focus(window, cx);
        self.watch_scroll(cx);

        let chrome = theme(cx);
        let this = cx.entity();
        let draft = self.draft(cx);
        let clashes = duplicates(&draft);
        let offenders: Vec<SharedString> = clashing_abbreviations(&draft);

        let apply = Checkbox::new("abbr-apply", ts!("generate.apply_abbreviations"))
            .checked(self.apply_to_names)
            .on_toggle({
                let this = this.clone();
                move |on, _window, cx| {
                    this.update(cx, |dialog, cx| dialog.set_apply(on, cx));
                }
            });

        let rows: Vec<_> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                self.render_row(index, row, clashes.contains(&index), &chrome, cx)
                    .into_any_element()
            })
            .collect();

        let table = div()
            .relative()
            .flex()
            .flex_col()
            // Give every editable row a definite width before laying it out.
            .w_full()
            .min_h_0()
            .child(
                div()
                    .id("abbr-table")
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .min_h_0()
                    .max_h(px(TABLE_MAX_HEIGHT))
                    .overflow_y_scroll()
                    .smooth_scroll(&self.scroll)
                    .restrict_scroll_to_axis()
                    .children(rows),
            )
            .children(
                self.bar()
                    .on_hover(cx.listener(|dialog, hovered: &bool, _window, cx| {
                        dialog.hover_scrollbar(*hovered, cx);
                    }))
                    .render(&chrome),
            );

        let complaint = (!offenders.is_empty()).then(|| {
            div()
                .text_size(px(11.))
                .text_color(chrome.danger)
                .child(ts!(
                    "abbr.duplicate",
                    abbreviation = offenders
                        .iter()
                        .map(SharedString::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                ))
        });

        let error = self.error.clone().map(|message| {
            div()
                .text_size(px(11.))
                .text_color(chrome.danger)
                .child(message)
        });

        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .child(Button::new("abbr-cancel", ts!("common.cancel")).on_click({
                let this = this.clone();
                move |_, _window, cx| {
                    this.update(cx, |dialog, cx| dialog.dismiss(cx));
                }
            }))
            .child(
                Button::new("abbr-save", ts!("common.save"))
                    .variant(ButtonVariant::Primary)
                    .disabled(self.unreadable || !clashes.is_empty())
                    .on_click({
                        let this = this.clone();
                        move |_, _window, cx| {
                            this.update(cx, |dialog, cx| dialog.save(cx));
                        }
                    }),
            );

        let body = div()
            .flex()
            .flex_col()
            .min_h_0()
            .gap(px(12.))
            .child(apply)
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(chrome.text_muted)
                    .child(ts!("abbr.how_it_works")),
            )
            .child(self.render_head(&chrome))
            .child(table)
            .children(complaint)
            .children(error)
            .child(footer);

        let on_dismiss = {
            let this = cx.entity();
            move |_window: &mut Window, cx: &mut App| {
                this.update(cx, |dialog, cx| dialog.escape(cx));
            }
        };

        div()
            .id("abbreviation-dialog")
            .absolute()
            .inset_0()
            .size_full()
            .track_focus(&self.focus_handle)
            .on_key_down(cx.listener(Self::on_key_down))
            .on_drag_move::<DraggedThumb>(cx.listener(
                |dialog, event: &DragMoveEvent<DraggedThumb>, _window, cx| {
                    dialog.drag_scrollbar(event, cx);
                },
            ))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|dialog, _: &MouseUpEvent, _window, cx| {
                    dialog.release_scrollbar(cx);
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|dialog, _: &MouseUpEvent, _window, cx| {
                    dialog.release_scrollbar(cx);
                }),
            )
            .child(modal(
                "abbr-modal",
                ts!("abbr.title"),
                px(DIALOG_WIDTH),
                body,
                on_dismiss,
            ))
    }
}

/// The rules as they are saved: the blank rows dropped.
///
/// A row is blank when it looks for nothing — the trailing empty row, or one
/// the user emptied to remove it. The *replacement* may legitimately be empty
/// (`TB_` → nothing is how a prefix is stripped), so it is not part of the
/// test; the abbreviation is trimmed, because a rule looking for `" EMP"` could
/// never fire on a segment the splitter produced.
pub fn collect_rules(rules: &[AbbreviationRule]) -> Vec<AbbreviationRule> {
    rules
        .iter()
        .filter(|rule| !rule.abbreviation.trim().is_empty())
        .map(|rule| AbbreviationRule {
            abbreviation: rule.abbreviation.trim().to_string(),
            ..rule.clone()
        })
        .collect()
}

/// Whether the table needs one more blank row at its foot.
///
/// The rule §4.6 calls "the trailing empty row", the same one the Generate
/// tab's variable table follows: there is always exactly one row to type a new
/// rule into, and it appears the moment the last one is filled in.
pub fn needs_trailing_blank(rules: &[AbbreviationRule]) -> bool {
    match rules.last() {
        None => true,
        Some(rule) => !rule.abbreviation.trim().is_empty() || !rule.replacement.is_empty(),
    }
}

/// Indices of the rows that repeat an earlier row's abbreviation.
///
/// Only **enabled** rows with something to look for take part, and a row
/// collides only with one of its own kind: whole names and words are two
/// dictionaries in the engine, so `EMP` in each is two rules, while `EMP` twice
/// in one is a map entry the second insertion silently wins. Matching ignores
/// case for the reason the engine does (D10): the dictionary is keyed by the
/// lower-cased abbreviation, so `Emp` and `EMP` are one key.
///
/// The **first** occurrence is not reported — it is the one the user meant to
/// keep, and marking both would leave nothing to point at.
pub fn duplicates(rules: &[AbbreviationRule]) -> Vec<usize> {
    let mut seen: Vec<(bool, String)> = Vec::new();
    let mut clashing = Vec::new();
    for (index, rule) in rules.iter().enumerate() {
        let key = rule.abbreviation.trim().to_lowercase();
        if !rule.enabled || key.is_empty() {
            continue;
        }
        let key = (rule.whole_name, key);
        if seen.contains(&key) {
            clashing.push(index);
        } else {
            seen.push(key);
        }
    }
    clashing
}

/// The abbreviations [`duplicates`] found, as the message names them.
///
/// Each one once, in the order the table shows them, and spelled as the
/// repeated row spells it — which is the row the user has to change.
fn clashing_abbreviations(rules: &[AbbreviationRule]) -> Vec<SharedString> {
    let mut names: Vec<SharedString> = Vec::new();
    for index in duplicates(rules) {
        let name = SharedString::from(rules[index].abbreviation.trim().to_owned());
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(enabled: bool, whole: bool, abbr: &str, replace: &str) -> AbbreviationRule {
        AbbreviationRule {
            enabled,
            whole_name: whole,
            abbreviation: abbr.to_string(),
            replacement: replace.to_string(),
        }
    }

    fn populated_window(cx: &mut gpui::TestAppContext) -> gpui::WindowHandle<AbbreviationDialog> {
        cx.update(rugpui::init);
        cx.add_window(|_, cx| {
            let mut dialog = AbbreviationDialog::new(cx);
            dialog.open(
                &AbbreviationStore {
                    rules: (0..50)
                        .map(|i| rule(true, true, &format!("T{i}"), &format!("Table{i}")))
                        .collect(),
                    ..Default::default()
                },
                vec!["EMP".into()],
                cx,
            );
            dialog
        })
    }

    #[gpui::test]
    fn a_rules_file_that_cannot_be_read_is_not_saved_over(cx: &mut gpui::TestAppContext) {
        // The file went bad after the dialog opened. Saving the draft anyway
        // would put fifty rules where the file has however many it has.
        let window = populated_window(cx);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("abbreviations.json");
        let damaged = b"{ \"rules\": [ not json";
        std::fs::write(&path, damaged).unwrap();

        let written = window
            .update(cx, |dialog, _, cx| {
                dialog.write_draft(
                    || AbbreviationStore::load_from(&path),
                    |store| store.save_to(&path),
                    cx,
                )
            })
            .unwrap();

        assert!(written.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), damaged);
    }

    #[gpui::test]
    fn a_dialog_opened_over_an_unreadable_file_cannot_save(cx: &mut gpui::TestAppContext) {
        cx.update(rugpui::init);
        let window = cx.add_window(|_, cx| {
            let mut dialog = AbbreviationDialog::new(cx);
            dialog.open_unreadable(&anyhow::anyhow!("expected value"), Vec::new(), cx);
            dialog
        });
        window
            .update(cx, |dialog, _, cx| {
                assert!(dialog.error.is_some(), "the reason is on screen");
                // Returns before reading or writing anything: the empty draft is
                // not the user's rules.
                dialog.save(cx);
                assert!(dialog.is_open());
            })
            .unwrap();
    }

    #[gpui::test]
    fn a_save_replaces_the_rules_and_keeps_the_rest_of_the_file(cx: &mut gpui::TestAppContext) {
        let window = populated_window(cx);
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("abbreviations.json");
        let mut on_disk = AbbreviationStore {
            rules: vec![rule(true, false, "OLD", "Old")],
            ..Default::default()
        };
        // Written by a later version since the dialog opened.
        on_disk
            .extra
            .insert("later".to_owned(), serde_json::json!("kept"));
        on_disk.save_to(&path).unwrap();

        let written = window
            .update(cx, |dialog, _, cx| {
                dialog.write_draft(
                    || AbbreviationStore::load_from(&path),
                    |store| store.save_to(&path),
                    cx,
                )
            })
            .unwrap()
            .expect("the file reads and writes");

        let reread = AbbreviationStore::load_from(&path).unwrap();
        assert_eq!(reread.rules.len(), 50);
        assert_eq!(reread.extra.get("later"), Some(&serde_json::json!("kept")));
        assert_eq!(written.rules, reread.rules);
    }

    #[gpui::test]
    fn long_rule_lists_keep_their_width_and_scroll_to_the_last_row(cx: &mut gpui::TestAppContext) {
        let window = populated_window(cx);
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        for (width, height) in [(1200., 900.), (900., 600.), (900., 400.)] {
            cx.simulate_resize(gpui::size(px(width), px(height)));
            cx.run_until_parked();
            window
                .update(&mut cx, |dialog, _, cx| {
                    let viewport = dialog.scroll.bounds();
                    let first = dialog.scroll.bounds_for_item(0).unwrap();
                    let last = dialog
                        .scroll
                        .bounds_for_item(dialog.rows.len() - 1)
                        .unwrap();
                    assert_eq!(first.size.width, viewport.size.width);
                    assert_eq!(last.size.width, viewport.size.width);
                    assert!(first.size.height >= px(20.));
                    assert!(last.size.height >= px(20.));
                    assert!(viewport.size.height > px(0.));
                    assert!(viewport.bottom() < px(height - 48.));
                    assert!(dialog.scroll.max_offset().y > px(0.));
                    dialog.scroll.scroll_to_item(dialog.rows.len() - 1);
                    cx.notify();
                })
                .unwrap();
            cx.run_until_parked();
            window
                .update(&mut cx, |dialog, _, _| {
                    assert!(dialog.scroll.offset().y < px(0.));
                    assert_eq!(dialog.scroll.bottom_item(), dialog.rows.len() - 1);
                })
                .unwrap();
        }
    }

    #[gpui::test]
    #[ignore = "manual dialog frame benchmark"]
    fn profile_abbreviation_frames(cx: &mut gpui::TestAppContext) {
        let window = populated_window(cx);
        crate::frame_profile::measure("Abbreviations (50 rules)", window, cx);
    }

    #[gpui::test]
    fn the_table_picker_stays_inside_its_column(cx: &mut gpui::TestAppContext) {
        cx.update(rugpui::init);
        let window = cx.add_window(|_, cx| {
            let mut dialog = AbbreviationDialog::new(cx);
            dialog.open(
                &AbbreviationStore {
                    rules: vec![rule(true, true, "EMP", "employee")],
                    ..Default::default()
                },
                vec!["T_SAMPLE_EMPLOYEE".into()],
                cx,
            );
            dialog
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui::size(px(1000.), px(700.)));
        cx.run_until_parked();
        let column = cx
            .debug_bounds("abbr-picker-0")
            .expect("a whole-name row has a picker while tables are loaded");

        // The gap between the picker and the replacement field belongs to
        // neither; a trigger wider than its column reaches over it.
        let beside = gpui::point(column.right() + px(3.), column.center().y);
        cx.simulate_click(beside, gpui::Modifiers::none());
        window
            .update(&mut cx, |dialog, _, _| {
                assert_eq!(dialog.picker, None, "the picker reaches past its column");
            })
            .unwrap();

        cx.simulate_click(column.center(), gpui::Modifiers::none());
        window
            .update(&mut cx, |dialog, _, _| assert_eq!(dialog.picker, Some(0)))
            .unwrap();
        cx.run_until_parked();
        // The list keeps the room a long table name needs, inside the window.
        let list = cx
            .debug_bounds("rugpui-select-list")
            .expect("the list is drawn");
        assert!(list.size.width > column.size.width, "{list:?}");
        assert!(list.right() <= px(1000.), "{list:?}");
    }

    #[gpui::test]
    fn tab_keeps_dynamic_rule_controls_in_row_order(cx: &mut gpui::TestAppContext) {
        cx.update(rugpui::init);
        let window = cx.add_window(|_, cx| {
            let mut dialog = AbbreviationDialog::new(cx);
            dialog.open(
                &AbbreviationStore {
                    rules: vec![
                        rule(true, true, "EMP", "employee"),
                        rule(true, false, "DEPT", "department"),
                    ],
                    ..Default::default()
                },
                vec!["EMP".into()],
                cx,
            );
            dialog
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        window
            .update(&mut cx, |dialog, window, cx| {
                dialog.rows[0]
                    .abbreviation
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            })
            .unwrap();
        // The optional table picker sits between abbreviation and replacement.
        cx.simulate_keystrokes("tab tab");
        window
            .update(&mut cx, |dialog, window, cx| {
                assert!(
                    dialog.rows[0]
                        .replacement
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
        cx.simulate_keystrokes("tab");
        let stroke = gpui::Keystroke::parse("enter").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: stroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke: stroke });
        window
            .update(&mut cx, |dialog, window, cx| {
                assert_eq!(dialog.rows.len(), 2);
                assert_eq!(dialog.rows[0].abbreviation.read(cx).content(), "DEPT");
                dialog.rows[0]
                    .abbreviation
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            })
            .unwrap();
        cx.simulate_keystrokes("tab");
        window
            .update(&mut cx, |dialog, window, cx| {
                assert!(
                    dialog.rows[0]
                        .replacement
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
        cx.simulate_keystrokes("shift-tab");
        window
            .update(&mut cx, |dialog, window, cx| {
                assert!(
                    dialog.rows[0]
                        .abbreviation
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
    }

    #[test]
    fn a_row_that_looks_for_nothing_is_not_saved() {
        let draft = vec![
            rule(true, false, "EMP", "Employee"),
            // The trailing blank row.
            rule(false, false, "", ""),
        ];
        let saved = collect_rules(&draft);
        assert_eq!(saved.len(), 1);
        assert_eq!(saved[0].abbreviation, "EMP");
    }

    #[test]
    fn an_empty_replacement_is_a_rule_that_strips_a_prefix() {
        // `TB_ORDERS` → `ORDERS`: the replacement is empty on purpose, and the
        // row must survive the save.
        let saved = collect_rules(&[rule(true, false, "TB", "")]);
        assert_eq!(saved.len(), 1);
        assert!(saved[0].replacement.is_empty());
    }

    #[test]
    fn a_typed_abbreviation_is_trimmed_on_the_way_to_the_file() {
        // A segment the splitter produces never carries a space, so a rule
        // looking for `" EMP"` could not fire at all.
        let saved = collect_rules(&[rule(true, false, "  EMP  ", "Employee")]);
        assert_eq!(saved[0].abbreviation, "EMP");
    }

    #[test]
    fn a_disabled_row_is_still_saved() {
        // A tick is not a deletion: switching a rule off for one project must
        // not cost the user the rule.
        let saved = collect_rules(&[rule(false, true, "T_ALBUM", "Disc")]);
        assert_eq!(saved.len(), 1);
        assert!(!saved[0].enabled);
    }

    #[test]
    fn there_is_always_exactly_one_row_to_type_into() {
        assert!(needs_trailing_blank(&[]));
        assert!(needs_trailing_blank(&[rule(
            true, false, "EMP", "Employee"
        )]));
        // Half-typed still counts as filled in: the row is being used.
        assert!(needs_trailing_blank(&[rule(false, false, "", "Employee")]));
        assert!(needs_trailing_blank(&[rule(false, false, "EMP", "")]));
        // A blank row at the foot needs no second one.
        assert!(!needs_trailing_blank(&[
            rule(true, false, "EMP", "Employee"),
            rule(false, false, "", ""),
        ]));
        assert!(!needs_trailing_blank(&[rule(false, false, "   ", "")]));
    }

    #[test]
    fn two_enabled_rules_of_one_kind_that_look_for_the_same_thing_are_refused() {
        let draft = vec![
            rule(true, false, "EMP", "Employee"),
            rule(true, false, "NO", "Number"),
            rule(true, false, "EMP", "Employer"),
        ];
        assert_eq!(duplicates(&draft), vec![2]);
        assert_eq!(clashing_abbreviations(&draft), vec!["EMP"]);
    }

    #[test]
    fn the_clash_ignores_case_because_the_dictionary_does() {
        // D10: the engine keys by the lower-cased abbreviation, so `Emp` and
        // `EMP` are one entry and one of the two replacements is lost.
        let draft = vec![
            rule(true, false, "Emp", "Employee"),
            rule(true, false, "EMP", "Employer"),
        ];
        assert_eq!(duplicates(&draft), vec![1]);
    }

    #[test]
    fn a_whole_name_and_a_word_may_look_for_the_same_thing() {
        // Two dictionaries, two entries, two useful rules: `EMP` on its own
        // becomes `Employee`, and `EMP_NO` becomes `EmployerNo`.
        let draft = vec![
            rule(true, true, "EMP", "Employee"),
            rule(true, false, "EMP", "Employer"),
        ];
        assert!(duplicates(&draft).is_empty());
    }

    #[test]
    fn a_rule_that_is_switched_off_clashes_with_nothing() {
        // Only the enabled rules reach the dictionary, so only they can
        // overwrite one another. Two spellings kept side by side, one of them
        // off, is a way of parking a rule rather than a mistake.
        let draft = vec![
            rule(true, false, "EMP", "Employee"),
            rule(false, false, "EMP", "Employer"),
        ];
        assert!(duplicates(&draft).is_empty());

        // And switching it on is what makes the clash appear.
        let draft = vec![
            rule(true, false, "EMP", "Employee"),
            rule(true, false, "EMP", "Employer"),
        ];
        assert_eq!(duplicates(&draft), vec![1]);
    }

    #[test]
    fn the_trailing_blank_rows_never_clash_with_each_other() {
        let draft = vec![
            rule(false, false, "", ""),
            rule(false, false, "", ""),
            rule(true, false, "  ", ""),
        ];
        assert!(duplicates(&draft).is_empty());
    }

    #[test]
    fn every_repeated_abbreviation_is_named_once() {
        let draft = vec![
            rule(true, false, "EMP", "a"),
            rule(true, false, "EMP", "b"),
            rule(true, false, "EMP", "c"),
            rule(true, true, "NO", "d"),
            rule(true, true, "no", "e"),
        ];
        assert_eq!(duplicates(&draft), vec![1, 2, 4]);
        assert_eq!(clashing_abbreviations(&draft), vec!["EMP", "no"]);
    }

    #[test]
    fn the_labels_the_dialog_draws_are_translated() {
        for label in [
            ts!("abbr.title"),
            ts!("abbr.how_it_works"),
            ts!("abbr.column_apply"),
            ts!("abbr.column_whole"),
            ts!("abbr.column_abbreviation"),
            ts!("abbr.column_replacement"),
            ts!("abbr.abbreviation_placeholder"),
            ts!("abbr.replacement_placeholder"),
            ts!("abbr.pick_table"),
            ts!("abbr.tip_remove"),
            ts!("abbr.duplicate", abbreviation = "EMP"),
            ts!("abbr.load_failed", error = "e"),
            ts!("abbr.save_failed", error = "e"),
        ] {
            assert!(!label.is_empty(), "empty label");
            assert!(!label.starts_with("abbr."), "untranslated label {label:?}");
        }
    }
}
