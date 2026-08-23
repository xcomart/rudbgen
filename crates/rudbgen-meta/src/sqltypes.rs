//! `java.sql.Types` and the two names a template writes for it, plus the two
//! type strings jdbgen derives from a column's own type name.
//!
//! The tables are jdbgen's `types/db/SqlTypes.java`, **verbatim** — oddities
//! included. `TIMESTAMP` maps to `String` and `DECIMAL` to `Integer`, which is
//! wrong for any model a person would write by hand and is exactly why it stays:
//! the shipped templates and every template a user carries over from jdbgen
//! were written against these names, and changing one silently changes every
//! generated file. A saner second mapping is offered under a new field name
//! when its name is settled (architecture.md, open question 2); it is not here,
//! because a field nobody has agreed on is harder to remove than to add.

/// The `java.sql.Types` constant of a column, as the JDBC specification numbers
/// them.
///
/// Only the 36 codes jdbgen's table covers are listed. `REF_CURSOR`,
/// `TIME_WITH_TIMEZONE` and `TIMESTAMP_WITH_TIMEZONE` were added to
/// `java.sql.Types` after that table was written and are deliberately absent —
/// jdbgen answers nothing for them, so this crate answers nothing for them.
const TYPES: &[(i32, &str, &str)] = &[
    (2003, "ARRAY", "array"),
    (-5, "BIGINT", "Long"),
    (-2, "BINARY", "byte[]"),
    (-7, "BIT", "Boolean"),
    (2004, "BLOB", "byte[]"),
    (16, "BOOLEAN", "Boolean"),
    (1, "CHAR", "String"),
    (2005, "CLOB", "String"),
    (70, "DATALINK", "String"),
    (91, "DATE", "Date"),
    // Not BigDecimal: jdbgen's table says Integer, and the assets follow it.
    (3, "DECIMAL", "Integer"),
    (2001, "DISTINCT", "String"),
    (8, "DOUBLE", "Double"),
    (6, "FLOAT", "Float"),
    (4, "INTEGER", "Integer"),
    (2000, "JAVA_OBJECT", "String"),
    (-16, "LONGNVARCHAR", "String"),
    (-4, "LONGVARBINARY", "byte[]"),
    (-1, "LONGVARCHAR", "String"),
    (-15, "NCHAR", "String"),
    (2011, "NCLOB", "String"),
    (0, "NULL", "null"),
    (2, "NUMERIC", "Integer"),
    (-9, "NVARCHAR", "String"),
    (1111, "OTHER", "String"),
    (7, "REAL", "Float"),
    (2006, "REF", "ref"),
    (-8, "ROWID", "Integer"),
    (5, "SMALLINT", "Short"),
    (2009, "SQLXML", "String"),
    (2002, "STRUCT", "struct"),
    (92, "TIME", "Time"),
    // Not Timestamp, and not LocalDateTime: jdbgen's table says String.
    (93, "TIMESTAMP", "String"),
    (-6, "TINYINT", "Short"),
    (-3, "VARBINARY", "byte[]"),
    (12, "VARCHAR", "String"),
];

/// A length above which a character or binary type is written as `(max)`.
///
/// SQL Server reports `varchar(max)` as a column size of `2^31-1`, and a
/// generated DDL that says `VARCHAR(2147483647)` is not valid anywhere. The
/// switch is strictly *above* a million, which is jdbgen's `>` and is what
/// keeps a genuine `VARCHAR(1000000)` written as itself.
const MAX_LENGTH: i64 = 1_000_000;

/// The JDBC name of a type code — `VARCHAR` for 12 — or `""` for a code no
/// version of the table knows.
///
/// jdbgen answers `null` here and renders it as the empty string; a template
/// cannot tell the two apart, so the empty string is what this returns.
pub fn jdbc_type(data_type: i32) -> &'static str {
    lookup(data_type).map_or("", |(_, jdbc, _)| jdbc)
}

/// The Java type a generated model uses for a type code, or `""` for an
/// unknown code. See the module documentation before being surprised by one.
pub fn java_type(data_type: i32) -> &'static str {
    lookup(data_type).map_or("", |(_, _, java)| java)
}

/// The row of the table for a type code.
fn lookup(data_type: i32) -> Option<&'static (i32, &'static str, &'static str)> {
    // A linear scan over 36 entries, once per column: a map would cost more to
    // build than every lookup a generation run makes.
    TYPES.iter().find(|(code, _, _)| *code == data_type)
}

/// Whether a database type name names a character type.
///
/// jdbgen's rule, and it is a substring test rather than a list: `CHAR`,
/// `CLOB` or `TEXT` anywhere in the upper-cased name. That is what makes
/// `NVARCHAR2`, `LONG VARCHAR` and MySQL's `MEDIUMTEXT` all answer true
/// without a per-product table.
pub fn is_char_type(type_name: &str) -> bool {
    let upper = type_name.to_uppercase();
    upper.contains("CHAR") || upper.contains("CLOB") || upper.contains("TEXT")
}

