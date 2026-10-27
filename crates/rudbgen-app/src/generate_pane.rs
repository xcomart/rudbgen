//! The Generate tab: what the run will be made of (architecture document,
//! §4.4).
//!
//! Three blocks, top to bottom: the **template set** selector, the **template
//! list**, and the **options** — output directory, author, custom variables and
//! the abbreviation switch. Between them they are the connection's
//! [`GenerationProfile`], and this panel is the only place that edits one:
//! there is no dialog behind it and no second form anywhere else.
//!
//! # Saving
//!
//! Every edit is written back to `connections.json`, debounced by
//! [`DEBOUNCE_MS`]. A debounce alone is not enough, though — a text field
//! notifies its observers when the caret moves as well as when the text
//! changes — so the save is also *diffed*: [`GeneratePane::collect`] builds the
//! profile the widgets currently describe, and the file is only rewritten when
//! it differs from what was last written. Walking the caret through the author
//! field therefore costs nothing.
//!
//! # Sets
//!
//! Picking a set **replaces** the template list, as jdbgen's presets do; it
//! does not merge. The selector shows `Custom` whenever the list matches no
//! saved set, which [`matching_set`] decides — an exact, ordered comparison of
//! every field, ticks included, because a template unticked is a list that no
//! longer describes the set it came from.
//!
//! *Save as set…* under a name a set already has **overwrites** that set in
//! place, which [`store_set`] decides; the prompt opens on the name of the set
//! the list came from, so writing an edited list back over its set is one
//! click.

use std::path::{Path, PathBuf};

use gpui::{
    App, Context, Entity, EventEmitter, FocusHandle, Focusable, PathPromptOptions, ScrollHandle,
    SharedString, Subscription, Task, Window, div, prelude::*, px,
};
use rudbgen_core::{
    AbbreviationStore, ConnectionStore, GenerationProfile, TemplateRef, TemplateSet,
    TemplateSetStore, config_dir,
};
use rugpui::{
    Button, ButtonVariant, Checkbox, Scrollbar, ScrollbarAxis, ScrollbarState, Select, TextInput,
    Theme, form_row, hide_later, hide_now, modal, scroll_to, scrolled, theme, tooltip_label,
};
use uuid::Uuid;

use crate::i18n::ts;
use crate::icons;

/// How long the panel waits after the last edit before writing
/// `connections.json`.
///
/// Half a second: long enough that typing a package name is one write and not
/// twenty, short enough that a user who edits a field and immediately closes
/// the window has their edit on disk.
pub const DEBOUNCE_MS: u64 = 500;

/// Width of the label column of the options block.
const LABEL_WIDTH: f32 = 110.;

/// Width of the template set selector.
const SET_WIDTH: f32 = 220.;

/// Narrowest the output name field of a template row gets; it grows with the
/// window beyond this.
const OUT_MIN_WIDTH: f32 = 200.;

/// Element id of the panel's scrolling box.
const PANE_SCROLL: &str = "generate-scroll";

/// Element id the panel's overlay scroll indicator is drawn under.
const PANE_SCROLLBAR: &str = "generate-scrollbar";

/// What the panel tells the shell about.
pub enum GeneratePaneEvent {
    /// The profile changed: the status bar has to recount.
    Changed,
    /// The pencil beside a template row was pressed; the shell opens the tab.
    EditTemplate(PathBuf),
    /// *Rules…* was pressed; the shell opens the abbreviation rules editor.
    ///
    /// The panel does not open it itself for the reason no panel opens a
    /// dialog here: the modals are the shell's, one at a time, and a panel that
    /// held one could not be closed with the tab it lives in.
    EditAbbreviations,
}

/// Why the run cannot be started.
///
/// Ordered by what the user has to do first, which is the order the status bar
/// reports them in: a tooltip may only name one reason, and naming the last one
/// while the first is still true would send the user to the wrong control.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Blocker {
    /// Nothing is connected.
    Disconnected,
    /// No table is ticked in the explorer.
    NoTables,
    /// No template is ticked in this panel.
    NoTemplates,
    /// No output directory has been chosen.
    NoOutputDir,
}

impl Blocker {
    /// The translated reason, for the tooltip on the disabled button.
    pub fn message(self) -> SharedString {
        match self {
            Blocker::Disconnected => ts!("generate.blocked_connection"),
            Blocker::NoTables => ts!("generate.blocked_tables"),
            Blocker::NoTemplates => ts!("generate.blocked_templates"),
            Blocker::NoOutputDir => ts!("generate.blocked_output"),
        }
    }
}

/// The arithmetic of §4.2's status bar: tables × templates → files.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Readiness {
    /// Tables ticked in the explorer.
    pub tables: usize,
    /// Templates ticked in this panel.
    pub templates: usize,
    /// What the run would write, if nothing were in its way.
    pub files: usize,
    /// Why the run cannot start, if it cannot.
    pub blocker: Option<Blocker>,
}

/// Works out what the status bar says and whether the buttons are live.
///
/// Pure, and the one place the rule lives: the count is a plain product, and
/// the reason is the *first* thing missing rather than a list, because a
/// tooltip that names four problems tells the user nothing about which control
/// to reach for.
pub fn readiness(
    connected: bool,
    tables: usize,
    templates: usize,
    output_dir: Option<&Path>,
) -> Readiness {
    let blocker = if !connected {
        Some(Blocker::Disconnected)
    } else if tables == 0 {
        Some(Blocker::NoTables)
    } else if templates == 0 {
        Some(Blocker::NoTemplates)
    } else if output_dir.is_none_or(|dir| dir.as_os_str().is_empty()) {
        Some(Blocker::NoOutputDir)
    } else {
        None
    };
    Readiness {
        tables,
        templates,
        files: tables * templates,
        blocker,
    }
}

/// The saved set `templates` is exactly, if it is any of them.
///
/// An ordered, field-by-field comparison including [`TemplateRef::selected`]:
/// applying a set copies its rows verbatim, so anything else the user does to
/// the list — reordering it, unticking a row, editing an output name — has
/// diverged from the set and the selector says `Custom`.
pub fn matching_set(sets: &[TemplateSet], templates: &[TemplateRef]) -> Option<Uuid> {
    sets.iter()
        .find(|set| set.templates == templates)
        .map(|set| set.id)
}

/// The saved set called `name`, if there is one.
///
/// Compared trimmed and case for case, because that is how a name is saved and
/// how the selector shows it. A file written before sets could be overwritten
/// may hold two under one name; the first is the one that answers.
pub fn set_named(sets: &[TemplateSet], name: &str) -> Option<Uuid> {
    let name = name.trim();
    if name.is_empty() {
        return None;
    }
    sets.iter()
        .find(|set| set.name.trim() == name)
        .map(|set| set.id)
}

/// Stores `templates` as the set called `name` and answers its id.
///
/// A name that is already a set's overwrites that set — same id, same place in
/// the list — instead of adding a second one the selector could not tell from
/// the first. Any other name is a new set at the end.
pub fn store_set(sets: &mut TemplateSetStore, name: String, templates: Vec<TemplateRef>) -> Uuid {
    let mut set = TemplateSet::new(name, templates);
    if let Some(id) = set_named(&sets.sets, &set.name) {
        set.id = id;
    }
    let id = set.id;
    sets.upsert(set);
    id
}

/// The custom variables of a key/value table, as they are saved.
///
/// A row whose key is blank is the empty row at the end of the table, or one
/// the user emptied to remove it; either way it is dropped. Keys are trimmed —
/// a stray space would make `package ` a variable no template can name — and
/// values are kept as they were typed, because trailing space in a value can be
/// deliberate.
pub fn collect_vars(rows: &[(String, String)]) -> Vec<(String, String)> {
    rows.iter()
        .map(|(key, value)| (key.trim().to_string(), value.clone()))
        .filter(|(key, _)| !key.is_empty())
        .collect()
}

/// Whether the variable table needs one more blank row at its foot.
///
/// The rule §4.4 calls "the trailing empty row": there is always exactly one
/// row to type a new variable into, and it appears the moment the last one is
/// filled in rather than after pressing a button.
pub fn needs_trailing_blank(rows: &[(String, String)]) -> bool {
    match rows.last() {
        None => true,
        Some((key, value)) => !key.trim().is_empty() || !value.is_empty(),
    }
}

