//! A small SQL highlighter, for the custom-query editors.
//!
//! rudbman's editor took its tokens from `rudbman-sql`, a lexer with a dialect
//! table behind it, because there the editor is a SQL console and every
//! difference between MySQL's `#` and PostgreSQL's `$$` mattered. Nothing here
//! writes SQL to a server: the only SQL rudbgen holds is the four per-driver
//! metadata overrides of the driver editor (architecture document, D9), each of
//! them a single `select` a person reads more often than they write. So this is
//! a lexer of two hundred lines with no dialect at all, and it takes the union
//! of what the drivers agree on:
//!
//! * `'...'`, with `''` for an embedded quote, is a string, and it may run over
//!   a line break;
//! * `"..."` and `` `...` `` are quoted identifiers, likewise;
//! * `--` runs to the end of the line and `/* ... */` over as many lines as it
//!   needs;
//! * a word is a keyword if it is one of [`KEYWORDS`], a function if a `(`
//!   follows it, and an identifier otherwise;
//! * `${...}` is a placeholder — the driver editor substitutes catalog and
//!   schema into a custom query the same way the connection URL substitutes a
//!   host — and it is painted in the type colour so that it stands out of the
//!   SQL around it rather than blending into a string.
//!
//! What it deliberately does not do is anything a parser would be needed for:
//! no type names (a `varchar` reads as an identifier), no dollar-quoted bodies,
//! no `DELIMITER`. A custom query that needs any of them has outgrown the four
//! text boxes it is typed into.

use crate::highlight::{Highlighter, LineState, Span, Token};

/// The reserved words painted as keywords.
///
/// The intersection of what the ten stock drivers call reserved, kept
/// lower-case and sorted so that the lookup is a binary search over a `&str`
/// slice rather than a hash. Sorted-ness is held down by a test.
pub const KEYWORDS: &[&str] = &[
    "add",
    "all",
    "alter",
    "analyze",
    "and",
    "any",
    "as",
    "asc",
    "before",
    "begin",
    "between",
    "both",
    "by",
    "call",
    "cascade",
    "case",
    "cast",
    "check",
    "collate",
    "column",
    "commit",
    "constraint",
    "create",
    "cross",
    "current",
    "cursor",
    "database",
    "declare",
    "default",
    "delete",
    "desc",
    "describe",
    "distinct",
    "drop",
    "each",
    "else",
    "end",
    "escape",
    "except",
    "exists",
    "explain",
    "false",
    "fetch",
    "first",
    "for",
    "foreign",
    "from",
    "full",
    "grant",
    "group",
    "having",
    "if",
    "ilike",
    "in",
    "index",
    "inner",
    "insert",
    "intersect",
    "into",
    "is",
    "join",
    "key",
    "left",
    "like",
    "limit",
    "not",
    "null",
    "nulls",
    "offset",
    "on",
    "only",
    "or",
    "order",
    "outer",
    "over",
    "partition",
    "primary",
    "procedure",
    "references",
    "rename",
    "replace",
    "restrict",
    "returning",
    "revoke",
    "right",
    "rollback",
    "row",
    "rows",
    "select",
    "set",
    "show",
    "some",
    "table",
    "then",
    "to",
    "top",
    "trigger",
    "true",
    "truncate",
    "union",
    "unique",
    "unknown",
    "update",
    "using",
    "values",
    "view",
    "when",
    "where",
    "while",
    "window",
    "with",
];

/// Whether `word` is one of [`KEYWORDS`], whatever case it is written in.
fn is_keyword(word: &str) -> bool {
    // `to_ascii_lowercase` rather than `to_lowercase`: every keyword is ASCII,
    // and the Turkish dotless i would otherwise make `LIMIT` fail to match in a
    // locale-aware fold.
    let lowered = word.to_ascii_lowercase();
    KEYWORDS.binary_search(&lowered.as_str()).is_ok()
}

/// What the lexer was in the middle of when a line ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing is open.
    Normal,
    /// Inside `/* ... */`.
    BlockComment,
    /// Inside `'...'`.
    String,
    /// Inside `"..."`.
    DoubleQuoted,
    /// Inside `` `...` ``.
    BackQuoted,
}

impl State {
    /// Reads a [`LineState`] back. An unknown one is [`State::Normal`], which
    /// is what a highlighter swap leaves behind for one frame.
    const fn decode(state: LineState) -> Self {
        match state.0 {
            1 => Self::BlockComment,
            2 => Self::String,
            3 => Self::DoubleQuoted,
            4 => Self::BackQuoted,
            _ => Self::Normal,
        }
    }

    /// The opaque form the cache stores.
    const fn encode(self) -> LineState {
        LineState(match self {
            Self::Normal => 0,
            Self::BlockComment => 1,
            Self::String => 2,
            Self::DoubleQuoted => 3,
            Self::BackQuoted => 4,
        })
    }

