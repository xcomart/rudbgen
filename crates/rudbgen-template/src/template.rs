//! The parsed template: what the app holds on to.

use crate::error::{Diagnostics, ParseError, RenderError, Span};
use crate::model::Model;
use crate::parse::{Else, Node, Parser, line_end_of};
use crate::render::{RenderContext, Renderer};

/// What kind of statement a [`StatementSpan`] covers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatementKind {
    /// Text outside of any placeholder.
    Text,
    /// A `${'...'}` literal, which is text written as a placeholder.
    Literal,
    /// `${item:...}` and its shorthand.
    Item,
    /// `${super:...}`.
    Super,
    /// `${if:...}`; an `${elif:...}` is one of these as well.
    If,
    /// `${for:...}`.
    For,
    /// `${date:...}`.
    Date,
    /// `${user}`.
    User,
    /// `${author}`.
    Author,
}

/// One statement of the template and where it is written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StatementSpan {
    /// The byte range of the statement.
    pub span: Span,
    /// What the statement is.
    pub kind: StatementKind,
}

/// A template, parsed once and rendered as often as needed.
///
/// The parsed form is immutable and carries no model, so one template may be
/// rendered against every table of a schema, from more than one thread.
#[derive(Debug)]
pub struct Template {
    nodes: Vec<Node>,
    line_end: &'static str,
}

impl Template {
    /// Parse a template.
    pub fn parse(source: &str) -> Result<Template, ParseError> {
        let nodes = Parser::new(source).parse()?;
        Ok(Template {
            nodes,
            line_end: line_end_of(source),
        })
    }

    /// The line separator of the source, which is what the engine writes
    /// wherever it inserts a line break of its own.
    pub fn line_end(&self) -> &'static str {
        self.line_end
    }

    /// Render against `model`, dropping the warnings.
    pub fn render(&self, model: &dyn Model, ctx: &RenderContext) -> Result<String, RenderError> {
        let mut diags = Diagnostics::new();
        self.render_diagnosed(model, ctx, &mut diags)
    }

    /// Render against `model`, collecting the keys nothing could answer into
    /// `diags` so that the editor can mark them.
    pub fn render_diagnosed(
        &self,
        model: &dyn Model,
        ctx: &RenderContext,
        diags: &mut Diagnostics,
    ) -> Result<String, RenderError> {
        Renderer::new(ctx, self.line_end, diags).run(&self.nodes, model)
    }

    /// Every statement of the template in source order, for highlighting and
    /// for marking diagnostics.
    pub fn spans(&self) -> Vec<StatementSpan> {
        let mut res = Vec::new();
        collect_spans(&self.nodes, &mut res);
        res.sort_by_key(|s| s.span.start);
        res
    }

    /// The model members the template reads, in the order they first appear.
    ///
    /// Only the member is listed, never the processors behind it: `${a.b}` is
    /// a processor chain and reads `a`.
    pub fn fields_referenced(&self) -> Vec<String> {
        let mut res = Vec::new();
        collect_fields(&self.nodes, &mut res);
        res
    }
}

fn collect_spans(nodes: &[Node], out: &mut Vec<StatementSpan>) {
    for node in nodes {
        match node {
            Node::Text { span, literal, .. } => out.push(StatementSpan {
                span: *span,
                kind: if *literal {
                    StatementKind::Literal
                } else {
                    StatementKind::Text
                },
            }),
            Node::Item(item) => out.push(StatementSpan {
                span: item.span,
                kind: StatementKind::Item,
            }),
            Node::Super(item) => out.push(StatementSpan {
                span: item.span,
                kind: StatementKind::Super,
            }),
            Node::For(node) => {
                out.push(StatementSpan {
                    span: node.span,
                    kind: StatementKind::For,
                });
                collect_spans(&node.body, out);
            }
            Node::If(node) => collect_if_spans(node, out),
            Node::Date(node) => out.push(StatementSpan {
                span: node.span,
                kind: StatementKind::Date,
            }),
            Node::User(node) => out.push(StatementSpan {
                span: node.span,
                kind: StatementKind::User,
            }),
            Node::Author(node) => out.push(StatementSpan {
                span: node.span,
                kind: StatementKind::Author,
            }),
        }
    }
}

fn collect_if_spans(node: &crate::parse::IfNode, out: &mut Vec<StatementSpan>) {
    out.push(StatementSpan {
        span: node.span,
        kind: StatementKind::If,
    });
    collect_spans(&node.then_body, out);
    match node.otherwise.as_deref() {
        Some(Else::If(nested)) => collect_if_spans(nested, out),
        Some(Else::Body(body)) => collect_spans(body, out),
        None => {}
    }
}

fn collect_fields(nodes: &[Node], out: &mut Vec<String>) {
    let add = |key: &str, out: &mut Vec<String>| {
        if !key.is_empty() && !out.iter().any(|k| k == key) {
            out.push(key.to_string());
        }
    };
    for node in nodes {
        match node {
            Node::Item(item) | Node::Super(item) => {
                if let Some(chain) = &item.chain {
                    add(chain.key(), out);
                }
            }
            Node::For(node) => {
                if let Some(key) = &node.key {
                    add(key, out);
                }
                collect_fields(&node.body, out);
            }
            Node::If(node) => collect_if_fields(node, out),
            _ => {}
        }
    }
}

fn collect_if_fields(node: &crate::parse::IfNode, out: &mut Vec<String>) {
    if let Some(chain) = &node.chain {
        let key = chain.key();
        if !key.is_empty() && !out.iter().any(|k| k == key) {
            out.push(key.to_string());
        }
    }
    collect_fields(&node.then_body, out);
    match node.otherwise.as_deref() {
        Some(Else::If(nested)) => collect_if_fields(nested, out),
        Some(Else::Body(body)) => collect_fields(body, out),
        None => {}
    }
}
