//! What the variable palette offers, and what the completion popup filters out
//! of it (architecture document, §4.5).
//!
//! Everything here is pure: a list of entries, the rule that decides which of
//! them a caret position may take, and the substring filter over the result.
//! The panel that draws the list is [`crate::variable_palette`] and the popup
//! that anchors to the caret is in [`crate::template_pane`]; both are gpui, and
//! neither has a rule of its own.
//!
//! # Why the entries are a table and not a function of the model
//!
//! The model answers a key with a value or with nothing, so it can say whether
//! `${javaType}` resolves — but not that there *is* a `javaType`, what it
//! means, or that it belongs to a column rather than to a table. That is
//! documentation, and documentation is what a palette is. The table below is
//! jdbgen's `docs/template-reference.md` and `rudbgen-meta`'s `Model` impls
//! read together: every key those impls answer appears here exactly once, so a
//! field added to the model without a line here is a field the palette cannot
//! offer.
//!
//! # The one thing the model is asked for
//!
//! The example beside an entry. `${name}` is worth more as
//! `name → T_SAMPLE_ALBUM` than as `name → the table name`, and that value can
//! only come from a table that was actually read — so [`items`] takes the
//! table the preview is rendered against, asks it for each key through
//! [`Model::get`], and leaves the example off when there is no table.

use std::borrow::Cow;

use gpui::SharedString;
use rudbgen_meta::Table;
use rudbgen_template::Model;

use crate::i18n::ts;

/// How long an example value may be before it is cut short.
///
/// A remark or a default value can be a paragraph; the palette has one line
/// per entry and the example is the smaller half of it.
const EXAMPLE_LIMIT: usize = 28;

/// The eight groups the palette is divided into (§4.5).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Section {
    /// Fields of the table being rendered.
    Table,
    /// Fields of a column, which are only in scope inside `${for:item=columns}`.
    Column,
    /// Fields of a foreign key, in scope inside `imports`, `exports` and `fk`.
    ForeignKey,
    /// Fields of an index, in scope inside `${for:item=indexes}`.
    Index,
    /// The statements the language is made of.
    Statement,
    /// The processors a key chain may run a value through.
    Decorator,
    /// The conditions `${if}` and `${elif}` branch on.
    Condition,
    /// The variables the Generate tab's table defines.
    Custom,
}

impl Section {
    /// Every section, in the order the panel lists them.
    pub const ALL: [Section; 8] = [
        Section::Table,
        Section::Column,
        Section::ForeignKey,
        Section::Index,
        Section::Statement,
        Section::Decorator,
        Section::Condition,
        Section::Custom,
    ];

    /// A stable identifier, for element ids and for tests.
    pub const fn id(self) -> &'static str {
        match self {
            Section::Table => "table",
            Section::Column => "column",
            Section::ForeignKey => "fk",
            Section::Index => "index",
            Section::Statement => "statement",
            Section::Decorator => "decorator",
            Section::Condition => "condition",
            Section::Custom => "custom",
        }
    }

    /// What the panel writes over the group.
    pub fn title(self) -> SharedString {
        match self {
            Section::Table => ts!("palette.section_table"),
            Section::Column => ts!("palette.section_column"),
            Section::ForeignKey => ts!("palette.section_fk"),
            Section::Index => ts!("palette.section_index"),
            Section::Statement => ts!("palette.section_statement"),
            Section::Decorator => ts!("palette.section_decorator"),
            Section::Condition => ts!("palette.section_condition"),
            Section::Custom => ts!("palette.section_custom"),
        }
    }
}

/// Where in a statement an entry may be written.
///
/// This is not the same question as which [`Section`] an entry is listed
/// under: the extra decorators are *options* — `${item:key=name, padSize=20}` —
/// but a reader looks for them beside the chain processors, so they are listed
/// with the decorators and completed with the options.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ctx {
    /// A member of the model: `${name}`, or `key=name` inside a statement.
    Field,
    /// A statement name: what may follow `${`.
    Statement,
    /// A processor in a key chain: what may follow a `.`.
    Chain,
    /// An option of a statement: what may follow a `:` or a `,`.
    Option,
}