/// A template file path as the generator has to open it.
///
/// §5 stores a path relative to the configuration directory when it is below
/// it, and absolute otherwise; this is the half that reads one back. A relative
/// path with no configuration directory to resolve against is handed back as it
/// stands, which fails later with a message naming the file rather than here
/// with one naming the home directory.
pub fn resolve_template(file: &Path) -> PathBuf {
    if file.is_absolute() {
        return file.to_path_buf();
    }
    match config_dir() {
        Ok(root) => root.join(file),
        Err(_) => file.to_path_buf(),
    }
}

/// A template file path as the profile stores it: relative to the configuration
/// directory when it is below it, absolute otherwise (§5).
pub fn store_template_path(file: &Path) -> PathBuf {
    let Ok(root) = config_dir() else {
        return file.to_path_buf();
    };
    match file.strip_prefix(&root) {
        Ok(relative) => relative.to_path_buf(),
        Err(_) => file.to_path_buf(),
    }
}

/// Re-reads a global store, applies `change` to that copy, and writes it back.
///
/// What the panel keeps of `template-sets.json` and `abbreviations.json` was
/// read when the connection opened, and the import wizard or the rules editor
/// may have written either file since; saving that copy would undo them. So
/// every change starts from the file as it is now, and the store handed back
/// is the one that was written, for the panel to keep instead.
///
/// # Errors
///
/// A file that cannot be read is left exactly as it is: `change` is not
/// applied and nothing is written. Starting from an empty store instead, as a
/// reader may, would have the save replace the file with only this change. A
/// failed write is the other error, and leaves the panel's copy the one to
/// keep.
pub fn read_modify_write<S>(
    load: impl FnOnce() -> anyhow::Result<S>,
    change: impl FnOnce(&mut S),
    save: impl FnOnce(&S) -> anyhow::Result<()>,
) -> anyhow::Result<S> {
    let mut store = load()?;
    change(&mut store);
    save(&store)?;
    Ok(store)
}

/// One row of the template list, as the panel holds it.
///
/// The name and the output template are text fields rather than strings for the
/// reason the connection dialog's property rows are: a value being typed passes
/// through states the saved model cannot represent, and rebuilding the list on
/// every keystroke would pull the caret out from under the user.
struct TemplateRow {
    /// Whether the template takes part in the next run.
    selected: bool,
    /// The file, exactly as the profile stores it.
    file: PathBuf,
    /// What the list calls it.
    name: Entity<TextInput>,
    /// The output name template.
    out: Entity<TextInput>,
    /// Keeps the two observers above alive.
    _subs: [Subscription; 2],
}

/// One row of the custom variable table.
struct VarRow {
    /// The variable's name.
    key: Entity<TextInput>,
    /// Its value.
    value: Entity<TextInput>,
    /// Keeps the two observers above alive.
    _subs: [Subscription; 2],
}

/// The Generate tab.
pub struct GeneratePane {
    focus_handle: FocusHandle,
    /// Which connection's profile is being edited, and where the save goes.
    profile_id: Option<Uuid>,
    /// The profile as it was last written, which every save is diffed against.
    saved: GenerationProfile,
    /// The template list.
    templates: Vec<TemplateRow>,
    /// The custom variable table, always with one blank row at its foot.
    vars: Vec<VarRow>,
    /// The output directory field.
    output: Entity<TextInput>,
    /// The author field.
    author: Entity<TextInput>,
    /// The saved sets, re-read whenever the panel is pointed at a connection.
    ///
    /// What the selector shows, never what a change is saved from: see
    /// [`read_modify_write`].
    sets: TemplateSetStore,
    /// Why the sets cannot be changed, shown under the selector.
    ///
    /// Set when `template-sets.json` could not be read — the selector is empty
    /// then — or written. A save or delete still re-reads the file first, so it
    /// refuses for as long as the file stays unreadable and goes through, and
    /// clears this, once it has been fixed.
    sets_error: Option<SharedString>,
    /// The abbreviation rules — one global store, not part of the profile.
    abbr: AbbreviationStore,
    /// Why the abbreviation switch cannot be changed; the same as
    /// [`Self::sets_error`], for `abbreviations.json`.
    abbr_error: Option<SharedString>,
    /// Whether the set dropdown is showing.
    set_open: bool,
    /// Whether the "Save as set…" prompt is up.
    naming_set: bool,
    /// The name being typed into that prompt.
    set_name: Entity<TextInput>,
    /// The set the list was last taken from or saved as, which names the
    /// "Save as set…" prompt opens on.
    ///
    /// Kept apart from [`matching_set`] because that one is lost with the
    /// first edit, and the edited list is exactly what gets saved back. A set
    /// deleted since is simply not found, and the prompt opens empty.
    origin_set: Option<Uuid>,
    /// The set the delete-confirmation prompt is up for, if any.
    deleting_set: Option<Uuid>,
    /// Vertical scroll of the panel.
    scroll: ScrollHandle,
    /// Whether the panel's overlay scroll indicator is on screen.
    scrollbar: ScrollbarState,
    /// The debounce behind the save; dropped — and so canceled — by the next
    /// edit.
    _debounce: Option<Task<()>>,
    /// Keeps the two fixed fields' observers alive.
    _subs: Vec<Subscription>,
}

impl GeneratePane {
    /// An empty panel, pointed at no connection.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let output =
            cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.output_placeholder")));
        let author =
            cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.author_placeholder")));
        let set_name = cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.set_name")));
        let subs = vec![
            cx.observe(&output, |pane, _, cx| pane.touch(cx)),
            cx.observe(&author, |pane, _, cx| pane.touch(cx)),
            // The prompt's button reads Overwrite while the name is a set's,
            // so the panel redraws as the name is typed.
            cx.observe(&set_name, |pane, _, cx| {
                if pane.naming_set {
                    cx.notify();
                }
            }),
        ];

