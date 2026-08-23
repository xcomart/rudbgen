//! Reading the model off a session: `DESCRIBE` for everything the driver
//! answers well, the four custom queries where a product needs them (D9).
//!
//! # No cache
//!
//! Every call here is a round trip. That is deliberate: the application owns
//! the cache, because only it knows when a refresh was asked for, which
//! connection a cached table belongs to and when the memory should go. A cache
//! down here would be a second, invisible one underneath it.
//!
//! # One reader, one connection
//!
//! [`MetaReader`] borrows a session and a driver definition and holds no state
//! of its own, so it is made where it is used and dropped after. Every call
//! blocks on the session's worker thread — this crate does not know gpui, and
//! moving these calls off the UI thread is `rudbgen-app`'s job
//! (architecture.md §3).

use std::collections::HashMap;

use rudbgen_core::{CustomQueryKind, DriverDef};
use rudbgen_jdbc::{DescribeRequest, Session};
use serde_json::{Map, Value as Json};

use crate::error::Result;
use crate::model::{Column, ForeignKey, ForeignKeyRef, Index, KeyColumn, Schema, Table, TableRef};
use crate::query::{Rows, substitute, to_int};

/// What a catalog with no name of its own is called, as in jdbgen.
pub const DEFAULT_CATALOG: &str = "Default Catalog";

/// What a database with neither catalogs nor schemas is called, as in jdbgen.
pub const DEFAULT_SCHEMA: &str = "Default Schema";

/// One row of `getIndexInfo` that describes the table rather than an index.
///
/// `DatabaseMetaData.tableIndexStatistic`. It carries a row count and no index
/// name, and letting it through would put a nameless index in every table.
const INDEX_STATISTIC: i64 = 0;

/// `importedKeyCascade` and its four siblings, in the order JDBC numbers them.
const RULES: [&str; 5] = [
    "CASCADE",
    "RESTRICT",
    "SET NULL",
    "NO ACTION",
    "SET DEFAULT",
];

/// Reads the table model of one connection.
pub struct MetaReader<'a> {
    /// The open connection everything is read through.
    session: &'a Session,
    /// The driver definition, for its custom queries (D9).
    driver: &'a DriverDef,
}

impl<'a> MetaReader<'a> {
    /// A reader over one session, reading it the way one driver definition
    /// says to.
    pub fn new(session: &'a Session, driver: &'a DriverDef) -> Self {
        MetaReader { session, driver }
    }

    /// The session this reads through.
    pub fn session(&self) -> &Session {
        self.session
    }

    /// The driver definition this reads by.
    pub fn driver(&self) -> &DriverDef {
        self.driver
    }

    /// Every schema of the database, in catalog order.
    ///
    /// jdbgen's `DBMeta.getSchemaTree` rules, kept because the explorer tree
    /// is built from the result and an empty list there reads as a failed
    /// connection:
    ///
    /// 1. catalogs first; a catalog the driver reports without a name is
    ///    called [`DEFAULT_CATALOG`];
    /// 2. a product with no catalogs at all — Oracle — has its schemas read
    ///    without one and grouped under that same name;
    /// 3. a catalog with no schemas — MySQL, where a catalog *is* the
    ///    database — gets one placeholder schema carrying the catalog;
    /// 4. a product with neither gets a single [`DEFAULT_SCHEMA`] entry.
    ///
    /// So the list is never empty, and every entry can be handed to
    /// [`MetaReader::tables`].
    ///
    /// One difference from jdbgen: the placeholder of rule 3 carries the
    /// catalog name as its display name and an **empty**
    /// [`Schema::catalog`] when the catalog is the synthetic
    /// [`DEFAULT_CATALOG`]. jdbgen stores the words "Default Catalog" as the
    /// catalog and then asks the driver for the tables of a catalog by that
    /// name, which no database has.
    pub fn schemas(&self) -> Result<Vec<Schema>> {
        let mut tree: Vec<(String, Vec<Schema>)> = Vec::new();
        for item in self.describe(DescribeRequest::new("catalogs"))? {
            let name = match text(&item, "name") {
                "" => DEFAULT_CATALOG.to_string(),
                name => name.to_string(),
            };
            if !tree.iter().any(|(catalog, _)| *catalog == name) {
                tree.push((name, Vec::new()));
            }
        }

        if tree.is_empty() {
            // A product without catalogs still has schemas; ask for them
            // without one rather than falling straight through to rule 4.
            let schemas = self.schemas_of(None)?;
            if !schemas.is_empty() {
                tree.push((DEFAULT_CATALOG.to_string(), schemas));
            }
        }

        let mut all = Vec::new();
        for (catalog, schemas) in &mut tree {
            let named = (catalog != DEFAULT_CATALOG).then_some(catalog.as_str());
            if schemas.is_empty() {
                *schemas = self.schemas_of(named)?;
            }
            if schemas.is_empty() {
                schemas.push(Schema {
                    catalog: named.unwrap_or_default().to_string(),
                    schema: String::new(),
                    name: catalog.clone(),
                });
            }
            all.append(schemas);
        }

        if all.is_empty() {
            all.push(Schema {
                catalog: String::new(),
                schema: String::new(),
                name: DEFAULT_SCHEMA.to_string(),
            });
        }
        Ok(all)
    }

