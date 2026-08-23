//! One lexer, reused for every language that only differs from C in its
//! keyword table and its comment and string syntax.
//!
//! [`SqlHighlighter`](crate::sql_syntax::SqlHighlighter) proved the shape: a
//! word is a keyword if a table says so, a call if a `(` follows it, and an
//! identifier otherwise; a quote opens a string; `//` or `#` runs to the end
//! of the line; `/* */` runs over as many lines as it needs. Every language in
//! [`crate::lang`] whose grammar is that shape and nothing more — C#, Kotlin,
//! TypeScript and JavaScript, Go, Rust, Python, JSON, YAML, Markdown,
//! properties and INI files, and shell scripts — is this same lexer with a
//! different [`CLikeConfig`], rather than a lexer of its own.
//!
//! # What is deliberately the same for every language here
//!
//! An identifier that starts with an ASCII uppercase letter is painted as a
//! [`Token::Type`] rather than looked up as a call, whatever the language: it
//! is the naming convention of most of them, and it costs nothing on a
//! language where it happens not to hold — a `.json` file has no bare
//! identifiers to begin with. This is a fixed rule of the lexer, not a
//! per-language switch, on purpose: the whole point of one shared lexer is
//! that only the *table* and the *comment and string syntax* change from one
//! language to the next.
//!
//! # What no configuration here can express
//!
//! A single- or double-quoted string never spans a line break — that is true
//! of every language configured here, [`PhpHighlighter`](crate::lang::php)'s
//! multi-line strings being the one exception in this crate, which is why PHP
//! has a lexer of its own. Raw strings (Rust's `r"..."`, Go's `` `...` ``),
//! Rust's byte-string and lifetime syntax, and C#'s and JavaScript's string
//! interpolation (`$"{x}"`, `` `${x}` ``) are not tokenized specially; the
//! interpolation case matters least of all of these, because a template file
//! opens exactly the `${...}` sequence for its own statements, and
//! [`CompositeHighlighter`](crate::composite::CompositeHighlighter) always
//! paints that as the template's, whatever the base language would have made
//! of it.

use crate::highlight::{Highlighter, LineState, Span, Token};

/// What one C-like language's [`CLikeHighlighter`] needs told about it.
#[derive(Debug, Clone, Copy)]
pub struct CLikeConfig {
    /// Reserved words, matched case-sensitively. Sorted, for a binary search;
    /// held down by a `the_keywords_are_sorted` test next to every table this
    /// module ships.
    pub keywords: &'static [&'static str],
    /// The prefixes that open a line comment: `&["//"]`, `&["#"]`,
    /// `&["#", "!"]`, or `&[]` for a language with none.
    pub line_comments: &'static [&'static str],
    /// The pair that opens and closes a block comment, or `None` for a
    /// language with no such thing.
    pub block_comment: Option<(&'static str, &'static str)>,
    /// The triple-quote delimiters that open a string running over as many
    /// lines as it needs — Python's `'''` and `"""` — or `&[]` for a language
    /// with no such string. At most two: [`State`] has one bit to say which.
    pub triple_quotes: &'static [&'static str],
}

/// What the lexer was in the middle of when a line ended.
///
/// Two bits: [`LineState::COMPOSABLE_BITS`] is sixteen, and this uses two.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Nothing open.
    Normal,
    /// Inside the language's block comment.
    BlockComment,
    /// Inside `config.triple_quotes[0]`.
    TripleQuote0,
    /// Inside `config.triple_quotes[1]`.
    TripleQuote1,
}

impl State {
    const fn decode(state: LineState) -> Self {
        match state.0 {
            1 => Self::BlockComment,
            2 => Self::TripleQuote0,
            3 => Self::TripleQuote1,
            _ => Self::Normal,
        }
    }

    const fn encode(self) -> LineState {
        LineState(match self {
            Self::Normal => 0,
            Self::BlockComment => 1,
            Self::TripleQuote0 => 2,
            Self::TripleQuote1 => 3,
        })
    }

    /// The index into `triple_quotes` this state reads from, for the two
    /// triple-quote states.
    const fn triple_index(self) -> usize {
        match self {
            Self::TripleQuote1 => 1,
            _ => 0,
        }
    }
}

/// A hand-written lexer for one C-like language, chosen by [`CLikeConfig`].
///
/// Not a unit struct, because the config is what makes one language different
/// from the next: [`crate::lang`] builds one `&'static CLikeConfig` per
/// language and wraps it in one of these.
#[derive(Debug, Clone, Copy)]
pub struct CLikeHighlighter(pub &'static CLikeConfig);

impl Highlighter for CLikeHighlighter {
    fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState) {
        let mut lexer = Lexer {
            config: self.0,
            bytes: text.as_bytes(),
            text,
            at: 0,
            spans: Vec::new(),
        };
        let end = lexer.run(State::decode(state));
        (lexer.spans, end.encode())
    }

    fn line_comment(&self) -> Option<&'static str> {
        self.0.line_comments.first().copied()
    }
}