        Self {
            focus_handle: cx.focus_handle(),
            profile_id: None,
            saved: GenerationProfile::default(),
            templates: Vec::new(),
            vars: Vec::new(),
            output,
            author,
            sets: TemplateSetStore::default(),
            sets_error: None,
            abbr: AbbreviationStore::default(),
            abbr_error: None,
            set_open: false,
            naming_set: false,
            set_name,
            origin_set: None,
            deleting_set: None,
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
            _debounce: None,
            _subs: subs,
        }
    }

    // --- what the shell asks it ------------------------------------------

    /// Points the panel at a connection's generation profile.
    ///
    /// Called by every connect. The sets and the abbreviation rules are re-read
    /// here rather than kept: both are global files another part of the
    /// application may have written since the last connection.
    ///
    /// A file that cannot be read shows as an empty store with the reason
    /// beside it. The empty store is only ever *shown*: every change re-reads
    /// the file (see [`read_modify_write`]), so it is never what gets saved
    /// over the one that could not be read.
    pub fn load(&mut self, id: Uuid, profile: &GenerationProfile, cx: &mut Context<Self>) {
        self.profile_id = Some(id);
        self.saved = profile.clone();
        (self.sets, self.sets_error) = match TemplateSetStore::load() {
            Ok(sets) => (sets, None),
            Err(error) => {
                log::error!("could not read template-sets.json: {error:#}");
                (TemplateSetStore::default(), Some(sets_failed(&error)))
            }
        };
        (self.abbr, self.abbr_error) = match AbbreviationStore::load() {
            Ok(rules) => (rules, None),
            Err(error) => {
                log::error!("could not read abbreviations.json: {error:#}");
                (AbbreviationStore::default(), Some(abbr_failed(&error)))
            }
        };
        self.origin_set = matching_set(&self.sets.sets, &profile.templates);
        self.rebuild(profile, cx);
    }

    /// Empties the panel; the connection it was editing is gone.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.profile_id = None;
        self.saved = GenerationProfile::default();
        self.origin_set = None;
        self._debounce = None;
        self.rebuild(&GenerationProfile::default(), cx);
    }

    /// The profile as the widgets currently describe it.
    ///
    /// The one place the panel is turned back into what is saved, so the diff
    /// the debounce makes and the plan a run is built from cannot disagree.
    pub fn collect(&self, cx: &App) -> GenerationProfile {
        let rows: Vec<(String, String)> = self
            .vars
            .iter()
            .map(|row| (text(&row.key, cx), text(&row.value, cx)))
            .collect();
        let output = text(&self.output, cx);
        GenerationProfile {
            templates: self
                .templates
                .iter()
                .map(|row| TemplateRef {
                    name: text(&row.name, cx),
                    file: row.file.clone(),
                    out_template: text(&row.out, cx),
                    selected: row.selected,
                })
                .collect(),
            output_dir: Some(PathBuf::from(output)).filter(|dir| !dir.as_os_str().is_empty()),
            author: text(&self.author, cx),
            custom_vars: collect_vars(&rows),
        }
    }

    /// The abbreviation rules, which a run needs and only this panel has read.
    pub fn abbreviations(&self) -> &AbbreviationStore {
        &self.abbr
    }

    /// Why the panel could not read or write `abbreviations.json` the last
    /// time it tried, if it could not.
    ///
    /// While this stands [`Self::abbreviations`] may be an empty stand-in
    /// rather than the user's rules.
    pub fn abbreviations_error(&self) -> Option<&SharedString> {
        self.abbr_error.as_ref()
    }

    /// Takes on the rules the editor has just written.
    ///
    /// The dialog saved the file; this replaces the panel's copy with the same
    /// value, so the switch on screen and the rules a run applies come from one
    /// place. Deliberately does **not** save: writing the file twice would be
    /// the second answer to a question already answered.
    pub fn adopt_abbreviations(&mut self, store: AbbreviationStore, cx: &mut Context<Self>) {
        self.abbr = store;
        // The caller has just read or written the file, so whatever kept the
        // switch from changing is no longer true.
        self.abbr_error = None;
        cx.emit(GeneratePaneEvent::Changed);
        cx.notify();
    }

    /// How many templates are ticked.
    pub fn selected_templates(&self) -> usize {
        self.templates.iter().filter(|row| row.selected).count()
    }

    /// The ticked templates' names, in list order — the preview header's
    /// dropdown, and the labels a summary reports against.
    pub fn selected_names(&self, cx: &App) -> Vec<SharedString> {
        self.templates
            .iter()
            .filter(|row| row.selected)
            .map(|row| SharedString::from(text(&row.name, cx)))
            .collect()
    }

    /// The output directory as it stands, absolute where the user typed a
    /// relative one — resolved against the home directory, which is the only
    /// place a bare `out/` could sensibly mean.
    pub fn output_dir(&self, cx: &App) -> Option<PathBuf> {
        let typed = text(&self.output, cx);
        if typed.is_empty() {
            return None;
        }
        let path = PathBuf::from(typed);
        if path.is_absolute() {
            return Some(path);
        }
        Some(
            directories::UserDirs::new()
                .map(|dirs| dirs.home_dir().join(&path))
                .unwrap_or(path),
        )
    }

    /// Writes the profile now, if it has changed, whatever the debounce was
    /// waiting for.
    ///
    /// The shell calls it before every run and when the connection closes, so a
    /// run always uses what is on screen and a window that closes mid-edit does
    /// not lose the edit.
    pub fn flush(&mut self, cx: &mut Context<Self>) {
        self._debounce = None;
        self.save_now(cx);
    }

    // --- editing ----------------------------------------------------------

    /// Records an edit: the status bar recounts, and the save clock restarts.
    fn touch(&mut self, cx: &mut Context<Self>) {
        cx.emit(GeneratePaneEvent::Changed);
        cx.notify();
        self.schedule_save(cx);
    }

    /// Restarts the debounce, dropping whatever was pending.
    fn schedule_save(&mut self, cx: &mut Context<Self>) {
        if self.profile_id.is_none() {
            return;
        }
        self._debounce = Some(cx.spawn(async move |pane, cx| {
            cx.background_executor()
                .timer(std::time::Duration::from_millis(DEBOUNCE_MS))
                .await;
            pane.update(cx, |pane, cx| pane.save_now(cx)).ok();
        }));
    }

    /// Writes `connections.json`, if there is anything to write.
    ///
    /// The diff is what makes the observers safe to hang off a text field: a
    /// field notifies when the caret moves as well as when the text changes,
    /// and a caret that moved describes exactly the profile that is already on
    /// disk.
    fn save_now(&mut self, cx: &mut Context<Self>) {
        let Some(id) = self.profile_id else {
            return;
        };
        let profile = self.collect(cx);
        if profile == self.saved {
            return;
        }
        let mut store = match ConnectionStore::load() {
            Ok(store) => store,
            Err(error) => {
                log::error!("could not read connections.json: {error:#}");
                return;
            }
        };
        let Some(mut connection) = store.get(id).cloned() else {
            log::warn!("the connection being edited is no longer saved");
            return;
        };
        connection.generation = profile.clone();
        store.upsert(connection);
        if let Err(error) = store.save() {
            log::error!("could not write connections.json: {error:#}");
            return;
        }
        self.saved = profile;
    }

    /// Rebuilds every row from `profile`.
    fn rebuild(&mut self, profile: &GenerationProfile, cx: &mut Context<Self>) {
        self.templates.clear();
        for template in &profile.templates {
            let row = self.new_template_row(template, cx);
            self.templates.push(row);
        }

        self.vars.clear();
        for (key, value) in &profile.custom_vars {
            let row = self.new_var_row(cx);
            set_text(&row.key, key.clone(), cx);
            set_text(&row.value, value.clone(), cx);
            self.vars.push(row);
        }
        self.ensure_blank_var(cx);

        set_text(
            &self.output,
            profile
                .output_dir
                .as_ref()
                .map(|dir| dir.display().to_string())
                .unwrap_or_default(),
            cx,
        );
        set_text(&self.author, profile.author.clone(), cx);
        cx.emit(GeneratePaneEvent::Changed);
        cx.notify();
    }

    /// One template row, with its two fields already observed.
    fn new_template_row(&self, template: &TemplateRef, cx: &mut Context<Self>) -> TemplateRow {
        let name = cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.template_name")));
        let out = cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.out_template")));
        set_text(&name, template.name.clone(), cx);
        set_text(&out, template.out_template.clone(), cx);
        let subs = [
            cx.observe(&name, |pane, _, cx| pane.touch(cx)),
            cx.observe(&out, |pane, _, cx| pane.touch(cx)),
        ];
        TemplateRow {
            selected: template.selected,
            file: template.file.clone(),
            name,
            out,
            _subs: subs,
        }
    }

    /// One variable row, with its two fields already observed.
    fn new_var_row(&self, cx: &mut Context<Self>) -> VarRow {
        let key = cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.var_key")));
        let value = cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.var_value")));
        let subs = [
            cx.observe(&key, |pane, _, cx| pane.var_edited(cx)),
            cx.observe(&value, |pane, _, cx| pane.var_edited(cx)),
        ];
        VarRow {
            key,
            value,
            _subs: subs,
        }
    }

    /// A variable field was touched: the table grows a blank row if it needs
    /// one, and the save clock restarts.
    fn var_edited(&mut self, cx: &mut Context<Self>) {
        self.ensure_blank_var(cx);
        self.touch(cx);
    }

    /// Keeps exactly one blank row at the foot of the variable table.
    fn ensure_blank_var(&mut self, cx: &mut Context<Self>) {
        let rows: Vec<(String, String)> = self
            .vars
            .iter()
            .map(|row| (text(&row.key, cx), text(&row.value, cx)))
            .collect();
        if needs_trailing_blank(&rows) {
            let row = self.new_var_row(cx);
            self.vars.push(row);
        }
    }

    /// Ticks or unticks one template.
    fn toggle_template(&mut self, index: usize, selected: bool, cx: &mut Context<Self>) {
        let Some(row) = self.templates.get_mut(index) else {
            return;
        };
        row.selected = selected;
        self.touch(cx);
    }

    /// Drops one template from the list.
    fn remove_template(&mut self, index: usize, cx: &mut Context<Self>) {
        if index >= self.templates.len() {
            return;
        }
        self.templates.remove(index);
        self.touch(cx);
    }

    /// Replaces the list with the templates of `set`.
    ///
    /// A replacement and not a merge, which is what jdbgen's presets do: a set
    /// is a description of a whole run, and half of one is not a set.
    fn apply_set(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let Some(set) = self.sets.get(id).cloned() else {
            return;
        };
        self.templates.clear();
        for template in &set.templates {
            let row = self.new_template_row(template, cx);
            self.templates.push(row);
        }
        self.origin_set = Some(id);
        self.touch(cx);
    }

    /// Opens the "Save as set…" prompt on the name of the set the list came
    /// from, or empty when there is none.
    fn open_set_prompt(&mut self, cx: &mut Context<Self>) {
        let name = self
            .origin_set
            .and_then(|id| self.sets.get(id))
            .map(|set| set.name.clone())
            .unwrap_or_default();
        self.naming_set = true;
        set_text(&self.set_name, name, cx);
        cx.notify();
    }

    /// Saves the current list as the set called `name`: a new one, or the one
    /// already called that, overwritten (see [`store_set`]).
    fn save_as_set(&mut self, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let templates = self.collect(cx).templates;
        let mut stored = None;
        let written = read_modify_write(
            TemplateSetStore::load,
            |sets| {
                // The name is matched against the file as it is now, not the
                // copy the prompt was drawn from, so a set another window
                // added meanwhile is overwritten rather than doubled.
                stored = Some(store_set(sets, name, templates));
                // The flag goes with it: a store with a set of the user's own
                // in it is no longer a first run, whatever it was before.
                sets.builtins_seeded = true;
            },
            TemplateSetStore::save,
        );
        if written.is_ok() {
            self.origin_set = stored;
        }
        self.keep_sets(written);
        self.naming_set = false;
        cx.notify();
    }

    /// Removes a saved set. The template list on screen is untouched — a
    /// deleted set is one fewer thing to pick, not an instruction to clear
    /// what is already ticked.
    fn delete_set(&mut self, id: Uuid, cx: &mut Context<Self>) {
        let written = read_modify_write(
            TemplateSetStore::load,
            |sets| {
                sets.remove(id);
            },
            TemplateSetStore::save,
        );
        self.keep_sets(written);
        self.deleting_set = None;
        cx.notify();
    }

    /// Keeps the sets a change wrote, or the reason it wrote nothing.
    fn keep_sets(&mut self, written: anyhow::Result<TemplateSetStore>) {
        match written {
            Ok(sets) => {
                self.sets = sets;
                self.sets_error = None;
            }
            Err(error) => {
                log::error!("template-sets.json was not changed: {error:#}");
                self.sets_error = Some(sets_failed(&error));
            }
        }
    }

    /// Asks the platform for template files and appends what it hands back.
    ///
    /// §4.4 asks for the picker to start in `<config>/templates`, which is
    /// where the built-ins land. gpui's [`PathPromptOptions`] carries no
    /// starting directory, so it starts wherever the platform last left it;
    /// the day the option exists, `rudbgen_core::templates_dir` is what goes in it.
    ///
    /// Nothing waits on the prompt: on X11 that call is the one gpui had to be
    /// patched around, so the click returns immediately and the answer is
    /// picked up on a task of its own — the shape `driver_manager` and the
    /// settings dialog both use.
    fn add_templates(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: true,
            prompt: Some(ts!("generate.add_template_select")),
        });
        cx.spawn(async move |pane, cx| {
            let chosen = match paths.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    log::warn!("the file picker could not be opened: {error:#}");
                    return;
                }
            };
            pane.update(cx, |pane, cx| pane.install_templates(chosen, cx))
                .ok();
        })
        .detach();
    }

    /// Appends `paths` to the template list, skipping ones already in it.
    ///
    /// The name starts as the file's stem and the output name as the file's own
    /// name with the table's pascal-case name in front of it, which is a
    /// template that renders to something plausible rather than to an empty
    /// string — the user then edits both in place.
    fn install_templates(&mut self, paths: Vec<PathBuf>, cx: &mut Context<Self>) {
        for path in paths {
            let stored = store_template_path(&path);
            if self.templates.iter().any(|row| row.file == stored) {
                continue;
            }
            let stem = path
                .file_stem()
                .map(|stem| stem.to_string_lossy().into_owned())
                .unwrap_or_else(|| stored.display().to_string());
            let extension = path
                .extension()
                .map(|ext| format!(".{}", ext.to_string_lossy()))
                .unwrap_or_default();
            let template = TemplateRef {
                name: stem,
                file: stored,
                out_template: format!("${{name.suffix.pascal}}{extension}"),
                selected: true,
            };
            let row = self.new_template_row(&template, cx);
            self.templates.push(row);
        }
        self.touch(cx);
    }

    /// Asks the platform for the output directory.
    fn choose_output(&mut self, cx: &mut Context<Self>) {
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(ts!("generate.output_select")),
        });
        cx.spawn(async move |pane, cx| {
            let chosen = match paths.await {
                Ok(Ok(Some(paths))) => paths,
                Ok(Ok(None)) | Err(_) => return,
                Ok(Err(error)) => {
                    log::warn!("the directory picker could not be opened: {error:#}");
                    return;
                }
            };
            let Some(directory) = chosen.into_iter().next() else {
                return;
            };
            pane.update(cx, |pane, cx| {
                set_text(&pane.output, directory.display().to_string(), cx);
                pane.touch(cx);
            })
            .ok();
        })
        .detach();
    }

    /// Flips the global abbreviation switch and writes it straight away.
    ///
    /// Not debounced: it is one boolean and one click, and the rules file is
    /// not the connection's. The rules come along with the switch: the panel
    /// keeps the file as it was written, rules another window added included.
    /// When it cannot be written the switch stays where it was.
    fn toggle_abbreviations(&mut self, on: bool, cx: &mut Context<Self>) {
        let written = read_modify_write(
            AbbreviationStore::load,
            |store| store.apply_to_names = on,
            AbbreviationStore::save,
        );
        match written {
            Ok(store) => {
                self.abbr = store;
                self.abbr_error = None;
            }
            Err(error) => {
                log::error!("abbreviations.json was not changed: {error:#}");
                self.abbr_error = Some(abbr_failed(&error));
            }
        }
        cx.emit(GeneratePaneEvent::Changed);
        cx.notify();
    }

    // --- the scroll bar ---------------------------------------------------

    /// The panel's overlay scroll indicator, as it now stands.
    fn bar(&self) -> Scrollbar {
        Scrollbar::for_handle(PANE_SCROLLBAR, ScrollbarAxis::Vertical, &self.scroll)
            .fade(self.scrollbar.fade())
    }

    /// Puts the bar up whenever the panel has moved, and starts the clock that
    /// takes it down again.
    fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        let scrolled = scrolled(&self.scroll, ScrollbarAxis::Vertical);
        if let Some(epoch) = self.scrollbar.moved(scrolled) {
            hide_later(epoch, cx, move |pane: &mut Self| Some(&mut pane.scrollbar));
        }
    }

    /// Scrolls the panel when its thumb is dragged.
    pub fn drag_scrollbar(
        &mut self,
        event: &gpui::DragMoveEvent<rugpui::DraggedThumb>,
        cx: &mut Context<Self>,
    ) {
        let Some(progress) = self.bar().dragged(event, cx) else {
            return;
        };
        self.scrollbar.hold();
        scroll_to(&self.scroll, ScrollbarAxis::Vertical, progress);
        cx.notify();
    }

    /// Lets go of the thumb, and starts its clock again.
    pub fn release_scrollbar(&mut self, cx: &mut Context<Self>) {
        if let Some(epoch) = self.scrollbar.release() {
            hide_later(epoch, cx, move |pane: &mut Self| Some(&mut pane.scrollbar));
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
        let Some(epoch) = self.scrollbar.hover_leave() else {
            return;
        };
        hide_now(self, epoch, cx, move |pane: &mut Self| {
            Some(&mut pane.scrollbar)
        });
    }

    // --- rendering --------------------------------------------------------

    /// The template set selector and the button that saves the list as one.
    fn render_sets(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let current = self.collect(cx).templates;
        let matched = matching_set(&self.sets.sets, &current);
        let options: Vec<SharedString> = self
            .sets
            .sets
            .iter()
            .map(|set| SharedString::from(set.name.clone()))
            .collect();
        // `Custom` is a label and not a row: it names the state the list is in
        // once it matches no set, and there is nothing to pick that would put
        // the list there. The trigger shows it without the list offering it,
        // which `Select` allows — a selection outside the options just
        // highlights no row.
        let selected = matched
            .and_then(|id| self.sets.get(id))
            .map(|set| SharedString::from(set.name.clone()))
            .unwrap_or_else(|| ts!("generate.set_custom"));

        let ids: Vec<Uuid> = self.sets.sets.iter().map(|set| set.id).collect();
        let this = cx.entity();
        let toggle = cx.entity();
        let naming = cx.entity();
        let deleting = cx.entity();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .min_w_0()
            .debug_selector(|| "generate-sets".to_string())
            .child(
                Select::new("generate-set")
                    .chevron_icon(icons::CHEVRON_DOWN)
                    .options(options)
                    .selected(Some(selected))
                    .open(self.set_open)
                    .width(px(SET_WIDTH))
                    .on_select(move |index, _text, _window, cx| {
                        this.update(cx, |pane, cx| {
                            pane.set_open = false;
                            if let Some(id) = ids.get(index).copied() {
                                pane.apply_set(id, cx);
                            }
                            cx.notify();
                        });
                    })
                    .on_open_change(move |open, _window, cx| {
                        toggle.update(cx, |pane, cx| {
                            pane.set_open = open;
                            cx.notify();
                        });
                    }),
            )
            .child(
                Button::new("generate-save-set", ts!("generate.save_as_set"))
                    .variant(ButtonVariant::Secondary)
                    .disabled(self.templates.is_empty())
                    .on_click(move |_, window, cx| {
                        naming.update(cx, |pane, cx| pane.open_set_prompt(cx));
                        let handle = naming.read(cx).set_name.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }),
            )
            .child(
                Button::new("generate-delete-set", ts!("generate.delete_set"))
                    .variant(ButtonVariant::Secondary)
                    .disabled(matched.is_none())
                    .on_click(move |_, _window, cx| {
                        deleting.update(cx, |pane, cx| {
                            pane.deleting_set = matched;
                            cx.notify();
                        });
                    }),
            )
    }

    /// The template list: one row per template, and the button that adds one.
    fn render_templates(&self, theme: &Theme, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let rows: Vec<_> = self
            .templates
            .iter()
            .enumerate()
            .map(|(index, row)| self.render_template_row(index, row, theme, cx))
            .collect();
        let this = cx.entity();

        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .min_w_0()
            .debug_selector(|| "generate-templates".to_string())
            .when(rows.is_empty(), |list| {
                list.child(
                    div()
                        .py(px(8.))
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child(ts!("generate.no_templates")),
                )
            })
            .children(rows)
            .child(
                div().pt(px(4.)).child(
                    Button::new("generate-add-template", ts!("generate.add_template"))
                        .variant(ButtonVariant::Secondary)
                        .on_click(move |_, _window, cx| {
                            this.update(cx, |pane, cx| pane.add_templates(cx));
                        }),
                ),
            )
    }

    /// One template row: the tick, the name, the file, the output name, and the
    /// two glyphs at the end.
    fn render_template_row(
        &self,
        index: usize,
        row: &TemplateRow,
        theme: &Theme,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let tick = cx.entity();
        let remove = cx.entity();
        let edit = cx.entity();
        let file = row.file.clone();
        let shown = SharedString::from(row.file.display().to_string());

        div()
            .id(("generate-template", index))
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .w_full()
            .min_w_0()
            .child(
                Checkbox::new(("generate-tick", index), "")
                    .checked(row.selected)
                    .on_toggle(move |checked, _window, cx| {
                        tick.update(cx, |pane, cx| pane.toggle_template(index, checked, cx));
                    }),
            )
            .child(div().w(px(150.)).flex_none().child(row.name.clone()))
            .child(
                // The file, and the second way into its tab: a double click on
                // the row opens what the pencil beside it opens. A *single*
                // click deliberately does nothing — the row is a form, and a
                // click on a form is how a caret is placed, not how a document
                // is opened.
                div()
                    .id(("generate-file", index))
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .cursor_pointer()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .tooltip(tooltip_label(ts!("generate.tip_open")))
                    .on_click({
                        let open = cx.entity();
                        let file = row.file.clone();
                        move |event: &gpui::ClickEvent, _window, cx| {
                            if event.click_count() < 2 {
                                return;
                            }
                            let file = file.clone();
                            open.update(cx, |_, cx| {
                                cx.emit(GeneratePaneEvent::EditTemplate(file));
                            });
                        }
                    })
                    .child(shown.clone()),
            )
            .child(
                // The output name takes two thirds of the room the window gives
                // the row, the file the rest: a name that writes into a package
                // tree is the longest thing a row holds, and the one typed in.
                div()
                    .debug_selector(move || format!("generate-out-{index}"))
                    .flex_1()
                    .flex_grow(2.)
                    .min_w(px(OUT_MIN_WIDTH))
                    .child(row.out.clone()),
            )
            .child(
                // The way into the template tab (§4.5). The path the event
                // carries is the profile's, which the shell resolves against
                // the configuration directory before opening it.
                div()
                    .id(("generate-edit", index))
                    .tab_index(0)
                    .focus(|style| style.bg(theme.surface_active).text_color(theme.accent))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(theme.icon)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface_hover).text_color(theme.text))
                    .tooltip(tooltip_label(ts!("generate.tip_edit")))
                    .on_click(move |_, _window, cx| {
                        let file = file.clone();
                        edit.update(cx, |_, cx| {
                            cx.emit(GeneratePaneEvent::EditTemplate(file));
                        });
                    })
                    .child("\u{270e}"),
            )
            .child(
                div()
                    .id(("generate-remove", index))
                    .tab_index(0)
                    .focus(|style| style.bg(theme.surface_active).text_color(theme.accent))
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .size(px(22.))
                    .rounded_md()
                    .text_size(px(12.))
                    .text_color(theme.icon)
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface_hover).text_color(theme.danger))
                    .tooltip(tooltip_label(ts!("generate.tip_remove")))
                    .on_click(move |_, _window, cx| {
                        remove.update(cx, |pane, cx| pane.remove_template(index, cx));
                    })
                    .child("\u{2715}"),
            )
    }

    /// The custom variable table.
    fn render_vars(&self) -> impl IntoElement + use<> {
        let rows: Vec<_> = self
            .vars
            .iter()
            .enumerate()
            .map(|(index, row)| {
                div()
                    .id(("generate-var", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .w_full()
                    .min_w_0()
                    .child(div().w(px(150.)).flex_none().child(row.key.clone()))
                    .child(div().flex_1().min_w_0().child(row.value.clone()))
            })
            .collect();
        div()
            .flex()
            .flex_col()
            .gap(px(4.))
            .w_full()
            .min_w_0()
            .debug_selector(|| "generate-vars".to_string())
            .children(rows)
    }

    /// The abbreviation switch, and the button that opens the rules editor.
    fn render_abbreviations(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.entity();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .w_full()
            .min_w_0()
            .child(
                Checkbox::new("generate-abbr", ts!("generate.apply_abbreviations"))
                    .checked(self.abbr.apply_to_names)
                    .on_toggle(move |checked, _window, cx| {
                        this.update(cx, |pane, cx| pane.toggle_abbreviations(checked, cx));
                    }),
            )
            .child(
                Button::new("generate-rules", ts!("generate.rules"))
                    .variant(ButtonVariant::Secondary)
                    .on_click({
                        let open = cx.entity();
                        move |_, _window, cx| {
                            open.update(cx, |_, cx| {
                                cx.emit(GeneratePaneEvent::EditAbbreviations);
                            });
                        }
                    }),
            )
    }

    /// The "Save as set…" prompt.
    ///
    /// While the name is a set's, a line under it says that set is replaced
    /// and the button reads Overwrite. Decided from the copy on screen, which
    /// is what the selector shows; the save matches against the file again.
    fn render_set_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let save = cx.entity();
        let cancel = cx.entity();
        let input = self.set_name.clone();
        let theme = theme(cx);
        let overwritten = set_named(&self.sets.sets, &text(&self.set_name, cx))
            .and_then(|id| self.sets.get(id))
            .map(|set| set.name.clone());
        let (label, variant) = if overwritten.is_some() {
            (ts!("generate.overwrite_set"), ButtonVariant::Danger)
        } else {
            (ts!("common.save"), ButtonVariant::Primary)
        };
        modal(
            "generate-set-name",
            ts!("generate.save_as_set"),
            px(360.),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(form_row(ts!("generate.set_name"), input.clone()))
                .children(overwritten.map(|name| {
                    div()
                        .debug_selector(|| "generate-set-overwrite".to_string())
                        .text_size(px(12.))
                        .text_color(theme.text_muted)
                        .child(ts!("generate.overwrite_set_note", name = name))
                }))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("generate-set-cancel", ts!("common.cancel")).on_click({
                                let cancel = cancel.clone();
                                move |_, _window, cx| {
                                    cancel.update(cx, |pane, cx| {
                                        pane.naming_set = false;
                                        cx.notify();
                                    });
                                }
                            }),
                        )
                        .child(
                            Button::new("generate-set-save", label)
                                .variant(variant)
                                .on_click(move |_, _window, cx| {
                                    let name = save.read(cx).set_name.read(cx).content().to_owned();
                                    save.update(cx, |pane, cx| pane.save_as_set(name, cx));
                                }),
                        ),
                ),
            move |_window, cx| {
                cancel.update(cx, |pane, cx| {
                    pane.naming_set = false;
                    cx.notify();
                });
            },
        )
    }

    /// The "Delete set" confirmation prompt.
    fn render_delete_set_prompt(
        &self,
        id: Uuid,
        cx: &mut Context<Self>,
    ) -> impl IntoElement + use<> {
        let name = self
            .sets
            .get(id)
            .map(|set| set.name.clone())
            .unwrap_or_default();
        let cancel = cx.entity();
        let delete = cx.entity();
        modal(
            "generate-delete-set",
            ts!("generate.delete_set"),
            px(360.),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(ts!("generate.delete_set_confirm", name = name))
                .child(
                    div()
                        .flex()
                        .flex_row()
                        .justify_end()
                        .gap(px(8.))
                        .child(
                            Button::new("generate-delete-set-cancel", ts!("common.cancel"))
                                .on_click({
                                    let cancel = cancel.clone();
                                    move |_, _window, cx| {
                                        cancel.update(cx, |pane, cx| {
                                            pane.deleting_set = None;
                                            cx.notify();
                                        });
                                    }
                                }),
                        )
                        .child(
                            Button::new("generate-delete-set-confirm", ts!("generate.delete_set"))
                                .variant(ButtonVariant::Danger)
                                .on_click(move |_, _window, cx| {
                                    delete.update(cx, |pane, cx| pane.delete_set(id, cx));
                                }),
                        ),
                ),
            move |_window, cx| {
                cancel.update(cx, |pane, cx| {
                    pane.deleting_set = None;
                    cx.notify();
                });
            },
        )
    }
}

