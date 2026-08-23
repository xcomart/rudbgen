//! A reusable text field: one line by default, several on request.
//!
//! The implementation is derived from the `input.rs` example shipped with gpui
//! 0.2.2 and extended with the features rudbgen needs: a placeholder, password
//! masking, a disabled state, an `Enter` submit callback and a
//! [`multiline`](TextInput::multiline) mode.
//!
//! All offsets stored in [`TextInput`] are byte offsets into the *real*
//! content. When the field is masked the rendered string is a different byte
//! sequence, so a [`DisplayMap`] translates between the two spaces; that keeps
//! the caret and selection correct for multi-byte text such as Hangul or emoji.
//!
//! # The two modes are one field
//!
//! A multiline field is the same entity with the same offsets; what changes is
//! the geometry. One line is shaped by [`shape_line`] and answers `x_for_index`
//! directly; several are shaped by [`shape_text`], which splits on `\n` and
//! wraps each logical line to the width it was measured at, so every caret
//! question becomes "which row, and where along it". [`TextInput::point_for`]
//! and [`TextInput::offset_for`] are the two directions of that, and every
//! caret movement, mouse press and selection rectangle goes through them —
//! which is what keeps wrapped text, `Up`/`Down` and click-to-place agreeing
//! with one another.
//!
//! [`shape_line`]: gpui::WindowTextSystem::shape_line
//! [`shape_text`]: gpui::WindowTextSystem::shape_text
//!
//! What it is not: a code editor. There is no undo, no syntax highlighting and
//! no gutter — the custom-query fields of the driver editor are what the mode
//! exists for, and `rudbgen-editor` is what replaces it there once the
//! template editor brings a real one.

use std::cell::RefCell;
use std::ops::Range;
use std::rc::Rc;

use gpui::{
    App, AvailableSpace, Bounds, ClipboardItem, Context, CursorStyle, ElementId,
    ElementInputHandler, Entity, EntityInputHandler, FocusHandle, Focusable, GlobalElementId,
    InspectorElementId, KeyBinding, LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent,
    MouseUpEvent, PaintQuad, Pixels, Point, ScrollHandle, ShapedLine, SharedString, Size, Style,
    TextAlign, TextRun, UTF16Selection, UnderlineStyle, Window, WrappedLine, actions, div, fill,
    point, prelude::*, px, relative, size,
};
use unicode_segmentation::UnicodeSegmentation;

use crate::theme::theme;

actions!(
    rudbgen_input,
    [
        /// Delete the grapheme before the caret, or the selection.
        Backspace,
        /// Delete the grapheme after the caret, or the selection.
        Delete,
        /// Move the caret one grapheme to the left.
        Left,
        /// Move the caret one grapheme to the right.
        Right,
        /// Extend the selection one grapheme to the left.
        SelectLeft,
        /// Extend the selection one grapheme to the right.
        SelectRight,
        /// Select the whole content.
        SelectAll,
        /// Move the caret to the start of the field.
        Home,
        /// Move the caret to the end of the field.
        End,
        /// Extend the selection to the start of the field.
        SelectHome,
        /// Extend the selection to the end of the field.
        SelectEnd,
        /// Open the macOS emoji / character palette.
        ShowCharacterPalette,
        /// Insert the clipboard contents.
        Paste,
        /// Copy the selection to the clipboard.
        Copy,
        /// Copy the selection to the clipboard and delete it.
        Cut,
        /// Confirm the current value, invoking the submit callback — or, in a
        /// multiline field, break the line.
        Submit,
        /// Move the caret to the row above.
        Up,
        /// Move the caret to the row below.
        Down,
        /// Extend the selection to the row above.
        SelectUp,
        /// Extend the selection to the row below.
        SelectDown,
    ]
);

/// Key context that [`TextInput::init`] binds its keys to.
const KEY_CONTEXT: &str = "TextInput";

/// Character substituted for every grapheme when the field is masked.
const MASK_CHAR: char = '\u{2022}';

/// Callback invoked when the user presses `Enter` inside a [`TextInput`].
type SubmitHandler = Rc<dyn Fn(&str, &mut Window, &mut App)>;

/// Translates byte offsets between the real content and the rendered string.
///
/// Only built for masked fields; unmasked fields render their content verbatim
/// and therefore use identity mapping.
#[derive(Clone, Debug)]
struct DisplayMap {
    /// `(content offset, display offset)` at every grapheme boundary, including
    /// `0` and the end of the string. Sorted ascending on both components.
    boundaries: Vec<(usize, usize)>,
}

impl DisplayMap {
    /// Maps an offset in the real content to the equivalent display offset.
    fn to_display(&self, content_offset: usize) -> usize {
        match self
            .boundaries
            .binary_search_by_key(&content_offset, |(content, _)| *content)
        {
            Ok(ix) => self.boundaries[ix].1,
            Err(ix) => self
                .boundaries
                .get(ix.saturating_sub(1))
                .map_or(0, |(_, display)| *display),
        }
    }

    /// Maps an offset in the rendered string back to the real content.
    fn to_content(&self, display_offset: usize) -> usize {
        match self
            .boundaries
            .binary_search_by_key(&display_offset, |(_, display)| *display)
        {
            Ok(ix) => self.boundaries[ix].0,
            Err(ix) => self
                .boundaries
                .get(ix.saturating_sub(1))
                .map_or(0, |(content, _)| *content),
        }
    }
}

/// Maps `offset` through `map`, or returns it unchanged when there is no map.
fn to_display(map: Option<&DisplayMap>, offset: usize) -> usize {
    map.map_or(offset, |map| map.to_display(offset))
}

/// What the last frame shaped, kept so that the pointer and the arrow keys can
/// ask where a byte offset is without waiting for the next one.
///
/// The two variants are the two modes, and they are not interchangeable: a
/// single line is one [`ShapedLine`] whose `x_for_index` is the whole of the
/// geometry, whereas a multiline field is one [`WrappedLine`] per *logical*
/// line — the ones `\n` separates — each of which may occupy several rows
/// once it has been wrapped to the field's width.
enum Shaped {
    /// One line, shaped without wrapping.
    ///
    /// Boxed because a [`ShapedLine`] is three kilobytes of glyph runs and the
    /// other variant is a pointer; an unboxed enum would make every field pay
    /// the larger of the two whichever mode it is in.
    Line(Box<ShapedLine>),
    /// One entry per logical line, in order, each wrapped to the field's width.
    Lines(Vec<WrappedLine>),
}

/// The shaped multiline text, handed from the measure closure to `prepaint` and
/// on to `paint`.
///
/// A cell rather than a return value because gpui measures through a callback:
/// [`Window::request_measured_layout`] hands the closure the width to wrap at
/// and takes back a size, and this is where what it shaped on the way is left
/// for the two passes that follow.
type ShapedCell = Rc<RefCell<Option<Vec<WrappedLine>>>>;