/// One entry of the palette.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PaletteItem {
    /// What the row is labelled, and what the filter matches on.
    pub name: SharedString,
    /// What a click on the row inserts at the caret — the whole `${…}` form.
    pub click: SharedString,
    /// What accepting the entry from the completion popup inserts, which is
    /// only the part that is missing: the caret is already inside a statement.
    pub complete: SharedString,
    /// Which group the panel lists it under.
    pub section: Section,
    /// Where it may be written.
    pub ctx: Ctx,
    /// The statements this option belongs to; empty means every one of them.
    ///
    /// Only read for [`Ctx::Option`]: `inStr` is a `for` control and offering
    /// it inside an `${if}` would be offering something the parser rejects.
    pub owners: &'static [&'static str],
    /// One line about what it does, from the template reference.
    pub description: SharedString,
    /// What it renders to for the table the preview is using, when there is
    /// one and the value is worth showing.
    pub example: Option<SharedString>,
}

/// One row of the static table below.
struct Spec {
    name: &'static str,
    click: &'static str,
    section: Section,
    ctx: Ctx,
    owners: &'static [&'static str],
    /// The i18n key of the description.
    doc: &'static str,
}

/// The statements that take a key and the extra decorators.
const VALUE_STATEMENTS: &[&str] = &["item", "super"];

/// The statements that take a condition.
const IF_STATEMENTS: &[&str] = &["if", "elif"];

/// Everything that may carry an extra decorator (`padSize` and its four
/// companions): the three statements that render a value.
const DECORATED: &[&str] = &["item", "super", "date", "user", "author"];

/// A field of the table.
const fn table(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        name,
        click: "",
        section: Section::Table,
        ctx: Ctx::Field,
        owners: &[],
        doc,
    }
}

/// A field of a column.
const fn column(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        section: Section::Column,
        ..table(name, doc)
    }
}

/// A field of a foreign key.
const fn fk(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        section: Section::ForeignKey,
        ..table(name, doc)
    }
}

/// A field of an index.
const fn index(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        section: Section::Index,
        ..table(name, doc)
    }
}

/// A statement, with the whole form a click writes.
const fn statement(name: &'static str, click: &'static str, doc: &'static str) -> Spec {
    Spec {
        name,
        click,
        section: Section::Statement,
        ctx: Ctx::Statement,
        owners: &[],
        doc,
    }
}

/// A control of `${for}`, listed with the statements because it is part of one.
const fn control(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        name,
        click: "",
        section: Section::Statement,
        ctx: Ctx::Option,
        owners: &["for"],
        doc,
    }
}

/// A processor of a key chain.
const fn decorator(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        name,
        click: "",
        section: Section::Decorator,
        ctx: Ctx::Chain,
        owners: &[],
        doc,
    }
}

/// An extra decorator, which is an option rather than a chain step.
const fn extra(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        name,
        click: "",
        section: Section::Decorator,
        ctx: Ctx::Option,
        owners: DECORATED,
        doc,
    }
}

/// A condition of `${if}` and `${elif}`.
const fn condition(name: &'static str, doc: &'static str) -> Spec {
    Spec {
        name,
        click: "",
        section: Section::Condition,
        ctx: Ctx::Option,
        owners: IF_STATEMENTS,
        doc,
    }
}

