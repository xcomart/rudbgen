//! The template-language highlighter.
//!
//! # Why a tokenizer of its own
//!
//! `rudbgen-template` has a parser, and it hands out statement spans
//! (`Template::spans`). It is the wrong thing to draw with, for one reason: it
//! either parses the whole document or fails. A template halfway through being
//! typed does not parse — an `${if:` with no `${endif}` yet is a
//! `ParseError` — and an editor whose colours disappear on every other
//! keystroke is worse than one with no colours at all. So the editor knows the
//! grammar twice: once in the engine, which decides what a template *means*,
//! and once here, which decides what it *looks like*, line by line and never
//! failing. That is also why this crate does not depend on `rudbgen-template`:
//! the editor knows `rudbgen-ui` and nothing else (architecture document, §3).
//!
//! The two have to agree on the grammar, and the places where agreement is
//! easy to lose are exactly jdbgen's oddities, so they are written out here:
//!
//! * A `${` is closed by the **first** `}` behind it — even one inside a
//!   quoted attribute value. `${item:key=a,default='}'}` is a statement that
//!   ends at the quoted brace, and the `'}` after it is text.
//! * `${'...'}` is a **literal**: its content is copied out, and a `}` inside
//!   it does not close it. It is the only way to write a `${` into the output.
//! * `endif`, `endfor`, `else` and `elif:` are matched in **lower case only**;
//!   every other statement name is case insensitive. `${ENDIF}` is not the end
//!   of an `if` — it is an item lookup for a field called `ENDIF` — and it is
//!   painted as a warning for that reason (architecture document, appendix A).
//! * A statement with no `:` in it is an item: `${name}` means
//!   `${item:key=name}`, and `${name.camel}` is a key with a processor chain,
//!   not a nested path.
//! * Whitespace, line breaks included, is allowed inside a statement, so a
//!   statement is carried from line to line in [`LineState`].
//!
//! # What is painted, and what is not
//!
//! | written | painted as |
//! |---|---|
//! | text between statements | nothing — the palette's foreground |
//! | `${`, `}`, `(`, `)` | punctuation |
//! | statement name: `if`, `for`, `item`, `endif` … | keyword |
//! | option name: `key`, `equals`, `format` … | type |
//! | `:`, `=`, `,`, `.` | operator |
//! | `'...'`, `"..."`, and the whole of a `${'...'}` literal | string |
//! | a numeric option value | number |
//! | a known processor: `.camel`, `.replace(…)` … | function |
//! | an unknown processor, and `${ENDIF}` and its kind | warning |
//! | an unknown statement name: the `foo` of `${foo:x}` | error |
//!
//! Two things the architecture document's §8 lists are deliberately **not**
//! painted as errors here. A `}` in the text between statements is not one: a
//! Java or XML template is full of braces that close nothing, and lighting all
//! of them up would make the colour useless. An `${` that has not been closed
//! yet is not one either: the state simply carries to the next line, because
//! the closing brace of a statement being typed arrives one keystroke later and
//! a highlighter that reddens the rest of the file in between is unusable.
//! Both are structural mistakes the *engine* reports, with a line number and a
//! span, and the template tab draws them in the gutter (§4.5) — which is where
//! a whole-document verdict belongs.

use std::ops::Range;

use crate::highlight::{Highlighter, LineState, Span, Token};

/// The statement names, lower-cased and sorted. Matched case-insensitively,
/// exactly as `parse_one` lower-cases the name before it dispatches.
const STATEMENTS: &[&str] = &["author", "date", "for", "if", "item", "super", "user"];

/// The block-structure names, which are matched in lower case **only**.
///
/// `elif` is one of them too, but it carries a `:` and options, so it is
/// checked where a statement name is checked rather than here.
const BLOCK_ENDS: &[&str] = &["else", "endfor", "endif"];

/// The processors a key chain may be built out of, sorted.
///
/// The same twelve `rudbgen-template`'s `keys::PROCESSORS` lists, matched
/// ignoring case as the engine matches them.
const PROCESSORS: &[&str] = &[
    "abbr",
    "camel",
    "kebab",
    "lower",
    "pascal",
    "prefix",
    "replace",
    "screaming",
    "skewer",
    "snake",
    "suffix",
    "upper",
];

