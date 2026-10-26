//! Turning template text into the tree the renderer walks.
//!
//! The parser follows jdbgen's statement for statement, because a template is
//! only compatible if it is read the same way - including the rules that look
//! like accidents: a `${` is closed by the **first** `}` behind it, even one
//! inside a quoted attribute value; and a trailing `${` at the very end of a
//! template is dropped without a word.
//!
//! One accident is deliberately not reproduced: jdbgen recognised `endif`,
//! `endfor`, `else` and `elif:` in lower case only, so `${ELSE}` silently
//! became a member lookup. Here they are case insensitive like every other
//! statement name - see [`is_keyword`] and [`elif_options`].

use crate::cond::{COND_NAMES, Cond, CondKind};
use crate::error::{ParseError, Span};
use crate::keys::KeyChain;
use crate::strutil;

/// How deep `if`s and `for`s may be nested inside one another.
///
/// The parser, the renderer and the drop of the parsed tree all recurse once
/// per level, so without a limit a template of nothing but tens of thousands
/// of unclosed `${if:...}` overflows the stack and takes the application down,
/// even from the editor's live preview while it is typed. The limit is set by the
/// unoptimised build, whose frames are the largest: an `if` costs the parser
/// and the renderer several kilobytes of stack per level there, and sixty-four
/// levels of it still fit a thread's default two megabytes with room to
/// spare. No real template comes near it; the shipped ones nest two deep.
pub(crate) const MAX_DEPTH: usize = 64;

/// The largest `padSize` or `indent`, in display columns.
pub(crate) const MAX_WIDTH: i64 = 4096;

/// The attribute list of a statement, in the order it was written.
///
/// jdbgen keeps these in a `HashMap`, where a repeated name overwrites the
/// earlier value and the order is lost. Keeping the order costs nothing and
/// makes the conditions of an `if` fire in the order the template writer sees
/// them, which is the only thing the order is observable through.
#[derive(Clone, Debug, Default)]
pub(crate) struct Attrs {
    pairs: Vec<(String, String)>,
}

impl Attrs {
    fn new() -> Attrs {
        Attrs::default()
    }

    /// Add a pair, replacing an earlier one of the same name - the last value
    /// wins, as it does in a map.
    fn put(&mut self, name: String, value: String) {
        match self.pairs.iter_mut().find(|(n, _)| *n == name) {
            Some(pair) => pair.1 = value,
            None => self.pairs.push((name, value)),
        }
    }

    /// The value of a lower case attribute name.
    pub(crate) fn get(&self, name: &str) -> Option<&str> {
        self.pairs
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.as_str())
    }

    /// Every attribute, in template order.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&str, &str)> {
        self.pairs.iter().map(|(n, v)| (n.as_str(), v.as_str()))
    }

    /// The attribute names, for the message of a statement that misses one.
    pub(crate) fn names(&self) -> Vec<&str> {
        self.pairs.iter().map(|(n, _)| n.as_str()).collect()
    }

    /// The key chain of a statement, written either as `key` or as `item`.
    fn key(&self) -> Option<&str> {
        self.get("key").or_else(|| self.get("item"))
    }

    /// The width an attribute such as `padSize` holds, `0` when it is not
    /// there. `label` is the attribute as the template writer spells it.
    ///
    /// A width is a count of blanks the renderer writes, so it is refused
    /// above [`MAX_WIDTH`]: `padSize=9999999999` would otherwise ask for ten
    /// gigabytes per value, and an allocation that cannot be served aborts the
    /// process. A negative width writes no blanks and is left alone.
    pub(crate) fn number(&self, name: &str, label: &str) -> Result<i64, String> {
        let Some(text) = self.get(name) else {
            return Ok(0);
        };
        let width = text
            .parse::<i64>()
            .map_err(|_| format!("{label} is not a number: \"{text}\""))?;
        if width > MAX_WIDTH {
            return Err(format!(
                "{label} is too large: {width} (at most {MAX_WIDTH})"
            ));
        }
        Ok(width)
    }
}

