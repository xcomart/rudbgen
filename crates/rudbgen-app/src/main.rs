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
//! # What M0 is, and is not
//!
//! This is the shell and nothing behind it. The window opens, the settings
//! dialog edits every setting there is and previews the two palettes live, the
//! about box and the update check work end to end, and the welcome screen lists
//! whatever `connections.json` holds. Nothing on that screen opens a database:
//! the connection dialog and the explorer arrive in M2, the Generate tab and
//! the work area in M3, the template editor in M4, and the jdbgen import in M5.
//! The three buttons the welcome screen offers are drawn disabled with a
//! tooltip that says so, rather than left out — a way in that is missing tells
//! the reader nothing about what the application will do, and a button that
//! looks live and does nothing is worse than either.

mod about_dialog;
mod app_settings;
mod caption;
// The menu rows are written as plain data with test helpers rather than for the
// call sites the shell currently has, because no surface of the M0 window draws
// a context menu — the explorer's rows (M2) and the work area's tabs (M3) are
// what they are written for. Inside a binary crate that reads as dead code.
#[allow(dead_code)]
mod context_menu;
mod i18n;
mod icons;
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
    AnyElement, App, Bounds, Context, Div, DragMoveEvent, Entity, FocusHandle, KeyBinding, Menu,
    MenuItem, MouseButton, MouseUpEvent, Pixels, Point, QuitMode, ScrollHandle, SharedString,
    Stateful, Subscription, TitlebarOptions, Window, WindowBackgroundAppearance, WindowBounds,
    WindowControlArea, WindowOptions, actions, div, img, prelude::*, px, size,
};
use rudbgen_core::{AppSettings, ConnectionStore, TitlebarStyle, WindowState};
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
use i18n::ts;
use icons::Icons;
use settings_dialog::{SettingsDialog, SettingsDialogEvent};
use update_dialog::{UpdateDialog, UpdateDialogEvent};

actions!(
    rudbgen,
    [
        /// Leaves the application.
        Quit,
        /// Opens the connection dialog (M2).
        NewConnection,
        /// Opens the settings dialog.
        OpenSettings,
        /// Opens the about box.
        ShowAbout,
        /// Asks GitHub whether there is a newer release.
        CheckUpdates,
        /// Closes whatever overlay is on top, innermost first.
        DismissDialog,
        /// Shows and hides the explorer sidebar (M2).
        ToggleExplorer,
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

        Self {
            focus_handle: cx.focus_handle(),
            profiles: load_profiles(),
            welcome_scroll: ScrollHandle::new(),
            welcome_scrollbar: ScrollbarState::new(),
            about,
            settings,
            update,
            menu_open: false,
            titlebar,
            _about_events: about_events,
            _settings_events: settings_events,
            _update_events: update_events,
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
    ///
    /// M2: the dialog, the driver store and the session behind it are what that
    /// milestone is. Bound and dispatched from now so that the route exists and
    /// the menu row is not a lie about where the command will live.
    fn new_connection_action(
        &mut self,
        _: &NewConnection,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        log::debug!("the connection dialog arrives in M2");
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
    ///
    /// Nothing to show yet, and deliberately nothing written either: the
    /// sidebar's visibility is a setting, and flipping a setting whose effect
    /// is off screen would leave the user's next session opening in a state
    /// they never chose. The command comes to life with the tree in M2.
    fn toggle_explorer_action(
        &mut self,
        _: &ToggleExplorer,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) {
        log::debug!("the explorer arrives in M2");
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
    /// apart without opening anything. In M0 it has no options and no handler —
    /// there is no session to pick — so it stands as the placeholder the
    /// architecture document's sketch shows, greyed and inert. M2 fills it.
    fn render_connection_select(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme(cx);
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
                    // Muted rather than `danger`: nothing has failed, there is
                    // simply nothing there.
                    .bg(theme.text_muted),
            )
            .child(
                div().flex_1().min_w_0().child(
                    Select::new("connection-select")
                        .placeholder(ts!("titlebar.no_connection"))
                        .width(px(CONNECTION_SELECT_WIDTH)),
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
                // M2.
                .disabled(true)
                .on_activate(|window, cx| window.dispatch_action(Box::new(NewConnection), cx)),
            MenuEntry::new(ts!("menu.settings"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+,"))
                .on_activate(|window, cx| window.dispatch_action(Box::new(OpenSettings), cx)),
            MenuEntry::new(ts!("menu.toggle_explorer"))
                .shortcut(format!("{SHORTCUT_MODIFIER}+B"))
                // Nothing to show while no connection is open, which in M0 is
                // always.
                .disabled(true)
                .on_activate(|window, cx| window.dispatch_action(Box::new(ToggleExplorer), cx)),
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
    /// The welcome screen and nothing else, because there is no connection to
    /// draw a work area for. The explorer and the inspector are out of the
    /// frame rather than empty (architecture document, §4.3), and the tabbed
    /// work area they flank arrives with them in M2/M3.
    fn render_body(&self, cx: &mut Context<Self>) -> AnyElement {
        let theme = theme(cx);
        div()
            .flex()
            .flex_col()
            .flex_grow_1()
            .min_h_0()
            .bg(app_settings::window_tint(theme.background, cx))
            .child(self.render_welcome(&theme, cx))
            .into_any_element()
    }

    /// The welcome screen: the name, what the application is for, the three
    /// ways in, and the connections already saved.
    ///
    /// None of the three buttons does anything yet, and each says so on hover
    /// rather than by silence. The saved list is read from `connections.json`
    /// and shown; a row of it becomes a way in when there is a session behind
    /// it, in M2.
    fn render_welcome(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let profiles = self.profiles.connections();

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
                        profile_row(index, &profile.name, &profile.driver_id, theme)
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
                        soon(
                            WELCOME_NEW_SELECTOR,
                            Button::new("welcome-new", ts!("welcome.new_connection"))
                                .variant(ButtonVariant::Primary)
                                .full_width(true)
                                .disabled(true)
                                .tab_index(WELCOME_FIRST_TAB),
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
                    .child(ts!("statusbar.no_connection")),
            )
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .child(ts!("statusbar.no_selection")),
            )
            .into_any_element()
    }
}

/// One row of the welcome screen's saved list.
///
/// Display only in M0: no hover, no pointer, no tab stop, because clicking a
/// row opens a session and there is nothing behind it to open one on. The row
/// gets its click, its context menu and its tab-ring place in M2, when it can
/// keep the promise a pointer cursor makes.
fn profile_row(index: usize, name: &str, driver: &str, theme: &Theme) -> AnyElement {
    div()
        .id(("profile-row", index))
        .flex()
        .flex_row()
        .items_center()
        .gap(px(8.))
        .px(px(6.))
        .py(px(5.))
        .rounded_md()
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
            .on_action(cx.listener(Self::dismiss_dialog_action))
            .child(toolbar)
            .child(body)
            .child(status_bar)
            .children(about)
            .children(settings)
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
            items: vec![MenuItem::action(
                ts!("menu.toggle_explorer"),
                ToggleExplorer,
            )],
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
        // After the widget layer, because it scopes its bindings to key
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
            ts!("menu.new_connection"),
            ts!("menu.settings"),
            ts!("menu.toggle_explorer"),
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