/// Every entry that does not depend on the connection.
///
/// The order inside a section is the order the panel lists it in, which is the
/// order `rudbgen-meta`'s `Model` impls answer the keys in rather than
/// alphabetical: a reader looking for `typeString` finds it beside `typeName`.
const SPECS: &[Spec] = &[
    // --- table ---
    table("catalog", "palette.table.catalog"),
    table("schema", "palette.table.schema"),
    table("name", "palette.table.name"),
    table("table", "palette.table.table"),
    table("title", "palette.table.title"),
    table("type", "palette.table.type"),
    table("remarks", "palette.table.remarks"),
    table("columns", "palette.table.columns"),
    table("keys", "palette.table.keys"),
    table("notKeys", "palette.table.notKeys"),
    table("imports", "palette.table.imports"),
    table("exports", "palette.table.exports"),
    table("indexes", "palette.table.indexes"),
    table("icon", "palette.table.icon"),
    table("no", "palette.table.position"),
    // --- column ---
    column("catalog", "palette.column.catalog"),
    column("schema", "palette.column.schema"),
    column("table", "palette.column.table"),
    column("name", "palette.column.name"),
    column("column", "palette.column.column"),
    column("typeName", "palette.column.typeName"),
    column("typeString", "palette.column.typeString"),
    column("isKey", "palette.column.isKey"),
    column("isCharType", "palette.column.isCharType"),
    column("length", "palette.column.length"),
    column("precision", "palette.column.precision"),
    column("scale", "palette.column.scale"),
    column("nullable", "palette.column.nullable"),
    column("remarks", "palette.column.remarks"),
    column("defVal", "palette.column.defVal"),
    column("dataType", "palette.column.dataType"),
    column("jdbcType", "palette.column.jdbcType"),
    column("javaType", "palette.column.javaType"),
    column("autoIncrement", "palette.column.autoIncrement"),
    column("keySeq", "palette.column.keySeq"),
    column("fk", "palette.column.fk"),
    column("no", "palette.column.position"),
    // --- foreign key ---
    fk("name", "palette.fk.name"),
    fk("columns", "palette.fk.columns"),
    fk("refCatalog", "palette.fk.refCatalog"),
    fk("refSchema", "palette.fk.refSchema"),
    fk("refTable", "palette.fk.refTable"),
    fk("refColumns", "palette.fk.refColumns"),
    fk("onUpdate", "palette.fk.onUpdate"),
    fk("onDelete", "palette.fk.onDelete"),
    fk("no", "palette.fk.position"),
    fk("catalog", "palette.fk.catalog"),
    fk("schema", "palette.fk.schema"),
    fk("table", "palette.fk.table"),
    fk("column", "palette.fk.column"),
    // --- index ---
    index("name", "palette.index.name"),
    index("unique", "palette.index.unique"),
    index("columns", "palette.index.columns"),
    index("no", "palette.index.position"),
    // --- statements ---
    statement("item", "${item:key=name}", "palette.stmt.item"),
    statement("super", "${super:key=name}", "palette.stmt.super"),
    statement(
        "if",
        "${if:item=type, equals='TABLE'}\n${endif}",
        "palette.stmt.if",
    ),
    statement(
        "elif",
        "${elif:item=type, equals='VIEW'}",
        "palette.stmt.elif",
    ),
    statement("else", "${else}", "palette.stmt.else"),
    statement("endif", "${endif}", "palette.stmt.endif"),
    statement("for", "${for:item=columns}\n${endfor}", "palette.stmt.for"),
    statement("endfor", "${endfor}", "palette.stmt.endfor"),
    statement("date", "${date}", "palette.stmt.date"),
    statement("user", "${user}", "palette.stmt.user"),
    statement("author", "${author}", "palette.stmt.author"),
    control("inStr", "palette.stmt.inStr"),
    control("indent", "palette.stmt.indent"),
    control("skipList", "palette.stmt.skipList"),
    // --- decorators ---
    decorator("abbr", "palette.dec.abbr"),
    decorator("suffix", "palette.dec.suffix"),
    decorator("prefix", "palette.dec.prefix"),
    decorator("camel", "palette.dec.camel"),
    decorator("pascal", "palette.dec.pascal"),
    decorator("snake", "palette.dec.snake"),
    decorator("screaming", "palette.dec.screaming"),
    decorator("skewer", "palette.dec.skewer"),
    decorator("kebab", "palette.dec.kebab"),
    decorator("lower", "palette.dec.lower"),
    decorator("upper", "palette.dec.upper"),
    decorator("replace", "palette.dec.replace"),
    extra("padSize", "palette.dec.padSize"),
    extra("padDir", "palette.dec.padDir"),
    extra("quote", "palette.dec.quote"),
    extra("prepend", "palette.dec.prepend"),
    extra("postpend", "palette.dec.postpend"),
    // --- conditions ---
    condition("equals", "palette.cond.equals"),
    condition("notEquals", "palette.cond.notEquals"),
    condition("startsWith", "palette.cond.startsWith"),
    condition("notStartsWith", "palette.cond.notStartsWith"),
    condition("endsWith", "palette.cond.endsWith"),
    condition("notEndsWith", "palette.cond.notEndsWith"),
    condition("contains", "palette.cond.contains"),
    condition("notContains", "palette.cond.notContains"),
    condition("matches", "palette.cond.matches"),
    condition("notMatches", "palette.cond.notMatches"),
];

/// The `key=` option every statement that reads a value takes.
///
/// Not in [`SPECS`] because it is neither a field nor a decorator and would
/// have to be listed under a ninth section nobody asked for; it exists so that
/// the completion inside `${item:` offers the option a user is actually about
/// to type.
const KEY_OPTIONS: [(&str, &str); 3] = [
    ("key", "palette.opt.key"),
    ("item", "palette.opt.item"),
    ("format", "palette.opt.format"),
];

