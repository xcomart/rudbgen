//! rudbgen — a database code generator.
//!
//! Point it at a database, tick the tables, tick the templates, and it writes
//! one file per table per template. This crate is the binary: the window, the
//! chrome around it, the dialogs that hang over it, and the bootstrap sequence
//! that gets all three on screen. Everything it draws with comes from `rugpui`;
//! everything it persists goes through `rudbgen-core`.
//!
//! # What the shell supplies
//!
//! `rugpui-shell` is the layer above the widgets, and a good deal of what used to
//! be here is now there: the window frame a self-decorated window has to draw
//! for itself, the self-updater and its dialog, the about box, the palette
//! catalogues and their colour editor, the split-pane tree, the context-menu
//! rows and the pieces a settings form is built out of. It knows nothing about
//! rudbgen, so [`run`] hands it three things before the first window opens —
//! [`IDENTITY`], [`AppStrings`] and [`IgnoredUpdate`] — and takes back the one
//! decision a dialog must not make on its own: a finished update ends in
//! [`UpdateDialogEvent::Installed`], and the restart is the [`Workspace`]'s.
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
//! The shell, the dialogs, the connection behind them and the run. The welcome
//! screen's saved rows open a session, `Ctrl+N` opens the connection dialog, the
//! driver editor inside it edits the four custom queries and tests them (D9),
//! and a connected window is §4.2's three columns: the explorer, the tabbed work
//! area with the Generate tab in it, and the inspector. The status bar carries
//! the arithmetic of the run and the three buttons that start one.
//!
//! A template is a document: the pencil beside a template row, a double click
//! on it, and the welcome screen's *Open a template* all open one in a tab of
//! its own — the editor, the live preview beside it and the variable palette in
//! the inspector's column (D12, §4.5). What is still to come is the jdbgen
//! import (M5), which is drawn disabled with a tooltip that says so rather than
//! left out: a way in that is missing tells the reader nothing about what the
//! application will do, and a button that looks live and does nothing is worse
//! than either.

pub(crate) use bootstrap::editor_theme_for;

use crate::{
    CheckUpdates, DismissDialog, DryRun, EditAbbreviations, Generate, ImportJdbgen, NewConnection,
    OpenSettings, OpenTemplate, Preview, Quit, SaveTemplate, ShowAbout, ToggleExplorer,
    ToggleInspector, ToggleLivePreview, TriggerCompletion, app_settings, builtin_templates,
    connection, generate_pane, i18n, icons, palette, sample_db,
};

mod actions;
mod bootstrap;
use bootstrap::{app_menus, apply_themes, chrome_style, record_window_geometry, window_appearance};
mod database;
mod dialogs;
mod generation;
mod layout;
mod templates;
mod view;
#[cfg(test)]
use view::centered_scroll;

use std::path::{Path, PathBuf};

use gpui::{
    AnyElement, App, Axis, Context, Div, DragMoveEvent, Entity, FocusHandle, Hsla, KeyBinding,
    Menu, MenuItem, MouseButton, MouseUpEvent, Pixels, Point, QuitMode, ScrollHandle, SharedString,
    Subscription, Task, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowOptions, div, img, prelude::*, px,
};
use rudbgen_core::{
    AppSettings, ConnectionProfile, ConnectionStore, DriverDef, DriverStore, TemplateSetStore,
    TitlebarStyle, WindowState,
};
use rudbgen_gen::{Plan, TemplateSpec};
use rudbgen_meta::{MetaReader, Table};
use rugpui::{
    Button, ButtonVariant, DraggedThumb, EditorThemeEntry, EditorThemeRegistry, MenuButton,
    MenuEntry, ResizeHandle, Scrollbar, ScrollbarAxis, ScrollbarState, Select, TabBar, TabItem,
    Theme, ThemeRegistry, hide_later, hide_now, scroll_to, scrolled, set_editor_theme, set_theme,
    set_window_tint, theme, tooltip_label,
};
use rugpui_shell::{
    AboutDialog, AboutDialogEvent, AppIdentity, Pane, UpdateDialog, UpdateDialogEvent,
    apply_caption_theme, chrome, menu_rows, update,
};

