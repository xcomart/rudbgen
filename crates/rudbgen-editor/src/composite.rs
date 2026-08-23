//! Two highlighters over one document: a base language with template
//! statements painted over it.
//!
//! A template is a file of some other language — Java, XML, PHP, SQL — with
//! `${…}` statements sprinkled through it, and the useful colouring of one is
//! both at once: the Java is Java, and the statements stand out of it. That is
//! what this composes.
//!
//! ```ignore
//! let java = Arc::new(SomeJavaHighlighter);
//! let highlighter = Arc::new(CompositeHighlighter::new(java));
//! editor.set_highlighter(Some(highlighter), cx);
//! ```
//!
//! # How the two are kept apart
//!
//! [`TemplateHighlighter::statements`] answers with the byte ranges its
//! statements cover as well as with its spans. The base runs over the whole
//! line — so that its own state stays coherent — and its spans are then cut
//! wherever a statement stands. What is left is the base's opinion about the
//! text between statements, and the template's about the statements
//! themselves, with no overlap and in order, which is the contract
//! [`Highlighter::line`] owes its caller.
//!
//! # The one thing a composable highlighter has to promise
//!
//! Both states have to fit in one [`LineState`], so each gets half of it:
//! [`LineState::pack`] puts the base's in the low sixteen bits and the
//! template's in the high sixteen. A highlighter that means to be composed must
//! keep its state inside [`LineState::COMPOSABLE_BITS`], which the two shipped
//! here do with room to spare — three bits for SQL, nine for the template
//! language.
//!
//! # What it does not do yet
//!
//! Picking the base from the file's extension is the *app*'s decision (M4) and
//! not this crate's: `rudbgen-editor` knows `rudbgen-ui` and no file system.
//! Only the SQL highlighter ships as a base today, so a `.java` template
//! composes over nothing until a Java lexer is written; the composition itself
//! is what is settled here.
//!
//! The base sees the statement text as well as the text around it, so a `${`
//! inside what the base would call a string can still confuse the base's own
//! state. Cutting the statements out of the base's *input* instead would fix
//! that and break something worse — the base would lex `"a" + "b"` as two
//! unrelated fragments whenever a statement stood between them — so the base
//! reads the line whole, and the cut happens to its output.

use std::ops::Range;
use std::sync::Arc;

use crate::highlight::{Highlighter, LineState, Span};
use crate::template_syntax::TemplateHighlighter;

/// A base-language highlighter with template statements painted over it.
pub struct CompositeHighlighter {
    /// The language the file is written in.
    base: Arc<dyn Highlighter>,
    /// The template language, which always wins where the two meet.
    template: TemplateHighlighter,
}

impl CompositeHighlighter {
    /// Paints `base` under the template language.
    pub fn new(base: Arc<dyn Highlighter>) -> Self {
        Self {
            base,
            template: TemplateHighlighter,
        }
    }

    /// The base language, for a caller that wants to ask it something.
    pub fn base(&self) -> &Arc<dyn Highlighter> {
        &self.base
    }
}

impl Highlighter for CompositeHighlighter {
    fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState) {
        let (base_state, template_state) = state.unpack();
        let found = self.template.statements(text, template_state);
        let (base_spans, base_end) = self.base.line(text, base_state);

        let mut spans = Vec::with_capacity(base_spans.len() + found.spans.len());
        clip(base_spans, &found.regions, &mut spans);
        spans.extend(found.spans);
        spans.sort_by_key(|span| span.range.start);
        (spans, LineState::pack(base_end, found.state))
    }

    fn line_comment(&self) -> Option<&'static str> {
        // The base language's, because that is what the file is: commenting a
        // line out of a Java template writes `//`, and the template language
        // has no comment of its own to offer.
        self.base.line_comment()
    }
}

/// Writes the parts of `spans` that no region covers into `out`.
///
/// `regions` is sorted and its members do not overlap, which is what
/// [`TemplateHighlighter::statements`] promises, so one walk over each is
/// enough.
fn clip(spans: Vec<Span>, regions: &[Range<usize>], out: &mut Vec<Span>) {
    if regions.is_empty() {
        out.extend(spans);
        return;
    }
    for span in spans {
        let mut at = span.range.start;
        for region in regions {
            if region.end <= at {
                continue;
            }
            if region.start >= span.range.end {
                break;
            }
            if region.start > at {
                out.push(Span::new(at..region.start, span.token));
            }
            at = region.end;
        }
        if at < span.range.end {
            out.push(Span::new(at..span.range.end, span.token));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::Token;
    use crate::sql_syntax::SqlHighlighter;

    /// `(text, token)` for every span of `line`, lexed from `state`.
    fn lex<'a>(
        highlighter: &CompositeHighlighter,
        line: &'a str,
        state: LineState,
    ) -> (Vec<(&'a str, Token)>, LineState) {
        let (spans, end) = highlighter.line(line, state);
        let mut last = 0;
        for span in &spans {
            assert!(
                span.range.start >= last,
                "spans overlap or are unsorted in {line:?}: {spans:?}"
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

    #[test]
    fn the_base_paints_the_text_and_the_template_paints_the_statements() {
        let highlighter = CompositeHighlighter::new(Arc::new(SqlHighlighter));
        let (spans, state) = lex(
            &highlighter,
            "select ${name.camel} from t",
            LineState::START,
        );
        assert_eq!(
            spans,
            vec![
                ("select", Token::Keyword),
                ("${", Token::Punctuation),
                ("name", Token::Identifier),
                (".", Token::Operator),
                ("camel", Token::Function),
                ("}", Token::Punctuation),
                ("from", Token::Keyword),
                ("t", Token::Identifier),
            ],
            "the SQL either side of the statement is still SQL, and the \
             placeholder the base would have called a type is the template's"
        );
        assert!(state.is_start());
        assert_eq!(highlighter.line_comment(), Some("--"));
    }

    #[test]
    fn both_states_survive_one_line_state() {
        let highlighter = CompositeHighlighter::new(Arc::new(SqlHighlighter));
        // A block comment the base opens and a statement the template opens,
        // both left hanging on the same line: neither may overwrite the other.
        let (_, state) = lex(&highlighter, "/* a ${if:key=x,", LineState::START);
        let (base, template) = state.unpack();
        assert!(!base.is_start(), "the base is inside its block comment");
        assert!(!template.is_start(), "the template is inside its statement");

        let (spans, state) = lex(&highlighter, "  equals=1} still */ select", state);
        assert_eq!(
            spans,
            vec![
                ("equals", Token::Type),
                ("=", Token::Operator),
                ("1", Token::Number),
                ("}", Token::Punctuation),
                (" still */", Token::Comment),
                ("select", Token::Keyword),
            ],
            "the statement closes on the template's state and the comment on \
             the base's"
        );
        assert!(state.is_start());
    }

    #[test]
    fn clipping_cuts_a_base_span_a_statement_stands_inside() {
        let spans = vec![Span::new(0..20, Token::Comment)];
        let mut out = Vec::new();
        clip(spans, &[5..8, 12..14], &mut out);
        assert_eq!(
            out,
            vec![
                Span::new(0..5, Token::Comment),
                Span::new(8..12, Token::Comment),
                Span::new(14..20, Token::Comment),
            ]
        );
    }
}
