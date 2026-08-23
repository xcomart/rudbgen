//! The code editor: a rope, a pluggable highlighter, an incremental syntax
//! cache, and a gpui element that draws only what fits on screen.
//!
//! `rudbgen-ui`'s [`TextInput`](rudbgen_ui::TextInput) is a single line by
//! construction — it replaces `\n` with a space — so the editor is a new
//! widget rather than an extension of it. What carries over is the discipline,
//! not the code: byte offsets everywhere, UTF-16 only at the platform boundary,
//! grapheme clusters for every caret step, and an `EntityInputHandler` that the
//! IME can drive without ever being handed an offset that is not on a character
//! boundary. [`mod@editor`] documents each departure and why it is one.
//!
//! # The boundary
//!
//! This crate knows `rudbgen-ui` and nothing else (architecture document, §3).
//! It does **not** know `rudbgen-template`, and that is the interesting half of
//! the boundary: the engine either parses a whole template or fails, and a
//! template halfway through being typed does not parse. An editor whose colours
//! vanish on every other keystroke is worse than one with no colours, so
//! [`mod@template_syntax`] tokenizes the template grammar a second time, line by
//! line and never failing. The whole-document verdict — parse errors, unknown
//! fields — comes from the engine, through the app, as gutter marks (§4.5).
//! `rudbman-sql` is gone for the same reason it is not in this repository at
//! all: nothing here writes SQL to a server, so [`mod@sql_syntax`] is a two
//! hundred line lexer with no dialect rather than a dependency with a dialect
//! table.
//!
//! # The three things that make it hold at 100MB
//!
//! * **The buffer is a rope.** An insert is O(log n), and so are
//!   `byte <-> line` and `byte <-> UTF-16 code unit`. [`mod@buffer`].
//! * **The syntax cache is one [`LineState`] per line**, and an edit re-lexes
//!   from the edited line down to the first line whose end state is unchanged —
//!   which for an ordinary keystroke is the line itself. [`mod@highlight`].
//! * **Only the visible lines are shaped.** The element works out the row range
//!   from the scroll offset and the line height, and shapes those and no
//!   others. [`mod@element`].
//!
//! The things a whole-buffer `&str` would be needed for — "which statement is
//! the caret in", "which bracket matches this one" — are answered over a window
//! of the rope cut at statement boundaries, so they cost the length of a
//! statement rather than the length of the document. [`mod@syntax`].
//!
//! # Using it
//!
//! ```ignore
//! rudbgen_editor::init(cx);            // once, after rudbgen_ui::init
//!
//! let editor = cx.new(|cx| {
//!     EditorView::new(cx).highlighter(Arc::new(TemplateHighlighter))
//! });
//! cx.subscribe(&editor, |_, editor, event: &EditorEvent, cx| {
//!     if let EditorEvent::Changed = event {
//!         let text = editor.read(cx).text();
//!         // re-render the preview
//!     }
//! })
//! .detach();
//! ```
//!
//! An editor with no highlighter is a plain-text editor, and that is what
//! [`EditorView::new`] makes. [`CompositeHighlighter`] is how a template that
//! is *also* a Java or an XML file gets both: the base language underneath, the
//! template statements painted over it. [`mod@lang`] is where those base
//! languages live -- one [`Highlighter`] per extension the generator targets --
//! and [`lang::template_highlighter_for_path`] is what turns a template's path
//! into the right composite, or the template language alone when the
//! extension is not one this crate has a base lexer for.
//!
//! # Out of scope, deliberately
//!
//! Multiple cursors would change the shape of every command in [`mod@editor`],
//! so they go in as a list of selections in one piece or not at all. Code
//! folding needs a row-to-line map between the buffer and the renderer, which
//! nothing else wants yet. A minimap needs a second, coarser shaping pass, and
//! is the least valuable of the three.
//!
//! The completion popup is the app's, not this crate's: what to offer comes
//! from the variable palette, which comes from the model, which this crate has
//! never heard of. What is here is what the popup needs from the document —
//! [`EditorView::word_before_caret`], [`EditorView::line_before_caret`],
//! [`EditorView::caret_bounds`], [`EditorView::replace_range`] — so that no
//! caller ever has to work out a byte offset into the rope for itself.

#![warn(missing_docs)]

pub mod buffer;
pub mod composite;
pub mod editor;
pub mod element;
pub mod find;
pub mod highlight;
pub mod history;
pub mod lang;
pub mod sql_syntax;
pub mod syntax;
pub mod template_syntax;

pub use buffer::Buffer;
pub use composite::CompositeHighlighter;
pub use editor::{EditorEvent, EditorView, init};
pub use element::EditorElement;
pub use find::{FindState, find_all};
pub use highlight::{Highlighter, LineState, Span, SyntaxCache, Token};
pub use history::{Edit, EditKind, History, SelectionState, Transaction};
pub use lang::{highlighter_for_extension, template_highlighter_for_path};
pub use sql_syntax::SqlHighlighter;
pub use syntax::StatementSpan;
pub use template_syntax::TemplateHighlighter;

#[cfg(test)]
mod tests;
