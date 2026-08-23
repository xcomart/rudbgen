//! The two questions the editor asks about a document that a single line
//! cannot answer: which statement the caret is in, and which bracket matches
//! the one next to it.
//!
//! Both have an obvious implementation that is wrong at this size — cut the
//! whole buffer into one `&str` and scan it — and materialising a large rope
//! into one on every caret move is exactly the cost the rope was chosen to
//! avoid. So both work over a **window** of the buffer, and the window is grown
//! only as far as the answer needs.
//!
//! # What a statement is here
//!
//! rudbman's editor asked `rudbman-sql` this, and `rudbman-sql` is not in this
//! tree (architecture document, §2.1). What replaces it is smaller and asks the
//! highlighter instead: a statement runs from the first non-blank byte after
//! the previous semicolon to the semicolon that ends it, and a semicolon counts
//! only when the highlighter did **not** call it part of a string or a comment.
//! That is where all the value was — a `;` inside `'a;b'` is not a terminator —
//! and it comes for free from the colours the editor is drawing anyway. A
//! fragment holding nothing but comments and blanks is not a statement and is
//! skipped; a last statement with no closing semicolon is a statement.
//!
//! A document whose highlighter is [`None`] has no strings and no comments, so
//! every `;` in it terminates. That is the right answer for plain text: it is
//! also the only answer available, and nothing asks the question of a plain
//! text document.
//!
//! # Why a window is sound
//!
//! A semicolon that terminates a statement resets the splitter completely, so a
//! byte offset just past one is a position from which splitting the rest of the
//! document gives the same statements as splitting all of it. [`statement_at`]
//! walks backwards from the caret's line until it has one whole statement
//! behind the caret — so that "the statement before the cursor wins in the gap"
//! can be answered — forwards until it has one whole statement ahead, and
//! splits that window. The answer is the whole-buffer one; the cost is the
//! length of two statements.
//!
//! `MAX_WINDOW` caps it, for the pathological buffer with no semicolon in it
//! at all. A document whose single statement is larger than the cap gets a span
//! that starts at the cap rather than at the statement's true start.
//!
//! # Brackets
//!
//! The same shape, and the same reason the depth counter can be trusted: a
//! bracket the highlighter put inside a string or a comment is not a bracket at
//! all. The scan walks outwards line by line, lexing each line from its cached
//! state, and gives up after `MAX_BRACKET_LINES` of them.

use std::ops::Range;

use crate::buffer::Buffer;
use crate::highlight::{SyntaxCache, Token};

/// How far either way a statement window may grow, in bytes.
///
/// Two megabytes is far past any statement a person writes and far short of
/// the buffer sizes this editor is meant to survive.
const MAX_WINDOW: usize = 2 * 1024 * 1024;

/// How many lines the bracket scan will walk before giving up.
const MAX_BRACKET_LINES: usize = 5_000;

/// One statement's place in the document, in bytes.
///
/// Two ranges rather than one, because the caller wants different things at
/// different moments: [`Self::range`] covers the statement as written, the
/// terminating semicolon included, and is what to highlight or select;
/// [`Self::sql_range`] stops before that semicolon, and is what to hand a JDBC
/// `Statement` — several drivers, Oracle's above all, reject a trailing `;`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StatementSpan {
    /// First byte of the statement: its first non-blank byte, which may be the
    /// start of a comment that precedes the code.
    pub start: usize,
    /// One past the terminating semicolon, or one past the last non-blank byte
    /// if the statement is the last one and has no semicolon.
    pub end: usize,
    /// One past the last non-blank byte before the terminating semicolon.
    ///
    /// Equal to [`Self::end`] when there is no semicolon.
    pub sql_end: usize,
}

impl StatementSpan {
    /// The statement as written, the terminating semicolon included.
    pub const fn range(&self) -> Range<usize> {
        self.start..self.end
    }

    /// The statement without its terminating semicolon.
    pub const fn sql_range(&self) -> Range<usize> {
        self.start..self.sql_end
    }