    /// The quote that closes this state, for the three quoted ones.
    const fn quote(self) -> Option<u8> {
        match self {
            Self::String => Some(b'\''),
            Self::DoubleQuoted => Some(b'"'),
            Self::BackQuoted => Some(b'`'),
            _ => None,
        }
    }

    /// What a run in this state is painted as.
    const fn token(self) -> Token {
        match self {
            Self::String => Token::String,
            Self::DoubleQuoted | Self::BackQuoted => Token::Identifier,
            _ => Token::Comment,
        }
    }
}

/// SQL, as much of it as a custom query needs.
///
/// A unit struct: there is nothing to configure, and one `Arc` of it can be
/// shared by every editor in the driver dialog.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SqlHighlighter;

impl Highlighter for SqlHighlighter {
    fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState) {
        let mut lexer = Lexer {
            bytes: text.as_bytes(),
            text,
            at: 0,
            spans: Vec::new(),
        };
        let end = lexer.run(State::decode(state));
        (lexer.spans, end.encode())
    }

    fn line_comment(&self) -> Option<&'static str> {
        Some("--")
    }

    fn statements(&self) -> bool {
        true
    }
}

/// One line's worth of scanning.
struct Lexer<'a> {
    bytes: &'a [u8],
    text: &'a str,
    at: usize,
    spans: Vec<Span>,
}

