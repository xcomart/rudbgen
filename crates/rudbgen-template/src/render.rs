//! Rendering a parsed template against a model.

use std::borrow::Cow;
use std::collections::HashMap;

use chrono::{Local, NaiveDateTime};

use crate::abbr::Abbreviations;
use crate::date;
use crate::error::{Diagnostics, RenderError, Span};
use crate::keys::{self, KeyChain};
use crate::model::{Model, Value};
use crate::parse::{Attrs, Else, ForNode, IfNode, ItemNode, Node, PlainNode};
use crate::strutil;

/// Everything a render pass needs besides the model.
///
/// The clock is a field rather than a call so that a test - or a preview that
/// has to stay stable while the user types - renders the same output twice.
#[derive(Clone, Debug)]
pub struct RenderContext {
    /// The custom variables, consulted whenever the model has no such member
    /// and the home of `${author}`.
    pub custom_vars: HashMap<String, String>,
    /// The abbreviation dictionary of the `abbr` processor.
    pub abbreviations: Abbreviations,
    /// Whether a `${name}` abbreviates by itself, that is whether an `abbr`
    /// step is inserted behind a leading `name`.
    pub apply_abbr: bool,
    /// What `${date}` formats.
    pub now: NaiveDateTime,
    /// What `${user}` renders as.
    pub user: String,
}

impl Default for RenderContext {
    fn default() -> Self {
        RenderContext {
            custom_vars: HashMap::new(),
            abbreviations: Abbreviations::new(),
            apply_abbr: false,
            now: Local::now().naive_local(),
            user: login_user(),
        }
    }
}

impl RenderContext {
    /// A context with the current time and the login user.
    pub fn new() -> Self {
        RenderContext::default()
    }

    /// Set a custom variable.
    pub fn with_var(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.custom_vars.insert(name.into(), value.into());
        self
    }

    /// Set the abbreviation dictionary.
    pub fn with_abbreviations(mut self, abbreviations: Abbreviations) -> Self {
        self.abbreviations = abbreviations;
        self
    }

    /// Turn the automatic `abbr` step behind a `name` key on or off.
    pub fn with_apply_abbr(mut self, apply: bool) -> Self {
        self.apply_abbr = apply;
        self
    }

    /// Pin the clock `${date}` reads.
    pub fn with_now(mut self, now: NaiveDateTime) -> Self {
        self.now = now;
        self
    }

    /// Set what `${user}` renders as.
    pub fn with_user(mut self, user: impl Into<String>) -> Self {
        self.user = user.into();
        self
    }
}

/// The login id of the user running the application, jdbgen's `${user}`.
fn login_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_default()
}

/// The models a statement may read from: the current one, and the one of the
/// enclosing loop.
///
/// `no` travels beside the model instead of inside it. jdbgen writes the loop
/// counter into the element object, which would force every model to be
/// mutable and every element to be owned; carrying it here renders the same
/// number and leaves the metadata tree shareable. The difference shows only on
/// a model that has no `no` member at all - jdbgen renders nothing for it,
/// this renders the position.
#[derive(Clone, Copy)]
struct Scope<'m> {
    mapper: Option<&'m dyn Model>,
    mapper_no: Option<usize>,
    supr: Option<&'m dyn Model>,
    supr_no: Option<usize>,
}

/// One render pass.
pub(crate) struct Renderer<'a> {
    ctx: &'a RenderContext,
    line_end: &'static str,
    diags: &'a mut Diagnostics,
    out: String,
}

impl<'a> Renderer<'a> {
    pub(crate) fn new(
        ctx: &'a RenderContext,
        line_end: &'static str,
        diags: &'a mut Diagnostics,
    ) -> Self {
        Renderer {
            ctx,
            line_end,
            diags,
            out: String::new(),
        }
    }

    /// Render `nodes` against `model` and hand back the text.
    pub(crate) fn run(mut self, nodes: &[Node], model: &dyn Model) -> Result<String, RenderError> {
        let scope = Scope {
            mapper: Some(model),
            mapper_no: None,
            supr: None,
            supr_no: None,
        };
        self.nodes(nodes, scope)?;
        Ok(self.out)
    }

    fn nodes(&mut self, nodes: &[Node], scope: Scope<'_>) -> Result<(), RenderError> {
        for node in nodes {
            self.node(node, scope)?;
        }
        Ok(())
    }