    /// The schemas of one catalog, or of no catalog at all.
    fn schemas_of(&self, catalog: Option<&str>) -> Result<Vec<Schema>> {
        let mut request = DescribeRequest::new("schemas");
        request.catalog = catalog.map(str::to_string);
        Ok(self
            .describe(request)?
            .iter()
            .map(|item| {
                let name = text(item, "name").to_string();
                Schema {
                    catalog: match text(item, "catalog") {
                        "" => catalog.unwrap_or_default().to_string(),
                        reported => reported.to_string(),
                    },
                    schema: name.clone(),
                    name,
                }
            })
            .collect())
    }

    /// The tables of one schema, views included when asked for.
    ///
    /// The table list comes from the driver, or from the driver definition's
    /// table-list query when it has one (D9); the comments then come from its
    /// table-comment query when it has one, and **only for the names that
    /// query returns** — see the note in architecture.md §6 on why jdbgen's
    /// blanking of the others is not reproduced.
    ///
    /// Anything whose type normalises to neither `TABLE` nor `VIEW` is
    /// dropped, which is what keeps sequences and system aliases out of a list
    /// whose every row is a file to generate.
    pub fn tables(&self, schema: &Schema, include_views: bool) -> Result<Vec<TableRef>> {
        let mut tables = match self.driver.tables_query() {
            Some(sql) => self.tables_by_query(schema, sql)?,
            None => self.tables_by_describe(schema)?,
        };

        if let Some(sql) = self.driver.table_comments_query() {
            let comments = self.comments(
                CustomQueryKind::TableComments,
                sql,
                &[("catalog", &schema.catalog), ("schema", &schema.schema)],
            )?;
            for table in &mut tables {
                if let Some(comment) = comments.get(&table.name) {
                    table.remarks = comment.clone();
                }
            }
        }

        // jdbgen's filter: a kind that normalised to neither is not shown at
        // all, and a view only when it was asked for.
        tables.retain(|table| {
            table.kind == crate::model::KIND_TABLE || (include_views && table.is_view())
        });
        for (index, table) in tables.iter_mut().enumerate() {
            table.no = index + 1;
        }
        Ok(tables)
    }

    /// The table list as the driver reports it.
    fn tables_by_describe(&self, schema: &Schema) -> Result<Vec<TableRef>> {
        let mut request = DescribeRequest::new("tables");
        request.catalog = non_empty(&schema.catalog);
        request.schema = non_empty(&schema.schema);
        Ok(self
            .describe(request)?
            .iter()
            .map(|item| TableRef {
                catalog: text(item, "catalog").to_string(),
                schema: text(item, "schema").to_string(),
                name: text(item, "name").to_string(),
                kind: crate::model::table_kind(item.get("type").and_then(Json::as_str)),
                remarks: text(item, "remarks").to_string(),
                no: 0,
            })
            .collect())
    }

