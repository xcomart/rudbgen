//! The inspector: what one table is made of.
//!
//! The right-hand panel of §4.2. It describes the table the explorer's cursor
//! is on — columns, primary key, foreign keys in both directions, indexes —
//! and it replaces jdbgen's *Table View* modal, which had to be opened, read
//! and dismissed before the list underneath could be touched again. A panel is
//! not a smaller dialog: it is what makes "which of these two tables did I
//! mean" a glance instead of two round trips through a modal.
//!
//! ## Four tabs, and why they are these four
//!
//! Columns is the table. Keys is the *order* of the primary key, which is the
//! one thing a generated `where` clause has to get right and the one thing a
//! column list cannot show. Foreign keys are both directions, labelled by what
//! they mean rather than by JDBC's "imported" and "exported" — which do not say
//! which way the key points. Indexes are last because they are the only tab a
//! template does not read.
//!
//! Columns is drawn with [`rudbgen_grid::GridView`] and the other three with a
//! flex table of a hundred lines. That is not an inconsistency: a column list
//! is the one of the four that runs to hundreds of rows and six columns of
//! wildly different widths, so it is the one that wants a header it can be
//! resized by and a body that lays out only what is visible. Three rows of
//! index columns want none of that.
//!
//! ## The cache
//!
//! `rudbgen-meta` caches nothing on purpose — only the application knows what a
//! refresh means — so the cache is here, keyed by [`TableKey`]. Walking the
//! explorer with the arrow keys therefore costs one round trip per table and
//! not one per keystroke, and **Refresh** drops the entry for the table on
//! screen and asks again.
//!
//! Like the explorer, the panel fetches nothing itself: it emits
//! [`InspectorEvent::Load`] and the workspace, which owns the session, answers
//! from a background task.

use std::collections::HashMap;
use std::rc::Rc;

use gpui::{
    AnyElement, App, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, Hsla,
    ScrollHandle, SharedString, Window, div, prelude::*, px,
};
use rudbgen_grid::{
    GridCell, GridColumn, GridColumnKind, GridSource, GridView, source::GridColumnAlign,
};
use rudbgen_meta::Table;
use rudbgen_ui::{
    Button, ButtonVariant, DraggedThumb, Scrollbar, ScrollbarAxis, ScrollbarState, Segmented,
    Theme, hide_later, hide_now, scroll_to, scrolled, theme,
};

use crate::app_settings;
use crate::explorer::TableKey;
use crate::i18n::ts;
use crate::icons;

/// Key context the panel's own bindings would hang off.
pub const KEY_CONTEXT: &str = "Inspector";

/// Element id of the body's overlay scroll indicator.
const BODY_SCROLLBAR: &str = "inspector-body-scrollbar";

/// Height of the header band, matching the explorer's.
const HEADER_HEIGHT: f32 = 30.;

/// The placeholder a cell with nothing in it draws.
///
/// Punctuation rather than a word, so it reads the same in every language and
/// cannot be mistaken for the string `"null"` in a default expression.
const NOTHING: SharedString = SharedString::new_static("—");

/// One row of a metadata table, already rendered to text.
type Row = Vec<SharedString>;

/// Which tab is on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Tab {
    /// The column list.
    #[default]
    Columns,
    /// The primary key, in key order.
    Keys,
    /// Foreign keys, both directions.
    ForeignKeys,
    /// The indexes.
    Indexes,
}

impl Tab {
    /// The four, in the order they are drawn.
    pub const ALL: [Tab; 4] = [Tab::Columns, Tab::Keys, Tab::ForeignKeys, Tab::Indexes];

    /// The tab's label in the active language.
    fn label(self) -> SharedString {
        match self {
            Tab::Columns => ts!("inspector.tab_columns"),
            Tab::Keys => ts!("inspector.tab_keys"),
            Tab::ForeignKeys => ts!("inspector.tab_foreign_keys"),
            Tab::Indexes => ts!("inspector.tab_indexes"),
        }
    }

    /// Element id fragment, which is never translated.
    fn slug(self) -> &'static str {
        match self {
            Tab::Columns => "columns",
            Tab::Keys => "keys",
            Tab::ForeignKeys => "foreign-keys",
            Tab::Indexes => "indexes",
        }
    }

    /// Where the tab sits in [`Tab::ALL`].
    fn index(self) -> usize {
        Tab::ALL.iter().position(|tab| *tab == self).unwrap_or(0)
    }
}

/// Where the panel's data has got to.
enum Load {
    /// Nothing is being described.
    Idle,
    /// A fetch is out.
    Running,
    /// It came back.
    Ready(Rc<Table>),
    /// It failed; the reader's own message.
    Failed(SharedString),
}