    fn node(&mut self, node: &Node, scope: Scope<'_>) -> Result<(), RenderError> {
        match node {
            Node::Text { text, .. } => self.out.push_str(text),
            Node::Item(item) => self.item(item, scope.mapper, scope.mapper_no)?,
            Node::Super(item) => self.item(item, scope.supr, scope.supr_no)?,
            Node::If(node) => self.if_statement(node, scope)?,
            Node::For(node) => self.for_statement(node, scope)?,
            Node::Date(node) => {
                let format = node.attrs.get("format").unwrap_or("yyyy-MM-dd");
                let text = date::format(format, &self.ctx.now)
                    .map_err(|message| RenderError::at(message, node.span))?;
                self.decorated(&node.attrs, Some(&text), node.span)?;
            }
            Node::User(PlainNode { span, attrs }) => {
                let user = self.ctx.user.clone();
                self.decorated(attrs, Some(&user), *span)?;
            }
            Node::Author(PlainNode { span, attrs }) => {
                // an author that was never set renders as nothing at all, not
                // even as its decorations
                let author = self.ctx.custom_vars.get("author").cloned();
                self.decorated(attrs, author.as_deref(), *span)?;
            }
        }
        Ok(())
    }

    /// `${item}` and `${super}`, which differ only in the model they read.
    fn item(
        &mut self,
        node: &ItemNode,
        model: Option<&dyn Model>,
        no: Option<usize>,
    ) -> Result<(), RenderError> {
        let chain = require_key(node.chain.as_ref(), &node.attrs, node.span)?;
        let value = self.resolve(chain, model, no, node.span)?;
        let text = value.to_text().into_owned();
        self.decorated(&node.attrs, Some(&text), node.span)
    }