/// A statement of the parsed template, or the literal text between two of
/// them.
#[derive(Debug)]
pub(crate) enum Node {
    /// Text that is copied as it is; `literal` marks the text a `${'...'}`
    /// placeholder stood for.
    Text {
        text: String,
        span: Span,
        literal: bool,
    },
    /// `${item:...}`, a value of the current model.
    Item(ItemNode),
    /// `${super:...}`, a value of the model enclosing the loop.
    Super(ItemNode),
    /// `${if:...}` with its branches.
    If(IfNode),
    /// `${for:...}` with its body.
    For(ForNode),
    /// `${date:...}`.
    Date(PlainNode),
    /// `${user:...}`.
    User(PlainNode),
    /// `${author:...}`.
    Author(PlainNode),
}

/// `${item}` and `${super}`.
#[derive(Debug)]
pub(crate) struct ItemNode {
    pub(crate) span: Span,
    pub(crate) attrs: Attrs,
    pub(crate) chain: KeyChain,
}

/// A statement whose whole content is its attribute list.
#[derive(Debug)]
pub(crate) struct PlainNode {
    pub(crate) span: Span,
    pub(crate) attrs: Attrs,
}

/// `${if:...}` up to its `${endif}`.
///
/// The `elif`s are a list beside the `if` rather than an `if` nested in the
/// false branch of the one before, which is how jdbgen holds them: a chain of
/// a hundred thousand `elif`s is then a long list and not a deep tree, which
/// neither the renderer nor the drop of the tree has to recurse through.
#[derive(Debug)]
pub(crate) struct IfNode {
    /// The `if` first, then every `${elif:...}` in the order written; the
    /// first one whose conditions all hold is rendered.
    pub(crate) branches: Vec<Branch>,
    /// The body of the `${else}`, rendered when no branch holds.
    pub(crate) otherwise: Option<Vec<Node>>,
}

/// One `${if:...}` or `${elif:...}` with the body behind it.
#[derive(Debug)]
pub(crate) struct Branch {
    pub(crate) span: Span,
    pub(crate) chain: KeyChain,
    pub(crate) conds: Vec<Cond>,
    pub(crate) body: Vec<Node>,
}

/// `${for:...}` up to its `${endfor}`.
#[derive(Debug)]
pub(crate) struct ForNode {
    pub(crate) span: Span,
    pub(crate) attrs: Attrs,
    /// The member holding the collection. A `for` never runs the key through
    /// the processors, so this is the plain name rather than a chain.
    pub(crate) key: String,
    pub(crate) body: Vec<Node>,
}

/// One statement as [`Parser::next`] hands it back.
struct Statement {
    body: String,
    span: Span,
}

/// A cursor over the template text.
///
/// Positions are character indices, the way Java counts them, with a table on
/// the side that turns one into the byte offset a [`Span`] reports.
pub(crate) struct Parser {
    chars: Vec<char>,
    byte_of: Vec<usize>,
    curr: usize,
    len: usize,
    line: usize,
    /// How many `if`s and `for`s the statement being read is inside of.
    depth: usize,
}

impl Parser {
    pub(crate) fn new(template: &str) -> Parser {
        let chars: Vec<char> = template.chars().collect();
        let mut byte_of = Vec::with_capacity(chars.len() + 1);
        let mut at = 0;
        for c in &chars {
            byte_of.push(at);
            at += c.len_utf8();
        }
        byte_of.push(at);
        let len = chars.len();
        Parser {
            chars,
            byte_of,
            curr: 0,
            len,
            line: 0,
            depth: 0,
        }
    }

    /// Parse the whole template.
    pub(crate) fn parse(&mut self) -> Result<Vec<Node>, ParseError> {
        let mut res = Vec::new();
        while let Some(stmt) = self.next(&mut res)? {
            let node = self.parse_one(stmt)?;
            res.push(node);
        }
        Ok(res)
    }

    // -------------------------------------------------------------- cursor