/// The six columns of the Columns tab.
///
/// Widths rather than a measurement pass: the panel is narrow, and the widest
/// value of a column is not what a reader wants it sized by — a `VARCHAR(4000)`
/// default would otherwise push the comment off the edge on every table that
/// has one. The grid lets any of them be dragged.
const COLUMN_WIDTHS: [f32; 6] = [140., 130., 74., 90., 44., 160.];

/// What the Columns tab draws, one row per column of the table.
///
/// Strings and not the model: every cell here is shown and none is computed
/// with, so turning `nullable: 1` into `NULL` once, at the edge, keeps the
/// renderer free of per-column special cases.
#[derive(Default)]
struct ColumnsSource {
    /// The headings, in the active language.
    headers: Vec<SharedString>,
    /// One row per column: name, type, nullability, default, key, comment.
    rows: Vec<Row>,
}

impl ColumnsSource {
    /// The headings, read afresh so that a language change reaches them.
    fn headings() -> Vec<SharedString> {
        vec![
            ts!("inspector.column"),
            ts!("inspector.type"),
            ts!("inspector.nullable"),
            ts!("inspector.default"),
            ts!("inspector.key"),
            ts!("inspector.comment"),
        ]
    }

    /// The rows of `table`.
    fn rows_of(table: &Table) -> Vec<Row> {
        table
            .columns
            .iter()
            .map(|column| {
                vec![
                    SharedString::from(column.name.clone()),
                    SharedString::from(column.type_string.clone()),
                    // The SQL keywords, untranslated: they are what the DDL
                    // says and what the reader is comparing against. `2` is
                    // JDBC's "the driver does not know", which is neither.
                    match column.nullable {
                        0 => SharedString::new_static("NOT NULL"),
                        1 => SharedString::new_static("NULL"),
                        _ => NOTHING,
                    },
                    if column.def_val.is_empty() {
                        NOTHING
                    } else {
                        SharedString::from(column.def_val.clone())
                    },
                    match column.key_seq {
                        // The position, not a bare mark: a composite key is
                        // ordered, and the order is what a generated `where`
                        // clause has to match.
                        Some(seq) => SharedString::from(format!("PK{seq}")),
                        None => SharedString::default(),
                    },
                    SharedString::from(column.remarks.clone()),
                ]
            })
            .collect()
    }
}

impl GridSource for ColumnsSource {
    fn column_count(&self) -> usize {
        self.headers.len()
    }

    fn column(&self, index: usize) -> GridColumn<'_> {
        let name = self.headers.get(index).map_or("", SharedString::as_ref);
        // Text throughout: the key column holds `PK1`, which is an identifier
        // and not a number, and right-aligning it would line it up with
        // nothing.
        GridColumn::new(name, GridColumnKind::Text).aligned(GridColumnAlign::Left)
    }

    fn row_count(&self) -> usize {
        self.rows.len()
    }

    fn cell(&self, row: usize, column: usize) -> GridCell<'_> {
        // `Text("")` and not `Null`: an empty comment is a comment nobody
        // wrote, and the grid draws a null as the SQL keyword — which here
        // would claim the catalog answered `NULL` when it answered nothing.
        GridCell::Text(
            self.rows
                .get(row)
                .and_then(|row| row.get(column))
                .map_or("", SharedString::as_ref),
        )
    }
}

/// What the panel asks the workspace for.
pub enum InspectorEvent {
    /// Describe this table; the workspace has the session.
    Load(TableKey),
}

/// The panel.
pub struct Inspector {
    /// What is being described, if anything.
    target: Option<TableKey>,
    /// Where the fetch for [`Inspector::target`] has got to.
    load: Load,
    /// Which tab is on screen. Kept across targets: a user reading foreign keys
    /// down a list of tables is reading foreign keys, not reading one table.
    tab: Tab,
    /// Every table already described, so that walking the tree is one round
    /// trip per table and not one per keystroke.
    cache: HashMap<TableKey, Rc<Table>>,
    /// The Columns tab.
    columns: Entity<GridView<ColumnsSource>>,
    focus_handle: FocusHandle,
    /// Scroll of the three tabs that are not Columns.
    body_scroll: ScrollHandle,
    /// Whether the body's overlay bar is on screen.
    body_scrollbar: ScrollbarState,
}