/// The whole palette for the current model and the current profile.
///
/// `table` is the table the live preview renders against, and is what the
/// examples come from; `custom` is the Generate tab's variable table, in the
/// order it was typed.
pub fn items(table: Option<&Table>, custom: &[(String, String)]) -> Vec<PaletteItem> {
    let mut items: Vec<PaletteItem> = SPECS
        .iter()
        .map(|spec| PaletteItem {
            name: SharedString::from(spec.name),
            click: SharedString::from(click_form(spec)),
            complete: SharedString::from(complete_form(spec)),
            section: spec.section,
            ctx: spec.ctx,
            owners: spec.owners,
            description: SharedString::from(rust_i18n::t!(spec.doc).into_owned()),
            example: example_of(spec, table),
        })
        .collect();

    for (name, doc) in KEY_OPTIONS {
        let owners: &'static [&'static str] = match name {
            "format" => &["date"],
            "item" => &["item", "super", "if", "elif", "for"],
            _ => VALUE_STATEMENTS,
        };
        items.push(PaletteItem {
            name: SharedString::from(name),
            click: SharedString::from(format!("{name}=")),
            complete: SharedString::from(format!("{name}=")),
            section: Section::Statement,
            ctx: Ctx::Option,
            owners,
            description: SharedString::from(rust_i18n::t!(doc).into_owned()),
            example: None,
        });
    }

    for (name, value) in custom {
        let name = name.trim();
        if name.is_empty() {
            continue;
        }
        items.push(PaletteItem {
            name: SharedString::from(name.to_owned()),
            click: SharedString::from(format!("${{{name}}}")),
            complete: SharedString::from(name.to_owned()),
            section: Section::Custom,
            ctx: Ctx::Field,
            owners: &[],
            description: ts!("palette.custom_var"),
            example: Some(SharedString::from(shorten(value))),
        });
    }

    items
}

/// What a click on the row writes at the caret.
fn click_form(spec: &Spec) -> String {
    match spec.ctx {
        Ctx::Field => format!("${{{}}}", spec.name),
        Ctx::Statement => spec.click.to_owned(),
        // A chain step is written onto whatever key is already under the
        // caret, so it carries its own dot.
        Ctx::Chain => format!(".{}", spec.name),
        Ctx::Option => format!("{}=", spec.name),
    }
}

/// What accepting the entry from the popup writes over the prefix.
fn complete_form(spec: &Spec) -> String {
    match spec.ctx {
        Ctx::Field => spec.name.to_owned(),
        // The popup is open because `${` was typed, so the brace is already
        // there and only the statement's head is missing. A statement that
        // takes a key is completed up to the `=` a value follows.
        Ctx::Statement => match spec.name {
            "item" | "super" => "item:key=".to_owned(),
            "if" | "elif" | "for" => format!("{}:item=", spec.name),
            other => other.to_owned(),
        },
        Ctx::Chain => spec.name.to_owned(),
        Ctx::Option => format!("{}=", spec.name),
    }
}

/// The value `spec` renders to for `table`, cut to one line.
///
/// Column, foreign-key and index fields are shown against the *first* element
/// of the relevant list, which is what a `${for}` would start with. A field
/// that renders to nothing gets no example rather than an empty one: a blank
/// after an arrow reads as a bug in the palette.
fn example_of(spec: &Spec, table: Option<&Table>) -> Option<SharedString> {
    let table = table?;
    let model: &dyn Model = match spec.section {
        Section::Table => table,
        Section::Column => table.columns.first()? as &dyn Model,
        Section::ForeignKey => table
            .imports
            .first()
            .or_else(|| table.exports.first())
            .map(|key| key as &dyn Model)?,
        Section::Index => table.indexes.first()? as &dyn Model,
        Section::Statement | Section::Decorator | Section::Condition | Section::Custom => {
            return None;
        }
    };
    let text = model.get(spec.name)?.to_text().into_owned();
    if text.is_empty() {
        return None;
    }
    Some(SharedString::from(shorten(&text)))
}

/// One line of `text`, at most [`EXAMPLE_LIMIT`] characters of it.
fn shorten(text: &str) -> String {
    let line: Cow<'_, str> = match text.find(['\n', '\r']) {
        Some(at) => Cow::Owned(text[..at].to_owned()),
        None => Cow::Borrowed(text),
    };
    let line = line.trim();
    if line.chars().count() <= EXAMPLE_LIMIT {
        return line.to_owned();
    }
    let cut: String = line.chars().take(EXAMPLE_LIMIT - 1).collect();
    format!("{cut}\u{2026}")
}