    /// The text of [`Self::range`].
    ///
    /// # Panics
    ///
    /// If `source` is not the document this span was cut from.
    pub fn text<'a>(&self, source: &'a str) -> &'a str {
        &source[self.range()]
    }

    /// The text of [`Self::sql_range`] — what to execute.
    ///
    /// # Panics
    ///
    /// If `source` is not the document this span was cut from.
    pub fn sql<'a>(&self, source: &'a str) -> &'a str {
        &source[self.sql_range()]
    }

    /// Whether `offset` falls inside this statement, its terminating semicolon
    /// and the position just after it included.
    pub const fn contains(&self, offset: usize) -> bool {
        self.start <= offset && offset <= self.end
    }
}

/// A non-blank run of one line, in buffer coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    /// First byte of the run.
    start: usize,
    /// One past its last byte.
    end: usize,
    /// Whether the run is a `;` that terminates a statement.
    semicolon: bool,
    /// Whether the run is more than a comment.
    code: bool,
}

/// Appends the non-blank runs of `line` to `out`.
///
/// The highlighter's spans need not tile the line — the bytes it had no opinion
/// about are the gaps between them — so this walks spans and gaps together. A
/// string or a comment is one run whatever is inside it, which is what makes a
/// `;` in a quoted value invisible to the splitter; everything else is split at
/// blanks and around every `;`.
fn runs(buffer: &Buffer, cache: &SyntaxCache, line: usize, out: &mut Vec<Run>) {
    let text = buffer.line_text(line);
    let bytes = text.as_bytes();
    let base = buffer.line_start(line);
    let spans = cache.spans(buffer, line);

    let mut at = 0;
    let mut spans = spans.into_iter().peekable();
    while at < bytes.len() {
        let (part, token) = match spans.peek() {
            Some(span) if span.range.start <= at => {
                let span = spans.next().expect("peeked");
                let end = span.range.end.max(at).min(bytes.len());
                (at..end, Some(span.token))
            }
            Some(span) => (at..span.range.start.min(bytes.len()), None),
            None => (at..bytes.len(), None),
        };
        at = part.end.max(at + 1).min(bytes.len());
        if part.is_empty() {
            continue;
        }

        if matches!(token, Some(Token::String | Token::Comment)) {
            out.push(Run {
                start: base + part.start,
                end: base + part.end,
                semicolon: false,
                code: token != Some(Token::Comment),
            });
            continue;
        }

        // Ordinary text: blanks separate runs and a `;` is a run of its own.
        let mut run: Option<usize> = None;
        for index in part.clone() {
            let byte = bytes[index];
            let breaks = byte.is_ascii_whitespace() || byte == b';';
            match (breaks, run) {
                (true, Some(from)) => {
                    out.push(Run {
                        start: base + from,
                        end: base + index,
                        semicolon: false,
                        code: true,
                    });
                    run = None;
                }
                (false, None) => run = Some(index),
                _ => {}
            }
            if byte == b';' {
                out.push(Run {
                    start: base + index,
                    end: base + index + 1,
                    semicolon: true,
                    code: true,
                });
            }
        }
        if let Some(from) = run {
            out.push(Run {
                start: base + from,
                end: base + part.end,
                semicolon: false,
                code: true,
            });
        }
    }
}

/// The statement the caret at `offset` is in, in buffer coordinates.
///
/// The rules, in order:
///
/// 1. A statement containing `offset` wins, where "containing" includes the
///    position just after its semicolon — a cursor at `select 1;|` is in that
///    statement, not between two.
/// 2. In the blank space between two statements, the one *before* the cursor
///    wins. This is what a person means after typing a query and pressing
///    return: the statement they just finished, not the empty line they are on.
/// 3. Before the first statement, the first statement wins.
///
/// So the answer is [`None`] only when the window holds no statement at all.
pub fn statement_at(buffer: &Buffer, cache: &SyntaxCache, offset: usize) -> Option<StatementSpan> {
    let offset = offset.min(buffer.len());
    let start = window_start(buffer, cache, offset);
    let end = window_end(buffer, cache, offset);

    let mut previous = None;
    for span in statements_in(buffer, cache, start..end) {
        if offset < span.start {
            // The cursor is in the gap ahead of this statement, so it belongs
            // to whatever came before — or to this one, if nothing did.
            return previous.or(Some(span));
        }
        if span.contains(offset) {
            return Some(span);
        }
        previous = Some(span);
    }
    previous
}