impl Inspector {
    /// An empty panel, describing nothing.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let columns = cx.new(|cx| {
            let mut grid = GridView::new(
                ColumnsSource {
                    headers: ColumnsSource::headings(),
                    rows: Vec::new(),
                },
                cx,
            );
            for (index, width) in COLUMN_WIDTHS.iter().enumerate() {
                grid.set_column_width(index, *width, cx);
            }
            grid
        });

        Self {
            target: None,
            load: Load::Idle,
            tab: Tab::default(),
            cache: HashMap::new(),
            columns,
            focus_handle: cx.focus_handle(),
            body_scroll: ScrollHandle::new(),
            body_scrollbar: ScrollbarState::new(),
        }
    }

    /// Which tab is on screen.
    #[cfg(test)]
    pub fn tab(&self) -> Tab {
        self.tab
    }

    /// The table on screen, once its fetch has come back.
    #[cfg(test)]
    pub fn table(&self) -> Option<&Table> {
        match &self.load {
            Load::Ready(table) => Some(table),
            _ => None,
        }
    }

    /// Whether a fetch is out.
    pub fn is_loading(&self) -> bool {
        matches!(self.load, Load::Running)
    }

    /// Why the last fetch failed, when it did.
    #[cfg(test)]
    pub fn failure(&self) -> Option<&SharedString> {
        match &self.load {
            Load::Failed(message) => Some(message),
            _ => None,
        }
    }

    /// Points the panel at `key`.
    ///
    /// A cache hit is drawn without asking anybody, which is what makes walking
    /// the explorer with the arrow keys cost nothing after the first pass.
    pub fn show(&mut self, key: TableKey, cx: &mut Context<Self>) {
        if self.target.as_ref() == Some(&key) && !matches!(self.load, Load::Idle) {
            return;
        }
        self.target = Some(key.clone());
        match self.cache.get(&key).cloned() {
            Some(table) => self.set_ready(table, cx),
            None => {
                self.load = Load::Running;
                cx.emit(InspectorEvent::Load(key));
                cx.notify();
            }
        }
    }

    /// Asks the database again about the table on screen.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let Some(key) = self.target.clone() else {
            return;
        };
        self.cache.remove(&key);
        self.load = Load::Running;
        cx.emit(InspectorEvent::Load(key));
        cx.notify();
    }

    /// Records what a fetch produced, or why it failed.
    ///
    /// Keyed, and dropped when the key is not the one on screen any more: a
    /// slow fetch for a table the user has already moved off must not overwrite
    /// the one they are looking at. It still goes into the cache — the work is
    /// done and the answer is good.
    pub fn deliver(
        &mut self,
        key: TableKey,
        outcome: Result<Table, SharedString>,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            Ok(table) => {
                let table = Rc::new(table);
                self.cache.insert(key.clone(), Rc::clone(&table));
                if self.target.as_ref() == Some(&key) {
                    self.set_ready(table, cx);
                }
            }
            Err(message) => {
                if self.target.as_ref() == Some(&key) {
                    self.load = Load::Failed(message);
                    cx.notify();
                }
            }
        }
    }

    /// Everything the last connection put here, gone.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.target = None;
        self.load = Load::Idle;
        self.cache.clear();
        self.columns.update(cx, |grid, cx| {
            grid.source_mut(cx).rows = Vec::new();
            grid.reset(cx);
        });
        cx.notify();
    }

    /// Re-reads the headings in the language now in force.
    ///
    /// The grid borrows its column names from the source, so a language change
    /// has to be written into it. Called by the shell when the settings dialog
    /// applies one; everything else on the panel is built afresh every frame.
    pub fn relabel(&mut self, cx: &mut Context<Self>) {
        self.columns.update(cx, |grid, cx| {
            grid.source_mut(cx).headers = ColumnsSource::headings();
        });
        cx.notify();
    }

    /// Puts a fetched table on screen and into the grid.
    fn set_ready(&mut self, table: Rc<Table>, cx: &mut Context<Self>) {
        self.columns.update(cx, |grid, cx| {
            grid.source_mut(cx).rows = ColumnsSource::rows_of(&table);
            grid.reset(cx);
        });
        self.load = Load::Ready(table);
        cx.notify();
    }

    /// Switches tabs. Costs no round trip: all four read the same fetch.
    pub fn select_tab(&mut self, tab: Tab, cx: &mut Context<Self>) {
        if self.tab != tab {
            self.tab = tab;
            cx.notify();
        }
    }

    /// The overlay bar of the body.
    ///
    /// Rebuilt every frame, as every bar in the shell is: it measures the
    /// handle, and the handle is remeasured by gpui on every layout pass.
    fn scrollbar(&self) -> Scrollbar {
        Scrollbar::for_handle(BODY_SCROLLBAR, ScrollbarAxis::Vertical, &self.body_scroll)
            .fade(self.body_scrollbar.fade())
    }

    /// Notices the body moved, and arms the bar's expiry.
    fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        let moved = scrolled(&self.body_scroll, ScrollbarAxis::Vertical);
        if let Some(epoch) = self.body_scrollbar.moved(moved) {
            hide_later(epoch, cx, |panel: &mut Self| {
                Some(&mut panel.body_scrollbar)
            });
        }
    }

    /// Scrolls the body when its thumb is dragged.
    ///
    /// Called from the workspace root: gpui hands a drag move to every listener
    /// of that type wherever it sits, and the root is the one element that is
    /// always mounted while one is in flight.
    pub fn drag_scrollbar(&mut self, event: &DragMoveEvent<DraggedThumb>, cx: &mut Context<Self>) {
        let Some(progress) = self.scrollbar().dragged(event, cx) else {
            return;
        };
        self.body_scrollbar.hold();
        scroll_to(&self.body_scroll, ScrollbarAxis::Vertical, progress);
        cx.notify();
    }

    /// Lets go of the thumb, and starts its clock again.
    pub fn release_scrollbar(&mut self, cx: &mut Context<Self>) {
        if let Some(epoch) = self.body_scrollbar.release() {
            hide_later(epoch, cx, |panel: &mut Self| {
                Some(&mut panel.body_scrollbar)
            });
            cx.notify();
        }
    }

    /// The header: what is being described, and the button that reads it again.
    fn render_header(&self, chrome: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let loading = self.is_loading();
        let (mark, name, kind, remarks) = match (&self.target, &self.load) {
            (Some(key), Load::Ready(table)) => (
                if table.is_view() {
                    icons::VIEW
                } else {
                    icons::TABLE
                },
                SharedString::from(key.name.clone()),
                Some(SharedString::from(table.kind.clone())),
                Some(table.remarks.clone()).filter(|text| !text.is_empty()),
            ),
            (Some(key), _) => (
                icons::TABLE,
                SharedString::from(key.name.clone()),
                None,
                None,
            ),
            (None, _) => (icons::TABLE, ts!("inspector.title"), None, None),
        };

        div()
            .flex()
            .flex_row()
            .flex_none()
            .items_center()
            .gap(px(6.))
            .h(px(HEADER_HEIGHT))
            .px(px(10.))
            .border_b_1()
            .border_color(chrome.border)
            .child(icons::icon(mark, px(14.), chrome.text_muted))
            .child(
                div()
                    .id("inspector-name")
                    .flex_none()
                    .max_w(px(180.))
                    .truncate()
                    .text_size(px(12.))
                    .text_color(chrome.text)
                    // The name is cut to keep the header one row tall, so the
                    // whole of it — catalog and schema included — has to be
                    // reachable somewhere, and here is where the reader is
                    // already pointing.
                    .when_some(self.target.as_ref(), |name, key| {
                        name.tooltip(rudbgen_ui::tooltip_label(SharedString::from(
                            key.qualified(),
                        )))
                    })
                    .child(name),
            )
            // The catalog's own words for what this is — `TABLE`, `VIEW` — and
            // so never translated.
            .children(kind.map(|kind| {
                div()
                    .flex_none()
                    .text_size(px(10.))
                    .text_color(chrome.text_muted)
                    .child(kind)
            }))
            .children(remarks.map(|remarks| {
                div()
                    .min_w_0()
                    .truncate()
                    .text_size(px(10.))
                    .text_color(chrome.text_muted)
                    .child(SharedString::from(remarks))
            }))
            .child(div().flex_1().min_w_0())
            .child(
                Button::new("inspector-refresh", ts!("inspector.refresh"))
                    .variant(ButtonVariant::Secondary)
                    .disabled(loading || self.target.is_none())
                    .on_click(move |_, _window, cx| {
                        this.update(cx, |panel, cx| panel.refresh(cx));
                    }),
            )
            .into_any_element()
    }

    /// The tab strip.
    fn render_tabs(&self, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        div()
            .flex_none()
            .px(px(8.))
            .py(px(6.))
            .child(
                Segmented::new("inspector-tabs")
                    .options(Tab::ALL.map(|tab| (tab.slug(), tab.label())))
                    .selected(self.tab.index())
                    .on_select(move |index, _window, cx| {
                        let Some(tab) = Tab::ALL.get(index).copied() else {
                            return;
                        };
                        this.update(cx, |panel, cx| panel.select_tab(tab, cx));
                    }),
            )
            .into_any_element()
    }

    /// The body of whichever tab is showing, or the state that stands in for it.
    fn render_body(&self, chrome: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let table = match &self.load {
            Load::Idle => {
                return fill(ts!("inspector.empty"), chrome.text_muted);
            }
            Load::Running => {
                return fill(ts!("inspector.loading"), chrome.text_muted);
            }
            Load::Failed(message) => {
                return fill(message.clone(), chrome.danger);
            }
            Load::Ready(table) => Rc::clone(table),
        };

        // The grid scrolls itself, on both axes, so it is dropped straight into
        // the panel rather than into the scrolling box the other three share.
        if self.tab == Tab::Columns {
            return div()
                .flex()
                .flex_1()
                .min_w_0()
                .min_h_0()
                .child(self.columns.clone())
                .into_any_element();
        }

        let content = match self.tab {
            Tab::Columns => unreachable!("handled above"),
            Tab::Keys => metadata_table(
                [
                    ts!("inspector.seq"),
                    ts!("inspector.column"),
                    ts!("inspector.type"),
                ],
                &key_rows(&table),
                chrome,
            ),
            // Two sections, labelled by direction rather than by JDBC's
            // "imported" and "exported": which way a key points is the one thing
            // a reader has to get right, and the JDBC words do not say it.
            Tab::ForeignKeys => div()
                .flex()
                .flex_col()
                .gap(px(14.))
                .child(section(
                    ts!("inspector.references_out"),
                    metadata_table(
                        [
                            ts!("inspector.name"),
                            ts!("inspector.columns"),
                            ts!("inspector.target"),
                            ts!("inspector.on_update"),
                            ts!("inspector.on_delete"),
                        ],
                        &foreign_key_rows(&table.imports),
                        chrome,
                    ),
                    chrome,
                ))
                .child(section(
                    ts!("inspector.references_in"),
                    metadata_table(
                        [
                            ts!("inspector.name"),
                            ts!("inspector.columns"),
                            ts!("inspector.source"),
                            ts!("inspector.on_update"),
                            ts!("inspector.on_delete"),
                        ],
                        &foreign_key_rows(&table.exports),
                        chrome,
                    ),
                    chrome,
                ))
                .into_any_element(),
            Tab::Indexes => metadata_table(
                [
                    ts!("inspector.name"),
                    ts!("inspector.unique"),
                    ts!("inspector.columns"),
                ],
                &index_rows(&table),
                chrome,
            ),
        };

        div()
            .relative()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .min_h_0()
            .child(
                div()
                    .id("inspector-body")
                    .track_scroll(&self.body_scroll)
                    .flex()
                    .flex_col()
                    .flex_1()
                    .min_h_0()
                    .p(px(10.))
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(content),
            )
            .children(
                self.scrollbar()
                    .on_hover(cx.listener(|panel, hovered: &bool, _window, cx| {
                        panel.hover_scrollbar(*hovered, cx);
                    }))
                    .render(chrome),
            )
            .into_any_element()
    }

    /// Puts the bar up while the pointer rests on the edge it rides, and starts
    /// it going the moment the pointer leaves.
    fn hover_scrollbar(&mut self, hovered: bool, cx: &mut Context<Self>) {
        if hovered {
            if self.body_scrollbar.hover_enter() {
                cx.notify();
            }
            return;
        }
        let Some(epoch) = self.body_scrollbar.hover_leave() else {
            return;
        };
        hide_now(self, epoch, cx, |panel: &mut Self| {
            Some(&mut panel.body_scrollbar)
        });
    }
}

