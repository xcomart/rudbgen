//! The explorer: the tree of tables, and the set of them that will be
//! generated.
//!
//! Three things live here and only the first is a widget. The **tree** is
//! catalog → schema → table, filled one round trip at a time through
//! [`rudbgen_ui::TreeView`]; the **selection** is a set of [`TableKey`]s that
//! survives everything the tree does to itself; and the **filter** is jdbgen's
//! `filterTables` rule, which is a pure function over a name and therefore
//! testable without a window.
//!
//! ## Why the selection is a set and not a flag on a row
//!
//! Because the rows come and go. A schema that is collapsed drops its children,
//! a filter hides half of them, the *views* toggle hides the rest, and a
//! refresh throws the whole list away and fetches it again — and none of that
//! is the user changing their mind about which tables they meant. A tick stored
//! on a row would be lost by every one of those; a set keyed by name is lost by
//! none of them (architecture.md §4.2).
//!
//! The set is ordered ([`BTreeSet`]) rather than hashed, because the generation
//! run walks it and the order files are written in should not depend on a hash
//! seed.
//!
//! ## Why the views toggle does not refetch
//!
//! [`rudbgen_meta::MetaReader::tables`] takes `include_views`, so the obvious
//! reading of the toggle is a second round trip. The cache holds the *wider*
//! answer instead — views included, always — and the toggle filters what is
//! drawn. Flipping it is then free, and the tables in the two states cannot
//! disagree about anything, which two fetches taken minutes apart could.
//!
//! ## What this module does not do
//!
//! Fetch. Every call into `rudbgen-meta` blocks, so the panel asks — with
//! [`ExplorerEvent::LoadSchemas`] and [`ExplorerEvent::LoadTables`] — and the
//! workspace, which owns the session, answers on a background task. The tree
//! draws a placeholder in the meantime; nothing here ever waits.

use std::collections::{BTreeSet, HashMap};

use gpui::{
    AnyElement, App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable,
    MouseButton, Pixels, Point, SharedString, Subscription, WeakEntity, Window, div, prelude::*,
    px,
};
use rudbgen_meta::{Schema, TableRef};
use rudbgen_ui::{
    ChildState, TextInput, Theme, TreeEvent, TreeRow, TreeRowInfo, TreeSource, TreeView, theme,
    tooltip_label,
};

use crate::app_settings;
use crate::context_menu::{self, MenuRow};
use crate::i18n::ts;
use crate::icons;

/// Key context the panel's own bindings would hang off.
pub const KEY_CONTEXT: &str = "Explorer";

/// Height of the header band over the tree, matching the inspector's.
const HEADER_HEIGHT: f32 = 30.;

/// Edge length of the tick box drawn on a schema or a table row.
const TICK_SIZE: f32 = 14.;

/// Glyph of a ticked box.
const TICK_ON: &str = "\u{2713}";

/// Glyph of a box some of whose children are ticked.
///
/// A dash rather than a smaller square: the three states have to be told apart
/// at 14 pixels, and a filled square reads as a tick that failed to draw.
const TICK_SOME: &str = "\u{2013}";

/// One schema of the connected database, by everything that identifies it.
///
/// All three names, because two of them are not enough: a product with no
/// schemas gets a placeholder per catalog whose [`Schema::schema`] is empty
/// (`MetaReader::schemas`, rule 3), so `(catalog, schema)` collides across
/// them and only the display name tells them apart.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SchemaKey {
    /// The catalog, empty when the product has none.
    pub catalog: String,
    /// The schema as the driver reports it, empty for a placeholder.
    pub schema: String,
    /// What the row is labelled with.
    pub name: String,
}

impl SchemaKey {
    /// The key of a schema the reader answered with.
    pub fn of(schema: &Schema) -> Self {
        Self {
            catalog: schema.catalog.clone(),
            schema: schema.schema.clone(),
            name: schema.name.clone(),
        }
    }

    /// The schema this names, for a fetch.
    pub fn schema(&self) -> Schema {
        Schema {
            catalog: self.catalog.clone(),
            schema: self.schema.clone(),
            name: self.name.clone(),
        }
    }
}

/// One table or view of the connected database.
///
/// The identity a tick is stored under and the inspector is pointed at. Ordered
/// by catalog, then schema, then name — which is the order the generation run
/// writes files in.
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct TableKey {
    /// The catalog, empty when the product has none.
    pub catalog: String,
    /// The schema, empty when the product has none.
    pub schema: String,
    /// The table name as the database reports it.
    pub name: String,
}

impl TableKey {
    /// The key of a table the reader answered with.
    pub fn of(table: &TableRef) -> Self {
        Self {
            catalog: table.catalog.clone(),
            schema: table.schema.clone(),
            name: table.name.clone(),
        }
    }

    /// `catalog.schema.name`, with the empty parts left out.
    ///
    /// What the inspector's header shows and what a tooltip carries. The parts
    /// a product does not have are absent rather than empty, so a MySQL table
    /// reads `app.orders` and not `app..orders`.
    pub fn qualified(&self) -> String {
        [
            self.catalog.as_str(),
            self.schema.as_str(),
            self.name.as_str(),
        ]
        .into_iter()
        .filter(|part| !part.is_empty())
        .collect::<Vec<_>>()
        .join(".")
    }
}

/// A node of the tree.
///
/// Ids have to survive a reload — the widget keys everything it remembers by
/// them — so every variant is made of names rather than of positions.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum NodeId {
    /// A catalog, drawn only when the connection has more than one.
    Catalog(String),
    /// A schema.
    Schema(SchemaKey),
    /// A table or a view.
    Table(TableKey),
    /// The row standing in for a schema whose tables could not be read.
    Error(SchemaKey),
}

/// Where one fetch has got to.
///
/// [`ChildState`] says the same thing for the widget, but it says it in terms
/// of child *ids*, and this has to hold the rows themselves — which are
/// filtered on the way out and so are not the ids.
#[derive(Debug, Default)]
enum Fetch<T> {
    /// Nobody has asked yet.
    #[default]
    NotLoaded,
    /// A round trip is out.
    Loading,
    /// It came back.
    Loaded(T),
    /// It failed, and the reader's own words are what is shown.
    Failed(SharedString),
}

/// jdbgen's `filterTables`: a name passes when it contains the filter, ignoring
/// case, and every name passes an empty filter.
///
/// Deliberately not a glob, a prefix match or a fuzzy score. It is what the
/// tool being replaced did, it is what a user typing three letters expects, and
/// the assets that depend on it are the users' habits.
pub fn matches_filter(name: &str, filter: &str) -> bool {
    let needle = filter.trim();
    if needle.is_empty() {
        return true;
    }
    name.to_lowercase().contains(&needle.to_lowercase())
}

