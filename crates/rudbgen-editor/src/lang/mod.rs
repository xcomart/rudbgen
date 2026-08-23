//! Per-extension syntax highlighters, and the resolver that turns a template
//! file's path into the right one.
//!
//! A jdbgen template is a file of some other language with `${…}` statements
//! sprinkled through it (architecture document, §8): a `Model.java.tpl`
//! generates Java, a `mapping.xml.tpl` generates XML, and so on. Colouring one
//! well needs two lexers at once — the base language, and the template
//! language over it — which is exactly what
//! [`CompositeHighlighter`](crate::composite::CompositeHighlighter) composes.
//! This module is the other half: a base [`Highlighter`] per language the
//! generator targets, and [`template_highlighter_for_path`], which reads a
//! path's extension and hands back the right composite.
//!
//! # The two ways a language is implemented here
//!
//! Three languages get a lexer of their own, because their grammar is not the
//! "keyword, string, line comment, block comment" shape the others share:
//! [`java::JavaHighlighter`] (annotations, text blocks), [`xml::XmlHighlighter`]
//! (tags and attributes, also used for HTML), and [`php::PhpHighlighter`]
//! (`$variables`, a `<?php` boundary, and strings that — unlike every other
//! language here — span line breaks). Every other language —  C#, Kotlin,
//! TypeScript and JavaScript, Go, Rust, Python, JSON, YAML, Markdown,
//! properties/INI files, and shell scripts — is [`clike::CLikeHighlighter`]
//! with a [`clike::CLikeConfig`] of its own keyword table and comment/string
//! syntax. SQL is [`SqlHighlighter`](crate::sql_syntax::SqlHighlighter),
//! unchanged.
//!
//! # Extension resolution
//!
//! [`highlighter_for_extension`] is case-insensitive and looks at the
//! extension alone. [`template_highlighter_for_path`] additionally strips a
//! trailing `.tpl` or `.tmpl` before that lookup, so `Model.java.tpl` composes
//! over Java the same as a hypothetical bare `Model.java` opened as a
//! template would; a path with no extension [`highlighter_for_extension`]
//! recognizes falls back to the template language on its own, with no base
//! underneath, exactly as an editor with no highlighter falls back to plain
//! text.

pub mod clike;
pub mod java;
pub mod php;
pub mod xml;

use std::path::Path;
use std::sync::Arc;

use crate::composite::CompositeHighlighter;
use crate::highlight::Highlighter;
use crate::sql_syntax::SqlHighlighter;
use crate::template_syntax::TemplateHighlighter;
use clike::{CLikeConfig, CLikeHighlighter};
use java::JavaHighlighter;
use php::PhpHighlighter;
use xml::XmlHighlighter;

