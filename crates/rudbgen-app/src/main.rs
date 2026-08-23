//! rudbgen — a database code generator.
//!
//! Point it at a database, tick the tables, tick the templates, and it writes
//! one file per table per template. This crate is the binary: the window, the
//! chrome around it, the dialogs that hang over it, and the bootstrap sequence
//! that gets all three on screen. Everything it draws with comes from
//! `rudbgen-ui`; everything it persists goes through `rudbgen-core`.
//!
//! # The window
//!
//! One window, and no modal in front of it (architecture document, D6). The
//! shell is three bands — a title bar, a body and a status bar — and the body
//! is the welcome screen for as long as no connection is open. The explorer and
//! the inspector are not merely empty then; they are out of the frame
//! altogether, because a sidebar with nothing in it is a promise the window
//! cannot keep.
//!
//! # The session
//!
//! The window owns exactly one, as [`ConnectionState`]: idle, opening, open, or
//! failed with the reason still on the status bar. Everything that blocks —
//! the keychain, the tunnel, `JNI_CreateJavaVM` on the first connection of the
//! run, `OPEN_SESSION` — happens on a background task, and the JVM is started
//! by the first connection rather than at start-up, so a user who never opens
//! one never pays for a Java runtime (architecture document, §4.1). A failure
//! is reported on the status bar and on the welcome screen rather than in a box
//! to dismiss: the user asked for a database, and the message has to stay
//! readable while they open the dialog to fix what it names.
//!
//! # What is here, and what is not
//!
//! The shell, the dialogs and the connection behind them. The welcome screen's
//! saved rows open a session, `Ctrl+N` opens the connection dialog, and the
//! driver editor inside it edits the four custom queries and tests them (D9).
//! What a connected window then shows is a placeholder: the explorer and the
//! inspector arrive with the metadata reader, the Generate tab and the work
//! area with the generation job, the template editor after that, and the jdbgen
//! import last. The two welcome buttons those milestones belong to are drawn
//! disabled with a tooltip that says so, rather than left out — a way in that
//! is missing tells the reader nothing about what the application will do, and
//! a button that looks live and does nothing is worse than either.

mod about_dialog;
mod app_settings;
mod caption;
mod connection;
mod connection_dialog;
// The explorer's rows press this; the work area's tabs and the template list
// (M3) are the call sites still to come, so a builder or two — a shortcut hint,
// a tick — has no caller yet and reads as dead code inside a binary crate.
#[allow(dead_code)]
mod context_menu;
mod driver_manager;
mod explorer;
mod i18n;
mod icons;
mod inspector;
mod maven;
// The pane tree is written as a self-contained data structure with its own
// tests rather than for the call sites the shell currently has, so it offers
// operations nothing reaches yet — merging a subtree, editing a payload — which
// inside a binary crate read as dead code. The work area it is the layout of
// arrives in M3.
#[allow(dead_code)]
mod pane_tree;
mod settings_dialog;
mod theme_editor;
mod update;
mod update_dialog;

// Compiles `locales/*.yml` into the binary and defines the machinery `t!`
// expands to, which is why it has to sit in the crate root. `fallback = "en"`
// is per key, not per locale: a string a translator has not got to yet shows
// in English while the rest of that language stays translated.
rust_i18n::i18n!("locales", fallback = "en");

use gpui::{
    AnyElement, App, Bounds, Context, Div, DragMoveEvent, Entity, FocusHandle, Hsla, KeyBinding,
    Menu, MenuItem, MouseButton, MouseUpEvent, Pixels, Point, QuitMode, ScrollHandle, SharedString,
    Stateful, Subscription, Task, TitlebarOptions, Window, WindowBackgroundAppearance,
    WindowBounds, WindowControlArea, WindowOptions, actions, div, img, prelude::*, px, size,
};
use rudbgen_core::{
    AppSettings, ConnectionProfile, ConnectionStore, DriverDef, DriverStore, TitlebarStyle,
    WindowState,
};
use rudbgen_meta::MetaReader;
use rudbgen_ui::{
    Button, ButtonVariant, DraggedThumb, EditorThemeEntry, EditorThemeRegistry, MenuButton,
    MenuEntry, Scrollbar, ScrollbarAxis, ScrollbarState, Select, Theme, ThemeRegistry,
    WindowControlIcons, WindowControls, hide_later, hide_now, scroll_to, scrolled,
    set_editor_theme, set_theme, set_window_tint, theme, theme_store, tooltip_label,
    window_controls,
};

use about_dialog::{AboutDialog, AboutDialogEvent};
use app_settings::WindowGeometry;
use caption::apply_caption_theme;
use connection::{ConnectError, Connected, Credentials, SessionHandle};
use connection_dialog::{ConnectionDialog, ConnectionDialogEvent};
use explorer::{Explorer, ExplorerEvent, SchemaKey, TableKey};
use i18n::ts;
use icons::Icons;
use inspector::{Inspector, InspectorEvent};
use settings_dialog::{SettingsDialog, SettingsDialogEvent};
use update_dialog::{UpdateDialog, UpdateDialogEvent};

actions!(
    rudbgen,
    [
        /// Leaves the application.
        Quit,
        /// Opens the connection dialog.
        NewConnection,
        /// Opens the settings dialog.
        OpenSettings,
        /// Opens the about box.
        ShowAbout,
        /// Asks GitHub whether there is a newer release.
        CheckUpdates,
        /// Closes whatever overlay is on top, innermost first.
        DismissDialog,
        /// Shows and hides the explorer sidebar.
        ToggleExplorer,
        /// Shows and hides the inspector panel.
        ToggleInspector,
    ]
);

/// Key context the shell's own shortcuts are scoped to.
const KEY_CONTEXT: &str = "Workspace";

/// Height of the title bar row.
///
/// The same height rudbman's toolbar takes, and for the same reason: it has to
/// hold a control of the [`Select`] trigger's height with a margin either side.
const TOOLBAR_HEIGHT: f32 = 36.;

/// Height of the status bar along the bottom of the window.
const STATUS_BAR_HEIGHT: f32 = 24.;

/// Distance from the top left of the window to the top left of the macOS
/// traffic lights, in the custom title bar style.
///
/// The buttons are 14 pt tall, so half the difference to [`TOOLBAR_HEIGHT`]
/// centres them in the title bar band.
const TRAFFIC_LIGHT_ORIGIN: Point<Pixels> = Point {
    x: px(12.),
    y: px(11.),
};

/// Width kept clear at the left of the title bar for the macOS traffic lights.
///
/// Three 14 pt buttons, 20 pt apart, starting at [`TRAFFIC_LIGHT_ORIGIN`], plus
/// the same margin again after the last one.
const TRAFFIC_LIGHT_GAP: f32 = 78.;

/// The application's own name, as the window and the title bar write it.
///
/// A wordmark, so it is never translated.
const APP_NAME: &str = "rudbgen";

/// Application id published to the desktop.
///
/// Wayland compositors and X11 docks match it against a `.desktop` file of the
/// same name to pick up the application icon, so `packaging/linux` has to ship
/// `com.aihouse.rudbgen.desktop` and nothing else.
const APP_ID: &str = "com.aihouse.rudbgen";

/// Modifier key named in the shortcut hints of the dropdown menu.
///
/// Never translated: it is the name printed on the key. It follows
/// [`bind_shortcuts`] on every platform so the two never drift.
const SHORTCUT_MODIFIER: &str = if cfg!(target_os = "macos") {
    "Cmd"
} else {
    "Ctrl"
};

/// Width of the grab area between two panels of the body, in logical pixels.
///
/// Pulled back over the panel's own border so the band straddles the seam
/// rather than pushing the work area across; see [`Workspace::render_work_area`].
const SPLIT_HANDLE: f32 = 6.;

/// Narrowest the explorer may be dragged. Mirrors `rudbgen-core`'s clamp, which
/// is what the stored width is loaded through.
const MIN_EXPLORER_WIDTH: f32 = 140.;

/// Widest the explorer may be dragged.
const MAX_EXPLORER_WIDTH: f32 = 720.;

/// Narrowest the inspector may be dragged.
const MIN_INSPECTOR_WIDTH: f32 = 200.;

/// Widest the inspector may be dragged.
const MAX_INSPECTOR_WIDTH: f32 = 720.;

/// The payload of a drag of the explorer's edge.
///
/// A marker type and not a value: the width is read from where the pointer is
/// against the row's own box on every move, so there is nothing to carry.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DraggedExplorer;

/// The payload of a drag of the inspector's edge.
#[derive(Clone, Debug, PartialEq, Eq)]
struct DraggedInspector;

/// Width of the title bar's connection selector, in logical pixels.
const CONNECTION_SELECT_WIDTH: f32 = 200.;

/// Diameter of the status dot inside that selector.
const STATUS_DOT: f32 = 8.;

/// Width of the welcome screen's column, in logical pixels.
///
/// Fixed rather than fluid: wide enough for a profile's name beside its driver,
/// narrow enough to read as one card in a maximised window rather than a
/// screen-wide smear of rows.
const WELCOME_WIDTH: f32 = 340.;

/// Element id of the welcome screen's scrolling box.
const WELCOME_STATE: &str = "welcome-state";

/// Element id the welcome screen's overlay scroll indicator is drawn under.
///
/// The id lives here rather than inside the box it overlays because a drag of
/// the thumb is answered by the workspace, and the id is what tells one bar's
/// drag from any other bar's in the window — of which there is exactly one
/// today and one per pane from M3.
const WELCOME_SCROLLBAR: &str = "welcome-scrollbar";

/// Room left above and below a column that [`centered_scroll`] is scrolling.
///
/// Only ever seen once there is scrolling to do — while the column fits, the
/// automatic margins dwarf it — and there it is what keeps the first and last
/// rows off the edges of the body at either end of the travel.
const SCROLL_MARGIN: f32 = 24.;

/// Tab-ring position of the welcome screen's first button.
const WELCOME_FIRST_TAB: isize = 1;

/// Debug selector of the welcome screen's "new connection" button.
///
/// Compiled away outside a test build; it saves a test working the button's
/// position out from the centred column's layout.
const WELCOME_NEW_SELECTOR: &str = "welcome-new";

/// Reads `connections.json` for the welcome screen's list.
///
/// A store that cannot be read is logged and answered as an empty one: the
/// welcome screen would otherwise have to grow an error strip of its own for a
/// file the connection dialog already reports on, and an empty list still shows
/// the button that makes the first profile.
fn load_profiles() -> ConnectionStore {
    match ConnectionStore::load() {
        Ok(store) => store,
        Err(error) => {
            log::error!("could not read connections.json: {error:#}");
            ConnectionStore::default()
        }
    }
}

/// Whether a database session is open, opening, or has just failed to open.
///
/// One connection at a time, deliberately: everything the window shows — the
/// explorer, the Generate tab's options, the status bar's arithmetic — belongs
/// to one database, and a second session would need a second window's worth of
/// state to say anything about. Switching connections replaces this whole
/// value, which is what closes the session that was open.
enum ConnectionState {
    /// No session, and none being opened.
    Idle,
    /// A session is opening on a background task.
    Connecting {
        /// What is being opened, for the label and the retry.
        profile: Box<ConnectionProfile>,
        /// The driver definition it is being opened through.
        ///
        /// Carried from here rather than looked up again when the handshake
        /// lands: `drivers.json` may have been rewritten in between, and a
        /// metadata read has to use the four custom queries (D9) the session
        /// was actually opened with.
        driver: Box<DriverDef>,
        /// Dropped — and so abandoned — when the state is replaced.
        _task: Task<()>,
    },
    /// A session is open.
    Open {
        /// The profile it was opened from.
        profile: Box<ConnectionProfile>,
        /// The driver definition behind it, which every metadata read needs.
        driver: Box<DriverDef>,
        /// The session and the tunnel under it.
        ///
        /// Boxed for the reason the profile is: `Connected` carries the
        /// bridge's whole `SESSION_INFO` answer, and an unboxed variant would
        /// make the idle state as large as the connected one.
        session: Box<Connected>,
    },
    /// The last attempt failed, and the reason is still on the status bar.
    Failed {
        /// What was being opened.
        profile: Box<ConnectionProfile>,
        /// [`ConnectError::message`], already rendered.
        message: SharedString,
    },
}

impl ConnectionState {
    /// The profile this state is about, if any.
    fn profile(&self) -> Option<&ConnectionProfile> {
        match self {
            ConnectionState::Idle => None,
            ConnectionState::Connecting { profile, .. }
            | ConnectionState::Open { profile, .. }
            | ConnectionState::Failed { profile, .. } => Some(profile),
        }
    }

    /// The open session, for whoever needs to run a statement on it.
    fn session(&self) -> Option<&Connected> {
        match self {
            ConnectionState::Open { session, .. } => Some(session),
            _ => None,
        }
    }

    /// The colour of the dot in front of the connection selector.
    ///
    /// The three states are three colours because they are three different
    /// situations: a session that is still opening and one that died are told
    /// apart without opening anything (architecture document, §4.2).
    fn dot(&self, theme: &Theme) -> Hsla {
        match self {
            ConnectionState::Idle => theme.text_muted,
            ConnectionState::Connecting { .. } => theme.accent,
            ConnectionState::Open { .. } => theme.success,
            ConnectionState::Failed { .. } => theme.danger,
        }
    }
}

