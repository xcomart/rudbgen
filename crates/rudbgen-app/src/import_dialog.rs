//! The *Import from jdbgen* wizard (architecture document, D5 and §4.6).
//!
//! Three steps over `rudbgen-import`, which does all of the reading and none of
//! the writing:
//!
//! 1. **the file and the master password** — the path
//!    [`locate`](rudbgen_import::locate) found, or one the user chose, and the
//!    one password jdbgen ever asks for. [`read`](rudbgen_import::read) and
//!    [`decrypt`](rudbgen_import::decrypt) run on a background task, because
//!    PBKDF2 is meant to be slow; a wrong password comes back as a message
//!    beside the field rather than as a failed import.
//! 2. **the checklist** — connections, drivers, template sets and abbreviation
//!    rules, each with a tick, plus every [`Note`] the mapping produced. The
//!    D10 note is always among them: word rules now match ignoring case, and
//!    this is one of the two places the architecture document says so out loud.
//! 3. **the result** — what was written, and what could not be.
//!
//! # The master password does not survive the step it is typed in
//!
//! It is read out of the field once, moved into the background task, and the
//! field is cleared the moment the task is spawned. What the task holds is
//! [`Zeroizing<String>`], so the buffer is wiped when the task ends, whether it
//! ended in a `Decrypted` or in [`Error::WrongPassword`]. Nothing keeps it for
//! step 2: by then the configuration is already open.
//!
//! # Where the pieces go
//!
//! Profiles, drivers, sets and rules go into the four JSON stores; the database
//! passwords go into the OS keychain, one [`SecretSlot::Connection`] entry per
//! imported profile. The keychain is reached through [`SecretSink`] rather than
//! called directly, so [`merge`] stays a pure function of its inputs and the
//! store merge is provable in a temporary directory with no credential store
//! anywhere near it. A machine without a keychain is not a failed import: the
//! profile is saved without its password and the wizard's last step names it,
//! which is exactly what happens when the user later opens that connection.
//!
//! [`Note`]: rudbgen_import::Note
//! [`Error::WrongPassword`]: rudbgen_import::Error::WrongPassword
//! [`Zeroizing<String>`]: zeroize::Zeroizing

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use gpui::{
    App, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, IntoElement,
    KeyDownEvent, MouseButton, MouseUpEvent, PathPromptOptions, Render, ScrollHandle, SharedString,
    Window, div, prelude::*, px,
};
use rudbgen_core::{
    AbbreviationStore, ConnectionStore, DriverDef, DriverStore, SecretSlot, SecretStore,
    TemplateSetStore,
};
use rudbgen_import::{Decrypted, Error, MapOptions, Mapped, Note, PathKind, Preview, SettingsHint};
use ruui::{
    Button, ButtonVariant, Checkbox, DraggedThumb, Scrollbar, ScrollbarAxis, ScrollbarState,
    TextInput, Theme, hide_later, modal, scroll_to, scrolled, theme,
};
use uuid::Uuid;
use zeroize::Zeroizing;

use crate::i18n::ts;

/// Width of the dialog panel.
const DIALOG_WIDTH: f32 = 640.;

/// Height at which the checklist starts scrolling.
///
/// Generous, because the checklist is the one thing here worth reading whole: a
/// configuration with a dozen connections in it should not be five rows and a
/// scroll bar. [`modal`] caps the panel at the window's own height anyway, so
/// this only decides when the *inner* box starts to scroll.
const BODY_MAX_HEIGHT: f32 = 540.;

/// Element id the checklist's overlay scroll indicator is drawn under.
const BODY_SCROLLBAR: &str = "import-scrollbar";

/// Tab-ring position of the first control in the dialog.
const FIRST_TAB: isize = 100;

/// What is appended to a name that is already taken.
///
/// Deliberately a word rather than a number: a user looking at two rows called
/// `staging` and `staging (imported)` can tell at a glance which is which,
/// which `staging (2)` does not.
const IMPORTED_SUFFIX: &str = "(imported)";

// --- the pure part: what an import writes -------------------------------

/// What the user ticked on the checklist.
///
/// One boolean per row rather than a set of ids, because the rows are what the
/// user sees and a [`Mapped`] is ordered: the *n*th tick is the *n*th row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// One per [`Mapped::connections`].
    pub connections: Vec<bool>,
    /// One per [`Mapped::drivers`].
    pub drivers: Vec<bool>,
    /// One per [`Mapped::sets`].
    pub sets: Vec<bool>,
    /// Whether the abbreviation rules come across at all.
    pub rules: bool,
    /// Whether the rules are switched **on** once they are here.
    ///
    /// Separate from [`Selection::rules`] because it is a different question:
    /// jdbgen was applying them, and rudbgen's own switch is off by default
    /// (`AbbreviationStore::apply_to_names`). Unticked leaves the switch as it
    /// was rather than turning it off.
    pub apply_abbr: bool,
    /// Whether jdbgen's language and theme are adopted.
    pub settings: bool,
}

impl Selection {
    /// Everything the preview offers, ticked.
    ///
    /// The default a checklist opens on: a user who came here to import wants
    /// the import, and unticking three rows is less work than ticking thirty.
    pub fn everything(mapped: &Mapped) -> Self {
        Self {
            connections: vec![true; mapped.connections.len()],
            drivers: vec![true; mapped.drivers.len()],
            sets: vec![true; mapped.sets.len()],
            rules: !mapped.rules.is_empty(),
            apply_abbr: mapped.apply_abbr,
            settings: false,
        }
    }

    /// Whether anything at all would be written.
    ///
    /// [`Selection::apply_abbr`] counts: it is one boolean in
    /// `abbreviations.json`, and switching the mechanism on is a thing an
    /// import can be asked to do on its own.
    pub fn is_empty(&self) -> bool {
        !self.connections.iter().any(|on| *on)
            && !self.drivers.iter().any(|on| *on)
            && !self.sets.iter().any(|on| *on)
            && !self.rules
            && !self.apply_abbr
            && !self.settings
    }
}

/// What to do with a name the installation already has.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OnConflict {
    /// Keep both, the newcomer under `name (imported)`.
    Rename,
    /// Keep what is here and leave the jdbgen entry behind.
    Skip,
}

/// The four stores an import writes into.
///
/// Passed in rather than loaded here, so that [`merge`] can be run against
/// stores a test built by hand and against the ones the application read from
/// disk without knowing the difference.
#[derive(Debug, Default)]
pub struct Stores {
    /// `connections.json`.
    pub connections: ConnectionStore,
    /// `drivers.json`.
    pub drivers: DriverStore,
    /// `template-sets.json`.
    pub sets: TemplateSetStore,
    /// `abbreviations.json`.
    pub abbreviations: AbbreviationStore,
}

/// What an import did, as its last step reports it.
#[derive(Default, Clone, PartialEq, Eq)]
pub struct Imported {
    /// Profiles written.
    pub connections: usize,
    /// Driver definitions written.
    pub drivers: usize,
    /// Template sets written.
    pub sets: usize,
    /// Abbreviation rules appended.
    pub rules: usize,
    /// Names left behind, because something here already had them.
    pub skipped: Vec<String>,
    /// Passwords on their way to the keychain, one per imported profile that
    /// had one.
    ///
    /// Handed back rather than stored, because storing needs a credential store
    /// and this function must be runnable without one; see [`store_secrets`].
    pub secrets: Vec<(Uuid, String)>,
}

impl std::fmt::Debug for Imported {
    /// Renders the counts and the skipped names, never a password.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Imported")
            .field("connections", &self.connections)
            .field("drivers", &self.drivers)
            .field("sets", &self.sets)
            .field("rules", &self.rules)
            .field("skipped", &self.skipped)
            .field(
                "secrets",
                &format_args!("<{} redacted>", self.secrets.len()),
            )
            .finish()
    }
}

impl Imported {
    /// Whether anything at all was written.
    pub fn is_empty(&self) -> bool {
        self.connections == 0 && self.drivers == 0 && self.sets == 0 && self.rules == 0
    }
}

/// Where an imported password goes.
///
/// A trait with one method, and the only reason it exists: [`store_secrets`]
/// has to be testable, and a test that reached the real credential store would
/// write into the developer's own login keyring.
pub trait SecretSink {
    /// Save `password` as the database password of the profile `id`.
    ///
    /// # Errors
    ///
    /// The message is shown to the user beside the connection's name, so it
    /// carries the platform's own words rather than a code.
    fn store(&mut self, id: Uuid, password: &str) -> Result<(), String>;
}

