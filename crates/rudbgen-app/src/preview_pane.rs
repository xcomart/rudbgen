//! The Preview tab: what one pair — or a whole dry run — would write
//! (architecture document, §4.1 rule 5 and §9).
//!
//! Two shapes, one tab. **Preview** renders a single table × template pair,
//! chosen by the two dropdowns in the header, and shows the text it would put
//! on disk. **Dry run** renders every pair into memory and shows the file list
//! — path, whether something is already there, how big it would be — with the
//! text of whichever row is selected underneath.
//!
//! The text is a read-only [`EditorView`], coloured by the *output* file's
//! language — a rendered Java template is Java, whatever the template it came
//! from was — so a preview scrolls, selects and copies exactly like the editor
//! a template tab holds. The panel's own scroll handle rides the dry run's file
//! list, which is the one thing here that is not the editor.

use std::path::PathBuf;

use gpui::{
    App, Context, DragMoveEvent, Entity, EventEmitter, FocusHandle, Focusable, ScrollHandle,
    SharedString, Window, div, prelude::*, px,
};
use rugpui::{
    DraggedThumb, Scrollbar, ScrollbarAxis, ScrollbarState, Select, hide_later, hide_now,
    scroll_to, scrolled, theme,
};
use rugpui_editor::EditorView;

use crate::app_settings;
use crate::i18n::ts;
use crate::template_pane::to_lf;

/// Element id of the panel's scrolling box.
const PANE_SCROLL: &str = "preview-scroll";

/// Element id of the panel's overlay scroll indicator.
const PANE_SCROLLBAR: &str = "preview-scrollbar";

/// Width of the two dropdowns in the preview header.
const CHOICE_WIDTH: f32 = 200.;

/// Height of the file list a dry run shows above the text.
const LIST_HEIGHT: f32 = 160.;

/// What the panel asks the shell for.
pub enum PreviewEvent {
    /// The header's dropdowns moved; render this pair instead.
    Reselect {
        /// Index into the ticked tables.
        table: usize,
        /// Index into the ticked templates.
        template: usize,
    },
}

/// Which of the two shapes the tab is showing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewKind {
    /// One pair, chosen in the header.
    Single,
    /// Every pair of the run, rendered to memory.
    DryRun,
}

/// One rendered file, as the panel shows it.
pub struct PreviewFile {
    /// Where it would be written.
    pub path: PathBuf,
    /// What would be written there.
    pub content: SharedString,
    /// Whether something is already at that path.
    pub exists: bool,
}

/// The Preview tab.
pub struct PreviewPane {
    focus_handle: FocusHandle,
    /// Which shape is on screen.
    kind: PreviewKind,
    /// The ticked tables, for the header's first dropdown.
    tables: Vec<SharedString>,
    /// The ticked templates, for the header's second dropdown.
    templates: Vec<SharedString>,
    /// Which table is being previewed.
    table_index: usize,
    /// Which template is being previewed.
    template_index: usize,
    /// Whether the table dropdown is showing.
    table_open: bool,
    /// Whether the template dropdown is showing.
    template_open: bool,
    /// What was rendered: one file for a preview, every file for a dry run.
    files: Vec<PreviewFile>,
    /// Which file's text is underneath the list.
    selected: usize,
    /// The engine's unknown-field warnings for the selected file.
    diagnostics: Vec<SharedString>,
    /// Why nothing could be rendered, when nothing could.
    error: Option<SharedString>,
    /// The rendered text, read-only.
    text: Entity<EditorView>,
    /// Vertical scroll of the text.
    scroll: ScrollHandle,
    /// Whether the panel's overlay scroll indicator is on screen.
    scrollbar: ScrollbarState,
}