impl EventEmitter<GeneratePaneEvent> for GeneratePane {}

impl Focusable for GeneratePane {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for GeneratePane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        self.watch_scroll(cx);

        let chooser = cx.entity();
        let output = div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(6.))
            .w_full()
            .min_w_0()
            .debug_selector(|| "generate-output-row".to_string())
            .child(div().flex_1().min_w_0().child(self.output.clone()))
            .child(
                Button::new("generate-output-pick", ts!("generate.browse"))
                    .variant(ButtonVariant::Secondary)
                    .on_click(move |_, _window, cx| {
                        chooser.update(cx, |pane, cx| pane.choose_output(cx));
                    }),
            );

        let heading = |text: SharedString| {
            div()
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(text)
        };

        let body = div()
            .flex()
            .flex_col()
            .gap(px(14.))
            .p(px(16.))
            .w_full()
            .min_w_0()
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w_full()
                    .min_w_0()
                    .child(heading(ts!("generate.template_set")))
                    .child(self.render_sets(cx))
                    .children(
                        self.sets_error
                            .clone()
                            .map(|message| error_line(message, &theme)),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .w_full()
                    .min_w_0()
                    .child(heading(ts!("generate.templates")))
                    .child(self.render_templates(&theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .w_full()
                    .min_w_0()
                    .child(heading(ts!("generate.options")))
                    .child(labeled(ts!("generate.output_dir"), output))
                    .child(labeled(ts!("generate.author"), self.author.clone()))
                    .child(labeled(ts!("generate.variables"), self.render_vars()))
                    .child(self.render_abbreviations(cx))
                    .children(
                        self.abbr_error
                            .clone()
                            .map(|message| error_line(message, &theme)),
                    ),
            );

        let bar = self
            .bar()
            .on_hover(cx.listener(|pane, hovered: &bool, _window, cx| {
                pane.hover_scrollbar(*hovered, cx);
            }));
        let prompt = self.naming_set.then(|| self.render_set_prompt(cx));
        let delete_prompt = self
            .deleting_set
            .map(|id| self.render_delete_set_prompt(id, cx));

        div()
            .key_context("GeneratePane")
            .track_focus(&self.focus_handle)
            .relative()
            .flex()
            .flex_col()
            .flex_grow_1()
            .min_w_0()
            .w_full()
            // Bounded by `cached_view`'s box, which a grow alone would not do:
            // without it the root is as tall as the form, the scroll box never
            // overflows and the form runs under the run buttons.
            .h_full()
            .min_h_0()
            .child(
                div()
                    .id(PANE_SCROLL)
                    .debug_selector(|| PANE_SCROLL.to_string())
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .flex_grow_1()
                    .min_w_0()
                    .w_full()
                    .min_h_0()
                    .overflow_y_scroll()
                    .restrict_scroll_to_axis()
                    .child(body),
            )
            .children(bar.render(&theme))
            .children(prompt)
            .children(delete_prompt)
    }
}

