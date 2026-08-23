//! The table model: what a template is rendered against.
//!
//! Two rules shape every type here.
//!
//! **jdbgen's field names are kept.** A template written against jdbgen has to
//! render against this model unedited, so the member names a template can say —
//! `typeName`, `notKeys`, `isKey`, `defVal`, `nvlColName` — are jdbgen's
//! spelling and not Rust's. That is why [`Model::get`] answers `type` for a
//! field called `kind` and `notKeys` for a method called `not_keys`: the Rust
//! side reads like Rust, the template side reads like jdbgen, and the
//! translation happens once, here.
//!
//! **The new fields are additions, never replacements** (architecture.md D8).
//! `imports`, `exports`, `indexes`, `precision`, `scale`, `autoIncrement` and
//! `fk` are what a relational generator needs and jdbgen never had; not one
//! jdbgen field changed meaning to make room for them.
//!
//! Two members are answered and always empty, because jdbgen answers them and
//! always empty: `source` on a table and `nvlColName` on a column are fields a
//! *template* fills in, never the metadata reader. A template that names one
//! must see an empty value rather than an unknown-field warning.

use std::borrow::Cow;

use rudbgen_template::{Model, Value};

use crate::sqltypes;

/// What [`Table::kind`] holds for a table.
pub const KIND_TABLE: &str = "TABLE";

/// What [`Table::kind`] holds for a view.
pub const KIND_VIEW: &str = "VIEW";

/// The icon locator of a table, in jdbgen's `fa:<glyph>` notation.
const ICON_TABLE: &str = "fa:TABLE";

/// The icon locator of a view.
const ICON_VIEW: &str = "fa:EYE";

/// Normalise whatever a driver calls a table type into `TABLE` or `VIEW`.
///
/// jdbgen's `DBTable` rule, kept whole because the explorer and the generator
/// both filter on the result:
///
/// * no type at all — which a custom table list is allowed to omit — is a
///   table, because a query written to list tables lists tables;
/// * `TABLE` and `VIEW` pass through;
/// * anything holding the word `TABLE` (`SYSTEM TABLE`, `BASE TABLE`,
///   `GLOBAL TEMPORARY TABLE`) is a table, and anything holding `VIEW`
///   (`MATERIALIZED VIEW`, `SYSTEM VIEW`) is a view;
/// * anything else is kept verbatim, and is then dropped by the filter in
///   [`MetaReader::tables`](crate::MetaReader::tables). A `SEQUENCE` is not a
///   thing this generator writes a file for, and guessing which of the two it
///   resembles would be worse than not showing it.
///
/// The word test is case sensitive, as jdbgen's is: every driver in the
/// specification's own list reports these upper-cased.
pub fn table_kind(reported: Option<&str>) -> String {
    let Some(reported) = reported.filter(|type_name| !type_name.is_empty()) else {
        return KIND_TABLE.to_string();
    };
    if reported == KIND_TABLE || reported == KIND_VIEW {
        return reported.to_string();
    }
    if reported.contains(KIND_TABLE) {
        KIND_TABLE.to_string()
    } else if reported.contains(KIND_VIEW) {
        KIND_VIEW.to_string()
    } else {
        reported.to_string()
    }
}

/// One schema of the connected database, or the placeholder standing in for a
/// database that has none.
///
/// jdbgen's `DBSchema`, and its rule that the list is never empty: see
/// [`MetaReader::schemas`](crate::MetaReader::schemas) for the three
/// placeholders that guarantee it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Schema {
    /// The catalog this schema belongs to, empty when the product has none.
    pub catalog: String,
    /// The schema name as the driver reports it, empty for a placeholder.
    ///
    /// Empty means "do not filter by schema": it is passed to `DESCRIBE` as no
    /// schema at all rather than as the empty string.
    pub schema: String,
    /// What the explorer shows. The schema name, or the catalog name for a
    /// catalog that has no schemas, or `Default Schema`.
    pub name: String,
}