impl PreviewPane {
    /// An empty panel, showing nothing.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            kind: PreviewKind::Single,
            tables: Vec::new(),
            templates: Vec::new(),
            table_index: 0,
            template_index: 0,
            table_open: false,
            template_open: false,
            files: Vec::new(),
            selected: 0,
            diagnostics: Vec::new(),
            error: None,
            text: cx.new(|cx| EditorView::new(cx).read_only(true)),
            scroll: ScrollHandle::new(),
            scrollbar: ScrollbarState::new(),
        }
    }

    /// What the tab is labelled.
    ///
    /// The file a preview would write, so two previews of different pairs are
    /// told apart on the strip; a dry run is labelled as one, because it is
    /// every file at once and no single name would do.
    pub fn title(&self) -> SharedString {
        match self.kind {
            PreviewKind::DryRun => ts!("generate.dry_run"),
            PreviewKind::Single => self
                .files
                .first()
                .and_then(|file| file.path.file_name())
                .map(|name| SharedString::from(name.to_string_lossy().into_owned()))
                .unwrap_or_else(|| ts!("generate.preview")),
        }
    }

    /// Which pair the header is pointing at.
    pub fn choice(&self) -> (usize, usize) {
        (self.table_index, self.template_index)
    }

    /// Replaces the lists the header chooses from.
    ///
    /// The indexes are clamped rather than reset: the run the preview belongs
    /// to may have grown a template since it was opened, and a preview that
    /// jumped back to the first table every time the list changed would be
    /// unusable while the list is being edited.
    pub fn set_choices(
        &mut self,
        tables: Vec<SharedString>,
        templates: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.table_index = self.table_index.min(tables.len().saturating_sub(1));
        self.template_index = self.template_index.min(templates.len().saturating_sub(1));
        self.tables = tables;
        self.templates = templates;
        cx.notify();
    }

    /// Shows one rendered pair.
    pub fn show_preview(
        &mut self,
        file: PreviewFile,
        diagnostics: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.kind = PreviewKind::Single;
        self.files = vec![file];
        self.selected = 0;
        self.diagnostics = diagnostics;
        self.error = None;
        self.show_selected(cx);
        cx.notify();
    }

    /// Shows a whole dry run.
    pub fn show_dry_run(
        &mut self,
        files: Vec<PreviewFile>,
        diagnostics: Vec<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.kind = PreviewKind::DryRun;
        self.files = files;
        self.selected = 0;
        self.diagnostics = diagnostics;
        self.error = None;
        self.show_selected(cx);
        cx.notify();
    }

    /// Shows why nothing could be rendered.
    pub fn show_error(&mut self, message: SharedString, cx: &mut Context<Self>) {
        self.files.clear();
        self.diagnostics.clear();
        self.error = Some(message);
        self.show_selected(cx);
        cx.notify();
    }

    /// Puts the selected file's text in the editor, coloured by what it is.
    ///
    /// The highlighter comes from the *output* file's extension: what is on
    /// screen is the Java, XML or PHP a template rendered to, and colouring it
    /// as a template would paint the one thing it no longer contains.
    fn show_selected(&mut self, cx: &mut Context<Self>) {
        let file = self.files.get(self.selected);
        let highlighter = file.and_then(|file| {
            file.path
                .extension()
                .and_then(|ext| rugpui_editor::highlighter_for_extension(&ext.to_string_lossy()))
        });
        let text = file.map(|file| to_lf(&file.content)).unwrap_or_default();
        self.text.update(cx, |editor, cx| {
            editor.set_highlighter(highlighter, cx);
            editor.set_text(&text, cx);
        });
    }

    /// Empties the panel; the connection it belonged to is gone.
    pub fn reset(&mut self, cx: &mut Context<Self>) {
        self.files.clear();
        self.diagnostics.clear();
        self.error = None;
        self.tables.clear();
        self.templates.clear();
        self.table_index = 0;
        self.template_index = 0;
        self.selected = 0;
        self.show_selected(cx);
        cx.notify();
    }

    /// How many files the panel is holding.
    #[cfg(test)]
    pub fn file_count(&self) -> usize {
        self.files.len()
    }

    /// The text currently under the list.
    #[cfg(test)]
    pub fn shown_text(&self) -> Option<&str> {
        self.files.get(self.selected).map(|file| &*file.content)
    }

    // --- the scroll bar ---------------------------------------------------

    /// The panel's overlay scroll indicator, as it now stands.
    fn bar(&self) -> Scrollbar {
        Scrollbar::for_handle(PANE_SCROLLBAR, ScrollbarAxis::Vertical, &self.scroll)
            .fade(self.scrollbar.fade())
    }

    /// Puts the bar up whenever the text has moved.
    fn watch_scroll(&mut self, cx: &mut Context<Self>) {
        let scrolled = scrolled(&self.scroll, ScrollbarAxis::Vertical);
        if let Some(epoch) = self.scrollbar.moved(scrolled) {
            hide_later(epoch, cx, move |pane: &mut Self| Some(&mut pane.scrollbar));
        }
    }

    /// Scrolls the text when its thumb is dragged.
    pub fn drag_scrollbar(&mut self, event: &DragMoveEvent<DraggedThumb>, cx: &mut Context<Self>) {
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

    /// The two dropdowns a single preview is chosen with.
    fn render_header(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let table = cx.entity();
        let table_toggle = cx.entity();
        let template = cx.entity();
        let template_toggle = cx.entity();
        let chosen_table = self.tables.get(self.table_index).cloned();
        let chosen_template = self.templates.get(self.template_index).cloned();

        div()
            .flex()
            .flex_row()
            .items_center()
            .gap(px(8.))
            .child(
                Select::new("preview-table")
                    .options(self.tables.clone())
                    .selected(chosen_table)
                    .placeholder(ts!("generate.preview_table"))
                    .open(self.table_open)
                    .width(px(CHOICE_WIDTH))
                    .on_select(move |index, _text, _window, cx| {
                        table.update(cx, |pane, cx| {
                            pane.table_open = false;
                            pane.table_index = index;
                            cx.emit(PreviewEvent::Reselect {
                                table: index,
                                template: pane.template_index,
                            });
                            cx.notify();
                        });
                    })
                    .on_open_change(move |open, _window, cx| {
                        table_toggle.update(cx, |pane, cx| {
                            pane.table_open = open;
                            cx.notify();
                        });
                    }),
            )
            .child(
                Select::new("preview-template")
                    .options(self.templates.clone())
                    .selected(chosen_template)
                    .placeholder(ts!("generate.preview_template"))
                    .open(self.template_open)
                    .width(px(CHOICE_WIDTH))
                    .on_select(move |index, _text, _window, cx| {
                        template.update(cx, |pane, cx| {
                            pane.template_open = false;
                            pane.template_index = index;
                            cx.emit(PreviewEvent::Reselect {
                                table: pane.table_index,
                                template: index,
                            });
                            cx.notify();
                        });
                    })
                    .on_open_change(move |open, _window, cx| {
                        template_toggle.update(cx, |pane, cx| {
                            pane.template_open = open;
                            cx.notify();
                        });
                    }),
            )
    }

    /// A dry run's file list: path, whether it would replace something, and how
    /// big it would be.
    fn render_file_list(&self, cx: &mut Context<Self>) -> impl IntoElement + use<> {
        let theme = theme(cx);
        let selected = self.selected;
        let rows: Vec<_> = self
            .files
            .iter()
            .enumerate()
            .map(|(index, file)| {
                let this = cx.entity();
                let active = index == selected;
                div()
                    .id(("preview-file", index))
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap(px(8.))
                    .px(px(6.))
                    .py(px(3.))
                    .rounded_md()
                    .cursor_pointer()
                    .when(active, |row| row.bg(theme.surface_hover))
                    .hover(|style| style.bg(theme.surface_hover))
                    .on_click(move |_, _window, cx| {
                        this.update(cx, |pane, cx| {
                            pane.selected = index;
                            pane.show_selected(cx);
                            cx.notify();
                        });
                    })
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .text_size(px(11.))
                            .text_color(theme.text)
                            .child(SharedString::from(file.path.display().to_string())),
                    )
                    .child(
                        div()
                            .flex_none()
                            .text_size(px(11.))
                            .text_color(if file.exists {
                                theme.danger
                            } else {
                                theme.text_muted
                            })
                            .child(if file.exists {
                                ts!("generate.dry_exists")
                            } else {
                                ts!("generate.dry_new")
                            }),
                    )
                    .child(
                        div()
                            .flex_none()
                            .w(px(70.))
                            .text_size(px(11.))
                            .text_color(theme.text_muted)
                            .child(ts!("generate.dry_bytes", bytes = file.content.len())),
                    )
            })
            .collect();

        div()
            .id(PANE_SCROLL)
            .track_scroll(&self.scroll)
            .flex()
            .flex_col()
            .gap(px(1.))
            .h(px(LIST_HEIGHT))
            .p(px(4.))
            .overflow_y_scroll()
            .rounded_md()
            .bg(theme.surface)
            .border_1()
            .border_color(theme.border)
            .children(rows)
    }
}