    /// The table list as the driver definition's own statement reports it.
    ///
    /// One difference from jdbgen: a `TABLE_CAT` or `TABLE_SCHEM` the
    /// statement answers empty falls back to the schema being listed. jdbgen
    /// keeps the null, and the column list of such a table is then read
    /// without a schema filter — which on a database with the same table name
    /// in two schemas reads the wrong one.
    fn tables_by_query(&self, schema: &Schema, sql: &str) -> Result<Vec<TableRef>> {
        let kind = CustomQueryKind::Tables;
        let sql = substitute(
            sql,
            &[("catalog", &schema.catalog), ("schema", &schema.schema)],
        );
        let rows = Rows::run(self.session, kind, &sql)?;
        rows.require_labels()?;

        let mut tables = Vec::with_capacity(rows.len());
        for row in 0..rows.len() {
            let reported = rows.label(row, "TABLE_TYPE");
            tables.push(TableRef {
                catalog: or_else(rows.label(row, "TABLE_CAT"), &schema.catalog),
                schema: or_else(rows.label(row, "TABLE_SCHEM"), &schema.schema),
                name: rows.label(row, "TABLE_NAME").to_string(),
                kind: crate::model::table_kind(
                    (!rows.is_null(row, "TABLE_TYPE")).then_some(reported),
                ),
                remarks: rows.label(row, "REMARKS").to_string(),
                no: 0,
            });
        }
        Ok(tables)
    }

    /// Everything about one table: its columns, its primary key, the foreign
    /// keys in both directions and its indexes.
    ///
    /// Four to five round trips, and no cache — see the module documentation.
    pub fn table(&self, table: &TableRef) -> Result<Table> {
        let mut loaded = Table {
            catalog: table.catalog.clone(),
            schema: table.schema.clone(),
            name: table.name.clone(),
            kind: table.kind.clone(),
            remarks: table.remarks.clone(),
            columns: Vec::new(),
            imports: Vec::new(),
            exports: Vec::new(),
            indexes: Vec::new(),
            no: table.no,
        };

        loaded.columns = match self.driver.columns_query() {
            Some(sql) => self.columns_by_query(table, sql)?,
            None => self.columns_by_describe(table)?,
        };

        if let Some(sql) = self.driver.column_comments_query() {
            let comments = self.comments(
                CustomQueryKind::ColumnComments,
                sql,
                &[
                    ("catalog", &table.catalog),
                    ("schema", &table.schema),
                    ("table", &table.name),
                ],
            )?;
            for column in &mut loaded.columns {
                if let Some(comment) = comments.get(&column.name) {
                    column.remarks = comment.clone();
                }
            }
        }

        loaded.imports = self.foreign_keys(table, true)?;
        loaded.exports = self.foreign_keys(table, false)?;
        loaded.indexes = self.indexes(table)?;
        link_foreign_keys(&mut loaded);
        Ok(loaded)
    }

    /// The columns as the driver reports them, with the primary key read
    /// separately.
    fn columns_by_describe(&self, table: &TableRef) -> Result<Vec<Column>> {
        let mut request = DescribeRequest::new("columns");
        request.catalog = non_empty(&table.catalog);
        request.schema = non_empty(&table.schema);
        request.table = Some(table.name.clone());
        let items = self.describe(request)?;

        // `getColumns` takes the table name as a *pattern*, so a table called
        // `TB_USER` also answers the columns of `TBXUSER` where one exists.
        // Escaping the wildcard needs the driver's own escape character and
        // several drivers get that wrong, so the answer is filtered instead —
        // unless the filter would empty a non-empty answer, which would mean
        // the driver reports the name differently than it listed it.
        let exact: Vec<&Map<String, Json>> = items
            .iter()
            .filter(|item| text(item, "table") == table.name)
            .collect();
        let items: Vec<&Map<String, Json>> = if exact.is_empty() && !items.is_empty() {
            log::warn!(
                "no column of {} reports that table name; keeping the whole answer",
                table.name
            );
            items.iter().collect()
        } else {
            exact
        };

        let mut columns: Vec<Column> = items
            .iter()
            .enumerate()
            .map(|(index, item)| {
                let length = number(item, "size");
                let mut column = Column {
                    catalog: text(item, "catalog").to_string(),
                    schema: text(item, "schema").to_string(),
                    table: text(item, "table").to_string(),
                    name: text(item, "name").to_string(),
                    type_name: text(item, "type_name").to_string(),
                    length,
                    precision: length,
                    scale: number(item, "digits"),
                    nullable: number(item, "nullable"),
                    remarks: text(item, "remarks").to_string(),
                    def_val: text(item, "default").to_string(),
                    data_type: number(item, "data_type") as i32,
                    auto_increment: flag(item, "auto_increment"),
                    // The position the columns are read in, as in jdbgen, and
                    // not ORDINAL_POSITION: a driver that reports a gap in the
                    // ordinals would otherwise number a `${for}` differently
                    // from the list it walks.
                    no: index + 1,
                    ..Column::default()
                };
                column.derive();
                column
            })
            .collect();

        let mut request = DescribeRequest::new("primary_keys");
        request.catalog = non_empty(&table.catalog);
        request.schema = non_empty(&table.schema);
        request.table = Some(table.name.clone());
        for item in self.describe(request)? {
            let name = text(&item, "column");
            match columns.iter_mut().find(|column| column.name == name) {
                // KEY_SEQ, not the order the rows arrive in: `getPrimaryKeys`
                // answers by column name, and a composite key generates a
                // predicate that has to match its declaration.
                Some(column) => column.key_seq = Some(number(&item, "seq") as u32),
                None => log::warn!(
                    "primary key column {name:?} of {} is not in its column list; \
                     the driver may report it in another letter case",
                    table.name
                ),
            }
        }
        Ok(columns)
    }