impl EventEmitter<InspectorEvent> for Inspector {}

impl Focusable for Inspector {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Inspector {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = theme(cx);
        self.watch_scroll(cx);
        let header = self.render_header(&chrome, cx);
        let tabs = self.render_tabs(cx);
        let body = self.render_body(&chrome, cx);

        div()
            .id("inspector")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            // The one fill over these pixels, for the reason the explorer's is;
            // see [`app_settings::window_tint`].
            .bg(app_settings::window_tint(chrome.surface, cx))
            .border_l_1()
            .border_color(chrome.border)
            .child(header)
            .child(tabs)
            .child(body)
    }
}

/// The primary key's columns, in key order.
fn key_rows(table: &Table) -> Vec<Row> {
    table
        .keys()
        .into_iter()
        .map(|column| {
            vec![
                SharedString::from(column.key_seq.unwrap_or_default().to_string()),
                SharedString::from(column.name.clone()),
                SharedString::from(column.type_string.clone()),
            ]
        })
        .collect()
}

/// One row per foreign key, whichever direction it points in.
fn foreign_key_rows(keys: &[rudbgen_meta::ForeignKey]) -> Vec<Row> {
    keys.iter()
        .map(|key| {
            vec![
                if key.name.is_empty() {
                    NOTHING
                } else {
                    SharedString::from(key.name.clone())
                },
                SharedString::from(column_list(&key.columns)),
                // The other table and the columns of it this key lands on, as
                // one cell: they are one fact, and two columns of a narrow panel
                // would truncate both.
                SharedString::from(format!(
                    "{}({})",
                    key.ref_table,
                    column_list(&key.ref_columns)
                )),
                rule(&key.on_update),
                rule(&key.on_delete),
            ]
        })
        .collect()
}