/// The OS keychain, which is where a password belongs (D5).
pub struct Keychain;

impl SecretSink for Keychain {
    fn store(&mut self, id: Uuid, password: &str) -> Result<(), String> {
        SecretStore::set(id, SecretSlot::Connection, password).map_err(|error| format!("{error:#}"))
    }
}

/// Write the imported passwords, and report the profiles that could not have
/// theirs written.
///
/// Never fails as a whole: a machine with no credential store still gets its
/// connections, and the user is told which of them will ask for a password.
/// `names` is looked up by id so the report can name the connection rather than
/// a UUID.
pub fn store_secrets(
    secrets: &[(Uuid, String)],
    names: &HashMap<Uuid, String>,
    sink: &mut dyn SecretSink,
) -> Vec<String> {
    let mut failed = Vec::new();
    for (id, password) in secrets {
        let Err(error) = sink.store(*id, password) else {
            continue;
        };
        let name = names.get(id).cloned().unwrap_or_else(|| id.to_string());
        log::warn!("the password of the imported connection {name} was not stored: {error}");
        failed.push(name);
    }
    failed
}

/// Fold everything the user ticked into the four stores.
///
/// Pure: it reads `mapped`, writes `stores`, and touches neither the disk nor
/// the keychain. The order is deliberate — **drivers first**, because a
/// renamed driver id has to reach the profiles that name it before they are
/// written.
///
/// Conflicts are decided by name, case-insensitively, against what the store
/// already holds *and* against what this same import has already added, so two
/// jdbgen entries called `staging` cannot both land as `staging (imported)`.
/// Drivers are the exception the rule needs: a stock driver keeps rudbgen's
/// built-in id (see `rudbgen_import::map`), so a clash there is jdbgen's
/// version of a definition rudbgen already ships. Under [`OnConflict::Rename`]
/// it is added beside the built-in under an id of its own and the imported
/// connections are repointed at it; under [`OnConflict::Skip`] it is left
/// behind and those connections use the definition that is already here, which
/// is the same product.
///
/// Abbreviation rules have no conflict policy of their own: a rule that looks
/// for what an existing enabled rule looks for could never fire, so it is
/// always skipped. That is the same test the rules editor refuses a duplicate
/// with (`crate::abbreviation_dialog::duplicates`).
pub fn merge(
    mapped: &Mapped,
    selection: &Selection,
    on_conflict: OnConflict,
    stores: &mut Stores,
) -> Imported {
    let mut out = Imported::default();
    let mut repointed: HashMap<String, String> = HashMap::new();

    for (index, driver) in mapped.drivers.iter().enumerate() {
        if !selection.drivers.get(index).copied().unwrap_or(false) {
            continue;
        }
        let mut driver = driver.clone();
        if stores.drivers.get(&driver.id).is_some() {
            if on_conflict == OnConflict::Skip {
                out.skipped.push(driver.name.clone());
                continue;
            }
            let id = free_driver_id(&stores.drivers, &driver.id);
            repointed.insert(driver.id.clone(), id.clone());
            driver.id = id;
            driver.name = suffixed(&driver.name);
        }
        stores.drivers.upsert(driver);
        out.drivers += 1;
    }

    for (index, (profile, secret)) in mapped.connections.iter().enumerate() {
        if !selection.connections.get(index).copied().unwrap_or(false) {
            continue;
        }
        let mut profile = profile.clone();
        if let Some(id) = repointed.get(&profile.driver_id) {
            profile.driver_id.clone_from(id);
        }
        let taken: Vec<&str> = stores
            .connections
            .connections()
            .iter()
            .map(|saved| saved.name.as_str())
            .collect();
        if let Some(name) = resolve_name(&profile.name, &taken, on_conflict) {
            profile.name = name;
        } else {
            out.skipped.push(profile.name.clone());
            continue;
        }
        if !secret.password.is_empty() {
            out.secrets.push((profile.id, secret.password.clone()));
        }
        stores.connections.upsert(profile);
        out.connections += 1;
    }

    for (index, set) in mapped.sets.iter().enumerate() {
        if !selection.sets.get(index).copied().unwrap_or(false) {
            continue;
        }
        let mut set = set.clone();
        let taken: Vec<&str> = stores.sets.sets.iter().map(|s| s.name.as_str()).collect();
        if let Some(name) = resolve_name(&set.name, &taken, on_conflict) {
            set.name = name;
        } else {
            out.skipped.push(set.name.clone());
            continue;
        }
        stores.sets.upsert(set);
        out.sets += 1;
    }

    if selection.rules {
        for rule in &mapped.rules {
            let key = rule_key(&rule.whole_name, &rule.abbreviation);
            if key.1.is_empty() {
                continue;
            }
            let clash = stores
                .abbreviations
                .rules
                .iter()
                .any(|saved| rule_key(&saved.whole_name, &saved.abbreviation) == key);
            if clash {
                out.skipped.push(rule.abbreviation.clone());
                continue;
            }
            stores.abbreviations.rules.push(rule.clone());
            out.rules += 1;
        }
    }
    if selection.apply_abbr {
        stores.abbreviations.apply_to_names = true;
    }

    out
}

/// What a rule is compared by: its kind, and its abbreviation ignoring case.
///
/// The key the engine's dictionary uses, which is what makes two rules one
/// (D10, and `crate::abbreviation_dialog`'s duplicate check).
fn rule_key(whole_name: &bool, abbreviation: &str) -> (bool, String) {
    (*whole_name, abbreviation.trim().to_lowercase())
}

/// The name an entry called `name` gets, or `None` when it is skipped.
///
/// Matching ignores case, because two connections called `Staging` and
/// `staging` are a list nobody can read, whatever the file system thinks.
fn resolve_name(name: &str, taken: &[&str], on_conflict: OnConflict) -> Option<String> {
    if !taken.iter().any(|used| used.eq_ignore_ascii_case(name)) {
        return Some(name.to_string());
    }
    if on_conflict == OnConflict::Skip {
        return None;
    }
    let mut candidate = suffixed(name);
    let mut attempt = 2;
    while taken
        .iter()
        .any(|used| used.eq_ignore_ascii_case(&candidate))
    {
        candidate = format!("{name} {IMPORTED_SUFFIX} {attempt}");
        attempt += 1;
    }
    Some(candidate)
}

/// `name` with the import marker after it.
fn suffixed(name: &str) -> String {
    format!("{name} {IMPORTED_SUFFIX}")
}

/// A driver id nothing in `store` is using yet.
fn free_driver_id(store: &DriverStore, id: &str) -> String {
    let mut candidate = format!("{id}-imported");
    let mut attempt = 2;
    while store.get(&candidate).is_some() {
        candidate = format!("{id}-imported-{attempt}");
        attempt += 1;
    }
    candidate
}

// --- the dialog ---------------------------------------------------------

/// What the wizard tells the shell about.
pub enum ImportDialogEvent {
    /// Something was written; the shell re-reads `connections.json`.
    ///
    /// Carries jdbgen's language and theme when the user ticked that box, so
    /// the shell — which owns the settings — can apply them.
    Imported(Option<Box<SettingsHint>>),
    /// The wizard was dismissed without writing anything.
    Dismissed,
}

/// Which step the wizard is on.
enum Stage {
    /// The file and the master password.
    Password {
        /// What went wrong with the last attempt, if anything.
        error: Option<SharedString>,
    },
    /// Reading and decrypting, on a background task.
    Opening,
    /// The checklist.
    Review,
    /// Writing the stores and the keychain.
    Importing,
    /// What was written.
    Done {
        /// The counts.
        result: Imported,
        /// Connections whose password could not be stored.
        no_keychain: Vec<String>,
    },
}

/// The import wizard.
pub struct ImportDialog {
    /// Whether the dialog is visible.
    open: bool,
    /// Focus of the dialog root; also the anchor for the `Escape` handler.
    focus_handle: FocusHandle,
    /// Whether focus should move into the dialog on the next render.
    pending_focus: bool,
    /// Which step is on screen.
    stage: Stage,
    /// The configuration being imported.
    path: PathBuf,
    /// The master password field.
    password: Entity<TextInput>,
    /// What the mapping produced, once the password opened the file.
    mapped: Option<Mapped>,
    /// The checklist, derived from the same mapping.
    preview: Option<Preview>,
    /// jdbgen's language and theme, for the settings row.
    hint: SettingsHint,
    /// What the user ticked.
    selection: Selection,
    /// What to do with a name that is taken.
    on_conflict: OnConflict,
    /// Vertical scroll of the body.
    scroll: ScrollHandle,
    /// Whether the body's overlay scroll indicator is on screen.
    scrollbar: ScrollbarState,
}