/// Byte offset of the start of every logical line of `text`, and of its end.
///
/// `len + 1` entries for `len` lines, so that line `i` covers
/// `starts[i]..starts[i + 1] - 1` — the `- 1` being the `\n` that ended it,
/// which belongs to no row. The last entry is the length of the text, which is
/// what makes the final line's range come out right without a special case.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(index, _)| index + 1));
    starts.push(text.len() + 1);
    starts
}

/// A single-line, focusable text field rendered as a gpui entity.
///
/// Create one with [`Context::new`](gpui::App::new) and keep the returned
/// [`Entity`] around; rendering it is as simple as passing the entity as a
/// child element.
///
/// ```ignore
/// let host = cx.new(|cx| TextInput::new(cx).placeholder("example.com"));
/// ```
pub struct TextInput {
    focus_handle: FocusHandle,
    content: SharedString,
    placeholder: SharedString,
    selected_range: Range<usize>,
    selection_reversed: bool,
    marked_range: Option<Range<usize>>,
    last_layout: Option<Shaped>,
    last_bounds: Option<Bounds<Pixels>>,
    last_display_map: Option<DisplayMap>,
    /// Row height of the last frame, which is what turns a `Up` into a point
    /// one row above the caret.
    last_line_height: Pixels,
    is_selecting: bool,
    masked: bool,
    disabled: bool,
    invalid: bool,
    /// Rows a multiline field is tall; `None` for the single-line field.
    rows: Option<usize>,
    /// Vertical scroll of a multiline field. Unused by the single-line one.
    scroll: ScrollHandle,
    on_submit: Option<SubmitHandler>,
}

impl TextInput {
    /// Creates an empty text field owned by `cx`.
    pub fn new(cx: &mut Context<Self>) -> Self {
        Self {
            focus_handle: cx.focus_handle(),
            content: SharedString::default(),
            placeholder: SharedString::default(),
            selected_range: 0..0,
            selection_reversed: false,
            marked_range: None,
            last_layout: None,
            last_bounds: None,
            last_display_map: None,
            last_line_height: px(20.),
            is_selecting: false,
            masked: false,
            disabled: false,
            invalid: false,
            rows: None,
            scroll: ScrollHandle::new(),
            on_submit: None,
        }
    }

    /// Registers the key bindings every `TextInput` relies on.
    ///
    /// Call once during application start-up. Bindings are scoped to the
    /// `TextInput` key context so they never leak into the rest of the app, and
    /// the clipboard / select-all chords follow platform conventions (`cmd` on
    /// macOS, `ctrl` elsewhere).
    pub fn init(cx: &mut App) {
        let modifier = if cfg!(target_os = "macos") {
            "cmd"
        } else {
            "ctrl"
        };

        let mut bindings = vec![
            KeyBinding::new("backspace", Backspace, Some(KEY_CONTEXT)),
            KeyBinding::new("delete", Delete, Some(KEY_CONTEXT)),
            KeyBinding::new("left", Left, Some(KEY_CONTEXT)),
            KeyBinding::new("right", Right, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-left", SelectLeft, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-right", SelectRight, Some(KEY_CONTEXT)),
            KeyBinding::new("home", Home, Some(KEY_CONTEXT)),
            KeyBinding::new("end", End, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-home", SelectHome, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-end", SelectEnd, Some(KEY_CONTEXT)),
            KeyBinding::new("enter", Submit, Some(KEY_CONTEXT)),
            KeyBinding::new("up", Up, Some(KEY_CONTEXT)),
            KeyBinding::new("down", Down, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-up", SelectUp, Some(KEY_CONTEXT)),
            KeyBinding::new("shift-down", SelectDown, Some(KEY_CONTEXT)),
            KeyBinding::new(&format!("{modifier}-a"), SelectAll, Some(KEY_CONTEXT)),
            KeyBinding::new(&format!("{modifier}-c"), Copy, Some(KEY_CONTEXT)),
            KeyBinding::new(&format!("{modifier}-v"), Paste, Some(KEY_CONTEXT)),
            KeyBinding::new(&format!("{modifier}-x"), Cut, Some(KEY_CONTEXT)),
        ];

        if cfg!(target_os = "macos") {
            bindings.push(KeyBinding::new(
                "ctrl-cmd-space",
                ShowCharacterPalette,
                Some(KEY_CONTEXT),
            ));
        }

        cx.bind_keys(bindings);
    }

    /// Sets the text shown while the field is empty.
    pub fn placeholder(mut self, placeholder: impl Into<SharedString>) -> Self {
        self.placeholder = placeholder.into();
        self
    }

    /// Replaces the text shown while the field is empty.
    ///
    /// The builder above covers a hint that is fixed for the life of the field.
    /// This is for the ones that have to follow a language switch, since the
    /// field entity outlives the locale it was created under.
    pub fn set_placeholder(
        &mut self,
        placeholder: impl Into<SharedString>,
        cx: &mut Context<Self>,
    ) {
        self.placeholder = placeholder.into();
        cx.notify();
    }

    /// Renders every grapheme as a bullet, for password entry.
    ///
    /// The stored content is untouched; only the rendered string is masked.
    /// Copy and cut are disabled while masked so secrets cannot leak into the
    /// clipboard.
    pub fn masked(mut self, masked: bool) -> Self {
        self.masked = masked;
        self
    }

    /// Makes the field `rows` rows tall, with `Enter` breaking the line
    /// instead of submitting.
    ///
    /// The height is what the field *shows*: longer text scrolls inside it
    /// rather than pushing the form apart, which is what keeps a dialog whose
    /// four SQL fields hold anything from one line to thirty the same shape.
    /// A masked multiline field is a contradiction and is not offered; the mask
    /// is ignored if one is asked for anyway.
    pub fn multiline(mut self, rows: usize) -> Self {
        self.rows = Some(rows.max(1));
        self
    }

    /// Whether this field breaks lines rather than submitting on `Enter`.
    pub fn is_multiline(&self) -> bool {
        self.rows.is_some()
    }

    /// Makes the field read-only and visually muted.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.disabled = disabled;
        self
    }

    /// Marks the content as refused, outlining the field in the danger color.
    ///
    /// The field itself has no notion of what a valid value is — only its owner
    /// does — so this is a setter rather than a builder: whoever is checking
    /// the content keeps the flag in step with it. The outline wins over the
    /// focus ring, since a field one is typing into is exactly the field whose
    /// refusal has to stay visible. Setting the flag to the value it already
    /// holds is a no-op, which is what keeps an observer that sets it from
    /// waking itself up again.
    pub fn set_invalid(&mut self, invalid: bool, cx: &mut Context<Self>) {
        if self.invalid != invalid {
            self.invalid = invalid;
            cx.notify();
        }
    }