impl Model for Schema {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "catalog" => Value::Str(Cow::Borrowed(&self.catalog)),
            "schema" => Value::Str(Cow::Borrowed(&self.schema)),
            "name" => Value::Str(Cow::Borrowed(&self.name)),
            _ => return None,
        })
    }
}

/// A table as the table list knows it: enough for a row of the explorer, and
/// nothing that costs a second round trip.
///
/// [`MetaReader::table`](crate::MetaReader::table) turns one of these into a
/// whole [`Table`]. They are separate types on purpose — a `Table` whose
/// `columns` are empty because they were never read is indistinguishable from
/// one whose table really has none, and that ambiguity is what a cache built on
/// top of this would get wrong.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct TableRef {
    /// Catalog, empty when the product has none.
    pub catalog: String,
    /// Schema, empty when the product has none.
    pub schema: String,
    /// The table name as the database reports it.
    pub name: String,
    /// [`KIND_TABLE`] or [`KIND_VIEW`]; see [`table_kind`].
    pub kind: String,
    /// The table comment, empty when there is none.
    pub remarks: String,
    /// One-based position in the list this came back in.
    pub no: usize,
}

impl TableRef {
    /// Whether this is a view rather than a table.
    pub fn is_view(&self) -> bool {
        self.kind == KIND_VIEW
    }

    /// The icon locator, in jdbgen's `fa:` notation.
    pub fn icon(&self) -> &'static str {
        if self.is_view() {
            ICON_VIEW
        } else {
            ICON_TABLE
        }
    }
}

impl Model for TableRef {
    /// The members of [`Table`] that do not need the columns.
    ///
    /// It answers as much as it can rather than nothing, because the output
    /// *file name* of a generation run is a template too and is rendered per
    /// table — usually against nothing but `${name}`.
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "catalog" => Value::Str(Cow::Borrowed(&self.catalog)),
            "schema" => Value::Str(Cow::Borrowed(&self.schema)),
            "name" | "table" | "title" => Value::Str(Cow::Borrowed(&self.name)),
            "type" => Value::Str(Cow::Borrowed(&self.kind)),
            "remarks" => Value::Str(Cow::Borrowed(&self.remarks)),
            "icon" => Value::Str(Cow::Borrowed(self.icon())),
            "no" => Value::Int(self.no as i64),
            _ => return None,
        })
    }
}

/// One table or view, with everything a template can ask about it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Table {
    /// Catalog, empty when the product has none.
    pub catalog: String,
    /// Schema, empty when the product has none.
    pub schema: String,
    /// The table name as the database reports it.
    ///
    /// jdbgen keeps a display `name` beside the database `table`, and lets the
    /// user rename the first. Nothing in rudbgen renames a table — the
    /// abbreviation rules rewrite the *rendered* name inside the engine — so
    /// there is one name here and `name`, `table` and `title` all answer it.
    pub name: String,
    /// [`KIND_TABLE`] or [`KIND_VIEW`]; see [`table_kind`].
    pub kind: String,
    /// The table comment, empty when there is none.
    pub remarks: String,
    /// Every column, in the order the database reports them.
    pub columns: Vec<Column>,
    /// The foreign keys this table declares, by name (D8).
    pub imports: Vec<ForeignKey>,
    /// The foreign keys other tables declare *on* this one, by name (D8).
    pub exports: Vec<ForeignKey>,
    /// The indexes of this table, by name (D8).
    pub indexes: Vec<Index>,
    /// One-based position in the list this came from.
    pub no: usize,
}

impl Table {
    /// Whether this is a view rather than a table.
    pub fn is_view(&self) -> bool {
        self.kind == KIND_VIEW
    }