/// Every statement whose runs fall inside `window`, in order.
fn statements_in(buffer: &Buffer, cache: &SyntaxCache, window: Range<usize>) -> Vec<StatementSpan> {
    let mut out = Vec::new();
    if window.is_empty() {
        return out;
    }
    let first_line = buffer.line_of(window.start);
    let last_line = buffer.line_of(window.end.min(buffer.len()));

    let mut start: Option<usize> = None;
    let mut last_end = 0;
    let mut has_code = false;
    let mut line_runs = Vec::new();

    for line in first_line..=last_line {
        line_runs.clear();
        runs(buffer, cache, line, &mut line_runs);
        for run in &line_runs {
            if run.start < window.start || run.end > window.end {
                continue;
            }
            if run.semicolon {
                if has_code {
                    out.push(StatementSpan {
                        start: start.unwrap_or(run.start),
                        end: run.end,
                        sql_end: last_end,
                    });
                }
                // `;;` or a comment with no statement under it: forget what
                // came before and start the next fragment from scratch.
                start = None;
                last_end = 0;
                has_code = false;
                continue;
            }
            if start.is_none() {
                start = Some(run.start);
            }
            last_end = run.end;
            if run.code {
                has_code = true;
            }
        }
    }
    if has_code {
        out.push(StatementSpan {
            start: start.unwrap_or(last_end),
            end: last_end,
            sql_end: last_end,
        });
    }
    out
}

/// Where a statement window may begin: past a semicolon far enough back that
/// one whole statement sits between it and the caret.
fn window_start(buffer: &Buffer, cache: &SyntaxCache, offset: usize) -> usize {
    let floor = offset.saturating_sub(MAX_WINDOW);
    let mut candidate = None;
    let mut code_since_semicolon = false;
    let mut line_runs = Vec::new();

    let first_line = buffer.line_of(offset);
    for line in (0..=first_line).rev() {
        let start = buffer.line_start(line);
        if start + buffer.line_text(line).len() < floor {
            break;
        }
        line_runs.clear();
        runs(buffer, cache, line, &mut line_runs);
        // Backwards, because the fragment boundaries are found in that order.
        for run in line_runs.iter().rev() {
            if run.start >= offset {
                continue;
            }
            if run.semicolon {
                if code_since_semicolon {
                    // One whole statement now lies between here and the caret,
                    // which is all `statement_at` can need behind it.
                    return run.end;
                }
                candidate = Some(run.end);
                continue;
            }
            if run.code {
                code_since_semicolon = true;
            }
        }
    }
    // Nothing but one fragment behind the caret: start from the top, or from
    // the nearest boundary inside the cap.
    if floor == 0 {
        0
    } else {
        candidate.unwrap_or(floor)
    }
}

/// Where a statement window may end: past the first semicolon that terminates
/// a statement holding actual code, or the end of the buffer.
///
/// A run of `;;` terminates nothing, so it does not end the window: rule three
/// of [`statement_at`] — before the first statement, the first statement wins —
/// needs a real statement ahead of the caret to answer with.
fn window_end(buffer: &Buffer, cache: &SyntaxCache, offset: usize) -> usize {
    let ceiling = (offset + MAX_WINDOW).min(buffer.len());
    let mut code_seen = false;
    let mut line_runs = Vec::new();
    let first_line = buffer.line_of(offset);
    for line in first_line..buffer.line_count() {
        let start = buffer.line_start(line);
        if start > ceiling {
            break;
        }
        line_runs.clear();
        runs(buffer, cache, line, &mut line_runs);
        for run in &line_runs {
            if run.end <= offset {
                continue;
            }
            if run.semicolon {
                if code_seen {
                    return run.end;
                }
                continue;
            }
            if run.code {
                code_seen = true;
            }
        }
    }
    buffer.len().min(ceiling.max(offset))
}