impl ImportDialog {
    /// Builds the wizard, closed.
    pub fn new(cx: &mut Context<Self>) -> Self {
        // `Enter` in the password field is *Next*: it is the only thing the
        // step can do, and reaching for the mouse to say so is a step nobody
        // wants twice after a wrong password.
        let this = cx.weak_entity();
        let password = cx.new(|cx| {
            TextInput::new(cx)
                .masked(true)
                .placeholder(ts!("import.password_placeholder"))
                .tab_index(FIRST_TAB + 1)
                .on_submit(move |_, _window, cx| {
                    this.update(cx, |dialog, cx| dialog.unlock(cx)).ok();
                })
        });
        Self {
            open: false,
            focus_handle: cx.focus_handle(),
            pending_focus: false,
            stage: Stage::Password { error: None },
            path: PathBuf::new(),
            password,
            mapped: None,
            preview: None,
            hint: SettingsHint::default(),
            selection: Selection::default(),
            on_conflict: OnConflict::Rename,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }

    /// Shows the wizard over `path`, on its first step.
    pub fn open(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.path = path;
        self.stage = Stage::Password { error: None };
        self.mapped = None;
        self.preview = None;
        self.hint = SettingsHint::default();
        self.selection = Selection::default();
        self.on_conflict = OnConflict::Rename;
        self.password.update(cx, |input, cx| input.clear(cx));
        self.open = true;
        self.pending_focus = true;
        cx.notify();
    }

    /// Whether the wizard is visible.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Hides the wizard without emitting an event.
    pub fn close(&mut self, cx: &mut Context<Self>) {
        self.open = false;
        self.pending_focus = false;
        self.mapped = None;
        self.preview = None;
        self.password.update(cx, |input, cx| input.clear(cx));
        cx.notify();
    }

    /// Closes the wizard and reports it, so the shell can restore focus.
    fn dismiss(&mut self, cx: &mut Context<Self>) {
        cx.emit(ImportDialogEvent::Dismissed);
        self.close(cx);
    }

    /// `Escape`, the backdrop and *Cancel*.
    ///
    /// Ignored while a background step is running: half an import is not a
    /// state the wizard can be left in, and both steps are seconds long.
    pub fn escape(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Opening | Stage::Importing) {
            return;
        }
        self.dismiss(cx);
    }