    /// Places the field at `index` in the window's tab order and makes it a tab
    /// stop.
    ///
    /// Fields without an explicit index stay out of the tab ring entirely, which
    /// is what keeps `Tab` from wandering into views that never opted in.
    pub fn tab_index(mut self, index: isize) -> Self {
        self.focus_handle = self.focus_handle.clone().tab_index(index).tab_stop(true);
        self
    }

    /// Sets the callback invoked when the user presses `Enter`.
    pub fn on_submit(mut self, handler: impl Fn(&str, &mut Window, &mut App) + 'static) -> Self {
        self.on_submit = Some(Rc::new(handler));
        self
    }

    /// The current value of the field.
    pub fn content(&self) -> &str {
        &self.content
    }

    /// Replaces the value, collapsing the caret to the end of the new text.
    pub fn set_content(&mut self, text: impl Into<SharedString>, cx: &mut Context<Self>) {
        self.content = text.into();
        let end = self.content.len();
        self.selected_range = end..end;
        self.selection_reversed = false;
        self.marked_range = None;
        cx.notify();
    }

    /// Clears the value and the selection.
    pub fn clear(&mut self, cx: &mut Context<Self>) {
        self.set_content(SharedString::default(), cx);
    }

    fn left(&mut self, _: &Left, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.previous_boundary(self.cursor_offset()), cx);
        } else {
            self.move_to(self.selected_range.start, cx);
        }
    }

    fn right(&mut self, _: &Right, _: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.move_to(self.next_boundary(self.selected_range.end), cx);
        } else {
            self.move_to(self.selected_range.end, cx);
        }
    }

    fn select_left(&mut self, _: &SelectLeft, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.previous_boundary(self.cursor_offset()), cx);
    }

    fn select_right(&mut self, _: &SelectRight, _: &mut Window, cx: &mut Context<Self>) {
        self.select_to(self.next_boundary(self.cursor_offset()), cx);
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        self.move_to(0, cx);
        self.select_to(self.content.len(), cx);
    }

    fn home(&mut self, _: &Home, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_start();
        self.move_to(offset, cx);
    }

    fn end(&mut self, _: &End, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_end();
        self.move_to(offset, cx);
    }

    fn select_home(&mut self, _: &SelectHome, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_start();
        self.select_to(offset, cx);
    }

    fn select_end(&mut self, _: &SelectEnd, _: &mut Window, cx: &mut Context<Self>) {
        let offset = self.row_end();
        self.select_to(offset, cx);
    }

    /// Start of the logical line the caret is on — the whole field on one line.
    fn row_start(&self) -> usize {
        if !self.is_multiline() {
            return 0;
        }
        self.content[..self.cursor_offset()]
            .rfind('\n')
            .map_or(0, |index| index + 1)
    }

    /// End of the logical line the caret is on — the whole field on one line.
    fn row_end(&self) -> usize {
        if !self.is_multiline() {
            return self.content.len();
        }
        let cursor = self.cursor_offset();
        self.content[cursor..]
            .find('\n')
            .map_or(self.content.len(), |index| cursor + index)
    }

    /// Moves the caret one row up or down, keeping the column it was in.
    ///
    /// The column is a *pixel* column rather than a byte one, which is what
    /// makes the movement land where the caret looks like it should over
    /// proportional text and over a row that was wrapped rather than typed. A
    /// field that has not been painted yet has no geometry to ask, so the
    /// movement falls back to the ends of the content — the answer `Up` and
    /// `Down` would give on a single line anyway.
    fn move_row(&mut self, down: bool, extend: bool, cx: &mut Context<Self>) {
        let cursor = self.cursor_offset();
        let offset = match self.point_for(cursor) {
            Some(point) => {
                let line_height = self.last_line_height;
                // Past the first row or the last one, the caret goes to the end
                // it was heading for rather than staying put: that is what
                // every other field does, and staying put reads as a key that
                // did not arrive.
                let last_row = self
                    .point_for(self.content.len())
                    .is_some_and(|end| (end.y - point.y).abs() < px(0.5));
                if down && last_row {
                    self.content.len()
                } else if !down && point.y < line_height / 2. {
                    0
                } else {
                    let step = if down { line_height } else { -line_height };
                    self.offset_for(gpui::point(point.x, point.y + step))
                }
            }
            None if down => self.content.len(),
            None => 0,
        };
        if extend {
            self.select_to(offset, cx);
        } else {
            self.move_to(offset, cx);
        }
    }

    fn up(&mut self, _: &Up, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_multiline() {
            cx.propagate();
            return;
        }
        self.move_row(false, false, cx);
    }

    fn down(&mut self, _: &Down, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_multiline() {
            cx.propagate();
            return;
        }
        self.move_row(true, false, cx);
    }

    fn select_up(&mut self, _: &SelectUp, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_multiline() {
            cx.propagate();
            return;
        }
        self.move_row(false, true, cx);
    }

    fn select_down(&mut self, _: &SelectDown, _: &mut Window, cx: &mut Context<Self>) {
        if !self.is_multiline() {
            cx.propagate();
            return;
        }
        self.move_row(true, true, cx);
    }

    fn backspace(&mut self, _: &Backspace, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.previous_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn delete(&mut self, _: &Delete, window: &mut Window, cx: &mut Context<Self>) {
        if self.selected_range.is_empty() {
            self.select_to(self.next_boundary(self.cursor_offset()), cx);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn submit(&mut self, _: &Submit, window: &mut Window, cx: &mut Context<Self>) {
        // In a multiline field `Enter` is a line break and nothing else: a
        // field one writes a statement into has no single value to confirm,
        // and a submit that fired on every new line would be unusable.
        if self.is_multiline() {
            self.replace_text_in_range(None, "\n", window, cx);
            return;
        }
        if let Some(handler) = self.on_submit.clone() {
            let content = self.content.clone();
            handler(&content, window, cx);
        }
    }

    fn show_character_palette(
        &mut self,
        _: &ShowCharacterPalette,
        window: &mut Window,
        _: &mut Context<Self>,
    ) {
        window.show_character_palette();
    }

    /// Inserts the clipboard contents, flattened to a single line.
    ///
    /// Read asynchronously rather than synchronously: the clipboard belongs to
    /// whichever application last wrote to it, and on Wayland asking that
    /// application for the bytes is a round trip that can stall for as long as
    /// the owner takes to answer. Awaiting it keeps the field responsive while
    /// the answer is on its way.
    fn paste(&mut self, _: &Paste, window: &mut Window, cx: &mut Context<Self>) {
        let read = cx.read_from_clipboard_async();
        cx.spawn_in(window, async move |this, cx| {
            let text = match read.await {
                Ok(item) => item.and_then(|item| item.text()),
                Err(error) => {
                    log::warn!("clipboard read failed: {error}");
                    return;
                }
            };
            let Some(text) = text else {
                return;
            };
            // A field torn down while the read was in flight drops the paste.
            this.update_in(cx, |this, window, cx| {
                // A single-line field flattens what it is given; a multiline
                // one keeps it, which is the whole point of pasting a statement
                // into it.
                let text = if this.is_multiline() {
                    text.replace("\r\n", "\n")
                } else {
                    text.replace(['\n', '\r'], " ")
                };
                this.replace_text_in_range(None, &text, window, cx);
            })
            .ok();
        })
        .detach();
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if self.masked || self.selected_range.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.content[self.selected_range.clone()].to_string(),
        ));
    }

    fn cut(&mut self, _: &Cut, window: &mut Window, cx: &mut Context<Self>) {
        if self.masked || self.selected_range.is_empty() {
            return;
        }
        cx.write_to_clipboard(ClipboardItem::new_string(
            self.content[self.selected_range.clone()].to_string(),
        ));
        self.replace_text_in_range(None, "", window, cx);
    }

    fn on_mouse_down(&mut self, event: &MouseDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        self.is_selecting = true;
        if event.modifiers.shift {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        } else {
            self.move_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn on_mouse_up(&mut self, _: &MouseUpEvent, _: &mut Window, _: &mut Context<Self>) {
        self.is_selecting = false;
    }

    fn on_mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        if self.is_selecting {
            self.select_to(self.index_for_mouse_position(event.position), cx);
        }
    }

    fn move_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        self.selected_range = offset..offset;
        cx.notify();
    }

    fn cursor_offset(&self) -> usize {
        if self.selection_reversed {
            self.selected_range.start
        } else {
            self.selected_range.end
        }
    }

    fn index_for_mouse_position(&self, position: Point<Pixels>) -> usize {
        if self.content.is_empty() {
            return 0;
        }

        let Some(bounds) = self.last_bounds.as_ref() else {
            return 0;
        };
        // `bounds` is where the text was *painted*, which in a scrolled
        // multiline field is already shifted by the scroll offset — so the
        // subtraction below needs no scroll term of its own.
        self.offset_for(position - bounds.origin)
    }

    /// Where the caret sits at `offset`, relative to the top left of the text.
    ///
    /// `None` until the field has been painted once: there is no geometry to
    /// answer from before that, and guessing one would put the caret somewhere
    /// the next frame moves it away from.
    fn point_for(&self, offset: usize) -> Option<Point<Pixels>> {
        let line_height = self.last_line_height;
        match self.last_layout.as_ref()? {
            Shaped::Line(line) => {
                let display = to_display(self.last_display_map.as_ref(), offset);
                Some(point(line.x_for_index(display), Pixels::ZERO))
            }
            Shaped::Lines(lines) => {
                let starts = line_starts(&self.content);
                let mut top = Pixels::ZERO;
                for (index, line) in lines.iter().enumerate() {
                    let start = *starts.get(index)?;
                    let end = starts.get(index + 1).map_or(self.content.len(), |next| {
                        next.saturating_sub(1).min(self.content.len())
                    });
                    if offset <= end {
                        let within = offset.saturating_sub(start).min(line.len());
                        let position = line.position_for_index(within, line_height)?;
                        return Some(point(position.x, top + position.y));
                    }
                    top += line.size(line_height).height;
                }
                // Past the last line: the very end of the text.
                let last = lines.last()?;
                let position = last.position_for_index(last.len(), line_height)?;
                let height = last.size(line_height).height;
                Some(point(position.x, top - height + position.y))
            }
        }
    }

    /// The offset nearest `position`, which is relative to the top left of the
    /// text.
    fn offset_for(&self, position: Point<Pixels>) -> usize {
        let line_height = self.last_line_height;
        let map = self.last_display_map.as_ref();
        match self.last_layout.as_ref() {
            None => 0,
            Some(Shaped::Line(line)) => {
                let display = line.closest_index_for_x(position.x);
                map.map_or(display, |map| map.to_content(display))
            }
            Some(Shaped::Lines(lines)) => {
                let starts = line_starts(&self.content);
                let mut top = Pixels::ZERO;
                for (index, line) in lines.iter().enumerate() {
                    let height = line.size(line_height).height;
                    let start = starts.get(index).copied().unwrap_or_default();
                    let last = index + 1 == lines.len();
                    if position.y < top + height || last {
                        // Clamped into the line's own rows: a press below the
                        // last one is a press at the end of it, and an
                        // unclamped `y` asks the layout about a row it has not
                        // got, which answers "offset zero".
                        let within = point(
                            position.x,
                            (position.y - top).clamp(Pixels::ZERO, height - px(1.)),
                        );
                        let offset = match line.closest_index_for_position(within, line_height) {
                            Ok(offset) | Err(offset) => offset,
                        };
                        return (start + offset.min(line.len())).min(self.content.len());
                    }
                    top += height;
                }
                self.content.len()
            }
        }
    }

    fn select_to(&mut self, offset: usize, cx: &mut Context<Self>) {
        if self.selection_reversed {
            self.selected_range.start = offset;
        } else {
            self.selected_range.end = offset;
        }
        if self.selected_range.end < self.selected_range.start {
            self.selection_reversed = !self.selection_reversed;
            self.selected_range = self.selected_range.end..self.selected_range.start;
        }
        cx.notify();
    }

    fn offset_from_utf16(&self, offset: usize) -> usize {
        let mut utf8_offset = 0;
        let mut utf16_count = 0;

        for ch in self.content.chars() {
            if utf16_count >= offset {
                break;
            }
            utf16_count += ch.len_utf16();
            utf8_offset += ch.len_utf8();
        }

        utf8_offset
    }

    fn offset_to_utf16(&self, offset: usize) -> usize {
        let mut utf16_offset = 0;
        let mut utf8_count = 0;

        for ch in self.content.chars() {
            if utf8_count >= offset {
                break;
            }
            utf8_count += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        }

        utf16_offset
    }

    fn range_to_utf16(&self, range: &Range<usize>) -> Range<usize> {
        self.offset_to_utf16(range.start)..self.offset_to_utf16(range.end)
    }

    fn range_from_utf16(&self, range_utf16: &Range<usize>) -> Range<usize> {
        self.offset_from_utf16(range_utf16.start)..self.offset_from_utf16(range_utf16.end)
    }

    fn previous_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .rev()
            .find_map(|(idx, _)| (idx < offset).then_some(idx))
            .unwrap_or(0)
    }

    fn next_boundary(&self, offset: usize) -> usize {
        self.content
            .grapheme_indices(true)
            .find_map(|(idx, _)| (idx > offset).then_some(idx))
            .unwrap_or(self.content.len())
    }

    /// Builds the string that is actually shaped, plus the offset map needed to
    /// place the caret when the content is masked.
    fn display_text(&self) -> (SharedString, Option<DisplayMap>) {
        if !self.masked || self.content.is_empty() {
            // A single line is shaped by `shape_line`, which *panics* on a
            // newline. Content with one in it is not a bug — a value loaded
            // from a file, a paste that arrived before the field was made
            // single-line — so the character is shown as the space it reads as.
            // One byte for one byte, so every offset in the field still points
            // at the same place.
            if self.rows.is_none() && self.content.contains(['\n', '\r']) {
                return (
                    SharedString::from(self.content.replace(['\n', '\r'], " ")),
                    None,
                );
            }
            return (self.content.clone(), None);
        }

        let mut display = String::with_capacity(self.content.len());
        let mut boundaries = Vec::new();
        for (offset, _) in self.content.grapheme_indices(true) {
            boundaries.push((offset, display.len()));
            display.push(MASK_CHAR);
        }
        boundaries.push((self.content.len(), display.len()));

        (SharedString::from(display), Some(DisplayMap { boundaries }))
    }
}