/// The tables of one schema that the filter and the *views* toggle leave.
///
/// The two conditions in one function because they are one question — "what is
/// on screen under this schema" — and every caller that asks it has to get the
/// same answer: the rows, the schema's own tick box, and the "select all shown"
/// command would otherwise be able to disagree.
pub fn visible_tables<'a>(
    tables: &'a [TableRef],
    filter: &str,
    show_views: bool,
) -> Vec<&'a TableRef> {
    tables
        .iter()
        .filter(|table| show_views || !table.is_view())
        .filter(|table| matches_filter(&table.name, filter))
        .collect()
}

/// Whether a group of tables is ticked wholly, partly, or not at all.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tick {
    /// None of them.
    Off,
    /// Some of them. Drawn as a dash, and clicked to tick the rest.
    Partial,
    /// All of them. An empty group is [`Tick::Off`], not [`Tick::On`]: a schema
    /// with nothing under it has nothing selected.
    On,
}

/// How `members` stand in `selection`.
pub fn tick_of(selection: &BTreeSet<TableKey>, members: &[TableKey]) -> Tick {
    if members.is_empty() {
        return Tick::Off;
    }
    let ticked = members
        .iter()
        .filter(|key| selection.contains(*key))
        .count();
    if ticked == 0 {
        Tick::Off
    } else if ticked == members.len() {
        Tick::On
    } else {
        Tick::Partial
    }
}

/// Ticks every one of `members`.
pub fn select_all(selection: &mut BTreeSet<TableKey>, members: &[TableKey]) {
    selection.extend(members.iter().cloned());
}

/// Unticks every one of `members`, leaving anything else alone.
pub fn deselect_all(selection: &mut BTreeSet<TableKey>, members: &[TableKey]) {
    for key in members {
        selection.remove(key);
    }
}

/// Flips each of `members`, leaving anything else alone.
///
/// Over the visible rows and not over the whole selection, because that is what
/// "invert" means on a filtered list: a table the filter is hiding was not part
/// of what the user was looking at when they asked.
pub fn invert(selection: &mut BTreeSet<TableKey>, members: &[TableKey]) {
    for key in members {
        if !selection.remove(key) {
            selection.insert(key.clone());
        }
    }
}

/// The distinct catalogs of a schema list, in the order they first appear.
pub fn catalogs_of(schemas: &[Schema]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    for schema in schemas {
        if !seen.contains(&schema.catalog) {
            seen.push(schema.catalog.clone());
        }
    }
    seen
}

/// Whether the tree draws a catalog level at all.
///
/// One catalog is every catalog, so a level naming it says nothing and costs a
/// click on every schema underneath. rudbman skips it for the same reason.
pub fn catalog_level(schemas: &[Schema]) -> bool {
    catalogs_of(schemas).len() > 1
}

/// Where the tree gets its nodes.
///
/// Owned by the [`TreeView`], which is owned by the [`Explorer`]; the panel
/// reaches it through [`TreeView::source_mut`]. It holds the fetched
/// metadata, the selection and the two things that narrow what is drawn.
#[derive(Default)]
pub struct ExplorerSource {
    /// The panel, so that a row's tick box can reach it.
    ///
    /// Weak: the panel owns the tree, the tree owns this, and a strong handle
    /// would close the ring and leak all three.
    host: Option<WeakEntity<Explorer>>,
    /// The schemas, once the root fetch answered.
    schemas: Fetch<Vec<Schema>>,
    /// The table list of each schema, **views included whatever the toggle
    /// says** — see the module documentation.
    tables: HashMap<SchemaKey, Fetch<Vec<TableRef>>>,
    /// Every fetched table by its key, so that drawing a row costs a lookup
    /// rather than a walk of the schema it belongs to.
    ///
    /// A second copy of the same rows, kept in step by the two methods that
    /// write [`ExplorerSource::tables`] and by no one else. A `TableKey` does
    /// not carry the schema's *display* name, so it cannot address the map
    /// above — which is the whole reason this one exists.
    index: HashMap<TableKey, TableRef>,
    /// The ticked tables.
    selection: BTreeSet<TableKey>,
    /// What the filter box holds.
    filter: String,
    /// Whether views are drawn.
    show_views: bool,
}

impl ExplorerSource {
    /// The source of an empty panel, with `host` to reach it by.
    fn new(host: WeakEntity<Explorer>) -> Self {
        Self {
            host: Some(host),
            ..Self::default()
        }
    }

    /// Everything the connection put here, gone.
    fn clear(&mut self) {
        self.schemas = Fetch::NotLoaded;
        self.tables.clear();
        self.index.clear();
        self.selection.clear();
    }

    /// Throws the fetched metadata away, keeping the selection and the filter.
    ///
    /// What "Refresh" does: the user is asking the database again, not asking
    /// to be given a blank sheet. A table that is gone the second time around
    /// stays in the selection and simply has no row — which is the honest
    /// answer, since the tick was about a name and the name is what is missing.
    fn invalidate(&mut self) {
        self.schemas = Fetch::NotLoaded;
        self.tables.clear();
        self.index.clear();
    }

    /// Records one schema's table list, or why it could not be read.
    fn set_tables(&mut self, key: SchemaKey, outcome: Result<Vec<TableRef>, SharedString>) {
        // Whatever this schema had in the index goes first, so that a refresh
        // which drops a table drops the row it drew too.
        self.index
            .retain(|_, table| table.schema != key.schema || table.catalog != key.catalog);
        let fetch = match outcome {
            Ok(tables) => {
                for table in &tables {
                    self.index.insert(TableKey::of(table), table.clone());
                }
                Fetch::Loaded(tables)
            }
            Err(message) => Fetch::Failed(message),
        };
        self.tables.insert(key, fetch);
    }

    /// The schemas under `catalog`, in the order the reader gave them.
    fn schemas_of(&self, catalog: &str) -> Vec<Schema> {
        let Fetch::Loaded(schemas) = &self.schemas else {
            return Vec::new();
        };
        schemas
            .iter()
            .filter(|schema| schema.catalog == catalog)
            .cloned()
            .collect()
    }

    /// The tables of `key` that the filter and the toggle leave, as keys.
    fn visible_keys(&self, key: &SchemaKey) -> Vec<TableKey> {
        let Some(Fetch::Loaded(tables)) = self.tables.get(key) else {
            return Vec::new();
        };
        visible_tables(tables, &self.filter, self.show_views)
            .into_iter()
            .map(TableKey::of)
            .collect()
    }

    /// The table `id` names, for its icon and its comment.
    fn table_of(&self, id: &TableKey) -> Option<&TableRef> {
        self.index.get(id)
    }

    /// The name of every table fetched so far, in no particular order.
    fn table_names(&self) -> Vec<String> {
        self.index.keys().map(|key| key.name.clone()).collect()
    }