    /// Asks the platform for a configuration file to import instead.
    fn choose_file(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(ts!("import.choose_file")),
        });
        cx.spawn(async move |dialog, cx| {
            let Ok(Ok(Some(chosen))) = paths.await else {
                return;
            };
            let Some(path) = chosen.into_iter().next() else {
                return;
            };
            dialog
                .update(cx, |dialog, cx| {
                    dialog.path = path;
                    dialog.stage = Stage::Password { error: None };
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// Step 1 → step 2: read the file and open it with the master password.
    ///
    /// The password leaves the field here and is not put back: the task owns
    /// the only copy, in a buffer that wipes itself, and a retry asks again.
    fn unlock(&mut self, cx: &mut Context<Self>) {
        if matches!(self.stage, Stage::Opening | Stage::Importing) {
            return;
        }
        let master = Zeroizing::new(self.password.read(cx).content().to_owned());
        self.password.update(cx, |input, cx| input.clear(cx));
        self.stage = Stage::Opening;
        cx.notify();

        let path = self.path.clone();
        cx.spawn(async move |dialog, cx| {
            let opened = cx
                .background_spawn(async move { read_and_open(&path, &master) })
                .await;
            dialog
                .update(cx, |dialog, cx| match opened {
                    Ok(decrypted) => dialog.opened(&decrypted, cx),
                    Err(error) => {
                        dialog.stage = Stage::Password {
                            error: Some(message_of(&error)),
                        };
                        dialog.pending_focus = true;
                        cx.notify();
                    }
                })
                .ok();
        })
        .detach();
    }

    /// The password was right: build the checklist.
    fn opened(&mut self, decrypted: &Decrypted, cx: &mut Context<Self>) {
        let mut options = MapOptions::new(
            self.path
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| PathBuf::from(".")),
        );
        if let Some(install) = jdbgen_install_dir() {
            options = options.with_install_dir(install);
        }
        let mapped = rudbgen_import::map(decrypted, &options);
        self.preview = Some(rudbgen_import::from_mapped(decrypted, &mapped));
        self.hint = mapped.settings_hint.clone();
        self.selection = Selection::everything(&mapped);
        self.mapped = Some(mapped);
        self.stage = Stage::Review;
        cx.notify();
    }

    /// Step 2 → step 3: write the stores, then the keychain.
    fn import(&mut self, cx: &mut Context<Self>) {
        let Some(mapped) = self.mapped.clone() else {
            return;
        };
        let selection = self.selection.clone();
        let on_conflict = self.on_conflict;
        self.stage = Stage::Importing;
        cx.notify();

        cx.spawn(async move |dialog, cx| {
            let written = cx
                .background_spawn(async move { write_stores(&mapped, &selection, on_conflict) })
                .await;
            dialog
                .update(cx, |dialog, cx| {
                    let (result, no_keychain) = written;
                    let settings = dialog
                        .selection
                        .settings
                        .then(|| Box::new(dialog.hint.clone()));
                    dialog.stage = Stage::Done {
                        result,
                        no_keychain,
                    };
                    cx.emit(ImportDialogEvent::Imported(settings));
                    cx.notify();
                })
                .ok();
        })
        .detach();
    }

    /// Ticks or unticks one checklist row.
    fn toggle(&mut self, section: Section, index: usize, on: bool, cx: &mut Context<Self>) {
        let list = match section {
            Section::Connections => &mut self.selection.connections,
            Section::Drivers => &mut self.selection.drivers,
            Section::Sets => &mut self.selection.sets,
        };
        if let Some(slot) = list.get_mut(index) {
            *slot = on;
        }
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

    /// `Escape` dismisses the wizard from anywhere inside it.
    fn on_key_down(&mut self, event: &KeyDownEvent, _window: &mut Window, cx: &mut Context<Self>) {
        if self.open && event.keystroke.key == "escape" {
            cx.stop_propagation();
            self.escape(cx);
        }
    }

    // --- the scroll bar ---------------------------------------------------

    /// The body's overlay scroll indicator, as it now stands.
    fn bar(&self) -> Scrollbar {
        Scrollbar::for_handle(BODY_SCROLLBAR, ScrollbarAxis::Vertical, &self.scroll)
            .fade(self.scrollbar.fade())
    }

    /// Puts the bar up whenever the body has moved.
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

    /// Scrolls the body when its thumb is dragged.
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

    /// Step 1: the file and the master password.
    fn render_password(&self, chrome: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.entity();
        let busy = matches!(self.stage, Stage::Opening);
        let error = match &self.stage {
            Stage::Password { error } => error.clone(),
            _ => None,
        };
        div()
            .flex()
            .flex_col()
            .gap(px(12.))
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(chrome.text_muted)
                    .child(ts!("import.password_hint")),
            )
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(12.))
                            .text_color(chrome.text)
                            .child(SharedString::from(self.path.display().to_string())),
                    )
                    .child(
                        Button::new("import-choose", ts!("import.other_file"))
                            .variant(ButtonVariant::Secondary)
                            .tab_index(FIRST_TAB)
                            .disabled(busy)
                            .on_click({
                                let this = this.clone();
                                move |_, _window, cx| {
                                    this.update(cx, |dialog, cx| dialog.choose_file(cx));
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(4.))
                    .child(
                        div()
                            .text_size(px(11.))
                            .text_color(chrome.text_muted)
                            .child(ts!("import.master_password")),
                    )
                    .child(self.password.clone()),
            )
            .children(error.map(|message| {
                div()
                    .text_size(px(11.))
                    .text_color(chrome.danger)
                    .child(message)
            }))
            .when(busy, |body| {
                body.child(
                    div()
                        .text_size(px(11.))
                        .text_color(chrome.text_muted)
                        .child(ts!("import.opening")),
                )
            })
    }

    /// Step 2: the checklist.
    fn render_review(&self, chrome: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let Some(preview) = self.preview.as_ref() else {
            return div();
        };
        let this = cx.entity();

        let connections = self.section(
            Section::Connections,
            ts!("import.section_connections"),
            preview
                .connections
                .iter()
                .map(|row| {
                    (
                        SharedString::from(row.name.clone()),
                        Some(SharedString::from(format!("{} · {}", row.driver, row.url))),
                    )
                })
                .collect(),
            chrome,
            cx,
        );
        // The driver rows come from the *mapping*, not from the preview: two
        // jdbgen entries naming one product collapse into one `DriverDef`, so
        // `Preview::drivers` — one row per entry in the file — is longer than
        // the list the ticks index into. Everything else lines up one to one.
        let drivers = self.section(
            Section::Drivers,
            ts!("import.section_drivers"),
            self.mapped.as_ref().map(driver_rows).unwrap_or_default(),
            chrome,
            cx,
        );
        let sets = self.section(
            Section::Sets,
            ts!("import.section_sets"),
            preview
                .sets
                .iter()
                .map(|row| {
                    (
                        SharedString::from(row.name.clone()),
                        Some(ts!(
                            "import.set_templates",
                            count = row.templates.to_string()
                        )),
                    )
                })
                .collect(),
            chrome,
            cx,
        );

        let rules = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading(ts!("import.section_rules"), chrome))
            .child(
                Checkbox::new(
                    "import-rules",
                    ts!(
                        "import.rules_count",
                        count = preview.rules.len().to_string()
                    ),
                )
                .checked(self.selection.rules)
                .on_toggle({
                    let this = this.clone();
                    move |on, _window, cx| {
                        this.update(cx, |dialog, cx| {
                            dialog.selection.rules = on;
                            cx.notify();
                        });
                    }
                }),
            )
            .child(
                Checkbox::new("import-apply-abbr", ts!("import.apply_abbr"))
                    .checked(self.selection.apply_abbr)
                    .on_toggle({
                        let this = this.clone();
                        move |on, _window, cx| {
                            this.update(cx, |dialog, cx| {
                                dialog.selection.apply_abbr = on;
                                cx.notify();
                            });
                        }
                    }),
            );

        let settings = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading(ts!("import.section_settings"), chrome))
            .child(
                Checkbox::new(
                    "import-settings",
                    ts!(
                        "import.adopt_settings",
                        language = self
                            .hint
                            .language
                            .clone()
                            .unwrap_or_else(|| ts!("import.system_language").to_string()),
                        theme = ts!(if self.hint.dark_ui {
                            "import.theme_dark"
                        } else {
                            "import.theme_light"
                        })
                    ),
                )
                .checked(self.selection.settings)
                .on_toggle({
                    let this = this.clone();
                    move |on, _window, cx| {
                        this.update(cx, |dialog, cx| {
                            dialog.selection.settings = on;
                            cx.notify();
                        });
                    }
                }),
            );

        let rename = self.on_conflict == OnConflict::Rename;
        let conflict = div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading(ts!("import.section_conflicts"), chrome))
            .child(
                Checkbox::new("import-rename", ts!("import.conflict_rename"))
                    .checked(rename)
                    .on_toggle({
                        let this = this.clone();
                        move |on, _window, cx| {
                            this.update(cx, |dialog, cx| {
                                dialog.on_conflict = if on {
                                    OnConflict::Rename
                                } else {
                                    OnConflict::Skip
                                };
                                cx.notify();
                            });
                        }
                    }),
            )
            .child(
                div()
                    .text_size(px(11.))
                    .text_color(chrome.text_muted)
                    .child(if rename {
                        ts!("import.conflict_rename_hint")
                    } else {
                        ts!("import.conflict_skip_hint")
                    }),
            );

        let notes = div()
            .flex()
            .flex_col()
            .gap(px(3.))
            .child(heading(ts!("import.section_notes"), chrome))
            .children(preview.notes.iter().map(|note| {
                div()
                    .text_size(px(11.))
                    .text_color(chrome.text_muted)
                    .child(note_text(note))
            }));

        div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .child(connections)
            .child(drivers)
            .child(sets)
            .child(rules)
            .child(settings)
            .child(conflict)
            .child(notes)
    }

    /// One ticked section of the checklist.
    fn section(
        &self,
        section: Section,
        title: SharedString,
        rows: Vec<(SharedString, Option<SharedString>)>,
        chrome: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let this = cx.entity();
        let ticks = match section {
            Section::Connections => &self.selection.connections,
            Section::Drivers => &self.selection.drivers,
            Section::Sets => &self.selection.sets,
        };
        let ticks = ticks.clone();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .child(heading(title, chrome))
            .when(rows.is_empty(), |block| {
                block.child(
                    div()
                        .text_size(px(11.))
                        .text_color(chrome.text_muted)
                        .child(ts!("import.nothing_here")),
                )
            })
            .children(rows.into_iter().enumerate().map(|(index, (name, note))| {
                let this = this.clone();
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .child(
                        Checkbox::new((section.id(), index), name)
                            .checked(ticks.get(index).copied().unwrap_or(false))
                            .on_toggle(move |on, _window, cx| {
                                this.update(cx, |dialog, cx| {
                                    dialog.toggle(section, index, on, cx);
                                });
                            }),
                    )
                    .children(note.map(|note| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(chrome.text_muted)
                            .child(note)
                    }))
            }))
    }

    /// Step 3: what was written.
    fn render_done(
        &self,
        result: &Imported,
        no_keychain: &[String],
        chrome: &Theme,
    ) -> impl IntoElement + use<> {
        div()
            .flex()
            .flex_col()
            .gap(px(10.))
            .child(
                div()
                    .text_size(px(13.))
                    .text_color(chrome.text)
                    .child(if result.is_empty() {
                        ts!("import.nothing_written")
                    } else {
                        ts!(
                            "import.written",
                            connections = result.connections.to_string(),
                            drivers = result.drivers.to_string(),
                            sets = result.sets.to_string(),
                            rules = result.rules.to_string()
                        )
                    }),
            )
            .when(!result.skipped.is_empty(), |body| {
                body.child(
                    div()
                        .text_size(px(11.))
                        .text_color(chrome.text_muted)
                        .child(ts!("import.skipped", names = result.skipped.join(", "))),
                )
            })
            .when(!no_keychain.is_empty(), |body| {
                body.child(
                    div()
                        .text_size(px(11.))
                        .text_color(chrome.danger)
                        .child(ts!("import.no_keychain", names = no_keychain.join(", "))),
                )
            })
    }

    /// The buttons under whichever step is showing.
    fn render_footer(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.entity();
        let cancel = Button::new("import-cancel", ts!("common.cancel"))
            .disabled(matches!(self.stage, Stage::Opening | Stage::Importing))
            .on_click({
                let this = this.clone();
                move |_, _window, cx| {
                    this.update(cx, |dialog, cx| dialog.dismiss(cx));
                }
            });

        let action = match &self.stage {
            Stage::Password { .. } | Stage::Opening => {
                Button::new("import-next", ts!("import.next"))
                    .variant(ButtonVariant::Primary)
                    .disabled(matches!(self.stage, Stage::Opening))
                    .on_click({
                        let this = this.clone();
                        move |_, _window, cx| {
                            this.update(cx, |dialog, cx| dialog.unlock(cx));
                        }
                    })
            }
            Stage::Review | Stage::Importing => Button::new("import-run", ts!("import.import"))
                .variant(ButtonVariant::Primary)
                .disabled(matches!(self.stage, Stage::Importing) || self.selection.is_empty())
                .on_click({
                    let this = this.clone();
                    move |_, _window, cx| {
                        this.update(cx, |dialog, cx| dialog.import(cx));
                    }
                }),
            Stage::Done { .. } => Button::new("import-close", ts!("common.close"))
                .variant(ButtonVariant::Primary)
                .on_click({
                    let this = this.clone();
                    move |_, _window, cx| {
                        this.update(cx, |dialog, cx| dialog.dismiss(cx));
                    }
                }),
        };

        div()
            .flex()
            .flex_row()
            .items_center()
            .justify_end()
            .gap(px(8.))
            .when(!matches!(self.stage, Stage::Done { .. }), |footer| {
                footer.child(cancel)
            })
            .child(action)
    }
}