    /// The columns as the driver definition's own statement reports them.
    ///
    /// The primary key comes from its `IS_KEY` column — read as a number, see
    /// [`to_int`] — in the order the statement returns, and
    /// `getPrimaryKeys` is not called at all. That is jdbgen's contract: a
    /// definition that overrides the column list overrides the key with it.
    fn columns_by_query(&self, table: &TableRef, sql: &str) -> Result<Vec<Column>> {
        let kind = CustomQueryKind::Columns;
        let sql = substitute(
            sql,
            &[
                ("catalog", &table.catalog),
                ("schema", &table.schema),
                ("table", &table.name),
            ],
        );
        let rows = Rows::run(self.session, kind, &sql)?;
        rows.require_labels()?;

        let mut columns = Vec::with_capacity(rows.len());
        let mut keys = 0u32;
        for row in 0..rows.len() {
            let length = i64::from(to_int(rows.label(row, "COLUMN_SIZE")));
            let is_key = to_int(rows.label(row, "IS_KEY")) != 0;
            if is_key {
                keys += 1;
            }
            let mut column = Column {
                catalog: or_else(rows.label(row, "TABLE_CAT"), &table.catalog),
                schema: or_else(rows.label(row, "TABLE_SCHEM"), &table.schema),
                table: or_else(rows.label(row, "TABLE_NAME"), &table.name),
                name: rows.label(row, "COLUMN_NAME").to_string(),
                type_name: rows.label(row, "TYPE_NAME").to_string(),
                length,
                precision: length,
                // No label for the scale in jdbgen's contract, and adding one
                // would make a statement written for jdbgen fail here.
                scale: 0,
                nullable: i64::from(to_int(rows.label(row, "NULLABLE"))),
                remarks: rows.label(row, "REMARKS").to_string(),
                def_val: rows.label(row, "COLUMN_DEF").to_string(),
                data_type: to_int(rows.label(row, "DATA_TYPE")),
                // The key order is the order the statement answers in.
                key_seq: is_key.then_some(keys),
                no: row + 1,
                ..Column::default()
            };
            column.derive();
            columns.push(column);
        }
        Ok(columns)
    }

    /// Run one of the two comment queries and collect what it names.
    ///
    /// Read positionally — name, then comment — and returned as a map, so that
    /// the caller can apply it to the names it holds and leave the rest alone.
    fn comments(
        &self,
        kind: CustomQueryKind,
        sql: &str,
        values: &[(&str, &str)],
    ) -> Result<HashMap<String, String>> {
        let sql = substitute(sql, values);
        let rows = Rows::run(self.session, kind, &sql)?;
        let expected = kind.positional_columns().unwrap_or(2);
        rows.require_width(expected)?;
        Ok((0..rows.len())
            .map(|row| (rows.at(row, 0).to_string(), rows.at(row, 1).to_string()))
            .collect())
    }