use crate::abbreviation_dialog::{AbbreviationDialog, AbbreviationDialogEvent};
use crate::connection::{ConnectError, Connected, Credentials, SessionHandle};
use crate::connection_dialog::{ConnectionDialog, ConnectionDialogEvent};
use crate::explorer::{Explorer, ExplorerEvent, SchemaKey, TableKey};
use crate::generate_job::{GenerationJob, JobEvent};
use crate::generate_pane::{Blocker, GeneratePane, GeneratePaneEvent};
use crate::i18n::ts;
use crate::import_dialog::{ImportDialog, ImportDialogEvent};
use crate::inspector::{Inspector, InspectorEvent};
use crate::pane_item::{PaneItem, WorkTabs};
use crate::preview_pane::{PreviewEvent, PreviewFile, PreviewPane};
use crate::settings_dialog::{SettingsDialog, SettingsDialogEvent};
use crate::template_pane::{PreviewOutcome, TemplatePane, TemplatePaneEvent};
use crate::variable_palette::{PaletteEvent, VariablePalette};

/// Which of the three things the status bar's buttons start.
///
/// The three are one code path — load the ticked tables, build the plan — and
/// differ only in what is done with the plan afterwards, which is what this
/// picks out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum RunKind {
    /// Render every pair and write it (§9).
    Generate,
    /// Render every pair into memory and show the file list.
    DryRun,
    /// Render one pair and show its text.
    Preview,
}

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

/// Theme the import wizard picks when jdbgen was last shown dark.
///
/// jdbgen has two appearances and rudbgen has six, so the hint can only pick a
/// side; these two are the pair the settings dialog itself defaults to.
const DARK_THEME: &str = "one-dark";

/// Theme the import wizard picks when jdbgen was last shown light.
const LIGHT_THEME: &str = "one-light";

/// Application id published to the desktop.
///
/// Wayland compositors and X11 docks match it against a `.desktop` file of the
/// same name to pick up the application icon, so `packaging/linux` has to ship
/// `com.aihouse.rudbgen.desktop` and nothing else.
const APP_ID: &str = "com.aihouse.rudbgen";

/// What a release archive holds that has to end up on disk, in install order.
///
/// The macOS archive carries one thing, the whole application bundle, and
/// everything rudbgen loads at runtime is inside it. The Windows and Linux
/// archives carry the executable and the two directories it resolves relative
/// to itself, and all three move together.
///
/// The first entry is always the executable (or the bundle), because that is
/// the one whose *installed* name may differ from the published one — a binary
/// someone renamed still updates, and the directories beside it never are.
#[cfg(windows)]
const PAYLOAD: &[&str] = &["rudbgen.exe", "lib", "runtime"];
/// See the Windows variant above.
#[cfg(target_os = "macos")]
const PAYLOAD: &[&str] = &["rudbgen.app"];
/// See the Windows variant above.
#[cfg(all(unix, not(target_os = "macos")))]
const PAYLOAD: &[&str] = &["rudbgen", "lib", "runtime"];