    fn byte(&self, index: usize) -> usize {
        self.byte_of[index.min(self.len)]
    }

    fn substring(&self, from: usize, to: usize) -> String {
        self.chars[from.min(self.len)..to.min(self.len)]
            .iter()
            .collect()
    }

    /// Jump to `end`, counting the line breaks skipped over on the way.
    fn update_line_count(&mut self, end: usize) {
        if end > self.curr {
            self.line += self.chars[self.curr..end]
                .iter()
                .filter(|c| **c == '\n')
                .count();
        }
        self.curr = end;
    }

    fn next_char(&mut self) -> Option<char> {
        if self.curr < self.len {
            let c = self.chars[self.curr];
            self.curr += 1;
            if c == '\n' {
                self.line += 1;
            }
            Some(c)
        } else {
            None
        }
    }

    fn skip_space(&mut self) {
        while let Some(c) = self.next_char() {
            if !strutil::is_space(c) {
                self.curr -= 1;
                break;
            }
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.curr).copied()
    }

    fn find_char(&self, needle: char, from: usize) -> Option<usize> {
        (from..self.len).find(|i| self.chars[*i] == needle)
    }

    fn find_open(&self, from: usize) -> Option<usize> {
        (from..self.len.saturating_sub(1))
            .find(|i| self.chars[*i] == '$' && self.chars[*i + 1] == '{')
    }

    /// The text right behind the cursor, so that the user can find the place a
    /// parse error was reported at.
    fn near(&self) -> String {
        const LENGTH: usize = 100;
        if self.curr + LENGTH < self.len {
            format!("{}...", self.substring(self.curr, self.curr + LENGTH))
        } else {
            self.substring(self.curr, self.len)
        }
    }

    fn error(&self, message: impl Into<String>) -> ParseError {
        ParseError::new(message, self.line)
    }

    /// An error about one statement, marked on the statement and reported on
    /// the line it starts on - the place a render error about it would have
    /// named.
    fn invalid(&self, message: impl Into<String>, span: Span) -> ParseError {
        ParseError {
            message: message.into(),
            line: span.line,
            span: Some(span),
        }
    }

    /// Step into the body of an `if` or a `for`, refusing to go deeper than
    /// [`MAX_DEPTH`]; the statement that opened it steps out again when its
    /// end has been read.
    fn enter(&mut self, span: Span) -> Result<(), ParseError> {
        self.depth += 1;
        if self.depth > MAX_DEPTH {
            return Err(self.invalid(
                format!(
                    "statements nested more than {MAX_DEPTH} deep, before: {}",
                    self.near()
                ),
                span,
            ));
        }
        Ok(())
    }

    /// The key chain of a statement, which every statement that reads one
    /// must have, with every processor in it checked.
    fn require_chain(&self, attrs: &Attrs, span: Span) -> Result<KeyChain, ParseError> {
        let Some(key) = attrs.key() else {
            return Err(self.invalid(missing_key(attrs), span));
        };
        let chain = KeyChain::parse(key);
        chain
            .check()
            .map_err(|message| self.invalid(message, span))?;
        Ok(chain)
    }

    /// Make sure the decorations of a value statement can be applied, so that
    /// a `padSize` that is no number fails the parse rather than the render.
    fn check_decorations(&self, attrs: &Attrs, span: Span) -> Result<(), ParseError> {
        attrs
            .number("padsize", "padSize")
            .map_err(|message| self.invalid(message, span))?;
        Ok(())
    }

    /// The attributes of a `${date}`, `${user}` or `${author}`, checked.
    fn plain(&self, options: &str, span: Span) -> Result<PlainNode, ParseError> {
        let attrs = self.parse_nv_pairs(options)?;
        self.check_decorations(&attrs, span)?;
        Ok(PlainNode { span, attrs })
    }

    // --------------------------------------------------------------- parse