/// One row per index.
fn index_rows(table: &Table) -> Vec<Row> {
    table
        .indexes
        .iter()
        .map(|index| {
            vec![
                SharedString::from(index.name.clone()),
                if index.unique {
                    // The SQL keyword, untranslated, and nothing at all for the
                    // ones that are not: a column of "no" reads as an answer to
                    // a question nobody asked.
                    SharedString::new_static("UNIQUE")
                } else {
                    SharedString::default()
                },
                SharedString::from(column_list(&index.columns)),
            ]
        })
        .collect()
}

/// `A, B` — the columns of a key or an index, in the order it declares them.
fn column_list(columns: &[rudbgen_meta::KeyColumn]) -> String {
    columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>()
        .join(", ")
}

/// A referential action, or the placeholder when the driver named none.
///
/// Never translated: `ON DELETE CASCADE` is what the DDL says and what the
/// reader is comparing against.
fn rule(action: &str) -> SharedString {
    if action.is_empty() {
        NOTHING
    } else {
        SharedString::from(action.to_owned())
    }
}

/// A one-line message filling the body.
fn fill(message: SharedString, color: Hsla) -> AnyElement {
    div()
        .flex()
        .flex_1()
        .min_h_0()
        .items_center()
        .justify_center()
        .px(px(12.))
        .text_size(px(11.))
        .text_color(color)
        .child(message)
        .into_any_element()
}