    /// The foreign keys of a table, in one direction (D8).
    ///
    /// `imported` asks for the keys this table declares; the other direction
    /// asks for the keys other tables declare on it. Both metadata calls have
    /// the same result shape, and `ref_` is always the table at the *other*
    /// end — which is what swaps the two sides over between the directions.
    fn foreign_keys(&self, table: &TableRef, imported: bool) -> Result<Vec<ForeignKey>> {
        let kind = if imported {
            "imported_keys"
        } else {
            "exported_keys"
        };
        let mut request = DescribeRequest::new(kind);
        request.catalog = non_empty(&table.catalog);
        request.schema = non_empty(&table.schema);
        request.table = Some(table.name.clone());

        // (constraint name, the other table) — a constraint name alone is not
        // an identity for an exported key: two child tables may each declare
        // one called `FK_PARENT`.
        let mut groups: Vec<(String, String, ForeignKey, Vec<i64>)> = Vec::new();
        for item in self.describe(request)? {
            let (here, there) = if imported {
                ("fk_", "pk_")
            } else {
                ("pk_", "fk_")
            };
            let name = text(&item, "fk_name").to_string();
            let other = qualified(&item, there);
            let seq = number(&item, "seq");

            let position = match groups
                .iter()
                .position(|(key, table, _, _)| *key == name && *table == other)
            {
                Some(position) => position,
                None => {
                    groups.push((
                        name.clone(),
                        other.clone(),
                        ForeignKey {
                            name,
                            ref_catalog: text(&item, &format!("{there}catalog")).to_string(),
                            ref_schema: text(&item, &format!("{there}schema")).to_string(),
                            ref_table: text(&item, &format!("{there}table")).to_string(),
                            on_update: rule(number(&item, "update_rule")),
                            on_delete: rule(number(&item, "delete_rule")),
                            ..ForeignKey::default()
                        },
                        Vec::new(),
                    ));
                    groups.len() - 1
                }
            };
            let (_, _, key, order) = &mut groups[position];
            key.columns.push(KeyColumn {
                name: text(&item, &format!("{here}column")).to_string(),
                no: 0,
            });
            key.ref_columns.push(KeyColumn {
                name: text(&item, &format!("{there}column")).to_string(),
                no: 0,
            });
            order.push(seq);
        }

        let mut keys: Vec<ForeignKey> = groups
            .into_iter()
            .map(|(_, _, mut key, order)| {
                // KEY_SEQ orders the columns of one key; the rows themselves
                // arrive ordered by the *other* table's name.
                let mut positions: Vec<usize> = (0..order.len()).collect();
                positions.sort_by_key(|index| (order[*index], *index));
                key.columns = take_in_order(&key.columns, &positions);
                key.ref_columns = take_in_order(&key.ref_columns, &positions);
                key
            })
            .collect();
        // By name, so that a generated file does not change because a driver
        // changed the order it answers in.
        keys.sort_by(|left, right| {
            (&left.name, &left.ref_table).cmp(&(&right.name, &right.ref_table))
        });
        for (index, key) in keys.iter_mut().enumerate() {
            key.no = index + 1;
        }
        Ok(keys)
    }