/// The "Apps & features" entry the Windows installer leaves behind, relative to
/// `HKEY_CURRENT_USER` or `HKEY_LOCAL_MACHINE`.
///
/// The GUID in the middle of it is one corner of a triangle that has to agree,
/// and it is a published identifier rather than an implementation detail:
///
/// * `packaging/windows/rudbgen.iss` sets it as Inno Setup's `AppId`, and Inno
///   derives this key's name from it by appending `_is1`;
/// * the manifests under `packaging/winget/*/` record the same string, braces
///   and suffix included, as the package's `ProductCode`;
/// * and the shell's updater is what finds the entry again by it, to correct
///   the `DisplayVersion` after a self-update.
///
/// Move any one corner without the other two and winget stops recognising an
/// installed rudbgen: `winget list` finds nothing, `winget upgrade` offers a
/// fresh install to sit beside the existing one, and `winget uninstall` has
/// nothing to remove — all silently, because a key that is not there is
/// indistinguishable from a copy that was never installed. None of the three
/// ever changes.
///
/// A GUID of rudbgen's own, minted for this repository: a product code names a
/// *product*, so sharing a sibling application's would have winget treat an
/// installed one as an installed rudbgen and offer to upgrade one into the
/// other.
const ARP_KEY: &str = concat!(
    r"Software\Microsoft\Windows\CurrentVersion\Uninstall\",
    "{9A84AE34-E54B-46EA-B55D-61C0666D6890}_is1"
);

/// Everything `rugpui-shell` has to be told about rudbgen.
///
/// Installed once in [`main`], before the first window and before anything can
/// start an update check. The shell composes none of it — it only reads — which
/// is why every field is a constant of this crate, [`AppIdentity::version`]
/// above all: `rugpui-shell` has a version of its own and it is not this one.
const IDENTITY: AppIdentity = AppIdentity {
    name: APP_NAME,
    version: env!("CARGO_PKG_VERSION"),
    repository_url: "https://github.com/xcomart/rudbgen",
    repository_label: "github.com/xcomart/rudbgen",
    latest_release_api: "https://api.github.com/repos/xcomart/rudbgen/releases/latest",
    releases_page: "https://github.com/xcomart/rudbgen/releases",
    fallback_archive: "rudbgen-update",
    payload: PAYLOAD,
    bundle_executable: "Contents/MacOS/rudbgen",
    windows_arp_key: ARP_KEY,
    // Whether an install has to leave its renames to the next launch. The
    // question is "is a JVM loaded into this process", because on Windows one
    // holds open handles on the very files the swap renames — and the answer is
    // `false` until `rudbgen-jdbc` starts one, which no path reachable from an
    // update check does.
    must_defer: || false,
};

/// The shell's window onto rudbgen's translations.
///
/// One line over `t!`, and deliberately no more: the shell looks its words up
/// by the very keys `locales/*.yml` already carries, and the `%{marker}`s come
/// back intact for it to fill in — which is what lets it interpolate an
/// application name into a sentence whose key never mentions one.
struct AppStrings;

impl rugpui_shell::Strings for AppStrings {
    fn text(&self, key: &str) -> SharedString {
        ts!(key)
    }
}

/// The shell's window onto the "never tell me about this version again" tag.
///
/// The tag lives in `settings.json`, which the shell does not own; both halves
/// run on the UI thread, so the settings global is reachable directly. Written
/// through immediately rather than at the next save: this is a decision the
/// user has just made in a dialog, and it should survive a crash the way a
/// saved setting does.
struct IgnoredUpdate;

impl rugpui_shell::UpdatePolicy for IgnoredUpdate {
    fn ignored(&self, cx: &App) -> Option<String> {
        app_settings::current(cx).ignored_update
    }

    fn set_ignored(&self, tag: Option<String>, cx: &mut App) {
        let mut settings = app_settings::current(cx);
        settings.ignored_update = tag;
        app_settings::replace(settings, cx);
        app_settings::save(cx);
    }
}

/// Width of the grab area between two panels of the body, in logical pixels.
///
/// Laid over the panel's own edge rather than wedged beside it, so the band
/// takes none of the row's width and the work area does not shift by six pixels
/// every time a panel appears; see [`Workspace::render_work_area`].
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

/// Debug selector of the welcome screen's "import from jdbgen" button.
const WELCOME_IMPORT_SELECTOR: &str = "welcome-import";