    /// The icon locator, in jdbgen's `fa:` notation.
    pub fn icon(&self) -> &'static str {
        if self.is_view() {
            ICON_VIEW
        } else {
            ICON_TABLE
        }
    }

    /// The primary key columns, in the order the key was declared in.
    ///
    /// Declaration order, not column order and not alphabetical: a composite
    /// key written `primary key (B, A)` generates a `where B = ? and A = ?`
    /// that has to match it. `getPrimaryKeys` answers by column name, which is
    /// why [`Column::key_seq`] is carried at all.
    pub fn keys(&self) -> Vec<&Column> {
        let mut keys: Vec<&Column> = self
            .columns
            .iter()
            .filter(|column| column.key_seq.is_some())
            .collect();
        keys.sort_by_key(|column| column.key_seq.unwrap_or_default());
        keys
    }

    /// Every column outside the primary key, in column order.
    pub fn not_keys(&self) -> Vec<&Column> {
        self.columns
            .iter()
            .filter(|column| column.key_seq.is_none())
            .collect()
    }

    /// The column of this table with that name, if it has one.
    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|column| column.name == name)
    }

    /// This table as the explorer knows it.
    pub fn as_ref(&self) -> TableRef {
        TableRef {
            catalog: self.catalog.clone(),
            schema: self.schema.clone(),
            name: self.name.clone(),
            kind: self.kind.clone(),
            remarks: self.remarks.clone(),
            no: self.no,
        }
    }
}

/// Collect a slice of models into a list value.
fn list<'a, T: Model>(items: &'a [T]) -> Value<'a> {
    Value::List(items.iter().map(|item| item as &dyn Model).collect())
}

/// Collect already-borrowed models into a list value.
fn list_of<'a>(items: Vec<&'a Column>) -> Value<'a> {
    Value::List(items.into_iter().map(|item| item as &dyn Model).collect())
}

impl Model for Table {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "catalog" => Value::Str(Cow::Borrowed(&self.catalog)),
            "schema" => Value::Str(Cow::Borrowed(&self.schema)),
            // jdbgen's three names for the one name this model has.
            "name" | "table" | "title" => Value::Str(Cow::Borrowed(&self.name)),
            "type" => Value::Str(Cow::Borrowed(&self.kind)),
            "remarks" => Value::Str(Cow::Borrowed(&self.remarks)),
            "columns" => list(&self.columns),
            "keys" => list_of(self.keys()),
            "notKeys" => list_of(self.not_keys()),
            "imports" => list(&self.imports),
            "exports" => list(&self.exports),
            "indexes" => list(&self.indexes),
            "icon" => Value::Str(Cow::Borrowed(self.icon())),
            "no" => Value::Int(self.no as i64),
            // A jdbgen field only a template ever writes into. Answered so
            // that naming it is not an unknown-field warning.
            "source" => Value::Str(Cow::Borrowed("")),
            _ => return None,
        })
    }
}

/// One column of a table.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Column {
    /// Catalog of the owning table.
    pub catalog: String,
    /// Schema of the owning table.
    pub schema: String,
    /// Name of the owning table.
    pub table: String,
    /// The column name. Answered as both `name` and `column`.
    pub name: String,
    /// The database's own type name, e.g. `NVARCHAR2`. Empty when the driver —
    /// or a custom column query — reports none.
    pub type_name: String,
    /// The type as DDL writes it; see [`sqltypes::type_string`].
    pub type_string: String,
    /// Whether [`Column::type_name`] names a character type.
    pub is_char_type: bool,
    /// `COLUMN_SIZE`: characters for a character type, digits for a numeric one.
    pub length: i64,
    /// `COLUMN_SIZE` again, under the name a numeric column wants (D8).
    ///
    /// The same number as [`Column::length`], because JDBC has one column for
    /// both. It is not a second reading of the metadata; it is the name a
    /// template writing `DECIMAL(${precision}, ${scale})` needs.
    pub precision: i64,
    /// `DECIMAL_DIGITS`: the scale of an exact numeric type, 0 otherwise (D8).
    pub scale: i64,
    /// `NULLABLE`, as JDBC numbers it: 0 not nullable, 1 nullable, 2 unknown.
    ///
    /// Kept as the driver's number rather than a `bool`, because that is what
    /// jdbgen's templates compare against — `${if:key=nullable,value=0}`.
    pub nullable: i64,
    /// The column comment, empty when there is none.
    pub remarks: String,
    /// `COLUMN_DEF`, the default value as SQL text; empty when there is none.
    pub def_val: String,
    /// The `java.sql.Types` constant.
    pub data_type: i32,
    /// The JDBC name of [`Column::data_type`]; see [`sqltypes::jdbc_type`].
    pub jdbc_type: String,
    /// The Java type of [`Column::data_type`]; see [`sqltypes::java_type`].
    pub java_type: String,
    /// Whether the database fills this column in by itself (D8).
    pub auto_increment: bool,
    /// Position inside the primary key, one-based; `None` when the column is
    /// not part of it.
    ///
    /// One field rather than an `is_key` flag beside a sequence number: two
    /// fields can disagree, and the order is what a composite key needs.
    pub key_seq: Option<u32>,
    /// What this column points at, when exactly one foreign key uses it (D8).
    pub fk: Option<ForeignKeyRef>,
    /// One-based position in the table.
    pub no: usize,
}