    /// Advance to the next statement, appending the literal text passed on the
    /// way to `items`.
    ///
    /// A quoted placeholder - `${'text'}` - is literal text itself: its
    /// unescaped content is appended and the search goes on. An empty literal
    /// stands for no text at all.
    fn next(&mut self, items: &mut Vec<Node>) -> Result<Option<Statement>, ParseError> {
        loop {
            if self.curr == self.len {
                return Ok(None);
            }
            let open = self.find_open(self.curr).unwrap_or(self.len);
            if open > self.curr {
                let span = Span::new(self.byte(self.curr), self.byte(open), self.line);
                items.push(Node::Text {
                    text: self.substring(self.curr, open),
                    span,
                    literal: false,
                });
            }
            let mut sp = open;
            if sp + 1 < self.len {
                sp += 2; // skip "${"
            }
            self.update_line_count(sp);
            if sp == self.len {
                // a template ending in a bare '${' has nothing left to read
                return Ok(None);
            }
            let start_line = self.line;
            self.skip_space();

            let mut literal = String::new();
            let mut is_literal = false;
            if matches!(self.peek(), Some('"') | Some('\'')) {
                is_literal = true;
                let open_char = self.next_char().expect("peeked");
                let mut escaped = false;
                while let Some(c) = self.next_char() {
                    if !escaped {
                        if c == '\\' {
                            // the escape character itself is not part of the text
                            escaped = true;
                            continue;
                        } else if c == open_char {
                            break;
                        }
                    } else {
                        escaped = false;
                    }
                    literal.push(c);
                }
                sp = self.curr;
            }

            let Some(last) = self.find_char('}', sp) else {
                return Err(self.error(format!("'}}' not found, before: {}", self.near())));
            };
            let body = if is_literal {
                literal
            } else {
                strutil::trim(&self.substring(sp, last)).to_string()
            };
            self.update_line_count(last);
            self.next_char(); // skip '}'
            let span = Span::new(self.byte(open), self.byte(self.curr), start_line);

            if is_literal {
                // an empty literal stands for no text at all
                if !body.is_empty() {
                    items.push(Node::Text {
                        text: body,
                        span,
                        literal: true,
                    });
                }
                continue;
            }
            return Ok(Some(Statement { body, span }));
        }
    }

    /// Read one statement body and hand it to the parser of its type.
    fn parse_one(&mut self, stmt: Statement) -> Result<Node, ParseError> {
        let mut body = stmt.body;
        let lowered = body.to_lowercase();
        if matches!(lowered.as_str(), "user" | "date" | "author") {
            body.push(':');
        } else if !body.contains(':') {
            body = format!("item:key={body}");
        }
        let idx = body.find(':').expect("a ':' was added when there was none");
        let kind = strutil::trim(&body[..idx]).to_lowercase();
        let options = body[idx + 1..].to_string();
        let span = stmt.span;

        match kind.as_str() {
            "item" => Ok(Node::Item(self.parse_item(&options, span)?)),
            "super" => Ok(Node::Super(self.parse_item(&options, span)?)),
            "if" => Ok(Node::If(self.parse_if(&options, span)?)),
            "for" => Ok(Node::For(self.parse_for(&options, span)?)),
            "date" => {
                // an attribute list without any '=' is the format itself, so
                // that ${date:yyyy-MM} works as well as ${date:format=yyyy-MM}
                let options = if options.contains('=') {
                    options
                } else {
                    format!("format={options}")
                };
                Ok(Node::Date(self.plain(&options, span)?))
            }
            "user" => Ok(Node::User(self.plain(&options, span)?)),
            "author" => Ok(Node::Author(self.plain(&options, span)?)),
            _ => Err(ParseError {
                message: format!("Unknown template: {body}, before: {}", self.near()),
                line: self.line,
                span: Some(span),
            }),
        }
    }

    fn parse_item(&mut self, options: &str, span: Span) -> Result<ItemNode, ParseError> {
        let attrs = self.parse_nv_pairs(options)?;
        let chain = self.require_chain(&attrs, span)?;
        self.check_decorations(&attrs, span)?;
        Ok(ItemNode { span, attrs, chain })
    }