    /// What the label of a row says.
    fn label_of(&self, id: &NodeId) -> SharedString {
        match id {
            NodeId::Catalog(name) => SharedString::from(name.clone()),
            NodeId::Schema(key) => SharedString::from(key.name.clone()),
            NodeId::Table(key) => SharedString::from(key.name.clone()),
            NodeId::Error(key) => match self.tables.get(key) {
                Some(Fetch::Failed(message)) => message.clone(),
                _ => ts!("explorer.load_failed_unknown"),
            },
        }
    }

    /// The tick box of a row, and what pressing it does.
    ///
    /// Drawn here rather than with [`rudbgen_ui::Checkbox`] for two reasons: it
    /// has a third state, and it has to swallow its own press so that aiming at
    /// the box does not also move the tree's selection — the same trick the
    /// disclosure arrow plays one element to the left.
    fn render_tick(
        &self,
        id: &NodeId,
        info: TreeRowInfo,
        state: Tick,
        chrome: &Theme,
    ) -> AnyElement {
        let (bg, border, glyph) = match state {
            Tick::On => (chrome.accent, chrome.accent, TICK_ON),
            Tick::Partial => (chrome.surface, chrome.accent, TICK_SOME),
            Tick::Off => (chrome.surface, chrome.border, ""),
        };
        let host = self.host.clone();
        let id = id.clone();

        div()
            .id(("explorer-tick", info.index))
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(TICK_SIZE))
            .rounded_sm()
            .border_1()
            .border_color(border)
            .bg(bg)
            .text_size(px(10.))
            .text_color(if matches!(state, Tick::On) {
                chrome.background
            } else {
                chrome.accent
            })
            .cursor_pointer()
            // The press is taken here so the row underneath never sees the
            // click: ticking a table is not selecting it, and a tick that also
            // moved the inspector would make "pick eight tables" eight fetches.
            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
            .on_click(move |_: &ClickEvent, _window, cx| {
                cx.stop_propagation();
                if let Some(host) = &host {
                    host.update(cx, |explorer, cx| explorer.toggle_tick(&id, cx))
                        .ok();
                }
            })
            .child(glyph)
            .into_any_element()
    }
}

impl TreeSource for ExplorerSource {
    type Id = NodeId;

    fn children(&self, parent: Option<&NodeId>) -> ChildState<NodeId> {
        let Some(parent) = parent else {
            return match &self.schemas {
                Fetch::NotLoaded => ChildState::NotLoaded,
                Fetch::Loading => ChildState::Loading,
                // The reason is drawn in place of the tree, so the root has
                // nothing left to say; see [`Explorer::render`].
                Fetch::Failed(_) => ChildState::Loaded(Vec::new()),
                Fetch::Loaded(schemas) => ChildState::Loaded(if catalog_level(schemas) {
                    catalogs_of(schemas)
                        .into_iter()
                        .map(NodeId::Catalog)
                        .collect()
                } else {
                    schemas
                        .iter()
                        .map(|schema| NodeId::Schema(SchemaKey::of(schema)))
                        .collect()
                }),
            };
        };

        match parent {
            NodeId::Catalog(catalog) => ChildState::Loaded(
                self.schemas_of(catalog)
                    .iter()
                    .map(|schema| NodeId::Schema(SchemaKey::of(schema)))
                    .collect(),
            ),
            NodeId::Schema(key) => match self.tables.get(key) {
                None | Some(Fetch::NotLoaded) => ChildState::NotLoaded,
                Some(Fetch::Loading) => ChildState::Loading,
                // One row carrying the driver's whole sentence, rather than a
                // schema that silently looks empty.
                Some(Fetch::Failed(_)) => ChildState::Loaded(vec![NodeId::Error(key.clone())]),
                Some(Fetch::Loaded(_)) => ChildState::Loaded(
                    self.visible_keys(key)
                        .into_iter()
                        .map(NodeId::Table)
                        .collect(),
                ),
            },
            NodeId::Table(_) | NodeId::Error(_) => ChildState::Leaf,
        }
    }

    fn has_children(&self, id: &NodeId) -> bool {
        !matches!(id, NodeId::Table(_) | NodeId::Error(_))
    }

    fn render_row(
        &self,
        id: &NodeId,
        info: TreeRowInfo,
        _window: &mut Window,
        cx: &mut App,
    ) -> AnyElement {
        let chrome = theme(cx);
        let label = self.label_of(id);

        if let NodeId::Error(_) = id {
            return div()
                .flex()
                .flex_row()
                .items_center()
                .min_w_0()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .truncate()
                        .text_size(px(11.))
                        .text_color(chrome.danger)
                        .child(label),
                )
                .into_any_element();
        }

        // A catalog has no tick box: it stands for a set of schemas whose
        // tables nobody has fetched, so a box on it would be a promise about
        // rows that do not exist yet.
        let tick = match id {
            NodeId::Catalog(_) => None,
            NodeId::Schema(key) => {
                let members = self.visible_keys(key);
                Some(self.render_tick(id, info, tick_of(&self.selection, &members), &chrome))
            }
            NodeId::Table(key) => {
                let state = if self.selection.contains(key) {
                    Tick::On
                } else {
                    Tick::Off
                };
                Some(self.render_tick(id, info, state, &chrome))
            }
            NodeId::Error(_) => None,
        };

        let mark = match id {
            NodeId::Catalog(_) | NodeId::Schema(_) => icons::SCHEMA,
            NodeId::Table(key) => match self.table_of(key) {
                Some(table) if table.is_view() => icons::VIEW,
                _ => icons::TABLE,
            },
            NodeId::Error(_) => unreachable!("handled above"),
        };

        // The table's own comment, after the name and quieter than it: a second
        // fact about the row rather than a second row. Never translated — it is
        // the database's text.
        let remarks = match id {
            NodeId::Table(key) => self
                .table_of(key)
                .map(|table| table.remarks.clone())
                .filter(|remarks| !remarks.is_empty())
                .map(SharedString::from),
            _ => None,
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .min_w_0()
            .children(tick)
            .child(icons::icon(mark, px(14.), chrome.text_muted))
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(12.))
                    .when(info.selected, |label| label.text_color(chrome.text))
                    .child(label),
            )
            .children(remarks.map(|remarks| {
                div()
                    .id(("explorer-remarks", info.index))
                    .flex_none()
                    .max_w(px(120.))
                    .truncate()
                    .text_size(px(10.))
                    .text_color(chrome.text_muted)
                    .tooltip(tooltip_label(remarks.clone()))
                    .child(remarks)
            }))
            .into_any_element()
    }

    fn render_loading(&self, _window: &mut Window, cx: &mut App) -> AnyElement {
        div()
            .text_size(px(11.))
            .text_color(theme(cx).text_muted)
            .child(ts!("explorer.loading"))
            .into_any_element()
    }
}

