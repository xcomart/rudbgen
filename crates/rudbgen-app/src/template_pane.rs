//! The template tab: the editor, the live preview beside it, and the
//! diagnostics under both (architecture document, D12 and §4.5).
//!
//! One of these per open file. It owns the buffer, the file's line ending, the
//! debounce that keeps the parser and the preview off the keystroke path, the
//! list of diagnostics and the completion popup. What it does **not** own is
//! anything that needs a database: the preview is rendered by the shell, which
//! has the session, the ticked tables and the generation profile, and handed
//! back through [`TemplatePane::deliver_preview`].
//!
//! # The two verdicts
//!
//! The highlighter in [`crate::template_syntax`] paints line by line and never
//! fails, so it says nothing about a template as a whole. This is where the whole-document
//! verdict lives, and there are two of them:
//!
//! * **the parse**, which is local — [`rudbgen_template::Template::parse`] over
//!   the buffer, no model needed — and which is a single error with a line;
//! * **the render**, which needs a table, comes back from the shell, and is a
//!   list of unknown fields with a span each.
//!
//! Both end up in the same list under the editor and as the same gutter marks,
//! because to the person reading them they are the same kind of thing: a place
//! in the file that is wrong. A parse error suppresses the render — a template
//! that does not parse cannot be rendered — which is why the two never appear
//! together.
//!
//! # Line endings
//!
//! The buffer is `\n` only: every offset in the editor, in the parser and in a
//! diagnostic is an offset into that. A file that arrived with CRLF is
//! remembered as CRLF and written back as CRLF, and the preview is rendered
//! from the restored text rather than from the buffer — the engine takes a
//! template's line ending from its first newline, so a preview of the buffer
//! would show LF output for a file that generates CRLF.

use std::path::{Path, PathBuf};
use std::time::Duration;

use gpui::{
    Action, Anchor, AnchoredPositionMode, App, Axis, Bounds, Context, Entity, EventEmitter,
    FocusHandle, Focusable, MouseButton, Pixels, Point, SharedString, Subscription, Task, Window,
    anchored, deferred, div, point, prelude::*, px,
};
use rudbgen_template::{ParseError, Template, Warning};
use rugpui::{
    Button, ButtonVariant, ContextMenu, Select, Splitter, editor_theme, theme, tooltip_label,
};
use rugpui_editor::editor::{CloseFind, Copy, Cut, Find, Paste, Redo, Replace, SelectAll, Undo};
use rugpui_editor::{EditorEvent, EditorScrollEvent, EditorView, MarkKind, NavKey};
use rugpui_shell::menu_rows::{self, MenuRow, SHORTCUT_MODIFIER};

use crate::app_settings;
use crate::i18n::ts;
use crate::icons;
use crate::palette::{self, CompletionKind, CompletionRequest, PaletteItem};

/// How long the buffer has to stand still before it is parsed and rendered.
///
/// Long enough that a run of typing costs one parse rather than one per
/// keystroke, short enough that it feels like the file answering rather than a
/// job that was started.
const DEBOUNCE: Duration = Duration::from_millis(300);

/// Narrowest either half of the split may be dragged, as a fraction.
///
/// [`Splitter`] reads it as both ends of the range: the trailing half is held
/// to the same minimum, so the divider stops at `1 - MIN_SPLIT` on the way
/// right without a second constant.
const MIN_SPLIT: f32 = 0.15;

/// Where the split starts.
const DEFAULT_SPLIT: f32 = 0.55;

/// Height of the diagnostics list when there is anything in it.
const DIAGNOSTICS_HEIGHT: f32 = 120.;

/// Widest the completion popup may grow.
const POPUP_WIDTH: f32 = 320.;

/// How many entries the popup shows at once.
const POPUP_ROWS: usize = 9;

/// Height of one popup row.
const POPUP_ROW_HEIGHT: f32 = 20.;

/// Width of the preview header's table selector.
const CHOICE_WIDTH: f32 = 190.;

/// The widest tab stop the header's tab width menu offers. The editor takes up
/// to sixteen, which a hand-edited `settings.json` can still ask for.
const MENU_TAB_WIDTHS: usize = 8;

/// What a file is known by in `settings.json`'s per-template settings.
fn settings_key(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// What a file's lines end with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineEnding {
    /// One byte, which is what the buffer always holds.
    Lf,
    /// Two bytes, which is what the shipped templates are written with.
    CrLf,
}

impl LineEnding {
    /// The ending `text` uses, decided by its first line break.
    ///
    /// A file with no line break at all is `\n`: nothing has been decided yet,
    /// and the platform's own ending would make a template written on Windows
    /// change the moment it is opened on Linux.
    pub fn of(text: &str) -> Self {
        match text.find('\n') {
            Some(0) => LineEnding::Lf,
            Some(at) if text.as_bytes()[at - 1] == b'\r' => LineEnding::CrLf,
            _ => LineEnding::Lf,
        }
    }

    /// `text` with every line break written this way.
    ///
    /// The input is the buffer, which is `\n` only, so this only ever adds.
    pub fn restore(self, text: &str) -> String {
        match self {
            LineEnding::Lf => text.to_owned(),
            LineEnding::CrLf => text.replace('\n', "\r\n"),
        }
    }
}

/// `text` with every `\r\n` reduced to `\n`.
///
/// A lone `\r` is left alone: it is not a line break in any file this opens,
/// and turning one into a break would change the document.
pub fn to_lf(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// One thing wrong with the template, as the list under the editor shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    /// Zero-based line, as the engine counts them.
    pub line: usize,
    /// Zero-based column, in characters.
    pub column: usize,
    /// Byte offset the caret moves to when the row is clicked.
    pub offset: usize,
    /// What is wrong.
    pub message: SharedString,
    /// Whether the template fails to parse, as opposed to merely rendering to
    /// something the model could not answer.
    pub error: bool,
}

/// Where `offset` is in `source`, as a zero-based line and character column.
pub fn point_of(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let line = source[..offset].matches('\n').count();
    let start = source[..offset].rfind('\n').map_or(0, |at| at + 1);
    (line, source[start..offset].chars().count())
}

/// The byte offset of `column` characters into `line` of `source`.
///
/// The inverse of [`point_of`], and what turns a diagnostic computed against
/// the *rendered* source — which carries the file's own line ending — into an
/// offset into the buffer, which is `\n` only. The two agree line for line and
/// column for column and differ by one byte per line break, so going back
/// through the point is the whole of the conversion.
fn offset_of(source: &str, line: usize, column: usize) -> usize {
    let start = line_start(source, line);
    let rest = &source[start..];
    let end = rest.find('\n').unwrap_or(rest.len());
    start
        + rest[..end]
            .char_indices()
            .nth(column)
            .map_or(end, |(at, _)| at)
}

/// The byte offset of the start of `line` in `source`.
fn line_start(source: &str, line: usize) -> usize {
    let mut at = 0;
    for _ in 0..line {
        match source[at..].find('\n') {
            Some(next) => at += next + 1,
            None => return source.len(),
        }
    }
    at
}

/// The diagnostic a failed parse is.
///
/// The engine reports a line always and a span sometimes, so the column comes
/// from the span where there is one and is the start of the line where there is
/// not.
pub fn parse_diagnostic(source: &str, error: &ParseError) -> Diagnostic {
    let offset = match error.span {
        Some(span) => span.start.min(source.len()),
        None => line_start(source, error.line),
    };
    // The span is the better answer where there is one: it points at the
    // statement rather than at the line the parser had reached. Without one
    // the engine's own line is all there is, and the column is the start of
    // it.
    let (line, column) = match error.span {
        Some(_) => point_of(source, offset),
        None => (error.line, 0),
    };
    Diagnostic {
        line,
        column,
        offset,
        message: SharedString::from(error.message.clone()),
        error: true,
    }
}

/// The diagnostics a render's warnings are.
pub fn warning_diagnostics(source: &str, warnings: &[Warning]) -> Vec<Diagnostic> {
    warnings
        .iter()
        .map(|warning| {
            let offset = warning.span.start.min(source.len());
            let (line, column) = point_of(source, offset);
            Diagnostic {
                line,
                column,
                offset,
                message: SharedString::from(warning.message.clone()),
                error: false,
            }
        })
        .collect()
}

/// The gutter marks a list of diagnostics comes to.
pub fn marks_of(diagnostics: &[Diagnostic]) -> Vec<(usize, MarkKind)> {
    diagnostics
        .iter()
        .map(|diagnostic| {
            (
                diagnostic.line,
                if diagnostic.error {
                    MarkKind::Error
                } else {
                    MarkKind::Warning
                },
            )
        })
        .collect()
}