    /// The indexes of a table (D8).
    ///
    /// The primary key's own index is among them, because the database has one
    /// and a template writing DDL needs to know. Rows that describe the table
    /// rather than an index — `tableIndexStatistic` — are dropped.
    fn indexes(&self, table: &TableRef) -> Result<Vec<Index>> {
        let mut request = DescribeRequest::new("indexes");
        request.catalog = non_empty(&table.catalog);
        request.schema = non_empty(&table.schema);
        request.table = Some(table.name.clone());
        request.unique_only = Some(false);
        // Approximate: a statistics refresh on a large schema is the
        // difference between an instant answer and a minute of waiting, and
        // nothing here reads the cardinality it would refresh.
        request.approximate = Some(true);

        let mut groups: Vec<(Index, Vec<i64>)> = Vec::new();
        for item in self.describe(request)? {
            let name = text(&item, "name");
            let column = text(&item, "column");
            if name.is_empty() || column.is_empty() || number(&item, "type") == INDEX_STATISTIC {
                continue;
            }
            let position = match groups.iter().position(|(index, _)| index.name == name) {
                Some(position) => position,
                None => {
                    groups.push((
                        Index {
                            name: name.to_string(),
                            unique: !flag(&item, "non_unique"),
                            ..Index::default()
                        },
                        Vec::new(),
                    ));
                    groups.len() - 1
                }
            };
            let (index, order) = &mut groups[position];
            index.columns.push(KeyColumn {
                name: column.to_string(),
                no: 0,
            });
            order.push(number(&item, "ordinal"));
        }

        let mut indexes: Vec<Index> = groups
            .into_iter()
            .map(|(mut index, order)| {
                let mut positions: Vec<usize> = (0..order.len()).collect();
                positions.sort_by_key(|position| (order[*position], *position));
                index.columns = take_in_order(&index.columns, &positions);
                index
            })
            .collect();
        indexes.sort_by(|left, right| left.name.cmp(&right.name));
        for (position, index) in indexes.iter_mut().enumerate() {
            index.no = position + 1;
        }
        Ok(indexes)
    }

    /// Run one metadata query and hand back its items.
    fn describe(&self, request: DescribeRequest) -> Result<Vec<Map<String, Json>>> {
        Ok(self.session.describe(&request)?.items)
    }
}

/// Reorder a column list by the positions of its sort.
fn take_in_order(columns: &[KeyColumn], positions: &[usize]) -> Vec<KeyColumn> {
    positions
        .iter()
        .enumerate()
        .map(|(rank, position)| KeyColumn {
            name: columns[*position].name.clone(),
            no: rank + 1,
        })
        .collect()
}

/// Point every column that takes part in exactly one imported key at what it
/// references.
///
/// Exactly one: a column used by two different foreign keys has no single
/// answer, and a `fk` that silently named one of them would be worse than
/// none. The template can still walk `imports`.
fn link_foreign_keys(table: &mut Table) {
    let mut refs: HashMap<&str, Option<ForeignKeyRef>> = HashMap::new();
    for key in &table.imports {
        for (position, column) in key.columns.iter().enumerate() {
            let target = ForeignKeyRef {
                name: key.name.clone(),
                catalog: key.ref_catalog.clone(),
                schema: key.ref_schema.clone(),
                table: key.ref_table.clone(),
                column: key
                    .ref_columns
                    .get(position)
                    .map(|reference| reference.name.clone())
                    .unwrap_or_default(),
            };
            refs.entry(column.name.as_str())
                // Seen before, from another key: no single answer to give.
                .and_modify(|existing| *existing = None)
                .or_insert(Some(target));
        }
    }
    let refs: HashMap<String, ForeignKeyRef> = refs
        .into_iter()
        .filter_map(|(name, target)| target.map(|target| (name.to_string(), target)))
        .collect();
    for column in &mut table.columns {
        column.fk = refs.get(&column.name).cloned();
    }
}

/// The name of a foreign key rule code, or `""` for one JDBC does not define.
fn rule(code: i64) -> String {
    usize::try_from(code)
        .ok()
        .and_then(|code| RULES.get(code))
        .map_or(String::new(), |name| (*name).to_string())
}

/// The catalog, schema and table of one side of a foreign key, as one string.
fn qualified(item: &Map<String, Json>, side: &str) -> String {
    format!(
        "{}.{}.{}",
        text(item, &format!("{side}catalog")),
        text(item, &format!("{side}schema")),
        text(item, &format!("{side}table"))
    )
}

/// A string member of a describe item, `""` for a null or an absent one.
fn text<'a>(item: &'a Map<String, Json>, key: &str) -> &'a str {
    item.get(key).and_then(Json::as_str).unwrap_or("")
}

/// A numeric member of a describe item, `0` for a null or an absent one.
fn number(item: &Map<String, Json>, key: &str) -> i64 {
    item.get(key).and_then(Json::as_i64).unwrap_or(0)
}

/// A boolean member of a describe item, `false` for a null or an absent one.
fn flag(item: &Map<String, Json>, key: &str) -> bool {
    item.get(key).and_then(Json::as_bool).unwrap_or(false)
}

