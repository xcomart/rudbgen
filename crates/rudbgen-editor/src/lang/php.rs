//! A PHP highlighter.
//!
//! A `.php` template is, like a `.java` or `.xml` one, a file in some other
//! language with jdbgen's `${…}` statements sprinkled through it -- except
//! that PHP's own source is already two languages in one file: HTML outside
//! `<?php ... ?>`, and PHP inside it. This lexer keeps that boundary too, so
//! that a `.php.tpl` generator that emits a page rather than a pure PHP class
//! still gets its markup left alone (uncoloured, the same as prose in a
//! template) and its code coloured.
//!
//! Inside PHP:
//!
//! * `//`, `#` and `/* ... */` are comments, the block form spanning as many
//!   lines as it needs.
//! * `'...'` and `"..."` are strings, and -- unlike every language in
//!   [`crate::lang::clike`] -- they may run over a line break: real PHP
//!   allows a literal newline inside either quote without escaping it, and a
//!   highlighter that reddened the rest of the file over one would be worse
//!   than wrong.
//! * `$name` is one identifier, dollar sign included.
//! * a word is a keyword if [`KEYWORDS`] says so, a type if it starts with an
//!   ASCII uppercase letter (PHP's class-naming convention), a call if a `(`
//!   follows it, and an identifier otherwise.
//! * `->` and `::` need no special case: both are runs of the same operator
//!   characters `-`, `>` and `:` that the generic operator scan already
//!   clumps into one span.
//!
//! What it does not attempt: heredoc and nowdoc (`<<<EOT ... EOT`), variable
//! interpolation *inside* a double-quoted string (`"$name"`, `"{$expr}"`
//! stay plain string colour throughout), and the historical short open tag
//! `<?` on its own -- only `<?php` and the short echo tag `<?=` are
//! recognized as opening PHP.

use crate::highlight::{Highlighter, LineState, Span, Token};

/// PHP's reserved words, sorted for a binary search. Matched case-sensitively
/// in lower case, which is how idiomatic PHP writes them, even though the
/// language itself does not care about the case of a keyword.
pub const KEYWORDS: &[&str] = &[
    "abstract",
    "and",
    "array",
    "as",
    "break",
    "callable",
    "case",
    "catch",
    "class",
    "clone",
    "const",
    "continue",
    "declare",
    "default",
    "do",
    "echo",
    "else",
    "elseif",
    "empty",
    "enddeclare",
    "endfor",
    "endforeach",
    "endif",
    "endswitch",
    "endwhile",
    "enum",
    "extends",
    "false",
    "final",
    "finally",
    "fn",
    "for",
    "foreach",
    "function",
    "global",
    "goto",
    "if",
    "implements",
    "include",
    "include_once",
    "instanceof",
    "insteadof",
    "interface",
    "isset",
    "list",
    "match",
    "namespace",
    "new",
    "null",
    "or",
    "print",
    "private",
    "protected",
    "public",
    "readonly",
    "require",
    "require_once",
    "return",
    "static",
    "switch",
    "throw",
    "trait",
    "true",
    "try",
    "unset",
    "use",
    "var",
    "while",
    "xor",
    "yield",
];

fn is_keyword(word: &str) -> bool {
    KEYWORDS.binary_search(&word).is_ok()
}

/// Where the lexer is between one line and the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Outside `<?php ... ?>`: plain markup, uncoloured.
    Html,
    /// Inside `<?php ... ?>`, not mid-comment or mid-string.
    Php,
    /// Inside `/* ... */`.
    BlockComment,
    /// Inside `'...'`.
    SingleQuoted,
    /// Inside `"..."`.
    DoubleQuoted,
}

impl State {
    const fn decode(bits: u32) -> Self {
        match bits {
            1 => Self::Php,
            2 => Self::BlockComment,
            3 => Self::SingleQuoted,
            4 => Self::DoubleQuoted,
            _ => Self::Html,
        }
    }

    const fn encode(self) -> LineState {
        LineState(match self {
            Self::Html => 0,
            Self::Php => 1,
            Self::BlockComment => 2,
            Self::SingleQuoted => 3,
            Self::DoubleQuoted => 4,
        })
    }
}

/// PHP, markup and code both.
///
/// A unit struct: there is nothing to configure, and one `Arc` of it can be
/// shared by every `.php` template tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PhpHighlighter;

