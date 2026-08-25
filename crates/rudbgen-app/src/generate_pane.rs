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

/// Element id of the panel's scrolling box.
const PANE_SCROLL: &str = "generate-scroll";

/// Element id the panel's overlay scroll indicator is drawn under.
const PANE_SCROLLBAR: &str = "generate-scrollbar";

/// Tab-ring position of the first control in the panel.
const FIRST_TAB: isize = 100;

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
    sets: TemplateSetStore,
    /// The abbreviation rules — one global store, not part of the profile.
    abbr: AbbreviationStore,
    /// Whether the set dropdown is showing.
    set_open: bool,
    /// Whether the "Save as set…" prompt is up.
    naming_set: bool,
    /// The name being typed into that prompt.
    set_name: Entity<TextInput>,
    /// The set the delete-confirmation prompt is up for, if any.
    deleting_set: Option<Uuid>,
    /// Vertical scroll of the panel.
    scroll: ScrollHandle,
    /// Whether the panel's overlay scroll indicator is on screen.
    scrollbar: ScrollbarState,
    /// The debounce behind the save; dropped — and so cancelled — by the next
    /// edit.
    _debounce: Option<Task<()>>,
    /// Keeps the two fixed fields' observers alive.
    _subs: Vec<Subscription>,
}

impl GeneratePane {
    /// An empty panel, pointed at no connection.
    pub fn new(cx: &mut Context<Self>) -> Self {
        let output = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("generate.output_placeholder"))
                .tab_index(FIRST_TAB)
        });
        let author = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("generate.author_placeholder"))
                .tab_index(FIRST_TAB + 1)
        });
        let set_name = cx.new(|cx| TextInput::new(cx).placeholder(ts!("generate.set_name")));
        let subs = vec![
            cx.observe(&output, |pane, _, cx| pane.touch(cx)),
            cx.observe(&author, |pane, _, cx| pane.touch(cx)),
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
            abbr: AbbreviationStore::default(),
            set_open: false,
            naming_set: false,
            set_name,
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
    pub fn load(&mut self, id: Uuid, profile: &GenerationProfile, cx: &mut Context<Self>) {
        self.profile_id = Some(id);
        self.saved = profile.clone();
        self.sets = match TemplateSetStore::load() {
            Ok(sets) => sets,
            Err(error) => {
                log::error!("could not read template-sets.json: {error:#}");
                TemplateSetStore::default()
            }
        };
        self.abbr = match AbbreviationStore::load() {
            Ok(rules) => rules,
            Err(error) => {
                log::error!("could not read abbreviations.json: {error:#}");
                AbbreviationStore::default()
            }
        };
        self.rebuild(profile, cx);
    }

    /// Empties the panel; the connection it was editing is gone.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.profile_id = None;
        self.saved = GenerationProfile::default();
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

    /// Takes on the rules the editor has just written.
    ///
    /// The dialog saved the file; this replaces the panel's copy with the same
    /// value, so the switch on screen and the rules a run applies come from one
    /// place. Deliberately does **not** save: writing the file twice would be
    /// the second answer to a question already answered.
    pub fn adopt_abbreviations(&mut self, store: AbbreviationStore, cx: &mut Context<Self>) {
        self.abbr = store;
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
        for (index, template) in profile.templates.iter().enumerate() {
            let row = self.new_template_row(index, template, cx);
            self.templates.push(row);
        }

        self.vars.clear();
        for (index, (key, value)) in profile.custom_vars.iter().enumerate() {
            let row = self.new_var_row(index, cx);
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
    fn new_template_row(
        &self,
        index: usize,
        template: &TemplateRef,
        cx: &mut Context<Self>,
    ) -> TemplateRow {
        let base = FIRST_TAB + 10 + (index as isize) * 2;
        let name = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("generate.template_name"))
                .tab_index(base)
        });
        let out = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("generate.out_template"))
                .tab_index(base + 1)
        });
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
    fn new_var_row(&self, index: usize, cx: &mut Context<Self>) -> VarRow {
        let base = FIRST_TAB + 200 + (index as isize) * 2;
        let key = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("generate.var_key"))
                .tab_index(base)
        });
        let value = cx.new(|cx| {
            TextInput::new(cx)
                .placeholder(ts!("generate.var_value"))
                .tab_index(base + 1)
        });
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
            let row = self.new_var_row(self.vars.len(), cx);
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
        for (index, template) in set.templates.iter().enumerate() {
            let row = self.new_template_row(index, template, cx);
            self.templates.push(row);
        }
        self.touch(cx);
    }

    /// Saves the current list as a new set under `name`.
    fn save_as_set(&mut self, name: String, cx: &mut Context<Self>) {
        let name = name.trim().to_string();
        if name.is_empty() {
            return;
        }
        let templates = self.collect(cx).templates;
        self.sets.upsert(TemplateSet::new(name, templates));
        // The flag goes with it: a store with a set of the user's own in it is
        // no longer a first run, whatever it was before.
        self.sets.builtins_seeded = true;
        if let Err(error) = self.sets.save() {
            log::error!("could not write template-sets.json: {error:#}");
        }
        self.naming_set = false;
        cx.notify();
    }

    /// Removes a saved set. The template list on screen is untouched — a
    /// deleted set is one fewer thing to pick, not an instruction to clear
    /// what is already ticked.
    fn delete_set(&mut self, id: Uuid, cx: &mut Context<Self>) {
        self.sets.remove(id);
        if let Err(error) = self.sets.save() {
            log::error!("could not write template-sets.json: {error:#}");
        }
        self.deleting_set = None;
        cx.notify();
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
            let row = self.new_template_row(self.templates.len(), &template, cx);
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
    /// not the connection's.
    fn toggle_abbreviations(&mut self, on: bool, cx: &mut Context<Self>) {
        self.abbr.apply_to_names = on;
        if let Err(error) = self.abbr.save() {
            log::error!("could not write abbreviations.json: {error:#}");
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
            .child(
                Select::new("generate-set")
                    .options(options)
                    .selected(Some(selected))
                    .open(self.set_open)
                    .width(px(SET_WIDTH))
                    .tab_index(FIRST_TAB - 3)
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
                    .tab_index(FIRST_TAB - 2)
                    .on_click(move |_, window, cx| {
                        naming.update(cx, |pane, cx| {
                            pane.naming_set = true;
                            pane.set_name.update(cx, |input, cx| input.clear(cx));
                            cx.notify();
                        });
                        let handle = naming.read(cx).set_name.read(cx).focus_handle(cx);
                        window.focus(&handle, cx);
                    }),
            )
            .child(
                Button::new("generate-delete-set", ts!("generate.delete_set"))
                    .variant(ButtonVariant::Secondary)
                    .disabled(matched.is_none())
                    .tab_index(FIRST_TAB - 1)
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
            .child(div().w(px(200.)).flex_none().child(row.out.clone()))
            .child(
                // The way into the template tab (§4.5). The path the event
                // carries is the profile's, which the shell resolves against
                // the configuration directory before opening it.
                div()
                    .id(("generate-edit", index))
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
                    .child(div().w(px(150.)).flex_none().child(row.key.clone()))
                    .child(div().flex_1().min_w_0().child(row.value.clone()))
            })
            .collect();
        div().flex().flex_col().gap(px(4.)).children(rows)
    }

    /// The abbreviation switch, and the button that opens the rules editor.
    fn render_abbreviations(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let this = cx.entity();
        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
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
    fn render_set_prompt(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let save = cx.entity();
        let cancel = cx.entity();
        let input = self.set_name.clone();
        modal(
            "generate-set-name",
            ts!("generate.save_as_set"),
            px(360.),
            div()
                .flex()
                .flex_col()
                .gap(px(12.))
                .child(form_row(ts!("generate.set_name"), input.clone()))
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
                            Button::new("generate-set-save", ts!("common.save"))
                                .variant(ButtonVariant::Primary)
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
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(heading(ts!("generate.template_set")))
                    .child(self.render_sets(cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(6.))
                    .child(heading(ts!("generate.templates")))
                    .child(self.render_templates(&theme, cx)),
            )
            .child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .child(heading(ts!("generate.options")))
                    .child(labelled(ts!("generate.output_dir"), output))
                    .child(labelled(ts!("generate.author"), self.author.clone()))
                    .child(labelled(ts!("generate.variables"), self.render_vars()))
                    .child(self.render_abbreviations(cx)),
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
            .min_h_0()
            .child(
                div()
                    .id(PANE_SCROLL)
                    .track_scroll(&self.scroll)
                    .flex()
                    .flex_col()
                    .flex_grow_1()
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

/// A labelled row of the options block.
///
/// [`rugpui::form_row`]'s label column is sized for a dialog; the options
/// here sit beside a template list and need a wider one, so the shape is
/// repeated rather than the widget parameterised for one caller.
fn labelled(label: SharedString, control: impl IntoElement) -> impl IntoElement {
    div()
        .flex()
        .flex_row()
        .items_start()
        .gap(px(10.))
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
}
