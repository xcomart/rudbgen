//! Running the four custom queries (architecture.md D9) and reading what they
//! answer.
//!
//! All of it is jdbgen's contract, on the Rust side of the bridge, so that a
//! driver definition imported from jdbgen works unedited: the same
//! placeholders, the same labels, the same tolerance for a label that is there
//! and null — and the same reading of `IS_KEY`, down to which strings count as
//! a number.
//!
//! What the bridge never sees is the statement itself. It runs through
//! `EXECUTE` like any other SQL, which is what lets the same code path serve
//! the driver editor's **Test** button: substitute, run, check the labels,
//! report.

use std::collections::HashMap;

use rudbgen_core::CustomQueryKind;
use rudbgen_jdbc::{Session, StatementSpec};

use crate::error::{Error, Result};

/// The characters jdbgen's `StrUtils.toInt` accepts around the digits.
const NUM_SIGNS: [char; 2] = ['+', '-'];

/// Fill the `${...}` holes of a custom query in.
///
/// jdbgen substitutes by reflection over the schema or the table it is reading,
/// which means every member of those objects is a possible hole; rudbgen
/// substitutes the three the contract documents — `${catalog}`, `${schema}`
/// and, for the per-table statements, `${table}` — and leaves everything else
/// alone.
///
/// A hole with no value is **left in the statement verbatim**, which is
/// jdbgen's behaviour and looks like a mistake until you see what the
/// alternative does: a `where TABLE_SCHEMA = '${schema}'` against a product
/// with no schemas would silently become `= ''` and answer nothing, where the
/// unsubstituted form fails loudly with the placeholder in the error message.
/// A warning is logged for each one, because it is the first thing to look at
/// when a custom query answers an empty list.
///
/// Values are matched case sensitively, as jdbgen's property lookup is.
pub fn substitute(sql: &str, values: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut rest = sql;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let Some(end) = after.find('}') else {
            // No closing brace: there is no placeholder here, only text that
            // begins like one. jdbgen fails the whole read; keeping the tail
            // is the lesser answer, and the statement will say so itself.
            out.push_str(&rest[start..]);
            return out;
        };
        let key = &after[..end];
        match values
            .iter()
            .find(|(name, value)| *name == key && !value.is_empty())
        {
            Some((_, value)) => out.push_str(value),
            None => {
                log::warn!(
                    "custom query placeholder ${{{key}}} has no value here; it is left in the statement"
                );
                out.push_str("${");
                out.push_str(key);
                out.push('}');
            }
        }
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    out
}

/// Read a number the way jdbgen's `StrUtils.toInt` does, which is what the
/// `IS_KEY` column of a custom column list is read with.
///
/// Thousands separators are dropped, a fractional part is truncated, and
/// **anything else at all makes the answer 0** — including the trailing space
/// a fixed-width `CHAR(1)` comes back with, and including `Y`. That last one
/// is worth knowing before writing a column query: `IS_KEY` is a *number*, and
/// a query answering `'Y'`/`'N'` marks no key at all. It is reproduced rather
/// than improved because a definition carried over from jdbgen was written
/// against it.
pub fn to_int(text: &str) -> i32 {
    let mut digits = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            ',' => {}
            '.' => digits.push(c),
            c if c.is_ascii_digit() || NUM_SIGNS.contains(&c) => digits.push(c),
            // jdbgen throws here and catches the throw as a zero.
            _ => return 0,
        }
    }
    let whole = match digits.find('.') {
        Some(dot) => &digits[..dot],
        None => &digits[..],
    };
    whole.parse::<i32>().unwrap_or(0)
}

/// The result of a custom query, as text.
///
/// Everything is a string because everything in these four result sets is a
/// name, a comment, a type or a number small enough to be written as one, and
/// because the alternative — a typed cell per driver per column — is a second
/// decoder to keep in step with the bridge's.
pub(crate) struct Rows {
    /// Which of the four queries produced this, for the error messages.
    kind: CustomQueryKind,
    /// Column position by upper-cased label.
    labels: HashMap<String, usize>,
    /// How many columns the result set has.
    width: usize,
    /// The rows, `None` for a SQL NULL.
    rows: Vec<Vec<Option<String>>>,
}

impl Rows {
    /// Run one custom query and read all of it.
    ///
    /// Metadata result sets are small — the tables of a schema, the columns of
    /// a table — so they are read whole rather than streamed: there is no
    /// cursor to keep open while a generation run walks the model.
    pub(crate) fn run(session: &Session, kind: CustomQueryKind, sql: &str) -> Result<Rows> {
        log::debug!(
            "running the {} query: {sql}",
            crate::error::kind_label(kind)
        );
        let cursor = session.execute(&StatementSpec::new(sql))?;
        if !cursor.result().has_result_set {
            return Err(Error::NoResultSet { kind });
        }

        let columns = cursor.columns();
        let width = columns.len();
        let mut labels = HashMap::with_capacity(width);
        for (index, column) in columns.iter().enumerate() {
            // First one wins: `findColumn` answers the lowest index for a
            // label a statement selected twice, and so does this.
            labels
                .entry(column.display_name().to_uppercase())
                .or_insert(index);
        }

        let mut rows = Vec::new();
        loop {
            let batch = cursor.fetch(0)?;
            for row in 0..batch.rows() {
                let mut cells = Vec::with_capacity(width);
                for (index, column) in columns.iter().enumerate() {
                    cells.push(
                        batch
                            .value(row, index)
                            .and_then(|value| value.to_text(column)),
                    );
                }
                rows.push(cells);
            }
            // Not `rows() < limit`: a batch that fills its limit exactly is
            // still not the last one.
            if batch.is_last() {
                break;
            }
        }
        Ok(Rows {
            kind,
            labels,
            width,
            rows,
        })
    }