/// Where the caret is, as far as a completion is concerned.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CompletionKind {
    /// In the text between statements: a whole `${…}` is what gets written.
    Text,
    /// Just inside a `${`, before any `:` or `.`: a field or a statement name.
    Head,
    /// After the `=` of a `key=` or an `item=`: a member of the model, and
    /// pointedly not a statement — `${item:key=for}` is not a thing.
    Member,
    /// After a `.` in a key chain: a processor, and nothing else.
    Chain,
    /// After a `:` or a `,` inside a statement: that statement's options.
    Option {
        /// The statement the options belong to, lower case.
        statement: String,
    },
}

/// What the popup should offer, given what stands before the caret.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CompletionRequest {
    /// Which set of entries is in scope.
    pub kind: CompletionKind,
    /// The word already typed, which the list filters on and which accepting
    /// an entry replaces. Always trailing identifier characters, so the range
    /// to replace is `caret - prefix.len()..caret`.
    pub prefix: String,
}

/// Reads the caret's context out of the text before it on its line.
///
/// [`None`] where no completion belongs: inside a quoted value, and after the
/// `=` of an option whose value is anything but a key chain. A statement may
/// run over more than one line, so a `${` opened on an earlier line is not
/// seen here — deliberately: the alternative is scanning back through the
/// whole buffer on every keystroke to answer a question about the four
/// characters in front of the caret.
pub fn completion_at(line_before_caret: &str) -> Option<CompletionRequest> {
    let prefix = trailing_word(line_before_caret);
    let Some(open) = open_statement(line_before_caret) else {
        return Some(CompletionRequest {
            kind: CompletionKind::Text,
            prefix,
        });
    };

    let inside = &line_before_caret[open + 2..];
    if quotes_open(inside) {
        return None;
    }

    let Some(colon) = inside.find(':') else {
        return Some(CompletionRequest {
            kind: if inside.contains('.') {
                CompletionKind::Chain
            } else {
                CompletionKind::Head
            },
            prefix,
        });
    };

    let statement = inside[..colon].trim().to_ascii_lowercase();
    // The fragment being typed: whatever follows the last separator.
    let cut = inside.rfind(',').map_or(colon, |at| at.max(colon));
    let fragment = &inside[cut + 1..];
    match fragment.find('=') {
        // A value rather than an option name. Two of them are worth
        // completing: a key chain past its first dot, and the member a `key=`
        // or an `item=` names. Everything else — a separator, a regular
        // expression, a date format — is prose the palette knows nothing
        // about.
        Some(equals) => {
            let option = fragment[..equals].trim().to_ascii_lowercase();
            let value = &fragment[equals + 1..];
            let kind = if value.contains('.') {
                CompletionKind::Chain
            } else if option == "key" || option == "item" {
                CompletionKind::Member
            } else {
                return None;
            };
            Some(CompletionRequest { kind, prefix })
        }
        None => Some(CompletionRequest {
            kind: CompletionKind::Option { statement },
            prefix,
        }),
    }
}

/// The `${` the caret is inside of, if it is inside one.
fn open_statement(line: &str) -> Option<usize> {
    let open = line.rfind("${")?;
    // A `}` after it closed that statement again, and the caret is back in the
    // text between statements.
    if line[open..].contains('}') {
        return None;
    }
    Some(open)
}

/// Whether `text` leaves a quote open.
fn quotes_open(text: &str) -> bool {
    let mut open: Option<char> = None;
    for ch in text.chars() {
        match open {
            Some(quote) if ch == quote => open = None,
            Some(_) => {}
            None if ch == '\'' || ch == '"' => open = Some(ch),
            None => {}
        }
    }
    open.is_some()
}

/// The identifier characters at the end of `line`.
fn trailing_word(line: &str) -> String {
    let bytes = line.as_bytes();
    let mut from = bytes.len();
    while from > 0 && (bytes[from - 1].is_ascii_alphanumeric() || bytes[from - 1] == b'_') {
        from -= 1;
    }
    line[from..].to_owned()
}