/// What the panel asks the workspace for.
///
/// Every variant needs the session, and the panel has none: it is handed
/// answers and never goes looking for them (architecture.md §6, "the UI thread
/// never waits").
pub enum ExplorerEvent {
    /// The catalogs and schemas of the connection are wanted.
    LoadSchemas,
    /// The table list of one schema is wanted.
    LoadTables(SchemaKey),
    /// The set of ticked tables changed; the status bar counts it.
    SelectionChanged,
    /// This table should be what the inspector describes.
    Inspect {
        /// Which table.
        table: TableKey,
        /// Whether the panel should be put back on screen if it is hidden.
        ///
        /// The menu row says yes, because it was asked for by name. Moving the
        /// tree's cursor says no: an arrow key that reopened a panel the user
        /// had put away would be the shell arguing with them.
        reveal: bool,
    },
}

/// The sidebar.
pub struct Explorer {
    tree: Entity<TreeView<ExplorerSource>>,
    /// The filter box. jdbgen's, in the same place and with the same rule.
    filter: Entity<TextInput>,
    /// The right-click menu, while one is open, and where it hangs from.
    menu: Option<(NodeId, Point<Pixels>)>,
    focus_handle: FocusHandle,
    /// Keeps the tree's subscription alive.
    _events: Subscription,
    /// Redraws the tree when the filter text changes.
    _filter_events: Subscription,
}

impl Explorer {
    /// Builds the panel around an empty tree.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let host = cx.weak_entity();
        let tree = cx.new(|cx| {
            TreeView::new(ExplorerSource::new(host), cx)
                .with_arrow_icons(icons::CHEVRON_RIGHT, icons::CHEVRON_DOWN)
        });

        let events = cx.subscribe(&tree, |explorer, _tree, event, cx| match event {
            // The widget deduplicates these, so a node is asked for once
            // however many times it is redrawn.
            TreeEvent::LoadChildren(None) => explorer.request_schemas(cx),
            TreeEvent::LoadChildren(Some(NodeId::Schema(key))) => {
                explorer.request_tables(key.clone(), cx);
            }
            // A catalog's schemas came with the root fetch; there is nothing to
            // go and get.
            TreeEvent::LoadChildren(Some(_)) => {}
            // `Enter` and `Space` on a table row. The keyboard's way to a tick,
            // which is the one thing on this panel a pointer can do that a
            // keyboard otherwise could not.
            TreeEvent::Activated(node) => explorer.toggle_tick(node, cx),
            // A click, or an arrow key. Pointing the inspector at a row is what
            // *selecting* it means here — there is nothing else a table row
            // could be selected for — so it follows the highlight rather than
            // waiting for a second gesture.
            TreeEvent::SelectionChanged(Some(NodeId::Table(key))) => {
                cx.emit(ExplorerEvent::Inspect {
                    table: key.clone(),
                    reveal: false,
                });
            }
            TreeEvent::SelectionChanged(_) => {}
            TreeEvent::ContextMenu { id, position } => {
                explorer.menu = Some((id.clone(), *position));
                cx.notify();
            }
        });

