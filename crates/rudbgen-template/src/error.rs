//! What a broken template says, and where.

use std::fmt;

/// A byte range of the template source, with the line it starts on.
///
/// The editor marks diagnostics with these, and [`crate::Template::spans`]
/// hands out one per statement for highlighting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Span {
    /// Byte offset of the first character.
    pub start: usize,
    /// Byte offset one past the last character.
    pub end: usize,
    /// Zero based line the span starts on, as jdbgen counts lines.
    pub line: usize,
}

impl Span {
    /// A span over `start..end` starting on `line`.
    pub fn new(start: usize, end: usize, line: usize) -> Self {
        Span { start, end, line }
    }
}

/// A template that cannot be parsed.
///
/// `line` is zero based, like the offset jdbgen puts into its
/// `ParseException`: the third line of a template is line 2.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// The message, worded as jdbgen words it.
    pub message: String,
    /// Zero based line the error was found on.
    pub line: usize,
    /// Where in the source the error was found, when that is known.
    pub span: Option<Span>,
}

impl ParseError {
    pub(crate) fn new(message: impl Into<String>, line: usize) -> Self {
        ParseError {
            message: message.into(),
            line,
            span: None,
        }
    }
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // the line is written one based, the way an editor numbers its lines
        write!(f, "line {}: {}", self.line + 1, self.message)
    }
}

impl std::error::Error for ParseError {}

/// A template that parses but cannot be rendered against this model.
///
/// jdbgen throws these from the render pass as well - a missing `key`, an
/// unknown processor, a `padSize` that is no number - and the port keeps them
/// there so that a template only fails on the model it actually fails on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RenderError {
    /// The message, worded as jdbgen words it.
    pub message: String,
    /// Where in the source the failing statement is, when that is known.
    pub span: Option<Span>,
}

impl RenderError {
    pub(crate) fn new(message: impl Into<String>) -> Self {
        RenderError {
            message: message.into(),
            span: None,
        }
    }

    pub(crate) fn at(message: impl Into<String>, span: Span) -> Self {
        RenderError {
            message: message.into(),
            span: Some(span),
        }
    }

    /// Zero based line of the failing statement, when it is known.
    pub fn line(&self) -> Option<usize> {
        self.span.map(|s| s.line)
    }
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.span {
            Some(span) => write!(f, "line {}: {}", span.line + 1, self.message),
            None => write!(f, "{}", self.message),
        }
    }
}

impl std::error::Error for RenderError {}

/// Something the template asked for that nothing could answer.
///
/// jdbgen writes these to the log and renders an empty string; the editor
/// wants them back so it can mark the place.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Warning {
    /// The statement the warning is about.
    pub span: Span,
    /// The key that could not be resolved.
    pub key: String,
    /// The message, worded as jdbgen words it.
    pub message: String,
}

/// Everything a render pass noticed without failing over it.
#[derive(Clone, Debug, Default)]
pub struct Diagnostics {
    warnings: Vec<Warning>,
}

impl Diagnostics {
    /// An empty set of diagnostics.
    pub fn new() -> Self {
        Diagnostics::default()
    }

    /// The warnings collected so far, in the order they were found.
    pub fn warnings(&self) -> &[Warning] {
        &self.warnings
    }

    /// Whether nothing was noticed.
    pub fn is_empty(&self) -> bool {
        self.warnings.is_empty()
    }

    /// Drop everything collected so far, so the same instance may be reused
    /// for the next model.
    pub fn clear(&mut self) {
        self.warnings.clear();
    }

    pub(crate) fn warn(&mut self, span: Span, key: &str) {
        self.warnings.push(Warning {
            span,
            key: key.to_string(),
            message: format!("cannot find '{key}' information from database/custom variables"),
        });
    }
}