/// Debug selector of the welcome screen's "open a template" button.
const WELCOME_TEMPLATE_SELECTOR: &str = "welcome-template";

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
    /// The abbreviation rules editor, rendered only while it reports itself
    /// open (§4.6, D7).
    ///
    /// Opened from the Generate tab's *Rules…* and from the menu, filled from
    /// the Generate tab's copy of the store and handing it back on `Save`, so
    /// that the one `apply_to_names` switch has two controls and no second
    /// value.
    abbreviations: Entity<AbbreviationDialog>,
    /// The jdbgen import wizard, rendered only while it reports itself open.
    import: Entity<ImportDialog>,
    /// The jdbgen configuration the welcome screen offers to import, if there
    /// is one.
    ///
    /// Read once, at start-up: §4.3 puts *Import from jdbgen…* on the welcome
    /// screen only when the file is actually there, and a `stat` on every frame
    /// to answer a question whose answer cannot change while the window is open
    /// would be a strange way to pay for it. The **menu** row is live either
    /// way — a configuration copied from another machine is chosen from inside
    /// the wizard, and that door must not depend on this answer.
    jdbgen_config: Option<PathBuf>,
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
    /// The Generate tab: the whole of what a run will be made of (§4.4).
    generate: Entity<GeneratePane>,
    /// The Preview tab's contents, when there is one.
    preview: Entity<PreviewPane>,
    /// One editor per open template file, in no particular order.
    ///
    /// The tab strip holds the paths and this holds the buffers, which is what
    /// lets a template tab survive a reconnection: the strip is rebuilt when
    /// the connection changes and these are not (§4.2 — switching the
    /// connection never closes an open template).
    templates: Vec<Entity<TemplatePane>>,
    /// Keeps the template tabs' subscriptions alive, one per entry above.
    template_events: Vec<Subscription>,
    /// The variable palette, which replaces the inspector while a template tab
    /// is on top (§4.5).
    palette: Entity<VariablePalette>,
    /// The tab a close is waiting on an answer about, when one is.
    closing: Option<usize>,
    /// The progress dialog, the overwrite question and the summary (D11).
    job: Entity<GenerationJob>,
    /// The work area's tab strip.
    ///
    /// A [`Pane`] rather than a [`rugpui_shell::PaneTree`]: there is one pane,
    /// and the template tab's side-by-side preview is a split *inside* the tab
    /// rather than a second pane beside it.
    pane: Pane<PaneItem>,
    /// Whether the tab strip's dropdown is showing.
    pane_menu_open: bool,
    /// The ticked tables, as the last run read them.
    ///
    /// Kept so that moving the preview header's dropdowns re-renders without
    /// going back to the database: the tables are what a render needs, and they
    /// have not changed because the user picked a different one of them. Emptied
    /// by every connect and disconnect, with everything else that belongs to a
    /// database (see [`Workspace::reset_panels`]).
    run_tables: Vec<Table>,
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
    /// Keeps the rules editor's subscription alive.
    _abbreviation_events: Subscription,
    /// Keeps the import wizard's subscription alive.
    _import_events: Subscription,
    /// Keeps the explorer's subscription alive.
    _explorer_events: Subscription,
    /// Keeps the inspector's subscription alive.
    _inspector_events: Subscription,
    /// Keeps the Generate tab's subscription alive.
    _generate_events: Subscription,
    /// Keeps the Preview tab's subscription alive.
    _preview_events: Subscription,
    /// Keeps the variable palette's subscription alive.
    _palette_events: Subscription,
    /// Keeps the generation dialog's subscription alive.
    _job_events: Subscription,
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

        let abbreviations = cx.new(AbbreviationDialog::new);
        let abbreviation_events = cx.subscribe_in(
            &abbreviations,
            window,
            |this, dialog, event, window, cx| match event {
                // The dialog has already written `abbreviations.json`. What is
                // left is the copy the Generate tab holds — the panel is what a
                // run reads the rules from, and re-reading the file here would
                // be a second answer to a question already answered.
                AbbreviationDialogEvent::Saved(store) => {
                    let store = (**store).clone();
                    this.generate.update(cx, |pane, cx| {
                        pane.adopt_abbreviations(store, cx);
                    });
                    dialog.update(cx, |dialog, cx| dialog.close(cx));
                    this.focus_shell(window, cx);
                }
                AbbreviationDialogEvent::Dismissed => {
                    dialog.update(cx, |dialog, cx| dialog.close(cx));
                    this.focus_shell(window, cx);
                }
            },
        );

        let import = cx.new(ImportDialog::new);
        let import_events =
            cx.subscribe_in(
                &import,
                window,
                |this, dialog, event, window, cx| match event {
                    // The wizard has already written the four stores and the
                    // keychain; the shell re-reads the connection list the welcome
                    // screen draws, and applies jdbgen's language and theme when
                    // the user ticked that box — settings are the shell's, never a
                    // dialog's.
                    ImportDialogEvent::Imported(hint) => {
                        this.profiles = load_profiles();
                        // The wizard wrote `abbreviations.json`; the Generate
                        // tab holds the copy a run reads the rules from, and it
                        // only re-reads the file when a connection opens.
                        match rudbgen_core::AbbreviationStore::load() {
                            Ok(store) => this.generate.update(cx, |pane, cx| {
                                pane.adopt_abbreviations(store, cx);
                            }),
                            Err(error) => {
                                log::error!("could not re-read abbreviations.json: {error:#}");
                            }
                        }
                        if let Some(hint) = hint {
                            this.adopt_jdbgen_settings(hint, window, cx);
                        }
                        cx.notify();
                    }
                    ImportDialogEvent::Dismissed => {
                        dialog.update(cx, |dialog, cx| dialog.close(cx));
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
                UpdateDialogEvent::Installed(_) => {
                    // The new build is on disk and the restart is the
                    // application's to perform: `cx.restart()` spawns a watcher
                    // that waits for this pid to exit and starts rudbgen again
                    // from the same path, which by then holds what was
                    // installed. The dialog stays on screen — the process is
                    // about to go, and closing it first would flash the window
                    // back into view for a fraction of a second.
                    //
                    // The path has to be pointed at explicitly: `install`
                    // renames the running image aside and writes the new build
                    // under the old name, and on Linux `current_exe()` follows
                    // the *inode* into the renamed-away copy, so a bare
                    // `cx.restart()` would come back up on the build that was
                    // just replaced. `restart_path()` is the path as it stood
                    // before anything moved, taken by `init_process_identity`
                    // at the top of `main`; where the platform never answered
                    // one, leaving `set_restart_path` uncalled is gpui's own
                    // default and behaves the same as before this build.
                    if let Some(path) = rugpui_shell::restart_path() {
                        cx.set_restart_path(path);
                    }
                    cx.restart();
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
        // The guard is here, in this crate, rather than inside the check:
        // `cfg!(test)` compiled into a dependency is that dependency's build,
        // so `rugpui_shell::update::check` cannot tell a test build of *this*
        // crate from a release one, and every render test that opens a window
        // would make a real request to GitHub.
        //
        // `rugpui_shell::update::set_startup_check_enabled(false)` says the same
        // thing to the shell and is the other half of the guard; the tests
        // install it once, so that a path reaching the check some other way is
        // still silent. See [`tests::silence_the_update_check`].
        if !cfg!(test) {
            let ignored = update::ignored_release(cx);
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
        }

        // The two panels of the body. Both are built empty and stay out of the
        // frame until a session opens; see [`Workspace::render_body`].
        let explorer = cx.new(Explorer::new);
        let explorer_events = cx.subscribe(&explorer, |workspace, _explorer, event, cx| {
            match event {
                // Both of these need the session, which is the workspace's.
                ExplorerEvent::LoadSchemas => workspace.load_schemas(cx),
                ExplorerEvent::LoadTables(schema) => workspace.load_tables(schema.clone(), cx),
                // The status bar counts the ticks, and so does the list of
                // tables a live preview may be rendered against.
                ExplorerEvent::SelectionChanged => {
                    workspace.refresh_templates(cx);
                    cx.notify();
                }
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
                    // A cache hit draws without asking anybody, so no Load
                    // event follows to refresh the palette on the way back —
                    // refresh it here, for the table now in hand.
                    workspace.refresh_palette(cx);
                    cx.notify();
                }
            }
        });

        let inspector = cx.new(Inspector::new);
        let inspector_events =
            cx.subscribe(&inspector, |workspace, _panel, event, cx| match event {
                InspectorEvent::Load(table) => workspace.load_table(table.clone(), cx),
            });

        // The three views of the work area. The Generate tab is permanent and
        // is pushed here rather than on the first connection: it is what the
        // strip is, and a strip that appears only once something is connected
        // would move the whole body down the first time it did.
        let generate = cx.new(GeneratePane::new);
        // `subscribe_in` rather than `subscribe`: *Rules…* opens a modal, and
        // opening one closes whatever else is on screen and moves the keyboard,
        // both of which need the window.
        let generate_events = cx.subscribe_in(
            &generate,
            window,
            |workspace, _pane, event, window, cx| match event {
                GeneratePaneEvent::Changed => {
                    // The palette lists the custom variables, so a variable
                    // typed into the Generate tab has to reach it.
                    workspace.refresh_palette(cx);
                    cx.notify();
                }
                GeneratePaneEvent::EditTemplate(file) => {
                    let file = generate_pane::resolve_template(file);
                    workspace.open_template(file, cx);
                }
                GeneratePaneEvent::EditAbbreviations => {
                    workspace.open_abbreviations(window, cx);
                }
            },
        );

        let preview = cx.new(PreviewPane::new);
        let preview_events = cx.subscribe(&preview, |workspace, _pane, event, cx| match event {
            PreviewEvent::Reselect { table, template } => {
                workspace.render_preview(*table, *template, cx);
            }
        });

        let palette = cx.new(VariablePalette::new);
        let palette_events = cx.subscribe_in(
            &palette,
            window,
            |workspace, _panel, event, window, cx| match event {
                PaletteEvent::Insert(text) => workspace.insert_into_template(text, window, cx),
            },
        );

        let job = cx.new(GenerationJob::new);
        let job_events =
            cx.subscribe_in(&job, window, |this, _job, event, window, cx| match event {
                JobEvent::Closed => this.focus_shell(window, cx),
            });

        let mut pane = Pane::new();
        pane.push(PaneItem::Generate);

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
            abbreviations,
            import,
            jdbgen_config: rudbgen_import::locate(),
            connection: ConnectionState::Idle,
            connection_epoch: 0,
            connection_list_open: false,
            explorer,
            inspector,
            generate,
            preview,
            templates: Vec::new(),
            template_events: Vec::new(),
            palette,
            closing: None,
            job,
            pane,
            pane_menu_open: false,
            run_tables: Vec::new(),
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
            _abbreviation_events: abbreviation_events,
            _import_events: import_events,
            _explorer_events: explorer_events,
            _inspector_events: inspector_events,
            _generate_events: generate_events,
            _preview_events: preview_events,
            _palette_events: palette_events,
            _job_events: job_events,
            _quit: quit,
            _bounds: bounds,
            _button_layout: button_layout,
        }
    }
}

#[cfg(test)]
mod tests;

/// What the welcome screen's box does when its column outgrows the window.
///
/// Only [`centered_scroll`] is put under test, and only through what its scroll
/// handle reports: the arrangement is entirely a question of layout, and the
/// handle is where gpui writes down the answer — the box it measured, and how
/// far past it the column ran.
#[cfg(test)]
mod centered_scroll_tests;

pub(crate) fn run() {
    bootstrap::run();
}