/// What a profile is called on screen.
///
/// The name the user gave it, and the URL when they have not given one yet: a
/// row reading "(unnamed)" in a list of three is not something to pick
/// between, where the URL at least says which database it is.
/// [`ConnectionProfile::label`] is the *session*'s name — `user@url` — which
/// is a different question and belongs in a tab, not in the picker.
fn label_of(profile: &ConnectionProfile) -> SharedString {
    let name = profile.name.trim();
    if name.is_empty() {
        SharedString::from(profile.label())
    } else {
        SharedString::from(name.to_owned())
    }
}

/// The whole of the window.
struct Workspace {
    /// Focus target for the window, so the shortcuts stay live.
    ///
    /// One handle for the whole shell in M0: nothing in the body holds anything
    /// focusable except the welcome screen's buttons, so there is little for the
    /// keyboard to be inside of. A pane that grows a view of its own brings a
    /// focus handle with it, and this one becomes what it is meant to be: the
    /// fallback that keeps the shortcuts alive while nothing else holds the
    /// keyboard.
    focus_handle: FocusHandle,
    /// The saved profiles, as the welcome screen lists them.
    ///
    /// A copy of `connections.json`, read at start-up. From M2 it is re-read
    /// whenever the connection dialog closes — the dialog is the only thing
    /// that edits the file, and it may have saved, renamed or deleted a profile
    /// while it was up.
    profiles: ConnectionStore,
    /// Vertical scroll of the welcome screen.
    welcome_scroll: ScrollHandle,
    /// Whether the welcome screen's overlay scroll indicator is on screen.
    welcome_scrollbar: ScrollbarState,
    /// The about dialog, rendered only while it reports itself open.
    about: Entity<AboutDialog>,
    /// The settings dialog, rendered only while it reports itself open.
    settings: Entity<SettingsDialog>,
    /// The connection dialog, rendered only while it reports itself open.
    ///
    /// It edits a draft of the store and writes `connections.json` on `Save`
    /// alone (architecture document, D7), so the copy in [`Workspace::profiles`]
    /// is re-read whenever it closes.
    connection_dialog: Entity<ConnectionDialog>,
    /// Whether a session is open, opening, or has just failed.
    connection: ConnectionState,
    /// Which connection the answers coming back off background tasks belong to.
    ///
    /// Bumped by every connect and every disconnect. A metadata read that was
    /// in flight when the session went away comes back carrying the number it
    /// left with, and is dropped rather than written into the tree of whatever
    /// is open now — which would otherwise be one database's schemas under
    /// another's name.
    connection_epoch: u64,
    /// Whether the title bar's connection list is showing.
    connection_list_open: bool,
    /// The left-hand panel: the tree, the filter and the ticks.
    explorer: Entity<Explorer>,
    /// The right-hand panel: what one table is made of.
    inspector: Entity<Inspector>,
    /// Whether the explorer is on screen, when a connection is open.
    explorer_visible: bool,
    /// Whether the inspector is on screen, when a connection is open.
    inspector_visible: bool,
    /// Width of the explorer, in logical pixels.
    ///
    /// Held here and written back to the settings when a drag ends: a drag is
    /// hundreds of events, and `settings.json` is written once, when the window
    /// closes.
    explorer_width: f32,
    /// Width of the inspector, in logical pixels.
    inspector_width: f32,
    /// The update dialog, rendered only while it reports itself open.
    ///
    /// Two things open it: the start-up check in [`update`], at most once per
    /// run and only when it found something worth saying, and the "Check for
    /// updates" command, as often as the user asks. It also owns the download
    /// and the swap that "Update" starts, which is why it is the one dialog the
    /// shell cannot always close.
    update: Entity<UpdateDialog>,
    /// Whether the help menu is showing.
    menu_open: bool,
    /// Title bar style currently *on the window*.
    ///
    /// Starts as the style the window was created with. Not read from the
    /// settings directly: the title bar has to branch on what the window
    /// actually carries, and once the settings dialog switches a live window
    /// this field is what follows the platform call rather than the stored
    /// preference.
    titlebar: TitlebarStyle,
    /// Keeps the about dialog subscription alive.
    _about_events: Subscription,
    /// Keeps the settings dialog subscription alive.
    _settings_events: Subscription,
    /// Keeps the update dialog subscription alive.
    _update_events: Subscription,
    /// Keeps the connection dialog subscription alive.
    _connection_events: Subscription,
    /// Keeps the explorer's subscription alive.
    _explorer_events: Subscription,
    /// Keeps the inspector's subscription alive.
    _inspector_events: Subscription,
    /// Closes the session before the process winds down.
    _quit: Subscription,
    /// Records the window's placement as it is moved and resized.
    _bounds: Subscription,
    /// Redraws the title bar when the desktop moves its caption buttons.
    _button_layout: Subscription,
}

impl Workspace {
    /// Builds the shell with no connection open, and so no work area at all.
    ///
    /// `titlebar` is the style the window was opened with; from then on the
    /// field tracks whatever the applied settings switched the window to.
    fn new(titlebar: TitlebarStyle, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let about = cx.new(AboutDialog::new);
        let about_events =
            cx.subscribe_in(
                &about,
                window,
                |this, dialog, event, window, cx| match event {
                    AboutDialogEvent::Dismissed => {
                        dialog.update(cx, |dialog, cx| dialog.close(cx));
                        this.focus_shell(window, cx);
                    }
                },
            );

        let settings = cx.new(SettingsDialog::new);
        let settings_events = cx.subscribe_in(
            &settings,
            window,
            |this, dialog, event, window, cx| match event {
                // The dialog has already replaced and persisted the settings
                // global by the time it emits this; the shell re-applies the
                // parts that touch the live window.
                SettingsDialogEvent::Applied => {
                    this.apply_settings(window, cx);
                    // The dialog closes itself after applying; without a refocus
                    // the window focus dangles on its unrendered controls and
                    // macOS disables every menu item validated through it.
                    this.focus_shell(window, cx);
                }
                // The user is still in the dialog and nothing has been saved, so
                // only the palettes and the fonts follow — and the focus stays
                // where it is, since taking it back now would pull it out from
                // under whoever is typing.
                SettingsDialogEvent::Previewed => this.apply_preview(window, cx),
                // Closing dropped the preview, so re-applying now resolves back
                // to the settings on disk. That is the whole of the undo.
                SettingsDialogEvent::Dismissed => {
                    dialog.update(cx, |dialog, cx| dialog.close(cx));
                    this.apply_preview(window, cx);
                    this.focus_shell(window, cx);
                }
            },
        );

        let connection_dialog = cx.new(ConnectionDialog::new);
        let connection_events = cx.subscribe_in(
            &connection_dialog,
            window,
            |this, dialog, event, window, cx| match event {
                // The dialog has already written `connections.json`; the shell
                // re-reads it and opens the session, because the session is the
                // shell's to own — a dialog that held one could not be closed
                // without closing the database with it.
                ConnectionDialogEvent::Connect(profile) => {
                    this.profiles = load_profiles();
                    let profile = (**profile).clone();
                    this.focus_shell(window, cx);
                    this.connect_to(profile, cx);
                }
                ConnectionDialogEvent::Dismissed => {
                    dialog.update(cx, |dialog, cx| dialog.close(cx));
                    // Saved, renamed or deleted while it was up, so the welcome
                    // list and the selector both have to be re-read.
                    this.profiles = load_profiles();
                    this.focus_shell(window, cx);
                }
            },
        );

        // The one thing that must happen before the process winds down: the
        // session is closed, and the tunnel under it after that. A session left
        // open holds an embedded database's lock file, which the next run then
        // cannot get past.
        let quit = cx.on_app_quit(|workspace: &mut Workspace, _cx| {
            workspace.close_session();
            async {}
        });

        let update = cx.new(UpdateDialog::new);
        let update_events = cx.subscribe_in(&update, window, |this, dialog, event, window, cx| {
            match event {
                UpdateDialogEvent::Ignored { tag } => {
                    // The dialog has already closed itself; writing the file is
                    // the shell's job because the shell is what owns settings.
                    update::remember_ignored(tag, cx);
                    this.focus_shell(window, cx);
                }
                UpdateDialogEvent::Dismissed => {
                    dialog.update(cx, |dialog, cx| dialog.close(cx));
                    this.focus_shell(window, cx);
                }
            }
        });

        // The start-up update check, off the UI thread: it is an HTTPS request
        // to GitHub, and nothing on screen waits for it (architecture document,
        // §4.1). The tag the user may have ignored is read here, on the UI
        // thread, because the settings global is only reachable from it.
        //
        // The answer opens a dialog, so it deliberately does *not* go through
        // `open_about`'s `close_overlays` route: this is the one dialog nobody
        // asked for, arriving at a moment nobody chose, and it must never take
        // the screen from something the user opened themselves. If anything is
        // already up, the check simply says nothing and tries again next
        // launch.
        //
        // `update::check` answers `None` outright in a test build; see the note
        // on it for why the guard is there and not here.
        let ignored = app_settings::current(cx).ignored_update;
        cx.spawn(async move |this, cx| {
            let found = cx
                .background_executor()
                .spawn(async move { update::check(ignored.as_deref()) })
                .await;
            let Some(release) = found else {
                return;
            };
            this.update(cx, |workspace, cx| {
                if workspace.dialog_open(cx) {
                    log::debug!("update {} announced while a dialog is open", release.tag);
                    return;
                }
                workspace.update.update(cx, |dialog, cx| {
                    dialog.open(release, cx);
                });
                cx.notify();
            })
            .ok();
        })
        .detach();

        // The two panels of the body. Both are built empty and stay out of the
        // frame until a session opens; see [`Workspace::render_body`].
        let explorer = cx.new(Explorer::new);
        let explorer_events = cx.subscribe(&explorer, |workspace, _explorer, event, cx| {
            match event {
                // Both of these need the session, which is the workspace's.
                ExplorerEvent::LoadSchemas => workspace.load_schemas(cx),
                ExplorerEvent::LoadTables(schema) => workspace.load_tables(schema.clone(), cx),
                // The status bar counts the ticks, and nothing else changed.
                ExplorerEvent::SelectionChanged => cx.notify(),
                ExplorerEvent::Inspect { table, reveal } => {
                    // Only the menu row asks for the panel to be shown. Moving
                    // the tree's cursor must not: an arrow key that reopened a
                    // panel the user had put away would be the shell arguing.
                    if *reveal {
                        workspace.inspector_visible = true;
                        workspace.remember_layout(cx);
                    }
                    let table = table.clone();
                    workspace
                        .inspector
                        .update(cx, |panel, cx| panel.show(table, cx));
                    cx.notify();
                }
            }
        });

        let inspector = cx.new(Inspector::new);
        let inspector_events =
            cx.subscribe(&inspector, |workspace, _panel, event, cx| match event {
                InspectorEvent::Load(table) => workspace.load_table(table.clone(), cx),
            });

        // In memory only; the file is written once, when the window closes. See
        // [`app_settings::record_window_geometry`].
        let bounds = cx.observe_window_bounds(window, |_this, window, cx| {
            record_window_geometry(window, cx);
        });

        // The desktop decides where the caption buttons go, and it can be told
        // to change its mind while the window is open — the settings dialog of
        // GNOME or KDE moves them the moment the choice is made. Nothing else
        // in the window changes when it does, so the layout is read afresh on
        // every frame (see [`Workspace::render_toolbar`]) and this only has to
        // ask for a frame.
        let this = cx.weak_entity();
        let button_layout = window.observe_button_layout_changed(move |_window, cx| {
            this.update(cx, |_, cx| cx.notify()).ok();
        });

        // The layout the last session left behind. Read here rather than in
        // `render`, so a drag can move the live value without the stored one
        // pulling it back on the next frame.
        let layout = app_settings::current(cx);

        Self {
            focus_handle: cx.focus_handle(),
            profiles: load_profiles(),
            welcome_scroll: ScrollHandle::new(),
            welcome_scrollbar: ScrollbarState::new(),
            about,
            settings,
            connection_dialog,
            connection: ConnectionState::Idle,
            connection_epoch: 0,
            connection_list_open: false,
            explorer,
            inspector,
            explorer_visible: layout.explorer_visible,
            inspector_visible: layout.inspector_visible,
            explorer_width: layout.explorer_width,
            inspector_width: layout.inspector_width,
            update,
            menu_open: false,
            titlebar,
            _about_events: about_events,
            _settings_events: settings_events,
            _update_events: update_events,
            _connection_events: connection_events,
            _explorer_events: explorer_events,
            _inspector_events: inspector_events,
            _quit: quit,
            _bounds: bounds,
            _button_layout: button_layout,
        }
    }

    // --- focus ------------------------------------------------------------