impl Column {
    /// Whether this column takes part in the primary key.
    pub fn is_key(&self) -> bool {
        self.key_seq.is_some()
    }

    /// Fill in the four derived members from the three raw ones.
    ///
    /// [`Column::type_string`], [`Column::is_char_type`],
    /// [`Column::jdbc_type`] and [`Column::java_type`] are functions of
    /// [`Column::type_name`], [`Column::length`] and [`Column::data_type`],
    /// which is why they are derived in one place instead of being set by each
    /// of the two paths that read a column.
    pub fn derive(&mut self) {
        self.type_string = sqltypes::type_string(&self.type_name, self.length);
        self.is_char_type = sqltypes::is_char_type(&self.type_name);
        self.jdbc_type = sqltypes::jdbc_type(self.data_type).to_string();
        self.java_type = sqltypes::java_type(self.data_type).to_string();
    }
}

impl Model for Column {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "catalog" => Value::Str(Cow::Borrowed(&self.catalog)),
            "schema" => Value::Str(Cow::Borrowed(&self.schema)),
            "table" => Value::Str(Cow::Borrowed(&self.table)),
            "name" | "column" => Value::Str(Cow::Borrowed(&self.name)),
            "typeName" => Value::Str(Cow::Borrowed(&self.type_name)),
            "typeString" => Value::Str(Cow::Borrowed(&self.type_string)),
            // `key` and `charType` are jdbgen's accidental second spellings:
            // its reflection tries `is<Property>()` last, so `${key}` finds
            // `isKey()` just as `${isKey}` does. Assets exist that use both.
            "isKey" | "key" => Value::Bool(self.is_key()),
            "isCharType" | "charType" => Value::Bool(self.is_char_type),
            "length" => Value::Int(self.length),
            "precision" => Value::Int(self.precision),
            "scale" => Value::Int(self.scale),
            "nullable" => Value::Int(self.nullable),
            "remarks" => Value::Str(Cow::Borrowed(&self.remarks)),
            "defVal" => Value::Str(Cow::Borrowed(&self.def_val)),
            "dataType" => Value::Int(i64::from(self.data_type)),
            "jdbcType" => Value::Str(Cow::Borrowed(&self.jdbc_type)),
            "javaType" => Value::Str(Cow::Borrowed(&self.java_type)),
            "autoIncrement" => Value::Bool(self.auto_increment),
            "keySeq" => match self.key_seq {
                Some(seq) => Value::Int(i64::from(seq)),
                None => Value::Null,
            },
            "fk" => match &self.fk {
                Some(fk) => Value::List(vec![fk as &dyn Model]),
                None => Value::Null,
            },
            "no" => Value::Int(self.no as i64),
            // jdbgen's field for the name a template gives the NVL of this
            // column. Never filled in by a metadata reader, in either project.
            "nvlColName" => Value::Str(Cow::Borrowed("")),
            _ => return None,
        })
    }
}

/// One column named inside a key or an index.
///
/// A named model rather than a bare string, because a template walks these
/// with `${for}` and reads `${name}` off each one — and because it is what
/// makes a list render as `[A, B]` instead of as nothing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct KeyColumn {
    /// The column name.
    pub name: String,
    /// One-based position inside the key or index.
    pub no: usize,
}