/// Whether `word`, in any case, is in `table`.
fn contains(table: &[&str], word: &str) -> bool {
    let lowered = word.to_lowercase();
    table.binary_search(&lowered.as_str()).is_ok()
}

/// Where the tokenizer is inside a statement.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Mode {
    /// Between statements.
    #[default]
    Text,
    /// Inside `${ … }`.
    Statement,
    /// Inside the `'…'` of a `${'…'}` literal.
    Literal,
    /// Past the closing quote of a literal, looking for the `}`.
    AfterLiteral,
}

/// Everything the tokenizer has to remember between two lines.
///
/// Encoded into the ten low bits of a [`LineState`]; the all-zero encoding is
/// [`LineState::START`], which is [`Mode::Text`] with nothing open.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct State {
    mode: Mode,
    /// Whether the statement's `:` has been passed, so the rest is options.
    seen_colon: bool,
    /// The quote a value — or the literal — was opened with, if one is open.
    quote: Option<u8>,
    /// Whether a processor's `(` is open.
    paren: bool,
    /// Whether nothing but whitespace has been seen since the `${`.
    ///
    /// What decides whether a `'` opens a literal: only the first thing in a
    /// statement can.
    fresh: bool,
    /// Whether the `=` of the current option has been passed.
    in_value: bool,
    /// Whether the value being read is a key chain, so that a `.` in it
    /// introduces a processor rather than being part of the text.
    key_value: bool,
}

impl State {
    /// Reads a [`LineState`] back.
    fn decode(state: LineState) -> Self {
        let bits = state.0;
        Self {
            mode: match bits & 0b11 {
                1 => Mode::Statement,
                2 => Mode::Literal,
                3 => Mode::AfterLiteral,
                _ => Mode::Text,
            },
            seen_colon: bits & (1 << 2) != 0,
            quote: match (bits >> 3) & 0b11 {
                1 => Some(b'\''),
                2 => Some(b'"'),
                _ => None,
            },
            paren: bits & (1 << 5) != 0,
            fresh: bits & (1 << 6) != 0,
            in_value: bits & (1 << 7) != 0,
            key_value: bits & (1 << 8) != 0,
        }
    }

    /// The opaque form the cache stores.
    fn encode(self) -> LineState {
        let mut bits = match self.mode {
            Mode::Text => 0,
            Mode::Statement => 1,
            Mode::Literal => 2,
            Mode::AfterLiteral => 3,
        };
        bits |= u32::from(self.seen_colon) << 2;
        bits |= match self.quote {
            Some(b'\'') => 1 << 3,
            Some(_) => 2 << 3,
            None => 0,
        };
        bits |= u32::from(self.paren) << 5;
        bits |= u32::from(self.fresh) << 6;
        bits |= u32::from(self.in_value) << 7;
        bits |= u32::from(self.key_value) << 8;
        LineState(bits)
    }

    /// The state a `${` opens.
    fn opened() -> Self {
        Self {
            mode: Mode::Statement,
            fresh: true,
            ..Self::default()
        }
    }
}

/// jdbgen's template language, as much of it as a colour needs.
///
/// A unit struct: there is nothing to configure, and one `Arc` of it can be
/// shared by every template tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TemplateHighlighter;

impl TemplateHighlighter {
    /// The spans of `text`, the byte ranges its `${…}` statements cover, and
    /// the state the next line starts in.
    ///
    /// The regions are what
    /// [`CompositeHighlighter`](crate::composite::CompositeHighlighter) needs
    /// and [`Highlighter::line`] throws away: to paint template statements over
    /// a Java or XML file, the base language's spans have to be cut out
    /// wherever a statement stands, and only the tokenizer knows where that is.
    /// They are sorted, do not overlap, and a statement that runs over a line
    /// break contributes one region to each line it touches.
    pub fn statements(&self, text: &str, state: LineState) -> Statements {
        let mut lexer = Lexer {
            text,
            bytes: text.as_bytes(),
            at: 0,
            spans: Vec::new(),
            regions: Vec::new(),
            region: None,
        };
        let start = State::decode(state);
        if start.mode != Mode::Text {
            lexer.region = Some(0);
        }
        let end = lexer.run(start);
        if let Some(open) = lexer.region.take() {
            lexer.regions.push(open..text.len());
        }
        Statements {
            spans: lexer.spans,
            regions: lexer.regions,
            state: end.encode(),
        }
    }
}