        let filter = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("explorer.filter_placeholder"))
                .tab_index(0)
        });
        // The box owns the text and the tree owns the shape, so the one has to
        // tell the other. Every keystroke, deliberately: a filter that waited
        // for `Enter` would be a search box, and this is a narrowing.
        let filter_events = cx.observe(&filter, |explorer, input, cx| {
            let text = input.read(cx).content().to_string();
            explorer.set_filter(text, cx);
        });

        Self {
            tree,
            filter,
            menu: None,
            focus_handle: cx.focus_handle(),
            _events: events,
            _filter_events: filter_events,
        }
    }

    /// Runs `edit` against the tree's source and redraws.
    fn update_source(&mut self, cx: &mut Context<Self>, edit: impl FnOnce(&mut ExplorerSource)) {
        self.tree.update(cx, |tree, cx| edit(tree.source_mut(cx)));
        cx.notify();
    }

    /// Marks the root as loading and asks the workspace for the schemas.
    fn request_schemas(&mut self, cx: &mut Context<Self>) {
        self.update_source(cx, |source| source.schemas = Fetch::Loading);
        cx.emit(ExplorerEvent::LoadSchemas);
    }

    /// Marks a schema as loading and asks the workspace for its tables.
    fn request_tables(&mut self, key: SchemaKey, cx: &mut Context<Self>) {
        let node = key.clone();
        self.update_source(cx, move |source| {
            source.tables.insert(node, Fetch::Loading);
        });
        cx.emit(ExplorerEvent::LoadTables(key));
    }

    /// Records what the schema fetch produced, or why it failed.
    pub fn deliver_schemas(
        &mut self,
        outcome: Result<Vec<Schema>, SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.update_source(cx, |source| {
            source.schemas = match outcome {
                Ok(schemas) => Fetch::Loaded(schemas),
                Err(message) => Fetch::Failed(message),
            };
        });
    }

    /// Records what one schema's table fetch produced, or why it failed.
    pub fn deliver_tables(
        &mut self,
        key: SchemaKey,
        outcome: Result<Vec<TableRef>, SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.update_source(cx, |source| source.set_tables(key, outcome));
    }

    /// Everything the last connection put here, gone.
    ///
    /// The selection with it: a tick names a table of *this* database, and
    /// carrying one across a reconnection to a different server would be a set
    /// of names that happen to match.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.update_source(cx, ExplorerSource::clear);
        self.filter.update(cx, |input, cx| input.clear(cx));
        cx.emit(ExplorerEvent::SelectionChanged);
    }

    /// Asks the database again, keeping the ticks and the filter.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.menu = None;
        self.update_source(cx, ExplorerSource::invalidate);
    }

    /// The ticked tables, in generation order.
    ///
    /// What M3's run walks. Nothing reads it yet — the status bar counts the
    /// set rather than listing it — but it is the panel's whole output, and a
    /// selection that could only be counted would not be one.
    #[allow(dead_code)]
    pub fn selection(&self, cx: &App) -> BTreeSet<TableKey> {
        self.tree.read(cx).source().selection.clone()
    }

    /// How many tables are ticked.
    pub fn selected_count(&self, cx: &App) -> usize {
        self.tree.read(cx).source().selection.len()
    }

    /// Whether views are drawn.
    pub fn show_views(&self, cx: &App) -> bool {
        self.tree.read(cx).source().show_views
    }

    /// Shows or hides views.
    ///
    /// Costs no round trip: the cache already holds them (see the module
    /// documentation). The ticks of the views being hidden stay — hiding a row
    /// is not unticking it.
    pub fn set_show_views(&mut self, show: bool, cx: &mut Context<Self>) {
        self.update_source(cx, |source| source.show_views = show);
    }

    /// Narrows the tree to the rows whose names contain `text`.
    fn set_filter(&mut self, text: String, cx: &mut Context<Self>) {
        if self.tree.read(cx).source().filter == text {
            return;
        }
        self.update_source(cx, |source| source.filter = text);
    }

    /// The row the table list came back with, for `key`.
    ///
    /// The whole [`TableRef`] and not just the name, because three of its
    /// fields — the kind, the comment and the position — are not in a
    /// [`TableKey`] and are not read a second time:
    /// [`rudbgen_meta::MetaReader::table`] copies them straight off what it is
    /// handed. Building one from a key alone would give the inspector a view
    /// that calls itself a table and a commented table with no comment.
    pub fn table_ref(&self, key: &TableKey, cx: &App) -> Option<TableRef> {
        self.tree.read(cx).source().table_of(key).cloned()
    }

    /// The tables the tree is showing right now, top to bottom.
    ///
    /// Read off the flattened row list rather than recomputed, so that "all
    /// shown" means exactly the rows the user can see: a schema nobody has
    /// opened has tables, and none of them is on screen.
    pub fn visible_tables(&self, cx: &App) -> Vec<TableKey> {
        self.tree
            .read(cx)
            .rows()
            .iter()
            .filter_map(|row| match row {
                TreeRow::Node {
                    id: NodeId::Table(key),
                    ..
                } => Some(key.clone()),
                _ => None,
            })
            .collect()
    }

    /// Every table name the tree has fetched, sorted and without repeats.
    ///
    /// The whole-name picker of the abbreviation rules editor (§4.6): a rule
    /// that replaces a whole identifier is written against a table that exists,
    /// and typing the name back by hand is where the typo comes from. Names
    /// rather than keys, because that is what the engine matches — a rule knows
    /// nothing of catalogs and schemas — which is also why two schemas holding
    /// an `ORDERS` each offer one entry and not two.
    ///
    /// The whole index, not the visible rows: the filter box and the *views*
    /// toggle are about what the run is over, and neither has anything to say
    /// about which names a rule may mention.
    pub fn loaded_table_names(&self, cx: &App) -> Vec<SharedString> {
        let mut names: Vec<SharedString> = self
            .tree
            .read(cx)
            .source()
            .table_names()
            .into_iter()
            .map(SharedString::from)
            .collect();
        names.sort();
        names.dedup();
        names
    }

    /// Ticks or unticks what a row's box stands for.
    ///
    /// A table is itself; a schema is every table of it that is on screen, and
    /// a schema that is partly ticked fills up rather than emptying — the
    /// gesture that finishes a job is the more likely one.
    pub fn toggle_tick(&mut self, id: &NodeId, cx: &mut Context<Self>) {
        let members = match id {
            NodeId::Table(key) => vec![key.clone()],
            NodeId::Schema(key) => self.tree.read(cx).source().visible_keys(key),
            NodeId::Catalog(_) | NodeId::Error(_) => return,
        };
        if members.is_empty() {
            return;
        }
        let state = tick_of(&self.tree.read(cx).source().selection, &members);
        self.update_source(cx, |source| match state {
            Tick::On => deselect_all(&mut source.selection, &members),
            Tick::Off | Tick::Partial => select_all(&mut source.selection, &members),
        });
        cx.emit(ExplorerEvent::SelectionChanged);
    }

    /// Runs `edit` over the selection and tells the status bar.
    fn edit_selection(
        &mut self,
        cx: &mut Context<Self>,
        edit: impl FnOnce(&mut BTreeSet<TableKey>),
    ) {
        self.menu = None;
        self.update_source(cx, |source| edit(&mut source.selection));
        cx.emit(ExplorerEvent::SelectionChanged);
    }

    /// The rows of the right-click menu over `id`.
    ///
    /// Built as data rather than as widget entries so that what a row offers is
    /// what a test reads; see [`crate::context_menu`].
    pub fn menu_rows(&self, id: &NodeId, cx: &mut Context<Self>) -> Vec<MenuRow> {
        let visible = self.visible_tables(cx);
        let this = cx.entity();
        let mut rows = Vec::new();

        rows.push({
            let this = this.clone();
            let members = visible.clone();
            MenuRow::new(ts!("explorer.menu_select_shown"))
                .enabled(!members.is_empty())
                .on_activate(move |_window, cx| {
                    this.update(cx, |explorer, cx| {
                        explorer.edit_selection(cx, |selection| select_all(selection, &members));
                    });
                })
        });
        rows.push({
            let this = this.clone();
            MenuRow::new(ts!("explorer.menu_clear"))
                .enabled(self.selected_count(cx) > 0)
                .on_activate(move |_window, cx| {
                    this.update(cx, |explorer, cx| {
                        explorer.edit_selection(cx, BTreeSet::clear);
                    });
                })
        });
        rows.push({
            let this = this.clone();
            let members = visible;
            MenuRow::new(ts!("explorer.menu_invert"))
                .enabled(!members.is_empty())
                .on_activate(move |_window, cx| {
                    this.update(cx, |explorer, cx| {
                        explorer.edit_selection(cx, |selection| invert(selection, &members));
                    });
                })
        });

        rows.push(MenuRow::separator());

        // Greyed on a schema rather than left out: the row is about the panel's
        // other half and belongs in every menu the panel draws, so that where
        // it is does not move with what was clicked.
        rows.push({
            let this = this.clone();
            let target = match id {
                NodeId::Table(key) => Some(key.clone()),
                _ => None,
            };
            MenuRow::new(ts!("explorer.menu_inspect"))
                .enabled(target.is_some())
                .on_activate(move |_window, cx| {
                    let Some(target) = target.clone() else {
                        return;
                    };
                    this.update(cx, |explorer, cx| {
                        explorer.menu = None;
                        cx.emit(ExplorerEvent::Inspect {
                            table: target,
                            reveal: true,
                        });
                        cx.notify();
                    });
                })
        });
        rows.push({
            MenuRow::new(ts!("explorer.menu_refresh")).on_activate(move |_window, cx| {
                this.update(cx, |explorer, cx| explorer.refresh(cx));
            })
        });

        rows
    }

    /// Closes the right-click menu, and says whether there was one.
    pub fn close_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let had = self.menu.take().is_some();
        if had {
            cx.notify();
        }
        had
    }

    /// Moves the tree's selection, for the shell's own tests.
    #[cfg(test)]
    pub fn select(&mut self, id: NodeId, cx: &mut Context<Self>) {
        self.tree
            .update(cx, |tree, cx| tree.set_selected(Some(id), cx));
    }

    /// Opens a node, for the shell's own tests.
    #[cfg(test)]
    pub fn expand(&mut self, id: &NodeId, cx: &mut Context<Self>) {
        self.tree.update(cx, |tree, cx| tree.expand(id, cx));
    }

    /// The rows on screen, for the shell's own tests.
    #[cfg(test)]
    pub fn row_ids(&self, cx: &App) -> Vec<Option<NodeId>> {
        self.tree
            .read(cx)
            .rows()
            .iter()
            .map(|row| row.id().cloned())
            .collect()
    }

    /// The reason the schema fetch failed, when it did.
    fn root_failure(&self, cx: &App) -> Option<SharedString> {
        match &self.tree.read(cx).source().schemas {
            Fetch::Failed(message) => Some(message.clone()),
            _ => None,
        }
    }

    /// The filter box and the views toggle.
    fn render_header(&self, chrome: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let this = cx.entity();
        let show_views = self.show_views(cx);

        div()
            .flex()
            .flex_col()
            .flex_none()
            .gap(px(6.))
            .px(px(8.))
            .py(px(6.))
            .border_b_1()
            .border_color(chrome.border)
            .child(self.filter.clone())
            .child(
                rudbgen_ui::Checkbox::new("explorer-views", ts!("explorer.show_views"))
                    .checked(show_views)
                    .tab_index(1)
                    .on_toggle(move |checked, _window, cx| {
                        this.update(cx, |explorer, cx| explorer.set_show_views(checked, cx));
                    }),
            )
            .into_any_element()
    }
}