impl Model for KeyColumn {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "name" | "column" => Value::Str(Cow::Borrowed(&self.name)),
            "no" => Value::Int(self.no as i64),
            _ => return None,
        })
    }
}

/// A foreign key, from the side that is looking at it (D8).
///
/// The same type is used for both directions of
/// [`Table::imports`] and [`Table::exports`], and `ref_` always means *the
/// other table*: for an import that is the parent whose key is referenced, for
/// an export it is the child that references this table. [`ForeignKey::columns`]
/// and [`ForeignKey::ref_columns`] are index-aligned and in `KEY_SEQ` order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForeignKey {
    /// The constraint name, empty when the driver reports none.
    pub name: String,
    /// The columns of *this* table the key is made of.
    pub columns: Vec<KeyColumn>,
    /// Catalog of the table at the other end.
    pub ref_catalog: String,
    /// Schema of the table at the other end.
    pub ref_schema: String,
    /// The table at the other end.
    pub ref_table: String,
    /// The columns of that table, aligned with [`ForeignKey::columns`].
    pub ref_columns: Vec<KeyColumn>,
    /// `UPDATE_RULE` as a word: `CASCADE`, `RESTRICT`, `SET NULL`,
    /// `NO ACTION`, `SET DEFAULT`, or empty for a code JDBC does not define.
    pub on_update: String,
    /// `DELETE_RULE` as a word; see [`ForeignKey::on_update`].
    pub on_delete: String,
    /// One-based position in the list this came from.
    pub no: usize,
}

impl Model for ForeignKey {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "name" => Value::Str(Cow::Borrowed(&self.name)),
            "columns" => list(&self.columns),
            "refCatalog" => Value::Str(Cow::Borrowed(&self.ref_catalog)),
            "refSchema" => Value::Str(Cow::Borrowed(&self.ref_schema)),
            "refTable" => Value::Str(Cow::Borrowed(&self.ref_table)),
            "refColumns" => list(&self.ref_columns),
            "onUpdate" => Value::Str(Cow::Borrowed(&self.on_update)),
            "onDelete" => Value::Str(Cow::Borrowed(&self.on_delete)),
            "no" => Value::Int(self.no as i64),
            _ => return None,
        })
    }
}

/// What one column points at, when exactly one foreign key uses it (D8).
///
/// The convenience the navigation-property case wants: a column that is a
/// single-column foreign key knows its target without the template walking
/// [`Table::imports`] to find it. A column inside a composite key still gets
/// one — the pair it takes part in — and a column used by two different keys
/// gets none, because there is no single answer to give.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ForeignKeyRef {
    /// The constraint name.
    pub name: String,
    /// Catalog of the referenced table.
    pub catalog: String,
    /// Schema of the referenced table.
    pub schema: String,
    /// The referenced table.
    pub table: String,
    /// The referenced column.
    pub column: String,
}

impl Model for ForeignKeyRef {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "name" => Value::Str(Cow::Borrowed(&self.name)),
            "catalog" => Value::Str(Cow::Borrowed(&self.catalog)),
            "schema" => Value::Str(Cow::Borrowed(&self.schema)),
            "table" => Value::Str(Cow::Borrowed(&self.table)),
            "column" => Value::Str(Cow::Borrowed(&self.column)),
            _ => return None,
        })
    }
}

/// One index of a table (D8).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Index {
    /// The index name.
    pub name: String,
    /// Whether it is a unique index.
    pub unique: bool,
    /// Its columns, in index order.
    pub columns: Vec<KeyColumn>,
    /// One-based position in the list this came from.
    pub no: usize,
}

impl Model for Index {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "name" => Value::Str(Cow::Borrowed(&self.name)),
            "unique" => Value::Bool(self.unique),
            "columns" => list(&self.columns),
            "no" => Value::Int(self.no as i64),
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn column(name: &str, key_seq: Option<u32>) -> Column {
        Column {
            name: name.to_string(),
            key_seq,
            ..Column::default()
        }
    }