impl EntityInputHandler for TextInput {
    fn text_for_range(
        &mut self,
        range_utf16: Range<usize>,
        actual_range: &mut Option<Range<usize>>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(&range_utf16);
        actual_range.replace(self.range_to_utf16(&range));
        Some(self.content[range].to_string())
    }

    fn selected_text_range(
        &mut self,
        _ignore_disabled_input: bool,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.range_to_utf16(&self.selected_range),
            reversed: self.selection_reversed,
        })
    }

    fn marked_text_range(
        &self,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Range<usize>> {
        self.marked_range
            .as_ref()
            .map(|range| self.range_to_utf16(range))
    }

    fn unmark_text(&mut self, _window: &mut Window, _cx: &mut Context<Self>) {
        self.marked_range = None;
    }

    fn replace_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        self.selected_range = range.start + new_text.len()..range.start + new_text.len();
        self.marked_range.take();
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range_utf16: Option<Range<usize>>,
        new_text: &str,
        new_selected_range_utf16: Option<Range<usize>>,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.disabled {
            return;
        }

        let range = range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .or(self.marked_range.clone())
            .unwrap_or(self.selected_range.clone());

        self.content =
            (self.content[0..range.start].to_owned() + new_text + &self.content[range.end..])
                .into();
        if new_text.is_empty() {
            self.marked_range = None;
        } else {
            self.marked_range = Some(range.start..range.start + new_text.len());
        }
        self.selected_range = new_selected_range_utf16
            .as_ref()
            .map(|range_utf16| self.range_from_utf16(range_utf16))
            .map(|new_range| new_range.start + range.start..new_range.end + range.end)
            .unwrap_or_else(|| range.start + new_text.len()..range.start + new_text.len());

        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range_utf16: Range<usize>,
        bounds: Bounds<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        // Where the input method should put its candidate window: the box the
        // marked range occupies. Over several rows the two ends are on
        // different ones, and a box drawn between them would cover the text in
        // between, so the range is reported as the row the *start* sits on.
        let range = self.range_from_utf16(&range_utf16);
        let start = self.point_for(range.start)?;
        let end = self.point_for(range.end)?;
        let line_height = self.last_line_height;
        let same_row = (end.y - start.y).abs() < px(0.5);
        Some(Bounds::from_corners(
            bounds.origin + start,
            bounds.origin
                + point(
                    if same_row { end.x } else { bounds.size.width },
                    start.y + line_height,
                ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _window: &mut Window,
        _cx: &mut Context<Self>,
    ) -> Option<usize> {
        let bounds = self.last_bounds?;
        Some(self.offset_to_utf16(self.offset_for(point - bounds.origin)))
    }
}

impl Focusable for TextInput {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus_handle.clone()
    }
}

impl Render for TextInput {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = theme(cx);
        let focused = !self.disabled && self.focus_handle.is_focused(window);
        let disabled = self.disabled;

        let rows = self.rows;
        // A single line is 32 pixels tall: the 20-pixel line box with six
        // either side. A multiline field keeps the same padding and grows by
        // whole rows, so a two-row field and two one-row fields stack to the
        // same height.
        let height = rows.map_or(px(32.), |rows| px(12.) + px(20.) * rows as f32);

        div()
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus_handle)
            .flex()
            .when(rows.is_none(), |this| this.items_center())
            .w_full()
            .h(height)
            .px(px(8.))
            .when(rows.is_some(), |this| this.py(px(6.)))
            .rounded_md()
            .overflow_hidden()
            .border_1()
            .border_color(match (self.invalid, focused) {
                (true, _) => theme.danger,
                (false, true) => theme.accent,
                (false, false) => theme.border,
            })
            .bg(if disabled {
                theme.surface.opacity(0.6)
            } else {
                theme.surface
            })
            .text_size(px(14.))
            .line_height(px(20.))
            .cursor(if disabled {
                CursorStyle::Arrow
            } else {
                CursorStyle::IBeam
            })
            .when(!disabled, |this| {
                this.on_action(cx.listener(Self::backspace))
                    .on_action(cx.listener(Self::delete))
                    .on_action(cx.listener(Self::left))
                    .on_action(cx.listener(Self::right))
                    .on_action(cx.listener(Self::select_left))
                    .on_action(cx.listener(Self::select_right))
                    .on_action(cx.listener(Self::select_all))
                    .on_action(cx.listener(Self::home))
                    .on_action(cx.listener(Self::end))
                    .on_action(cx.listener(Self::select_home))
                    .on_action(cx.listener(Self::select_end))
                    .on_action(cx.listener(Self::submit))
                    .on_action(cx.listener(Self::show_character_palette))
                    .on_action(cx.listener(Self::paste))
                    .on_action(cx.listener(Self::copy))
                    .on_action(cx.listener(Self::cut))
                    .on_mouse_down(MouseButton::Left, cx.listener(Self::on_mouse_down))
                    .on_mouse_up(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_up_out(MouseButton::Left, cx.listener(Self::on_mouse_up))
                    .on_mouse_move(cx.listener(Self::on_mouse_move))
                    .on_action(cx.listener(Self::up))
                    .on_action(cx.listener(Self::down))
                    .on_action(cx.listener(Self::select_up))
                    .on_action(cx.listener(Self::select_down))
            })
            .child(match rows {
                // The scrolling box is inside the frame rather than around it,
                // so the border stays where the field is and the text moves
                // under it. `track_scroll` is deliberately absent: the box
                // scrolls with the wheel, and the caret is kept in view by the
                // element asking gpui to scroll to it.
                Some(_) => div()
                    .id("multiline")
                    .size_full()
                    .track_scroll(&self.scroll)
                    .overflow_y_scroll()
                    .child(TextElement { input: cx.entity() })
                    .into_any_element(),
                None => TextElement { input: cx.entity() }.into_any_element(),
            })
    }
}