/// The type as DDL writes it: the upper-cased type name, with a length on the
/// character and binary types.
///
/// jdbgen's `DBColumn` derivation, kept exactly: the length is appended when
/// the name contains `CHAR` **or** `BINARY` — so `CLOB` and `TEXT` get none
/// even though they are character types — and a length above
/// [`MAX_LENGTH`] is written as `(max)` instead of the number.
pub fn type_string(type_name: &str, length: i64) -> String {
    let mut upper = type_name.to_uppercase();
    if upper.contains("CHAR") || upper.contains("BINARY") {
        if length > MAX_LENGTH {
            upper.push_str("(max)");
        } else {
            upper.push('(');
            upper.push_str(&length.to_string());
            upper.push(')');
        }
    }
    upper
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_type_code_is_translated_into_its_name_and_its_java_type() {
        // jdbgen's SqlTypesTest, case for case.
        for (code, jdbc, java) in [
            (1, "CHAR", "String"),
            (12, "VARCHAR", "String"),
            (-9, "NVARCHAR", "String"),
            (-1, "LONGVARCHAR", "String"),
            (2005, "CLOB", "String"),
            (2011, "NCLOB", "String"),
            (4, "INTEGER", "Integer"),
            (5, "SMALLINT", "Short"),
            (-6, "TINYINT", "Short"),
            (-5, "BIGINT", "Long"),
            (2, "NUMERIC", "Integer"),
            (3, "DECIMAL", "Integer"),
            (6, "FLOAT", "Float"),
            (7, "REAL", "Float"),
            (8, "DOUBLE", "Double"),
            (16, "BOOLEAN", "Boolean"),
            (-7, "BIT", "Boolean"),
            (91, "DATE", "Date"),
            (92, "TIME", "Time"),
            (93, "TIMESTAMP", "String"),
            (-2, "BINARY", "byte[]"),
            (-3, "VARBINARY", "byte[]"),
            (-4, "LONGVARBINARY", "byte[]"),
            (2004, "BLOB", "byte[]"),
            (2009, "SQLXML", "String"),
            (1111, "OTHER", "String"),
        ] {
            assert_eq!(jdbc_type(code), jdbc, "jdbc type of {code}");
            assert_eq!(java_type(code), java, "java type of {code}");
        }
    }

    #[test]
    fn a_code_that_is_no_sql_type_is_answered_with_nothing() {
        for code in [9999, -100_000, i32::MAX, i32::MIN, 2012, 2013, 2014] {
            assert_eq!(jdbc_type(code), "", "jdbc type of {code}");
            assert_eq!(java_type(code), "", "java type of {code}");
        }
    }

    #[test]
    fn the_table_holds_every_code_once_and_both_names_for_each() {
        assert_eq!(TYPES.len(), 36, "jdbgen's table has 36 entries");
        for (index, (code, jdbc, java)) in TYPES.iter().enumerate() {
            assert!(
                !jdbc.is_empty() && !java.is_empty(),
                "{code} is half mapped"
            );
            assert!(
                !TYPES[..index].iter().any(|(other, _, _)| other == code),
                "{code} appears twice"
            );
        }
    }

    #[test]
    fn the_placeholder_java_types_are_kept_as_they_are() {
        // Not types a model can be declared with, and jdbgen writes them
        // lower-cased. Templates that switch on them depend on the spelling.
        assert_eq!(java_type(2003), "array");
        assert_eq!(java_type(2002), "struct");
        assert_eq!(java_type(2006), "ref");
        assert_eq!(java_type(0), "null");
    }

    #[test]
    fn a_character_type_carries_its_length() {
        assert_eq!(type_string("varchar", 40), "VARCHAR(40)");
        assert_eq!(type_string("char", 2), "CHAR(2)");
        assert_eq!(type_string("varbinary", 16), "VARBINARY(16)");
    }

    #[test]
    fn a_type_without_a_length_is_written_as_it_is() {
        assert_eq!(type_string("integer", 10), "INTEGER");
        assert_eq!(type_string("timestamp", 26), "TIMESTAMP");
        // A CLOB is a character type and still gets no length: the rule that
        // appends one asks for CHAR or BINARY, not for isCharType.
        assert_eq!(type_string("clob", 2_147_483_647), "CLOB");
    }

    #[test]
    fn an_unreasonable_length_is_written_as_max() {
        assert_eq!(type_string("varchar", 2_147_483_647), "VARCHAR(max)");
        assert_eq!(
            type_string("varchar", 1_000_000),
            "VARCHAR(1000000)",
            "the switch is above a million, not at it"
        );
    }

    #[test]
    fn the_type_is_recognised_whatever_case_the_driver_reports_it_in() {
        assert!(is_char_type("nVarChar"));
        assert_eq!(type_string("nVarChar", 10), "NVARCHAR(10)");
    }

    #[test]
    fn every_text_type_is_recognised_as_a_character_type() {
        for name in [
            "CHAR",
            "VARCHAR",
            "NVARCHAR",
            "LONGVARCHAR",
            "CLOB",
            "NCLOB",
            "TEXT",
        ] {
            assert!(is_char_type(name), "{name}");
        }
    }

    #[test]
    fn every_other_type_is_not_a_character_type() {
        for name in ["INTEGER", "NUMBER", "BLOB", "DATE", "VARBINARY"] {
            assert!(!is_char_type(name), "{name}");
        }
    }

    #[test]
    fn a_column_without_a_type_name_derives_nothing_from_it() {
        // A custom column query may select no TYPE_NAME at all; jdbgen reads
        // that as the empty type name rather than as a failure.
        assert_eq!(type_string("", 10), "");
        assert!(!is_char_type(""));
    }
}