/// Which ticked list a row belongs to.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Section {
    /// [`Selection::connections`].
    Connections,
    /// [`Selection::drivers`].
    Drivers,
    /// [`Selection::sets`].
    Sets,
}

impl Section {
    /// Element id prefix of the section's checkboxes.
    fn id(self) -> &'static str {
        match self {
            Self::Connections => "import-connection",
            Self::Drivers => "import-driver",
            Self::Sets => "import-set",
        }
    }
}

impl EventEmitter<ImportDialogEvent> for ImportDialog {}

impl Focusable for ImportDialog {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for ImportDialog {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if !self.open {
            return div().id("import-dialog");
        }

        self.apply_pending_focus(window, cx);
        self.watch_scroll(cx);

        let chrome = theme(cx);
        let step = match &self.stage {
            Stage::Password { .. } | Stage::Opening => {
                self.render_password(&chrome, cx).into_any_element()
            }
            Stage::Review => self.render_review(&chrome, cx).into_any_element(),
            Stage::Importing => div()
                .text_size(px(12.))
                .text_color(chrome.text_muted)
                .child(ts!("import.importing"))
                .into_any_element(),
            Stage::Done {
                result,
                no_keychain,
            } => self
                .render_done(result, no_keychain, &chrome)
                .into_any_element(),
        };

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
                            .id("import-body")
                            .track_scroll(&self.scroll)
                            .flex()
                            .flex_col()
                            .min_h_0()
                            .max_h(px(BODY_MAX_HEIGHT))
                            .overflow_y_scroll()
                            .restrict_scroll_to_axis()
                            .child(step),
                    )
                    .children(
                        self.bar()
                            .on_hover(cx.listener(|dialog, hovered: &bool, _window, cx| {
                                dialog.hover_scrollbar(*hovered, cx);
                            }))
                            .render(&chrome),
                    ),
            )
            .child(self.render_footer(cx));

        let on_dismiss = {
            let this = cx.entity();
            move |_window: &mut Window, cx: &mut App| {
                this.update(cx, |dialog, cx| dialog.escape(cx));
            }
        };

        div()
            .id("import-dialog")
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
                "import-modal",
                ts!("import.title"),
                px(DIALOG_WIDTH),
                body,
                on_dismiss,
            ))
    }
}

/// One checklist row per driver the import would write.
///
/// Named by the driver definition rather than by jdbgen's entry, and marked
/// when the definition is one rudbgen already ships — which is what
/// `rudbgen_import::map` produces for a stock driver it recognised: the
/// built-in's own id, carrying jdbgen's JAR, SQL overrides and properties.
///
/// The list is the one [`Selection::drivers`] indexes into, and that is the
/// reason this exists rather than a walk of [`Preview::drivers`]: the preview
/// has one row per entry in the file, and two entries naming one product become
/// one definition.
fn driver_rows(mapped: &Mapped) -> Vec<(SharedString, Option<SharedString>)> {
    let builtins: Vec<String> = DriverDef::builtins()
        .into_iter()
        .map(|builtin| builtin.id)
        .collect();
    mapped
        .drivers
        .iter()
        .map(|driver| {
            let note = builtins
                .contains(&driver.id)
                .then(|| ts!("import.matches_builtin", builtin = driver.id.clone()));
            (SharedString::from(driver.name.clone()), note)
        })
        .collect()
}

/// A section heading of the checklist.
fn heading(text: SharedString, chrome: &Theme) -> impl IntoElement + use<> {
    div()
        .text_size(px(11.))
        .text_color(chrome.text_muted)
        .child(text)
}

/// Read and decrypt, on the background task.
///
/// **Blocks**: PBKDF2 with jdbgen's iteration count is the point of the wait.
fn read_and_open(path: &Path, master: &str) -> Result<Decrypted, Error> {
    let config = rudbgen_import::read(path)?;
    rudbgen_import::decrypt(&config, master)
}

/// Merge and save, on the background task.
///
/// The four stores are read here rather than passed in, so that the wizard
/// folds into whatever is on disk at the moment the button is pressed rather
/// than into a copy taken when the window opened. A store that cannot be read
/// starts empty, which is what every other reader in the application does.
fn write_stores(
    mapped: &Mapped,
    selection: &Selection,
    on_conflict: OnConflict,
) -> (Imported, Vec<String>) {
    let mut stores = Stores {
        connections: ConnectionStore::load().unwrap_or_default(),
        drivers: DriverStore::load().unwrap_or_default(),
        sets: TemplateSetStore::load().unwrap_or_default(),
        abbreviations: AbbreviationStore::load().unwrap_or_default(),
    };
    let result = merge(mapped, selection, on_conflict, &mut stores);

    for (what, outcome) in [
        ("connections.json", stores.connections.save()),
        ("drivers.json", stores.drivers.save()),
        ("template-sets.json", stores.sets.save()),
        ("abbreviations.json", stores.abbreviations.save()),
    ] {
        if let Err(error) = outcome {
            log::error!("the import could not write {what}: {error:#}");
        }
    }

    let names: HashMap<Uuid, String> = stores
        .connections
        .connections()
        .iter()
        .map(|profile| (profile.id, profile.name.clone()))
        .collect();
    let failed = store_secrets(&result.secrets, &names, &mut Keychain);
    (result, failed)
}

/// The directory jdbgen was unpacked into, when there is a plausible one.
///
/// jdbgen resolves a relative path against its own installation as a fallback,
/// which is where the shipped templates live for a copy that was unzipped
/// rather than installed (`rudbgen_import::MapOptions`). rudbgen has no way to
/// find that directory in general, so it offers the one candidate that costs
/// nothing to test: a `jdbgen` directory beside the user's home. `None`
/// otherwise, and then a path that resolves in neither directory simply comes
/// across as it stands, with a note saying so.
fn jdbgen_install_dir() -> Option<PathBuf> {
    let home = directories::UserDirs::new()?.home_dir().join("jdbgen");
    home.is_dir().then_some(home)
}

/// What a failure to open the file says.
fn message_of(error: &Error) -> SharedString {
    match error {
        Error::WrongPassword => ts!("import.wrong_password"),
        Error::Malformed { .. } => ts!("import.malformed"),
        Error::Read { .. } => ts!("import.unreadable"),
        Error::Parse { .. } => ts!("import.not_jdbgen"),
    }
}

