//! jdbgen's template engine, ported to Rust.
//!
//! A template is parsed once into an immutable tree and rendered against as
//! many models as needed. Everything outside of a `${...}` placeholder is
//! copied verbatim; the line separator the template is written with is reused
//! wherever the engine inserts a line break of its own.
//!
//! ```
//! use rudbgen_template::{RenderContext, Template};
//! use std::collections::HashMap;
//!
//! let mut model = HashMap::new();
//! model.insert("name".to_string(), "tb_user_account".to_string());
//!
//! let template = Template::parse("class ${name.suffix.pascal}Model").unwrap();
//! let text = template.render(&model, &RenderContext::new()).unwrap();
//! assert_eq!(text, "class UserAccountModel");
//! ```
//!
//! # The language
//!
//! A placeholder is `${type:name=value, ...}`. It ends at the **first** `}`
//! behind the `${`, even one inside a quoted value. Type names, attribute
//! names, processor names and condition names are case insensitive; the
//! keywords `endif`, `endfor`, `else` and `elif:` are matched in lower case
//! only, so `${ELSE}` is read as a member lookup.
//!
//! | Statement | What it does |
//! |---|---|
//! | `${key}` | short for `${item:key=key}` |
//! | `${item:key=..., ...}` | a member of the current model |
//! | `${super:key=...}` | a member of the model enclosing the loop |
//! | `${for:key=..., inStr=..., indent=..., skipList=...}` … `${endfor}` | repeat over a collection |
//! | `${if:key=..., <condition>=...}` … `${elif:...}` … `${else}` … `${endif}` | branch |
//! | `${date}`, `${date:yyyy-MM}`, `${date:format=yyyy-MM}` | the current date |
//! | `${user}`, `${author}` | the login user and the `author` custom variable |
//! | `${'text'}`, `${"text"}` | literal text, which is how a template writes a `${` |
//!
//! A key may be followed by a chain of processors - `${name.suffix.camel}` -
//! applied left to right. `${a.b}` is such a chain and **never** a nested
//! path. The processors are `abbr`, `prefix`, `suffix`, `camel`, `pascal`,
//! `snake`, `screaming`, `skewer`/`kebab`, `lower`, `upper` and
//! `replace(find, replacement)`.
//!
//! Every rendered value then passes the same decorations: `prepend` and
//! `postpend` - both defaulting to `quote` - surround it, and `padSize`
//! together with `padDir` (`left` or the default `right`) pads it. Padding and
//! the `indent` of a loop count **display columns** - the same wcwidth-style
//! measure a terminal lays cells out by, not jdbgen's EUC-KR byte count (see
//! `docs/architecture.md` §7.4 for where the two diverge) - so that a double
//! width character takes the two columns it occupies in a fixed width font,
//! and a value wider than the padding is never cut off.
//!
//! The ten conditions of an `if` are `equals`/`value`, `notEquals`,
//! `contains`, `notContains`, `startsWith`, `notStartsWith`, `endsWith`,
//! `notEndsWith`, `matches` and `notMatches`. All of them are combined with
//! AND and compare ignoring case, except `matches`, which is a regular
//! expression the whole value has to match.
//!
//! A key nothing answers - neither the model nor the custom variables -
//! renders as an empty string and leaves a [`Warning`] in the
//! [`Diagnostics`], which is what the editor marks unknown fields with.
//!
//! # Compatibility
//!
//! The template assets of jdbgen have to render byte for byte the same, so the
//! rules that look like accidents are reproduced on purpose: the first `}`
//! closing a placeholder, the lower case only keywords, a quoted attribute
//! value losing exactly one enclosing pair of quotes while `(...)` keeps its
//! parentheses, and `${''}` standing for no text at all. jdbgen's three engine
//! test classes are ported case for case and are the canary for all of it.
//!
//! Three differences are deliberate:
//!
//! * **D10** - word level abbreviation rules match ignoring case. jdbgen
//!   looked the raw segment up in a lower cased map, so a word rule never
//!   fired on an upper case identifier, which is what database identifiers
//!   usually are. See [`Abbreviations::apply`].
//! * The loop counter is handed to the element by the renderer rather than
//!   written into it, which keeps [`Model`] free of interior mutability. A
//!   model without a `no` member now renders its position where jdbgen
//!   rendered nothing.
//! * Errors that jdbgen reported with a character offset in place of a line
//!   number report the line.

#![warn(missing_docs)]

mod abbr;
mod cond;
mod date;
mod error;
mod keys;
mod model;
mod parse;
mod render;
mod strutil;
mod template;

/// The date library the clock of a [`RenderContext`] is expressed in,
/// re-exported so that a caller needs no dependency of its own for it.
pub use chrono;

pub use abbr::{AbbrRule, Abbreviations};
pub use error::{Diagnostics, ParseError, RenderError, Span, Warning};
pub use model::{Model, Value};
pub use render::RenderContext;
pub use template::{StatementKind, StatementSpan, Template};