    /// Puts the keyboard back on the shell after a dialog closes.
    fn focus_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus_handle, cx);
        cx.notify();
    }

    // --- dialogs ----------------------------------------------------------

    /// Whether any modal is on screen.
    ///
    /// Exactly the set [`Workspace::close_overlays`] closes, minus the dropdown
    /// menu: it is transient and dismisses itself on the next press, so a
    /// dialog appearing over one takes nothing away.
    ///
    /// One caller, and the reason this exists at all: the start-up update check
    /// announces itself only into an empty window. It is the one dialog nobody
    /// asked for, and it must not land on top of something the user opened.
    fn dialog_open(&self, cx: &App) -> bool {
        self.about.read(cx).is_open()
            || self.settings.read(cx).is_open()
            || self.update.read(cx).is_open()
            || self.connection_dialog.read(cx).is_open()
    }

    /// Closes every dialog and the dropdown menu.
    ///
    /// Every `open_*` method starts here, which is what keeps the modals
    /// mutually exclusive: only one of them can be on screen at a time, and
    /// opening one always puts the menu away.
    ///
    /// Closing the settings dialog drops its live preview, so the palettes are
    /// re-applied on the way out; without that the window would keep wearing a
    /// theme that nothing in the settings names any more.
    ///
    /// The update dialog is closed here like the rest, so a user who reaches
    /// for a command instead of one of its buttons is not left with a stale
    /// announcement floating over the window — except while it is installing,
    /// when its own `close` refuses and the swap is allowed to finish; see
    /// [`UpdateDialog::close`].
    fn close_overlays(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.menu_open = false;
        if self.about.read(cx).is_open() {
            self.about.update(cx, |dialog, cx| dialog.close(cx));
        }
        if self.settings.read(cx).is_open() {
            self.settings.update(cx, |dialog, cx| dialog.close(cx));
            self.apply_preview(window, cx);
        }
        if self.update.read(cx).is_open() {
            self.update.update(cx, |dialog, cx| dialog.close(cx));
        }
        if self.connection_dialog.read(cx).is_open() {
            self.connection_dialog
                .update(cx, |dialog, cx| dialog.close(cx));
        }
    }

    /// Opens the about dialog, closing whatever else was showing.
    fn open_about(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        self.about.update(cx, |dialog, cx| dialog.open(cx));
        cx.notify();
    }

    /// Asks GitHub for the latest release and shows the answer.
    ///
    /// Goes through `close_overlays` where the start-up check pointedly does
    /// not: this dialog was asked for, so it is entitled to the screen the way
    /// every other menu command is.
    ///
    /// Refuses while an install is already running, which is the one case where
    /// the update dialog cannot be closed and so must not be reopened into a
    /// different state.
    fn check_updates(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.update.read(cx).is_busy() {
            return;
        }
        self.close_overlays(window, cx);
        self.update.update(cx, |dialog, cx| dialog.start_check(cx));
        cx.notify();
    }

    /// Opens the connection dialog over the profile that is open, if one is.
    fn open_connection_dialog(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        // What the driver editor's custom-query **Test** runs against when the
        // driver being edited is the one this session was opened through.
        let session = self.connection.session().map(|connected| {
            (
                self.connection
                    .profile()
                    .map(|profile| profile.driver_id.clone())
                    .unwrap_or_default(),
                connected.handle(),
            )
        });
        let at = self.connection.profile().map(|profile| profile.id);
        self.connection_dialog.update(cx, |dialog, cx| {
            dialog.set_open_session(session);
            match at {
                Some(id) => dialog.open_at(id, cx),
                None => dialog.open(cx),
            }
        });
        cx.notify();
    }

    // --- the session ------------------------------------------------------

    /// Opens a session for `profile`, replacing whatever was open.
    ///
    /// Everything that blocks — the keychain read, the tunnel, `JNI_CreateJavaVM`
    /// on the first connection of the run, `OPEN_SESSION` — happens on a
    /// background task, so the window stays live while a database that is not
    /// answering takes its time about saying so.
    fn connect_to(&mut self, profile: ConnectionProfile, cx: &mut Context<Self>) {
        self.close_session();

        let drivers = match DriverStore::load() {
            Ok(drivers) => drivers,
            Err(error) => {
                log::error!("could not read drivers.json: {error:#}");
                DriverStore::default()
            }
        };
        let Some(driver) = drivers.get(&profile.driver_id).cloned() else {
            let message = ts!("connect.no_driver", driver = profile.driver_id.clone());
            self.connection = ConnectionState::Failed {
                profile: Box::new(profile),
                message,
            };
            cx.notify();
            return;
        };

        let settings = app_settings::current(cx);
        let opening = profile.clone();
        let definition = driver.clone();
        let run = cx.background_spawn(async move {
            // Read here rather than on the UI thread: a keychain that is locked
            // puts a system prompt up, and waiting for that on the UI thread
            // would freeze the window behind it.
            let credentials = Credentials::read(&opening);
            connection::connect(&opening, &driver, &credentials, &settings)
        });
        let task = cx.spawn(async move |this, cx| {
            let outcome = run.await;
            this.update(cx, |workspace, cx| workspace.connected(outcome, cx))
                .ok();
        });

        self.connection = ConnectionState::Connecting {
            profile: Box::new(profile),
            driver: Box::new(definition),
            _task: task,
        };
        cx.notify();
    }

    /// Empties both panels and retires whatever fetch was in flight.
    ///
    /// Called by every connect and every disconnect. The ticks go with the
    /// rest: a tick names a table of *that* database, and carrying one across
    /// to another server would be a set of names that happen to match.
    fn reset_panels(&mut self, cx: &mut Context<Self>) {
        self.connection_epoch = self.connection_epoch.wrapping_add(1);
        self.explorer.update(cx, |explorer, cx| explorer.reset(cx));
        self.inspector.update(cx, |panel, cx| panel.reset(cx));
    }

    /// The session, the driver definition and the epoch a metadata read needs.
    ///
    /// `None` while nothing is open, which is what makes every call site below
    /// a no-op rather than a panic when the session went away between the click
    /// and the frame.
    fn meta_context(&self) -> Option<(SessionHandle, DriverDef, u64)> {
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
    fn load_schemas(&mut self, cx: &mut Context<Self>) {
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
    fn load_tables(&mut self, key: SchemaKey, cx: &mut Context<Self>) {
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
    fn load_table(&mut self, key: TableKey, cx: &mut Context<Self>) {
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
            })
            .ok();
        })
        .detach();
    }

    /// Records what the connection attempt came back with.
    ///
    /// A failure is reported on the status bar rather than in a dialog: the
    /// user asked for a database, not for a box to dismiss, and the message
    /// has to stay readable while they open the connection dialog to fix
    /// whatever it names.
    fn connected(&mut self, outcome: Result<Connected, ConnectError>, cx: &mut Context<Self>) {
        // Anything but `Connecting` means the attempt was abandoned — the user
        // asked for another connection, or disconnected — and its answer is no
        // longer about the state the window is in.
        let ConnectionState::Connecting {
            profile, driver, ..
        } = std::mem::replace(&mut self.connection, ConnectionState::Idle)
        else {
            if let Ok(session) = outcome
                && let Err(error) = session.close()
            {
                log::warn!("an abandoned session did not close: {error:#}");
            }
            return;
        };

        self.connection = match outcome {
            Ok(session) => {
                log::info!(
                    "connected to {} ({})",
                    profile.name,
                    session.product().unwrap_or_else(|| "unknown".into())
                );
                // A tunnel that breaks takes the session above it with it, and
                // is never repaired silently: a reconnection would hide what
                // was in flight when the socket went away. The watch fires once,
                // with the reason, and the window goes to the failed state
                // wearing it.
                if let Some(lease) = session.lease() {
                    let watch = lease.watch();
                    let id = profile.id;
                    cx.spawn(async move |this, cx| {
                        let Ok(reason) = watch.await else {
                            return;
                        };
                        this.update(cx, |workspace, cx| {
                            workspace.tunnel_died(id, reason, cx);
                        })
                        .ok();
                    })
                    .detach();
                }
                ConnectionState::Open {
                    profile,
                    driver,
                    session: Box::new(session),
                }
            }
            Err(error) => {
                log::warn!("could not connect to {}: {error}", profile.name);
                ConnectionState::Failed {
                    profile,
                    message: error.message().into(),
                }
            }
        };
        // After the state is in place, not before: the tree asks for its root
        // on the first frame it is drawn in, and the request runs through
        // [`Workspace::meta_context`], which reads that state.
        self.reset_panels(cx);
        cx.notify();
    }

    /// The tunnel under the open session ended; the session goes with it.
    ///
    /// Keyed on the profile's id rather than on its name: by the time this
    /// arrives the user may have connected to something else, and only the
    /// session that was running over *this* tunnel is the one to take down.
    fn tunnel_died(&mut self, id: uuid::Uuid, reason: String, cx: &mut Context<Self>) {
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
    fn disconnect(&mut self, cx: &mut Context<Self>) {
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
    fn close_session(&mut self) {
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
    fn session_handle(&self) -> Option<SessionHandle> {
        self.connection.session().map(Connected::handle)
    }

    /// Opens the settings dialog, closing whatever else was showing.
    fn open_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.close_overlays(window, cx);
        self.settings.update(cx, |dialog, cx| dialog.open(cx));
        cx.notify();
    }

    /// Shows or hides the help menu.
    fn set_menu_open(&mut self, open: bool, cx: &mut Context<Self>) {
        if self.menu_open != open {
            self.menu_open = open;
            cx.notify();
        }
    }

    /// Re-applies everything a saved settings file changes about the live
    /// window.
    ///
    /// Every platform call in here acts on the window, and one of them —
    /// `request_decorations` on X11 — is the call that used to re-enter gpui's
    /// window callbacks and panic. It is safe from this stack: the settings
    /// dialog emits its event, gpui delivers it after the button's own callback
    /// has returned and released every borrow, and this runs from there. It must
    /// stay that way; calling it from inside a widget callback would put the
    /// borrow back.
    fn apply_settings(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let settings = app_settings::current(cx);
        // Before the repaint below, so the next frame is already drawn in the
        // newly chosen language.
        i18n::apply(settings.language.as_deref());
        // The native macOS menu bar is built once and owned by the platform, so
        // unlike the in-app menu it does not follow a repaint; it has to be
        // handed over again.
        cx.set_menus(app_menus());
        // The grid borrows its column names from the source it was given, so a
        // language change has to be written into it; everything else on the two
        // panels is built afresh every frame.
        self.inspector.update(cx, |panel, cx| panel.relabel(cx));
        apply_themes(&settings, cx);
        // Ahead of the repaint, so the title bar's next frame already knows
        // whether it has to stand in for a caption; and ahead of the two calls
        // below, which leave the accent policy and the caption colors on the
        // window, so a caption that comes back here comes back already themed.
        //
        // The field follows the call rather than the stored setting: everything
        // that branches on it is asking what the window carries, not what was
        // last saved.
        if settings.window.titlebar != self.titlebar {
            self.titlebar = settings.window.titlebar;
            let custom = self.titlebar == TitlebarStyle::Custom;
            window.set_titlebar_transparent(custom, custom.then_some(TRAFFIC_LIGHT_ORIGIN));
            // The Linux counterpart of the call above, which only the Windows
            // and macOS backends implement: swap the compositor's frame for
            // client-side decorations (or back) on the live window.
            #[cfg(not(any(target_os = "windows", target_os = "macos")))]
            window.request_decorations(if custom {
                gpui::WindowDecorations::Client
            } else {
                gpui::WindowDecorations::Server
            });
        }
        // Paired with the call below, and never with a preview: the leaf crates
        // read this to decide whether to paint their own background, and the
        // answer is only right once the surface itself permits alpha. Ahead of
        // the repaint, so the next frame already draws under the new answer.
        set_window_tint(settings.window.background_opacity, cx);
        cx.refresh_windows();
        window.set_background_appearance(window_appearance(&settings.window));
        // After the background appearance, never before: on Windows that call
        // re-arms the accent policy that would otherwise repaint the caption out
        // from under us.
        apply_caption_theme(window, &theme(cx), cx);
    }

    /// Re-applies the palettes the settings dialog is currently showing.
    ///
    /// The unsaved half of [`Workspace::apply_settings`], and deliberately much
    /// smaller: only the two palettes and the fonts are previewed, so this
    /// touches no platform state beyond the native caption's colours, which have
    /// to follow the chrome theme or the window would be half repainted.
    ///
    /// Reads [`app_settings::effective`], which answers the preview while one is
    /// installed and the saved settings once it is dropped — so the same call
    /// both applies a preview and undoes it.
    fn apply_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        apply_themes(&app_settings::effective(cx), cx);
        cx.refresh_windows();
        apply_caption_theme(window, &theme(cx), cx);
    }

    // --- actions ----------------------------------------------------------

    /// Opens the connection dialog.
    fn new_connection_action(
        &mut self,
        _: &NewConnection,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_connection_dialog(window, cx);
    }

    /// Opens the settings dialog.
    fn open_settings_action(
        &mut self,
        _: &OpenSettings,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.open_settings(window, cx);
    }

    /// Opens the about box.
    fn show_about_action(&mut self, _: &ShowAbout, window: &mut Window, cx: &mut Context<Self>) {
        self.open_about(window, cx);
    }

    /// Asks GitHub for the latest release.
    fn check_updates_action(
        &mut self,
        _: &CheckUpdates,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.check_updates(window, cx);
    }

    /// Shows and hides the explorer sidebar.
    fn toggle_explorer_action(
        &mut self,
        _: &ToggleExplorer,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.explorer_visible = !self.explorer_visible;
        self.hid_a_panel(window, cx);
    }

    /// Shows and hides the inspector panel.
    fn toggle_inspector_action(
        &mut self,
        _: &ToggleInspector,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.inspector_visible = !self.inspector_visible;
        self.hid_a_panel(window, cx);
    }

    /// Takes the keyboard back and records what the layout now is.
    ///
    /// The focus half is not optional: the panel that has just gone may have
    /// been holding the keyboard — the filter box, the tree — and a focus
    /// handle left dangling on an unrendered element takes every shortcut in
    /// the window with it (architecture document, Appendix A). It has to happen
    /// in the same update that hides the subtree, which is this one.
    fn hid_a_panel(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.focus_shell(window, cx);
        self.remember_layout(cx);
        cx.notify();
    }

    /// Writes the panel widths and visibilities into the settings global.
    ///
    /// [`app_settings::save`] takes them to disk with everything else when the
    /// last window closes, so this costs a struct copy and no file write.
    fn remember_layout(&mut self, cx: &mut Context<Self>) {
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
    fn explorer_showing(&self) -> bool {
        self.explorer_visible && matches!(self.connection, ConnectionState::Open { .. })
    }

    /// Whether the inspector is on screen.
    fn inspector_showing(&self) -> bool {
        self.inspector_visible && matches!(self.connection, ConnectionState::Open { .. })
    }

    /// Moves the explorer's edge to wherever the pointer has dragged it.
    ///
    /// Measured against the row's own box rather than tracked as a delta, so
    /// the edge sits under the pointer however far the gesture wandered —
    /// including outside the window, which a delta would have to keep
    /// integrating.
    fn drag_explorer(&mut self, event: &DragMoveEvent<DraggedExplorer>, cx: &mut Context<Self>) {
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
    fn drag_inspector(&mut self, event: &DragMoveEvent<DraggedInspector>, cx: &mut Context<Self>) {
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

    /// Closes whatever overlay is on top, in the order they are stacked.
    fn dismiss_dialog_action(
        &mut self,
        _: &DismissDialog,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // The dropdown menu paints above everything else, so it goes first.
        if self.menu_open {
            self.set_menu_open(false, cx);
            return;
        }
        if self.update.read(cx).is_open() {
            // Swallowed rather than propagated while an install runs: the key
            // must not reach anything else, but nothing may take the screen
            // from a swap either, so `Escape` simply does nothing until it is
            // over.
            if !self.update.read(cx).is_busy() {
                self.update.update(cx, |dialog, cx| dialog.close(cx));
                self.focus_shell(window, cx);
            }
            return;
        }
        if self.about.read(cx).is_open() {
            self.about.update(cx, |dialog, cx| dialog.close(cx));
            self.focus_shell(window, cx);
            return;
        }
        if self.connection_dialog.read(cx).is_open() {
            // Routed through the dialog for the reason the settings dialog is:
            // it stacks the driver editor and a delete confirmation of its own,
            // and each has to be able to take `Escape` for itself before the
            // whole form is thrown away.
            self.connection_dialog
                .update(cx, |dialog, cx| dialog.escape(cx));
            return;
        }
        if self.settings.read(cx).is_open() {
            // Routed through the dialog rather than closed from here: it stacks
            // a colour editor, two dropdowns and a delete confirmation of its
            // own, and each of those has to be able to take `Escape` for itself
            // before the whole form is thrown away. gpui matches key bindings
            // ahead of key listeners, so this handler — not the dialog's own —
            // is where the key actually lands.
            self.settings.update(cx, |dialog, cx| dialog.escape(cx));
            return;
        }
        cx.propagate();
    }

    // --- the scroll bar over the welcome screen ---------------------------

    /// The welcome screen's overlay scroll indicator, as it now stands.
    ///
    /// Rebuilt on demand rather than kept, because everything it is made of —
    /// the box, how far it overflows, where it sits — is measured afresh by
    /// gpui on every layout pass.
    fn scrollbar(&self) -> Scrollbar {
        Scrollbar::for_handle(
            WELCOME_SCROLLBAR,
            ScrollbarAxis::Vertical,
            &self.welcome_scroll,
        )
        .fade(self.welcome_scrollbar.fade())
    }

    /// The same bar, listening for the pointer reaching the edge it rides.
    fn hovering_scrollbar(&self, cx: &mut Context<Self>) -> Scrollbar {
        self.scrollbar()
            .on_hover(cx.listener(move |workspace, hovered: &bool, _window, cx| {
                workspace.hover_scrollbar(*hovered, cx);
            }))
    }

    /// Puts the bar up whenever the welcome screen has moved, and starts the
    /// clock that takes it down again.
    fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        let scrolled = scrolled(&self.welcome_scroll, ScrollbarAxis::Vertical);
        if let Some(epoch) = self.welcome_scrollbar.moved(scrolled) {
            hide_later(epoch, cx, move |workspace| {
                Some(&mut workspace.welcome_scrollbar)
            });
        }
    }

    /// Scrolls the welcome screen when its thumb is dragged.
    fn drag_scrollbar(&mut self, event: &DragMoveEvent<DraggedThumb>, cx: &mut Context<Self>) {
        // Every bar in the window answers a drag of this type and all but one
        // of them finds it is about somebody else's thumb; the inspector's is
        // asked here because the panel has no always-mounted element of its own
        // to listen from.
        self.inspector
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
    fn release_scrollbars(&mut self, cx: &mut Context<Self>) {
        self.inspector
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
    fn hover_scrollbar(&mut self, hovered: bool, cx: &mut Context<Self>) {
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

    /// Renders the title bar: the application mark, the connection selector,
    /// the settings button and the help menu.
    ///
    /// In the custom title bar style this row *is* the title bar. It then marks
    /// itself as the window's drag area, takes over writing the application's
    /// name at its left end, and — off macOS, which keeps its native traffic
    /// lights — grows a set of caption buttons at its right end. Every *control*
    /// inside it occludes, so the drag area only ever answers for the gaps
    /// between them; see [`rudbgen_ui::window_controls`]. The name is not a
    /// control and deliberately does not.
    fn render_toolbar(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let custom = draws_own_titlebar(self.titlebar, window);

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
            // The shipped icon in its own colours: img() keeps them, where the
            // svg element would flatten the mark into a theme-tinted glyph;
            // see [`icons::APP_ICON`].
            let icon = (!cfg!(target_os = "macos"))
                .then(|| img(icons::APP_ICON).size(px(16.)).flex_none());
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
                .text_color(theme.text_muted)
                .children(icon)
                .child(APP_NAME)
        });

        // The caption buttons the other two platforms have to draw themselves.
        //
        // Two strips rather than one, because a Linux desktop decides where its
        // caption buttons go and putting them on the left is a setting people
        // actually use; [`rudbgen_ui::window_controls::split`] turns what the
        // platform reports into the two ends. Off Linux nothing is reported,
        // which is the same answer as "the usual three on the right".
        let (leading_buttons, trailing_buttons) = if custom && !cfg!(target_os = "macos") {
            window_controls::split(cx.button_layout(), window.window_controls())
        } else {
            (Vec::new(), Vec::new())
        };
        let strip = |id: &'static str, buttons: Vec<gpui::WindowButton>| {
            (!buttons.is_empty()).then(|| {
                WindowControls::new(
                    id,
                    WindowControlIcons {
                        minimize: icons::WINDOW_MINIMIZE.into(),
                        maximize: icons::WINDOW_MAXIMIZE.into(),
                        restore: icons::WINDOW_RESTORE.into(),
                        close: icons::WINDOW_CLOSE.into(),
                    },
                    buttons,
                )
            })
        };
        let leading_controls = strip("window-controls-leading", leading_buttons);
        let trailing_controls = strip("window-controls-trailing", trailing_buttons);

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
                titlebar_gestures(this.occlude().window_control_area(WindowControlArea::Drag))
            })
            // Ahead of the wordmark, which is where a desktop that asks for
            // left-hand caption buttons expects them: the buttons are the
            // window's, the name is the application's.
            .children(leading_controls)
            .children(traffic_lights)
            .children(title)
            .child(self.render_connection_select(cx))
            // The gap the window is dragged by: everything to its left and
            // right is a control, and this is what is left of the caption.
            .child(div().flex_1().min_w_0())
            .child(self.render_settings_button(cx))
            .child(self.render_help_menu(cx))
            .children(trailing_controls)
            .into_any_element()
    }

    /// The connection selector, at the left of the title bar.
    ///
    /// A [`Select`] with a status dot in front of it: connecting, connected or
    /// failed, so a session that is still opening and one that died are told
    /// apart without opening anything. The list is the saved profiles, with
    /// **Disconnect** on the end while a session is open — the one row that is
    /// not a connection, which is why the handler branches on the index rather
    /// than on the text.
    fn render_connection_select(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
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

    /// The settings button, drawn like a menu trigger so the two read as a pair.
    fn render_settings_button(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme(cx);
        div()
            .id("titlebar-settings")
            // For [`Workspace::render_connection_select`]'s reason.
            .occlude()
            .flex()
            .flex_none()
            .items_center()
            .justify_center()
            .size(px(28.))
            .rounded_md()
            .text_size(px(14.))
            .text_color(theme.icon)
            .cursor_pointer()
            .hover(|style| style.bg(theme.surface_hover).text_color(theme.text))
            .tooltip(tooltip_label(ts!("titlebar.tip_settings")))
            // The same action the menu row and `Ctrl+,` dispatch: one command,
            // however it is reached.
            .on_click(|_, window, cx| window.dispatch_action(Box::new(OpenSettings), cx))
            .child("\u{2699}")
    }

    /// The help menu: every command the shell has, on the platforms without a
    /// native menu bar.
    ///
    /// Every row dispatches the action its keyboard shortcut dispatches, so the
    /// menu adds a way in rather than a second implementation. A row is greyed
    /// exactly when the action behind it would return without doing anything,
    /// and drawn rather than dropped: a command that is missing tells the reader
    /// nothing about what the application can do.
    fn render_help_menu(&self, cx: &mut Context<Self>) -> MenuButton {
        let this = cx.entity();
        let entries = vec![
            MenuEntry::new(ts!("menu.new_connection"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+N"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(NewConnection), cx)),
            MenuEntry::new(ts!("menu.settings"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+,"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            // Greyed while nothing is connected, which is when the two panels
            // are out of the frame altogether and the command would flip a
            // switch with nothing behind it.
            MenuEntry::new(ts!("menu.toggle_explorer"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+B"))
                .checked(self.explorer_visible)
                .disabled(!matches!(self.connection, ConnectionState::Open { .. }))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ToggleExplorer), cx)),
            MenuEntry::new(ts!("menu.toggle_inspector"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+I"))
                .checked(self.inspector_visible)
                .disabled(!matches!(self.connection, ConnectionState::Open { .. }))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ToggleInspector), cx)),
            MenuEntry::separator(),
            // Next to About, where a Help menu would put it and where users of
            // every other desktop application look for it.
            MenuEntry::new(ts!("menu.check_updates"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(CheckUpdates), cx)),
            MenuEntry::new(ts!("menu.about"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(ShowAbout), cx)),
            MenuEntry::separator(),
            MenuEntry::new(ts!("menu.quit"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+Q"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(Quit), cx)),
        ];

        MenuButton::new("help-menu")
            .glyph("?")
            .tooltip(ts!("titlebar.tip_help"))
            .open(self.menu_open)
            .entries(entries)
            .on_open_change(move |open, _window, cx| {
                this.update(cx, |workspace, cx| workspace.set_menu_open(open, cx));
            })
    }

    /// Renders the body of the window.
    ///
    /// The welcome screen while no session is open, and the work area once one
    /// is. The explorer and the inspector are out of the frame rather than
    /// empty until then (architecture document, §4.3); the tree that fills the
    /// first of them arrives with the metadata reader, and the tab strip
    /// between them with the Generate tab.
    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        let body = match &self.connection {
            ConnectionState::Open {
                profile, session, ..
            } => self.render_work_area(profile, session, &theme, cx),
            _ => self.render_welcome(&theme, cx),
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
    /// its divider's hit area with it — and so is the divider beside each.
    ///
    /// The row paints no fill of its own. Its children tile it and each tints
    /// its own share: the panels' surface at the two edges, the background
    /// between them. Side by side rather than stacked is what
    /// [`app_settings::window_tint`] requires, and it is what lets the blur
    /// behind the window carry on under the panels too.
    fn render_work_area(
        &self,
        profile: &ConnectionProfile,
        session: &Connected,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let sidebar = self.explorer_showing().then(|| {
            div()
                .flex()
                .flex_none()
                .w(px(self.explorer_width))
                .min_h_0()
                .child(self.explorer.clone())
        });
        let left_handle = self.explorer_showing().then(|| {
            div()
                .id("explorer-divider")
                .occlude()
                .flex_none()
                .w(px(SPLIT_HANDLE))
                // Pulled back over the sidebar's own border so the grab area
                // straddles the seam rather than pushing the work area across.
                .ml(px(-SPLIT_HANDLE))
                .cursor_ew_resize()
                .on_drag(DraggedExplorer, |_, _, _, cx| cx.new(|_| gpui::Empty))
        });
        let right_handle = self.inspector_showing().then(|| {
            div()
                .id("inspector-divider")
                .occlude()
                .flex_none()
                .w(px(SPLIT_HANDLE))
                .mr(px(-SPLIT_HANDLE))
                .cursor_ew_resize()
                .on_drag(DraggedInspector, |_, _, _, cx| cx.new(|_| gpui::Empty))
        });
        let panel = self.inspector_showing().then(|| {
            div()
                .flex()
                .flex_none()
                .w(px(self.inspector_width))
                .min_h_0()
                .child(self.inspector.clone())
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
            .children(left_handle)
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_w_0()
                    .min_h_0()
                    // Everything the row is not already covering with a panel's
                    // own fill, and so the one fill over these pixels; see
                    // [`app_settings::window_tint`].
                    .bg(app_settings::window_tint(theme.background, cx))
                    .child(self.render_generate_placeholder(profile, session, theme, cx)),
            )
            .children(right_handle)
            .children(panel)
            .into_any_element()
    }

    /// What stands where the tab strip and the Generate tab will be.
    ///
    /// The one part of §4.2 M2 does not fill: the explorer and the inspector
    /// are around it, so the centre says what it is waiting for rather than
    /// being blank.
    fn render_generate_placeholder(
        &self,
        profile: &ConnectionProfile,
        session: &Connected,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        let product = session
            .product()
            .map(SharedString::from)
            .unwrap_or_else(|| ts!("statusbar.unknown_product"));
        let content = div()
            .flex()
            .flex_col()
            .items_center()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(20.))
                    .text_color(theme.text)
                    .child(label_of(profile)),
            )
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(product),
            )
            .child(
                div()
                    .w(px(WELCOME_WIDTH))
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(ts!("workarea.next")),
            );

        let bar = self.hovering_scrollbar(cx);
        centered_scroll(WELCOME_STATE, &self.welcome_scroll, bar, theme, content).into_any_element()
    }

    /// The welcome screen: the name, what the application is for, the three
    /// ways in, and the connections already saved.
    ///
    /// A saved row opens the session it names; the two buttons whose milestone
    /// has not arrived say so on hover rather than by silence.
    fn render_welcome(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
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
                    shortcut = format!("{SHORTCUT_MODIFIER}+N")
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
                    .child(
                        soon(
                            "welcome-import",
                            Button::new("welcome-import", ts!("welcome.import_jdbgen"))
                                .full_width(true)
                                .disabled(true)
                                .tab_index(WELCOME_FIRST_TAB + 1),
                        )
                        .into_any_element(),
                    )
                    .child(
                        soon(
                            "welcome-template",
                            Button::new("welcome-template", ts!("welcome.open_template"))
                                .full_width(true)
                                .disabled(true)
                                .tab_index(WELCOME_FIRST_TAB + 2),
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
    fn render_status_bar(&self, cx: &mut Context<Self>) -> AnyElement {
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
                        // The first half of §4.2's arithmetic. The templates
                        // and the file count join it with the Generate tab; the
                        // tables are what there is to count today, and counting
                        // them is what makes a tick visible from the other side
                        // of the window.
                        _ => match self.explorer.read(cx).selected_count(cx) {
                            0 => ts!("statusbar.no_selection"),
                            count => ts!("statusbar.tables_selected", count = count),
                        },
                    }),
            )
            .into_any_element()
    }
}

/// One row of the welcome screen's saved list, which opens the session it
/// names.
///
/// A press connects rather than opening the editor: the list is the way *in*,
/// and a saved connection that asks to be confirmed before it opens is a
/// dialog nobody wanted. Editing one is the connection dialog's job, which the
/// title bar and `Ctrl+N` both reach.
fn profile_row(
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

/// Wraps a control whose milestone has not arrived in the tooltip that says so.
///
/// The tooltip has to go on a box around the button rather than on the button:
/// a disabled control takes no pointer events of its own, so the only element
/// that can answer a hover is one outside it.
fn soon(id: &'static str, control: impl IntoElement) -> Stateful<Div> {
    div()
        .id(id)
        .debug_selector(move || id.to_string())
        .tooltip(tooltip_label(ts!("welcome.tip_soon")))
        .child(control)
}

/// A box that centres its content while it fits and scrolls it when it does not.
///
/// The two halves of that are what the shape is for: `my_auto` on a `flex_none`
/// column centres it in a box with room to spare, and `overflow_y_scroll` on
/// the box takes over once there is not — so the column reads as centred until
/// there is more of it than the window, and is scrolled from the top once there
/// is not.
fn centered_scroll(
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

        // With client-side decorations the compositor stops drawing the drop
        // shadow along with the frame, so the window has to bring its own: the
        // surface grows a transparent band all round, the content is inset by
        // it, and the shadow is painted into it. The inset call keeps
        // `_GTK_FRAME_EXTENTS` in step so the compositor treats the content
        // edge, not the surface edge, as the window.
        let tiling = client_tiling(window);
        if tiling.is_some() {
            window.set_client_inset(px(SHADOW_BAND));
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
            .on_action(cx.listener(Self::dismiss_dialog_action))
            .child(toolbar)
            .child(body)
            .child(status_bar)
            .children(about)
            .children(settings)
            .children(connection)
            .children(update);

        let Some(tiling) = tiling else {
            // A server-decorated window: the compositor frames and shadows it,
            // and the content is the whole surface.
            return content.into_any_element();
        };

        div()
            .size_full()
            .relative()
            .bg(gpui::transparent_black())
            .when(!tiling.top, |outer| outer.pt(px(SHADOW_BAND)))
            .when(!tiling.bottom, |outer| outer.pb(px(SHADOW_BAND)))
            .when(!tiling.left, |outer| outer.pl(px(SHADOW_BAND)))
            .when(!tiling.right, |outer| outer.pr(px(SHADOW_BAND)))
            .child(
                content
                    // A hairline where the frame's own outline used to be, per
                    // untiled edge; a tiled edge meets the neighbour flush, the
                    // way the compositor would have drawn it.
                    .border_color(theme.border)
                    .when(!tiling.top, |content| content.border_t_1())
                    .when(!tiling.bottom, |content| content.border_b_1())
                    .when(!tiling.left, |content| content.border_l_1())
                    .when(!tiling.right, |content| content.border_r_1())
                    .when(!tiling.is_tiled(), |content| {
                        content.shadow(vec![gpui::BoxShadow {
                            color: gpui::hsla(0., 0., 0., 0.35),
                            blur_radius: px(SHADOW_BAND / 2.),
                            spread_radius: px(0.),
                            offset: gpui::point(px(0.), px(2.)),
                            // The band is drawn outside the window, not inside
                            // its content, which is what this frame casts.
                            inset: false,
                        }])
                    }),
            )
            // Last on purpose: the window border outranks whatever it crosses,
            // dialogs included, the way a compositor frame would.
            .children(render_resize_edges(tiling))
            .into_any_element()
    }
}

/// Installs both palettes the settings name.
///
/// The chrome theme comes straight from the configured id; the editor theme
/// goes through [`editor_theme_for`], which is where "follow the UI theme" is
/// decided. That decision lives here rather than in the settings dialog because
/// it has to hold whatever changed the inputs — a theme file appearing in the
/// user's directory moves the answer without anybody having opened a dialog.
///
/// An id nothing answers to — a theme file the user has since deleted — falls
/// back to the default rather than failing; see [`ThemeRegistry::resolve`].
fn apply_themes(settings: &AppSettings, cx: &mut App) {
    let ui = ThemeRegistry::resolve(&settings.theme, cx);
    let editor_id = editor_theme_for(
        &settings.editor_theme,
        settings.editor_theme_follows_ui,
        &settings.theme,
        ui.dark,
        &EditorThemeRegistry::all(cx),
    );
    let editor = EditorThemeRegistry::resolve(&editor_id, cx);
    set_theme(ui, cx);
    set_editor_theme(editor, cx);
}

/// The editor theme to install, given the configured one and the chrome theme.
///
/// With the "follows the UI" switch off the configured id is used as it stands.
/// With it on the answer is the first of these that exists:
///
/// 1. the editor theme sharing the chrome theme's id, when its cast matches the
///    chrome — which is how the pairs that ship under one name stay together,
///    and, since every built-in chrome theme has an editor theme of the same id,
///    is the answer for every built-in;
/// 2. the configured theme, when its cast matches the chrome — the fallback for
///    a chrome theme of the user's own, which no editor theme is named after;
/// 3. any editor theme of the right cast;
/// 4. the configured id after all, when nothing of the right cast exists.
///
/// The namesake comes first because that is what the switch promises: its label
/// says the editor theme is *matched to* the interface theme, not merely kept on
/// the same side of light and dark, and while it is on the settings dialog
/// disables the editor theme dropdown outright — so there is no pick of the
/// user's here to preserve, and letting the configured id win would freeze the
/// editor on one palette however far the chrome moved.
///
/// The cast is still checked in rule 1, though, and deliberately: a user who has
/// written a *dark* editor theme under the id of a *light* chrome theme must not
/// have it dragged into a light window. Preventing that pairing is the whole
/// reason the switch exists, so it outranks the name match.
///
/// Pure and taking the theme list as an argument so that the rule can be tested
/// without an [`App`]; the caller supplies [`EditorThemeRegistry::all`].
fn editor_theme_for(
    configured: &str,
    follows_ui: bool,
    ui_theme_id: &str,
    ui_dark: bool,
    entries: &[EditorThemeEntry],
) -> String {
    if !follows_ui {
        return configured.to_string();
    }

    let matching = |id: &str| {
        entries
            .iter()
            .find(|entry| entry.id.eq_ignore_ascii_case(id) && entry.dark == ui_dark)
    };
    if let Some(entry) = matching(ui_theme_id).or_else(|| matching(configured)) {
        return entry.id.clone();
    }
    entries
        .iter()
        .find(|entry| entry.dark == ui_dark)
        .map(|entry| entry.id.clone())
        .unwrap_or_else(|| configured.to_string())
}

/// Records the window's placement in the settings global.
///
/// Fullscreen is knowingly stored as "not maximized". gpui hands out the
/// restore bounds either way, so the size survives; coming back fullscreen with
/// no title bar and no way to tell why would read as a broken window.
fn record_window_geometry(window: &Window, cx: &mut App) {
    let (bounds, maximized) = match window.window_bounds() {
        WindowBounds::Windowed(bounds) => (bounds, false),
        WindowBounds::Maximized(bounds) => (bounds, true),
        WindowBounds::Fullscreen(bounds) => (bounds, false),
    };
    app_settings::record_window_geometry(WindowGeometry::of(bounds, maximized), cx);
}

/// The placement to open the window at.
///
/// A saved position is used as it stands; without one the saved *size* is
/// centred on the active display, which is what a first run does and what a
/// window that has never been moved deserves.
fn window_bounds(state: &WindowState, cx: &mut App) -> WindowBounds {
    let bounds = match WindowGeometry::saved(state) {
        Some(geometry) => geometry.bounds(),
        None => Bounds::centered(
            None,
            size(px(state.width as f32), px(state.height as f32)),
            cx,
        ),
    };
    if state.maximized {
        WindowBounds::Maximized(bounds)
    } else {
        WindowBounds::Windowed(bounds)
    }
}

/// Whether the title bar row has to stand in for the window's caption.
///
/// On Windows and macOS the style applied to the window settles it: a
/// transparent title bar leaves no platform caption, so the row is all there
/// is.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn draws_own_titlebar(style: TitlebarStyle, _window: &Window) -> bool {
    style == TitlebarStyle::Custom
}

/// Whether the title bar row has to stand in for the window's caption.
///
/// Linux is not the configured style alone. The custom style makes the window
/// ask for client-side decorations, but the ask can be declined — gpui falls
/// back to server decorations when no compositor is running — so what the window
/// actually ended up with is what decides here. Deciding from the style alone
/// would draw a second caption under the compositor's own.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn draws_own_titlebar(style: TitlebarStyle, window: &Window) -> bool {
    style == TitlebarStyle::Custom
        && matches!(
            window.window_decorations(),
            gpui::Decorations::Client { .. }
        )
}

/// Wires the gestures a system title bar answers to onto the custom one.
///
/// Windows needs none of them. The row reports itself as
/// [`WindowControlArea::Drag`], the hit test turns that into `HTCAPTION`, and
/// the window procedure then does the dragging, the aero-snap gestures and the
/// double-click to maximise on its own — before the app is ever told a button
/// went down.
#[cfg(target_os = "windows")]
fn titlebar_gestures(row: Stateful<Div>) -> Stateful<Div> {
    row
}

/// Wires the gestures a system title bar answers to onto the custom one.
///
/// AppKit still drags the window for the strip its own title bar would have
/// covered, so only the double-click is left to answer — and it has to go
/// through [`Window::titlebar_double_click`], which follows whatever the user
/// picked in System Settings (zoom, minimise, or nothing at all).
#[cfg(target_os = "macos")]
fn titlebar_gestures(row: Stateful<Div>) -> Stateful<Div> {
    row.on_click(|event, window, _cx| {
        if event.standard_click() && event.click_count() == 2 {
            window.titlebar_double_click();
        }
    })
}

/// Wires the gestures a system title bar answers to onto the custom one.
///
/// Everything is the app's here: the compositor is told to take over the move,
/// and the window menu and the zoom have to be asked for explicitly. Only
/// meaningful once the window carries client-side decorations, which is why the
/// caller gates them on [`Window::window_decorations`].
///
/// The move starts on the press rather than the click because the compositor
/// takes the pointer with it, so a release would never arrive.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn titlebar_gestures(row: Stateful<Div>) -> Stateful<Div> {
    row.on_click(|event, window, _cx| {
        if event.standard_click() && event.click_count() == 2 {
            window.zoom_window();
        }
    })
    .on_mouse_down(MouseButton::Left, |_, window, _cx| {
        window.start_window_move();
    })
    .on_mouse_down(MouseButton::Right, |event, window, _cx| {
        window.show_window_menu(event.position);
    })
}

/// Width of the transparent band around a self-decorated window.
///
/// The band carries the drop shadow the compositor no longer draws once the
/// window asks for client-side decorations, and doubles as the resize grip. It
/// is part of the window's surface but not of the window as the user
/// understands it: [`Window::set_client_inset`] publishes the visible bounds
/// through `_GTK_FRAME_EXTENTS`, so the compositor snaps, maximises and stacks
/// by the visible edge, exactly as it does for GTK's frames.
const SHADOW_BAND: f32 = 12.;

/// Edge length of the corner squares, where the resize goes diagonal.
const RESIZE_CORNER: f32 = 24.;

/// The tiling state of a window that draws its own frame, `None` under a
/// server-side one.
///
/// Always `None` here: Windows keeps resizing and framing the window through the
/// caption hit test even under a custom title bar, and AppKit never gives the
/// frame up at all — neither window ever carries the shadow band.
#[cfg(any(target_os = "windows", target_os = "macos"))]
fn client_tiling(_window: &Window) -> Option<gpui::Tiling> {
    None
}

/// The tiling state of a window that draws its own frame, `None` under a
/// server-side one.
///
/// `Some` exactly when the compositor granted client-side decorations, with the
/// edges that currently touch a screen or neighbour edge marked tiled — those
/// edges get no band, no shadow and no resize grip. Fullscreen counts as tiled
/// all round.
#[cfg(not(any(target_os = "windows", target_os = "macos")))]
fn client_tiling(window: &Window) -> Option<gpui::Tiling> {
    match window.window_decorations() {
        gpui::Decorations::Client { tiling } => Some(tiling),
        gpui::Decorations::Server => None,
    }
}

/// The resize handles the compositor's frame would have provided.
///
/// Asking for client-side decorations takes the frame away, resize borders
/// included, so the shadow band has to start the resize itself — the compositor
/// takes over once told, exactly as it does for the title-bar drag. The strips
/// cover the band, the corner squares reach past it into the window, and every
/// tiled edge goes without: a maximised or snapped window has no border to drag
/// there.
fn render_resize_edges(tiling: gpui::Tiling) -> Vec<AnyElement> {
    use gpui::{CursorStyle, ResizeEdge};

    let strip = px(SHADOW_BAND);
    let corner = px(RESIZE_CORNER);
    // A strip stops short of a corner square only where that square exists;
    // against a tiled perpendicular edge it runs to the end of the band.
    let inset = |tiled: bool| if tiled { px(0.) } else { corner };
    let handle = |id: &'static str, cursor: CursorStyle, edge: ResizeEdge| {
        div()
            .id(id)
            .occlude()
            .absolute()
            .cursor(cursor)
            .on_mouse_down(MouseButton::Left, move |_, window, _cx| {
                window.start_window_resize(edge);
            })
    };

    let mut handles: Vec<AnyElement> = Vec::new();
    if !tiling.top {
        handles.push(
            handle("resize-top", CursorStyle::ResizeUpDown, ResizeEdge::Top)
                .top_0()
                .left(inset(tiling.left))
                .right(inset(tiling.right))
                .h(strip)
                .into_any_element(),
        );
    }
    if !tiling.bottom {
        handles.push(
            handle(
                "resize-bottom",
                CursorStyle::ResizeUpDown,
                ResizeEdge::Bottom,
            )
            .bottom_0()
            .left(inset(tiling.left))
            .right(inset(tiling.right))
            .h(strip)
            .into_any_element(),
        );
    }
    if !tiling.left {
        handles.push(
            handle(
                "resize-left",
                CursorStyle::ResizeLeftRight,
                ResizeEdge::Left,
            )
            .left_0()
            .top(inset(tiling.top))
            .bottom(inset(tiling.bottom))
            .w(strip)
            .into_any_element(),
        );
    }
    if !tiling.right {
        handles.push(
            handle(
                "resize-right",
                CursorStyle::ResizeLeftRight,
                ResizeEdge::Right,
            )
            .right_0()
            .top(inset(tiling.top))
            .bottom(inset(tiling.bottom))
            .w(strip)
            .into_any_element(),
        );
    }
    if !tiling.top && !tiling.left {
        handles.push(
            handle(
                "resize-top-left",
                CursorStyle::ResizeUpLeftDownRight,
                ResizeEdge::TopLeft,
            )
            .top_0()
            .left_0()
            .size(corner)
            .into_any_element(),
        );
    }
    if !tiling.top && !tiling.right {
        handles.push(
            handle(
                "resize-top-right",
                CursorStyle::ResizeUpRightDownLeft,
                ResizeEdge::TopRight,
            )
            .top_0()
            .right_0()
            .size(corner)
            .into_any_element(),
        );
    }
    if !tiling.bottom && !tiling.left {
        handles.push(
            handle(
                "resize-bottom-left",
                CursorStyle::ResizeUpRightDownLeft,
                ResizeEdge::BottomLeft,
            )
            .bottom_0()
            .left_0()
            .size(corner)
            .into_any_element(),
        );
    }
    if !tiling.bottom && !tiling.right {
        handles.push(
            handle(
                "resize-bottom-right",
                CursorStyle::ResizeUpLeftDownRight,
                ResizeEdge::BottomRight,
            )
            .bottom_0()
            .right_0()
            .size(corner)
            .into_any_element(),
        );
    }
    handles
}

/// Maps the window settings onto a gpui background appearance.
///
/// Blur wins when requested; failing that, any opacity below fully opaque asks
/// for a plain transparent window; otherwise the window stays opaque.
fn window_appearance(window: &WindowState) -> WindowBackgroundAppearance {
    if window.background_blur {
        WindowBackgroundAppearance::Blurred
    } else if window.background_opacity < 1.0 {
        WindowBackgroundAppearance::Transparent
    } else {
        WindowBackgroundAppearance::Opaque
    }
}

/// The application menu bar, in macOS layout.
///
/// gpui only turns this into a real menu bar on macOS — the Windows and Linux
/// backends store it and draw nothing — so the other platforms get the same
/// commands from the help menu built by [`Workspace::render_help_menu`].
/// Every item dispatches an action that is also bound to a shortcut in
/// [`bind_shortcuts`], which is what lets the macOS backend label the items with
/// their key equivalents; register the bindings first so the keymap it reads is
/// already populated.
///
/// About, Check for updates, Settings and Quit live in the application menu
/// because that is where macOS users look for them.
///
/// The item labels are translated, but the application menu's own name is the
/// "rudbgen" wordmark and stays as it is. Rebuilt and re-installed whenever the
/// language changes, because gpui takes the menu bar by value.
fn app_menus() -> Vec<Menu> {
    vec![
        Menu {
            name: APP_NAME.into(),
            items: vec![
                MenuItem::action(ts!("menu.about"), ShowAbout),
                MenuItem::action(ts!("menu.check_updates"), CheckUpdates),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.settings"), OpenSettings),
                MenuItem::separator(),
                MenuItem::action(ts!("menu.mac.quit"), Quit),
            ],
            disabled: false,
        },
        Menu {
            name: ts!("menu.connection"),
            items: vec![MenuItem::action(
                ts!("menu.mac.new_connection"),
                NewConnection,
            )],
            disabled: false,
        },
        Menu {
            name: ts!("menu.view"),
            items: vec![
                MenuItem::action(ts!("menu.toggle_explorer"), ToggleExplorer),
                MenuItem::action(ts!("menu.toggle_inspector"), ToggleInspector),
            ],
            disabled: false,
        },
    ]
}

/// Registers every shortcut the workspace listens for.
///
/// A binding here beats the focused view: gpui matches key bindings along the
/// whole dispatch path before it delivers the key event itself, so every chord
/// bound in this function is taken away from the template editor that will one
/// day be inside a pane. Only chords no editor claims are bound from here for
/// that reason.
fn bind_shortcuts(cx: &mut App) {
    let modifier = if cfg!(target_os = "macos") {
        "cmd"
    } else {
        "ctrl"
    };

    cx.bind_keys(vec![
        KeyBinding::new(&format!("{modifier}-q"), Quit, None),
        KeyBinding::new(&format!("{modifier}-n"), NewConnection, Some(KEY_CONTEXT)),
        KeyBinding::new(&format!("{modifier}-,"), OpenSettings, Some(KEY_CONTEXT)),
        // `Ctrl+B` is what every editor with a sidebar binds it to, and unlike
        // an editing chord it has no contender inside a template editor.
        KeyBinding::new(&format!("{modifier}-b"), ToggleExplorer, Some(KEY_CONTEXT)),
        // The panel on the other edge, on the letter it starts with. Nothing
        // else in the shell claims it, and no editor does either.
        KeyBinding::new(&format!("{modifier}-i"), ToggleInspector, Some(KEY_CONTEXT)),
        KeyBinding::new("escape", DismissDialog, Some(KEY_CONTEXT)),
    ]);
}

fn main() {
    env_logger::init();

    // An update the previous run could only stage — because a JVM was loaded
    // into it and Windows will not let its files be renamed — is applied here,
    // synchronously, before the application exists and therefore before anything
    // can load a JVM into *this* process. It answers `true` only when it has
    // already spawned a fresh process on the new build, at which point the one
    // useful thing left to do is get out of its way. See `update::apply_pending`.
    if update::apply_pending() {
        return;
    }

    // The icon set has to be installed before the app runs: `svg()` resolves
    // every path through this source, and the default one answers `None`.
    // `LastWindowClosed` rather than the default, which is this only away from
    // macOS: there an app whose last window closes stays in the Dock, and
    // rudbgen has nothing to offer once its window is gone — no menu bar
    // command that opens a new one, no connection or template worth keeping
    // alive in the background. One rule on every platform is what the sibling
    // applications do too.
    let app = gpui_platform::application()
        .with_assets(Icons)
        .with_quit_mode(QuitMode::LastWindowClosed);
    app.run(|cx: &mut App| {
        if let Err(error) = rudbgen_core::init_secrets() {
            log::warn!("the OS keychain is unavailable: {error}");
        }

        // A self-update renames the copies it replaces aside instead of
        // deleting them — Windows cannot delete a running image, and one code
        // path for three platforms is worth more than an immediate unlink on
        // the two that could. This is the other half: the leftovers are swept
        // up on the next launch. On the background executor because a bundled
        // JRE or a `.app` bundle is a recursive delete of thousands of files
        // and nothing on screen depends on it.
        cx.background_executor()
            .spawn(async { update::clean_leftovers() })
            .detach();

        // Load the settings before the widget layer installs its default
        // palettes, then override those to match what the user configured.
        app_settings::init(cx);
        let settings = app_settings::current(cx);
        // Ahead of everything that renders a string — the menu bar included —
        // so nothing is ever built in the wrong language and then corrected.
        i18n::apply(settings.language.as_deref());

        rudbgen_ui::init(cx);
        // The inspector's Columns tab is a grid, and the grid binds its own
        // keys; without this the panel draws and cannot be walked.
        rudbgen_grid::init(cx);
        // After the widget layers, because they scope their bindings to key
        // contexts the shell's own bindings have to be able to outrank.
        bind_shortcuts(cx);
        cx.set_menus(app_menus());

        // Before the palettes are applied: the ids in the settings may well
        // name themes of the user's own.
        theme_store::reload(cx);
        apply_themes(&settings, cx);
        // The same value `window_appearance` below reads, handed to the widget
        // layer so the widgets know whether to paint a background of their own;
        // see [`app_settings::window_tint`].
        set_window_tint(settings.window.background_opacity, cx);

        cx.on_action(|_: &Quit, cx: &mut App| cx.quit());
        // The window's geometry is only in memory until here; this is the one
        // write of `settings.json` the shell performs. Nothing in the closure
        // re-enters gpui — the file write is the whole of it — which is what
        // keeps it clear of the X11 backend's re-entrancy trap, the one the
        // vendored `client.rs` patch exists for. Quitting is no longer this
        // closure's business: gpui does it, from the quit mode set on the
        // application above, and it runs these observers first — so the
        // settings are on disk before the process starts winding down.
        cx.on_window_closed(|cx, _closed| {
            if cx.windows().is_empty() {
                app_settings::save(cx);
            }
        })
        .detach();

        let bounds = window_bounds(&settings.window, cx);
        // Read once, here: `appears_transparent` is what strips the platform
        // caption, and both Windows and macOS decide that when the window is
        // created. Changing the setting later cannot reach an open window,
        // which is why the settings dialog has to say a restart is needed.
        let titlebar = settings.window.titlebar;
        cx.open_window(
            WindowOptions {
                window_bounds: Some(bounds),
                titlebar: Some(TitlebarOptions {
                    title: Some(APP_NAME.into()),
                    appears_transparent: titlebar == TitlebarStyle::Custom,
                    // Ignored unless the caption is transparent; it moves the
                    // traffic lights AppKit keeps drawing into the title bar
                    // band the app puts in the caption's place.
                    traffic_light_position: (titlebar == TitlebarStyle::Custom)
                        .then_some(TRAFFIC_LIGHT_ORIGIN),
                }),
                // Only the Linux backends read this. `appears_transparent`
                // above means nothing to X11 and Wayland: the caption stays the
                // compositor's until the window asks for client-side
                // decorations outright. gpui falls back to server decorations
                // on its own when no compositor is present, and
                // [`draws_own_titlebar`] follows what the window actually got.
                window_decorations: (titlebar == TitlebarStyle::Custom)
                    .then_some(gpui::WindowDecorations::Client),
                app_id: Some(APP_ID.into()),
                // A translucent or blurred window needs the platform surface to
                // permit alpha; the body then tints its own background.
                window_background: window_appearance(&settings.window),
                ..Default::default()
            },
            |window, cx| {
                let workspace = cx.new(|cx| Workspace::new(titlebar, window, cx));
                let handle = workspace.read(cx).focus_handle.clone();
                window.focus(&handle, cx);
                apply_caption_theme(window, &theme(cx), cx);
                workspace
            },
        )
        .expect("failed to open the rudbgen window");

        cx.activate(true);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in registry listing: the six built-in ids, each of which a
    /// chrome theme shares a name with, plus one dark theme of the user's own
    /// that none does — which is the case the rule below has to keep straight.
    fn entries() -> Vec<EditorThemeEntry> {
        [
            ("one-dark", true, true),
            ("one-light", false, true),
            ("solarized-dark", true, true),
            ("solarized-light", false, true),
            ("gruvbox-dark", true, true),
            ("dracula", true, true),
            ("tokyo-night", true, false),
        ]
        .into_iter()
        .map(|(id, dark, builtin)| EditorThemeEntry {
            id: id.to_string(),
            name: id.to_string(),
            dark,
            builtin,
        })
        .collect()
    }

    #[test]
    fn a_pinned_editor_theme_is_left_alone() {
        // The switch is off, so nothing about the chrome may reach the editor —
        // not even a cast that clashes with it.
        assert_eq!(
            editor_theme_for("tokyo-night", false, "one-light", false, &entries()),
            "tokyo-night"
        );
    }

    #[test]
    fn following_the_ui_prefers_the_chrome_themes_namesake() {
        // The rule the switch is named for: the editor theme sharing the chrome
        // theme's id wins, whatever the settings file still carries. A dark
        // editor under a light window is dropped for the light namesake…
        assert_eq!(
            editor_theme_for("tokyo-night", true, "one-light", false, &entries()),
            "one-light"
        );
        // …and so is a dark editor under a *different* dark window: the cast
        // already matched, so the configured id would otherwise win and the
        // editor would never move off One Dark however far the chrome went.
        assert_eq!(
            editor_theme_for("one-dark", true, "dracula", true, &entries()),
            "dracula"
        );
        assert_eq!(
            editor_theme_for("dracula", true, "gruvbox-dark", true, &entries()),
            "gruvbox-dark"
        );
    }

    #[test]
    fn following_the_ui_keeps_the_configured_pick_when_the_chrome_has_no_namesake() {
        // A chrome theme of the user's own, which no editor theme is named
        // after: there is no pair to honour, so the configured editor theme
        // stands as long as its cast fits the window.
        assert_eq!(
            editor_theme_for("tokyo-night", true, "my-chrome", true, &entries()),
            "tokyo-night"
        );
    }

    #[test]
    fn following_the_ui_refuses_a_namesake_of_the_wrong_cast() {
        // The one thing that outranks the name match. A user has written a dark
        // editor theme under the id of a light chrome theme; pairing them would
        // put a dark editor in a light window, which is the accident this switch
        // exists to prevent, so the namesake is passed over.
        let mut entries = entries();
        entries.push(EditorThemeEntry {
            id: "my-chrome".to_string(),
            name: "Mine".to_string(),
            dark: true,
            builtin: false,
        });
        assert_eq!(
            editor_theme_for("solarized-light", true, "my-chrome", false, &entries),
            "solarized-light"
        );
    }

    #[test]
    fn following_the_ui_falls_back_to_any_theme_of_the_right_cast() {
        // A chrome theme with no editor theme of its name — a palette the user
        // wrote themselves, say — still has to produce a light editor.
        assert_eq!(
            editor_theme_for("one-dark", true, "my-light-theme", false, &entries()),
            "one-light"
        );
    }

    #[test]
    fn following_the_ui_keeps_the_configured_id_when_nothing_matches() {
        // Nothing of the right cast exists, so there is no better answer than
        // the id the settings already carry; resolving it falls back on its own.
        let only_dark = vec![EditorThemeEntry {
            id: "one-dark".to_string(),
            name: "One Dark".to_string(),
            dark: true,
            builtin: true,
        }];
        assert_eq!(
            editor_theme_for("one-dark", true, "one-light", false, &only_dark),
            "one-dark"
        );
        // And an empty registry cannot make one up either.
        assert_eq!(
            editor_theme_for("whatever", true, "one-light", false, &[]),
            "whatever"
        );
    }

    #[test]
    fn ids_are_matched_case_insensitively() {
        // `settings.json` is hand-editable and the registries resolve ids
        // case-insensitively, so this rule has to as well.
        assert_eq!(
            editor_theme_for("One-Dark", true, "irrelevant", true, &entries()),
            "one-dark"
        );
    }

    #[test]
    fn every_word_the_shell_draws_is_translated() {
        // `t!` answers with the key path when a key is missing, so a typo
        // reaches the screen as "welcome.taglne".
        for label in [
            ts!("welcome.tagline"),
            ts!("welcome.hint", shortcut = "Ctrl+N"),
            ts!("welcome.new_connection"),
            ts!("welcome.import_jdbgen"),
            ts!("welcome.open_template"),
            ts!("welcome.saved"),
            ts!("welcome.empty"),
            ts!("welcome.tip_soon"),
            ts!("titlebar.no_connection"),
            ts!("titlebar.tip_settings"),
            ts!("titlebar.tip_help"),
            ts!("statusbar.no_connection"),
            ts!("statusbar.no_selection"),
            ts!("statusbar.tables_selected", count = 2),
            ts!("menu.new_connection"),
            ts!("menu.settings"),
            ts!("menu.toggle_explorer"),
            ts!("menu.toggle_inspector"),
            ts!("menu.check_updates"),
            ts!("menu.about"),
            ts!("menu.quit"),
            ts!("menu.connection"),
            ts!("menu.view"),
            ts!("menu.mac.new_connection"),
            ts!("menu.mac.quit"),
        ] {
            assert!(!label.is_empty(), "empty label");
            for namespace in ["welcome.", "titlebar.", "statusbar.", "menu."] {
                assert!(
                    !label.starts_with(namespace),
                    "untranslated label {label:?}"
                );
            }
        }
        // The welcome screen shows one heading or the other, never both, so a
        // shared wording would make the two states indistinguishable.
        assert_ne!(ts!("welcome.saved"), ts!("welcome.empty"));
    }

    #[test]
    fn a_maximized_window_is_restored_maximized() {
        // The bounds a maximized window carries are its *restore* size, so both
        // halves have to survive: the state, and the size to un-maximize to.
        let state = WindowState {
            x: Some(10),
            y: Some(20),
            width: 1280,
            height: 720,
            maximized: true,
            ..WindowState::default()
        };
        let geometry = WindowGeometry::saved(&state).expect("the position is set");
        assert_eq!(geometry.bounds().size.width, px(1280.));
        assert_eq!(geometry.bounds().origin.x, px(10.));
        assert!(state.maximized);
    }

    #[test]
    fn the_window_appearance_follows_the_settings() {
        let opaque = WindowState::default();
        assert_eq!(
            window_appearance(&opaque),
            WindowBackgroundAppearance::Opaque
        );

        let translucent = WindowState {
            background_opacity: 0.8,
            ..WindowState::default()
        };
        assert_eq!(
            window_appearance(&translucent),
            WindowBackgroundAppearance::Transparent
        );

        // Blur wins even at full opacity: it is the stronger request, and a
        // blurred surface has to permit alpha whatever the fill does.
        let blurred = WindowState {
            background_blur: true,
            ..WindowState::default()
        };
        assert_eq!(
            window_appearance(&blurred),
            WindowBackgroundAppearance::Blurred
        );
    }

    /// The window opens, and the welcome screen is what it opens on.
    ///
    /// The whole of M0 in one assertion: no dialog is up, the shell holds the
    /// keyboard, and the button the architecture document's §4.3 puts first is
    /// in the tree.
    #[gpui::test]
    fn the_window_opens_on_the_welcome_screen(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            app_settings::init(cx);
            rudbgen_ui::init(cx);
        });
        let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

        window
            .update(cx, |workspace, window, cx| {
                assert!(!workspace.dialog_open(cx), "a dialog opened by itself");
                assert!(!workspace.menu_open);
                // What `main` does once the window exists, and what every
                // dialog's close returns to: the shell holds the keyboard, so
                // the shortcuts are live before anything has been clicked.
                workspace.focus_shell(window, cx);
                assert!(workspace.focus_handle.is_focused(window));
            })
            .expect("the window is open");
        cx.run_until_parked();

        let mut cx = gpui::VisualTestContext::from_window(*std::ops::Deref::deref(&window), cx);
        cx.run_until_parked();
        assert!(
            cx.debug_bounds(WELCOME_NEW_SELECTOR).is_some(),
            "the welcome screen drew no way in"
        );
    }

    /// M2's whole promise, against a real database: connect, tick, read.
    ///
    /// A real JVM, a real driver and a real H2 — the same route
    /// [`connection::tests`] takes — because everything below this line has
    /// already been tested against fixtures, and what is left to find out is
    /// whether the three fetches the shell makes ask `rudbgen-meta` for the
    /// right things and put the answers where the panels look for them.
    ///
    /// No window is on screen in a test, but the panels are laid out: the row
    /// list the tree reports is rebuilt on a draw, so asserting on it is
    /// asserting about what a user would see.
    #[gpui::test]
    fn a_real_database_fills_the_tree_and_the_inspector(cx: &mut gpui::TestAppContext) {
        use rudbgen_jdbc::StatementSpec;

        let profile = connection::h2::profile("explorer");
        let driver = connection::h2::driver();
        let session = connection::connect(
            &profile,
            &driver,
            &Credentials::typed(Some(String::new()), None),
            &AppSettings::default(),
        )
        .expect("H2 opens an in-memory database without a server");

        for sql in [
            "create table T_SAMPLE_ARTIST (               ARTIST_ID integer not null,               NAME varchar(80) not null,               constraint PK_ARTIST primary key (ARTIST_ID))",
            "create table T_SAMPLE_ALBUM (               ALBUM_ID integer not null,               ARTIST_ID integer not null,               TITLE varchar(120),               constraint PK_ALBUM primary key (ALBUM_ID),               constraint FK_ALBUM_ARTIST foreign key (ARTIST_ID)                 references T_SAMPLE_ARTIST (ARTIST_ID))",
            "comment on table T_SAMPLE_ALBUM is 'an album'",
        ] {
            session
                .session()
                .execute(&StatementSpec::new(sql))
                .unwrap_or_else(|error| panic!("{sql}: {error}"));
        }

        cx.update(|cx| {
            app_settings::init(cx);
            rudbgen_ui::init(cx);
            rudbgen_grid::init(cx);
        });
        let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));
        window
            .update(cx, |workspace, _window, cx| {
                workspace.connection = ConnectionState::Open {
                    profile: Box::new(profile.clone()),
                    driver: Box::new(driver.clone()),
                    session: Box::new(session),
                };
                // What `connected` does once the state is in place: the panels
                // are emptied, and the tree asks for its root on the next draw.
                workspace.reset_panels(cx);
                assert!(workspace.explorer_showing());
                assert!(workspace.inspector_showing());
            })
            .expect("the window is open");
        cx.run_until_parked();

        // The schemas arrived and PUBLIC is among them.
        let public = window
            .update(cx, |workspace, _window, cx| {
                let rows = workspace.explorer.read(cx).row_ids(cx);
                rows.into_iter()
                    .flatten()
                    .find_map(|id| match id {
                        explorer::NodeId::Schema(key) if key.name == "PUBLIC" => Some(key),
                        _ => None,
                    })
                    .expect("H2 answers with a PUBLIC schema")
            })
            .expect("the window is open");

        window
            .update(cx, |workspace, _window, cx| {
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.expand(&explorer::NodeId::Schema(public.clone()), cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();

        // Both tables, and only the tables. The keys come off the panel rather
        // than being spelled out here: H2 names the catalog after the URL, and
        // what a key *is* is the driver's answer and not this test's guess.
        let keys = window
            .update(cx, |workspace, _window, cx| {
                workspace.explorer.read(cx).visible_tables(cx)
            })
            .expect("the window is open");
        assert_eq!(
            keys.iter().map(|key| key.name.clone()).collect::<Vec<_>>(),
            vec!["T_SAMPLE_ALBUM".to_string(), "T_SAMPLE_ARTIST".to_string()]
        );
        let album = keys
            .into_iter()
            .find(|key| key.name == "T_SAMPLE_ALBUM")
            .expect("the album is a row");

        // Ticking the schema ticks both, which is what the status bar counts.
        window
            .update(cx, |workspace, _window, cx| {
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.toggle_tick(&explorer::NodeId::Schema(public.clone()), cx);
                });
                assert_eq!(workspace.explorer.read(cx).selected_count(cx), 2);
            })
            .expect("the window is open");
        cx.run_until_parked();

        // Moving the cursor onto a row points the inspector at it, and the
        // fetch behind that is the shell's.
        window
            .update(cx, |workspace, _window, cx| {
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.select(explorer::NodeId::Table(album.clone()), cx);
                });
            })
            .expect("the window is open");
        cx.run_until_parked();

        window
            .update(cx, |workspace, _window, cx| {
                let panel = workspace.inspector.read(cx);
                let table = panel.table().expect("the inspector read the table");
                assert_eq!(table.name, "T_SAMPLE_ALBUM");
                assert_eq!(table.remarks, "an album");
                assert_eq!(
                    table
                        .columns
                        .iter()
                        .map(|column| column.name.clone())
                        .collect::<Vec<_>>(),
                    vec!["ALBUM_ID", "ARTIST_ID", "TITLE"]
                );
                // The primary key, and the foreign key out of it.
                assert_eq!(
                    table
                        .keys()
                        .iter()
                        .map(|column| column.name.clone())
                        .collect::<Vec<_>>(),
                    vec!["ALBUM_ID"]
                );
                assert_eq!(table.imports.len(), 1, "the foreign key is missing");
                assert_eq!(table.imports[0].ref_table, "T_SAMPLE_ARTIST");
            })
            .expect("the window is open");

        // Disconnecting takes the session, the tree and the ticks with it.
        window
            .update(cx, |workspace, _window, cx| {
                workspace.disconnect(cx);
                assert_eq!(workspace.explorer.read(cx).selected_count(cx), 0);
                assert!(!workspace.explorer_showing());
            })
            .expect("the window is open");
        cx.run_until_parked();
    }

    /// The two panels of §4.2 are switches, and the switches are remembered.
    ///
    /// Both of them are also out of the frame while nothing is connected, which
    /// is the state every test here runs in: the assertion is about the switch
    /// and the setting behind it, not about pixels.
    #[gpui::test]
    fn the_panels_toggle_and_the_layout_is_remembered(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            app_settings::init(cx);
            rudbgen_ui::init(cx);
        });
        let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

        window
            .update(cx, |workspace, window, cx| {
                // Both start where the settings left them, which on a first run
                // is on: a workbench whose tree is hidden until it is found in a
                // menu is a workbench that looks like it has none.
                assert!(workspace.explorer_visible);
                assert!(workspace.inspector_visible);
                // ...and out of the frame regardless, with nothing connected.
                assert!(!workspace.explorer_showing());
                assert!(!workspace.inspector_showing());

                workspace.toggle_explorer_action(&ToggleExplorer, window, cx);
                assert!(!workspace.explorer_visible);
                assert!(
                    !app_settings::current(cx).explorer_visible,
                    "the switch was flipped and not written down"
                );
                // The keyboard comes back in the same update the subtree went
                // in; see [`Workspace::hid_a_panel`].
                assert!(workspace.focus_handle.is_focused(window));

                workspace.toggle_inspector_action(&ToggleInspector, window, cx);
                assert!(!workspace.inspector_visible);
                assert!(!app_settings::current(cx).inspector_visible);

                workspace.toggle_explorer_action(&ToggleExplorer, window, cx);
                assert!(workspace.explorer_visible);
                assert!(app_settings::current(cx).explorer_visible);
            })
            .expect("the window is open");
    }

    /// The status bar counts the ticks, wherever they were made.
    #[gpui::test]
    fn the_status_bar_counts_the_ticked_tables(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            app_settings::init(cx);
            rudbgen_ui::init(cx);
        });
        let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

        window
            .update(cx, |workspace, _window, cx| {
                assert_eq!(workspace.explorer.read(cx).selected_count(cx), 0);
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.deliver_schemas(
                        Ok(vec![rudbgen_meta::Schema {
                            catalog: String::new(),
                            schema: "PUBLIC".to_string(),
                            name: "PUBLIC".to_string(),
                        }]),
                        cx,
                    );
                });
            })
            .expect("the window is open");
        cx.run_until_parked();

        window
            .update(cx, |workspace, _window, cx| {
                let key = SchemaKey {
                    catalog: String::new(),
                    schema: "PUBLIC".to_string(),
                    name: "PUBLIC".to_string(),
                };
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.deliver_tables(
                        key,
                        Ok(vec![rudbgen_meta::TableRef {
                            catalog: String::new(),
                            schema: "PUBLIC".to_string(),
                            name: "T_SAMPLE_ALBUM".to_string(),
                            kind: rudbgen_meta::KIND_TABLE.to_string(),
                            ..rudbgen_meta::TableRef::default()
                        }]),
                        cx,
                    );
                });
                workspace.explorer.update(cx, |explorer, cx| {
                    explorer.toggle_tick(
                        &explorer::NodeId::Table(TableKey {
                            catalog: String::new(),
                            schema: "PUBLIC".to_string(),
                            name: "T_SAMPLE_ALBUM".to_string(),
                        }),
                        cx,
                    );
                });
                assert_eq!(workspace.explorer.read(cx).selected_count(cx), 1);
                assert_ne!(
                    ts!("statusbar.tables_selected", count = 1),
                    ts!("statusbar.no_selection"),
                    "the bar would say the same thing either way"
                );
            })
            .expect("the window is open");
    }

    /// Every command the shell offers reaches the workspace, and `Escape`
    /// closes what the command opened.
    #[gpui::test]
    fn the_dialogs_open_from_their_actions_and_close_on_escape(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            app_settings::init(cx);
            rudbgen_ui::init(cx);
        });
        let window = cx.add_window(|window, cx| Workspace::new(TitlebarStyle::Custom, window, cx));

        window
            .update(cx, |workspace, window, cx| {
                workspace.open_about(window, cx);
                assert!(workspace.about.read(cx).is_open());
                // One at a time: opening the settings dialog puts the about box
                // away rather than stacking on top of it.
                workspace.open_settings(window, cx);
                assert!(!workspace.about.read(cx).is_open());
                assert!(workspace.settings.read(cx).is_open());

                workspace.dismiss_dialog_action(&DismissDialog, window, cx);
                assert!(
                    !workspace.settings.read(cx).is_open(),
                    "escape left the settings dialog up"
                );
                assert!(!workspace.dialog_open(cx));
            })
            .expect("the window is open");
        cx.run_until_parked();
    }
}