/// One line's worth of scanning.
struct Lexer<'a> {
    config: &'static CLikeConfig,
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
                let (_, close) = self
                    .config
                    .block_comment
                    .expect("a block comment state implies the language has one");
                if self.block_comment_body_from(0, close) {
                    State::Normal
                } else {
                    return State::BlockComment;
                }
            }
            triple @ (State::TripleQuote0 | State::TripleQuote1) => {
                if self.triple_quote_body_from(0, triple.triple_index()) {
                    State::Normal
                } else {
                    return triple;
                }
            }
        };

        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            if byte == b' ' || byte == b'\t' || byte == b'\r' {
                self.at += 1;
                continue;
            }
            if let Some(rest) = self.line_comment_prefix() {
                self.push(self.at, self.bytes.len(), Token::Comment);
                self.at = self.bytes.len();
                let _ = rest;
                continue;
            }
            if let Some((open, close)) = self.config.block_comment
                && self.starts_with(open.as_bytes())
            {
                let start = self.at;
                self.at += open.len();
                if !self.block_comment_body_from(start, close) {
                    state = State::BlockComment;
                    break;
                }
                continue;
            }
            if let Some(idx) = self.triple_quote_opening() {
                let start = self.at;
                self.at += self.config.triple_quotes[idx].len();
                if !self.triple_quote_body_from(start, idx) {
                    state = if idx == 0 {
                        State::TripleQuote0
                    } else {
                        State::TripleQuote1
                    };
                    break;
                }
                continue;
            }
            match byte {
                b'\'' | b'"' => self.quoted(byte),
                b'0'..=b'9' => self.number(),
                b';' | b',' | b'.' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'@' | b'?' => {
                    self.push(self.at, self.at + 1, Token::Punctuation);
                    self.at += 1;
                }
                b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'|' | b'&'
                | b'^' | b'~' | b':' => self.operator(),
                _ => {
                    if !self.word() {
                        self.at = self.next_char_boundary(self.at);
                    }
                }
            }
        }
        state
    }

    /// The line-comment prefix at the cursor, if one opens here.
    fn line_comment_prefix(&self) -> Option<&'static str> {
        self.config
            .line_comments
            .iter()
            .copied()
            .find(|prefix| self.starts_with(prefix.as_bytes()))
    }

    /// The index of the triple-quote delimiter at the cursor, if one opens
    /// here. Checked before a plain quote, so `"""` is not read as an empty
    /// `""` followed by a stray `"`.
    fn triple_quote_opening(&self) -> Option<usize> {
        self.config
            .triple_quotes
            .iter()
            .position(|delim| self.starts_with(delim.as_bytes()))
    }

    /// Scans a run of operator characters as one span.
    fn operator(&mut self) {
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
        let token = if self.config.keywords.binary_search(&word).is_ok() {
            Token::Keyword
        } else if word.as_bytes()[0].is_ascii_uppercase() {
            Token::Type
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

    /// Reads a numeric literal: a hex literal, or digits with an optional
    /// fraction and exponent. `_` is accepted as a digit separator throughout,
    /// as most of these languages allow.
    fn number(&mut self) {
        let start = self.at;
        if self.bytes[self.at] == b'0' && matches!(self.bytes.get(self.at + 1), Some(b'x' | b'X')) {
            self.at += 2;
            while self
                .bytes
                .get(self.at)
                .is_some_and(|b| b.is_ascii_hexdigit() || *b == b'_')
            {
                self.at += 1;
            }
            self.push(start, self.at, Token::Number);
            return;
        }
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
        // A trailing type suffix -- `1L`, `1.0f`, `1u32` -- is still one
        // number rather than a number followed by a stray identifier.
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_alphanumeric())
        {
            self.at += 1;
        }
        self.push(start, self.at, Token::Number);
    }

    /// Steps over a run of digits and `_` separators.
    fn digits(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
        {
            self.at += 1;
        }
    }

    /// Scans a quoted run. Never spans a line break: an unterminated quote
    /// simply paints to the end of the line and opens no state, which is
    /// right for every language configured here (§ module docs).
    fn quoted(&mut self, quote: u8) {
        let start = self.at;
        self.at += 1;
        let mut escaped = false;
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            self.at += 1;
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == quote {
                break;
            }
        }
        self.push(start, self.at, Token::String);
    }

    /// Scans to the closing delimiter of a block comment, painting from
    /// `start`. `false` when the line ends first.
    fn block_comment_body_from(&mut self, start: usize, close: &str) -> bool {
        let close = close.as_bytes();
        while self.at < self.bytes.len() {
            if self.starts_with(close) {
                self.at += close.len();
                self.push(start, self.at, Token::Comment);
                return true;
            }
            self.at += 1;
        }
        self.push(start, self.bytes.len(), Token::Comment);
        false
    }

    /// Scans to the closing delimiter of a triple-quoted string, painting
    /// from `start`. `false` when the line ends first.
    fn triple_quote_body_from(&mut self, start: usize, idx: usize) -> bool {
        let delim = self.config.triple_quotes[idx].as_bytes();
        while self.at < self.bytes.len() {
            if self.starts_with(delim) {
                self.at += delim.len();
                self.push(start, self.at, Token::String);
                return true;
            }
            self.at += 1;
        }
        self.push(start, self.bytes.len(), Token::String);
        false
    }

    /// Whether the line reads `needle` at the cursor.
    fn starts_with(&self, needle: &[u8]) -> bool {
        !needle.is_empty() && self.bytes[self.at..].starts_with(needle)
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
    use crate::lang::test_support::lex;

    const TEST_CONFIG: CLikeConfig = CLikeConfig {
        keywords: &["if", "return"],
        line_comments: &["//"],
        block_comment: Some(("/*", "*/")),
        triple_quotes: &[],
    };

    fn highlighter() -> CLikeHighlighter {
        CLikeHighlighter(&TEST_CONFIG)
    }

    #[test]
    fn a_keyword_is_a_keyword_and_a_call_is_a_function() {
        let (spans, _) = lex(&highlighter(), "if foo(x) { return Bar }", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("if", Token::Keyword),
                ("foo", Token::Function),
                ("(", Token::Punctuation),
                ("x", Token::Identifier),
                (")", Token::Punctuation),
                ("{", Token::Punctuation),
                ("return", Token::Keyword),
                ("Bar", Token::Type),
                ("}", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn line_and_block_comments_behave_like_sql() {
        let (spans, _) = lex(&highlighter(), "x // trailing", LineState::START);
        assert_eq!(
            spans,
            vec![("x", Token::Identifier), ("// trailing", Token::Comment)]
        );

        let (first, state) = lex(&highlighter(), "/* open", LineState::START);
        assert_eq!(first, vec![("/* open", Token::Comment)]);
        assert!(!state.is_start());
        let (second, state) = lex(&highlighter(), "still */ x", state);
        assert_eq!(
            second,
            vec![("still */", Token::Comment), ("x", Token::Identifier)]
        );
        assert!(state.is_start());
    }

    #[test]
    fn a_string_never_spans_a_line() {
        let (spans, state) = lex(&highlighter(), "\"unterminated", LineState::START);
        assert_eq!(spans, vec![("\"unterminated", Token::String)]);
        assert!(state.is_start(), "an unterminated quote opens no state");
    }

    #[test]
    fn numbers_carry_hex_and_a_trailing_suffix() {
        let (spans, _) = lex(&highlighter(), "0xFF, 1.5f, 3, 1_000", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("0xFF", Token::Number),
                (",", Token::Punctuation),
                ("1.5f", Token::Number),
                (",", Token::Punctuation),
                ("3", Token::Number),
                (",", Token::Punctuation),
                ("1_000", Token::Number),
            ]
        );
    }

    #[test]
    fn a_triple_quoted_string_spans_lines() {
        const PY: CLikeConfig = CLikeConfig {
            keywords: &[],
            line_comments: &["#"],
            block_comment: None,
            triple_quotes: &["'''", "\"\"\""],
        };
        let h = CLikeHighlighter(&PY);
        let (first, state) = lex(&h, "x = '''one", LineState::START);
        assert_eq!(
            first,
            vec![
                ("x", Token::Identifier),
                ("=", Token::Operator),
                ("'''one", Token::String)
            ]
        );
        assert!(!state.is_start());
        let (second, state) = lex(&h, "two'''", state);
        assert_eq!(second, vec![("two'''", Token::String)]);
        assert!(state.is_start());
    }

    #[test]
    fn no_language_has_a_line_comment_when_the_table_is_empty() {
        const JSON: CLikeConfig = CLikeConfig {
            keywords: &["false", "null", "true"],
            line_comments: &[],
            block_comment: None,
            triple_quotes: &[],
        };
        assert_eq!(CLikeHighlighter(&JSON).line_comment(), None);
        assert_eq!(highlighter().line_comment(), Some("//"));
    }
}