    /// Check that every label the contract requires is there.
    ///
    /// Presence only: a `TABLE_TYPE` that is there and null is a table list a
    /// custom query is allowed to write. The labels are checked in the order
    /// the contract lists them, so the same broken statement always names the
    /// same label.
    pub(crate) fn require_labels(&self) -> Result<()> {
        for label in self.kind.required_labels() {
            if !self.labels.contains_key(*label) {
                return Err(Error::MissingLabel {
                    kind: self.kind,
                    label: (*label).to_string(),
                });
            }
        }
        Ok(())
    }

    /// Check that a positionally read result has the columns it is read by.
    pub(crate) fn require_width(&self, expected: usize) -> Result<()> {
        if self.width < expected {
            return Err(Error::Shape {
                kind: self.kind,
                expected,
                found: self.width,
            });
        }
        Ok(())
    }

    /// How many rows came back.
    pub(crate) fn len(&self) -> usize {
        self.rows.len()
    }

    /// The value of a labelled column, `""` for a NULL and for a label the
    /// result set does not carry.
    ///
    /// A label that is not there can only happen where the contract does not
    /// require it, because [`Rows::require_labels`] has already run.
    pub(crate) fn label(&self, row: usize, label: &str) -> &str {
        match self.labels.get(label) {
            Some(index) => self.at(row, *index),
            None => "",
        }
    }

    /// Whether a labelled column carried a SQL NULL.
    ///
    /// The one place a null and an empty string differ: jdbgen reads a null
    /// `TABLE_TYPE` as `TABLE`, and a driver that says nothing about a type is
    /// not the same as one that says the type is the empty string — even
    /// though [`crate::model::table_kind`] answers `TABLE` for both.
    pub(crate) fn is_null(&self, row: usize, label: &str) -> bool {
        match self.labels.get(label) {
            Some(index) => self.rows[row][*index].is_none(),
            None => true,
        }
    }

    /// The value of a column by position, `""` for a NULL.
    pub(crate) fn at(&self, row: usize, index: usize) -> &str {
        self.rows[row]
            .get(index)
            .and_then(Option::as_deref)
            .unwrap_or("")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_documented_placeholders_are_filled_in() {
        let sql = "select * from t where c = '${catalog}' and s = '${schema}' and n = '${table}'";
        assert_eq!(
            substitute(
                sql,
                &[("catalog", "DB"), ("schema", "PUBLIC"), ("table", "T")]
            ),
            "select * from t where c = 'DB' and s = 'PUBLIC' and n = 'T'"
        );
    }

    #[test]
    fn a_placeholder_with_no_value_is_left_where_it_is() {
        // jdbgen's rule: a hole it cannot fill stays a hole, so the statement
        // fails with the placeholder in the message instead of quietly
        // matching nothing.
        let sql = "select * from t where c = '${catalog}' and s = '${schema}'";
        assert_eq!(
            substitute(sql, &[("catalog", ""), ("schema", "PUBLIC")]),
            "select * from t where c = '${catalog}' and s = 'PUBLIC'"
        );
    }

    #[test]
    fn a_hole_this_contract_does_not_know_is_left_alone() {
        // `${table}` is not offered to a schema-level statement, and a name
        // nobody offers is somebody else's syntax, not a typo to guess at.
        assert_eq!(
            substitute("a ${table} b ${owner} c", &[("catalog", "DB")]),
            "a ${table} b ${owner} c"
        );
    }

    #[test]
    fn text_that_only_begins_like_a_placeholder_survives() {
        assert_eq!(substitute("select '${' from t", &[]), "select '${' from t");
        assert_eq!(substitute("", &[]), "");
        assert_eq!(substitute("no holes here", &[]), "no holes here");
    }

    #[test]
    fn the_same_hole_can_appear_twice() {
        assert_eq!(
            substitute(
                "${schema}.${table} ${schema}",
                &[("schema", "S"), ("table", "T")]
            ),
            "S.T S"
        );
    }

    #[test]
    fn is_key_is_read_as_a_number_the_way_jdbgen_reads_it() {
        assert_eq!(to_int("1"), 1);
        assert_eq!(to_int("0"), 0);
        assert_eq!(to_int("-1"), -1);
        assert_eq!(to_int("+5"), 5);
        assert_eq!(to_int("1.9"), 1, "the fractional part is truncated");
        assert_eq!(to_int("1,000"), 1000, "thousands separators are dropped");
    }

    #[test]
    fn anything_that_is_not_a_number_is_not_a_key() {
        // The trap worth knowing about: a column query answering 'Y' marks no
        // key at all, and neither does one whose CHAR(1) comes back padded.
        for text in ["Y", "N", "true", "", "1 ", " 1", "1abc", "+", "-", "."] {
            assert_eq!(to_int(text), 0, "{text:?}");
        }
    }

    #[test]
    fn a_number_too_large_for_the_java_int_is_no_key_either() {
        assert_eq!(to_int("99999999999999"), 0);
    }
}