    #[test]
    fn a_compound_table_type_is_reduced_to_the_kind_it_holds() {
        // jdbgen's DBTableTest, case for case.
        for (reported, expected) in [
            ("TABLE", "TABLE"),
            ("VIEW", "VIEW"),
            ("SYSTEM TABLE", "TABLE"),
            ("GLOBAL TEMPORARY TABLE", "TABLE"),
            ("BASE TABLE", "TABLE"),
            ("MATERIALIZED VIEW", "VIEW"),
            ("SYSTEM VIEW", "VIEW"),
        ] {
            assert_eq!(table_kind(Some(reported)), expected, "{reported}");
        }
    }

    #[test]
    fn a_custom_query_may_omit_the_table_type() {
        assert_eq!(table_kind(None), "TABLE");
        // The empty string is what a `select '' as TABLE_TYPE` answers, and it
        // says as little as a null does.
        assert_eq!(table_kind(Some("")), "TABLE");
    }

    #[test]
    fn a_type_that_is_neither_a_table_nor_a_view_is_left_as_it_is() {
        assert_eq!(table_kind(Some("SEQUENCE")), "SEQUENCE");
    }

    #[test]
    fn a_view_is_shown_with_another_icon_than_a_table() {
        let table = TableRef {
            kind: KIND_TABLE.to_string(),
            ..TableRef::default()
        };
        let view = TableRef {
            kind: KIND_VIEW.to_string(),
            ..TableRef::default()
        };
        assert!(table.icon().starts_with("fa:"));
        assert!(view.icon().starts_with("fa:"));
        assert_ne!(table.icon(), view.icon());
        assert!(!table.is_view() && view.is_view());
    }

    #[test]
    fn the_key_columns_come_back_in_the_order_the_key_was_declared_in() {
        let table = Table {
            // Declared `primary key (B, A)`, reported by column name.
            columns: vec![
                column("A", Some(2)),
                column("B", Some(1)),
                column("C", None),
            ],
            ..Table::default()
        };

        let keys: Vec<&str> = table.keys().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(keys, vec!["B", "A"]);
        let rest: Vec<&str> = table.not_keys().iter().map(|c| c.name.as_str()).collect();
        assert_eq!(rest, vec!["C"]);
    }

    #[test]
    fn a_table_answers_jdbgens_member_names() {
        let mut table = Table {
            catalog: "DB".to_string(),
            schema: "PUBLIC".to_string(),
            name: "TB_USER".to_string(),
            kind: KIND_TABLE.to_string(),
            remarks: "the users".to_string(),
            columns: vec![column("ID", Some(1)), column("NAME", None)],
            no: 3,
            ..Table::default()
        };
        table.indexes.push(Index {
            name: "IX_USER".to_string(),
            unique: false,
            columns: vec![KeyColumn {
                name: "NAME".to_string(),
                no: 1,
            }],
            no: 1,
        });

        let text = |key: &str| table.get(key).expect(key).to_text().into_owned();
        assert_eq!(text("catalog"), "DB");
        assert_eq!(text("schema"), "PUBLIC");
        // One name under three of jdbgen's spellings.
        assert_eq!(text("name"), "TB_USER");
        assert_eq!(text("table"), "TB_USER");
        assert_eq!(text("title"), "TB_USER");
        assert_eq!(text("type"), "TABLE");
        assert_eq!(text("remarks"), "the users");
        assert_eq!(text("icon"), "fa:TABLE");
        assert_eq!(text("no"), "3");
        assert_eq!(
            text("source"),
            "",
            "a template-only field is still answered"
        );
        assert_eq!(text("columns"), "[ID, NAME]");
        assert_eq!(text("keys"), "[ID]");
        assert_eq!(text("notKeys"), "[NAME]");
        assert_eq!(text("indexes"), "[IX_USER]");
        assert_eq!(text("imports"), "[]", "an empty list is still a list");
        assert!(table.get("nosuchfield").is_none());
    }