impl EventEmitter<ExplorerEvent> for Explorer {}

impl Focusable for Explorer {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for Explorer {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let chrome = theme(cx);
        let header = self.render_header(&chrome, cx);
        let failure = self.root_failure(cx);
        let menu = self.menu.clone().map(|(id, position)| {
            let this = cx.entity();
            let rows = self.menu_rows(&id, cx);
            rudbgen_ui::ContextMenu::new("explorer-context")
                .position(position)
                .entries(context_menu::entries(rows))
                .on_dismiss(move |_window, cx| {
                    this.update(cx, |explorer, cx| {
                        explorer.close_menu(cx);
                    });
                })
        });

        div()
            .id("explorer")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .size_full()
            .min_w_0()
            .min_h_0()
            // Tinted, because this is the only fill over the sidebar: the
            // body's own stops at the work area beside it. Left untinted, the
            // blur behind the window would stop dead at the sidebar's edge; see
            // [`app_settings::window_tint`].
            .bg(app_settings::window_tint(chrome.surface, cx))
            .border_r_1()
            .border_color(chrome.border)
            .child(
                div()
                    .flex()
                    .flex_none()
                    .items_center()
                    .h(px(HEADER_HEIGHT))
                    .px(px(10.))
                    .text_size(px(10.))
                    .text_color(chrome.text_muted)
                    .child(ts!("explorer.title")),
            )
            .child(header)
            .child(match failure {
                // In place of the tree, not under it: a tree drawing nothing
                // beside a message saying why is one state, and it reads as one.
                Some(message) => div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .items_center()
                    .justify_center()
                    .px(px(12.))
                    .text_size(px(11.))
                    .text_color(chrome.danger)
                    .child(message)
                    .into_any_element(),
                None => div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(self.tree.clone())
                    .into_any_element(),
            })
            .children(menu)
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use gpui::{TestAppContext, VisualTestContext};

    use super::*;

    fn table(name: &str) -> TableRef {
        TableRef {
            catalog: String::new(),
            schema: "PUBLIC".to_string(),
            name: name.to_string(),
            kind: rudbgen_meta::KIND_TABLE.to_string(),
            remarks: String::new(),
            no: 0,
        }
    }

    fn view(name: &str) -> TableRef {
        TableRef {
            kind: rudbgen_meta::KIND_VIEW.to_string(),
            ..table(name)
        }
    }

    fn key(name: &str) -> TableKey {
        TableKey {
            catalog: String::new(),
            schema: "PUBLIC".to_string(),
            name: name.to_string(),
        }
    }

    fn schema(catalog: &str, name: &str) -> Schema {
        Schema {
            catalog: catalog.to_string(),
            schema: name.to_string(),
            name: name.to_string(),
        }
    }

    /// A view that does nothing but hold the panel, as the workspace does.
    struct Harness {
        explorer: Entity<Explorer>,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div().size_full().child(self.explorer.clone())
        }
    }

    /// What a test reads back off [`ExplorerEvent`].
    ///
    /// The event carries a `TableKey` and a `SchemaKey`, neither of which a
    /// test wants to spell out in an assertion; this is the shape of the
    /// announcement rather than its payload.
    #[derive(Clone, Debug, PartialEq, Eq)]
    enum Announced {
        Schemas,
        Tables(String),
        Selection,
        Inspect(String, bool),
    }

    /// Opens the panel in a window and hands back it and what it announced.
    ///
    /// A real window, because the row list is rebuilt on a draw: the tree's
    /// shape is a function of what has been drawn, and asserting on it without
    /// laying anything out would be asserting about a state the panel never has
    /// on screen.
    fn open(cx: &mut TestAppContext) -> (Entity<Explorer>, Recorder, VisualTestContext) {
        cx.update(rudbgen_ui::init);

        let events: Recorder = Rc::new(RefCell::new(Vec::new()));
        let window = cx.add_window({
            let events = events.clone();
            move |_window, cx| {
                let explorer = cx.new(Explorer::new);
                cx.subscribe(
                    &explorer,
                    move |_: &mut Harness, _, event: &ExplorerEvent, _| {
                        events.borrow_mut().push(match event {
                            ExplorerEvent::LoadSchemas => Announced::Schemas,
                            ExplorerEvent::LoadTables(key) => Announced::Tables(key.name.clone()),
                            ExplorerEvent::SelectionChanged => Announced::Selection,
                            ExplorerEvent::Inspect { table, reveal } => {
                                Announced::Inspect(table.name.clone(), *reveal)
                            }
                        });
                    },
                )
                .detach();
                Harness { explorer }
            }
        });
        let explorer = window
            .update(cx, |harness, _, _| harness.explorer.clone())
            .expect("the window is open");
        let cx = VisualTestContext::from_window(*std::ops::Deref::deref(&window), cx);
        cx.run_until_parked();
        (explorer, events, cx)
    }

    /// Everything the panel has announced since the last look.
    type Recorder = Rc<RefCell<Vec<Announced>>>;

    /// Drains the recorder.
    fn drain(events: &Recorder) -> Vec<Announced> {
        events.borrow_mut().drain(..).collect()
    }

    /// The rows on screen, by the name each one draws.
    fn rows(explorer: &Entity<Explorer>, cx: &mut VisualTestContext) -> Vec<String> {
        cx.update(|_, cx| {
            explorer
                .read(cx)
                .row_ids(cx)
                .into_iter()
                .map(|id| match id {
                    Some(NodeId::Catalog(name)) => name,
                    Some(NodeId::Schema(key)) => key.name,
                    Some(NodeId::Table(key)) => key.name,
                    Some(NodeId::Error(key)) => format!("!{}", key.name),
                    None => "…".to_string(),
                })
                .collect()
        })
    }

    /// The schema every fixture below hangs off.
    fn public() -> SchemaKey {
        SchemaKey::of(&schema("", "PUBLIC"))
    }