/// What the welcome screen's box does when its column outgrows the window.
///
/// Only [`centered_scroll`] is put under test, and only through what its scroll
/// handle reports: the arrangement is entirely a question of layout, and the
/// handle is where gpui writes down the answer — the box it measured, and how
/// far past it the column ran.
#[cfg(test)]
mod centered_scroll_tests {
    use std::ops::Deref;

    use gpui::{TestAppContext, VisualTestContext, point};

    use super::*;

    /// Height of the stand-in column.
    ///
    /// Nothing about the real welcome screen's contents matters here — only that
    /// there is a definite height to hold the window against — so the test hands
    /// the box one plain child rather than rebuilding the screen.
    const COLUMN: f32 = 400.;

    /// A window tall enough for the column and both its margins, several times
    /// over.
    const ROOMY: f32 = 900.;

    /// A window shorter than the column, which is the whole point of the box.
    const CRAMPED: f32 = 300.;

    /// Wide enough that nothing wraps; the box only scrolls one way.
    const WIDTH: f32 = 600.;

    /// How far apart two measurements may be and still count as the same, in a
    /// layout whose lengths are rounded to hundredths of a pixel.
    const SLACK: f32 = 0.5;

    /// A window holding nothing but the box under test.
    struct Harness {
        scroll: ScrollHandle,
        bar: ScrollbarState,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            let theme = Theme::dark();
            let bar =
                Scrollbar::for_handle(WELCOME_SCROLLBAR, ScrollbarAxis::Vertical, &self.scroll)
                    .fade(self.bar.fade());