/// The bracket next to the caret and the one it pairs with.
///
/// Looks at the character before the caret first and the one after it second,
/// which is what puts the highlight on the bracket a person has just typed.
/// Answers [`None`] when neither is a bracket, or when the partner is missing.
pub fn bracket_pair(buffer: &Buffer, cache: &SyntaxCache, caret: usize) -> Option<(usize, usize)> {
    let before = buffer.prev_grapheme(caret);
    for at in [before, caret] {
        if at >= buffer.len() {
            continue;
        }
        let Some(bracket) = bracket_at(buffer, cache, at) else {
            continue;
        };
        if let Some(partner) = match_bracket(buffer, cache, at, bracket) {
            return Some((at, partner));
        }
    }
    None
}

/// One half of a bracket pair, as the scanner sees it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Bracket {
    /// The character itself.
    byte: u8,
    /// The character that closes it, or that it closes.
    partner: u8,
    /// Whether this is the opening half.
    opening: bool,
}

/// The bracket at `offset`, if there is a real one there.
///
/// "Real" means the highlighter did not put it inside a string or a comment: a
/// `(` in a quoted value is not a bracket, and neither is one in a `--` tail.
fn bracket_at(buffer: &Buffer, cache: &SyntaxCache, offset: usize) -> Option<Bracket> {
    let bracket = classify(buffer.rope().byte(offset))?;
    let (line, column) = buffer.point_of(offset);
    is_code(&cache.spans(buffer, line), column).then_some(bracket)
}

/// Whether the byte at `column` is neither string nor comment.
fn is_code(spans: &[crate::highlight::Span], column: usize) -> bool {
    !spans.iter().any(|span| {
        span.range.contains(&column) && matches!(span.token, Token::String | Token::Comment)
    })
}

/// Whether a byte is a bracket, and which way round.
const fn classify(byte: u8) -> Option<Bracket> {
    let (partner, opening) = match byte {
        b'(' => (b')', true),
        b')' => (b'(', false),
        b'[' => (b']', true),
        b']' => (b'[', false),
        b'{' => (b'}', true),
        b'}' => (b'{', false),
        _ => return None,
    };
    Some(Bracket {
        byte,
        partner,
        opening,
    })
}

/// Scans for the partner of the bracket at `from`.
fn match_bracket(
    buffer: &Buffer,
    cache: &SyntaxCache,
    from: usize,
    bracket: Bracket,
) -> Option<usize> {
    let first_line = buffer.line_of(from);
    let mut depth = 0i32;
    let lines: Box<dyn Iterator<Item = usize>> = if bracket.opening {
        Box::new(first_line..buffer.line_count().min(first_line + MAX_BRACKET_LINES))
    } else {
        Box::new((first_line.saturating_sub(MAX_BRACKET_LINES)..=first_line).rev())
    };

    for line in lines {
        let start = buffer.line_start(line);
        let text = buffer.line_text(line);
        let bytes = text.as_bytes();
        let spans = cache.spans(buffer, line);

        let mut candidates: Vec<usize> = bytes
            .iter()
            .enumerate()
            .filter(|(_, byte)| **byte == bracket.byte || **byte == bracket.partner)
            .map(|(column, _)| column)
            .filter(|column| is_code(&spans, *column))
            .collect();
        if !bracket.opening {
            candidates.reverse();
        }

        for column in candidates {
            let at = start + column;
            if bracket.opening && at < from {
                continue;
            }
            if !bracket.opening && at > from {
                continue;
            }
            if bytes[column] == bracket.byte {
                depth += 1;
            } else {
                depth -= 1;
                if depth == 0 {
                    return Some(at);
                }
            }
        }
    }
    None
}