/// A line saying why a global store could not be changed.
fn error_line(message: SharedString, theme: &Theme) -> impl IntoElement + use<> {
    div()
        .text_size(px(11.))
        .text_color(theme.danger)
        .child(message)
}

/// What [`GeneratePane::sets_error`] says about `error`.
fn sets_failed(error: &anyhow::Error) -> SharedString {
    ts!("generate.sets_failed", error = format!("{error:#}"))
}

/// What [`GeneratePane::abbr_error`] says about `error`.
fn abbr_failed(error: &anyhow::Error) -> SharedString {
    ts!("generate.abbr_failed", error = format!("{error:#}"))
}

/// A labeled row of the options block.
///
/// [`rugpui::form_row`]'s label column is sized for a dialog; the options
/// here sit beside a template list and need a wider one, so the shape is
/// repeated rather than the widget parameterized for one caller.
fn labeled(label: SharedString, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(10.))
        .w_full()
        .min_w_0()
        .child(
            div()
                .flex_none()
                .w(px(LABEL_WIDTH))
                .pt(px(6.))
                .text_size(px(12.))
                .child(label),
        )
        .child(div().flex_grow_1().min_w_0().child(control))
}

/// Reads a text field.
fn text(input: &Entity<TextInput>, cx: &App) -> String {
    input.read(cx).content().to_owned()
}