    /// Split an attribute list into name/value pairs.
    ///
    /// A value may be wrapped in `'`, `"` or `(...)`, in which case a comma
    /// inside does not separate; the wrapping characters stay part of the
    /// value and the quotes - never the parentheses - are stripped off again
    /// by [`strutil::trim`]. `\n`, `\r` and `\t` are translated and any other
    /// escaped character stands for itself.
    fn parse_nv_pairs(&self, data: &str) -> Result<Attrs, ParseError> {
        let chars: Vec<char> = data.chars().collect();
        let mut attrs = Attrs::new();
        let mut buf = String::new();
        let mut name = String::new();
        let mut open: Option<char> = None;
        let mut idx = 0;

        let mismatch = |data: &str, near: String| {
            ParseError::new(
                format!("Name value pair not matched: {data}. invalid syntax before: {near}"),
                self.line,
            )
        };

        while idx < chars.len() {
            let mut c = chars[idx];
            if c == '\\' {
                idx += 1;
                if idx >= chars.len() {
                    return Err(self.error(format!(
                        "Dangling escape character at end of: {data}. invalid syntax before: {}",
                        self.near()
                    )));
                }
                c = chars[idx];
                match c {
                    'n' => buf.push('\n'),
                    'r' => buf.push('\r'),
                    't' => buf.push('\t'),
                    other => buf.push(other),
                }
            } else if open.is_none() && (c == '"' || c == '\'' || c == '(') {
                open = Some(if c == '(' { ')' } else { c });
                buf.push(c);
            } else if Some(c) == open {
                buf.push(c);
                open = None;
            } else if open.is_none() && c == '=' {
                if !strutil::is_blank(&name) {
                    return Err(mismatch(data, self.near()));
                }
                name = strutil::trim(&buf).to_string();
                buf.clear();
            } else if open.is_none() && c == ',' {
                let value = strutil::trim(&buf).to_string();
                if strutil::is_blank(&name) {
                    return Err(mismatch(data, self.near()));
                }
                attrs.put(name.to_lowercase(), value);
                name.clear();
                buf.clear();
            } else {
                buf.push(c);
            }
            idx += 1;
        }
        let value = strutil::trim(&buf).to_string();
        if !value.is_empty() {
            if strutil::is_blank(&name) {
                return Err(mismatch(data, self.near()));
            }
            attrs.put(name.to_lowercase(), value);
        }
        Ok(attrs)
    }

    /// Make sure an `if` carries nothing but a key and known conditions, so
    /// that a misspelled condition is reported instead of silently holding.
    fn check_if_conditions(&self, attrs: &Attrs, extra: &str) -> Result<(), ParseError> {
        for (name, _) in attrs.iter() {
            if name != "key" && name != "item" && !COND_NAMES.contains(&name) {
                return Err(self.error(format!(
                    "Unknown if condition: {extra}, before: {}",
                    self.near()
                )));
            }
        }
        Ok(())
    }

    /// One branch of an if/elif chain while it is being read.
    fn branch(&self, attrs: Attrs, span: Span) -> Result<Branch, ParseError> {
        let chain = self.require_chain(&attrs, span)?;
        let conds = attrs
            .iter()
            .filter_map(|(name, value)| {
                CondKind::from_name(name).map(|kind| Cond::new(kind, value.to_string()))
            })
            .collect::<Result<_, _>>()
            .map_err(|message| self.invalid(message, span))?;
        Ok(Branch {
            span,
            chain,
            conds,
            body: Vec::new(),
        })
    }