/// One [`Note`], in the user's language.
///
/// The crate hands back data rather than sentences on purpose — it has no
/// translations — so this is where a note becomes a line.
fn note_text(note: &Note) -> SharedString {
    match note {
        Note::AbbreviationCaseRule => ts!("import.note_case_rule"),
        Note::LegacyEncryption => ts!("import.note_legacy"),
        Note::StockDriverMatched { driver, builtin } => ts!(
            "import.note_stock_matched",
            driver = driver.clone(),
            builtin = builtin.clone()
        ),
        Note::StockDriverUnknown { driver, class } => ts!(
            "import.note_stock_unknown",
            driver = driver.clone(),
            class = class.clone()
        ),
        Note::UnknownDriver {
            connection,
            driver_type,
        } => ts!(
            "import.note_unknown_driver",
            connection = connection.clone(),
            driver = driver_type.clone()
        ),
        Note::KeepAliveNotANumber { connection, value } => ts!(
            "import.note_keep_alive",
            connection = connection.clone(),
            value = value.clone()
        ),
        Note::UnresolvedPath { kind, owner, path } => ts!(
            "import.note_unresolved_path",
            kind = ts!(match kind {
                PathKind::DriverJar => "import.path_jar",
                PathKind::TemplateFile => "import.path_template",
                PathKind::OutputDir => "import.path_output",
            }),
            owner = owner.clone(),
            path = path.clone()
        ),
        Note::IconDropped { owner, icon } => ts!(
            "import.note_icon",
            owner = owner.clone(),
            icon = icon.clone()
        ),
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use rudbgen_core::{AbbreviationRule, ConnectionProfile, DriverDef, TemplateSet};
    use rudbgen_import::{decrypt, map, preview, read};
    use tempfile::TempDir;

    use super::*;

    /// The synthetic jdbgen configuration `rudbgen-import` fixes its mapping
    /// against, and the master password it was written under.
    ///
    /// Reached across the crate boundary rather than copied: one file, one
    /// truth. `rudbgen-import` proves the mapping of it; what is proved here is
    /// the *merge* — that everything the mapping produced reaches the four
    /// stores and comes back out of the files they were written to.
    const CONFIG: &str = include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rudbgen-import/tests/vectors/config.json"
    ));

    /// The master password `config.json` was written under; see its `_source`.
    const MASTER: &str = "correct horse battery staple";

    /// A keychain that keeps what it is given, or refuses everything.
    #[derive(Default)]
    struct FakeKeychain {
        stored: Vec<(Uuid, String)>,
        refuse: bool,
    }

    impl SecretSink for FakeKeychain {
        fn store(&mut self, id: Uuid, password: &str) -> Result<(), String> {
            if self.refuse {
                return Err("no credential store on this machine".into());
            }
            self.stored.push((id, password.to_owned()));
            Ok(())
        }
    }

    /// A driver store with nothing in it.
    ///
    /// [`DriverStore::default`] is the *built-in* definitions, which is the
    /// right answer for the application and the wrong one for a test about
    /// which drivers an import added.
    fn no_drivers() -> DriverStore {
        serde_json::from_str("{}").expect("an empty driver store")
    }

    /// The four stores of a fresh installation with no drivers in it.
    fn bare() -> Stores {
        Stores {
            drivers: no_drivers(),
            ..Stores::default()
        }
    }

    /// The checked-in configuration, decrypted and mapped.
    fn fixture() -> (TempDir, Mapped) {
        let dir = TempDir::new().expect("a temporary jdbgen data directory");
        fs::write(dir.path().join("config.json"), CONFIG).unwrap();
        let config = read(&dir.path().join("config.json")).expect("the fixture parses");
        let opened = decrypt(&config, MASTER).expect("the master password opens it");
        let mapped = map(&opened, &MapOptions::new(dir.path()));
        (dir, mapped)
    }

    fn driver(id: &str, name: &str) -> DriverDef {
        DriverDef {
            id: id.to_string(),
            name: name.to_string(),
            ..DriverDef::default()
        }
    }

    fn rule(whole: bool, abbr: &str, replace: &str) -> AbbreviationRule {
        AbbreviationRule {
            enabled: true,
            whole_name: whole,
            abbreviation: abbr.to_string(),
            replacement: replace.to_string(),
        }
    }

    #[test]
    fn everything_ticked_is_everything_the_file_holds() {
        let (_dir, mapped) = fixture();
        let selection = Selection::everything(&mapped);
        assert!(!selection.is_empty());

        let mut stores = bare();
        let result = merge(&mapped, &selection, OnConflict::Rename, &mut stores);
        assert_eq!(result.connections, mapped.connections.len());
        assert_eq!(result.drivers, mapped.drivers.len());
        assert_eq!(result.sets, mapped.sets.len());
        assert_eq!(result.rules, mapped.rules.len());
        assert_eq!(stores.connections.len(), mapped.connections.len());
        assert_eq!(stores.drivers.len(), mapped.drivers.len());
    }

    #[test]
    fn an_unticked_row_writes_nothing() {
        let (_dir, mapped) = fixture();
        let selection = Selection::default();
        assert!(selection.is_empty());
        let mut stores = bare();
        let result = merge(&mapped, &selection, OnConflict::Rename, &mut stores);
        assert!(result.is_empty());
        assert!(stores.connections.is_empty());
        assert!(stores.drivers.is_empty());
        assert!(result.secrets.is_empty());
        assert!(stores.sets.sets.is_empty());
        assert!(stores.abbreviations.rules.is_empty());
    }

    #[test]
    fn the_whole_import_reaches_the_files_it_is_saved_to() {
        // The headless end of the milestone's acceptance test: a jdbgen
        // configuration goes into a temporary configuration directory and comes
        // back out of the four JSON files, keychain excepted — that is what
        // `SecretSink` is for.
        let (_source, mapped) = fixture();
        let config = TempDir::new().expect("a temporary rudbgen configuration directory");
        let mut stores = bare();
        let result = merge(
            &mapped,
            &Selection::everything(&mapped),
            OnConflict::Rename,
            &mut stores,
        );

        stores
            .connections
            .save_to(&config.path().join("connections.json"))
            .unwrap();
        stores
            .drivers
            .save_to(&config.path().join("drivers.json"))
            .unwrap();
        stores
            .sets
            .save_to(&config.path().join("template-sets.json"))
            .unwrap();
        stores
            .abbreviations
            .save_to(&config.path().join("abbreviations.json"))
            .unwrap();

        let connections =
            ConnectionStore::load_from(&config.path().join("connections.json")).unwrap();
        assert_eq!(connections.len(), result.connections);
        let first = &mapped.connections[0].0;
        let saved = connections
            .get(first.id)
            .expect("the profile is stored under the id the mapping gave it");
        assert_eq!(saved.name, first.name);
        assert_eq!(saved.url, first.url);
        // The password is not in the file — that is the whole point of D5.
        let text = fs::read_to_string(config.path().join("connections.json")).unwrap();
        for (_, secret) in &mapped.connections {
            if secret.password.is_empty() {
                continue;
            }
            assert!(
                !text.contains(&secret.password),
                "a password reached connections.json"
            );
        }

        let drivers = DriverStore::load_from(&config.path().join("drivers.json")).unwrap();
        assert_eq!(drivers.len(), result.drivers);
        // Every imported profile names a driver the same import wrote.
        for profile in connections.connections() {
            if profile.driver_id.is_empty() {
                continue; // the fixture's connection whose driverType is unknown
            }
            assert!(
                drivers.get(&profile.driver_id).is_some(),
                "{} names the missing driver {}",
                profile.name,
                profile.driver_id
            );
        }

        let sets = TemplateSetStore::load_from(&config.path().join("template-sets.json")).unwrap();
        assert_eq!(sets.sets.len(), result.sets);
        let rules =
            AbbreviationStore::load_from(&config.path().join("abbreviations.json")).unwrap();
        assert_eq!(rules.rules.len(), result.rules);
        assert_eq!(rules.apply_to_names, mapped.apply_abbr);
    }

    #[test]
    fn a_stock_driver_lands_beside_the_built_in_it_matches_rather_than_over_it() {
        // The store an installation actually has is the built-in definitions,
        // and jdbgen's stock entries map onto their ids. Under the default
        // policy the user keeps both: the definition rudbgen ships, and the one
        // whose JAR path and SQL overrides they edited in jdbgen.
        let (_dir, mapped) = fixture();
        let before = DriverStore::default().len();
        let mut stores = Stores::default();
        let result = merge(
            &mapped,
            &Selection::everything(&mapped),
            OnConflict::Rename,
            &mut stores,
        );
        assert_eq!(result.drivers, mapped.drivers.len());
        assert_eq!(stores.drivers.len(), before + mapped.drivers.len());
    }

    #[test]
    fn a_name_that_is_taken_is_renamed_or_skipped() {
        let (_dir, mapped) = fixture();
        let taken = mapped.connections[0].0.name.clone();

        // Rename: both survive, and the newcomer is marked.
        let mut stores = bare();
        stores.connections.upsert(ConnectionProfile::new(
            taken.clone(),
            "h2",
            "jdbc:h2:mem:x",
            "",
        ));
        let selection = Selection {
            connections: vec![true; mapped.connections.len()],
            ..Selection::default()
        };
        merge(&mapped, &selection, OnConflict::Rename, &mut stores);
        let names: Vec<&str> = stores
            .connections
            .connections()
            .iter()
            .map(|p| p.name.as_str())
            .collect();
        assert!(names.contains(&taken.as_str()));
        assert!(
            names.iter().any(|name| name.ends_with(IMPORTED_SUFFIX)),
            "{names:?}"
        );

        // Skip: what is here stays, and the name is reported.
        let mut stores = bare();
        stores.connections.upsert(ConnectionProfile::new(
            taken.clone(),
            "h2",
            "jdbc:h2:mem:x",
            "",
        ));
        let result = merge(&mapped, &selection, OnConflict::Skip, &mut stores);
        assert!(result.skipped.contains(&taken));
        assert_eq!(
            stores.connections.len(),
            mapped.connections.len(), // one fewer imported, one already there
        );
    }

    #[test]
    fn a_second_clash_of_the_same_name_gets_a_number() {
        let taken = ["staging", "staging (imported)"];
        assert_eq!(
            resolve_name("staging", &taken, OnConflict::Rename).as_deref(),
            Some("staging (imported) 2")
        );
        // Case does not save a name from the clash.
        assert_eq!(
            resolve_name("STAGING", &["staging"], OnConflict::Rename).as_deref(),
            Some("STAGING (imported)")
        );
        assert_eq!(resolve_name("staging", &taken, OnConflict::Skip), None);
        assert_eq!(
            resolve_name("fresh", &taken, OnConflict::Skip).as_deref(),
            Some("fresh")
        );
    }

    #[test]
    fn a_driver_that_is_already_here_is_kept_beside_it_and_the_profiles_follow() {
        // The stock case: jdbgen's H2 maps onto rudbgen's built-in id, which the
        // installation already has. Renaming has to repoint the connections, or
        // they would name a driver that is not there.
        let mut mapped = Mapped {
            connections: Vec::new(),
            drivers: vec![driver("h2", "H2")],
            sets: Vec::new(),
            rules: Vec::new(),
            apply_abbr: false,
            settings_hint: SettingsHint::default(),
            notes: Vec::new(),
        };
        let mut profile = ConnectionProfile::new("dev", "h2", "jdbc:h2:mem:x", "sa");
        profile.driver_id = "h2".into();
        mapped.connections.push((
            profile,
            rudbgen_import::Secret {
                password: String::new(),
            },
        ));

        let mut stores = bare();
        stores.drivers.upsert(driver("h2", "H2 (already here)"));
        let selection = Selection {
            connections: vec![true],
            drivers: vec![true],
            ..Selection::default()
        };
        merge(&mapped, &selection, OnConflict::Rename, &mut stores);
        assert_eq!(stores.drivers.len(), 2);
        let imported = &stores.connections.connections()[0];
        assert_eq!(imported.driver_id, "h2-imported");
        assert!(stores.drivers.get("h2-imported").is_some());
        assert_eq!(
            stores.drivers.get("h2").map(|d| d.name.as_str()),
            Some("H2 (already here)"),
            "the definition that was here is untouched"
        );

        // Skipping leaves the profile on the definition that is already here,
        // which is the same product.
        let mut stores = bare();
        stores.drivers.upsert(driver("h2", "H2 (already here)"));
        merge(&mapped, &selection, OnConflict::Skip, &mut stores);
        assert_eq!(stores.drivers.len(), 1);
        assert_eq!(stores.connections.connections()[0].driver_id, "h2");
    }

    #[test]
    fn a_rule_that_could_never_fire_is_not_appended() {
        let mapped = Mapped {
            connections: Vec::new(),
            drivers: Vec::new(),
            sets: Vec::new(),
            rules: vec![
                rule(false, "EMP", "Employer"),
                rule(false, "NO", "Number"),
                rule(true, "EMP", "Employee"),
            ],
            apply_abbr: true,
            settings_hint: SettingsHint::default(),
            notes: Vec::new(),
        };
        let mut stores = bare();
        // Already here, spelled differently: the dictionary is keyed by the
        // lower-cased abbreviation, so `emp` and `EMP` are one entry.
        stores
            .abbreviations
            .rules
            .push(rule(false, "emp", "Employ"));

        let selection = Selection {
            rules: true,
            apply_abbr: true,
            ..Selection::default()
        };
        let result = merge(&mapped, &selection, OnConflict::Rename, &mut stores);
        assert_eq!(result.rules, 2);
        assert!(result.skipped.contains(&"EMP".to_string()));
        // The whole-name `EMP` is a different dictionary and comes across.
        assert!(
            stores
                .abbreviations
                .rules
                .iter()
                .any(|saved| saved.whole_name && saved.abbreviation == "EMP")
        );
        assert!(stores.abbreviations.apply_to_names);
    }

    #[test]
    fn the_apply_switch_is_only_ever_turned_on() {
        // Unticked leaves whatever the user chose alone, rather than switching
        // the mechanism off behind their back.
        let mapped = Mapped {
            connections: Vec::new(),
            drivers: Vec::new(),
            sets: Vec::new(),
            rules: Vec::new(),
            apply_abbr: false,
            settings_hint: SettingsHint::default(),
            notes: Vec::new(),
        };
        let mut stores = bare();
        stores.abbreviations.apply_to_names = true;
        merge(
            &mapped,
            &Selection {
                rules: true,
                apply_abbr: false,
                ..Selection::default()
            },
            OnConflict::Rename,
            &mut stores,
        );
        assert!(stores.abbreviations.apply_to_names);
    }

    #[test]
    fn a_set_whose_name_is_taken_follows_the_same_policy() {
        let mapped = Mapped {
            connections: Vec::new(),
            drivers: Vec::new(),
            sets: vec![TemplateSet::new("Java + MyBatis", Vec::new())],
            rules: Vec::new(),
            apply_abbr: false,
            settings_hint: SettingsHint::default(),
            notes: Vec::new(),
        };
        let mut stores = bare();
        stores
            .sets
            .upsert(TemplateSet::new("Java + MyBatis", Vec::new()));
        let selection = Selection {
            sets: vec![true],
            ..Selection::default()
        };
        merge(&mapped, &selection, OnConflict::Rename, &mut stores);
        assert_eq!(stores.sets.sets.len(), 2);
        assert!(stores.sets.sets[1].name.ends_with(IMPORTED_SUFFIX));
    }

    #[test]
    fn the_checklist_has_exactly_one_row_per_thing_the_ticks_index_into() {
        // The trap this guards: `Preview::drivers` is one row per entry in the
        // file, while the ticks — and `merge` — index `Mapped::drivers`, which
        // is shorter whenever two jdbgen entries name one product. A checklist
        // drawn from the preview would untick the wrong driver.
        let (dir, mapped) = fixture();
        let config = read(&dir.path().join("config.json")).unwrap();
        let opened = decrypt(&config, MASTER).unwrap();
        let shown = preview(&opened, &MapOptions::new(dir.path()));
        let selection = Selection::everything(&mapped);

        assert_eq!(driver_rows(&mapped).len(), selection.drivers.len());
        assert_eq!(shown.connections.len(), selection.connections.len());
        assert_eq!(shown.sets.len(), selection.sets.len());

        // And the row a tick belongs to is the definition it names.
        for (row, driver) in driver_rows(&mapped).iter().zip(&mapped.drivers) {
            assert_eq!(row.0.as_ref(), driver.name);
        }
    }

    #[test]
    fn two_jdbgen_entries_naming_one_product_are_one_row() {
        let mut mapped = Mapped {
            connections: Vec::new(),
            drivers: vec![driver("h2-embedded", "H2 Embedded")],
            sets: Vec::new(),
            rules: Vec::new(),
            apply_abbr: false,
            settings_hint: SettingsHint::default(),
            notes: Vec::new(),
        };
        assert_eq!(driver_rows(&mapped).len(), 1);
        // A built-in id is marked as such; one the import invented is not.
        assert!(
            driver_rows(&mapped)[0].1.is_some(),
            "h2-embedded is a built-in id"
        );
        mapped.drivers = vec![driver("our-warehouse-1234", "Warehouse")];
        assert!(driver_rows(&mapped)[0].1.is_none());
    }

    #[test]
    fn a_password_that_cannot_be_stored_is_reported_rather_than_lost_silently() {
        let id = Uuid::new_v4();
        let secrets = vec![(id, "hunter2".to_string())];
        let names: HashMap<Uuid, String> = [(id, "staging".to_string())].into_iter().collect();

        let mut good = FakeKeychain::default();
        assert!(store_secrets(&secrets, &names, &mut good).is_empty());
        assert_eq!(good.stored, vec![(id, "hunter2".to_string())]);

        let mut none = FakeKeychain {
            refuse: true,
            ..FakeKeychain::default()
        };
        assert_eq!(
            store_secrets(&secrets, &names, &mut none),
            vec!["staging".to_string()]
        );
    }

    #[test]
    fn a_connection_with_no_password_needs_no_keychain_entry() {
        let (_dir, mapped) = fixture();
        let mut stores = bare();
        let result = merge(
            &mapped,
            &Selection::everything(&mapped),
            OnConflict::Rename,
            &mut stores,
        );
        let with_passwords = mapped
            .connections
            .iter()
            .filter(|(_, secret)| !secret.password.is_empty())
            .count();
        assert_eq!(result.secrets.len(), with_passwords);
    }

    #[test]
    fn the_report_never_carries_a_password() {
        let result = Imported {
            secrets: vec![(Uuid::new_v4(), "hunter2".to_string())],
            ..Imported::default()
        };
        assert!(!format!("{result:?}").contains("hunter2"), "{result:?}");
    }

    #[test]
    fn every_note_the_crate_can_produce_has_a_sentence() {
        let notes = [
            Note::AbbreviationCaseRule,
            Note::LegacyEncryption,
            Note::StockDriverMatched {
                driver: "H2".into(),
                builtin: "h2".into(),
            },
            Note::StockDriverUnknown {
                driver: "X".into(),
                class: "com.example.Driver".into(),
            },
            Note::UnknownDriver {
                connection: "staging".into(),
                driver_type: "Gone".into(),
            },
            Note::KeepAliveNotANumber {
                connection: "staging".into(),
                value: "30 sec".into(),
            },
            Note::UnresolvedPath {
                kind: PathKind::DriverJar,
                owner: "X".into(),
                path: "drivers/x.jar".into(),
            },
            Note::IconDropped {
                owner: "X".into(),
                icon: "fa:database".into(),
            },
        ];
        for note in &notes {
            let text = note_text(note);
            assert!(!text.is_empty(), "{note:?}");
            // `contains`, not `starts_with`: the path note interpolates a
            // second lookup — the kind — and a key that failed to resolve would
            // land in the middle of the sentence rather than at its start.
            assert!(!text.contains("import."), "untranslated {text:?}");
        }
    }

    #[test]
    fn every_failure_to_open_the_file_says_something() {
        for error in [
            Error::WrongPassword,
            Error::Malformed {
                field: "userPassword",
                reason: "shorter than the envelope it has to carry",
            },
            Error::Read {
                path: PathBuf::from("/nowhere/config.json"),
                source: std::io::Error::from(std::io::ErrorKind::NotFound),
            },
        ] {
            let text = message_of(&error);
            assert!(!text.is_empty());
            assert!(!text.starts_with("import."), "untranslated {text:?}");
        }
    }

    /// The wizard opened over `path`, in a window so that `render` runs.
    fn wizard(
        cx: &mut gpui::TestAppContext,
        path: &Path,
    ) -> (Entity<ImportDialog>, gpui::WindowHandle<ImportDialog>) {
        cx.update(|cx| {
            crate::app_settings::init(cx);
            ruui::init(cx);
        });
        let window = cx.add_window(|_, cx| ImportDialog::new(cx));
        let dialog = window
            .update(cx, |_, _, cx| cx.entity())
            .expect("the window is open");
        cx.update(|cx| {
            dialog.update(cx, |dialog, cx| dialog.open(path.to_path_buf(), cx));
        });
        (dialog, window)
    }

    /// What the wizard does with the one password it asks for.
    ///
    /// The whole of step 1, without a configuration directory in sight:
    /// `write_stores` is what touches the disk, and this stops before it. A
    /// wrong password is a message beside the field and another go, not a
    /// failed import — which is the point of `Error::WrongPassword` being one
    /// answer to every cryptographic failure.
    #[gpui::test]
    fn a_wrong_master_password_is_another_go_and_the_right_one_opens_the_checklist(
        cx: &mut gpui::TestAppContext,
    ) {
        let dir = TempDir::new().expect("a temporary jdbgen data directory");
        let path = dir.path().join("config.json");
        fs::write(&path, CONFIG).unwrap();
        cx.executor().allow_parking();
        let (dialog, _window) = wizard(cx, &path);

        cx.update(|cx| {
            dialog.update(cx, |dialog, cx| {
                assert!(dialog.is_open());
                assert!(matches!(dialog.stage, Stage::Password { error: None }));
                let field = dialog.password.clone();
                field.update(cx, |input, cx| input.set_content("not the master", cx));
                dialog.unlock(cx);
                // The field is emptied on the way out: the task owns the only
                // copy from here, in a buffer that wipes itself.
                assert!(dialog.password.read(cx).content().is_empty());
            });
        });
        cx.run_until_parked();
        cx.update(|cx| {
            dialog.update(cx, |dialog, _cx| {
                let Stage::Password {
                    error: Some(message),
                } = &dialog.stage
                else {
                    panic!("a wrong password did not come back as a message: it is not step 1");
                };
                assert_eq!(message, &ts!("import.wrong_password"));
                assert!(dialog.mapped.is_none(), "nothing was mapped");
            });
        });

        // The right one, into the same dialog: a retry is a retry, not a
        // reopen.
        cx.update(|cx| {
            dialog.update(cx, |dialog, cx| {
                let field = dialog.password.clone();
                field.update(cx, |input, cx| input.set_content(MASTER, cx));
                dialog.unlock(cx);
            });
        });
        cx.run_until_parked();
        cx.update(|cx| {
            dialog.update(cx, |dialog, _cx| {
                assert!(
                    matches!(dialog.stage, Stage::Review),
                    "the checklist did not open"
                );
                let mapped = dialog.mapped.as_ref().expect("the mapping is held");
                let shown = dialog.preview.as_ref().expect("the checklist is held");
                assert!(!mapped.connections.is_empty());
                // The D10 announcement is on the checklist whatever the file
                // holds — one of the two places the architecture document says
                // it out loud.
                assert!(shown.notes.contains(&Note::AbbreviationCaseRule));
                // Everything is ticked, and something would therefore happen.
                assert!(!dialog.selection.is_empty());
                assert_eq!(dialog.selection.drivers.len(), mapped.drivers.len());
            });
        });
    }

    /// A file that is not a jdbgen configuration says so rather than asking for
    /// the password again.
    #[gpui::test]
    fn a_file_that_is_not_a_configuration_is_not_a_wrong_password(cx: &mut gpui::TestAppContext) {
        let dir = TempDir::new().expect("a temporary directory");
        let path = dir.path().join("config.json");
        fs::write(&path, b"not json at all").unwrap();
        cx.executor().allow_parking();
        let (dialog, _window) = wizard(cx, &path);

        cx.update(|cx| {
            dialog.update(cx, |dialog, cx| dialog.unlock(cx));
        });
        cx.run_until_parked();
        cx.update(|cx| {
            dialog.update(cx, |dialog, _cx| {
                let Stage::Password {
                    error: Some(message),
                } = &dialog.stage
                else {
                    panic!("the wizard moved on from a file it could not read");
                };
                assert_eq!(message, &ts!("import.not_jdbgen"));
            });
        });
    }

    #[test]
    fn the_labels_the_wizard_draws_are_translated() {
        for label in [
            ts!("import.title"),
            ts!("import.password_hint"),
            ts!("import.master_password"),
            ts!("import.password_placeholder"),
            ts!("import.other_file"),
            ts!("import.choose_file"),
            ts!("import.opening"),
            ts!("import.next"),
            ts!("import.import"),
            ts!("import.importing"),
            ts!("import.section_connections"),
            ts!("import.section_drivers"),
            ts!("import.section_sets"),
            ts!("import.section_rules"),
            ts!("import.section_settings"),
            ts!("import.section_conflicts"),
            ts!("import.section_notes"),
            ts!("import.nothing_here"),
            ts!("import.rules_count", count = "3"),
            ts!("import.apply_abbr"),
            ts!("import.set_templates", count = "2"),
            ts!("import.matches_builtin", builtin = "h2"),
            ts!("import.adopt_settings", language = "ko", theme = "dark"),
            ts!("import.system_language"),
            ts!("import.theme_dark"),
            ts!("import.theme_light"),
            ts!("import.conflict_rename"),
            ts!("import.conflict_rename_hint"),
            ts!("import.conflict_skip_hint"),
            ts!(
                "import.written",
                connections = "1",
                drivers = "2",
                sets = "3",
                rules = "4"
            ),
            ts!("import.nothing_written"),
            ts!("import.skipped", names = "a"),
            ts!("import.no_keychain", names = "a"),
            ts!("import.wrong_password"),
            ts!("import.malformed"),
            ts!("import.unreadable"),
            ts!("import.not_jdbgen"),
        ] {
            assert!(!label.is_empty(), "empty label");
            assert!(
                !label.starts_with("import."),
                "untranslated label {label:?}"
            );
        }
    }
}