/// What a rendered preview came back as.
pub enum PreviewOutcome {
    /// It rendered: the file it would write, its text, and what the render
    /// could not answer.
    Rendered {
        /// Where a real run would put it.
        path: PathBuf,
        /// The rendered text.
        content: String,
        /// The unknown fields the render noticed.
        warnings: Vec<Warning>,
    },
    /// It could not be rendered, and this is why — a message already
    /// translated, because the shell owns the strings.
    Refused(SharedString),
}

/// What the tab asks the shell for.
pub enum TemplatePaneEvent {
    /// The dirty marker changed, so the tab strip has to be redrawn.
    DirtyChanged,
    /// Render this text against the table at `table`.
    Render {
        /// The buffer, with the file's own line ending restored.
        source: String,
        /// Index into the choices the shell last set.
        table: usize,
    },
}

/// The completion popup, while it is up.
struct Completion {
    /// What the caret's context was when it was opened.
    request: CompletionRequest,
    /// The entries on offer, best first.
    matches: Vec<PaletteItem>,
    /// Which of them is highlighted.
    selected: usize,
    /// Where the caret was, for the popup to hang off.
    anchor: Option<Bounds<Pixels>>,
}

/// One open template file.
pub struct TemplatePane {
    focus_handle: FocusHandle,
    /// The file, resolved: this is what a save writes to.
    path: PathBuf,
    /// What the tab is labelled.
    title: SharedString,
    /// The buffer.
    editor: Entity<EditorView>,
    /// The read-only buffer the preview is shown in.
    preview: Entity<EditorView>,
    /// What the file's lines end with, remembered from the load.
    line_ending: LineEnding,
    /// Why the file could not be opened, when it could not.
    failure: Option<SharedString>,
    /// What the last save said, when it went wrong.
    save_failure: Option<SharedString>,
    /// Whether the preview half is on screen.
    preview_open: bool,
    /// Where the split sits, as a fraction of the row.
    split: f32,
    /// The tables the preview may be rendered against.
    choices: Vec<SharedString>,
    /// Which of them it is being rendered against.
    choice: usize,
    /// Whether the table selector is showing.
    choice_open: bool,
    /// What the preview would be written to.
    preview_path: Option<SharedString>,
    /// Why there is no preview, when there is none.
    preview_note: Option<SharedString>,
    /// Everything wrong with the template, parse errors first.
    diagnostics: Vec<Diagnostic>,
    /// The palette, for the completion popup to filter.
    items: Vec<PaletteItem>,
    /// The popup, while it is up.
    completion: Option<Completion>,
    /// Pointer position of the source editor's context menu.
    context_menu: Option<Point<Pixels>>,
    /// Where the header's tab width menu hangs from, while it is up.
    tab_menu: Option<Point<Pixels>>,
    /// Whether the buffer should take the keyboard on the next frame.
    ///
    /// A tab that has just been opened or brought to the front is a document
    /// the user means to type into, and the shell cannot focus the buffer
    /// itself: the focus handle belongs to an element that has not been drawn
    /// yet. The flag is consumed by the first render after it is set.
    pending_focus: bool,
    /// The debounce in flight, dropped — and so cancelled — by the next edit.
    _debounce: Option<Task<()>>,
    /// Keeps the editor's subscription alive.
    _editor_events: Subscription,
    /// The source and preview follow each other's wheel and scrollbar moves.
    _scroll_events: [Subscription; 2],
}

impl TemplatePane {
    /// Opens `path`, reading it now.
    ///
    /// A file that cannot be read is not an empty editor: the pane says why,
    /// and offers nothing to type into, because a save would then overwrite
    /// whatever is there with nothing.
    pub fn open(path: PathBuf, cx: &mut Context<Self>) -> Self {
        let highlighter = crate::template_syntax::template_highlighter_for_path(&path);
        let wrap = app_settings::effective(cx).editor_word_wrap;
        // The preview draws the tabs the template copies into it as wide as the
        // template's own, so the two halves line up.
        let tabs = app_settings::current(cx).template_tab_width(&settings_key(&path)) as usize;
        let editor = cx.new(|cx| {
            let mut editor = EditorView::new(cx).highlighter(highlighter).word_wrap(wrap);
            editor.set_tab_width(tabs, cx);
            editor
        });
        let preview = cx.new(|cx| {
            let mut preview = EditorView::new(cx).read_only(true).word_wrap(wrap);
            preview.set_tab_width(tabs, cx);
            preview
        });
        let editor_events = cx.subscribe(&editor, |pane, _editor, event, cx| match event {
            EditorEvent::Changed => pane.changed(cx),
            EditorEvent::SelectionChanged => pane.moved(cx),
            EditorEvent::Intercepted(key) => pane.intercepted(*key, cx),
            EditorEvent::ContextMenu { position } => {
                pane.close_completion(cx);
                pane.choice_open = false;
                pane.tab_menu = None;
                pane.context_menu = Some(*position);
                cx.notify();
            }
            _ => {}
        });
        let scroll_events = [
            cx.subscribe(&editor, |pane, _, event: &EditorScrollEvent, cx| {
                if pane.preview_open && pane.preview_note.is_none() {
                    pane.preview.update(cx, |preview, cx| {
                        preview.set_scroll_progress(event.axis, event.progress, cx);
                    });
                }
            }),
            cx.subscribe(&preview, |pane, _, event: &EditorScrollEvent, cx| {
                if pane.preview_open && pane.preview_note.is_none() {
                    pane.editor.update(cx, |editor, cx| {
                        editor.set_scroll_progress(event.axis, event.progress, cx);
                    });
                }
            }),
        ];

        let title = path
            .file_name()
            .map(|name| SharedString::from(name.to_string_lossy().into_owned()))
            .unwrap_or_else(|| SharedString::from(path.display().to_string()));

        let mut pane = Self {
            focus_handle: cx.focus_handle(),
            path,
            title,
            editor,
            preview,
            line_ending: LineEnding::Lf,
            failure: None,
            save_failure: None,
            preview_open: true,
            split: DEFAULT_SPLIT,
            choices: Vec::new(),
            choice: 0,
            choice_open: false,
            preview_path: None,
            preview_note: None,
            diagnostics: Vec::new(),
            items: Vec::new(),
            completion: None,
            context_menu: None,
            tab_menu: None,
            pending_focus: true,
            _debounce: None,
            _editor_events: editor_events,
            _scroll_events: scroll_events,
        };
        pane.load(cx);
        pane
    }