    /// Read an `if` up to its `${endif}`.
    ///
    /// An `elif` opens the next branch and, as in jdbgen, it replaces an
    /// `${else}` that was already read, so that an `else` before an `elif`
    /// loses its body.
    fn parse_if(&mut self, extra: &str, span: Span) -> Result<IfNode, ParseError> {
        self.enter(span)?;
        let attrs = self.parse_nv_pairs(extra)?;
        self.check_if_conditions(&attrs, extra)?;
        let mut branches = vec![self.branch(attrs, span)?];
        let mut tail: Option<Vec<Node>> = None;

        loop {
            let stmt = {
                let target = Self::target(&mut branches, &mut tail);
                self.next(target)?
            };
            let Some(stmt) = stmt else {
                return Err(
                    self.error(format!("if statements not closed, before: {}", self.near()))
                );
            };
            if let Some(rest) = elif_options(&stmt.body) {
                let extra = strutil::trim(rest).to_string();
                let attrs = self.parse_nv_pairs(&extra)?;
                self.check_if_conditions(&attrs, &extra)?;
                tail = None;
                branches.push(self.branch(attrs, stmt.span)?);
            } else if is_keyword(&stmt.body, "else") {
                tail = Some(Vec::new());
            } else if is_keyword(&stmt.body, "endif") {
                break;
            } else {
                let node = self.parse_one(stmt)?;
                Self::target(&mut branches, &mut tail).push(node);
            }
        }

        self.depth -= 1;
        Ok(IfNode {
            branches,
            otherwise: tail,
        })
    }

    /// Where the statements read right now belong: the body of the `${else}`
    /// when one was seen, the body of the last branch otherwise.
    fn target<'a>(
        branches: &'a mut [Branch],
        tail: &'a mut Option<Vec<Node>>,
    ) -> &'a mut Vec<Node> {
        match tail {
            Some(body) => body,
            None => &mut branches.last_mut().expect("at least one branch").body,
        }
    }

    /// Read a `for` up to its `${endfor}`.
    fn parse_for(&mut self, extra: &str, span: Span) -> Result<ForNode, ParseError> {
        self.enter(span)?;
        let attrs = self.parse_nv_pairs(extra)?;
        let Some(key) = attrs.key().map(|k| strutil::trim(k).to_string()) else {
            return Err(self.invalid(missing_key(&attrs), span));
        };
        attrs
            .number("indent", "indent")
            .map_err(|message| self.invalid(message, span))?;
        let mut body = Vec::new();
        loop {
            let Some(stmt) = self.next(&mut body)? else {
                return Err(self.error(format!(
                    "for statements not closed. before: {}",
                    self.near()
                )));
            };
            if is_keyword(&stmt.body, "endfor") {
                break;
            }
            let node = self.parse_one(stmt)?;
            body.push(node);
        }
        self.depth -= 1;
        Ok(ForNode {
            span,
            attrs,
            key,
            body,
        })
    }
}

/// Whether a statement body is the block keyword `word` - `else`, `endif` or
/// `endfor` - written in any case.
///
/// jdbgen compared these in lower case only, so `${ENDIF}` left its `if` open
/// and `${ELSE}` was silently read as a member lookup. Every other statement
/// name is case insensitive, and these follow them.
fn is_keyword(body: &str, word: &str) -> bool {
    body.eq_ignore_ascii_case(word)
}

/// The options of an `${elif:...}`, or `None` when the statement is no `elif`.
///
/// The name is trimmed and matched ignoring case, exactly as
/// [`Parser::parse_one`] reads the name of any other statement, so
/// `${ELIF:...}` and `${elif :...}` are an `elif` too.
fn elif_options(body: &str) -> Option<&str> {
    let (name, options) = body.split_once(':')?;
    is_keyword(strutil::trim(name), "elif").then_some(options)
}

/// What a statement that reads a key but names none is reported as.
fn missing_key(attrs: &Attrs) -> String {
    format!(
        "'key' or 'item' is required, but none given in: {:?}",
        attrs.names()
    )
}

/// The line separator a template is written with, reused wherever the engine
/// itself inserts one.
pub(crate) fn line_end_of(template: &str) -> &'static str {
    match template.find('\n') {
        Some(idx) if idx > 0 && template.as_bytes()[idx - 1] == b'\r' => "\r\n",
        Some(_) => "\n",
        // a template without a single line break falls back to the platform
        None if cfg!(windows) => "\r\n",
        None => "\n",
    }
}