/// What [`TemplateHighlighter::statements`] found on one line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Statements {
    /// The coloured runs, as [`Highlighter::line`] would answer with them.
    pub spans: Vec<Span>,
    /// The byte ranges the line's `${…}` statements cover.
    pub regions: Vec<Range<usize>>,
    /// The state the next line starts in.
    pub state: LineState,
}

impl Highlighter for TemplateHighlighter {
    fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState) {
        let found = self.statements(text, state);
        (found.spans, found.state)
    }

    // No `line_comment`: a template has no comment syntax at all, and the
    // editor's comment toggle does nothing in one. Commenting a line out of a
    // Java template would have to write the *output* language's comment, which
    // this crate cannot know.
}

/// One line's worth of scanning.
struct Lexer<'a> {
    text: &'a str,
    bytes: &'a [u8],
    at: usize,
    spans: Vec<Span>,
    /// The statement regions closed so far on this line.
    regions: Vec<Range<usize>>,
    /// Where the statement now open began, when one is open.
    region: Option<usize>,
}

impl Lexer<'_> {
    /// Scans the whole line and answers with the state it ends in.
    ///
    /// Every branch below either advances `at` or leaves the loop, so this
    /// terminates on every input.
    fn run(&mut self, mut state: State) -> State {
        while self.at < self.bytes.len() {
            match state.mode {
                Mode::Text => self.text(&mut state),
                Mode::Statement => self.statement(&mut state),
                Mode::Literal => self.literal(&mut state),
                Mode::AfterLiteral => self.after_literal(&mut state),
            }
        }
        state
    }

    // ----------------------------------------------------------------- text

    /// Walks the text between statements up to the next `${`.
    fn text(&mut self, state: &mut State) {
        let mut at = self.at;
        while at + 1 < self.bytes.len() {
            if self.bytes[at] == b'$' && self.bytes[at + 1] == b'{' {
                self.push(at, at + 2, Token::Punctuation);
                self.at = at + 2;
                self.region = Some(at);
                *state = State::opened();
                return;
            }
            at += 1;
        }
        // A `$` in the last byte of the line opens nothing: the `{` that would
        // pair with it is on the other side of a line break, and jdbgen's
        // opener is two adjacent characters.
        self.at = self.bytes.len();
    }

    // ------------------------------------------------------------ literals

    /// Scans the body of a `${'…'}` literal that opened on an earlier line.
    fn literal(&mut self, state: &mut State) {
        let quote = state.quote.unwrap_or(b'\'');
        if self.quoted(self.at, quote, self.bytes.len()) {
            state.mode = Mode::AfterLiteral;
            state.quote = None;
        }
    }

    /// Skips whatever stands between a closed literal and its `}`.
    ///
    /// jdbgen ignores it — the literal's content is the whole statement — so
    /// nothing here is painted.
    fn after_literal(&mut self, state: &mut State) {
        match self.find(b'}', self.at, self.bytes.len()) {
            Some(close) => {
                self.push(close, close + 1, Token::Punctuation);
                self.at = close + 1;
                self.close_region(self.at);
                *state = State::default();
            }
            None => self.at = self.bytes.len(),
        }
    }

    // ----------------------------------------------------------- statements

    /// Scans as much of a statement as this line holds.
    fn statement(&mut self, state: &mut State) {
        let opened_here = state.fresh;
        if state.fresh {
            self.skip_blanks();
            if self.at >= self.bytes.len() {
                return;
            }
            let byte = self.bytes[self.at];
            if byte == b'\'' || byte == b'"' {
                let start = self.at;
                self.at += 1;
                state.fresh = false;
                if self.quoted(start, byte, self.bytes.len()) {
                    state.mode = Mode::AfterLiteral;
                } else {
                    state.mode = Mode::Literal;
                    state.quote = Some(byte);
                }
                return;
            }
            state.fresh = false;
        }

        let close = self.find(b'}', self.at, self.bytes.len());
        let region = close.unwrap_or(self.bytes.len());

        if !state.seen_colon {
            match self.find(b':', self.at, region) {
                Some(colon) => {
                    if opened_here {
                        self.statement_name(self.at, colon);
                    }
                    self.push(colon, colon + 1, Token::Operator);
                    self.at = colon + 1;
                    state.seen_colon = true;
                    state.in_value = false;
                    state.key_value = false;
                }
                None => {
                    if close.is_none() {
                        // The name runs on to the next line; there is nothing
                        // here to classify it by yet.
                        self.at = self.bytes.len();
                        return;
                    }
                    if opened_here {
                        self.colonless_body(self.at, region);
                    }
                    self.at = region;
                }
            }
        }

        while self.at < region {
            self.option(state, region);
        }

        match close {
            Some(close) => {
                self.push(close, close + 1, Token::Punctuation);
                self.at = close + 1;
                self.close_region(self.at);
                *state = State::default();
            }
            None => self.at = self.bytes.len(),
        }
    }

    /// Records the statement that ends at `end` and opens no other.
    fn close_region(&mut self, end: usize) {
        let start = self.region.take().unwrap_or(0);
        if end > start {
            self.regions.push(start..end);
        }
    }

    /// Paints the name of a statement that carries a `:`.
    fn statement_name(&mut self, start: usize, end: usize) {
        let Some((from, to)) = self.trimmed(start, end) else {
            return;
        };
        let name = &self.text[from..to];
        let token = if contains(STATEMENTS, name) {
            Token::Keyword
        } else if name.eq_ignore_ascii_case("elif") {
            // `elif:` is one of the four the engine matches in lower case only.
            if name == "elif" {
                Token::Keyword
            } else {
                Token::Warning
            }
        } else {
            Token::Error
        };
        self.push(from, to, token);
    }

    /// Paints a statement with no `:` in it.
    ///
    /// Four shapes: a block end (`${endif}`), a block end written in the wrong
    /// case (`${ENDIF}`, which the engine reads as an item lookup), a bare
    /// `${user}`/`${date}`/`${author}`, and everything else — a key chain.
    fn colonless_body(&mut self, start: usize, end: usize) {
        let Some((from, to)) = self.trimmed(start, end) else {
            return;
        };
        let body = &self.text[from..to];
        if BLOCK_ENDS.binary_search(&body).is_ok() {
            self.push(from, to, Token::Keyword);
        } else if contains(BLOCK_ENDS, body) || body.eq_ignore_ascii_case("elif") {
            self.push(from, to, Token::Warning);
        } else if contains(STATEMENTS, body) {
            self.push(from, to, Token::Keyword);
        } else {
            self.chain(from, to);
        }
    }

    /// Scans one thing out of a statement's option list.
    fn option(&mut self, state: &mut State, region: usize) {
        if let Some(quote) = state.quote {
            if self.quoted(self.at, quote, region) {
                state.quote = None;
            }
            return;
        }
        let byte = self.bytes[self.at];
        match byte {
            b' ' | b'\t' | b'\r' | b'\n' => self.at += 1,
            b'\'' | b'"' => {
                let start = self.at;
                self.at += 1;
                if !self.quoted(start, byte, region) {
                    state.quote = Some(byte);
                }
            }
            b'(' => {
                self.push(self.at, self.at + 1, Token::Punctuation);
                self.at += 1;
                state.paren = true;
            }
            b')' => {
                self.push(self.at, self.at + 1, Token::Punctuation);
                self.at += 1;
                state.paren = false;
            }
            b'=' => {
                self.push(self.at, self.at + 1, Token::Operator);
                self.at += 1;
                state.in_value = !state.paren;
            }
            b',' => {
                self.push(self.at, self.at + 1, Token::Operator);
                self.at += 1;
                if !state.paren {
                    state.in_value = false;
                    state.key_value = false;
                }
            }
            _ => self.atom(state, region),
        }
    }

    /// Scans a bare run — an option name or an unquoted value — and paints it.
    fn atom(&mut self, state: &mut State, region: usize) {
        let start = self.at;
        while self.at < region {
            let byte = self.bytes[self.at];
            if byte == b'\\' {
                self.at = (self.at + 2).min(region);
                continue;
            }
            // Exactly the bytes `option` handles itself, so that this loop
            // always consumes at least one and the caller always makes
            // progress.
            if matches!(byte, b',' | b'=' | b'(' | b')' | b'\'' | b'"') {
                break;
            }
            self.at += 1;
        }
        let Some((from, to)) = self.trimmed(start, self.at) else {
            return;
        };

        // An option *name* is a run the `=` of its pair stopped. Anything else
        // — the run after that `=`, an argument inside parentheses, the whole
        // of a `${date:yyyy-MM-dd}` — is a value.
        let is_name = !state.in_value && !state.paren && self.bytes.get(self.at) == Some(&b'=');
        if is_name {
            let name = &self.text[from..to];
            state.key_value = name.eq_ignore_ascii_case("key") || name.eq_ignore_ascii_case("item");
            self.push(from, to, Token::Type);
        } else if state.key_value && !state.paren {
            self.chain(from, to);
        } else {
            self.push(from, to, value_token(&self.text[from..to]));
        }
    }

    /// Paints a key chain: a field name and the processors applied to it.
    ///
    /// A run that starts with a `.` is the tail of a chain whose head was cut
    /// off by a processor's parentheses — the `.camel` of
    /// `key=name.replace('_','-').camel` — so its first segment is a processor
    /// and not a field.
    fn chain(&mut self, start: usize, end: usize) {
        let mut head = true;
        let mut segment = start;
        let mut at = start;
        while at <= end {
            if at == end || self.bytes[at] == b'.' {
                if segment < at {
                    let text = &self.text[segment..at];
                    let token = if head {
                        value_token(text)
                    } else if contains(PROCESSORS, text.trim()) {
                        Token::Function
                    } else {
                        Token::Warning
                    };
                    self.push(segment, at, token);
                }
                if at < end {
                    self.push(at, at + 1, Token::Operator);
                }
                head = false;
                segment = at + 1;
            }
            at += 1;
        }
    }

    // -------------------------------------------------------------- helpers

    /// Scans a quoted run to its closing quote, painting `start..` as a string.
    ///
    /// A backslash escapes whatever follows it, which is how the engine reads
    /// both an attribute value and a literal. `limit` is where the run has to
    /// stop whatever happens: the end of the line for a literal, and the
    /// statement's `}` for an attribute value, because a `}` closes a statement
    /// even from inside a quote. Answers whether the quote was closed.
    fn quoted(&mut self, start: usize, quote: u8, limit: usize) -> bool {
        let mut escaped = false;
        while self.at < limit {
            let byte = self.bytes[self.at];
            self.at += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                self.push(start, self.at, Token::String);
                return true;
            }
        }
        self.at = limit;
        self.push(start, limit, Token::String);
        false
    }

    /// The first `needle` in `from..limit`.
    fn find(&self, needle: u8, from: usize, limit: usize) -> Option<usize> {
        self.bytes[from..limit]
            .iter()
            .position(|byte| *byte == needle)
            .map(|offset| from + offset)
    }

    /// `start..end` with the whitespace at either end taken off, or `None` when
    /// nothing is left.
    fn trimmed(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        let mut from = start;
        let mut to = end;
        while from < to && self.bytes[from].is_ascii_whitespace() {
            from += 1;
        }
        while to > from && self.bytes[to - 1].is_ascii_whitespace() {
            to -= 1;
        }
        (from < to).then_some((from, to))
    }

    /// Steps over the whitespace at the cursor.
    fn skip_blanks(&mut self) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    /// Records a span, dropping empty ones.
    fn push(&mut self, start: usize, end: usize, token: Token) {
        if end > start {
            self.spans.push(Span::new(start..end, token));
        }
    }
}

