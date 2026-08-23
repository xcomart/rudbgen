//! An XML highlighter, reused for HTML.
//!
//! XML (and HTML closely enough to share a lexer with it) is not the
//! "keyword, string, comment" shape [`crate::lang::clike`] covers: what there
//! is to colour is tag names, attribute names, attribute values, and three
//! kinds of bracketed run that can carry over a line break — `<!-- -->`,
//! `<![CDATA[ ]]>`, and `<?...?>` — so this is a small state machine of its
//! own, closer in shape to [`crate::template_syntax`] than to
//! [`crate::sql_syntax`].
//!
//! | written | painted as |
//! |---|---|
//! | a tag name: `<foo>`, `</foo>` | keyword |
//! | an attribute name | type |
//! | an attribute value, quotes included | string |
//! | `<!-- ... -->`, over as many lines as it needs | comment |
//! | `<![CDATA[ ... ]]>`, likewise | string -- it is literal character data, not a comment |
//! | `<?...?>` (a PI; `<?xml ... ?>` is the common one) | comment |
//! | an entity reference: `&amp;`, `&#39;` | operator |
//! | `<`, `</`, `/>`, `>`, `=` | punctuation / operator |
//! | text between tags | nothing -- the palette's foreground |
//!
//! A `<!DOCTYPE ...>` is read the same way an ordinary tag is -- the `!` is
//! punctuation, `DOCTYPE` is the tag "name" and so a keyword, and whatever
//! follows is read as attributes -- which is a simplification (a doctype has
//! no attributes, only tokens) but a harmless one: nothing here parses a
//! doctype's grammar, and the alternative is a fourth bracketed-run kind for a
//! construct that shows up once per document.

use crate::highlight::{Highlighter, LineState, Span, Token};

/// Where the lexer is between one line and the next.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Ordinary text, outside any tag or bracketed run.
    Content,
    /// Inside `< ... >`, past the opening delimiter.
    Tag,
    /// Inside `<!-- ... -->`.
    Comment,
    /// Inside `<![CDATA[ ... ]]>`.
    Cdata,
    /// Inside `<? ... ?>`.
    Pi,
}

impl Mode {
    const fn decode(bits: u32) -> Self {
        match bits {
            1 => Self::Tag,
            2 => Self::Comment,
            3 => Self::Cdata,
            4 => Self::Pi,
            _ => Self::Content,
        }
    }

    const fn code(self) -> u32 {
        match self {
            Self::Content => 0,
            Self::Tag => 1,
            Self::Comment => 2,
            Self::Cdata => 3,
            Self::Pi => 4,
        }
    }
}

/// Everything the lexer remembers between two lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct XState {
    mode: Mode,
    /// The quote an attribute value was opened with, when one is open.
    quote: Option<u8>,
    /// Whether [`Mode::Tag`] has not read its name yet. Set the moment a tag
    /// opens; false from the moment the name is read, including across a
    /// line break inside the tag's attribute list.
    fresh: bool,
}

impl Default for XState {
    fn default() -> Self {
        Self {
            mode: Mode::Content,
            quote: None,
            fresh: false,
        }
    }
}

impl XState {
    fn decode(state: LineState) -> Self {
        let bits = state.0;
        Self {
            mode: Mode::decode(bits & 0b111),
            quote: match (bits >> 3) & 0b11 {
                1 => Some(b'\''),
                2 => Some(b'"'),
                _ => None,
            },
            fresh: bits & (1 << 5) != 0,
        }
    }

    fn encode(self) -> LineState {
        let mut bits = self.mode.code();
        bits |= match self.quote {
            Some(b'\'') => 1 << 3,
            Some(_) => 2 << 3,
            None => 0,
        };
        bits |= u32::from(self.fresh) << 5;
        LineState(bits)
    }
}

/// XML, and HTML by way of the same grammar.
///
/// A unit struct: there is nothing to configure, and one `Arc` of it can be
/// shared by every `.xml`/`.html` template tab.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct XmlHighlighter;