    /// Resolve a key chain: read the member off the model, fall back to the
    /// custom variables, then run the processors of the chain over it.
    fn resolve<'m>(
        &mut self,
        chain: &KeyChain,
        model: Option<&'m dyn Model>,
        no: Option<usize>,
        span: Span,
    ) -> Result<Value<'m>, RenderError> {
        let key = chain.key();
        let value = self.lookup(key, model, no, span);

        // ${name} abbreviates by itself when the option is on; the step goes
        // behind the member and in front of whatever the template asked for
        let auto_abbr = self.ctx.apply_abbr && chain.takes_auto_abbr();
        if !auto_abbr && chain.steps.len() < 2 {
            return Ok(value);
        }
        let mut text = value.to_text().into_owned();
        if auto_abbr {
            text = self.ctx.abbreviations.apply(&text);
        }
        for step in &chain.steps[1..] {
            text = keys::apply(&step.name, &text, &step.params, &self.ctx.abbreviations)
                .map_err(|e| RenderError::at(e.message, span))?;
        }
        Ok(Value::Str(Cow::Owned(text)))
    }

    /// The value of a single member, with the custom variables behind it.
    fn lookup<'m>(
        &mut self,
        key: &str,
        model: Option<&'m dyn Model>,
        no: Option<usize>,
        span: Span,
    ) -> Value<'m> {
        // inside a loop the counter is the loop's, whatever the element says
        if let Some(no) = no
            && key == "no"
        {
            return Value::Int(no as i64);
        }
        if let Some(value) = model.and_then(|m| m.get(key))
            && !value.is_null()
        {
            return value;
        }
        if let Some(custom) = self.ctx.custom_vars.get(key) {
            return Value::Str(Cow::Owned(custom.clone()));
        }
        self.diags.warn(span, key);
        Value::Str(Cow::Borrowed(""))
    }

    /// Append a value with the decorations of its statement: `prepend` and
    /// `postpend` - both defaulting to `quote` - surround it, `padSize`
    /// together with `padDir` pads it to a fixed width in display columns -
    /// see architecture.md §7.4.
    fn decorated(
        &mut self,
        attrs: &Attrs,
        value: Option<&str>,
        span: Span,
    ) -> Result<(), RenderError> {
        let Some(value) = value else {
            return Ok(());
        };
        let pad_size = match attrs.get("padsize") {
            Some(text) => text.parse::<i64>().map_err(|_| {
                RenderError::at(format!("padSize is not a number: \"{text}\""), span)
            })?,
            None => 0,
        };
        let pad_left = attrs
            .get("paddir")
            .is_some_and(|dir| dir.eq_ignore_ascii_case("left"));
        let quote = attrs.get("quote");
        let prepend = attrs.get("prepend").or(quote);
        let postpend = attrs.get("postpend").or(quote);

        let mut text = String::with_capacity(value.len() + 2);
        if let Some(prepend) = prepend {
            text.push_str(prepend);
        }
        text.push_str(value);
        if let Some(postpend) = postpend {
            text.push_str(postpend);
        }

        if !pad_left {
            self.out.push_str(&text);
        }
        if pad_size > 0 {
            // a value wider than the padding is never cut off
            let fill = (pad_size - strutil::display_width(&text) as i64).max(0);
            self.out.push_str(&strutil::spaces(fill as usize));
        }
        if pad_left {
            self.out.push_str(&text);
        }
        Ok(())
    }

    /// `${if}`: every condition has to hold for the true branch to be
    /// rendered.
    fn if_statement(&mut self, node: &IfNode, scope: Scope<'_>) -> Result<(), RenderError> {
        let chain = require_key(node.chain.as_ref(), &node.attrs, node.span)?;
        let value = self.resolve(chain, scope.mapper, scope.mapper_no, node.span)?;
        let mut held = true;
        for cond in &node.conds {
            if !cond
                .eval(&value)
                .map_err(|e| RenderError::at(e.message, node.span))?
            {
                held = false;
                break;
            }
        }

        if held {
            self.nodes(&node.then_body, scope)
        } else {
            match node.otherwise.as_deref() {
                Some(Else::If(nested)) => self.if_statement(nested, scope),
                Some(Else::Body(body)) => self.nodes(body, scope),
                None => Ok(()),
            }
        }
    }

    /// `${for}`: render the body once per element, with the element as the
    /// model and the current model as its super.
    fn for_statement(&mut self, node: &ForNode, scope: Scope<'_>) -> Result<(), RenderError> {
        let Some(key) = node.key.as_deref() else {
            return Err(missing_key(&node.attrs, node.span));
        };
        let separator = node.attrs.get("instr");
        let indent = match node.attrs.get("indent") {
            Some(text) => text.parse::<i64>().map_err(|_| {
                RenderError::at(format!("indent is not a number: \"{text}\""), node.span)
            })?,
            None => 0,
        };
        // the skip list is a comma separated list of names, compared exactly
        let skips = node
            .attrs
            .get("skiplist")
            .map(|list| strutil::split_trim(list, ','));

        let items = match scope.mapper.and_then(|m| m.get(key)) {
            Some(Value::List(items)) => items,
            Some(other) => {
                return Err(RenderError::at(
                    format!("'{key}' is not a collection but {other:?}"),
                    node.span,
                ));
            }
            None => {
                return Err(RenderError::at(
                    format!("Model has no '{key}' member"),
                    node.span,
                ));
            }
        };

        // the separator is re-indented to the column the loop starts in
        let column_start = self.out.rfind('\n').map(|idx| idx + 1).unwrap_or(0);
        let column = strutil::display_width(&self.out[column_start..]) as i64 + indent;
        let prepend = strutil::spaces(column.max(0) as usize);

        let mut no = 0usize;
        let mut first = true;
        for item in items {
            if let Some(skips) = &skips
                && let Some(name) = item.get("name").filter(|v| !v.is_null())
                && skips.contains(&name.to_text().as_ref())
            {
                continue;
            }
            if !first && let Some(separator) = separator {
                self.separator(separator, &prepend);
            }
            no += 1;
            let inner = Scope {
                mapper: Some(item),
                mapper_no: Some(no),
                supr: scope.mapper,
                supr_no: scope.mapper_no,
            };
            self.nodes(&node.body, inner)?;
            first = false;
        }
        Ok(())
    }

    /// Write the `inStr` separator, normalising its line breaks to the line
    /// end of the template and indenting every following fragment.
    fn separator(&mut self, separator: &str, prepend: &str) {
        let mut parts = separator.split('\n');
        if let Some(head) = parts.next() {
            self.out.push_str(head.strip_suffix('\r').unwrap_or(head));
        }
        for part in parts {
            self.out.push_str(self.line_end);
            self.out.push_str(prepend);
            self.out.push_str(part.strip_suffix('\r').unwrap_or(part));
        }
    }
}

/// The key chain of a statement, or the error jdbgen reports when there is
/// none.
fn require_key<'n>(
    chain: Option<&'n KeyChain>,
    attrs: &Attrs,
    span: Span,
) -> Result<&'n KeyChain, RenderError> {
    chain.ok_or_else(|| missing_key(attrs, span))
}

fn missing_key(attrs: &Attrs, span: Span) -> RenderError {
    RenderError::at(
        format!(
            "'key' or 'item' is required, but none given in: {:?}",
            attrs.names()
        ),
        span,
    )
}