/// What an unquoted value is painted as: a number if it reads as one, a plain
/// identifier otherwise.
fn value_token(text: &str) -> Token {
    let digits = text.trim().trim_start_matches(['-', '+']);
    let mut dots = 0;
    let numeric = !digits.is_empty()
        && digits.chars().all(|c| {
            if c == '.' {
                dots += 1;
                dots == 1
            } else {
                c.is_ascii_digit()
            }
        })
        && digits.chars().any(|c| c.is_ascii_digit());
    if numeric {
        Token::Number
    } else {
        Token::Identifier
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(text, token)` for every span of `line`, lexed from `state`, with the
    /// span contract checked on the way through.
    fn lex(line: &str, state: LineState) -> (Vec<(&str, Token)>, LineState) {
        let (spans, end) = TemplateHighlighter.line(line, state);
        let mut last = 0;
        for span in &spans {
            assert!(
                span.range.start >= last,
                "spans overlap or are unsorted in {line:?}: {spans:?}"
            );
            assert!(
                span.range.end <= line.len(),
                "span past the end of {line:?}"
            );
            assert!(!span.is_empty(), "empty span in {line:?}");
            last = span.range.end;
        }
        (
            spans
                .iter()
                .map(|span| (&line[span.range.clone()], span.token))
                .collect(),
            end,
        )
    }

    /// The spans of one whole line, lexed from the start state.
    fn spans(line: &str) -> Vec<(&str, Token)> {
        let (spans, state) = lex(line, LineState::START);
        assert!(state.is_start(), "{line:?} left a statement open");
        spans
    }

    /// The spans of every line of `text`, lexed the way the cache lexes them.
    fn lines(text: &str) -> Vec<Vec<(&str, Token)>> {
        let mut state = LineState::START;
        let mut out = Vec::new();
        for line in text.split('\n') {
            let (spans, next) = lex(line, state);
            state = next;
            out.push(spans);
        }
        out
    }

    #[test]
    fn the_tables_are_sorted_and_lower_case() {
        for table in [STATEMENTS, BLOCK_ENDS, PROCESSORS] {
            for pair in table.windows(2) {
                assert!(pair[0] < pair[1], "{pair:?} is out of order");
            }
            for word in table {
                assert_eq!(*word, word.to_lowercase());
            }
        }
    }

    #[test]
    fn text_between_statements_is_painted_as_nothing() {
        assert!(spans("public class Foo {").is_empty());
        assert!(
            spans("}").is_empty(),
            "a brace that closes nothing is ordinary text, not an error"
        );
        assert!(spans("").is_empty());
        assert!(spans("cost is 100$").is_empty());
    }

    #[test]
    fn a_bare_item_is_a_key_chain() {
        assert_eq!(
            spans("class ${name} {"),
            vec![
                ("${", Token::Punctuation),
                ("name", Token::Identifier),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_processor_chain_names_its_steps() {
        assert_eq!(
            spans("${name.camel}"),
            vec![
                ("${", Token::Punctuation),
                ("name", Token::Identifier),
                (".", Token::Operator),
                ("camel", Token::Function),
                ("}", Token::Punctuation),
            ]
        );
        assert_eq!(
            spans("${name.CAMEL}"),
            vec![
                ("${", Token::Punctuation),
                ("name", Token::Identifier),
                (".", Token::Operator),
                ("CAMEL", Token::Function),
                ("}", Token::Punctuation),
            ],
            "a processor name is matched ignoring case, as the engine matches it"
        );
    }

    #[test]
    fn an_unknown_processor_is_a_warning() {
        assert_eq!(
            spans("${name.camelCase}"),
            vec![
                ("${", Token::Punctuation),
                ("name", Token::Identifier),
                (".", Token::Operator),
                ("camelCase", Token::Warning),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_processor_carries_its_arguments() {
        assert_eq!(
            spans("${item:key=name.replace('_','-').pascal}"),
            vec![
                ("${", Token::Punctuation),
                ("item", Token::Keyword),
                (":", Token::Operator),
                ("key", Token::Type),
                ("=", Token::Operator),
                ("name", Token::Identifier),
                (".", Token::Operator),
                ("replace", Token::Function),
                ("(", Token::Punctuation),
                ("'_'", Token::String),
                (",", Token::Operator),
                ("'-'", Token::String),
                (")", Token::Punctuation),
                (".", Token::Operator),
                ("pascal", Token::Function),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn option_names_and_values_are_told_apart() {
        assert_eq!(
            spans("${if:key=type,equals=VARCHAR}"),
            vec![
                ("${", Token::Punctuation),
                ("if", Token::Keyword),
                (":", Token::Operator),
                ("key", Token::Type),
                ("=", Token::Operator),
                ("type", Token::Identifier),
                (",", Token::Operator),
                ("equals", Token::Type),
                ("=", Token::Operator),
                ("VARCHAR", Token::Identifier),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_numeric_value_is_a_number() {
        assert_eq!(
            spans("${item:key=size,width=-12.5}"),
            vec![
                ("${", Token::Punctuation),
                ("item", Token::Keyword),
                (":", Token::Operator),
                ("key", Token::Type),
                ("=", Token::Operator),
                ("size", Token::Identifier),
                (",", Token::Operator),
                ("width", Token::Type),
                ("=", Token::Operator),
                ("-12.5", Token::Number),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_statement_name_is_case_insensitive() {
        for name in ["if", "IF", "If"] {
            let line = format!("${{{name}:key=a}}");
            let (spans, _) = lex(&line, LineState::START);
            assert_eq!(spans[1], (name, Token::Keyword), "in {line:?}");
        }
    }

    #[test]
    fn an_unknown_statement_name_is_an_error() {
        assert_eq!(
            spans("${loop:key=a}"),
            vec![
                ("${", Token::Punctuation),
                ("loop", Token::Error),
                (":", Token::Operator),
                ("key", Token::Type),
                ("=", Token::Operator),
                ("a", Token::Identifier),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn block_ends_are_lower_case_only() {
        for name in ["endif", "endfor", "else"] {
            assert_eq!(
                spans(&format!("${{{name}}}"))[1],
                (name, Token::Keyword),
                "{name} in lower case is the block end"
            );
            let shouted = name.to_uppercase();
            let line = format!("${{{shouted}}}");
            let (spans, _) = lex(&line, LineState::START);
            assert_eq!(
                spans[1],
                (shouted.as_str(), Token::Warning),
                "{shouted} is an item lookup, not a block end"
            );
        }
    }

    #[test]
    fn elif_is_lower_case_only_too() {
        assert_eq!(spans("${elif:key=a}")[1], ("elif", Token::Keyword));
        assert_eq!(
            lex("${ELIF:key=a}", LineState::START).0[1],
            ("ELIF", Token::Warning)
        );
    }

    #[test]
    fn the_bare_statements_stay_keywords() {
        for name in ["user", "date", "author", "USER"] {
            let line = format!("${{{name}}}");
            let (spans, _) = lex(&line, LineState::START);
            assert_eq!(spans[1], (name, Token::Keyword), "in {line:?}");
        }
        assert_eq!(
            spans("${date:yyyy-MM-dd}"),
            vec![
                ("${", Token::Punctuation),
                ("date", Token::Keyword),
                (":", Token::Operator),
                ("yyyy-MM-dd", Token::Identifier),
                ("}", Token::Punctuation),
            ],
            "a date format is the whole option list and carries no `=`"
        );
    }

    #[test]
    fn a_literal_is_one_string() {
        assert_eq!(
            spans("${'${not a statement}'}"),
            vec![
                ("${", Token::Punctuation),
                ("'${not a statement}'", Token::String),
                ("}", Token::Punctuation),
            ],
            "a closing brace inside a literal does not close the literal"
        );
        assert_eq!(
            spans("${ \"double\" }"),
            vec![
                ("${", Token::Punctuation),
                ("\"double\"", Token::String),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn the_first_brace_closes_a_statement_even_inside_a_quote() {
        // jdbgen looks for the `}` before it ever reads the attribute list, so
        // the quoted brace ends the statement and the `'}` after it is text.
        assert_eq!(
            spans("${item:key=a,default='}'}"),
            vec![
                ("${", Token::Punctuation),
                ("item", Token::Keyword),
                (":", Token::Operator),
                ("key", Token::Type),
                ("=", Token::Operator),
                ("a", Token::Identifier),
                (",", Token::Operator),
                ("default", Token::Type),
                ("=", Token::Operator),
                ("'", Token::String),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_statement_carries_over_a_line_break() {
        assert_eq!(
            lines("${if:key=type,\n      equals=VARCHAR}\ntail"),
            vec![
                vec![
                    ("${", Token::Punctuation),
                    ("if", Token::Keyword),
                    (":", Token::Operator),
                    ("key", Token::Type),
                    ("=", Token::Operator),
                    ("type", Token::Identifier),
                    (",", Token::Operator),
                ],
                vec![
                    ("equals", Token::Type),
                    ("=", Token::Operator),
                    ("VARCHAR", Token::Identifier),
                    ("}", Token::Punctuation),
                ],
                vec![],
            ]
        );
    }

    #[test]
    fn a_quoted_value_carries_over_a_line_break() {
        assert_eq!(
            lines("${item:key=a,default='one\ntwo'}"),
            vec![
                vec![
                    ("${", Token::Punctuation),
                    ("item", Token::Keyword),
                    (":", Token::Operator),
                    ("key", Token::Type),
                    ("=", Token::Operator),
                    ("a", Token::Identifier),
                    (",", Token::Operator),
                    ("default", Token::Type),
                    ("=", Token::Operator),
                    ("'one", Token::String),
                ],
                vec![("two'", Token::String), ("}", Token::Punctuation)],
            ]
        );
    }

    #[test]
    fn a_literal_carries_over_a_line_break() {
        assert_eq!(
            lines("${'one\ntwo'}"),
            vec![
                vec![("${", Token::Punctuation), ("'one", Token::String)],
                vec![("two'", Token::String), ("}", Token::Punctuation)],
            ]
        );
    }

    #[test]
    fn the_opener_may_stand_alone_on_its_line() {
        assert_eq!(
            lines("${\n  if:key=a}"),
            vec![
                vec![("${", Token::Punctuation)],
                vec![
                    ("if", Token::Keyword),
                    (":", Token::Operator),
                    ("key", Token::Type),
                    ("=", Token::Operator),
                    ("a", Token::Identifier),
                    ("}", Token::Punctuation),
                ],
            ],
            "nothing but whitespace has been seen, so the name is still ahead"
        );
    }

    #[test]
    fn a_dollar_at_the_end_of_a_line_opens_nothing() {
        assert_eq!(
            lines("a $\n{b}"),
            vec![vec![], vec![]],
            "the two characters of an opener have to be adjacent"
        );
    }

    #[test]
    fn two_statements_on_one_line_are_both_read() {
        assert_eq!(
            spans("${a}-${b}"),
            vec![
                ("${", Token::Punctuation),
                ("a", Token::Identifier),
                ("}", Token::Punctuation),
                ("${", Token::Punctuation),
                ("b", Token::Identifier),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn an_unclosed_statement_carries_rather_than_reddening_the_file() {
        let (spans, state) = lex("${if:key=a", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("${", Token::Punctuation),
                ("if", Token::Keyword),
                (":", Token::Operator),
                ("key", Token::Type),
                ("=", Token::Operator),
                ("a", Token::Identifier),
            ]
        );
        assert!(!state.is_start());
        assert!(
            spans.iter().all(|(_, token)| *token != Token::Error),
            "an unfinished statement is a statement being typed"
        );
    }

    #[test]
    fn an_empty_statement_paints_only_its_braces() {
        assert_eq!(
            spans("${}"),
            vec![("${", Token::Punctuation), ("}", Token::Punctuation)]
        );
        assert_eq!(
            spans("${   }"),
            vec![("${", Token::Punctuation), ("}", Token::Punctuation)]
        );
    }

    #[test]
    fn a_non_ascii_key_is_one_identifier() {
        assert_eq!(
            spans("${사원_이름.camel}"),
            vec![
                ("${", Token::Punctuation),
                ("사원_이름", Token::Identifier),
                (".", Token::Operator),
                ("camel", Token::Function),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn every_state_round_trips_through_its_encoding() {
        for bits in 0u32..(1 << 9) {
            let state = State::decode(LineState(bits));
            assert_eq!(State::decode(state.encode()), state, "for {bits:#b}");
        }
        assert_eq!(State::decode(LineState::START), State::default());
        assert!(State::default().encode().is_start());
    }
}
