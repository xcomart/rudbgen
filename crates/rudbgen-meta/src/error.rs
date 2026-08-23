//! What reading metadata can fail with.
//!
//! Three of the four variants are about a *custom* query (architecture.md D9),
//! and that is the point: a `DESCRIBE` either answers or fails with a bridge
//! error, while a user-written statement can succeed and still be unusable —
//! the wrong labels, too few columns, no result set at all. jdbgen's failure
//! mode for exactly this was a whole run silently producing nothing, which is
//! what the driver editor's **Test** button and these variants exist to
//! replace: each one names the query, and what was wrong with it.

use rudbgen_core::CustomQueryKind;

/// The result of every fallible call in this crate.
pub type Result<T> = std::result::Result<T, Error>;

/// A metadata read that could not be completed.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The bridge, the driver or the connection failed.
    #[error(transparent)]
    Jdbc(#[from] rudbgen_jdbc::Error),

    /// A custom query read by label does not return one of the labels its
    /// contract requires.
    ///
    /// The label is named, because "the table list is empty" is what this looks
    /// like from the outside and it is not a diagnosis. Only the *presence* of
    /// the label is checked: a `TABLE_TYPE` that is there and null is a table
    /// list a custom query is allowed to write (jdbgen reads it as `TABLE`).
    #[error("the {} query does not return a column labelled {label}", kind_label(*kind))]
    MissingLabel {
        /// Which of the four queries.
        kind: CustomQueryKind,
        /// The label the contract requires and the result set does not carry.
        label: String,
    },

    /// A custom query read positionally returns fewer columns than the contract
    /// asks for.
    #[error(
        "the {} query returns {found} column(s); it is read positionally and needs {expected}",
        kind_label(*kind)
    )]
    Shape {
        /// Which of the four queries.
        kind: CustomQueryKind,
        /// How many columns the contract reads.
        expected: usize,
        /// How many the statement answered with.
        found: usize,
    },

    /// A custom query produced an update count rather than a result set.
    ///
    /// Usually a statement that is not a `SELECT` at all — and one that has
    /// already run against the user's database by the time this is raised,
    /// which is worth saying plainly rather than reporting as "no rows".
    #[error("the {} query returned no result set", kind_label(*kind))]
    NoResultSet {
        /// Which of the four queries.
        kind: CustomQueryKind,
    },
}

/// How a query is named in a message: what it answers, not its enum variant.
pub(crate) fn kind_label(kind: CustomQueryKind) -> &'static str {
    match kind {
        CustomQueryKind::Tables => "table-list",
        CustomQueryKind::Columns => "column-list",
        CustomQueryKind::TableComments => "table-comment",
        CustomQueryKind::ColumnComments => "column-comment",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_label_says_which_one_it_is() {
        let error = Error::MissingLabel {
            kind: CustomQueryKind::Tables,
            label: "TABLE_TYPE".to_string(),
        };
        let message = error.to_string();
        assert!(message.contains("TABLE_TYPE"), "{message}");
        assert!(message.contains("table-list"), "{message}");
    }

    #[test]
    fn a_positional_query_reports_both_counts() {
        let message = Error::Shape {
            kind: CustomQueryKind::ColumnComments,
            expected: 2,
            found: 1,
        }
        .to_string();
        assert!(message.contains('1') && message.contains('2'), "{message}");
        assert!(message.contains("column-comment"), "{message}");
    }
}
