//! A Java highlighter.
//!
//! Close enough to [`crate::lang::clike::CLikeHighlighter`] to have been one,
//! except for the two things that make Java's grammar its own: an annotation
//! (`@Override`) is painted as a type, the same as the class name it can
//! precede, and a text block (`"""..."""`, since Java 15) is a string that
//! runs over as many lines as it needs, the same shape as
//! [`crate::lang::clike`]'s Python `'''`/`"""` but with a third rule of its
//! own — a text block's opening `"""` must be followed only by whitespace
//! before the line ends, which is what tells it apart from three consecutive
//! empty string literals, an edge case JSON-ish `""""""` would otherwise
//! confuse this lexer about.
//!
//! Everything else is the shape [`SqlHighlighter`](crate::sql_syntax::SqlHighlighter)
//! set: a word is a keyword if [`KEYWORDS`] says so, a call if a `(` follows
//! it, a type if it starts with an ASCII uppercase letter, and an identifier
//! otherwise; `//` and `/* */` are comments; `'...'` and `"..."` are strings
//! that do not span a line break; numbers take a hex prefix, digit
//! separators, and a trailing type suffix (`1_000L`, `0xFFL`, `1.0f`).

use crate::highlight::{Highlighter, LineState, Span, Token};

/// Java's reserved words, sorted for a binary search. Includes the
/// contextual keywords (`var`, `record`, `sealed`, `permits`, `yield`) that a
/// highlighter has no parser to tell from an identifier used the ordinary
/// way, and paints as keywords always -- the same simplification
/// [`crate::sql_syntax`] makes for SQL's reserved words.
pub const KEYWORDS: &[&str] = &[
    "abstract",
    "assert",
    "boolean",
    "break",
    "byte",
    "case",
    "catch",
    "char",
    "class",
    "const",
    "continue",
    "default",
    "do",
    "double",
    "else",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "float",
    "for",
    "goto",
    "if",
    "implements",
    "import",
    "instanceof",
    "int",
    "interface",
    "long",
    "native",
    "new",
    "null",
    "package",
    "permits",
    "private",
    "protected",
    "public",
    "record",
    "return",
    "sealed",
    "short",
    "static",
    "strictfp",
    "super",
    "switch",
    "synchronized",
    "this",
    "throw",
    "throws",
    "transient",
    "true",
    "try",
    "var",
    "void",
    "volatile",
    "while",
    "yield",
];

fn is_keyword(word: &str) -> bool {
    KEYWORDS.binary_search(&word).is_ok()
}

/// What the lexer was in the middle of when a line ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Normal,
    BlockComment,
    TextBlock,
}

impl State {
    const fn decode(state: LineState) -> Self {
        match state.0 {
            1 => Self::BlockComment,
            2 => Self::TextBlock,
            _ => Self::Normal,
        }
    }

    const fn encode(self) -> LineState {
        LineState(match self {
            Self::Normal => 0,
            Self::BlockComment => 1,
            Self::TextBlock => 2,
        })
    }
}

/// Java.
///
/// A unit struct: there is nothing to configure, and one `Arc` of it can be
/// shared by every `.java` template tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JavaHighlighter;

impl Highlighter for JavaHighlighter {
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
        Some("//")
    }
}

struct Lexer<'a> {
    bytes: &'a [u8],
    text: &'a str,
    at: usize,
    spans: Vec<Span>,
}