/// The custom element that shapes, measures and paints the field's text.
struct TextElement {
    input: Entity<TextInput>,
}

/// Everything [`TextElement::prepaint`] hands over to `paint`.
struct PrepaintState {
    /// What was shaped: one line, or one per logical line.
    shaped: Option<Shaped>,
    cursor: Option<PaintQuad>,
    /// One quad per row the selection covers; a single-line field never has
    /// more than one.
    selection: Vec<PaintQuad>,
    display_map: Option<DisplayMap>,
    /// Row height this frame was laid out at.
    line_height: Pixels,
    /// Where the caret is, relative to the top left of the text — what a
    /// multiline field scrolls itself by.
    caret: Point<Pixels>,
}

impl IntoElement for TextElement {
    type Element = Self;

    fn into_element(self) -> Self::Element {
        self
    }
}

impl Element for TextElement {
    /// The cell the measure closure of a multiline field leaves its shaped
    /// lines in; `None` for the single-line field, which shapes in `prepaint`.
    type RequestLayoutState = Option<ShapedCell>;
    type PrepaintState = PrepaintState;

    fn id(&self) -> Option<ElementId> {
        None
    }

    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    /// One line is a fixed box; several have to be measured.
    ///
    /// The difference is wrapping: a multiline field only knows how tall it is
    /// once it knows how wide it is, which is what
    /// [`Window::request_measured_layout`] exists for. The shaping done inside
    /// the measure closure is the shaping `prepaint` and `paint` then use, so
    /// the text is laid out once per frame rather than three times.
    fn request_layout(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, Self::RequestLayoutState) {
        let input = self.input.read(cx);
        if input.rows.is_none() {
            let mut style = Style::default();
            style.size.width = relative(1.).into();
            style.size.height = window.line_height().into();
            return (window.request_layout(style, [], cx), None);
        }

        let theme = theme(cx);
        let is_empty = input.content.is_empty();
        let style = window.text_style();
        let text = if is_empty {
            input.placeholder.clone()
        } else {
            input.content.clone()
        };
        let color = match (is_empty, input.disabled) {
            (true, _) | (_, true) => theme.text_muted,
            _ => style.color,
        };
        let run = TextRun {
            len: text.len(),
            font: style.font(),
            color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let font_size = style.font_size.to_pixels(window.rem_size());
        let line_height = window.line_height();

        let cell: ShapedCell = Rc::new(RefCell::new(None));
        let shaped = Rc::clone(&cell);
        let layout_id = window.request_measured_layout(
            Style::default(),
            move |known, available, window, _cx| {
                let wrap_width = known.width.or(match available.width {
                    AvailableSpace::Definite(width) => Some(width),
                    _ => None,
                });
                let lines: Vec<WrappedLine> = window
                    .text_system()
                    .shape_text(
                        text.clone(),
                        font_size,
                        std::slice::from_ref(&run),
                        wrap_width,
                        None,
                    )
                    .map(|lines| lines.into_iter().collect())
                    .unwrap_or_default();

                let mut size = Size {
                    width: wrap_width.unwrap_or(Pixels::ZERO),
                    height: Pixels::ZERO,
                };
                for line in &lines {
                    let line_size = line.size(line_height);
                    size.height += line_size.height;
                    size.width = size.width.max(line_size.width);
                }
                // An empty field is still one row tall, or the caret would have
                // nowhere to stand.
                size.height = size.height.max(line_height);
                *shaped.borrow_mut() = Some(lines);
                size
            },
        );
        (layout_id, Some(cell))
    }

    fn prepaint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        request_layout: &mut Self::RequestLayoutState,
        window: &mut Window,
        cx: &mut App,
    ) -> Self::PrepaintState {
        match request_layout.take() {
            Some(cell) => self.prepaint_wrapped(cell, bounds, window, cx),
            None => self.prepaint_line(bounds, window, cx),
        }
    }