impl EventEmitter<PreviewEvent> for PreviewPane {}

impl Focusable for PreviewPane {
    fn focus_handle(&self, _cx: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for PreviewPane {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        self.watch_scroll(cx);
        let mono = app_settings::editor_font(cx);
        let font_size = app_settings::effective(cx).editor_font_size;

        let header = match self.kind {
            PreviewKind::Single => Some(self.render_header(cx).into_any_element()),
            PreviewKind::DryRun => {
                let bar = self
                    .bar()
                    .on_hover(cx.listener(|pane, hovered: &bool, _window, cx| {
                        pane.hover_scrollbar(*hovered, cx);
                    }));
                Some(
                    // The bar hangs off this wrapper rather than the pane root:
                    // the thumb is measured against the scrolling box, which is
                    // the file list alone — hung off the root it would ride over
                    // the diagnostics and the text below it and never reach the
                    // list's own bottom.
                    div()
                        .relative()
                        .flex()
                        .flex_col()
                        .flex_none()
                        .child(self.render_file_list(cx))
                        .children(bar.render(&theme))
                        .into_any_element(),
                )
            }
        };

        let diagnostics = (!self.diagnostics.is_empty()).then(|| {
            div()
                .flex()
                .flex_col()
                .gap(px(2.))
                .p(px(8.))
                .rounded_md()
                .bg(theme.surface)
                .border_1()
                .border_color(theme.accent)
                .text_size(px(11.))
                .text_color(theme.text_muted)
                .child(
                    div()
                        .text_color(theme.accent)
                        .child(ts!("generate.diagnostics", count = self.diagnostics.len())),
                )
                .children(
                    self.diagnostics
                        .iter()
                        .map(|line| div().child(line.clone())),
                )
        });

        let body = match (&self.error, self.files.get(self.selected)) {
            (Some(message), _) => div()
                .p(px(12.))
                .text_size(px(12.))
                .text_color(theme.danger)
                .child(message.clone())
                .into_any_element(),
            (None, Some(file)) => div()
                .flex()
                .flex_col()
                .flex_grow_1()
                .min_h_0()
                .gap(px(6.))
                .child(
                    div()
                        .px(px(2.))
                        .text_size(px(11.))
                        .text_color(theme.text_muted)
                        .child(SharedString::from(file.path.display().to_string())),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .flex_grow_1()
                        .min_h_0()
                        .rounded_md()
                        .overflow_hidden()
                        .border_1()
                        .border_color(theme.border)
                        .font_family(mono)
                        .text_size(px(font_size))
                        .child(self.text.clone()),
                )
                .into_any_element(),
            (None, None) => div()
                .p(px(12.))
                .text_size(px(12.))
                .text_color(theme.text_muted)
                .child(ts!("generate.preview_empty"))
                .into_any_element(),
        };

        div()
            .key_context("PreviewPane")
            .track_focus(&self.focus_handle)
            .flex()
            .flex_col()
            .flex_grow_1()
            .min_h_0()
            .gap(px(8.))
            .p(px(12.))
            .children(header)
            .children(diagnostics)
            .child(body)
    }
}