            div().flex().flex_col().size_full().child(centered_scroll(
                WELCOME_STATE,
                &self.scroll,
                bar,
                &theme,
                div().flex_none().w(px(320.)).h(px(COLUMN)),
            ))
        }
    }

    /// Opens the harness in a window `height` tall and hands back its handle.
    ///
    /// Drawn twice: a bar is built from the box as the previous frame measured
    /// it, so the opening frame has nothing to build one out of.
    fn open(cx: &mut TestAppContext, height: f32) -> ScrollHandle {
        let scroll = ScrollHandle::new();
        let window = cx.add_window({
            let scroll = scroll.clone();
            move |_, _| Harness {
                scroll,
                bar: ScrollbarState::new(),
            }
        });

        let mut cx = VisualTestContext::from_window(*window.deref(), cx);
        cx.simulate_resize(size(px(WIDTH), px(height)));
        cx.run_until_parked();
        cx.update(|window, _| window.refresh());
        cx.run_until_parked();

        scroll
    }

    /// The bar the workspace would draw over the box as it now stands.
    fn scrollbar(scroll: &ScrollHandle) -> Scrollbar {
        Scrollbar::for_handle(WELCOME_SCROLLBAR, ScrollbarAxis::Vertical, scroll)
    }

    /// With room to spare the column sits in the middle, exactly where
    /// `justify_center` used to put it, and there is nothing to scroll — so no
    /// bar is drawn either.
    #[gpui::test]
    fn a_column_that_fits_stays_in_the_middle(cx: &mut TestAppContext) {
        let scroll = open(cx, ROOMY);
        let box_ = scroll.bounds();
        let column = scroll
            .bounds_for_item(0)
            .expect("the box never measured its column");

        let above = f32::from(column.top() - box_.top());
        let below = f32::from(box_.bottom() - column.bottom());
        assert!(
            (above - below).abs() < SLACK,
            "the column was not centred: {above} above, {below} below"
        );
        assert_eq!(
            scroll.max_offset().y,
            px(0.),
            "a column that fits left something to scroll"
        );
        assert!(
            scrollbar(&scroll).thumb().is_none(),
            "a box with nothing to scroll drew a bar anyway"
        );
    }

    /// With less room than the column needs it starts at the top of the box,
    /// and everything past the bottom is reachable by scrolling.
    #[gpui::test]
    fn a_column_that_does_not_fit_starts_at_the_top(cx: &mut TestAppContext) {
        let scroll = open(cx, CRAMPED);
        let box_ = scroll.bounds();
        let column = scroll
            .bounds_for_item(0)
            .expect("the box never measured its column");

        assert!(
            f32::from(column.top() - box_.top()).abs() < SLACK,
            "the column did not start at the top of the box: {:?} in {:?}",
            column,
            box_
        );
        assert!(
            (f32::from(scroll.max_offset().y) - f32::from(column.size.height - box_.size.height))
                .abs()
                < SLACK,
            "the scrollable range did not cover the whole of the column"
        );
        assert!(
            scrollbar(&scroll).thumb().is_some(),
            "a box with something to scroll drew no bar"
        );
    }

    /// And the far end of that scroll reaches the foot of the column, margin and
    /// all, rather than stopping short of the last button.
    #[gpui::test]
    fn scrolling_to_the_end_reaches_the_foot_of_the_column(cx: &mut TestAppContext) {
        let scroll = open(cx, CRAMPED);
        scroll.set_offset(point(px(0.), -scroll.max_offset().y));
        let box_ = scroll.bounds();
        let column = scroll
            .bounds_for_item(0)
            .expect("the box never measured its column");

        let foot = column.bottom() + scroll.offset().y;
        assert!(
            f32::from(foot - box_.bottom()).abs() < SLACK,
            "the end of the scroll left {:?} of the column below the box",
            foot - box_.bottom()
        );
        assert!(
            f32::from(column.size.height) > COLUMN + SCROLL_MARGIN,
            "the column was scrolled to its last button rather than past it"
        );
    }
}