/// The entries a request may take, best match first.
///
/// Ranked rather than merely filtered: an entry whose name *starts* with what
/// was typed comes before one that merely contains it, and a shorter name
/// before a longer one, so typing `na` offers `name` before `typeName` and
/// `nullable`. The comparison is case-insensitive because everything in the
/// language but `endif` and its three companions is.
pub fn matching<'a>(items: &'a [PaletteItem], request: &CompletionRequest) -> Vec<&'a PaletteItem> {
    let query = request.prefix.to_ascii_lowercase();
    let mut found: Vec<(u8, usize, &PaletteItem)> = items
        .iter()
        .filter(|item| in_scope(item, &request.kind))
        .filter_map(|item| {
            let name = item.name.to_ascii_lowercase();
            if query.is_empty() {
                // Nothing has been typed, so there is nothing to rank by: the
                // order stays the one the table is written in, which puts the
                // fields of the model first and `no` last. Ranking by length
                // here would open the list on `no`, `fk` and `if`.
                return Some((0, 0, item));
            }
            let at = name.find(&query)?;
            Some((if at == 0 { 0 } else { 1 }, name.len(), item))
        })
        .collect();
    // Stable, so an empty query keeps the table's own order.
    found.sort_by(|left, right| left.0.cmp(&right.0).then(left.1.cmp(&right.1)));
    // One row per thing that can be written. `name` is a field of the table,
    // of a column, of a foreign key and of an index — four entries in the
    // panel, where each is listed under the model it belongs to, but one
    // suggestion in the popup, because what typing it inserts is the same four
    // characters. The first survivor keeps its description, which is the
    // outermost model's: the popup cannot know which one the caret is inside.
    let mut seen: Vec<&str> = Vec::with_capacity(found.len());
    found.retain(|(_, _, item)| {
        let key = item.complete.as_ref();
        if seen.contains(&key) {
            return false;
        }
        seen.push(key);
        true
    });
    found.into_iter().map(|(_, _, item)| item).collect()
}

/// Whether `item` may be written where `kind` says the caret is.
fn in_scope(item: &PaletteItem, kind: &CompletionKind) -> bool {
    match kind {
        CompletionKind::Text | CompletionKind::Head => {
            matches!(item.ctx, Ctx::Field | Ctx::Statement)
        }
        CompletionKind::Member => item.ctx == Ctx::Field,
        CompletionKind::Chain => item.ctx == Ctx::Chain,
        CompletionKind::Option { statement } => {
            item.ctx == Ctx::Option
                && (item.owners.is_empty() || item.owners.contains(&statement.as_str()))
        }
    }
}