    /// Fills the tree with one schema holding two tables and a view.
    fn fill(explorer: &Entity<Explorer>, cx: &mut VisualTestContext) {
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.deliver_schemas(Ok(vec![schema("", "PUBLIC")]), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.expand(&NodeId::Schema(public()), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.deliver_tables(
                    public(),
                    Ok(vec![table("T_ALBUM"), table("T_ARTIST"), view("V_ALBUM")]),
                    cx,
                );
            });
        });
        cx.run_until_parked();
    }

    #[gpui::test]
    fn the_tree_asks_for_its_schemas_and_then_for_the_tables_of_one(cx: &mut TestAppContext) {
        let (explorer, events, mut cx) = open(cx);
        // The first draw asks for the root, and draws a placeholder meanwhile.
        assert_eq!(drain(&events), vec![Announced::Schemas]);
        assert_eq!(rows(&explorer, &mut cx), vec!["…"]);

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.deliver_schemas(Ok(vec![schema("", "PUBLIC")]), cx);
            });
        });
        cx.run_until_parked();
        // One catalog costs no level of its own.
        assert_eq!(rows(&explorer, &mut cx), vec!["PUBLIC"]);
        assert!(drain(&events).is_empty(), "a closed schema fetched nothing");

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.expand(&NodeId::Schema(public()), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(
            drain(&events),
            vec![Announced::Tables("PUBLIC".to_string())]
        );

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.deliver_tables(public(), Ok(vec![table("T_ALBUM")]), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(rows(&explorer, &mut cx), vec!["PUBLIC", "T_ALBUM"]);
    }

    #[gpui::test]
    fn a_schema_that_cannot_be_read_gets_one_row_saying_so(cx: &mut TestAppContext) {
        let (explorer, _events, mut cx) = open(cx);
        // One step per frame, because that is the order the shell answers in:
        // the request the expansion emits reaches the workspace after the draw
        // that raised it, and the answer after that.
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.deliver_schemas(Ok(vec![schema("", "PUBLIC")]), cx);
                explorer.expand(&NodeId::Schema(public()), cx);
            });
        });
        cx.run_until_parked();
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.deliver_tables(public(), Err("no such schema".into()), cx);
            });
        });
        cx.run_until_parked();
        // The driver's own sentence, in place of the tables, and the tree is
        // still standing.
        assert_eq!(rows(&explorer, &mut cx), vec!["PUBLIC", "!PUBLIC"]);
    }

    #[gpui::test]
    fn a_tick_outlives_the_filter_the_toggle_and_a_collapse(cx: &mut TestAppContext) {
        let (explorer, events, mut cx) = open(cx);
        fill(&explorer, &mut cx);
        drain(&events);
        // Views are off by default, so the view is not a row.
        assert_eq!(
            rows(&explorer, &mut cx),
            vec!["PUBLIC", "T_ALBUM", "T_ARTIST"]
        );

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&NodeId::Table(key("T_ALBUM")), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(drain(&events), vec![Announced::Selection]);
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 1);

        // Everything that changes what is on screen, and none of it changes
        // what is ticked.
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.set_filter("artist".to_string(), cx);
                explorer.set_show_views(true, cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(rows(&explorer, &mut cx), vec!["PUBLIC", "T_ARTIST"]);
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 1);

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.set_filter(String::new(), cx);
                explorer.refresh(cx);
            });
        });
        cx.run_until_parked();
        // A refresh throws the metadata away and asks again — and keeps the
        // ticks, because the user did not change their mind.
        assert_eq!(drain(&events), vec![Announced::Schemas]);
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 1);
    }

    #[gpui::test]
    fn a_schemas_box_ticks_what_is_on_screen_and_nothing_else(cx: &mut TestAppContext) {
        let (explorer, _events, mut cx) = open(cx);
        fill(&explorer, &mut cx);
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.set_filter("album".to_string(), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(rows(&explorer, &mut cx), vec!["PUBLIC", "T_ALBUM"]);

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&NodeId::Schema(public()), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(
            cx.update(|_, cx| explorer.read(cx).selection(cx)),
            [key("T_ALBUM")].into_iter().collect::<BTreeSet<_>>(),
            "the hidden table was ticked by a box that could not see it"
        );

        // And a second press on a full box empties it again.
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&NodeId::Schema(public()), cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 0);
    }

    #[gpui::test]
    fn moving_the_cursor_points_the_inspector_at_the_row_under_it(cx: &mut TestAppContext) {
        let (explorer, events, mut cx) = open(cx);
        fill(&explorer, &mut cx);
        drain(&events);

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.select(NodeId::Table(key("T_ARTIST")), cx);
            });
        });
        cx.run_until_parked();
        // `reveal` is false: an arrow key must not reopen a panel the user put
        // away. Only the menu row asks for that.
        assert_eq!(
            drain(&events),
            vec![Announced::Inspect("T_ARTIST".to_string(), false)]
        );

        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.select(NodeId::Schema(public()), cx);
            });
        });
        cx.run_until_parked();
        assert!(
            drain(&events).is_empty(),
            "a schema row is not a table and has nothing to inspect"
        );
    }

    #[gpui::test]
    fn a_new_connection_takes_the_ticks_with_it(cx: &mut TestAppContext) {
        let (explorer, events, mut cx) = open(cx);
        fill(&explorer, &mut cx);
        cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                explorer.toggle_tick(&NodeId::Table(key("T_ALBUM")), cx);
                explorer.reset(cx);
            });
        });
        cx.run_until_parked();
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 0);
        // And the tree asks the new session for its schemas.
        assert!(drain(&events).contains(&Announced::Schemas));
    }

    #[gpui::test]
    fn the_menu_acts_on_the_rows_on_screen(cx: &mut TestAppContext) {
        let (explorer, events, mut cx) = open(cx);
        fill(&explorer, &mut cx);
        drain(&events);

        let labels = cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                context_menu::labels(&explorer.menu_rows(&NodeId::Table(key("T_ALBUM")), cx))
            })
        });
        assert_eq!(
            labels,
            vec![
                ts!("explorer.menu_select_shown").to_string(),
                ts!("explorer.menu_clear").to_string(),
                ts!("explorer.menu_invert").to_string(),
                String::new(),
                ts!("explorer.menu_inspect").to_string(),
                ts!("explorer.menu_refresh").to_string(),
            ]
        );

        // "Open in inspector" is the one row that needs a table under the
        // pointer, and it says so rather than doing nothing. "Clear selection"
        // is greyed too, because nothing is ticked yet — which is the other
        // half of the rule and not an accident of this menu.
        let greyed = cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                context_menu::greyed(&explorer.menu_rows(&NodeId::Schema(public()), cx))
            })
        });
        assert_eq!(
            greyed,
            vec![
                ts!("explorer.menu_clear").to_string(),
                ts!("explorer.menu_inspect").to_string(),
            ]
        );
        // On a table row the inspector command is live.
        let greyed = cx.update(|_, cx| {
            explorer.update(cx, |explorer, cx| {
                context_menu::greyed(&explorer.menu_rows(&NodeId::Table(key("T_ALBUM")), cx))
            })
        });
        assert_eq!(greyed, vec![ts!("explorer.menu_clear").to_string()]);

        // Built inside the panel's own update and run outside it, which is the
        // order the shell has: the rows are made while the menu is drawn and
        // pressed a frame or more later.
        let run = |label: SharedString, cx: &mut VisualTestContext| {
            let rows = cx.update(|_, cx| {
                explorer.update(cx, |explorer, cx| {
                    explorer.menu_rows(&NodeId::Table(key("T_ALBUM")), cx)
                })
            });
            cx.update(|window, cx| context_menu::row(&rows, &label).activate(window, cx));
            cx.run_until_parked();
        };

        run(ts!("explorer.menu_select_shown"), &mut cx);
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 2);

        run(ts!("explorer.menu_invert"), &mut cx);
        assert_eq!(cx.update(|_, cx| explorer.read(cx).selected_count(cx)), 0);

        // The inspector row is the one that asks for the panel to be shown.
        run(ts!("explorer.menu_inspect"), &mut cx);
        assert!(
            drain(&events).contains(&Announced::Inspect("T_ALBUM".to_string(), true)),
            "the menu row did not ask for the panel"
        );
    }

    #[test]
    fn the_filter_is_contains_and_ignores_case() {
        // jdbgen's `filterTables`, which is what a user's habits were built on.
        assert!(matches_filter("T_SAMPLE_ALBUM", "sample"));
        assert!(matches_filter("T_SAMPLE_ALBUM", "SAMPLE"));
        assert!(matches_filter("T_SAMPLE_ALBUM", "album"));
        assert!(!matches_filter("T_SAMPLE_ALBUM", "artist"));
        // Not a glob: a star is a character like any other.
        assert!(!matches_filter("T_SAMPLE_ALBUM", "T_*"));
    }

    #[test]
    fn an_empty_filter_keeps_everything() {
        assert!(matches_filter("ANYTHING", ""));
        assert!(matches_filter("ANYTHING", "   "));
    }

    #[test]
    fn the_filter_and_the_views_toggle_narrow_the_same_list() {
        let tables = vec![table("T_ALBUM"), view("V_ALBUM"), table("T_ARTIST")];

        let names = |filter: &str, views: bool| {
            visible_tables(&tables, filter, views)
                .into_iter()
                .map(|table| table.name.clone())
                .collect::<Vec<_>>()
        };

        assert_eq!(names("", false), vec!["T_ALBUM", "T_ARTIST"]);
        assert_eq!(names("", true), vec!["T_ALBUM", "V_ALBUM", "T_ARTIST"]);
        assert_eq!(names("album", true), vec!["T_ALBUM", "V_ALBUM"]);
        assert_eq!(names("album", false), vec!["T_ALBUM"]);
    }

    #[test]
    fn a_tick_survives_the_filter_that_hides_its_row() {
        // The whole reason the selection is a set: narrowing the list is not
        // changing one's mind about what it holds.
        let mut selection = BTreeSet::new();
        selection.insert(key("T_ALBUM"));
        let tables = vec![table("T_ALBUM"), table("T_ARTIST")];

        let shown: Vec<TableKey> = visible_tables(&tables, "artist", false)
            .into_iter()
            .map(TableKey::of)
            .collect();
        assert_eq!(shown, vec![key("T_ARTIST")]);
        assert!(selection.contains(&key("T_ALBUM")));

        // And an invert over what is shown leaves it alone.
        invert(&mut selection, &shown);
        assert_eq!(
            selection.iter().cloned().collect::<Vec<_>>(),
            vec![key("T_ALBUM"), key("T_ARTIST")]
        );
    }

    #[test]
    fn a_group_is_ticked_wholly_partly_or_not_at_all() {
        let members = vec![key("A"), key("B")];
        let mut selection = BTreeSet::new();
        assert_eq!(tick_of(&selection, &members), Tick::Off);

        selection.insert(key("A"));
        assert_eq!(tick_of(&selection, &members), Tick::Partial);

        selection.insert(key("B"));
        assert_eq!(tick_of(&selection, &members), Tick::On);

        // An empty group is not "all of nothing": a schema with no rows on
        // screen has nothing ticked.
        assert_eq!(tick_of(&selection, &[]), Tick::Off);
    }

    #[test]
    fn the_three_selection_commands_act_only_on_what_is_shown() {
        let shown = vec![key("A"), key("B")];
        let mut selection = BTreeSet::new();
        selection.insert(key("HIDDEN"));

        select_all(&mut selection, &shown);
        assert_eq!(selection.len(), 3);

        invert(&mut selection, &shown);
        assert_eq!(
            selection.iter().cloned().collect::<Vec<_>>(),
            vec![key("HIDDEN")]
        );

        select_all(&mut selection, &shown);
        deselect_all(&mut selection, &shown);
        assert_eq!(
            selection.iter().cloned().collect::<Vec<_>>(),
            vec![key("HIDDEN")]
        );
    }

    #[test]
    fn one_catalog_costs_no_level_and_two_do() {
        assert!(!catalog_level(&[
            schema("APP", "PUBLIC"),
            schema("APP", "SYS")
        ]));
        assert!(catalog_level(&[
            schema("APP", "PUBLIC"),
            schema("OTHER", "PUBLIC")
        ]));
        // A product with no catalogs at all reports one empty name.
        assert!(!catalog_level(&[schema("", "PUBLIC")]));
        assert_eq!(
            catalogs_of(&[schema("B", "S"), schema("A", "S"), schema("B", "T")]),
            vec!["B".to_string(), "A".to_string()]
        );
    }

    #[test]
    fn a_qualified_name_leaves_out_the_parts_the_product_has_not_got() {
        assert_eq!(key("T_ALBUM").qualified(), "PUBLIC.T_ALBUM");
        assert_eq!(
            TableKey {
                catalog: "app".to_string(),
                schema: String::new(),
                name: "orders".to_string(),
            }
            .qualified(),
            "app.orders"
        );
    }

    #[test]
    fn every_word_the_explorer_draws_is_translated() {
        for label in [
            ts!("explorer.title"),
            ts!("explorer.filter_placeholder"),
            ts!("explorer.show_views"),
            ts!("explorer.loading"),
            ts!("explorer.load_failed_unknown"),
            ts!("explorer.menu_select_shown"),
            ts!("explorer.menu_clear"),
            ts!("explorer.menu_invert"),
            ts!("explorer.menu_inspect"),
            ts!("explorer.menu_refresh"),
        ] {
            assert!(!label.is_empty(), "empty label");
            assert!(
                !label.starts_with("explorer."),
                "untranslated label {label:?}"
            );
        }
    }
}