    fn paint(
        &mut self,
        _id: Option<&GlobalElementId>,
        _inspector_id: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _request_layout: &mut Self::RequestLayoutState,
        prepaint: &mut Self::PrepaintState,
        window: &mut Window,
        cx: &mut App,
    ) {
        let focus_handle = self.input.read(cx).focus_handle.clone();
        let disabled = self.input.read(cx).disabled;
        let line_height = prepaint.line_height;

        if !disabled {
            window.handle_input(
                &focus_handle,
                ElementInputHandler::new(bounds, self.input.clone()),
                cx,
            );
        }

        for selection in prepaint.selection.drain(..) {
            window.paint_quad(selection);
        }

        let shaped = prepaint.shaped.take().expect("prepaint always shapes");
        match &shaped {
            Shaped::Line(line) => {
                line.paint(
                    bounds.origin,
                    line_height,
                    TextAlign::Left,
                    None,
                    window,
                    cx,
                )
                .ok();
            }
            Shaped::Lines(lines) => {
                let mut top = bounds.top();
                for line in lines {
                    line.paint(
                        point(bounds.left(), top),
                        line_height,
                        TextAlign::Left,
                        None,
                        window,
                        cx,
                    )
                    .ok();
                    top += line.size(line_height).height;
                }
            }
        }

        if !disabled
            && focus_handle.is_focused(window)
            && let Some(cursor) = prepaint.cursor.take()
        {
            window.paint_quad(cursor);
        }

        let display_map = prepaint.display_map.take();
        let caret = prepaint.caret;
        self.input.update(cx, |input, cx| {
            input.last_layout = Some(shaped);
            input.last_bounds = Some(bounds);
            input.last_display_map = display_map;
            input.last_line_height = line_height;
            if let Some(rows) = input.rows
                && focus_handle.is_focused(window)
            {
                input.follow_caret(caret, rows, line_height, cx);
            }
        });
    }
}

impl TextInput {
    /// Scrolls a multiline field so that the caret stays inside it.
    ///
    /// Runs from `paint` — after the frame that moved the caret, not during it
    /// — so a keystroke that pushes the caret out of view is followed by one
    /// more frame that brings it back. Only an offset that actually changes
    /// asks for that frame; without the guard the field would redraw itself
    /// for ever.
    fn follow_caret(
        &mut self,
        caret: Point<Pixels>,
        rows: usize,
        line_height: Pixels,
        cx: &mut Context<Self>,
    ) {
        let offset = self.scroll.offset();
        let view_top = -offset.y;
        let view_height = line_height * rows as f32;
        let wanted = if caret.y < view_top {
            caret.y
        } else if caret.y + line_height > view_top + view_height {
            caret.y + line_height - view_height
        } else {
            view_top
        };
        if (wanted - view_top).abs() > px(0.5) {
            self.scroll.set_offset(point(offset.x, -wanted));
            cx.notify();
        }
    }
}