    #[test]
    fn a_column_answers_jdbgens_member_names() {
        let mut column = Column {
            catalog: "DB".to_string(),
            schema: "PUBLIC".to_string(),
            table: "TB_USER".to_string(),
            name: "USER_NAME".to_string(),
            type_name: "varchar".to_string(),
            length: 40,
            precision: 40,
            scale: 0,
            nullable: 1,
            remarks: "the name".to_string(),
            def_val: "'anonymous'".to_string(),
            data_type: 12,
            key_seq: Some(1),
            no: 7,
            ..Column::default()
        };
        column.derive();

        let text = |key: &str| column.get(key).expect(key).to_text().into_owned();
        assert_eq!(text("name"), "USER_NAME");
        assert_eq!(text("column"), "USER_NAME");
        assert_eq!(text("typeName"), "varchar", "the driver's own spelling");
        assert_eq!(text("typeString"), "VARCHAR(40)");
        assert_eq!(text("isCharType"), "true");
        assert_eq!(text("charType"), "true", "jdbgen's second spelling");
        assert_eq!(text("isKey"), "true");
        assert_eq!(text("key"), "true", "jdbgen's second spelling");
        assert_eq!(text("length"), "40");
        assert_eq!(text("precision"), "40");
        assert_eq!(text("scale"), "0");
        assert_eq!(text("nullable"), "1");
        assert_eq!(text("remarks"), "the name");
        assert_eq!(text("defVal"), "'anonymous'");
        assert_eq!(text("dataType"), "12");
        assert_eq!(text("jdbcType"), "VARCHAR");
        assert_eq!(text("javaType"), "String");
        assert_eq!(text("autoIncrement"), "false");
        assert_eq!(text("keySeq"), "1");
        assert_eq!(text("no"), "7");
        assert_eq!(text("nvlColName"), "");
        assert!(column.get("fk").expect("fk").is_null());
        assert!(column.get("nosuchfield").is_none());
    }

    #[test]
    fn a_type_code_no_jdbc_version_knows_leaves_the_type_names_empty() {
        let mut column = Column {
            type_name: "GEOMETRY".to_string(),
            data_type: 9999,
            length: 10,
            ..Column::default()
        };
        column.derive();

        assert_eq!(column.jdbc_type, "");
        assert_eq!(column.java_type, "");
        assert_eq!(
            column.type_string, "GEOMETRY",
            "the database type is still usable"
        );
    }

    #[test]
    fn a_foreign_key_answers_both_ends_of_itself() {
        let fk = ForeignKey {
            name: "FK_TRACK_ALBUM".to_string(),
            columns: vec![KeyColumn {
                name: "ALBUM_ID".to_string(),
                no: 1,
            }],
            ref_catalog: "DB".to_string(),
            ref_schema: "PUBLIC".to_string(),
            ref_table: "T_ALBUM".to_string(),
            ref_columns: vec![KeyColumn {
                name: "ID".to_string(),
                no: 1,
            }],
            on_update: "NO ACTION".to_string(),
            on_delete: "CASCADE".to_string(),
            no: 1,
        };

        let text = |key: &str| fk.get(key).expect(key).to_text().into_owned();
        assert_eq!(text("name"), "FK_TRACK_ALBUM");
        assert_eq!(text("columns"), "[ALBUM_ID]");
        assert_eq!(text("refCatalog"), "DB");
        assert_eq!(text("refSchema"), "PUBLIC");
        assert_eq!(text("refTable"), "T_ALBUM");
        assert_eq!(text("refColumns"), "[ID]");
        assert_eq!(text("onUpdate"), "NO ACTION");
        assert_eq!(text("onDelete"), "CASCADE");
    }

    #[test]
    fn a_schema_and_a_key_column_answer_their_own_members() {
        let schema = Schema {
            catalog: "DB".to_string(),
            schema: "PUBLIC".to_string(),
            name: "PUBLIC".to_string(),
        };
        assert_eq!(
            schema.get("catalog").expect("catalog").to_text().as_ref(),
            "DB"
        );
        assert_eq!(
            schema.get("name").expect("name").to_text().as_ref(),
            "PUBLIC"
        );

        let key = KeyColumn {
            name: "ID".to_string(),
            no: 2,
        };
        assert_eq!(key.get("column").expect("column").to_text().as_ref(), "ID");
        assert_eq!(key.get("no").expect("no").to_text().as_ref(), "2");
    }
}