impl Highlighter for PhpHighlighter {
    fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState) {
        let mut lexer = Lexer {
            bytes: text.as_bytes(),
            text,
            at: 0,
            spans: Vec::new(),
        };
        let end = lexer.run(State::decode(state.0));
        (lexer.spans, end.encode())
    }

    fn line_comment(&self) -> Option<&'static str> {
        // The toggle writes PHP's line comment; it is meaningless in the HTML
        // stretches of the file, the same way SQL's comment toggle would be
        // meaningless run over a template's placeholder text.
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
            State::Html => State::Html,
            State::Php => State::Php,
            State::BlockComment => {
                if self.bracketed_run(0, b"*/", Token::Comment) {
                    State::Php
                } else {
                    return State::BlockComment;
                }
            }
            State::SingleQuoted => {
                if self.quoted_from(0, b'\'') {
                    State::Php
                } else {
                    return State::SingleQuoted;
                }
            }
            State::DoubleQuoted => {
                if self.quoted_from(0, b'"') {
                    State::Php
                } else {
                    return State::DoubleQuoted;
                }
            }
        };

        loop {
            match state {
                State::Html => {
                    if !self.html() {
                        break;
                    }
                    state = State::Php;
                }
                State::Php => match self.php() {
                    PhpExit::EndOfLine => break,
                    PhpExit::ClosingTag => state = State::Html,
                    PhpExit::OpenBlockComment => {
                        state = State::BlockComment;
                        break;
                    }
                    PhpExit::OpenSingleQuoted => {
                        state = State::SingleQuoted;
                        break;
                    }
                    PhpExit::OpenDoubleQuoted => {
                        state = State::DoubleQuoted;
                        break;
                    }
                },
                State::BlockComment | State::SingleQuoted | State::DoubleQuoted => {
                    unreachable!("the loop only ever assigns these right before breaking out of it")
                }
            }
        }
        state
    }

    /// Scans markup up to `<?php` or `<?=`, painting neither. `false` when
    /// the line ends without either opening.
    fn html(&mut self) -> bool {
        while self.at < self.bytes.len() {
            if self.starts_with(b"<?php") {
                self.push(self.at, self.at + 5, Token::Punctuation);
                self.at += 5;
                return true;
            }
            if self.starts_with(b"<?=") {
                self.push(self.at, self.at + 3, Token::Punctuation);
                self.at += 3;
                return true;
            }
            self.at = self.next_char_boundary(self.at);
        }
        false
    }

    /// Scans PHP code up to the next thing that changes state: `?>`, an
    /// unterminated comment or string, or the end of the line.
    fn php(&mut self) -> PhpExit {
        while self.at < self.bytes.len() {
            if self.starts_with(b"?>") {
                self.push(self.at, self.at + 2, Token::Punctuation);
                self.at += 2;
                return PhpExit::ClosingTag;
            }
            let byte = self.bytes[self.at];
            match byte {
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'/' if self.starts_with(b"//") => {
                    self.push(self.at, self.bytes.len(), Token::Comment);
                    self.at = self.bytes.len();
                }
                b'#' if !self.starts_with(b"#[") => {
                    self.push(self.at, self.bytes.len(), Token::Comment);
                    self.at = self.bytes.len();
                }
                b'/' if self.starts_with(b"/*") => {
                    let start = self.at;
                    self.at += 2;
                    if !self.bracketed_run(start, b"*/", Token::Comment) {
                        return PhpExit::OpenBlockComment;
                    }
                }
                b'\'' => {
                    let start = self.at;
                    self.at += 1;
                    if !self.quoted_from(start, b'\'') {
                        return PhpExit::OpenSingleQuoted;
                    }
                }
                b'"' => {
                    let start = self.at;
                    self.at += 1;
                    if !self.quoted_from(start, b'"') {
                        return PhpExit::OpenDoubleQuoted;
                    }
                }
                b'$' => self.variable(),
                b'0'..=b'9' => self.number(),
                b';' | b',' | b'.' | b'(' | b')' | b'[' | b']' | b'{' | b'}' | b'@' | b'#' => {
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
        PhpExit::EndOfLine
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

    /// `$name`, painted whole as one identifier.
    fn variable(&mut self) {
        let start = self.at;
        self.at += 1;
        let base = self.at;
        let mut end = base;
        for (offset, c) in self.text[base..].char_indices() {
            if c.is_alphanumeric() || c == '_' {
                end = base + offset + c.len_utf8();
            } else {
                break;
            }
        }
        self.at = end;
        self.push(start, self.at, Token::Identifier);
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
        let lowered = word.to_ascii_lowercase();
        let token = if is_keyword(&lowered) {
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

    /// Scans a quoted run, painting from `start`. Unlike every language in
    /// [`crate::lang::clike`], a PHP string is allowed to run over a line
    /// break, so `false` -- the line ended first -- leaves the caller to
    /// carry the state rather than closing it there.
    fn quoted_from(&mut self, start: usize, quote: u8) -> bool {
        let mut escaped = false;
        while self.at < self.bytes.len() {
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
        self.push(start, self.bytes.len(), Token::String);
        false
    }

    fn bracketed_run(&mut self, start: usize, close: &[u8], token: Token) -> bool {
        while self.at < self.bytes.len() {
            if self.starts_with(close) {
                self.at += close.len();
                self.push(start, self.at, token);
                return true;
            }
            self.at += 1;
        }
        self.push(start, self.bytes.len(), token);
        false
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

/// Why [`Lexer::php`] stopped scanning.
enum PhpExit {
    EndOfLine,
    ClosingTag,
    OpenBlockComment,
    OpenSingleQuoted,
    OpenDoubleQuoted,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::test_support::{lex, lex_lines};

    fn h() -> PhpHighlighter {
        PhpHighlighter
    }

    #[test]
    fn the_keyword_table_is_sorted() {
        for pair in KEYWORDS.windows(2) {
            assert!(pair[0] < pair[1], "{pair:?} is out of order");
        }
    }

    #[test]
    fn markup_outside_the_tag_is_unpainted() {
        let (spans, state) = lex(&h(), "<p>hello</p>", LineState::START);
        assert_eq!(spans, vec![]);
        assert!(state.is_start());
    }

    #[test]
    fn the_open_and_close_tags_are_punctuation_and_bound_the_code() {
        let (spans, state) = lex(&h(), "<?php echo 1; ?>after", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("<?php", Token::Punctuation),
                ("echo", Token::Keyword),
                ("1", Token::Number),
                (";", Token::Punctuation),
                ("?>", Token::Punctuation),
            ]
        );
        assert!(state.is_start());
    }

    #[test]
    fn a_variable_is_one_identifier_and_a_call_is_a_function() {
        let (spans, _) = lex(&h(), "<?php $x = strlen($x);", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("<?php", Token::Punctuation),
                ("$x", Token::Identifier),
                ("=", Token::Operator),
                ("strlen", Token::Function),
                ("(", Token::Punctuation),
                ("$x", Token::Identifier),
                (")", Token::Punctuation),
                (";", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_class_name_is_a_type() {
        let (spans, _) = lex(&h(), "<?php new Foo();", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("<?php", Token::Punctuation),
                ("new", Token::Keyword),
                ("Foo", Token::Type),
                ("(", Token::Punctuation),
                (")", Token::Punctuation),
                (";", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_line_comment_stays_open_for_the_hash_form_but_not_a_php8_attribute() {
        let (spans, _) = lex(&h(), "<?php $x; // one", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("<?php", Token::Punctuation),
                ("$x", Token::Identifier),
                (";", Token::Punctuation),
                ("// one", Token::Comment),
            ]
        );

        let (spans, _) = lex(&h(), "<?php $x; # one", LineState::START);
        assert_eq!(
            spans,
            vec![
                ("<?php", Token::Punctuation),
                ("$x", Token::Identifier),
                (";", Token::Punctuation),
                ("# one", Token::Comment),
            ]
        );
    }

    #[test]
    fn a_block_comment_spans_lines_and_closes() {
        let lines = lex_lines(&h(), "<?php /* open\nstill\nclosed */ $x;");
        assert_eq!(
            lines[0],
            vec![("<?php", Token::Punctuation), ("/* open", Token::Comment)]
        );
        assert_eq!(lines[1], vec![("still", Token::Comment)]);
        assert_eq!(
            lines[2],
            vec![
                ("closed */", Token::Comment),
                ("$x", Token::Identifier),
                (";", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn a_string_spans_a_line_break_unlike_the_clike_languages() {
        let lines = lex_lines(&h(), "<?php $s = 'one\ntwo';");
        assert_eq!(
            lines[0],
            vec![
                ("<?php", Token::Punctuation),
                ("$s", Token::Identifier),
                ("=", Token::Operator),
                ("'one", Token::String),
            ]
        );
        assert_eq!(
            lines[1],
            vec![("two'", Token::String), (";", Token::Punctuation)]
        );
    }

    #[test]
    fn a_returned_html_boundary_resumes_markup() {
        let lines = lex_lines(&h(), "<?php if ($x): ?>\n<p>text</p>\n<?php endif; ?>");
        assert_eq!(
            lines[0],
            vec![
                ("<?php", Token::Punctuation),
                ("if", Token::Keyword),
                ("(", Token::Punctuation),
                ("$x", Token::Identifier),
                (")", Token::Punctuation),
                (":", Token::Operator),
                ("?>", Token::Punctuation),
            ]
        );
        assert_eq!(lines[1], vec![]);
        assert_eq!(
            lines[2],
            vec![
                ("<?php", Token::Punctuation),
                ("endif", Token::Keyword),
                (";", Token::Punctuation),
                ("?>", Token::Punctuation),
            ]
        );
    }
}