impl TextElement {
    /// Shapes and places the single line of a one-line field.
    fn prepaint_line(
        &mut self,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let theme = theme(cx);
        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let cursor = input.cursor_offset();
        let marked_range = input.marked_range.clone();
        let is_empty = input.content.is_empty();
        let disabled = input.disabled;
        let style = window.text_style();

        let (display_text, display_map, text_color) = if is_empty {
            (input.placeholder.clone(), None, theme.text_muted)
        } else {
            let (text, map) = input.display_text();
            let color = if disabled {
                theme.text_muted
            } else {
                style.color
            };
            (text, map, color)
        };
        let map = display_map.as_ref();

        let run = TextRun {
            len: display_text.len(),
            font: style.font(),
            color: text_color,
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let runs = if let Some(marked_range) = marked_range.filter(|_| !is_empty) {
            let start = to_display(map, marked_range.start);
            let end = to_display(map, marked_range.end);
            vec![
                TextRun {
                    len: start,
                    ..run.clone()
                },
                TextRun {
                    len: end - start,
                    underline: Some(UnderlineStyle {
                        color: Some(run.color),
                        thickness: px(1.0),
                        wavy: false,
                    }),
                    ..run.clone()
                },
                TextRun {
                    len: display_text.len() - end,
                    ..run
                },
            ]
            .into_iter()
            .filter(|run| run.len > 0)
            .collect()
        } else {
            vec![run]
        };

        let font_size = style.font_size.to_pixels(window.rem_size());
        let line = window
            .text_system()
            .shape_line(display_text, font_size, &runs, None);

        let caret_x = line.x_for_index(to_display(map, cursor));
        let (selection, cursor) = if selected_range.is_empty() {
            (
                Vec::new(),
                Some(fill(
                    Bounds::new(
                        point(bounds.left() + caret_x, bounds.top()),
                        size(px(2.), bounds.bottom() - bounds.top()),
                    ),
                    theme.accent,
                )),
            )
        } else {
            (
                vec![fill(
                    Bounds::from_corners(
                        point(
                            bounds.left() + line.x_for_index(to_display(map, selected_range.start)),
                            bounds.top(),
                        ),
                        point(
                            bounds.left() + line.x_for_index(to_display(map, selected_range.end)),
                            bounds.bottom(),
                        ),
                    ),
                    theme.accent.opacity(0.3),
                )],
                None,
            )
        };

        PrepaintState {
            shaped: Some(Shaped::Line(Box::new(line))),
            cursor,
            selection,
            display_map,
            line_height: window.line_height(),
            caret: point(caret_x, Pixels::ZERO),
        }
    }

    /// Places the caret and the selection over the wrapped lines the measure
    /// closure shaped.
    ///
    /// The lines are handed to the field first and taken back afterwards, so
    /// that the geometry questions are answered by
    /// [`TextInput::point_for`] — the same method the pointer and the arrow
    /// keys ask, which is what keeps a click and a caret agreeing about where
    /// a row is.
    fn prepaint_wrapped(
        &mut self,
        cell: ShapedCell,
        bounds: Bounds<Pixels>,
        window: &mut Window,
        cx: &mut App,
    ) -> PrepaintState {
        let theme = theme(cx);
        let line_height = window.line_height();
        let lines = cell.borrow_mut().take().unwrap_or_default();

        self.input.update(cx, |input, _cx| {
            input.last_layout = Some(Shaped::Lines(lines));
            input.last_line_height = line_height;
        });

        let input = self.input.read(cx);
        let selected_range = input.selected_range.clone();
        let caret = input
            .point_for(input.cursor_offset())
            .unwrap_or_else(|| point(Pixels::ZERO, Pixels::ZERO));
        let ends = (!selected_range.is_empty())
            .then(|| {
                Some((
                    input.point_for(selected_range.start)?,
                    input.point_for(selected_range.end)?,
                ))
            })
            .flatten();

        let shaped = self
            .input
            .update(cx, |input, _cx| input.last_layout.take())
            .expect("the lines were just handed over");

        let origin = bounds.origin;
        let right = bounds.size.width;
        let tint = theme.accent.opacity(0.3);
        let mut selection = Vec::new();
        if let Some((start, end)) = ends {
            if (end.y - start.y).abs() < px(0.5) {
                selection.push(fill(
                    Bounds::from_corners(
                        origin + start,
                        origin + point(end.x, end.y + line_height),
                    ),
                    tint,
                ));
            } else {
                // The first row runs to the edge of the field, the last one
                // from its left edge, and everything between is a full row.
                selection.push(fill(
                    Bounds::from_corners(
                        origin + start,
                        origin + point(right, start.y + line_height),
                    ),
                    tint,
                ));
                let mut top = start.y + line_height;
                while top + px(0.5) < end.y {
                    selection.push(fill(
                        Bounds::from_corners(
                            origin + point(Pixels::ZERO, top),
                            origin + point(right, top + line_height),
                        ),
                        tint,
                    ));
                    top += line_height;
                }
                selection.push(fill(
                    Bounds::from_corners(
                        origin + point(Pixels::ZERO, end.y),
                        origin + point(end.x, end.y + line_height),
                    ),
                    tint,
                ));
            }
        }

        let cursor = selected_range.is_empty().then(|| {
            fill(
                Bounds::new(origin + caret, size(px(2.), line_height)),
                theme.accent,
            )
        });

        PrepaintState {
            shaped: Some(shaped),
            cursor,
            selection,
            display_map: None,
            line_height,
            caret,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ops::{Deref, DerefMut};

    use gpui::{Modifiers, MouseButton, MouseDownEvent, TestAppContext, VisualTestContext};

    /// Where the field is drawn in the test window, and how big.
    const FIELD_ORIGIN: f32 = 20.;
    /// Width of the field under test, which is what the text wraps at.
    const FIELD_WIDTH: f32 = 400.;

    #[test]
    fn the_start_of_every_line_is_where_the_newline_left_it() {
        // `len + 1` entries: line `i` covers `starts[i]..starts[i + 1] - 1`,
        // the `- 1` being the newline that ended it.
        assert_eq!(line_starts(""), vec![0, 1]);
        assert_eq!(line_starts("abc"), vec![0, 4]);
        assert_eq!(line_starts("a\nbb\nc"), vec![0, 2, 5, 7]);
        // A trailing newline opens a line that is empty, and it is still a
        // line: the caret can stand on it.
        assert_eq!(line_starts("a\n"), vec![0, 2, 3]);
    }

    /// A harness holding one field, so that a real window lays it out.
    struct Harness {
        input: Entity<TextInput>,
    }

    impl Render for Harness {
        fn render(&mut self, _window: &mut Window, _cx: &mut Context<Self>) -> impl IntoElement {
            div()
                .absolute()
                .left(px(FIELD_ORIGIN))
                .top(px(FIELD_ORIGIN))
                .w(px(FIELD_WIDTH))
                .child(self.input.clone())
        }
    }

    /// Opens a window over one field, laid out and ready to be typed into.
    fn open(
        rows: Option<usize>,
        content: &str,
        cx: &mut TestAppContext,
    ) -> (Entity<TextInput>, VisualTestContext) {
        cx.update(crate::init);
        let text = content.to_string();
        let window = cx.add_window(move |_window, cx| {
            let input = cx.new(|cx| {
                let mut input = TextInput::new(cx);
                if let Some(rows) = rows {
                    input = input.multiline(rows);
                }
                input.set_content(text.clone(), cx);
                input
            });
            Harness { input }
        });
        let input = window
            .update(cx, |harness, _window, _cx| harness.input.clone())
            .expect("the window is up");
        let mut cx = VisualTestContext::from_window(*window.deref(), cx);
        cx.run_until_parked();
        // A second frame, because the caret and the mouse mapping both read
        // what the *last* frame shaped.
        cx.refresh().expect("the window redraws");
        cx.run_until_parked();
        (input, cx)
    }

    /// The caret, as an offset into the content.
    fn caret(input: &Entity<TextInput>, cx: &mut VisualTestContext) -> usize {
        cx.update(|_window, cx| input.read(cx).cursor_offset())
    }

    /// Puts the caret at `offset`.
    fn place(input: &Entity<TextInput>, offset: usize, cx: &mut VisualTestContext) {
        cx.update(|_window, cx| {
            input.update(cx, |input, cx| input.move_to(offset, cx));
        });
    }

    /// `Enter` is a line break in a multiline field and a submit in a
    /// single-line one — the one difference the two modes make to typing.
    #[gpui::test]
    fn enter_breaks_the_line_in_a_multiline_field(cx: &mut TestAppContext) {
        let (input, mut cx) = open(Some(3), "select 1", cx);
        place(&input, 6, &mut cx);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.submit(&Submit, window, cx));
        });
        cx.update(|_window, cx| {
            assert_eq!(input.read(cx).content(), "select\n 1");
        });
    }