/// A filter value, or `None` where the empty string means "do not filter".
fn non_empty(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_string())
}

/// `value` unless it is empty, in which case `fallback`.
fn or_else(value: &str, fallback: &str) -> String {
    if value.is_empty() {
        fallback.to_string()
    } else {
        value.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::KIND_TABLE;

    #[test]
    fn the_foreign_key_rules_are_named_the_way_ddl_writes_them() {
        assert_eq!(rule(0), "CASCADE");
        assert_eq!(rule(1), "RESTRICT");
        assert_eq!(rule(2), "SET NULL");
        assert_eq!(rule(3), "NO ACTION");
        assert_eq!(rule(4), "SET DEFAULT");
        // A code no version of JDBC defines says nothing rather than guessing.
        assert_eq!(rule(9), "");
        assert_eq!(rule(-1), "");
    }

    #[test]
    fn a_column_used_by_one_foreign_key_learns_what_it_points_at() {
        let mut table = Table {
            name: "T_TRACK".to_string(),
            kind: KIND_TABLE.to_string(),
            columns: vec![
                Column {
                    name: "ALBUM_ID".to_string(),
                    ..Column::default()
                },
                Column {
                    name: "TITLE".to_string(),
                    ..Column::default()
                },
            ],
            imports: vec![ForeignKey {
                name: "FK_TRACK_ALBUM".to_string(),
                columns: vec![KeyColumn {
                    name: "ALBUM_ID".to_string(),
                    no: 1,
                }],
                ref_table: "T_ALBUM".to_string(),
                ref_columns: vec![KeyColumn {
                    name: "ID".to_string(),
                    no: 1,
                }],
                ..ForeignKey::default()
            }],
            ..Table::default()
        };

        link_foreign_keys(&mut table);

        let fk = table.columns[0].fk.as_ref().expect("the column has one");
        assert_eq!(fk.table, "T_ALBUM");
        assert_eq!(fk.column, "ID");
        assert_eq!(fk.name, "FK_TRACK_ALBUM");
        assert!(table.columns[1].fk.is_none(), "TITLE points at nothing");
    }

    #[test]
    fn a_column_two_foreign_keys_use_learns_nothing() {
        // There is no single answer, and naming one of the two would be a
        // wrong navigation property in every generated file.
        let shared = |ref_table: &str, name: &str| ForeignKey {
            name: name.to_string(),
            columns: vec![KeyColumn {
                name: "OWNER_ID".to_string(),
                no: 1,
            }],
            ref_table: ref_table.to_string(),
            ref_columns: vec![KeyColumn {
                name: "ID".to_string(),
                no: 1,
            }],
            ..ForeignKey::default()
        };
        let mut table = Table {
            columns: vec![Column {
                name: "OWNER_ID".to_string(),
                ..Column::default()
            }],
            imports: vec![shared("T_USER", "FK_A"), shared("T_TEAM", "FK_B")],
            ..Table::default()
        };

        link_foreign_keys(&mut table);

        assert!(table.columns[0].fk.is_none());
    }

    #[test]
    fn a_key_is_reordered_by_its_sequence_and_numbered_from_one() {
        let columns = vec![
            KeyColumn {
                name: "B".to_string(),
                no: 0,
            },
            KeyColumn {
                name: "A".to_string(),
                no: 0,
            },
        ];
        // KEY_SEQ says A comes first even though the rows arrived B, A.
        let mut positions: Vec<usize> = (0..2).collect();
        let order = [2i64, 1i64];
        positions.sort_by_key(|index| (order[*index], *index));

        let ordered = take_in_order(&columns, &positions);
        assert_eq!(ordered[0].name, "A");
        assert_eq!(ordered[0].no, 1);
        assert_eq!(ordered[1].name, "B");
        assert_eq!(ordered[1].no, 2);
    }

    #[test]
    fn an_empty_filter_is_no_filter_rather_than_an_empty_name() {
        assert_eq!(non_empty(""), None);
        assert_eq!(non_empty("PUBLIC"), Some("PUBLIC".to_string()));
        assert_eq!(or_else("", "PUBLIC"), "PUBLIC");
        assert_eq!(or_else("APP", "PUBLIC"), "APP");
    }
}