/// A titled block inside a tab.
fn section(title: SharedString, body: AnyElement, chrome: &Theme) -> AnyElement {
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .text_size(px(11.))
                .text_color(chrome.text_muted)
                .child(title),
        )
        .child(body)
        .into_any_element()
}

/// A metadata table: a header row and one row per item.
///
/// Every column shares the width evenly rather than being measured, which is
/// what keeps this thirty lines instead of a second grid: the widest cell
/// truncates, and the tab that needs real column sizing is the one that already
/// has [`GridView`].
fn metadata_table<const N: usize>(
    headers: [SharedString; N],
    rows: &[Row],
    chrome: &Theme,
) -> AnyElement {
    if rows.is_empty() {
        return div()
            .py(px(6.))
            .text_size(px(11.))
            .text_color(chrome.text_muted)
            .child(ts!("inspector.nothing_here"))
            .into_any_element();
    }

    let cell = |text: SharedString, muted: bool| {
        div()
            .flex_1()
            .min_w_0()
            .truncate()
            .px(px(6.))
            .py(px(3.))
            .text_size(px(11.))
            .text_color(if muted {
                chrome.text_muted
            } else {
                chrome.text
            })
            .child(text)
    };

    div()
        .flex()
        .flex_col()
        .w_full()
        .child(
            div()
                .flex()
                .flex_row()
                .border_b_1()
                .border_color(chrome.border)
                .children(headers.iter().map(|text| cell(text.clone(), true))),
        )
        .children(rows.iter().enumerate().map(|(index, row)| {
            div()
                .flex()
                .flex_row()
                // Zebra striping, because a row of five truncated cells is hard
                // to follow across otherwise.
                .when(index % 2 == 1, |row| row.bg(chrome.surface))
                .children((0..N).map(|column| {
                    cell(
                        row.get(column).cloned().unwrap_or_default(),
                        // The first column is the name and is what the eye
                        // follows; the rest are detail.
                        column > 0,
                    )
                }))
        }))
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use gpui::{TestAppContext, VisualTestContext};
    use rudbgen_meta::{Column, ForeignKey, Index, KeyColumn};

    use super::*;

    fn column(name: &str, key_seq: Option<u32>) -> Column {
        let mut column = Column {
            name: name.to_string(),
            type_name: "VARCHAR".to_string(),
            length: 40,
            nullable: if key_seq.is_some() { 0 } else { 1 },
            key_seq,
            ..Column::default()
        };
        column.derive();
        column
    }

    fn sample() -> Table {
        Table {
            catalog: String::new(),
            schema: "PUBLIC".to_string(),
            name: "T_SAMPLE_ALBUM".to_string(),
            kind: rudbgen_meta::KIND_TABLE.to_string(),
            remarks: "albums".to_string(),
            columns: vec![
                column("ALBUM_ID", Some(1)),
                column("ARTIST_ID", Some(2)),
                column("TITLE", None),
            ],
            imports: vec![ForeignKey {
                name: "FK_ALBUM_ARTIST".to_string(),
                columns: vec![KeyColumn {
                    name: "ARTIST_ID".to_string(),
                    no: 1,
                }],
                ref_table: "T_SAMPLE_ARTIST".to_string(),
                ref_columns: vec![KeyColumn {
                    name: "ARTIST_ID".to_string(),
                    no: 1,
                }],
                on_update: "NO ACTION".to_string(),
                on_delete: "CASCADE".to_string(),
                ..ForeignKey::default()
            }],
            indexes: vec![Index {
                name: "IX_ALBUM_TITLE".to_string(),
                unique: true,
                columns: vec![KeyColumn {
                    name: "TITLE".to_string(),
                    no: 1,
                }],
                no: 1,
            }],
            ..Table::default()
        }
    }

    fn key(name: &str) -> TableKey {
        TableKey {
            catalog: String::new(),
            schema: "PUBLIC".to_string(),
            name: name.to_string(),
        }
    }

    /// A view that does nothing but hold the panel, as the workspace does.
    struct Harness {
        panel: Entity<Inspector>,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.panel.clone())
        }
    }

    /// What the panel asked for, in order.
    type Asked = Rc<RefCell<Vec<String>>>;

    /// Opens the panel in a window and hands back it and what it asked for.
    fn open(cx: &mut TestAppContext) -> (Entity<Inspector>, Asked, VisualTestContext) {
        cx.update(|cx| {
            rudbgen_ui::init(cx);
            rudbgen_grid::init(cx);
        });

        let asked: Asked = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let asked = asked.clone();
            move |_window, cx| {
                let panel = cx.new(Inspector::new);
                cx.subscribe(
                    &panel,
                    move |_: &mut Harness, _, event: &InspectorEvent, _| match event {
                        InspectorEvent::Load(table) => asked.borrow_mut().push(table.name.clone()),
                    },
                )
                .detach();
                Harness { panel }
            }
        });
        let panel = window
            .update(cx, |harness, _, _| harness.panel.clone())
            .expect("the window is open");
        let cx = VisualTestContext::from_window(*std::ops::Deref::deref(&window), cx);
        cx.run_until_parked();
        (panel, asked, cx)
    }

    /// Everything asked for since the last look.
    fn drain(asked: &Asked) -> Vec<String> {
        asked.borrow_mut().drain(..).collect()
    }

    #[gpui::test]
    fn a_table_is_asked_for_once_and_read_from_the_cache_after_that(cx: &mut TestAppContext) {
        let (panel, asked, mut cx) = open(cx);
        assert!(
            drain(&asked).is_empty(),
            "an empty panel asked for something"
        );

        cx.update(|_, cx| panel.update(cx, |panel, cx| panel.show(key("T_ALBUM"), cx)));
        cx.run_until_parked();
        assert_eq!(drain(&asked), vec!["T_ALBUM".to_string()]);
        assert!(cx.update(|_, cx| panel.read(cx).is_loading()));

        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.deliver(key("T_ALBUM"), Ok(sample()), cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| panel.read(cx).table().map(|table| table.name.clone())),
            Some("T_SAMPLE_ALBUM".to_string())
        );

        // Away and back again: the cache answers, and nothing is asked for.
        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.show(key("T_ARTIST"), cx);
                panel.deliver(key("T_ARTIST"), Ok(sample()), cx);
                panel.show(key("T_ALBUM"), cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(drain(&asked), vec!["T_ARTIST".to_string()]);

        // A refresh drops the entry and asks again.
        cx.update(|_, cx| panel.update(cx, |panel, cx| panel.refresh(cx)));
        cx.run_until_parked();
        assert_eq!(drain(&asked), vec!["T_ALBUM".to_string()]);
    }

    #[gpui::test]
    fn switching_tabs_costs_no_round_trip(cx: &mut TestAppContext) {
        let (panel, asked, mut cx) = open(cx);
        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.show(key("T_ALBUM"), cx);
                panel.deliver(key("T_ALBUM"), Ok(sample()), cx);
            })
        });
        cx.run_until_parked();
        drain(&asked);

        for tab in Tab::ALL {
            cx.update(|_, cx| panel.update(cx, |panel, cx| panel.select_tab(tab, cx)));
            cx.run_until_parked();
            assert_eq!(cx.update(|_, cx| panel.read(cx).tab()), tab);
        }
        assert!(drain(&asked).is_empty(), "a tab switch went to the server");
    }

    #[gpui::test]
    fn an_answer_for_a_table_nobody_is_looking_at_any_more_is_kept_not_drawn(
        cx: &mut TestAppContext,
    ) {
        let (panel, _asked, mut cx) = open(cx);
        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.show(key("T_ALBUM"), cx);
                panel.show(key("T_ARTIST"), cx);
                // The first fetch lands late, after the user moved on.
                panel.deliver(key("T_ALBUM"), Ok(sample()), cx);
            })
        });
        cx.run_until_parked();
        assert!(
            cx.update(|_, cx| panel.read(cx).is_loading()),
            "a stale answer overwrote the table on screen"
        );
        assert!(cx.update(|_, cx| panel.read(cx).table().is_none()));
    }

    #[gpui::test]
    fn a_failure_is_reported_and_not_mistaken_for_an_empty_table(cx: &mut TestAppContext) {
        let (panel, _asked, mut cx) = open(cx);
        cx.update(|_, cx| {
            panel.update(cx, |panel, cx| {
                panel.show(key("T_ALBUM"), cx);
                panel.deliver(key("T_ALBUM"), Err("the session is closed".into()), cx);
            })
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| panel.read(cx).failure().cloned()),
            Some(SharedString::from("the session is closed"))
        );
        assert!(cx.update(|_, cx| panel.read(cx).table().is_none()));

        // And a new connection empties it back out.
        cx.update(|_, cx| panel.update(cx, |panel, cx| panel.reset(cx)));
        cx.run_until_parked();
        assert!(cx.update(|_, cx| panel.read(cx).failure().is_none()));
    }

    #[test]
    fn a_column_row_carries_the_type_the_nullability_and_the_key_position() {
        let rows = ColumnsSource::rows_of(&sample());
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0][0], "ALBUM_ID");
        assert_eq!(rows[0][1], "VARCHAR(40)");
        assert_eq!(rows[0][2], "NOT NULL");
        // No default: the punctuation, which cannot be read as the SQL literal.
        assert_eq!(rows[0][3], NOTHING);
        // The position and not a bare mark — a composite key is ordered.
        assert_eq!(rows[0][4], "PK1");
        assert_eq!(rows[1][4], "PK2");
        assert_eq!(rows[2][4], "");
        assert_eq!(rows[2][2], "NULL");
    }

    #[test]
    fn the_keys_tab_is_in_key_order_and_not_in_column_order() {
        let mut table = sample();
        // Declared `primary key (ARTIST_ID, ALBUM_ID)`, which is not the order
        // the columns come back in.
        table.columns[0].key_seq = Some(2);
        table.columns[1].key_seq = Some(1);

        let rows = key_rows(&table);
        assert_eq!(
            rows.iter().map(|row| row[1].clone()).collect::<Vec<_>>(),
            vec!["ARTIST_ID", "ALBUM_ID"]
        );
        assert_eq!(rows[0][0], "1");
    }

    #[test]
    fn a_foreign_key_names_the_other_table_and_the_columns_it_lands_on() {
        let table = sample();
        let rows = foreign_key_rows(&table.imports);
        assert_eq!(rows[0][0], "FK_ALBUM_ARTIST");
        assert_eq!(rows[0][1], "ARTIST_ID");
        assert_eq!(rows[0][2], "T_SAMPLE_ARTIST(ARTIST_ID)");
        assert_eq!(rows[0][3], "NO ACTION");
        assert_eq!(rows[0][4], "CASCADE");
        // The other direction is a table nothing points at, and says so.
        assert!(foreign_key_rows(&table.exports).is_empty());
    }

    #[test]
    fn an_index_says_unique_or_says_nothing() {
        let mut table = sample();
        assert_eq!(index_rows(&table)[0][1], "UNIQUE");
        assert_eq!(index_rows(&table)[0][2], "TITLE");
        table.indexes[0].unique = false;
        assert_eq!(index_rows(&table)[0][1], "");
    }

    #[test]
    fn a_referential_action_the_driver_did_not_name_is_not_an_empty_cell() {
        assert_eq!(rule(""), NOTHING);
        assert_eq!(rule("SET NULL"), "SET NULL");
    }

    #[test]
    fn the_grid_reads_its_rows_back_as_the_source_wrote_them() {
        let source = ColumnsSource {
            headers: ColumnsSource::headings(),
            rows: ColumnsSource::rows_of(&sample()),
        };
        assert_eq!(source.column_count(), 6);
        assert_eq!(source.row_count(), 3);
        assert_eq!(source.cell(0, 0), GridCell::Text("ALBUM_ID"));
        // A column nobody wrote a comment on is the empty string and not a
        // null: the grid draws the two differently, and one of them is a lie.
        assert_eq!(source.cell(0, 5), GridCell::Text(""));
        // Out of range answers rather than panicking: the grid asks about a row
        // range it worked out a frame ago.
        assert_eq!(source.cell(99, 0), GridCell::Text(""));
    }

    #[test]
    fn every_word_the_inspector_draws_is_translated() {
        let mut labels: Vec<SharedString> = Tab::ALL.iter().map(|tab| tab.label()).collect();
        labels.extend([
            ts!("inspector.title"),
            ts!("inspector.empty"),
            ts!("inspector.loading"),
            ts!("inspector.refresh"),
            ts!("inspector.nothing_here"),
            ts!("inspector.column"),
            ts!("inspector.columns"),
            ts!("inspector.type"),
            ts!("inspector.nullable"),
            ts!("inspector.default"),
            ts!("inspector.key"),
            ts!("inspector.comment"),
            ts!("inspector.name"),
            ts!("inspector.seq"),
            ts!("inspector.unique"),
            ts!("inspector.target"),
            ts!("inspector.source"),
            ts!("inspector.on_update"),
            ts!("inspector.on_delete"),
            ts!("inspector.references_out"),
            ts!("inspector.references_in"),
        ]);
        for label in labels {
            assert!(!label.is_empty(), "empty label");
            assert!(
                !label.starts_with("inspector."),
                "untranslated label {label:?}"
            );
        }
    }
}