impl Lexer<'_> {
    /// Scans the whole line and answers with the state it ends in.
    fn run(&mut self, start: State) -> State {
        let mut state = match start {
            State::Normal => State::Normal,
            State::BlockComment => {
                if self.block_comment_body() {
                    State::Normal
                } else {
                    return State::BlockComment;
                }
            }
            quoted => {
                let quote = quoted.quote().expect("only the quoted states get here");
                if self.quoted_body(quote, quoted.token()) {
                    State::Normal
                } else {
                    return quoted;
                }
            }
        };

        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            match byte {
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'-' if self.starts_with(b"--") => {
                    self.push(self.at, self.bytes.len(), Token::Comment);
                    self.at = self.bytes.len();
                }
                b'/' if self.starts_with(b"/*") => {
                    let start = self.at;
                    self.at += 2;
                    let closed = self.block_comment_body_from(start);
                    if !closed {
                        state = State::BlockComment;
                        break;
                    }
                }
                b'\'' | b'"' | b'`' => {
                    let opened = match byte {
                        b'\'' => State::String,
                        b'"' => State::DoubleQuoted,
                        _ => State::BackQuoted,
                    };
                    let start = self.at;
                    self.at += 1;
                    if !self.quoted_body_from(start, byte, opened.token()) {
                        state = opened;
                        break;
                    }
                }
                b'$' if self.starts_with(b"${") => {
                    let start = self.at;
                    self.at += 2;
                    while self.at < self.bytes.len() && self.bytes[self.at] != b'}' {
                        self.at += 1;
                    }
                    // A placeholder that is not closed on this line is still a
                    // placeholder: it opens no state, because `}` is not a
                    // character a person leaves dangling over a line break in a
                    // one-line query.
                    self.at = (self.at + 1).min(self.bytes.len());
                    self.push(start, self.at, Token::Type);
                }
                b'0'..=b'9' => self.number(),
                b';' | b',' | b'.' | b'(' | b')' | b'[' | b']' | b'{' | b'}' => {
                    self.push(self.at, self.at + 1, Token::Punctuation);
                    self.at += 1;
                }
                b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'|' | b'&'
                | b'^' | b'~' | b':' => {
                    let start = self.at;
                    while self.at < self.bytes.len()
                        && matches!(
                            self.bytes[self.at],
                            b'+' | b'-'
                                | b'*'
                                | b'/'
                                | b'%'
                                | b'<'
                                | b'>'
                                | b'='
                                | b'!'
                                | b'|'
                                | b'&'
                                | b'^'
                                | b'~'
                                | b':'
                        )
                    {
                        self.at += 1;
                    }
                    self.push(start, self.at, Token::Operator);
                }
                _ => {
                    if self.word() {
                        continue;
                    }
                    // Something the lexer has no opinion about: leave it
                    // uncoloured and step over one whole character, never one
                    // byte of one.
                    self.at = self.next_char_boundary(self.at);
                }
            }
        }
        state
    }

    /// Reads a word and classifies it. `false` when there is no word here.
    fn word(&mut self) -> bool {
        let start = self.at;
        let mut chars = self.text[start..].char_indices();
        let Some((_, first)) = chars.next() else {
            return false;
        };
        if !(first.is_alphabetic() || first == '_') {
            return false;
        }
        let mut end = start + first.len_utf8();
        for (offset, c) in chars {
            if c.is_alphanumeric() || c == '_' {
                end = start + offset + c.len_utf8();
            } else {
                break;
            }
        }
        self.at = end;
        let word = &self.text[start..end];
        let token = if is_keyword(word) {
            Token::Keyword
        } else if self.next_is_open_paren() {
            Token::Function
        } else {
            Token::Identifier
        };
        self.push(start, end, token);
        true
    }

    /// Whether the next non-blank byte opens a call.
    fn next_is_open_paren(&self) -> bool {
        self.bytes[self.at..]
            .iter()
            .find(|byte| !matches!(byte, b' ' | b'\t'))
            == Some(&b'(')
    }

    /// Reads a numeric literal: digits, an optional fraction, an optional
    /// exponent.
    fn number(&mut self) {
        let start = self.at;
        self.digits();
        if self.bytes.get(self.at) == Some(&b'.')
            && self.bytes.get(self.at + 1).is_some_and(u8::is_ascii_digit)
        {
            self.at += 1;
            self.digits();
        }
        if matches!(self.bytes.get(self.at), Some(b'e' | b'E')) {
            let mut ahead = self.at + 1;
            if matches!(self.bytes.get(ahead), Some(b'+' | b'-')) {
                ahead += 1;
            }
            if self.bytes.get(ahead).is_some_and(u8::is_ascii_digit) {
                self.at = ahead;
                self.digits();
            }
        }
        self.push(start, self.at, Token::Number);
    }

    /// Steps over a run of digits.
    fn digits(&mut self) {
        while self.bytes.get(self.at).is_some_and(u8::is_ascii_digit) {
            self.at += 1;
        }
    }

    /// Scans a block comment that was opened on an earlier line.
    fn block_comment_body(&mut self) -> bool {
        self.block_comment_body_from(0)
    }

    /// Scans to `*/`, painting from `start`. `false` when the line ends first.
    fn block_comment_body_from(&mut self, start: usize) -> bool {
        while self.at < self.bytes.len() {
            if self.starts_with(b"*/") {
                self.at += 2;
                self.push(start, self.at, Token::Comment);
                return true;
            }
            self.at += 1;
        }
        self.push(start, self.bytes.len(), Token::Comment);
        false
    }

    /// Scans a quoted run that was opened on an earlier line.
    fn quoted_body(&mut self, quote: u8, token: Token) -> bool {
        self.quoted_body_from(0, quote, token)
    }

    /// Scans to the closing `quote`, painting from `start`.
    ///
    /// A doubled quote is an embedded one and does not close the run, which is
    /// the escape every dialect here agrees on. `false` when the line ends
    /// first.
    fn quoted_body_from(&mut self, start: usize, quote: u8, token: Token) -> bool {
        while self.at < self.bytes.len() {
            if self.bytes[self.at] == quote {
                if self.bytes.get(self.at + 1) == Some(&quote) {
                    self.at += 2;
                    continue;
                }
                self.at += 1;
                self.push(start, self.at, token);
                return true;
            }
            self.at += 1;
        }
        self.push(start, self.bytes.len(), token);
        false
    }

    /// Whether the line reads `needle` at the cursor.
    fn starts_with(&self, needle: &[u8]) -> bool {
        self.bytes[self.at..].starts_with(needle)
    }

    /// The next character boundary strictly after `at`.
    fn next_char_boundary(&self, at: usize) -> usize {
        let mut next = at + 1;
        while next < self.text.len() && !self.text.is_char_boundary(next) {
            next += 1;
        }
        next.min(self.text.len())
    }

    /// Records a span, dropping empty ones.
    fn push(&mut self, start: usize, end: usize, token: Token) {
        if end > start {
            self.spans.push(Span::new(start..end, token));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(text, token)` for every span of `line`, lexed from `state`.
    fn lex(line: &str, state: LineState) -> (Vec<(&str, Token)>, LineState) {
        let (spans, end) = SqlHighlighter.line(line, state);
        let mut last = 0;
        for span in &spans {
            assert!(span.range.start >= last, "spans overlap in {line:?}");
            assert!(
                span.range.end <= line.len(),
                "span past the end of {line:?}"
            );
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

    /// The spans of a line lexed from the start state.
    fn spans(line: &str) -> Vec<(&str, Token)> {
        lex(line, LineState::START).0
    }

    #[test]
    fn the_keyword_table_is_sorted_and_lower_case() {
        for pair in KEYWORDS.windows(2) {
            assert!(pair[0] < pair[1], "{pair:?} is out of order");
        }
        for word in KEYWORDS {
            assert_eq!(*word, word.to_ascii_lowercase());
        }
    }

    #[test]
    fn keywords_are_case_insensitive_and_identifiers_are_not_keywords() {
        assert_eq!(
            spans("SELECT a From t"),
            vec![
                ("SELECT", Token::Keyword),
                ("a", Token::Identifier),
                ("From", Token::Keyword),
                ("t", Token::Identifier),
            ]
        );
        assert_eq!(
            spans("selected"),
            vec![("selected", Token::Identifier)],
            "a word that merely starts with a keyword is not one"
        );
    }

    #[test]
    fn a_word_before_a_paren_is_a_call() {
        assert_eq!(
            spans("count (x)"),
            vec![
                ("count", Token::Function),
                ("(", Token::Punctuation),
                ("x", Token::Identifier),
                (")", Token::Punctuation),
            ]
        );
        assert_eq!(
            spans("select (1)"),
            vec![
                ("select", Token::Keyword),
                ("(", Token::Punctuation),
                ("1", Token::Number),
                (")", Token::Punctuation),
            ],
            "a keyword before a paren is still a keyword"
        );
    }

    #[test]
    fn a_doubled_quote_does_not_close_a_string() {
        assert_eq!(
            spans("'it''s'"),
            vec![("'it''s'", Token::String)],
            "the whole literal is one run"
        );
        let (first, state) = lex("'open", LineState::START);
        assert_eq!(first, vec![("'open", Token::String)]);
        assert!(!state.is_start());
        let (second, state) = lex("still' , 1", state);
        assert_eq!(
            second,
            vec![
                ("still'", Token::String),
                (",", Token::Punctuation),
                ("1", Token::Number),
            ]
        );
        assert!(state.is_start());
    }

    #[test]
    fn quoted_identifiers_read_as_identifiers() {
        assert_eq!(
            spans(r#"select "my col", `other` from t"#),
            vec![
                ("select", Token::Keyword),
                (r#""my col""#, Token::Identifier),
                (",", Token::Punctuation),
                ("`other`", Token::Identifier),
                ("from", Token::Keyword),
                ("t", Token::Identifier),
            ]
        );
    }

    #[test]
    fn comments_run_to_the_end_of_the_line_and_over_line_breaks() {
        assert_eq!(
            spans("select 1 -- and ' a quote"),
            vec![
                ("select", Token::Keyword),
                ("1", Token::Number),
                ("-- and ' a quote", Token::Comment),
            ]
        );

        let (first, state) = lex("/* open", LineState::START);
        assert_eq!(first, vec![("/* open", Token::Comment)]);
        assert!(!state.is_start());
        let (middle, state) = lex("select 1", state);
        assert_eq!(middle, vec![("select 1", Token::Comment)]);
        assert!(!state.is_start());
        let (last, state) = lex("still */ select", state);
        assert_eq!(
            last,
            vec![("still */", Token::Comment), ("select", Token::Keyword)]
        );
        assert!(state.is_start());
    }

    #[test]
    fn a_block_comment_that_opens_and_closes_on_one_line_opens_no_state() {
        let (spans, state) = lex("select /* x */ 1", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("select", Token::Keyword),
                ("/* x */", Token::Comment),
                ("1", Token::Number),
            ]
        );
        assert!(state.is_start());
    }

    #[test]
    fn numbers_carry_their_fraction_and_exponent() {
        assert_eq!(
            spans("1, 2.5, 3e10, 4.5E-3, 6."),
            vec![
                ("1", Token::Number),
                (",", Token::Punctuation),
                ("2.5", Token::Number),
                (",", Token::Punctuation),
                ("3e10", Token::Number),
                (",", Token::Punctuation),
                ("4.5E-3", Token::Number),
                (",", Token::Punctuation),
                ("6", Token::Number),
                (".", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_placeholder_is_painted_as_a_type() {
        assert_eq!(
            spans("select 1 from ${schema}.t"),
            vec![
                ("select", Token::Keyword),
                ("1", Token::Number),
                ("from", Token::Keyword),
                ("${schema}", Token::Type),
                (".", Token::Punctuation),
                ("t", Token::Identifier),
            ]
        );
    }

    #[test]
    fn operators_clump_and_a_semicolon_does_not() {
        assert_eq!(
            spans("a <> b;"),
            vec![
                ("a", Token::Identifier),
                ("<>", Token::Operator),
                ("b", Token::Identifier),
                (";", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_non_ascii_identifier_is_one_word() {
        assert_eq!(
            spans("select 사원_이름 from t"),
            vec![
                ("select", Token::Keyword),
                ("사원_이름", Token::Identifier),
                ("from", Token::Keyword),
                ("t", Token::Identifier),
            ]
        );
    }

    #[test]
    fn an_unclassifiable_character_is_stepped_over_whole() {
        // A stray `¶` colours nothing and must not split a UTF-8 sequence.
        assert_eq!(spans("¶ a"), vec![("a", Token::Identifier)]);
    }
}