    #[gpui::test]
    fn enter_submits_in_a_single_line_field(cx: &mut TestAppContext) {
        cx.update(crate::init);
        let submitted = std::rc::Rc::new(std::cell::RefCell::new(Vec::<String>::new()));
        let seen = submitted.clone();
        let window = cx.add_window(move |_window, cx| {
            let seen = seen.clone();
            let input = cx.new(move |cx| {
                let mut input = TextInput::new(cx)
                    .on_submit(move |text, _window, _cx| seen.borrow_mut().push(text.to_string()));
                input.set_content("staging", cx);
                input
            });
            Harness { input }
        });
        let input = window
            .update(cx, |harness, _window, _cx| harness.input.clone())
            .expect("the window is up");
        let mut cx = VisualTestContext::from_window(*window.deref(), cx);
        cx.run_until_parked();

        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.submit(&Submit, window, cx));
        });
        assert_eq!(submitted.borrow().as_slice(), ["staging".to_string()]);
        cx.update(|_window, cx| assert_eq!(input.read(cx).content(), "staging"));
    }

    /// `Home` and `End` are about the row the caret is on, not about the field.
    #[gpui::test]
    fn home_and_end_stay_on_the_row_they_were_pressed_on(cx: &mut TestAppContext) {
        let (input, mut cx) = open(Some(4), "one\ntwo\nthree", cx);
        // In the middle of "two".
        place(&input, 5, &mut cx);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.home(&Home, window, cx));
        });
        assert_eq!(caret(&input, &mut cx), 4);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.end(&End, window, cx));
        });
        assert_eq!(caret(&input, &mut cx), 7);

        // A single-line field has one row, and it is the whole content.
        let (single, mut cx) = open(None, "one\ntwo", cx.deref_mut());
        place(&single, 5, &mut cx);
        cx.update(|window, cx| {
            single.update(cx, |input, cx| input.home(&Home, window, cx));
        });
        assert_eq!(caret(&single, &mut cx), 0);
        cx.update(|window, cx| {
            single.update(cx, |input, cx| input.end(&End, window, cx));
        });
        assert_eq!(caret(&single, &mut cx), 7);
    }

    /// `Up` and `Down` move by row and keep the column, which is what makes
    /// them usable over text that was wrapped rather than typed.
    #[gpui::test]
    fn up_and_down_move_a_row_at_a_time(cx: &mut TestAppContext) {
        let (input, mut cx) = open(Some(4), "aaaa\nbbbb\ncccc", cx);
        // Two graphemes into the middle line.
        place(&input, 7, &mut cx);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.up(&Up, window, cx));
        });
        assert_eq!(caret(&input, &mut cx), 2, "the same column, a row up");
        cx.update(|window, cx| {
            input.update(cx, |input, cx| {
                input.down(&Down, window, cx);
                input.down(&Down, window, cx);
            });
        });
        assert_eq!(caret(&input, &mut cx), 12, "and two rows down again");

        // Past the last row, `Down` goes to the end rather than staying put.
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.down(&Down, window, cx));
        });
        assert_eq!(caret(&input, &mut cx), 14);
        // And `Up` from the first row goes to the start.
        place(&input, 2, &mut cx);
        cx.update(|window, cx| {
            input.update(cx, |input, cx| input.up(&Up, window, cx));
        });
        assert_eq!(caret(&input, &mut cx), 0);
    }

    /// A press lands on the row it was made over.
    ///
    /// The whole of what a multiline field adds to hit testing: the same press,
    /// one row lower, is a different offset.
    #[gpui::test]
    fn a_press_lands_on_the_row_it_was_made_over(cx: &mut TestAppContext) {
        let (input, mut cx) = open(Some(4), "aaaa\nbbbb\ncccc", cx);
        // Measured from where the text was actually painted rather than from
        // the frame's own geometry: this test is about the mapping from a
        // point to an offset, not about where a parent put the field.
        let origin = cx.update(|_window, cx| {
            input
                .read(cx)
                .last_bounds
                .expect("the field has been painted")
                .origin
        });
        let press = |cx: &mut VisualTestContext, row: f32| {
            cx.simulate_event(MouseDownEvent {
                position: origin + point(px(9.), px(20. * row + 10.)),
                modifiers: Modifiers::none(),
                button: MouseButton::Left,
                click_count: 1,
                first_mouse: false,
            });
            cx.run_until_parked();
        };

        press(&mut cx, 0.);
        let first = caret(&input, &mut cx);
        press(&mut cx, 1.);
        let second = caret(&input, &mut cx);
        press(&mut cx, 2.);
        let third = caret(&input, &mut cx);

        assert!(first < 4, "the first row is offsets 0..4, got {first}");
        assert!(
            (5..9).contains(&second),
            "the second row is offsets 5..9, got {second}"
        );
        assert!(
            (10..14).contains(&third),
            "the third row is offsets 10..14, got {third}"
        );
    }
}