/// C#.
const CSHARP: CLikeConfig = CLikeConfig {
    keywords: &[
        "abstract",
        "as",
        "async",
        "await",
        "base",
        "bool",
        "break",
        "byte",
        "case",
        "catch",
        "char",
        "checked",
        "class",
        "const",
        "continue",
        "decimal",
        "default",
        "delegate",
        "do",
        "double",
        "dynamic",
        "else",
        "enum",
        "event",
        "explicit",
        "extern",
        "false",
        "finally",
        "fixed",
        "float",
        "for",
        "foreach",
        "get",
        "goto",
        "if",
        "implicit",
        "in",
        "init",
        "int",
        "interface",
        "internal",
        "is",
        "lock",
        "long",
        "namespace",
        "new",
        "null",
        "object",
        "operator",
        "out",
        "override",
        "params",
        "partial",
        "private",
        "protected",
        "public",
        "readonly",
        "record",
        "ref",
        "return",
        "sbyte",
        "sealed",
        "set",
        "short",
        "sizeof",
        "stackalloc",
        "static",
        "string",
        "struct",
        "switch",
        "this",
        "throw",
        "true",
        "try",
        "typeof",
        "uint",
        "ulong",
        "unchecked",
        "unsafe",
        "ushort",
        "using",
        "var",
        "virtual",
        "void",
        "volatile",
        "while",
        "yield",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    triple_quotes: &[],
};

/// Kotlin.
const KOTLIN: CLikeConfig = CLikeConfig {
    keywords: &[
        "abstract",
        "actual",
        "annotation",
        "as",
        "break",
        "by",
        "catch",
        "class",
        "companion",
        "const",
        "constructor",
        "continue",
        "crossinline",
        "data",
        "do",
        "dynamic",
        "else",
        "enum",
        "expect",
        "external",
        "false",
        "final",
        "finally",
        "for",
        "fun",
        "get",
        "if",
        "import",
        "in",
        "infix",
        "init",
        "inline",
        "inner",
        "interface",
        "internal",
        "is",
        "lateinit",
        "noinline",
        "null",
        "object",
        "open",
        "operator",
        "out",
        "override",
        "package",
        "private",
        "protected",
        "public",
        "reified",
        "return",
        "sealed",
        "set",
        "super",
        "suspend",
        "tailrec",
        "this",
        "throw",
        "true",
        "try",
        "typealias",
        "typeof",
        "val",
        "var",
        "vararg",
        "when",
        "where",
        "while",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    triple_quotes: &[],
};

/// TypeScript and JavaScript: one config for both, since JavaScript's
/// keywords are a subset of TypeScript's and a `.js` file that used a
/// TypeScript-only reserved word as an identifier would be unusual enough to
/// not be worth a second table.
const TYPESCRIPT: CLikeConfig = CLikeConfig {
    keywords: &[
        "abstract",
        "any",
        "as",
        "async",
        "await",
        "bigint",
        "boolean",
        "break",
        "case",
        "catch",
        "class",
        "const",
        "continue",
        "debugger",
        "declare",
        "default",
        "delete",
        "do",
        "else",
        "enum",
        "export",
        "extends",
        "false",
        "finally",
        "for",
        "from",
        "function",
        "get",
        "if",
        "implements",
        "import",
        "in",
        "infer",
        "instanceof",
        "interface",
        "is",
        "keyof",
        "let",
        "namespace",
        "never",
        "new",
        "null",
        "number",
        "object",
        "of",
        "package",
        "private",
        "protected",
        "public",
        "readonly",
        "return",
        "set",
        "static",
        "string",
        "super",
        "switch",
        "symbol",
        "this",
        "throw",
        "true",
        "try",
        "type",
        "typeof",
        "unknown",
        "var",
        "void",
        "while",
        "with",
        "yield",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    triple_quotes: &[],
};

/// Go.
const GO: CLikeConfig = CLikeConfig {
    keywords: &[
        "bool",
        "break",
        "byte",
        "case",
        "chan",
        "complex128",
        "complex64",
        "const",
        "continue",
        "default",
        "defer",
        "else",
        "error",
        "fallthrough",
        "false",
        "float32",
        "float64",
        "for",
        "func",
        "go",
        "goto",
        "if",
        "import",
        "int",
        "int16",
        "int32",
        "int64",
        "int8",
        "interface",
        "iota",
        "map",
        "nil",
        "package",
        "range",
        "return",
        "rune",
        "select",
        "string",
        "struct",
        "switch",
        "true",
        "type",
        "uint",
        "uint16",
        "uint32",
        "uint64",
        "uint8",
        "uintptr",
        "var",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    triple_quotes: &[],
};

/// Rust. Uppercase library types (`String`, `Vec`, `Option`, `Self`, …) are
/// deliberately not in the table: they are not reserved words, and the
/// PascalCase rule every C-like config shares (§ module docs of
/// [`clike`](crate::lang::clike)) already paints them as [`Token::Type`].
///
/// [`Token::Type`]: crate::highlight::Token::Type
const RUST: CLikeConfig = CLikeConfig {
    keywords: &[
        "as", "async", "await", "bool", "break", "char", "const", "continue", "crate", "dyn",
        "else", "enum", "extern", "f32", "f64", "false", "fn", "for", "i128", "i16", "i32", "i64",
        "i8", "if", "impl", "in", "isize", "let", "loop", "match", "mod", "move", "mut", "pub",
        "ref", "return", "self", "static", "str", "struct", "super", "trait", "true", "type",
        "u128", "u16", "u32", "u64", "u8", "unsafe", "use", "usize", "where", "while",
    ],
    line_comments: &["//"],
    block_comment: Some(("/*", "*/")),
    triple_quotes: &[],
};

/// Python. `False`, `None` and `True` are capitalized exactly as Python
/// reserves them; the keyword table is matched before the PascalCase rule
/// ever runs, so they are keywords and a class `Foo` is still a type.
const PYTHON: CLikeConfig = CLikeConfig {
    keywords: &[
        "False", "None", "True", "and", "as", "assert", "async", "await", "break", "class",
        "continue", "def", "del", "elif", "else", "except", "finally", "for", "from", "global",
        "if", "import", "in", "is", "lambda", "nonlocal", "not", "or", "pass", "raise", "return",
        "try", "while", "with", "yield",
    ],
    line_comments: &["#"],
    block_comment: None,
    triple_quotes: &["'''", "\"\"\""],
};

/// JSON. `CLikeHighlighter`'s generic quote handling accepts a `'...'` string
/// as well as a `"..."` one, which is looser than the JSON grammar — a
/// deliberate simplification, since a `.json` document that actually uses
/// single quotes is already invalid and no shade of colour will fix that.
const JSON: CLikeConfig = CLikeConfig {
    keywords: &["false", "null", "true"],
    line_comments: &[],
    block_comment: None,
    triple_quotes: &[],
};

/// YAML, as much of it as a "simple" highlighter needs: scalars, `#`
/// comments, numbers and the handful of reserved-looking words. It does not
/// know a block scalar (`|`, `>`) from a folded string, or a mapping key from
/// a plain scalar — both need a lexer that tracks indentation, which is a
/// grammar of its own and not a config of this one.
const YAML: CLikeConfig = CLikeConfig {
    keywords: &["false", "no", "null", "true", "yes"],
    line_comments: &["#"],
    block_comment: None,
    triple_quotes: &[],
};

/// Markdown, as "simple" as a highlighter can be and still be one: no
/// keywords and no comment syntax, so the only thing it paints at all is a
/// bare number. Headings, emphasis and code spans are inline punctuation, not
/// tokens a per-line lexer without look-behind can tell from the prose around
/// them without far more machinery than a "simple" highlighter is asking for.
const MARKDOWN: CLikeConfig = CLikeConfig {
    keywords: &[],
    line_comments: &[],
    block_comment: None,
    triple_quotes: &[],
};

/// `.properties` files: both `#` and `!` introduce a comment, no block
/// comment, no keywords.
const PROPERTIES: CLikeConfig = CLikeConfig {
    keywords: &[],
    line_comments: &["#", "!"],
    block_comment: None,
    triple_quotes: &[],
};

/// `.ini`/`.cfg`/`.conf` files: `;` is the classic comment marker, and `#` is
/// accepted too, since enough INI dialects allow it that rejecting it would
/// surprise more people than allowing it does.
const INI: CLikeConfig = CLikeConfig {
    keywords: &[],
    line_comments: &[";", "#"],
    block_comment: None,
    triple_quotes: &[],
};

/// Shell scripts (`sh`, `bash`, `zsh`): the POSIX control-flow words. `$VAR`
/// and `$(...)` get no special reading — a `$` is punctuation and the name
/// after it an ordinary identifier — which is the same simplification the
/// rest of this table makes for every language it does not have a dedicated
/// lexer for.
const SH: CLikeConfig = CLikeConfig {
    keywords: &[
        "case", "do", "done", "elif", "else", "esac", "fi", "for", "function", "if", "in",
        "select", "then", "until", "while",
    ],
    line_comments: &["#"],
    block_comment: None,
    triple_quotes: &[],
};

/// The base highlighter for one file extension, if this crate ships one.
///
/// `ext` is matched case-insensitively and without its leading dot — the same
/// form [`std::path::Path::extension`] returns. `None` covers every extension
/// the generator does not target a base lexer for; [`template_highlighter_for_path`]
/// is what falls back to the template language alone in that case.
pub fn highlighter_for_extension(ext: &str) -> Option<Arc<dyn Highlighter>> {
    Some(match ext.to_ascii_lowercase().as_str() {
        "java" => Arc::new(JavaHighlighter),
        "xml" | "html" | "htm" => Arc::new(XmlHighlighter),
        "php" => Arc::new(PhpHighlighter),
        "sql" => Arc::new(SqlHighlighter),
        "cs" => Arc::new(CLikeHighlighter(&CSHARP)),
        "kt" | "kts" => Arc::new(CLikeHighlighter(&KOTLIN)),
        "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" => Arc::new(CLikeHighlighter(&TYPESCRIPT)),
        "go" => Arc::new(CLikeHighlighter(&GO)),
        "rs" => Arc::new(CLikeHighlighter(&RUST)),
        "py" | "pyw" => Arc::new(CLikeHighlighter(&PYTHON)),
        "json" => Arc::new(CLikeHighlighter(&JSON)),
        "yaml" | "yml" => Arc::new(CLikeHighlighter(&YAML)),
        "md" | "markdown" => Arc::new(CLikeHighlighter(&MARKDOWN)),
        "properties" => Arc::new(CLikeHighlighter(&PROPERTIES)),
        "ini" | "cfg" | "conf" => Arc::new(CLikeHighlighter(&INI)),
        "sh" | "bash" | "zsh" => Arc::new(CLikeHighlighter(&SH)),
        _ => return None,
    })
}

/// The highlighter for a template file at `path`: its base language, if
/// [`highlighter_for_extension`] has one, with the template language painted
/// over it — or the template language alone, for a base the table above does
/// not cover.
///
/// A trailing `.tpl` or `.tmpl` is stripped before the extension is read, so
/// `Model.java.tpl` resolves on `java` exactly as `Model.java` would. Neither
/// suffix is itself a language, so a bare `Model.tpl` — no second extension —
/// resolves to the template language alone, the same as a `.tpl` file with an
/// extension this crate has no base lexer for.
pub fn template_highlighter_for_path(path: &Path) -> Arc<dyn Highlighter> {
    let raw_ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or_default();
    let lang_ext = if raw_ext.eq_ignore_ascii_case("tpl") || raw_ext.eq_ignore_ascii_case("tmpl") {
        path.file_stem()
            .map(Path::new)
            .and_then(|stem| stem.extension())
            .and_then(|e| e.to_str())
            .unwrap_or_default()
    } else {
        raw_ext
    };
    match highlighter_for_extension(lang_ext) {
        Some(base) => Arc::new(CompositeHighlighter::new(base)),
        None => Arc::new(TemplateHighlighter),
    }
}

/// Test-only helpers shared by every highlighter in this module: running a
/// line through a [`Highlighter`] while checking the span contract it owes
/// its caller (sorted, non-overlapping, inside the line, on char boundaries,
/// never empty), the same checks
/// [`highlight`](crate::highlight)'s and
/// [`template_syntax`](crate::template_syntax)'s own tests hand-roll one at a
/// time.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::highlight::{Highlighter, LineState, Span, Token};

    /// Runs `highlighter` over `line` from `state`, checking the span
    /// contract, and answers with `(text, token)` pairs and the end state.
    pub(crate) fn lex<'a>(
        highlighter: &dyn Highlighter,
        line: &'a str,
        state: LineState,
    ) -> (Vec<(&'a str, Token)>, LineState) {
        let (spans, end) = highlighter.line(line, state);
        check_span_invariants(&spans, line);
        (
            spans
                .iter()
                .map(|span| (&line[span.range.clone()], span.token))
                .collect(),
            end,
        )
    }

    /// Every line of `text`, lexed the way [`crate::highlight::SyntaxCache`]
    /// lexes them: each from the state the line before it ended in.
    pub(crate) fn lex_lines<'a>(
        highlighter: &dyn Highlighter,
        text: &'a str,
    ) -> Vec<Vec<(&'a str, Token)>> {
        let mut state = LineState::START;
        let mut out = Vec::new();
        for line in text.split('\n') {
            let (spans, next) = lex(highlighter, line, state);
            state = next;
            out.push(spans);
        }
        out
    }

    /// Checks that `spans` are sorted, non-overlapping, non-empty, inside
    /// `line`, and cut on character boundaries -- the contract
    /// [`Highlighter::line`] owes its caller.
    pub(crate) fn check_span_invariants(spans: &[Span], line: &str) {
        let mut last = 0;
        for span in spans {
            assert!(
                span.range.start >= last,
                "spans overlap or are unsorted in {line:?}: {spans:?}"
            );
            assert!(
                span.range.end <= line.len(),
                "span past the end of {line:?}: {spans:?}"
            );
            assert!(!span.is_empty(), "empty span in {line:?}: {spans:?}");
            assert!(
                line.is_char_boundary(span.range.start) && line.is_char_boundary(span.range.end),
                "span not on a char boundary in {line:?}: {spans:?}"
            );
            last = span.range.end;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::highlight::LineState;

    /// Every keyword table in this file is sorted -- required for the
    /// binary search each [`CLikeConfig`] is looked up with -- and every
    /// entry is exactly as it is meant to be matched (Python's three
    /// capitalized words aside).
    #[test]
    fn every_keyword_table_is_sorted() {
        for config in [
            &CSHARP,
            &KOTLIN,
            &TYPESCRIPT,
            &GO,
            &RUST,
            &PYTHON,
            &JSON,
            &YAML,
            &MARKDOWN,
            &PROPERTIES,
            &INI,
            &SH,
        ] {
            for pair in config.keywords.windows(2) {
                assert!(pair[0] < pair[1], "{pair:?} is out of order");
            }
        }
    }

    #[test]
    fn every_config_fits_the_composable_budget() {
        // Two bits of state today; the assertion is what would catch a future
        // config that needed a third triple-quote slot or more.
        for config in [
            &CSHARP,
            &KOTLIN,
            &TYPESCRIPT,
            &GO,
            &RUST,
            &PYTHON,
            &JSON,
            &YAML,
            &MARKDOWN,
            &PROPERTIES,
            &INI,
            &SH,
        ] {
            assert!(config.triple_quotes.len() <= 2);
        }
    }

    #[test]
    fn known_extensions_resolve_case_insensitively() {
        for ext in [
            "java",
            "XML",
            "Html",
            "htm",
            "php",
            "sql",
            "CS",
            "kt",
            "kts",
            "ts",
            "tsx",
            "js",
            "jsx",
            "mjs",
            "cjs",
            "go",
            "rs",
            "py",
            "pyw",
            "json",
            "yaml",
            "yml",
            "md",
            "markdown",
            "properties",
            "ini",
            "cfg",
            "conf",
            "sh",
            "bash",
            "zsh",
        ] {
            assert!(
                highlighter_for_extension(ext).is_some(),
                "{ext} should resolve to a highlighter"
            );
        }
        assert!(highlighter_for_extension("").is_none());
        assert!(highlighter_for_extension("cobol").is_none());
    }

    #[test]
    fn a_template_path_composes_over_its_base_language() {
        let h = template_highlighter_for_path(Path::new("Model.java.tpl"));
        // A Java keyword, painted through the composite, is still a keyword.
        let (spans, _) = h.line("public class ${name} {", LineState::START);
        assert!(
            spans
                .iter()
                .any(|s| s.range == (0..6) && s.token == crate::highlight::Token::Keyword),
            "the base language survives composition: {spans:?}"
        );
        assert_eq!(
            h.line_comment(),
            Some("//"),
            "the base's comment, not the template's"
        );
    }

    #[test]
    fn a_double_extension_resolves_on_the_inner_one() {
        for (path, expect_comment) in [
            ("Model.java.tpl", Some("//")),
            ("Model.java.tmpl", Some("//")),
            ("query.sql", Some("--")),
            ("mapping.xml.TPL", None),
            ("plain.tpl", None),
            ("no_extension_at_all", None),
        ] {
            let h = template_highlighter_for_path(Path::new(path));
            assert_eq!(h.line_comment(), expect_comment, "for {path}");
        }
    }
}