/// Writes a text field.
fn set_text(input: &Entity<TextInput>, value: impl Into<SharedString>, cx: &mut App) {
    input.update(cx, |input, cx| input.set_content(value, cx));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn reference(name: &str, selected: bool) -> TemplateRef {
        TemplateRef {
            name: name.to_string(),
            file: PathBuf::from(format!("templates/{name}.java")),
            out_template: "${name.suffix.pascal}.java".to_string(),
            selected,
        }
    }

    #[gpui::test]
    fn tab_follows_template_rows_and_reaches_remove(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::app_settings::init(cx);
            rugpui::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let mut pane = GeneratePane::new(cx);
            pane.rebuild(
                &GenerationProfile {
                    templates: vec![reference("first", true), reference("second", true)],
                    ..Default::default()
                },
                cx,
            );
            pane
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        window
            .update(&mut cx, |pane, window, cx| {
                pane.templates[0]
                    .name
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            })
            .unwrap();
        cx.simulate_keystrokes("tab");
        window
            .update(&mut cx, |pane, window, cx| {
                assert!(
                    pane.templates[0]
                        .out
                        .read(cx)
                        .focus_handle(cx)
                        .is_focused(window)
                );
            })
            .unwrap();
        // Edit, remove: these used to be mouse-only controls.
        cx.simulate_keystrokes("tab tab");
        let stroke = gpui::Keystroke::parse("enter").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: stroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke: stroke });
        window
            .update(&mut cx, |pane, window, cx| {
                assert_eq!(pane.templates.len(), 1);
                assert_eq!(text(&pane.templates[0].name, cx), "second");
                pane.templates[0]
                    .name
                    .read(cx)
                    .focus_handle(cx)
                    .focus(window, cx);
            })
            .unwrap();
        // Removing a row must keep its checkbox before its text fields.
        cx.simulate_keystrokes("shift-tab");
        let stroke = gpui::Keystroke::parse("space").unwrap();
        cx.simulate_event(gpui::KeyDownEvent {
            keystroke: stroke.clone(),
            is_held: false,
            prefer_character_input: false,
        });
        cx.simulate_event(gpui::KeyUpEvent { keystroke: stroke });
        window
            .update(&mut cx, |pane, _, _| assert!(!pane.templates[0].selected))
            .unwrap();
    }

    #[test]
    fn the_status_bar_multiplies_tables_by_templates() {
        let ready = readiness(true, 3, 2, Some(Path::new("/tmp/out")));
        assert_eq!(ready.files, 6);
        assert_eq!(ready.blocker, None);
    }

    #[test]
    fn the_first_missing_thing_is_the_one_reported() {
        // Everything is missing at once on a fresh window, and the tooltip may
        // only name one: the one that has to be fixed first.
        assert_eq!(
            readiness(false, 0, 0, None).blocker,
            Some(Blocker::Disconnected)
        );
        assert_eq!(readiness(true, 0, 0, None).blocker, Some(Blocker::NoTables));
        assert_eq!(
            readiness(true, 2, 0, None).blocker,
            Some(Blocker::NoTemplates)
        );
        assert_eq!(
            readiness(true, 2, 1, None).blocker,
            Some(Blocker::NoOutputDir)
        );
        // An output directory that is the empty string is no output directory.
        assert_eq!(
            readiness(true, 2, 1, Some(Path::new(""))).blocker,
            Some(Blocker::NoOutputDir)
        );
        assert_eq!(readiness(true, 2, 1, Some(Path::new("/out"))).blocker, None);
        // Every reason says something, and no two say the same thing.
        let messages = [
            Blocker::Disconnected.message(),
            Blocker::NoTables.message(),
            Blocker::NoTemplates.message(),
            Blocker::NoOutputDir.message(),
        ];
        for message in &messages {
            assert!(!message.is_empty());
            assert!(!message.starts_with("generate."), "untranslated {message}");
        }
        for (index, message) in messages.iter().enumerate() {
            assert!(
                !messages[index + 1..].contains(message),
                "two reasons read the same: {message}"
            );
        }
    }

    #[test]
    fn a_list_that_is_a_set_is_named_and_anything_else_is_custom() {
        let set = TemplateSet::new("Java + MyBatis", vec![reference("a", true)]);
        let other = TemplateSet::new("PHP", vec![reference("b", true)]);
        let sets = vec![set.clone(), other.clone()];

        assert_eq!(matching_set(&sets, &set.templates), Some(set.id));
        assert_eq!(matching_set(&sets, &other.templates), Some(other.id));
        // An empty list is nobody's set, and neither is a longer one.
        assert_eq!(matching_set(&sets, &[]), None);
        assert_eq!(
            matching_set(&sets, &[reference("a", true), reference("b", true)]),
            None
        );
        // The tick is part of the list: unticking a row diverges from the set.
        assert_eq!(matching_set(&sets, &[reference("a", false)]), None);
        // So is the order.
        let two = TemplateSet::new("both", vec![reference("a", true), reference("b", true)]);
        let sets = vec![two.clone()];
        assert_eq!(
            matching_set(&sets, &[reference("b", true), reference("a", true)]),
            None
        );
        assert_eq!(matching_set(&sets, &two.templates), Some(two.id));
    }

    #[test]
    fn a_set_is_found_by_its_name_as_the_selector_shows_it() {
        let set = TemplateSet::new("Java + MyBatis", vec![reference("a", true)]);
        let twin = TemplateSet::new("Java + MyBatis", vec![reference("b", true)]);
        let sets = vec![set.clone(), twin];

        assert_eq!(set_named(&sets, "Java + MyBatis"), Some(set.id));
        // Trimmed, as a name is saved; and of two that share a name, which an
        // older file may hold, the first.
        assert_eq!(set_named(&sets, "  Java + MyBatis "), Some(set.id));
        // The case is part of the name, and a blank name is nobody's.
        assert_eq!(set_named(&sets, "java + mybatis"), None);
        assert_eq!(set_named(&sets, "   "), None);
    }

    #[test]
    fn saving_under_a_set_name_overwrites_that_set_in_place() {
        let mut store = TemplateSetStore::default();
        let first = store_set(&mut store, "first".to_string(), vec![reference("a", true)]);
        let second = store_set(&mut store, "second".to_string(), vec![reference("b", true)]);
        assert_ne!(first, second);

        let edited = vec![reference("a", false), reference("c", true)];
        let again = store_set(&mut store, "first".to_string(), edited.clone());
        // The same set, where it was, now holding the edited list.
        assert_eq!(again, first);
        let names: Vec<&str> = store.sets.iter().map(|set| set.name.as_str()).collect();
        assert_eq!(names, ["first", "second"]);
        assert_eq!(store.get(first).map(|set| &set.templates), Some(&edited));

        // Any other name is a new set at the end.
        let third = store_set(&mut store, "third".to_string(), vec![reference("d", true)]);
        assert_eq!(store.sets.len(), 3);
        assert_eq!(store.sets[2].id, third);
    }

    #[gpui::test]
    fn the_prompt_opens_on_the_set_the_edited_list_came_from(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::app_settings::init(cx);
            rugpui::init(cx);
        });
        let set = TemplateSet::new(
            "Java + MyBatis",
            vec![reference("a", true), reference("b", true)],
        );
        let window = cx.add_window(|_, cx| {
            let mut pane = GeneratePane::new(cx);
            pane.sets.upsert(set.clone());
            pane
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        window
            .update(&mut cx, |pane, _, cx| {
                pane.apply_set(set.id, cx);
                // An unticked row: the selector says Custom from here on.
                pane.toggle_template(1, false, cx);
                let current = pane.collect(cx).templates;
                assert_eq!(matching_set(&pane.sets.sets, &current), None);

                pane.open_set_prompt(cx);
                assert_eq!(text(&pane.set_name, cx), "Java + MyBatis");
            })
            .unwrap();
        cx.run_until_parked();
        assert!(
            cx.debug_bounds("generate-set-overwrite").is_some(),
            "the prompt does not say the set is replaced"
        );

        // A name of its own is a new set, and the note goes as it is typed.
        window
            .update(&mut cx, |pane, _, cx| {
                set_text(&pane.set_name, "Java only", cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert!(cx.debug_bounds("generate-set-overwrite").is_none());

        // With no set to come from, the prompt opens empty.
        window
            .update(&mut cx, |pane, _, cx| {
                pane.naming_set = false;
                pane.reset(cx);
                pane.open_set_prompt(cx);
                assert_eq!(text(&pane.set_name, cx), "");
            })
            .unwrap();
    }

    #[test]
    fn a_blank_key_is_not_a_variable() {
        let rows = [
            ("package".to_string(), "com.abc".to_string()),
            ("  spaced  ".to_string(), "kept ".to_string()),
            (String::new(), "orphan".to_string()),
            ("   ".to_string(), String::new()),
        ];
        assert_eq!(
            collect_vars(&rows),
            vec![
                ("package".to_string(), "com.abc".to_string()),
                // The key is trimmed; the value is not, because a trailing
                // space in a value can be deliberate.
                ("spaced".to_string(), "kept ".to_string()),
            ]
        );
        // The trailing blank row of a fresh table saves nothing at all.
        assert!(collect_vars(&[(String::new(), String::new())]).is_empty());
    }

    #[test]
    fn the_variable_table_always_has_one_row_to_type_into() {
        assert!(needs_trailing_blank(&[]));
        assert!(needs_trailing_blank(&[(
            "package".to_string(),
            "com.abc".to_string()
        )]));
        // A row with a value but no key still counts as filled in: the user is
        // mid-edit, and the table has to offer the next row already.
        assert!(needs_trailing_blank(&[(String::new(), "x".to_string())]));
        assert!(!needs_trailing_blank(&[(String::new(), String::new())]));
        assert!(!needs_trailing_blank(&[
            ("package".to_string(), "com.abc".to_string()),
            ("   ".to_string(), String::new()),
        ]));
    }

    #[test]
    fn an_absolute_template_path_is_left_alone() {
        // A drive letter on Windows, where `/opt/templates/mine.java` is a
        // *relative* path and would be resolved against the config directory.
        let absolute = if cfg!(windows) {
            PathBuf::from(r"C:\opt\templates\mine.java")
        } else {
            PathBuf::from("/opt/templates/mine.java")
        };
        assert_eq!(resolve_template(&absolute), absolute);
        assert_eq!(store_template_path(&absolute), absolute);
    }

    #[test]
    fn a_relative_template_path_is_read_back_under_the_config_directory() {
        // The pair has to round-trip, or a template saved by one run is not
        // found by the next.
        let Ok(root) = config_dir() else {
            return;
        };
        let below = root.join("templates").join("java_model.java");
        let stored = store_template_path(&below);
        assert_eq!(stored, PathBuf::from("templates/java_model.java"));
        assert_eq!(resolve_template(&stored), below);
    }

    #[test]
    fn a_template_set_file_that_cannot_be_read_is_not_saved_over() {
        // Saving the empty store the panel shows in its place would replace
        // every set the user has with the one being saved.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("template-sets.json");
        let damaged = b"{ \"sets\": [ not json";
        std::fs::write(&path, damaged).unwrap();

        let written = read_modify_write(
            || TemplateSetStore::load_from(&path),
            |_| panic!("a change was applied to a store that could not be read"),
            |sets| sets.save_to(&path),
        );

        assert!(written.is_err());
        assert_eq!(std::fs::read(&path).unwrap(), damaged);
    }

    #[test]
    fn a_change_starts_from_the_file_rather_than_the_copy_read_at_connect() {
        // The panel read the sets when the connection opened; the import
        // wizard has written one since. Saving a set of the panel's own must
        // keep the wizard's.
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("template-sets.json");
        let stale = TemplateSetStore::default();
        let mut meanwhile = stale.clone();
        meanwhile.upsert(TemplateSet::new("imported", vec![reference("a", true)]));
        meanwhile.save_to(&path).unwrap();

        let written = read_modify_write(
            || TemplateSetStore::load_from(&path),
            |sets| sets.upsert(TemplateSet::new("mine", vec![reference("b", true)])),
            |sets| sets.save_to(&path),
        )
        .expect("the file reads and writes");

        let names: Vec<&str> = written.sets.iter().map(|set| set.name.as_str()).collect();
        assert_eq!(names, ["imported", "mine"]);
        assert_eq!(TemplateSetStore::load_from(&path).unwrap(), written);
    }

    #[test]
    fn the_error_lines_are_translated() {
        let error = anyhow::anyhow!("failed to parse");
        for label in [sets_failed(&error), abbr_failed(&error)] {
            assert!(!label.starts_with("generate."), "untranslated {label:?}");
            assert!(label.contains("failed to parse"), "{label:?}");
        }
    }

    /// The pane as the workspace draws it: through `cached_view`, above the
    /// band the run buttons sit in.
    struct CachedHarness {
        pane: Entity<GeneratePane>,
    }

    impl Render for CachedHarness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .child(
                    div()
                        .flex()
                        .flex_1()
                        .min_h_0()
                        .child(rugpui::cached_view(self.pane.clone())),
                )
                .child(div().flex_none().h(px(60.)))
        }
    }

    #[gpui::test]
    fn a_short_window_scrolls_the_form_above_the_run_buttons(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::app_settings::init(cx);
            rugpui::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let pane = cx.new(|cx| {
                let mut pane = GeneratePane::new(cx);
                pane.rebuild(
                    &GenerationProfile {
                        templates: vec![reference("first", true), reference("second", true)],
                        custom_vars: vec![("package".to_string(), "com.example".to_string())],
                        output_dir: Some(PathBuf::from("/tmp/out")),
                        author: "author".to_string(),
                    },
                    cx,
                );
                pane
            });
            CachedHarness { pane }
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui::size(px(800.), px(200.)));
        cx.run_until_parked();
        let scroll = cx
            .debug_bounds(PANE_SCROLL)
            .expect("the scroll box is drawn");
        // The scroll box has to stop where the button band starts; a root that
        // only grows lets it run as tall as the form, under the buttons.
        assert!(
            (scroll.size.height - px(140.)).abs() < px(2.),
            "the form was not bounded above the buttons: {scroll:?}"
        );
    }

    #[gpui::test]
    fn the_output_name_field_grows_with_the_window(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::app_settings::init(cx);
            rugpui::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let mut pane = GeneratePane::new(cx);
            pane.rebuild(
                &GenerationProfile {
                    templates: vec![reference("first", true)],
                    ..Default::default()
                },
                cx,
            );
            pane
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        let mut widths = Vec::new();
        for width in [600., 1000., 1400.] {
            cx.simulate_resize(gpui::size(px(width), px(700.)));
            cx.run_until_parked();
            let field = cx
                .debug_bounds("generate-out-0")
                .expect("the output name field is drawn");
            widths.push(field.size.width);
        }
        // Never narrower than the fixed width it used to have, and two thirds
        // of what the window gains on top of that.
        assert!(widths[0] >= px(OUT_MIN_WIDTH), "{widths:?}");
        let gained = widths[2] - widths[1];
        assert!(
            (gained - px(400. * 2. / 3.)).abs() < px(2.),
            "the field did not grow with the window: {widths:?}"
        );
    }

    #[gpui::test]
    fn narrowing_the_window_narrows_the_generate_controls(cx: &mut gpui::TestAppContext) {
        cx.update(|cx| {
            crate::app_settings::init(cx);
            rugpui::init(cx);
        });
        let window = cx.add_window(|_, cx| {
            let mut pane = GeneratePane::new(cx);
            pane.rebuild(
                &GenerationProfile {
                    templates: vec![reference("first", true), reference("second", true)],
                    custom_vars: vec![("package".to_string(), "com.example".to_string())],
                    output_dir: Some(PathBuf::from("/tmp/out")),
                    author: "author".to_string(),
                },
                cx,
            );
            pane
        });
        let mut cx = gpui::VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui::size(px(1000.), px(700.)));
        cx.run_until_parked();
        let wide_output = cx
            .debug_bounds("generate-output-row")
            .expect("the output row is drawn")
            .size
            .width;
        let wide_templates = cx
            .debug_bounds("generate-templates")
            .expect("the template list is drawn")
            .size
            .width;
        let wide_vars = cx
            .debug_bounds("generate-vars")
            .expect("the variable table is drawn")
            .size
            .width;
        cx.simulate_resize(gpui::size(px(700.), px(700.)));
        cx.run_until_parked();
        let narrow_output = cx
            .debug_bounds("generate-output-row")
            .expect("the output row is drawn")
            .size
            .width;
        let narrow_templates = cx
            .debug_bounds("generate-templates")
            .expect("the template list is drawn")
            .size
            .width;
        let narrow_vars = cx
            .debug_bounds("generate-vars")
            .expect("the variable table is drawn")
            .size
            .width;
        // The window lost 300px; every stretchable row has to lose the same.
        // A chain that refuses to shrink keeps the wide width here, which is
        // the regression this guards against.
        for (name, wide, narrow) in [
            ("output", wide_output, narrow_output),
            ("templates", wide_templates, narrow_templates),
            ("vars", wide_vars, narrow_vars),
        ] {
            assert!(
                narrow < wide,
                "{name} did not shrink: wide {wide:?}, narrow {narrow:?}"
            );
            let delta = wide - narrow;
            assert!(
                (delta - px(300.)).abs() < px(2.),
                "{name} did not track the window: wide {wide:?}, narrow {narrow:?}, delta {delta:?}"
            );
        }
    }
}