/// The byte range of the lines `range` touches, terminators of the last one
/// excluded.
pub fn line_span(buffer: &Buffer, range: &Range<usize>) -> (usize, usize) {
    let first = buffer.line_of(range.start);
    // A selection that ends exactly at the head of a line has not touched it.
    let last_offset =
        if range.end > range.start && buffer.line_start(buffer.line_of(range.end)) == range.end {
            range.end - 1
        } else {
            range.end
        };
    (first, buffer.line_of(last_offset))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::sql_syntax::SqlHighlighter;

    /// A buffer and its cache over `text`, with the SQL highlighter.
    fn open(text: &str) -> (Buffer, SyntaxCache) {
        let buffer = Buffer::new(text);
        let cache = SyntaxCache::new(&buffer, Some(Arc::new(SqlHighlighter)));
        (buffer, cache)
    }

    /// The `sql` text of every statement of `script`.
    fn split(script: &str) -> Vec<String> {
        let (buffer, cache) = open(script);
        statements_in(&buffer, &cache, 0..buffer.len())
            .into_iter()
            .map(|span| span.sql(script).to_owned())
            .collect()
    }

    #[test]
    fn a_semicolon_in_a_string_or_a_comment_is_not_a_terminator() {
        assert_eq!(
            split("insert into t values (';'); -- and a ; here\nselect 1"),
            vec![
                "insert into t values (';')".to_owned(),
                // The comment after the semicolon belongs to the statement it
                // introduces, which is what puts the caret in the right query
                // when it is parked in the comment above one.
                "-- and a ; here\nselect 1".to_owned(),
            ]
        );
    }

    #[test]
    fn fragments_with_no_code_in_them_are_not_statements() {
        assert_eq!(
            split(";;;\nselect 1;\n;;\nselect 2;\n"),
            vec!["select 1", "select 2"]
        );
        assert!(split("   \n\n  ").is_empty());
        assert!(split("").is_empty());
        assert!(split("-- nothing under me\n").is_empty());
        assert_eq!(
            split("select 1\n"),
            vec!["select 1"],
            "no semicolon is fine"
        );
    }

    #[test]
    fn a_block_comment_and_a_multiline_string_hide_their_semicolons() {
        assert_eq!(
            split("/* block\n with a ; in it */\nselect 1;\nselect 2;"),
            vec!["/* block\n with a ; in it */\nselect 1", "select 2"]
        );
        assert_eq!(
            split("select '\nmultiline; string\n';\nselect 2;"),
            vec!["select '\nmultiline; string\n'", "select 2"]
        );
    }

    #[test]
    fn the_statement_changes_at_a_semicolon() {
        let script = "select 1;\n\nselect 2;\n";
        let (buffer, cache) = open(script);
        let sql = |offset| {
            statement_at(&buffer, &cache, offset)
                .map(|span| span.sql(script).to_owned())
                .expect("a statement")
        };
        assert_eq!(sql(3), "select 1");
        assert_eq!(sql(9), "select 1", "just past the semicolon");
        assert_eq!(sql(10), "select 1", "the blank line between them");
        assert_eq!(sql(12), "select 2");
        assert_eq!(sql(script.len()), "select 2");
        assert_eq!(sql(0), "select 1", "before the first, the first wins");
    }

    #[test]
    fn the_windowed_answer_is_the_whole_buffer_answer() {
        const SCRIPTS: &[&str] = &[
            "select 1;\n\nselect 2;\n",
            "-- a comment\nselect 1;\nselect 2",
            "insert into t values (';'); -- and a ; here\nselect 1",
            ";;;\nselect 1;\n;;\nselect 2;\n",
            "select 1\n",
            "",
            "   \n\n  ",
            "/* block\n with a ; in it */\nselect 1;\nselect 2;",
            "select '\nmultiline; string\n';\nselect 2;",
        ];
        for script in SCRIPTS {
            let (buffer, cache) = open(script);
            let all = statements_in(&buffer, &cache, 0..buffer.len());
            for offset in 0..=script.len() {
                if !script.is_char_boundary(offset) {
                    continue;
                }
                // The whole-buffer answer, worked out from the whole-buffer
                // split by the same three rules the windowed one applies.
                let mut previous = None;
                let mut whole = None;
                for span in &all {
                    if offset < span.start {
                        whole = previous.or(Some(*span));
                        break;
                    }
                    if span.contains(offset) {
                        whole = Some(*span);
                        break;
                    }
                    previous = Some(*span);
                }
                let whole = whole.or(previous);
                assert_eq!(
                    statement_at(&buffer, &cache, offset),
                    whole,
                    "at {offset} of {script:?}"
                );
            }
        }
    }

    #[test]
    fn the_window_holds_over_a_long_script() {
        // Ten thousand statements: the answer in the middle has to be the same
        // as the whole-buffer one, and getting it must not read all of it.
        let mut script = String::new();
        for i in 0..10_000 {
            script.push_str(&format!("select {i} from t;\n"));
        }
        let (buffer, cache) = open(&script);

        let offset = buffer.line_start(5_000) + 3;
        let before = cache.lex_calls();
        let span = statement_at(&buffer, &cache, offset).expect("a statement");
        let lexed = cache.lex_calls() - before;

        assert_eq!(span.sql(&script), "select 5000 from t");
        assert!(lexed < 20, "read {lexed} lines of ten thousand");
    }

    #[test]
    fn brackets_pair_across_lines() {
        let script = "select coalesce(\n  a,\n  (b + c)\n)\nfrom t;\n";
        let (buffer, cache) = open(script);

        let open_paren = script.find('(').expect("an opener");
        let close_paren = script.rfind(')').expect("a closer");
        assert_eq!(
            bracket_pair(&buffer, &cache, open_paren + 1),
            Some((open_paren, close_paren))
        );
        assert_eq!(
            bracket_pair(&buffer, &cache, close_paren + 1),
            Some((close_paren, open_paren))
        );
    }

    #[test]
    fn a_bracket_in_a_string_or_a_comment_is_not_a_bracket() {
        let script = "select '(' , 1); -- )\n";
        let (buffer, cache) = open(script);

        // The `(` inside the quotes must not be found as the partner of the
        // real `)`.
        let close_paren = script.find(')').expect("a closer");
        assert_eq!(bracket_pair(&buffer, &cache, close_paren + 1), None);

        // And the caret next to the quoted one finds nothing at all.
        let quoted = script.find('(').expect("an opener");
        assert_eq!(bracket_pair(&buffer, &cache, quoted + 1), None);
    }

    #[test]
    fn an_unmatched_bracket_pairs_with_nothing() {
        let (buffer, cache) = open("select (1;\n");
        assert_eq!(bracket_pair(&buffer, &cache, 8), None);
    }

    #[test]
    fn brackets_pair_in_a_template_too() {
        // The other shipped highlighter, over the language it is for: the
        // scanner asks the highlighter and not a SQL lexer, so it works.
        let buffer = Buffer::new("${item:key=a.replace('x','y')}\n");
        let cache = SyntaxCache::new(
            &buffer,
            Some(Arc::new(crate::template_syntax::TemplateHighlighter)),
        );
        let opener = 20;
        let closer = 28;
        assert_eq!(
            bracket_pair(&buffer, &cache, opener + 1),
            Some((opener, closer))
        );
    }

    #[test]
    fn a_line_span_stops_at_the_head_of_a_line() {
        let (buffer, _) = open("a\nb\nc\n");
        assert_eq!(line_span(&buffer, &(0..1)), (0, 0));
        // A selection that stops at the head of the next line has not reached
        // it, which is what keeps `shift-down` from indenting one line too
        // many.
        assert_eq!(line_span(&buffer, &(0..2)), (0, 0));
        assert_eq!(line_span(&buffer, &(0..4)), (0, 1));
        assert_eq!(line_span(&buffer, &(2..2)), (1, 1));
    }
}