impl Lexer<'_> {
    fn run(&mut self, start: State) -> State {
        let mut state = match start {
            State::Normal => State::Normal,
            State::BlockComment => {
                if self.block_comment_body(0) {
                    State::Normal
                } else {
                    return State::BlockComment;
                }
            }
            State::TextBlock => {
                if self.text_block_body(0) {
                    State::Normal
                } else {
                    return State::TextBlock;
                }
            }
        };

        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            match byte {
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'/' if self.starts_with(b"//") => {
                    self.push(self.at, self.bytes.len(), Token::Comment);
                    self.at = self.bytes.len();
                }
                b'/' if self.starts_with(b"/*") => {
                    let start = self.at;
                    self.at += 2;
                    if !self.block_comment_body(start) {
                        state = State::BlockComment;
                        break;
                    }
                }
                b'"' if self.starts_with(b"\"\"\"") && self.opens_text_block() => {
                    let start = self.at;
                    self.at += 3;
                    self.skip_blanks();
                    self.at = self.bytes.len();
                    self.push(start, self.at, Token::String);
                    state = State::TextBlock;
                    break;
                }
                b'"' | b'\'' => self.quoted(byte),
                b'@' => self.annotation(),
                b'0'..=b'9' => self.number(),
                b';' | b',' | b'.' | b'(' | b')' | b'[' | b']' | b'{' | b'}' => {
                    self.push(self.at, self.at + 1, Token::Punctuation);
                    self.at += 1;
                }
                b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'=' | b'!' | b'|' | b'&'
                | b'^' | b'~' | b':' | b'?' => self.operator(),
                _ => {
                    if !self.word() {
                        self.at = self.next_char_boundary(self.at);
                    }
                }
            }
        }
        state
    }

    /// Whether the `"""` at the cursor opens a text block: nothing but
    /// whitespace may follow it before the line ends, which is the rule that
    /// tells a text block's opener apart from three abutting empty strings.
    fn opens_text_block(&self) -> bool {
        self.bytes[self.at + 3..]
            .iter()
            .all(u8::is_ascii_whitespace)
    }

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
                    | b'?'
            )
        {
            self.at += 1;
        }
        self.push(start, self.at, Token::Operator);
    }

    /// An annotation: `@` followed immediately -- no space -- by a name,
    /// painted whole as a type. A lone `@` with nothing after it, or with
    /// blank space, is just punctuation: nothing in Java writes one, but a
    /// half-typed line should not paint the same token colour as a real one.
    fn annotation(&mut self) {
        let start = self.at;
        self.at += 1;
        let word_start = self.at;
        while self.at < self.bytes.len() {
            let rest = &self.text[self.at..];
            let Some(c) = rest.chars().next() else {
                break;
            };
            if (self.at == word_start && (c.is_alphabetic() || c == '_'))
                || (self.at > word_start && (c.is_alphanumeric() || c == '_' || c == '.'))
            {
                self.at += c.len_utf8();
            } else {
                break;
            }
        }
        if self.at > word_start {
            self.push(start, self.at, Token::Type);
        } else {
            self.push(start, self.at, Token::Punctuation);
        }
    }

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

    fn next_is_open_paren(&self) -> bool {
        self.bytes[self.at..]
            .iter()
            .find(|byte| !matches!(byte, b' ' | b'\t'))
            == Some(&b'(')
    }

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
            // A trailing type suffix -- `0xFFL` -- is still one number.
            while self
                .bytes
                .get(self.at)
                .is_some_and(|b| b.is_ascii_alphanumeric())
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
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_alphanumeric())
        {
            self.at += 1;
        }
        self.push(start, self.at, Token::Number);
    }

    fn digits(&mut self) {
        while self
            .bytes
            .get(self.at)
            .is_some_and(|b| b.is_ascii_digit() || *b == b'_')
        {
            self.at += 1;
        }
    }

    /// A `'...'` or `"..."` run. Never spans a line break: Java requires the
    /// closing quote on the same line for either.
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

    fn block_comment_body(&mut self, start: usize) -> bool {
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

    /// Scans to the closing `"""` of a text block, painting from `start`.
    fn text_block_body(&mut self, start: usize) -> bool {
        while self.at < self.bytes.len() {
            if self.starts_with(b"\"\"\"") {
                self.at += 3;
                self.push(start, self.at, Token::String);
                return true;
            }
            self.at += 1;
        }
        self.push(start, self.bytes.len(), Token::String);
        false
    }

    fn skip_blanks(&mut self) {
        while self.at < self.bytes.len() && self.bytes[self.at].is_ascii_whitespace() {
            self.at += 1;
        }
    }

    fn starts_with(&self, needle: &[u8]) -> bool {
        self.bytes[self.at..].starts_with(needle)
    }

    fn next_char_boundary(&self, at: usize) -> usize {
        let mut next = at + 1;
        while next < self.text.len() && !self.text.is_char_boundary(next) {
            next += 1;
        }
        next.min(self.text.len())
    }

    fn push(&mut self, start: usize, end: usize, token: Token) {
        if end > start {
            self.spans.push(Span::new(start..end, token));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::test_support::{lex, lex_lines};

    fn h() -> JavaHighlighter {
        JavaHighlighter
    }

    #[test]
    fn the_keyword_table_is_sorted() {
        for pair in KEYWORDS.windows(2) {
            assert!(pair[0] < pair[1], "{pair:?} is out of order");
        }
    }

    #[test]
    fn a_class_declaration_reads_as_expected() {
        let (spans, _) = lex(&h(), "public class Foo extends Bar {", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("public", Token::Keyword),
                ("class", Token::Keyword),
                ("Foo", Token::Type),
                ("extends", Token::Keyword),
                ("Bar", Token::Type),
                ("{", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn an_annotation_is_a_type() {
        let (spans, _) = lex(&h(), "@Override public void run() {", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("@Override", Token::Type),
                ("public", Token::Keyword),
                ("void", Token::Keyword),
                ("run", Token::Function),
                ("(", Token::Punctuation),
                (")", Token::Punctuation),
                ("{", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_call_is_a_function_and_a_lower_case_identifier_stays_one() {
        let (spans, _) = lex(&h(), "count(items)", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("count", Token::Function),
                ("(", Token::Punctuation),
                ("items", Token::Identifier),
                (")", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn line_and_block_comments() {
        let (spans, _) = lex(&h(), "int x; // trailing", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("int", Token::Keyword),
                ("x", Token::Identifier),
                (";", Token::Punctuation),
                ("// trailing", Token::Comment),
            ]
        );

        let (first, state) = lex(&h(), "/* open", LineState::START);
        assert_eq!(first, vec![("/* open", Token::Comment)]);
        assert!(!state.is_start());
        let (second, state) = lex(&h(), "still */ int y;", state);
        assert_eq!(
            second,
            vec![
                ("still */", Token::Comment),
                ("int", Token::Keyword),
                ("y", Token::Identifier),
                (";", Token::Punctuation),
            ]
        );
        assert!(state.is_start());
    }

    #[test]
    fn strings_and_chars_never_span_a_line() {
        let (spans, state) = lex(&h(), "String s = \"unterminated", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("String", Token::Type),
                ("s", Token::Identifier),
                ("=", Token::Operator),
                ("\"unterminated", Token::String),
            ]
        );
        assert!(state.is_start());

        let (spans, _) = lex(&h(), "char c = '\\n';", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("char", Token::Keyword),
                ("c", Token::Identifier),
                ("=", Token::Operator),
                ("'\\n'", Token::String),
                (";", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_text_block_spans_lines_and_closes() {
        let lines = lex_lines(&h(), "String s = \"\"\"\n  hello\n  \"\"\";");
        assert_eq!(
            lines[0],
            vec![
                ("String", Token::Type),
                ("s", Token::Identifier),
                ("=", Token::Operator),
                ("\"\"\"", Token::String),
            ]
        );
        assert_eq!(lines[1], vec![("  hello", Token::String)]);
        assert_eq!(
            lines[2],
            vec![("  \"\"\"", Token::String), (";", Token::Punctuation)]
        );
    }

    #[test]
    fn six_quotes_read_as_three_empty_strings_not_a_text_block() {
        // A text block's opening `"""` must be followed by nothing but
        // whitespace before the line ends; a fourth quote right after it
        // fails that, so this reads the ordinary way: an empty string,
        // three times over.
        let (spans, state) = lex(&h(), "\"\"\"\"\"\"", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("\"\"", Token::String),
                ("\"\"", Token::String),
                ("\"\"", Token::String),
            ]
        );
        assert!(state.is_start());
    }

    #[test]
    fn numbers_carry_a_suffix_and_a_hex_prefix() {
        let (spans, _) = lex(&h(), "0xFFL, 1_000, 1.5f", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("0xFFL", Token::Number),
                (",", Token::Punctuation),
                ("1_000", Token::Number),
                (",", Token::Punctuation),
                ("1.5f", Token::Number),
            ]
        );
    }
}