    /// Reads the file into the buffer.
    fn load(&mut self, cx: &mut Context<Self>) {
        let bytes = match std::fs::read(&self.path) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.failure = Some(ts!(
                    "template.read_failed",
                    file = self.path.display().to_string(),
                    reason = error.to_string()
                ));
                return;
            }
        };
        let text = match String::from_utf8(bytes) {
            Ok(text) => text,
            Err(_) => {
                self.failure = Some(ts!(
                    "template.not_utf8",
                    file = self.path.display().to_string()
                ));
                return;
            }
        };
        self.line_ending = LineEnding::of(&text);
        self.failure = None;
        let lf = to_lf(&text);
        self.editor
            .update(cx, |editor, cx| editor.set_text(&lf, cx));
        // The preview highlighter follows the *output* file, which is only
        // known once the shell has rendered one; until then the template's own
        // extension is the better guess than none.
        let highlighter = crate::template_syntax::template_highlighter_for_path(&self.path);
        self.preview.update(cx, |editor, cx| {
            editor.set_highlighter(Some(highlighter), cx)
        });
    }

    /// Puts `message` where the buffer would be, as a file that could not be
    /// read does: the tab says why and offers nothing to type into.
    pub fn show_failure(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.failure = Some(message);
        cx.notify();
    }

    /// The file this tab edits.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// What the tab strip calls it.
    pub fn title(&self) -> SharedString {
        self.title.clone()
    }

    /// Whether there is anything to save.
    pub fn is_dirty(&self, cx: &App) -> bool {
        self.failure.is_none() && self.editor.read(cx).is_dirty()
    }

    /// The buffer, as the file would be written.
    pub fn source(&self, cx: &App) -> String {
        self.line_ending.restore(&self.editor.read(cx).text())
    }

    /// The diagnostics as they stand, for the tests.
    #[cfg(test)]
    pub fn diagnostics(&self) -> &[Diagnostic] {
        &self.diagnostics
    }

    /// Why the tab has no buffer to offer, for the tests.
    #[cfg(test)]
    pub fn failure(&self) -> Option<&SharedString> {
        self.failure.as_ref()
    }

    /// Writes the buffer back to the file, atomically.
    ///
    /// The line ending the file arrived with is restored first: a template that
    /// was CRLF stays CRLF, because the engine reads a template's line ending
    /// off its first newline and a file that changed its mind would change
    /// every generated file with it.
    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        if self.failure.is_some() {
            return false;
        }
        let text = self.source(cx);
        match rudbgen_core::paths::write_atomic(&self.path, text.as_bytes()) {
            Ok(()) => {
                self.save_failure = None;
                self.editor.update(cx, |editor, cx| editor.mark_clean(cx));
                cx.emit(TemplatePaneEvent::DirtyChanged);
                cx.notify();
                true
            }
            Err(error) => {
                log::error!("could not write {}: {error:#}", self.path.display());
                self.save_failure = Some(ts!(
                    "template.write_failed",
                    file = self.path.display().to_string(),
                    reason = format!("{error:#}")
                ));
                cx.notify();
                false
            }
        }
    }

    /// Asks for the keyboard on the next frame.
    ///
    /// What the shell calls when this tab comes to the front: by then the
    /// element the focus handle belongs to has stopped being drawn, so the
    /// focus has to be taken again once it is back.
    pub fn request_focus(&mut self, cx: &mut Context<Self>) {
        self.pending_focus = true;
        cx.notify();
    }

    /// Puts the keyboard in the buffer.
    pub fn focus_editor(&self, window: &mut Window, cx: &mut App) {
        let handle = self.editor.read(cx).focus_handle(cx);
        window.focus(&handle, cx);
    }

    /// Dismisses the menus when a shell command changes the active surface.
    pub fn close_context_menu(&mut self, cx: &mut Context<Self>) -> bool {
        let closed = self.context_menu.take().is_some() | self.tab_menu.take().is_some();
        if closed {
            cx.notify();
        }
        closed
    }

    /// How many columns apart this file's tab stops are drawn.
    pub fn tab_width(&self, cx: &App) -> usize {
        self.editor.read(cx).tab_width()
    }

    /// Draws this file's tabs reaching a stop every `width` columns, in both
    /// halves, and remembers the width for the file in the settings global.
    ///
    /// Only the drawing changes: the buffer keeps its tabs and is no dirtier
    /// than it was. The settings are recorded but not written — the menu that
    /// calls this writes them, and [`app_settings::save`] takes them to disk at
    /// shutdown in any case.
    pub fn set_tab_width(&mut self, width: usize, cx: &mut Context<Self>) {
        for editor in [&self.editor, &self.preview] {
            editor.update(cx, |editor, cx| editor.set_tab_width(width, cx));
        }
        let width = self.tab_width(cx);
        let key = settings_key(&self.path);
        let mut settings = app_settings::current(cx);
        if settings.template_tab_width(&key) as usize != width {
            settings.set_template_tab_width(&key, width as u32);
            app_settings::replace(settings, cx);
        }
        cx.notify();
    }

    /// Puts the tab width menu up at `position`, putting away everything else
    /// that floats over the pane.
    fn open_tab_menu(&mut self, position: Point<Pixels>, cx: &mut Context<Self>) {
        self.close_completion(cx);
        self.choice_open = false;
        self.context_menu = None;
        self.tab_menu = Some(position);
        cx.notify();
    }

    /// The tab width menu's rows: one per width, the one in force ticked.
    fn tab_width_rows(&self, cx: &Context<Self>) -> Vec<MenuRow> {
        let current = self.tab_width(cx);
        (1..=MENU_TAB_WIDTHS)
            .map(|width| {
                let this = cx.entity();
                MenuRow::new(width.to_string())
                    .checked(width == current)
                    .on_activate(move |_window, cx| {
                        this.update(cx, |pane, cx| pane.set_tab_width(width, cx));
                        // Written through at once, as a choice made in a menu
                        // is everywhere else: it should survive a crash the way
                        // a saved setting does.
                        app_settings::save(cx);
                    })
            })
            .collect()
    }

    fn render_tab_menu(&self, cx: &mut Context<Self>) -> Option<ContextMenu> {
        let position = self.tab_menu?;
        let this = cx.entity();
        Some(
            ContextMenu::new("template-tab-width")
                .position(position)
                .entries(menu_rows::entries(self.tab_width_rows(cx)))
                .on_dismiss(move |_window, cx| {
                    this.update(cx, |pane, cx| {
                        pane.tab_menu = None;
                        cx.notify();
                    });
                }),
        )
    }

    fn editor_rows(&self, cx: &Context<Self>) -> Vec<MenuRow> {
        let editor = self.editor.read(cx);
        let handle = editor.focus_handle(cx);
        let selected = editor.has_selection();
        let modifier = SHORTCUT_MODIFIER;
        let row =
            |label: SharedString, shortcut: String, enabled: bool, action: Box<dyn Action>| {
                let handle = handle.clone();
                MenuRow::new(label)
                    .shortcut(shortcut)
                    .enabled(enabled)
                    .on_activate(move |window, cx| handle.dispatch_action(&*action, window, cx))
            };
        let this = cx.entity();
        vec![
            row(
                ts!("input.menu_cut"),
                format!("{modifier}+X"),
                selected,
                Box::new(Cut),
            ),
            row(
                ts!("input.menu_copy"),
                format!("{modifier}+C"),
                selected,
                Box::new(Copy),
            ),
            row(
                ts!("input.menu_paste"),
                format!("{modifier}+V"),
                true,
                Box::new(Paste),
            ),
            MenuRow::separator(),
            row(
                ts!("input.menu_select_all"),
                format!("{modifier}+A"),
                true,
                Box::new(SelectAll),
            ),
            MenuRow::separator(),
            row(
                ts!("template.menu_undo"),
                format!("{modifier}+Z"),
                editor.can_undo(),
                Box::new(Undo),
            ),
            row(
                ts!("template.menu_redo"),
                format!("{modifier}+Shift+Z"),
                editor.can_redo(),
                Box::new(Redo),
            ),
            MenuRow::separator(),
            row(
                ts!("template.menu_find"),
                format!("{modifier}+F"),
                true,
                Box::new(Find),
            ),
            row(
                ts!("template.menu_replace"),
                format!("{modifier}+H"),
                true,
                Box::new(Replace),
            ),
            MenuRow::separator(),
            MenuRow::new(ts!("common.save"))
                .shortcut(format!("{modifier}+S"))
                .on_activate(move |_window, cx| {
                    this.update(cx, |pane, cx| {
                        pane.save(cx);
                    });
                }),
        ]
    }

    fn render_context_menu(&self, cx: &mut Context<Self>) -> Option<ContextMenu> {
        let position = self.context_menu?;
        let this = cx.entity();
        Some(
            ContextMenu::new("template-context")
                .position(position)
                .entries(menu_rows::entries(self.editor_rows(cx)))
                .on_dismiss(move |_window, cx| {
                    this.update(cx, |pane, cx| {
                        pane.close_context_menu(cx);
                    });
                }),
        )
    }

    /// Replaces the tables the preview may be rendered against.
    pub fn set_choices(&mut self, choices: Vec<SharedString>, cx: &mut Context<Self>) {
        self.choice = self.choice.min(choices.len().saturating_sub(1));
        self.choices = choices;
        // Explorer changes do not run the dropdown's selection callback.
        // Even an unchanged index can now name a different table, or the
        // first table available since this template was opened.
        self.request_render(cx);
        cx.notify();
    }

    /// Which table the preview is being rendered against.
    pub fn choice(&self) -> usize {
        self.choice
    }

    /// Picks the table the preview renders against, as the header does.
    ///
    /// For the tests: the dropdown in [`TemplatePane::render`] is the only
    /// other way in, and it is a click on an element.
    #[cfg(test)]
    pub fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.choice = index;
        self.request_render(cx);
        cx.notify();
    }

    /// Replaces the entries the completion popup filters.
    pub fn set_items(&mut self, items: Vec<PaletteItem>, cx: &mut Context<Self>) {
        self.items = items;
        if self.completion.is_some() {
            self.refresh_completion(cx);
        }
    }

    /// Shows and hides the preview half.
    pub fn toggle_preview(&mut self, cx: &mut Context<Self>) {
        self.preview_open = !self.preview_open;
        if self.preview_open {
            self.request_render(cx);
        }
        cx.notify();
    }

    /// Whether the preview half is on screen.
    #[cfg(test)]
    pub fn preview_open(&self) -> bool {
        self.preview_open
    }

    /// Replaces the whole buffer, as a reload would.
    ///
    /// For the tests: everything else edits through the editor's own commands.
    #[cfg(test)]
    pub fn set_source(&mut self, text: &str, cx: &mut Context<Self>) {
        let lf = to_lf(text);
        self.editor
            .update(cx, |editor, cx| editor.set_text(&lf, cx));
    }

    /// What the preview half is showing, for the tests.
    #[cfg(test)]
    pub fn preview_text(&self, cx: &App) -> String {
        self.preview.read(cx).text()
    }

    /// Writes `text` at the caret, which is what a palette click does.
    pub fn insert(&mut self, text: &str, window: &mut Window, cx: &mut Context<Self>) {
        if self.failure.is_some() {
            return;
        }
        self.editor
            .update(cx, |editor, cx| editor.insert_at_caret(text, cx));
        self.focus_editor(window, cx);
    }

    /// Records what the shell rendered.
    pub fn deliver_preview(
        &mut self,
        outcome: PreviewOutcome,
        source: &str,
        cx: &mut Context<Self>,
    ) {
        match outcome {
            PreviewOutcome::Rendered {
                path,
                content,
                warnings,
            } => {
                self.preview_note = None;
                self.preview_path = Some(SharedString::from(path.display().to_string()));
                // The preview is the *output* file, so it is coloured by the
                // output file's language rather than by the template's.
                let highlighter = path.extension().and_then(|ext| {
                    rugpui_editor::highlighter_for_extension(&ext.to_string_lossy())
                });
                self.preview.update(cx, |editor, cx| {
                    editor.set_highlighter(highlighter, cx);
                    editor.set_text(&to_lf(&content), cx);
                });
                // A parse error outranks a warning and is computed locally, so
                // it is never overwritten by an answer that is on its way.
                if !self.diagnostics.iter().any(|found| found.error) {
                    let mut found = warning_diagnostics(source, &warnings);
                    // The spans are offsets into what was *rendered*, which
                    // carries the file's line ending; the caret moves through
                    // the buffer, which is `\n` only. The line and the column
                    // are the same in both, so the offset is taken again from
                    // those.
                    let buffer = self.editor.read(cx).text();
                    for diagnostic in &mut found {
                        diagnostic.offset = offset_of(&buffer, diagnostic.line, diagnostic.column);
                    }
                    self.diagnostics = found;
                    self.apply_marks(cx);
                }
            }
            PreviewOutcome::Refused(message) => {
                self.preview_note = Some(message);
                self.preview_path = None;
                self.preview
                    .update(cx, |editor, cx| editor.set_text("", cx));
            }
        }
        cx.notify();
    }

    /// Puts the current diagnostics in the gutter.
    fn apply_marks(&mut self, cx: &mut Context<Self>) {
        let marks = marks_of(&self.diagnostics);
        self.editor
            .update(cx, |editor, cx| editor.set_marks(marks, cx));
    }

    // --- the debounce -----------------------------------------------------

    /// The buffer changed: arm the debounce and follow the completion popup.
    fn changed(&mut self, cx: &mut Context<Self>) {
        cx.emit(TemplatePaneEvent::DirtyChanged);
        self.refresh_completion(cx);
        let task = cx.spawn(async move |pane, cx| {
            cx.background_executor().timer(DEBOUNCE).await;
            pane.update(cx, |pane, cx| {
                pane.diagnose(cx);
                pane.request_render(cx);
            })
            .ok();
        });
        self._debounce = Some(task);
        cx.notify();
    }

    /// The caret moved: a popup that was opened somewhere else is no longer
    /// about where the caret is.
    fn moved(&mut self, cx: &mut Context<Self>) {
        if self.completion.is_some() {
            self.refresh_completion(cx);
        }
    }

    /// Parses the buffer and marks what the parser says.
    ///
    /// The whole verdict is replaced every time rather than merged: a parse
    /// error that has been fixed has to disappear, and the warnings that were
    /// under it belong to a render that has not happened yet.
    fn diagnose(&mut self, cx: &mut Context<Self>) {
        if self.failure.is_some() {
            return;
        }
        let source = self.editor.read(cx).text();
        self.diagnostics = match Template::parse(&source) {
            Ok(_) => Vec::new(),
            Err(error) => vec![parse_diagnostic(&source, &error)],
        };
        self.apply_marks(cx);
        cx.notify();
    }

    /// Asks the shell for a preview of what is in the buffer now.
    pub fn request_render(&mut self, cx: &mut Context<Self>) {
        if self.failure.is_some() || !self.preview_open {
            return;
        }
        if self.diagnostics.iter().any(|found| found.error) {
            // A template that does not parse renders to nothing; the error is
            // already in the list, and a second message saying the same thing
            // in the preview half would be noise.
            self.preview_note = Some(ts!("template.preview_parse_error"));
            self.preview_path = None;
            cx.notify();
            return;
        }
        cx.emit(TemplatePaneEvent::Render {
            source: self.source(cx),
            table: self.choice,
        });
    }

    /// Runs the parse and the render straight away, skipping the debounce.
    ///
    /// What a freshly opened tab and a changed table both want: there is
    /// nothing to wait for, because nothing is being typed.
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        self.diagnose(cx);
        self.request_render(cx);
    }

    /// Moves the caret to a diagnostic and gives the buffer the keyboard.
    fn jump_to(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        let Some(diagnostic) = self.diagnostics.get(index) else {
            return;
        };
        let offset = diagnostic.offset;
        self.editor
            .update(cx, |editor, cx| editor.move_to(offset, cx));
        self.focus_editor(window, cx);
        cx.notify();
    }

    // --- the completion popup ---------------------------------------------

    /// Opens the popup where the caret is, whatever the caret's context.
    ///
    /// What `Ctrl+Space` does. Unlike the automatic path it offers the whole
    /// `${…}` form when the caret is in the text between statements, which is
    /// the only way to get one without typing the brace first.
    pub fn trigger_completion(&mut self, cx: &mut Context<Self>) {
        if self.failure.is_some() {
            return;
        }
        let Some(request) = self.caret_request(cx) else {
            return;
        };
        self.open_completion(request, cx);
    }

    /// What the caret's context is, as [`palette::completion_at`] reads it.
    fn caret_request(&self, cx: &App) -> Option<CompletionRequest> {
        palette::completion_at(&self.editor.read(cx).line_before_caret())
    }

    /// Re-filters an open popup, and opens one where typing has earned it.
    ///
    /// Automatic only inside a statement: a popup that appeared over ordinary
    /// text on every word would be in the way of writing the Java the template
    /// is mostly made of.
    fn refresh_completion(&mut self, cx: &mut Context<Self>) {
        let open = self.completion.is_some();
        let Some(request) = self.caret_request(cx) else {
            self.close_completion(cx);
            return;
        };
        if !open && request.kind == CompletionKind::Text {
            return;
        }
        self.open_completion(request, cx);
    }

    /// Puts the popup up with whatever `request` matches, or takes it down
    /// when nothing does.
    fn open_completion(&mut self, request: CompletionRequest, cx: &mut Context<Self>) {
        let matches: Vec<PaletteItem> = palette::matching(&self.items, &request)
            .into_iter()
            .cloned()
            .collect();
        if matches.is_empty() {
            self.close_completion(cx);
            return;
        }
        let anchor = self.editor.read(cx).caret_bounds();
        let selected = match &self.completion {
            // Keep the highlighted row while the same list is being narrowed,
            // so that typing one more character does not move the choice out
            // from under the finger about to press Enter.
            Some(open) => open
                .matches
                .get(open.selected)
                .and_then(|item| matches.iter().position(|other| other.name == item.name))
                .unwrap_or(0),
            None => 0,
        };
        self.completion = Some(Completion {
            request,
            matches,
            selected,
            anchor,
        });
        self.editor
            .update(cx, |editor, _cx| editor.set_intercept(true));
        cx.notify();
    }

    /// Takes the popup down and gives the five keys back to the editor.
    fn close_completion(&mut self, cx: &mut Context<Self>) {
        if self.completion.take().is_some() {
            self.editor
                .update(cx, |editor, _cx| editor.set_intercept(false));
            cx.notify();
        }
    }

    /// Whether the popup is up.
    #[cfg(test)]
    pub fn completion_open(&self) -> bool {
        self.completion.is_some()
    }

    /// What the popup is offering, for the tests.
    #[cfg(test)]
    pub fn completion_names(&self) -> Vec<SharedString> {
        self.completion
            .as_ref()
            .map(|open| open.matches.iter().map(|item| item.name.clone()).collect())
            .unwrap_or_default()
    }

    /// One of the five keys the editor was asked to hand over.
    fn intercepted(&mut self, key: NavKey, cx: &mut Context<Self>) {
        let Some(open) = &mut self.completion else {
            return;
        };
        let count = open.matches.len();
        match key {
            NavKey::Up => {
                open.selected = open.selected.checked_sub(1).unwrap_or(count - 1);
                cx.notify();
            }
            NavKey::Down => {
                open.selected = (open.selected + 1) % count;
                cx.notify();
            }
            NavKey::Enter | NavKey::Tab => {
                let selected = open.selected;
                self.accept_completion(selected, cx);
            }
            NavKey::Escape => self.close_completion(cx),
        }
    }

    /// Writes the highlighted entry over the prefix that was typed.
    ///
    /// `pub(crate)` for the test that drives the popup from the shell; the
    /// popup's own rows and the intercepted keys are the callers that matter.
    pub(crate) fn accept_completion(&mut self, index: usize, cx: &mut Context<Self>) {
        let Some(open) = &self.completion else {
            return;
        };
        let Some(item) = open.matches.get(index) else {
            return;
        };
        let text = palette::insertion(item, &open.request.kind);
        let length = open.request.prefix.len();
        self.editor.update(cx, |editor, cx| {
            let caret = editor.caret();
            editor.replace_range(caret.saturating_sub(length)..caret, &text, cx);
        });
        self.close_completion(cx);
    }

    // --- rendering --------------------------------------------------------

    /// The strip over the two halves: the file, the dirty marker, the save
    /// button, and the toggle.
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme(cx);
        let dirty = self.is_dirty(cx);
        let save = cx.entity();
        let toggle = cx.entity();
        let tabs = cx.entity();
        let open = self.preview_open;
        let tab_menu_open = self.tab_menu.is_some();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .flex_none()
            .w_full()
            .min_w_0()
            .px(px(10.))
            .py(px(5.))
            .border_b_1()
            .border_color(theme.border)
            .child(
                div()
                    .flex_1()
                    .min_w_0()
                    .truncate()
                    .text_size(px(11.))
                    .text_color(theme.text_muted)
                    .child(SharedString::from(self.path.display().to_string())),
            )
            .children(dirty.then(|| {
                div()
                    .flex_none()
                    .text_size(px(11.))
                    .text_color(theme.accent)
                    .child(ts!("template.unsaved"))
            }))
            .child(
                div()
                    .id("template-tab-width")
                    .debug_selector(|| "template-tab-width".to_string())
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(px(20.))
                    .px(px(8.))
                    .rounded_md()
                    .text_size(px(11.))
                    .text_color(if tab_menu_open {
                        theme.text
                    } else {
                        theme.text_muted
                    })
                    .when(tab_menu_open, |this| this.bg(theme.surface_active))
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface_hover))
                    .tooltip(tooltip_label(ts!("template.tip_tab_width")))
                    .on_click(move |event, _window, cx| {
                        let position = event.position();
                        tabs.update(cx, |pane, cx| pane.open_tab_menu(position, cx));
                    })
                    .child(ts!(
                        "template.tab_width",
                        width = self.tab_width(cx).to_string()
                    )),
            )
            .child(
                Button::new("template-save", ts!("common.save"))
                    .variant(ButtonVariant::Secondary)
                    .compact()
                    .disabled(!dirty)
                    .on_click(move |_, _window, cx| {
                        save.update(cx, |pane, cx| {
                            pane.save(cx);
                        });
                    }),
            )
            .child(
                div()
                    .id("template-preview-toggle")
                    .flex()
                    .flex_none()
                    .items_center()
                    .justify_center()
                    .h(px(20.))
                    .px(px(8.))
                    .rounded_md()
                    .text_size(px(11.))
                    .text_color(if open { theme.text } else { theme.text_muted })
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface_hover))
                    .tooltip(tooltip_label(ts!("template.tip_preview")))
                    .on_click(move |_, _window, cx| {
                        toggle.update(cx, |pane, cx| pane.toggle_preview(cx));
                    })
                    .child(if open {
                        ts!("template.hide_preview")
                    } else {
                        ts!("template.show_preview")
                    }),
            )
    }

    /// The preview half: which table it is of, where it would be written, and
    /// the text itself.
    fn render_preview(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme(cx);
        let select = cx.entity();
        let toggle = cx.entity();
        let chosen = self.choices.get(self.choice).cloned();
        let mono = app_settings::editor_font(cx);
        let font_size = app_settings::effective(cx).editor_font_size;
        app_settings::apply_word_wrap(&self.preview, cx);

        div()
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .w_full()
            .min_h_0()
            .child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .flex_none()
                    .w_full()
                    .min_w_0()
                    .px(px(8.))
                    .py(px(4.))
                    .border_b_1()
                    .border_color(theme.border)
                    .child(
                        Select::new("template-preview-table")
                            .chevron_icon(icons::CHEVRON_DOWN)
                            .options(self.choices.clone())
                            .selected(chosen)
                            .placeholder(ts!("template.preview_table"))
                            .open(self.choice_open)
                            .width(px(CHOICE_WIDTH))
                            .on_select(move |index, _text, _window, cx| {
                                select.update(cx, |pane, cx| {
                                    pane.choice_open = false;
                                    pane.choice = index;
                                    pane.request_render(cx);
                                    cx.notify();
                                });
                            })
                            .on_open_change(move |open, _window, cx| {
                                toggle.update(cx, |pane, cx| {
                                    pane.choice_open = open;
                                    cx.notify();
                                });
                            }),
                    )
                    .children(self.preview_path.clone().map(|path| {
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child(path)
                    })),
            )
            .child(match &self.preview_note {
                Some(note) => div()
                    .flex()
                    .flex_grow_1()
                    .min_h_0()
                    // Without this the message — which is the engine's, and
                    // can be a whole sentence — runs off the half instead of
                    // wrapping inside it.
                    .min_w_0()
                    .w_full()
                    .p(px(12.))
                    .text_size(px(12.))
                    .text_color(theme.text_muted)
                    .child(note.clone())
                    .into_any_element(),
                None => div()
                    .id("template-preview-source")
                    .debug_selector(|| "template-preview-source".to_string())
                    .flex()
                    .flex_grow_1()
                    .min_w_0()
                    .w_full()
                    .min_h_0()
                    // The same face and size the editor half draws with: the
                    // two sit side by side showing source and result, and a
                    // preview in the interface font would read as chrome
                    // rather than as the file the run will write.
                    .font_family(mono)
                    .text_size(px(font_size))
                    .child(self.preview.clone())
                    .into_any_element(),
            })
    }

    /// The list under both halves, when there is anything in it.
    fn render_diagnostics(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme(cx);
        // The chrome palette has no "looks wrong" colour — the editor's does,
        // and it is the one the gutter marks are painted in, so the list and
        // the marks agree.
        let warning = editor_theme(cx).warning;
        let rows: Vec<_> = self
            .diagnostics
            .iter()
            .enumerate()
            .map(|(index, diagnostic)| {
                let color = if diagnostic.error {
                    theme.danger
                } else {
                    warning
                };
                div()
                    .id(("template-diagnostic", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .w_full()
                    .min_w_0()
                    .px(px(8.))
                    .py(px(2.))
                    .rounded_md()
                    .cursor_pointer()
                    .hover(|style| style.bg(theme.surface_hover))
                    .on_click(cx.listener(move |pane, _, window, cx| {
                        pane.jump_to(index, window, cx);
                    }))
                    .child(
                        div()
                            .flex_none()
                            .w(px(64.))
                            .text_size(px(11.))
                            .text_color(color)
                            .child(ts!(
                                "diagnostics.at",
                                line = diagnostic.line + 1,
                                column = diagnostic.column + 1
                            )),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.text)
                            .child(diagnostic.message.clone()),
                    )
            })
            .collect();

        div()
            .id("template-diagnostics")
            .flex()
            .flex_col()
            .flex_none()
            .w_full()
            .min_w_0()
            .h(px(DIAGNOSTICS_HEIGHT))
            .gap(px(1.))
            .py(px(4.))
            .overflow_y_scroll()
            .bg(theme.surface)
            .border_t_1()
            .border_color(theme.border)
            .child(
                div()
                    .px(px(8.))
                    .pb(px(2.))
                    .text_size(px(10.))
                    .text_color(theme.text_muted)
                    .child(ts!("diagnostics.title", count = self.diagnostics.len())),
            )
            .children(rows)
    }

    /// The completion popup, hung off the caret.
    fn render_completion(&self, cx: &mut Context<Self>) -> Option<impl IntoElement + use<>> {
        let open = self.completion.as_ref()?;
        let anchor = open.anchor?;
        let theme = theme(cx);
        let mono = app_settings::editor_font(cx);
        let selected = open.selected;
        // A window of the list around the highlighted row, so that walking
        // past the bottom of the popup keeps the choice on screen without the
        // popup itself scrolling.
        let first = selected.saturating_sub(POPUP_ROWS - 1);
        let rows: Vec<_> = open
            .matches
            .iter()
            .enumerate()
            .skip(first)
            .take(POPUP_ROWS)
            .map(|(index, item)| {
                let active = index == selected;
                div()
                    .id(("template-completion", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(6.))
                    .px(px(8.))
                    .h(px(POPUP_ROW_HEIGHT))
                    .cursor_pointer()
                    .when(active, |row| row.bg(theme.surface_hover))
                    .hover(|style| style.bg(theme.surface_hover))
                    .on_mouse_down(
                        MouseButton::Left,
                        cx.listener(move |pane, _, _window, cx| {
                            pane.accept_completion(index, cx);
                        }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .font_family(mono.clone())
                            .text_size(px(11.))
                            .text_color(theme.text)
                            .child(item.name.clone()),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(10.))
                            .text_color(theme.text_muted)
                            .child(item.description.clone()),
                    )
            })
            .collect();

        // What is left below the window of rows, so that a list of sixty
        // entries does not look like a list of nine.
        let hidden = open.matches.len().saturating_sub(first + POPUP_ROWS);
        let footer = div()
            .flex()
            .flex_row()
            .items_center()
            .justify_between()
            .gap(px(8.))
            .px(px(8.))
            .pt(px(2.))
            .border_t_1()
            .border_color(theme.border)
            .text_size(px(10.))
            .text_color(theme.text_muted)
            .child(ts!("completion.hint"))
            .children((hidden > 0).then(|| div().child(ts!("completion.more", count = hidden))));

        let list = div()
            .occlude()
            .flex()
            .flex_col()
            .w(px(POPUP_WIDTH))
            .py(px(2.))
            .rounded_md()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .shadow_md()
            .children(rows)
            .child(footer);

        Some(
            div().w(px(0.)).h(px(0.)).child(
                deferred(
                    anchored()
                        .position(point(anchor.left(), anchor.bottom()))
                        .position_mode(AnchoredPositionMode::Window)
                        .anchor(Anchor::TopLeft)
                        .snap_to_window_with_margin(px(8.))
                        .child(list),
                )
                .with_priority(2),
            ),
        )
    }
}

impl EventEmitter<TemplatePaneEvent> for TemplatePane {}

impl Focusable for TemplatePane {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TemplatePane {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        // The one place the buffer can be focused from: everywhere else the
        // element that holds the focus handle is not on screen yet.
        if self.pending_focus && self.failure.is_none() {
            self.pending_focus = false;
            self.focus_editor(window, cx);
        }
        let theme = theme(cx);
        let mono = app_settings::editor_font(cx);
        let font_size = app_settings::effective(cx).editor_font_size;
        app_settings::apply_word_wrap(&self.editor, cx);

        if let Some(failure) = &self.failure {
            return div()
                .key_context("TemplatePane")
                .track_focus(&self.focus_handle)
                .flex()
                .flex_col()
                .flex_grow_1()
                .min_w_0()
                .w_full()
                .h_full()
                .min_h_0()
                .p(px(16.))
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(failure.clone())
                .into_any_element();
        }

        let editor = div()
            .id("template-source")
            .debug_selector(|| "template-source".to_string())
            .flex()
            .flex_col()
            .flex_1()
            .min_w_0()
            .w_full()
            .min_h_0()
            .font_family(mono)
            .text_size(px(font_size))
            .child(self.editor.clone());

        // With the preview up the two halves are a `Splitter`, which owns the
        // divider, the drag and the grab band; the pane keeps nothing but the
        // ratio it is drawn at. With the preview away there is nothing to
        // divide and the editor simply fills the row.
        let body = if self.preview_open {
            let preview = self.render_preview(cx).into_any_element();
            let this = cx.entity();
            Splitter::new("template-split", Axis::Horizontal)
                .ratio(self.split)
                .min_ratio(MIN_SPLIT)
                .first(editor)
                .second(preview)
                .on_change(move |ratio, _window, cx| {
                    this.update(cx, |pane, cx| {
                        pane.split = ratio;
                        cx.notify();
                    });
                })
                .into_any_element()
        } else {
            editor.into_any_element()
        };

        div()
            .key_context("TemplatePane")
            .track_focus(&self.focus_handle)
            .capture_action(cx.listener(|pane, _: &CloseFind, _window, cx| {
                if pane.close_context_menu(cx) {
                    cx.stop_propagation();
                } else {
                    cx.propagate();
                }
            }))
            .relative()
            .flex()
            .flex_col()
            .flex_grow_1()
            .min_w_0()
            .w_full()
            // `h_full`, not just the grow: the pane is drawn inside
            // `cached_view`'s box, which is no flex container, so a grow alone
            // leaves the root as tall as its content and the editor never
            // follows the window.
            .h_full()
            .min_h_0()
            .child(self.render_header(cx))
            .children(self.save_failure.clone().map(|message| {
                div()
                    .flex_none()
                    .px(px(10.))
                    .py(px(4.))
                    .text_size(px(11.))
                    .text_color(theme.danger)
                    .child(message)
            }))
            .child(
                div()
                    .flex()
                    .flex_row()
                    .flex_grow_1()
                    .min_h_0()
                    .min_w_0()
                    .w_full()
                    .child(body),
            )
            .children((!self.diagnostics.is_empty()).then(|| self.render_diagnostics(cx)))
            .children(self.render_completion(cx))
            .children(self.render_context_menu(cx))
            .children(self.render_tab_menu(cx))
            .into_any_element()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{TestAppContext, VisualTestContext};

    struct Harness {
        pane: Entity<TemplatePane>,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .size_full()
                .flex()
                .flex_col()
                .min_w_0()
                .w_full()
                .child(self.pane.clone())
        }
    }

    fn open_pane(
        source: &str,
        cx: &mut TestAppContext,
    ) -> (tempfile::TempDir, Entity<TemplatePane>, VisualTestContext) {
        cx.update(|cx| {
            app_settings::init(cx);
            app_settings::replace(Default::default(), cx);
            rugpui::init(cx);
            rugpui_editor::init(cx);
        });
        let dir = tempfile::tempdir().expect("a temporary directory");
        let file = dir.path().join("model.java");
        std::fs::write(&file, source).expect("the template is written");
        let window = cx.add_window(|_, cx| Harness {
            pane: cx.new(|cx| TemplatePane::open(file, cx)),
        });
        let pane = window
            .update(cx, |harness, _, _| harness.pane.clone())
            .unwrap();
        let cx = VisualTestContext::from_window(window.into(), cx);
        cx.run_until_parked();
        (dir, pane, cx)
    }

    #[gpui::test]
    fn narrowing_the_window_narrows_both_split_halves(cx: &mut TestAppContext) {
        let source = "class Album {\n    String title;\n}\n";
        let (_dir, _pane, mut cx) = open_pane(source, cx);
        cx.simulate_resize(gpui::size(px(1100.), px(700.)));
        cx.run_until_parked();
        let wide_source = cx
            .debug_bounds("template-source")
            .expect("the source half is drawn")
            .size
            .width;
        let wide_preview = cx
            .debug_bounds("template-preview-source")
            .expect("the preview half is drawn")
            .size
            .width;
        cx.simulate_resize(gpui::size(px(800.), px(700.)));
        cx.run_until_parked();
        let narrow_source = cx
            .debug_bounds("template-source")
            .expect("the source half is drawn")
            .size
            .width;
        let narrow_preview = cx
            .debug_bounds("template-preview-source")
            .expect("the preview half is drawn")
            .size
            .width;
        // The window lost 300px; the split keeps its ratio, so both halves
        // have to lose their share of it. A chain that refuses to shrink keeps
        // the wide widths here, which is the regression this guards against.
        assert!(
            narrow_source < wide_source,
            "the source half did not shrink: wide {wide_source:?}, narrow {narrow_source:?}"
        );
        assert!(
            narrow_preview < wide_preview,
            "the preview half did not shrink: wide {wide_preview:?}, narrow {narrow_preview:?}"
        );
        let total = (wide_source + wide_preview) - (narrow_source + narrow_preview);
        assert!(
            (total - px(300.)).abs() < px(4.),
            "the halves did not track the window: wide {wide_source:?} + {wide_preview:?}, narrow {narrow_source:?} + {narrow_preview:?}"
        );
        // The ratio survives the resize: the default split starts at 0.55.
        let wide_ratio = wide_source / (wide_source + wide_preview);
        let narrow_ratio = narrow_source / (narrow_source + narrow_preview);
        assert!(
            (wide_ratio - narrow_ratio).abs() < 0.05,
            "the split moved: wide {wide_ratio}, narrow {narrow_ratio}"
        );
    }

    /// The pane as the workspace draws it: through `cached_view`, in the slot
    /// above a fixed band.
    struct CachedHarness {
        pane: Entity<TemplatePane>,
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
                        .flex_col()
                        .flex_1()
                        .min_h_0()
                        .child(rugpui::cached_view(self.pane.clone())),
                )
                .child(div().flex_none().h(px(40.)))
        }
    }

    #[gpui::test]
    fn shortening_the_window_shortens_the_editor(cx: &mut TestAppContext) {
        let (_dir, pane, _) = open_pane("class Album {}\n", cx);
        let window = cx.add_window(|_, _| CachedHarness { pane });
        let mut cx = VisualTestContext::from_window(window.into(), cx);
        cx.simulate_resize(gpui::size(px(900.), px(700.)));
        cx.run_until_parked();
        let tall = cx
            .debug_bounds("template-source")
            .expect("the source is drawn")
            .size
            .height;
        cx.simulate_resize(gpui::size(px(900.), px(400.)));
        cx.run_until_parked();
        let short = cx
            .debug_bounds("template-source")
            .expect("the source is drawn")
            .size
            .height;
        // A root that only grows is as tall as its content inside the cached
        // box, so the editor kept one height whatever the window did.
        let delta = tall - short;
        assert!(
            (delta - px(300.)).abs() < px(2.),
            "the editor did not track the window: tall {tall:?}, short {short:?}"
        );
    }

    #[gpui::test]
    fn keyboard_selection_survives_a_rendered_live_preview(cx: &mut TestAppContext) {
        let source = "class Album {\n    String title;\n}\n";
        let (_dir, pane, mut cx) = open_pane(source, cx);
        cx.update(|window, cx| {
            pane.update(cx, |pane, cx| {
                pane.set_choices(vec!["ALBUM".into()], cx);
                pane.deliver_preview(
                    PreviewOutcome::Rendered {
                        path: PathBuf::from("Album.java"),
                        content: source.to_owned(),
                        warnings: Vec::new(),
                    },
                    source,
                    cx,
                );
                pane.editor
                    .update(cx, |editor, cx| editor.select_range(0..0, cx));
                pane.focus_editor(window, cx);
            });
        });
        cx.run_until_parked();
        cx.simulate_keystrokes("shift-right shift-right shift-right");
        cx.update(|window, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.editor.read(cx).selection(), 0..3);
            assert!(pane.editor.read(cx).focus_handle(cx).is_focused(window));
            assert!(!pane.preview.read(cx).has_selection());
        });
    }

    #[gpui::test]
    fn wheel_scrolling_either_side_keeps_the_preview_and_source_together(cx: &mut TestAppContext) {
        use gpui::{Modifiers, ScrollDelta, ScrollWheelEvent, TouchPhase};
        use rugpui::scrollbar::ScrollbarAxis;

        let source = format!("{}\n", "source ".repeat(60)).repeat(200);
        let content = format!("{}\n", "output ".repeat(100)).repeat(400);
        let (_dir, pane, mut cx) = open_pane(&source, cx);
        cx.update(|_, cx| {
            pane.update(cx, |pane, cx| {
                pane.deliver_preview(
                    PreviewOutcome::Rendered {
                        path: PathBuf::from("output.java"),
                        content,
                        warnings: Vec::new(),
                    },
                    &source,
                    cx,
                )
            });
        });
        cx.run_until_parked();
        for (wrap, axis, delta) in [
            (false, ScrollbarAxis::Vertical, point(0., -3.)),
            (false, ScrollbarAxis::Horizontal, point(-3., 0.)),
            (true, ScrollbarAxis::Vertical, point(0., -3.)),
        ] {
            cx.update(|_, cx| {
                let mut settings = app_settings::current(cx);
                settings.editor_word_wrap = wrap;
                app_settings::replace(settings, cx);
                pane.update(cx, |pane, cx| {
                    for editor in [&pane.editor, &pane.preview] {
                        editor.update(cx, |editor, cx| editor.set_scroll_progress(axis, 0., cx));
                    }
                    cx.notify();
                });
            });
            cx.run_until_parked();
            let mut previous = 0.;
            for selector in ["template-source", "template-preview-source"] {
                let position = cx.debug_bounds(selector).unwrap().center();
                cx.simulate_mouse_move(position, None, Modifiers::none());
                cx.simulate_event(ScrollWheelEvent {
                    position,
                    delta: ScrollDelta::Lines(delta),
                    modifiers: Modifiers::none(),
                    touch_phase: TouchPhase::Moved,
                });
                cx.run_until_parked();
                previous = cx.update(|_, cx| {
                    let pane = pane.read(cx);
                    let source_progress = pane.editor.read(cx).scroll_progress(axis);
                    let preview_progress = pane.preview.read(cx).scroll_progress(axis);
                    assert!(source_progress > previous,
                        "{selector} {axis:?} did not scroll: {source_progress}, previous {previous}, preview {preview_progress}, wrap {}",
                        pane.editor.read(cx).is_word_wrap());
                    assert!(
                        (source_progress - preview_progress).abs() < 0.00001,
                        "{selector}: source {source_progress}, preview {preview_progress}"
                    );
                    assert_eq!(pane.editor.read(cx).selection(), 0..0);
                    assert_eq!(pane.preview.read(cx).selection(), 0..0);
                    assert!(!pane.is_dirty(cx));
                    source_progress
                });
            }
        }
    }

    #[gpui::test]
    fn the_context_menu_preserves_selection_and_edits_the_source(cx: &mut TestAppContext) {
        use gpui::{Modifiers, MouseDownEvent, MouseUpEvent};
        use menu_rows::row;

        let (dir, pane, mut cx) = open_pane("class Album {}\n", cx);
        cx.update(|_window, cx| {
            pane.update(cx, |pane, cx| {
                let rows = pane.editor_rows(cx);
                assert!(!row(&rows, &ts!("input.menu_cut")).is_enabled());
                assert!(!row(&rows, &ts!("input.menu_copy")).is_enabled());
                assert!(!row(&rows, &ts!("template.menu_undo")).is_enabled());
                pane.editor
                    .update(cx, |editor, cx| editor.select_range(6..11, cx));
            });
        });
        cx.run_until_parked();
        let bounds = cx
            .debug_bounds("template-source")
            .expect("the source is rendered");
        let position = bounds.origin + point(px(70.), px(10.));
        cx.simulate_event(MouseDownEvent {
            position,
            modifiers: Modifiers::none(),
            button: MouseButton::Right,
            click_count: 1,
            first_mouse: false,
        });
        cx.simulate_event(MouseUpEvent {
            position,
            modifiers: Modifiers::none(),
            button: MouseButton::Right,
            click_count: 1,
        });
        cx.run_until_parked();
        let rows = cx.update(|_window, cx| {
            pane.update(cx, |pane, cx| {
                assert_eq!(pane.context_menu, Some(position));
                assert_eq!(pane.editor.read(cx).selection(), 6..11);
                pane.editor_rows(cx)
            })
        });
        // Escape puts the menu away without clearing the selection.
        cx.simulate_keystrokes("escape");
        cx.update(|_window, cx| {
            assert!(pane.read(cx).context_menu.is_none());
            assert_eq!(pane.read(cx).editor.read(cx).selection(), 6..11);
        });
        cx.update(|window, cx| row(&rows, &ts!("input.menu_cut")).activate(window, cx));
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert_eq!(pane.read(cx).source(cx), "class  {}\n");
            assert_eq!(
                cx.read_from_clipboard().and_then(|item| item.text()),
                Some("Album".into())
            );
            let rows = pane.update(cx, |pane, cx| pane.editor_rows(cx));
            row(&rows, &ts!("template.menu_undo")).activate(window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            assert_eq!(pane.read(cx).source(cx), "class Album {}\n");
            let rows = pane.update(cx, |pane, cx| pane.editor_rows(cx));
            row(&rows, &ts!("template.menu_redo")).activate(window, cx);
        });
        cx.run_until_parked();
        cx.update(|window, cx| {
            let rows = pane.update(cx, |pane, cx| pane.editor_rows(cx));
            row(&rows, &ts!("common.save")).activate(window, cx);
            assert!(!pane.read(cx).is_dirty(cx));
            assert_eq!(pane.read(cx).preview.read(cx).text(), "");
        });
        assert_eq!(
            std::fs::read_to_string(dir.path().join("model.java")).unwrap(),
            "class  {}\n"
        );
    }

    /// The width a file's tabs are drawn at is the file's: both halves take
    /// it, the buffer keeps its tabs, and the next pane over the same file
    /// opens at it. The menu's own write to disk is left out — a test has no
    /// business in the real `settings.json` — so the rows are read rather than
    /// run.
    #[gpui::test]
    fn a_file_keeps_the_tab_width_chosen_for_it(cx: &mut TestAppContext) {
        let source = "class Album {\n\tString title;\n}\n";
        let (dir, pane, mut cx) = open_pane(source, cx);
        let checked = |pane: &Entity<TemplatePane>, cx: &mut VisualTestContext| {
            cx.update(|_window, cx| {
                pane.update(cx, |pane, cx| {
                    let rows = pane.tab_width_rows(cx);
                    assert_eq!(
                        menu_rows::labels(&rows),
                        ["1", "2", "3", "4", "5", "6", "7", "8"]
                    );
                    rows.iter()
                        .filter(|row| row.is_checked())
                        .map(|row| row.label().to_owned())
                        .collect::<Vec<_>>()
                })
            })
        };
        assert_eq!(checked(&pane, &mut cx), ["4"], "four until told otherwise");

        // The header names the width in force and puts the menu up.
        cx.run_until_parked();
        let header = cx
            .debug_bounds("template-tab-width")
            .expect("the tab width control is drawn");
        cx.simulate_click(header.center(), gpui::Modifiers::none());
        cx.update(|_window, cx| assert!(pane.read(cx).tab_menu.is_some()));
        cx.simulate_keystrokes("escape");
        cx.update(|_window, cx| assert!(pane.read(cx).tab_menu.is_none()));

        cx.update(|_window, cx| pane.update(cx, |pane, cx| pane.set_tab_width(2, cx)));
        cx.update(|_window, cx| {
            let pane = pane.read(cx);
            assert_eq!(pane.editor.read(cx).tab_width(), 2);
            assert_eq!(pane.preview.read(cx).tab_width(), 2);
            assert_eq!(pane.source(cx), source, "the tab is still a tab");
            assert!(!pane.is_dirty(cx));
            let key = settings_key(pane.path());
            assert_eq!(app_settings::current(cx).template_tab_width(&key), 2);
        });
        assert_eq!(checked(&pane, &mut cx), ["2"]);

        // Opened again, the file comes back at its own width; another file
        // does not.
        let file = dir.path().join("model.java");
        let other = dir.path().join("mapper.xml");
        std::fs::write(&other, "<mapper/>\n").expect("the other template is written");
        cx.update(|_window, cx| {
            let again = cx.new(|cx| TemplatePane::open(file, cx));
            assert_eq!(again.read(cx).tab_width(cx), 2);
            let elsewhere = cx.new(|cx| TemplatePane::open(other, cx));
            assert_eq!(elsewhere.read(cx).tab_width(cx), 4);
        });

        // And back to the default leaves nothing behind for the file.
        cx.update(|_window, cx| pane.update(cx, |pane, cx| pane.set_tab_width(4, cx)));
        cx.update(|_window, cx| {
            assert!(app_settings::current(cx).template_tab_widths.is_empty());
        });
    }

    #[test]
    fn a_files_line_ending_is_read_off_its_first_break() {
        assert_eq!(LineEnding::of("one\r\ntwo\r\n"), LineEnding::CrLf);
        assert_eq!(LineEnding::of("one\ntwo\n"), LineEnding::Lf);
        // Nothing has been decided yet, so nothing is imposed.
        assert_eq!(LineEnding::of("one line"), LineEnding::Lf);
        assert_eq!(LineEnding::of(""), LineEnding::Lf);
        // A file whose first line is empty is still telling us something.
        assert_eq!(LineEnding::of("\none\r\n"), LineEnding::Lf);
    }

    #[test]
    fn a_crlf_file_survives_a_round_trip_through_the_buffer() {
        let original = "package ${package};\r\n\r\npublic class X {\r\n}\r\n";
        let ending = LineEnding::of(original);
        let buffer = to_lf(original);
        assert!(!buffer.contains('\r'));
        assert_eq!(ending.restore(&buffer), original);
    }

    #[test]
    fn a_lone_carriage_return_is_not_a_line_break() {
        // Not a line ending in any file this opens, and turning one into a
        // break would change the document under the user.
        assert_eq!(to_lf("a\rb"), "a\rb");
        assert_eq!(LineEnding::Lf.restore("a\rb"), "a\rb");
    }

    #[test]
    fn a_point_is_counted_in_characters_not_bytes() {
        let source = "hello\n\u{d55c}\u{ae00}${name}\n";
        let offset = source.find("${").expect("there is a statement");
        assert_eq!(point_of(source, offset), (1, 2));
        assert_eq!(point_of(source, 0), (0, 0));
        // Past the end is the end, not a panic.
        assert_eq!(point_of(source, 9_999).0, 2);
    }

    #[test]
    fn a_parse_error_lands_on_the_line_the_engine_named() {
        let source = "line one\n${for:item=columns}\nno end\n";
        let error = Template::parse(source).expect_err("an unclosed for is an error");
        let diagnostic = parse_diagnostic(source, &error);
        assert!(diagnostic.error);
        assert!(!diagnostic.message.is_empty());
        assert_eq!(
            marks_of(std::slice::from_ref(&diagnostic)),
            vec![(diagnostic.line, MarkKind::Error)]
        );
    }

    #[test]
    fn a_warning_lands_on_the_statement_it_is_about() {
        let source = "${name}\n${nmae}\n";
        let template = Template::parse(source).expect("it parses");
        let table = rudbgen_meta::Table {
            name: "T_SAMPLE".to_owned(),
            ..rudbgen_meta::Table::default()
        };
        let mut diagnostics = rudbgen_template::Diagnostics::new();
        let ctx = rudbgen_template::RenderContext::new();
        template
            .render_diagnosed(&table, &ctx, &mut diagnostics)
            .expect("an unknown field renders as nothing");
        let found = warning_diagnostics(source, diagnostics.warnings());
        assert_eq!(found.len(), 1, "{diagnostics:?}");
        assert_eq!(found[0].line, 1);
        assert!(!found[0].error);
        assert_eq!(marks_of(&found), vec![(1, MarkKind::Warning)]);
    }

    #[test]
    fn an_offset_survives_the_trip_through_a_line_and_a_column() {
        // The round trip the CRLF case rests on: a warning's span is an offset
        // into the rendered source and the caret moves through the buffer, so
        // the point is what carries between them.
        let crlf = "class ${nmae} {\r\n}\r\n";
        let buffer = to_lf(crlf);
        let span = crlf.find("${nmae}").expect("the statement is there");
        let (line, column) = point_of(crlf, span);
        assert_eq!((line, column), (0, 6));
        assert_eq!(
            offset_of(&buffer, line, column),
            buffer.find("${nmae}").unwrap()
        );

        // A second line, where the two sources have already drifted apart.
        let at = crlf.rfind('}').expect("the brace is there");
        let (line, column) = point_of(crlf, at);
        assert_eq!(line, 1);
        assert_eq!(offset_of(&buffer, line, column), buffer.rfind('}').unwrap());

        // A column past the end of the line lands at its end, not in the next.
        assert_eq!(offset_of(&buffer, 0, 999), buffer.find('\n').unwrap());
    }

    #[test]
    fn a_line_start_is_found_without_a_span() {
        let source = "one\ntwo\nthree";
        assert_eq!(line_start(source, 0), 0);
        assert_eq!(line_start(source, 1), 4);
        assert_eq!(line_start(source, 2), 8);
        // Past the last line is the end of the buffer.
        assert_eq!(line_start(source, 9), source.len());
    }
}