/// What accepting `item` writes over the prefix.
pub fn insertion(item: &PaletteItem, kind: &CompletionKind) -> SharedString {
    match kind {
        CompletionKind::Text => item.click.clone(),
        _ => item.complete.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(kind: CompletionKind, prefix: &str) -> CompletionRequest {
        CompletionRequest {
            kind,
            prefix: prefix.to_owned(),
        }
    }

    #[test]
    fn a_caret_in_plain_text_asks_for_the_whole_form() {
        assert_eq!(
            completion_at("    private String "),
            Some(request(CompletionKind::Text, ""))
        );
        // The word before the caret is the filter, brace or no brace.
        assert_eq!(
            completion_at("public class Nam"),
            Some(request(CompletionKind::Text, "Nam"))
        );
    }

    #[test]
    fn an_opened_brace_asks_for_a_field_or_a_statement() {
        assert_eq!(
            completion_at("class ${"),
            Some(request(CompletionKind::Head, ""))
        );
        assert_eq!(
            completion_at("class ${nam"),
            Some(request(CompletionKind::Head, "nam"))
        );
    }

    #[test]
    fn a_closed_statement_puts_the_caret_back_in_the_text() {
        // The brace before the caret is closed, so this is text again — and
        // the one thing that has to be got right, because it is what every
        // second line of a template looks like.
        assert_eq!(
            completion_at("private ${javaType} fie"),
            Some(request(CompletionKind::Text, "fie"))
        );
    }

    #[test]
    fn a_dot_asks_for_a_decorator_and_nothing_else() {
        assert_eq!(
            completion_at("${name.suffix.pas"),
            Some(request(CompletionKind::Chain, "pas"))
        );
        // Inside an option's value, too: `key=name.` is a chain like any other.
        assert_eq!(
            completion_at("${item:key=name.ca"),
            Some(request(CompletionKind::Chain, "ca"))
        );
    }

    #[test]
    fn a_colon_asks_for_the_statements_own_options() {
        assert_eq!(
            completion_at("${for:"),
            Some(request(
                CompletionKind::Option {
                    statement: "for".to_owned()
                },
                ""
            ))
        );
        assert_eq!(
            completion_at("${if:item=type, equ"),
            Some(request(
                CompletionKind::Option {
                    statement: "if".to_owned()
                },
                "equ"
            ))
        );
        // Statement names are case-insensitive, so the scope lookup is too.
        assert_eq!(
            completion_at("${FOR:item=columns, in"),
            Some(request(
                CompletionKind::Option {
                    statement: "for".to_owned()
                },
                "in"
            ))
        );
    }

    #[test]
    fn nothing_is_offered_inside_a_quoted_value_or_after_a_plain_equals() {
        assert_eq!(completion_at("${if:item=type, equals='TAB"), None);
        assert_eq!(completion_at("${for:item=columns, inStr=\",\n"), None);
        // A date format is prose, not a key.
        assert_eq!(completion_at("${date:format=yyyy-MM"), None);
    }

    #[test]
    fn the_value_of_a_key_option_is_a_member_of_the_model() {
        assert_eq!(
            completion_at("${item:key=nam"),
            Some(request(CompletionKind::Member, "nam"))
        );
        let items = items(None, &[]);
        let found = matching(&items, &request(CompletionKind::Member, ""));
        let names: Vec<&str> = found.iter().map(|item| item.name.as_ref()).collect();
        assert!(names.contains(&"remarks"), "{names:?}");
        assert!(!names.contains(&"for"), "a key is never a statement");
    }

    #[test]
    fn an_empty_query_keeps_the_order_the_palette_is_written_in() {
        // The first thing offered inside a bare `${` is the first field of the
        // table, not the shortest name in the language.
        let items = items(None, &[]);
        let found = matching(&items, &request(CompletionKind::Head, ""));
        let names: Vec<&str> = found
            .iter()
            .take(4)
            .map(|item| item.name.as_ref())
            .collect();
        assert_eq!(names, vec!["catalog", "schema", "name", "table"]);
    }

    #[test]
    fn the_scope_of_an_option_is_the_statement_it_belongs_to() {
        let items = items(None, &[]);
        let controls = matching(
            &items,
            &request(
                CompletionKind::Option {
                    statement: "for".to_owned(),
                },
                "",
            ),
        );
        let names: Vec<&str> = controls.iter().map(|item| item.name.as_ref()).collect();
        assert!(names.contains(&"inStr"), "{names:?}");
        assert!(names.contains(&"skipList"), "{names:?}");
        assert!(!names.contains(&"equals"), "a for takes no condition");
        assert!(!names.contains(&"padSize"), "a for pads nothing");

        let conditions = matching(
            &items,
            &request(
                CompletionKind::Option {
                    statement: "elif".to_owned(),
                },
                "",
            ),
        );
        let names: Vec<&str> = conditions.iter().map(|item| item.name.as_ref()).collect();
        assert!(names.contains(&"matches"), "{names:?}");
        assert!(!names.contains(&"inStr"), "an elif takes no separator");
    }

    #[test]
    fn a_prefix_match_outranks_a_substring_one() {
        let items = items(None, &[]);
        let found = matching(&items, &request(CompletionKind::Head, "na"));
        let names: Vec<&str> = found.iter().map(|item| item.name.as_ref()).collect();
        assert_eq!(names.first(), Some(&"name"), "{names:?}");
        // `typeName` merely contains the prefix, so it comes after everything
        // that starts with it.
        assert!(names.contains(&"typeName"), "{names:?}");
        assert!(
            names.iter().position(|name| *name == "notKeys")
                < names.iter().position(|name| *name == "typeName"),
            "a prefix match comes before a substring one: {names:?}"
        );
    }

    #[test]
    fn the_popup_offers_one_row_per_thing_that_can_be_written() {
        // `name` is a field of all four models; the panel lists it four times,
        // under four headings, and the popup once — typing it is the same four
        // characters whichever model the caret is inside.
        let items = items(None, &[]);
        assert_eq!(
            items
                .iter()
                .filter(|item| item.name == "name" && item.ctx == Ctx::Field)
                .count(),
            4
        );
        let found = matching(&items, &request(CompletionKind::Head, "name"));
        assert_eq!(
            found.iter().filter(|item| item.name == "name").count(),
            1,
            "{:?}",
            found.iter().map(|item| &item.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_decorator_is_only_offered_after_a_dot() {
        let items = items(None, &[]);
        let head = matching(&items, &request(CompletionKind::Head, "camel"));
        assert!(head.is_empty(), "a decorator is not a field");
        let chain = matching(&items, &request(CompletionKind::Chain, "cam"));
        assert_eq!(
            chain
                .iter()
                .map(|item| item.name.as_ref())
                .collect::<Vec<_>>(),
            vec!["camel"]
        );
    }

    #[test]
    fn a_field_is_written_whole_in_text_and_bare_inside_a_brace() {
        let items = items(None, &[]);
        let field = items
            .iter()
            .find(|item| item.name == "name" && item.section == Section::Table)
            .expect("the table has a name");
        assert_eq!(insertion(field, &CompletionKind::Text), "${name}");
        assert_eq!(insertion(field, &CompletionKind::Head), "name");

        let loop_ = items
            .iter()
            .find(|item| item.name == "for")
            .expect("there is a for");
        assert_eq!(
            insertion(loop_, &CompletionKind::Text),
            "${for:item=columns}\n${endfor}"
        );
        assert_eq!(insertion(loop_, &CompletionKind::Head), "for:item=");
    }

    #[test]
    fn a_custom_variable_joins_the_list_and_shows_its_value() {
        let items = items(
            None,
            &[("package".to_owned(), "com.example.model".to_owned())],
        );
        let custom: Vec<&PaletteItem> = items
            .iter()
            .filter(|item| item.section == Section::Custom)
            .collect();
        assert_eq!(custom.len(), 1);
        assert_eq!(custom[0].name, "package");
        assert_eq!(custom[0].click, "${package}");
        assert_eq!(custom[0].example.as_deref(), Some("com.example.model"));
    }

    #[test]
    fn a_blank_variable_row_is_not_an_entry() {
        // The variable table always carries one empty row (the Generate tab's
        // trailing-blank rule), and it is not something to offer.
        let items = items(None, &[("  ".to_owned(), String::new())]);
        assert!(!items.iter().any(|item| item.section == Section::Custom));
    }

    #[test]
    fn the_examples_come_from_the_table_the_preview_uses() {
        let table = rudbgen_meta::Table {
            name: "T_SAMPLE_ALBUM".to_owned(),
            kind: "TABLE".to_owned(),
            columns: vec![rudbgen_meta::Column {
                name: "ALBUM_ID".to_owned(),
                type_name: "INTEGER".to_owned(),
                ..rudbgen_meta::Column::default()
            }],
            ..rudbgen_meta::Table::default()
        };
        let items = items(Some(&table), &[]);
        let name = items
            .iter()
            .find(|item| item.name == "name" && item.section == Section::Table)
            .expect("the table has a name");
        assert_eq!(name.example.as_deref(), Some("T_SAMPLE_ALBUM"));

        let column = items
            .iter()
            .find(|item| item.name == "name" && item.section == Section::Column)
            .expect("a column has a name");
        assert_eq!(column.example.as_deref(), Some("ALBUM_ID"));

        // Nothing describes an index here, so that section has no examples
        // rather than empty ones.
        let index = items
            .iter()
            .find(|item| item.section == Section::Index)
            .expect("the index section is not empty");
        assert_eq!(index.example, None);
    }

    #[test]
    fn a_long_example_is_cut_to_one_line() {
        let long = "a".repeat(80);
        assert_eq!(shorten(&long).chars().count(), EXAMPLE_LIMIT);
        assert!(shorten(&long).ends_with('\u{2026}'));
        assert_eq!(shorten("first\nsecond"), "first");
    }

    #[test]
    fn every_entry_carries_a_description() {
        // A `doc` key nothing answers to comes back as the key itself, which
        // is what this catches: the palette would show `palette.table.name`.
        for item in items(None, &[]) {
            assert!(
                !item.description.contains("palette."),
                "{} has no description",
                item.name
            );
        }
    }

    #[test]
    fn every_key_the_model_answers_is_in_the_palette() {
        // The palette is documentation, and documentation that is missing an
        // entry is worse than none: the reader concludes the field does not
        // exist. Every key `rudbgen-meta` answers has to be listed.
        let table = rudbgen_meta::Table::default();
        let column = rudbgen_meta::Column::default();
        let items = items(None, &[]);
        let listed = |section: Section, key: &str| {
            items
                .iter()
                .any(|item| item.section == section && item.name == key)
        };
        for key in [
            "catalog", "schema", "name", "table", "title", "type", "remarks", "columns", "keys",
            "notKeys", "imports", "exports", "indexes", "icon", "no",
        ] {
            assert!(table.get(key).is_some(), "the model dropped {key}");
            assert!(listed(Section::Table, key), "the palette is missing {key}");
        }
        for key in [
            "catalog",
            "schema",
            "table",
            "name",
            "column",
            "typeName",
            "typeString",
            "isKey",
            "isCharType",
            "length",
            "precision",
            "scale",
            "nullable",
            "remarks",
            "defVal",
            "dataType",
            "jdbcType",
            "javaType",
            "autoIncrement",
            "keySeq",
            "fk",
            "no",
        ] {
            assert!(column.get(key).is_some(), "the model dropped {key}");
            assert!(listed(Section::Column, key), "the palette is missing {key}");
        }
    }
}