impl Highlighter for XmlHighlighter {
    fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState) {
        let mut lexer = Lexer {
            bytes: text.as_bytes(),
            text,
            at: 0,
            spans: Vec::new(),
        };
        let end = lexer.run(XState::decode(state));
        (lexer.spans, end.encode())
    }

    // No `line_comment`: XML's only comment syntax is `<!-- ... -->`, which
    // wraps a whole run rather than opening at the start of a line, so there
    // is no prefix for the editor's line-comment toggle to write.
}

struct Lexer<'a> {
    bytes: &'a [u8],
    text: &'a str,
    at: usize,
    spans: Vec<Span>,
}

impl Lexer<'_> {
    fn run(&mut self, mut state: XState) -> XState {
        while self.at < self.bytes.len() {
            match state.mode {
                Mode::Content => self.content(&mut state),
                Mode::Tag => self.tag(&mut state),
                Mode::Comment => {
                    if self.comment_body_from(0) {
                        state.mode = Mode::Content;
                    }
                }
                Mode::Cdata => {
                    if self.cdata_body_from(0) {
                        state.mode = Mode::Content;
                    }
                }
                Mode::Pi => {
                    if self.pi_body_from(0) {
                        state.mode = Mode::Content;
                    }
                }
            }
        }
        state
    }

    // -------------------------------------------------------------- content

    /// Scans ordinary text up to the next `<` (dispatching whatever it
    /// opens) or an entity reference, painting neither the plain text nor
    /// anything it does not recognize.
    fn content(&mut self, state: &mut XState) {
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            if byte == b'<' {
                if self.starts_with(b"<!--") {
                    let start = self.at;
                    self.at += 4;
                    if !self.comment_body_from(start) {
                        state.mode = Mode::Comment;
                    }
                    return;
                }
                if self.starts_with(b"<![CDATA[") {
                    let start = self.at;
                    self.at += 9;
                    if !self.cdata_body_from(start) {
                        state.mode = Mode::Cdata;
                    }
                    return;
                }
                if self.starts_with(b"<?") {
                    let start = self.at;
                    self.at += 2;
                    if !self.pi_body_from(start) {
                        state.mode = Mode::Pi;
                    }
                    return;
                }
                let open_len = if self.starts_with(b"</") { 2 } else { 1 };
                self.push(self.at, self.at + open_len, Token::Punctuation);
                self.at += open_len;
                state.mode = Mode::Tag;
                state.fresh = true;
                return;
            }
            if byte == b'&'
                && let Some(end) = self.entity_end()
            {
                self.push(self.at, end, Token::Operator);
                self.at = end;
                continue;
            }
            self.at = self.next_char_boundary(self.at);
        }
    }

    /// The end of an entity reference (`&name;` or `&#123;`/`&#x1F;`)
    /// starting at the cursor's `&`, if one is there.
    fn entity_end(&self) -> Option<usize> {
        let mut at = self.at + 1;
        if self.bytes.get(at) == Some(&b'#') {
            at += 1;
            if matches!(self.bytes.get(at), Some(b'x' | b'X')) {
                at += 1;
            }
            let digits_start = at;
            while self.bytes.get(at).is_some_and(u8::is_ascii_alphanumeric) {
                at += 1;
            }
            if at == digits_start {
                return None;
            }
        } else {
            let name_start = at;
            while self.bytes.get(at).is_some_and(u8::is_ascii_alphanumeric) {
                at += 1;
            }
            if at == name_start {
                return None;
            }
        }
        (self.bytes.get(at) == Some(&b';')).then_some(at + 1)
    }

    // ------------------------------------------------------------------ tag

    /// Scans as much of a tag as this line holds: its name, if not yet read,
    /// then its attributes, until `>` or `/>` closes it or the line runs out.
    fn tag(&mut self, state: &mut XState) {
        if let Some(quote) = state.quote {
            if self.attr_value_from(0, quote) {
                state.quote = None;
            } else {
                return;
            }
        }

        if state.fresh {
            self.skip_blanks();
            if self.at >= self.bytes.len() {
                return;
            }
            if self.bytes[self.at] == b'!' {
                self.push(self.at, self.at + 1, Token::Punctuation);
                self.at += 1;
            }
            if let Some((start, end)) = self.read_name() {
                self.push(start, end, Token::Keyword);
            }
            state.fresh = false;
        }

        while self.at < self.bytes.len() {
            match self.bytes[self.at] {
                b' ' | b'\t' | b'\r' => self.at += 1,
                b'/' if self.starts_with(b"/>") => {
                    self.push(self.at, self.at + 2, Token::Punctuation);
                    self.at += 2;
                    state.mode = Mode::Content;
                    return;
                }
                b'>' => {
                    self.push(self.at, self.at + 1, Token::Punctuation);
                    self.at += 1;
                    state.mode = Mode::Content;
                    return;
                }
                b'=' => {
                    self.push(self.at, self.at + 1, Token::Operator);
                    self.at += 1;
                }
                b'\'' | b'"' => {
                    let quote = self.bytes[self.at];
                    let start = self.at;
                    self.at += 1;
                    if !self.attr_value_from(start, quote) {
                        state.quote = Some(quote);
                        return;
                    }
                }
                _ => {
                    if let Some((start, end)) = self.read_name() {
                        self.push(start, end, Token::Type);
                    } else {
                        self.at = self.next_char_boundary(self.at);
                    }
                }
            }
        }
    }

    /// An XML name: a letter, `_` or `:` to start, then letters, digits,
    /// `_`, `-`, `.` or `:`. `None` when there is no name at the cursor.
    fn read_name(&mut self) -> Option<(usize, usize)> {
        let start = self.at;
        let mut chars = self.text[start..].char_indices();
        let (_, first) = chars.next()?;
        if !(first.is_alphabetic() || first == '_' || first == ':') {
            return None;
        }
        let mut end = start + first.len_utf8();
        for (offset, c) in chars {
            if c.is_alphanumeric() || matches!(c, '_' | '-' | '.' | ':') {
                end = start + offset + c.len_utf8();
            } else {
                break;
            }
        }
        self.at = end;
        Some((start, end))
    }

    /// Scans an attribute value to its closing `quote`, painting the whole
    /// run -- quotes included -- from `start`. `false` when the line ends
    /// first, which is how an attribute value is allowed to carry over a
    /// line break.
    fn attr_value_from(&mut self, start: usize, quote: u8) -> bool {
        while self.at < self.bytes.len() {
            if self.bytes[self.at] == quote {
                self.at += 1;
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

    // ------------------------------------------------------- bracketed runs

    /// Scans to `-->`, painting from `start`. `false` when the line ends
    /// first.
    fn comment_body_from(&mut self, start: usize) -> bool {
        self.bracketed_run(start, b"-->", Token::Comment)
    }

    /// Scans to `]]>`, painting from `start`. `false` when the line ends
    /// first.
    fn cdata_body_from(&mut self, start: usize) -> bool {
        self.bracketed_run(start, b"]]>", Token::String)
    }

    /// Scans to `?>`, painting from `start`. `false` when the line ends
    /// first.
    fn pi_body_from(&mut self, start: usize) -> bool {
        self.bracketed_run(start, b"?>", Token::Comment)
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

    // -------------------------------------------------------------- helpers

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

    fn h() -> XmlHighlighter {
        XmlHighlighter
    }

    #[test]
    fn a_tag_names_its_attributes_and_their_values() {
        let (spans, state) = lex(&h(), r#"<foo bar="1" baz='two'>"#, LineState::START);
        assert_eq!(
            spans,
            vec![
                ("<", Token::Punctuation),
                ("foo", Token::Keyword),
                ("bar", Token::Type),
                ("=", Token::Operator),
                ("\"1\"", Token::String),
                ("baz", Token::Type),
                ("=", Token::Operator),
                ("'two'", Token::String),
                (">", Token::Punctuation),
            ]
        );
        assert!(state.is_start());
    }

    #[test]
    fn a_closing_and_a_self_closing_tag() {
        assert_eq!(
            lex(&h(), "</foo>", LineState::START).0,
            vec![
                ("</", Token::Punctuation),
                ("foo", Token::Keyword),
                (">", Token::Punctuation),
            ]
        );
        assert_eq!(
            lex(&h(), "<br/>", LineState::START).0,
            vec![
                ("<", Token::Punctuation),
                ("br", Token::Keyword),
                ("/>", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn text_between_tags_is_unpainted() {
        assert_eq!(lex(&h(), "hello, world", LineState::START).0, vec![]);
    }

    #[test]
    fn an_entity_reference_is_an_operator() {
        assert_eq!(
            lex(&h(), "a &amp; b &#39; c &#x41; d", LineState::START).0,
            vec![
                ("&amp;", Token::Operator),
                ("&#39;", Token::Operator),
                ("&#x41;", Token::Operator),
            ]
        );
    }

    #[test]
    fn a_comment_spans_lines_and_closes() {
        let lines = lex_lines(&h(), "<!-- one\n  two\n  three -->after");
        assert_eq!(lines[0], vec![("<!-- one", Token::Comment)]);
        assert_eq!(lines[1], vec![("  two", Token::Comment)]);
        assert_eq!(lines[2], vec![("  three -->", Token::Comment)]);
    }

    #[test]
    fn cdata_spans_lines_and_is_painted_as_a_string() {
        let lines = lex_lines(&h(), "<![CDATA[ a < b\n c ]]>");
        assert_eq!(lines[0], vec![("<![CDATA[ a < b", Token::String)]);
        assert_eq!(lines[1], vec![(" c ]]>", Token::String)]);
    }

    #[test]
    fn a_processing_instruction_is_a_comment() {
        assert_eq!(
            lex(&h(), r#"<?xml version="1.0"?>"#, LineState::START).0,
            vec![(r#"<?xml version="1.0"?>"#, Token::Comment)]
        );
    }

    #[test]
    fn an_attribute_value_spans_a_line_break() {
        let lines = lex_lines(&h(), "<foo bar=\"one\ntwo\">");
        assert_eq!(
            lines[0],
            vec![
                ("<", Token::Punctuation),
                ("foo", Token::Keyword),
                ("bar", Token::Type),
                ("=", Token::Operator),
                ("\"one", Token::String),
            ]
        );
        assert_eq!(
            lines[1],
            vec![("two\"", Token::String), (">", Token::Punctuation)]
        );
    }

    #[test]
    fn a_tag_may_open_and_its_name_be_read_on_a_later_line() {
        let lines = lex_lines(&h(), "<\n  foo>");
        assert_eq!(lines[0], vec![("<", Token::Punctuation)]);
        assert_eq!(
            lines[1],
            vec![("foo", Token::Keyword), (">", Token::Punctuation)]
        );
    }

    #[test]
    fn a_doctype_reads_its_name_as_a_keyword() {
        assert_eq!(
            lex(&h(), "<!DOCTYPE html>", LineState::START).0,
            vec![
                ("<", Token::Punctuation),
                ("!", Token::Punctuation),
                ("DOCTYPE", Token::Keyword),
                ("html", Token::Type),
                (">", Token::Punctuation),
            ]
        );
    }

    #[test]
    fn every_state_round_trips_through_its_encoding() {
        for bits in 0u32..64 {
            let state = XState::decode(LineState(bits));
            assert_eq!(XState::decode(state.encode()), state, "for {bits:#b}");
        }
        assert_eq!(XState::decode(LineState::START), XState::default());
    }
}
