# Custom queries

Some JDBC drivers do not implement `DatabaseMetaData` completely, so rudbgen
cannot discover tables, columns or comments through the standard calls. For
those drivers you can supply your own database-specific SQL and rudbgen will use
it instead.

Custom queries are configured **per driver**, in the **Custom queries** section
of the driver editor (reached from the connection dialog — see the
[UI guide](ui-guide.md)). Each of the four is a checkbox, a SQL editor and a
**Test** button that runs the statement on a chosen open connection and reports
the labels it found against the labels the contract requires.

The contract is jdbgen's, kept verbatim, so a driver definition imported from a
jdbgen `config.json` works unedited.

[← Documentation index](README.md)

---

## How substitution works

Before a custom query runs, rudbgen replaces the `${...}` placeholders in it
with the values of the object being expanded:

| Query | Runs | Placeholders |
|:---|:---|:---|
| Table list | once per schema | `${catalog}`, `${schema}` |
| Table comments | once per schema | `${catalog}`, `${schema}` |
| Column list | once per table | `${catalog}`, `${schema}`, `${table}` |
| Column comments | once per table | `${catalog}`, `${schema}`, `${table}` |

> **Corrected — the placeholder set is fixed, not "every getter".**
> jdbgen substituted by reflection, which made every property of its internal
> schema and table objects a possible hole — `${name}`, `${tables}`,
> `${type}`, `${remarks}`, and nested `${schema.name}` forms. rudbgen
> substitutes the three the contract documents and leaves everything else
> exactly as written. A statement that leaned on `${name}` as an alias of
> `${schema}` or `${table}` has to be edited to say so.
>
> This is a narrowing, not a loss: the reflected names were an accident of the
> implementation, none of the shipped driver definitions used them, and a
> placeholder that is left in the statement fails loudly (see below) rather than
> quietly matching nothing.

### Substitution pitfalls

Substitution is deliberately simple, and that simplicity has consequences:

- **An empty value is not substituted.** The placeholder is left in the SQL
  exactly as written, so the statement reaches the database containing the
  literal text `${schema}` and comes back as a syntax error. That is on purpose:
  the alternative — substituting an empty string — turns
  `where TABLE_SCHEMA = '${schema}'` into `= ''`, which is a valid statement
  that silently answers nothing. A warning naming the placeholder is written to
  the log every time it happens, and it is the first thing to look at when a
  custom query returns an empty list.
- **Placeholder names are case-sensitive.** `${SCHEMA}` is not `${schema}`.
- **There is no quoting and no escaping.** The value is pasted in verbatim. An
  identifier containing a single quote breaks the statement.
- **A `${` with no closing `}` is not a placeholder.** The tail of the statement
  is kept as written and the database decides what to make of it.
- **Template processors do not work here.** `${name.camel}`, `${schema.upper}`
  and the rest of the [template processors](template-reference.md) belong to the
  code generator, not to custom queries: `${schema.upper}` is simply an unknown
  placeholder and stays in the SQL.

### When the database has no catalog or no schema

Not every product reports both levels. rudbgen fills a missing level with an
empty value, which means the placeholder is **left in the SQL** — so for such a
product, do not reference `${catalog}` or `${schema}` at all. Hard-code the
value or drop the predicate.

> **Corrected.** jdbgen substituted the literal string `Default Catalog` for a
> missing catalog, which quietly matched nothing. rudbgen leaves the placeholder
> in and logs a warning, so the query fails where it is wrong instead of
> succeeding with no rows.

---

## Mixing custom and standard metadata

The four queries are independent switches. Whatever you leave unchecked keeps
using the bridge's standard `DatabaseMetaData` path, so you only need to
override the parts your driver gets wrong. A driver that lists tables and
columns correctly but exposes no comments only needs the two comment queries —
which is exactly how the bundled Microsoft SQL Server entry is configured.

One combination changes behaviour rather than just the data source:

- **Without** a custom column list query, primary keys come from
  `getPrimaryKeys()`, and so does the key *order*.
- **With** a custom column list query, `getPrimaryKeys()` is never called. The
  only thing that marks a column as a key is the `IS_KEY` field of your result
  set, and the key order is the order your statement returns the key columns in.

Foreign keys and indexes (`imports`, `exports`, `indexes` — architecture
decision D8) always come from the driver's own metadata. There is no custom
query for them, and enabling a custom column list does not switch them off.

---

## Table comments

A query returning the comment of every table in a schema. Runs once per schema,
after the table list has been loaded.

The result set is read **positionally**: column 1 is the table name, column 2 is
its comment. Labels are irrelevant, so name them anything. The statement must
return at least two columns; extra ones are ignored.

Table names are matched against the table list loaded just before, and a comment
is applied **only when the name matches a known table**. Tables missing from the
result set keep whatever comment they already had, so a query returning only some
of the tables is safe.

| Position | Type | Description |
|:---:|:---:|:---|
| 1 | text | Table name. Must match the name in the table list exactly, letter case included |
| 2 | text | Table comment |

Example for Microsoft SQL Server (shipped with the built-in entry):

```sql
SELECT OBJNAME, cast(value as varchar(8000)) as VALUE
FROM fn_listextendedproperty ('MS_DESCRIPTION','schema','${schema}','table',null,null,null)
```

---

## Column comments

A query returning the comment of every column of one table. Runs once per table,
after the column list has been loaded. Read **positionally**, exactly as table
comments are: column 1 is the column name, column 2 is the comment.

> **Changed — a partial result no longer erases comments.**
> jdbgen collected the rows into a map and then applied that map to *every*
> column, so a column missing from the result set had its comment overwritten
> with null. A partial column-comments query therefore deleted the comments the
> JDBC driver had already reported — and jdbgen's own documentation warned users
> to return a row per column or leave the query off.
>
> rudbgen applies a comment **only to the names the query returns**, which is
> what the table-comment path always did. A partial query is now safe, and the
> two comment queries behave the same way as each other. This is the second of
> the two deliberate behaviour changes listed in the architecture document §7.2.

| Position | Type | Description |
|:---:|:---:|:---|
| 1 | text | Column name. Must match the name in the column list exactly, letter case included |
| 2 | text | Column comment |

Example for Microsoft SQL Server (shipped with the built-in entry):

```sql
SELECT OBJNAME, cast(value as varchar(8000)) as VALUE
FROM fn_listextendedproperty ('MS_DESCRIPTION','schema','${schema}','table','${table}','column',null)
```

---

## Table list

A query returning all tables and views of a schema.

Unlike the comment queries, this result set is read **by column label**, and
every label below is mandatory. A missing label fails the read and no tables are
listed at all — so alias your columns when their natural names differ. Labels
are compared upper-cased; a label the statement selects twice is read from its
first occurrence.

| Label | Type | Description |
|:---:|:---:|:---|
| `TABLE_CAT` | text | Catalog the table belongs to. May be null; the schema's own catalog is used then |
| `TABLE_SCHEM` | text | Schema the table belongs to. May be null; likewise |
| `TABLE_NAME` | text | Table name |
| `TABLE_TYPE` | text | Table type. The label must exist; the value may be null — see below |
| `REMARKS` | text | Table comment. May be null |

### How `TABLE_TYPE` is interpreted

It is normalised before use:

1. A null or empty value is treated as `TABLE`.
2. `TABLE` and `VIEW` are used as they are.
3. Any other value is scanned: if it *contains* `TABLE` it becomes `TABLE`, if it
   contains `VIEW` it becomes `VIEW`. This is what makes `BASE TABLE` and
   `SYSTEM VIEW` work with no extra effort. The test is **case-sensitive** —
   `base table` in lower case matches neither.
4. A value that matches none of these is **dropped**: the table is read and then
   filtered out, so it never reaches the explorer.

Point 4 is the usual reason for a mysteriously short table list. Types such as
`SYNONYM`, `ALIAS` or `SEQUENCE` disappear silently; map them to `TABLE`
yourself if you want to generate code from them.

Views are additionally hidden unless the **views** toggle in the explorer is on.

Example for H2 (shipped with the built-in `H2 Embedded` and `H2 Server`
entries):

```sql
select TABLE_CATALOG as "TABLE_CAT",
       TABLE_SCHEMA as "TABLE_SCHEM",
       TABLE_NAME,
       CASE WHEN TABLE_TYPE='BASE TABLE' THEN 'TABLE' ELSE TABLE_TYPE END AS "TABLE_TYPE",
       REMARKS
  from information_schema.tables
 where TABLE_CATALOG='${catalog}'
   and TABLE_SCHEMA='${schema}'
```

---

## Column list

A query returning all columns of one table. Runs once per table. Read **by
column label** as well, and every label below is mandatory — including `IS_KEY`,
which has no JDBC equivalent.

| Label | Type | Description | Template field |
|:---:|:---:|:---|:---:|
| `TABLE_CAT` | text | Catalog the table belongs to | `catalog` |
| `TABLE_SCHEM` | text | Schema the table belongs to | `schema` |
| `TABLE_NAME` | text | Table name | `table` |
| `COLUMN_NAME` | text | Column name | `column`, `name` |
| `DATA_TYPE` | int | The matching `java.sql.Types` constant | `dataType` |
| `TYPE_NAME` | text | Database-specific type name. Null becomes an empty string | `typeName` |
| `COLUMN_SIZE` | int | Column length | `length`, `precision` |
| `NULLABLE` | int | `1` nullable, `0` not | `nullable` |
| `REMARKS` | text | Column comment | `remarks` |
| `COLUMN_DEF` | text | Column default value | `defVal` |
| `IS_KEY` | int | Non-zero when the column is part of the primary key | `isKey`, `keySeq` |

Columns are numbered in the order the query returns them, so add an `ORDER BY`
if your database does not already return them in declaration order. `keySeq`
counts the key columns in that same order.

Three template fields have **no label in this contract** and are therefore not
available on a driver that overrides the column list: `scale` is `0`,
`autoIncrement` is `false`, and `fk` is empty. Adding labels for them would make
a statement written for jdbgen fail here, which is the trade the contract makes.
A driver good enough to need no column override gets all three.

### `IS_KEY` must be numeric

`IS_KEY` is read as text and then parsed as an integer, with jdbgen's parser:
thousands separators are dropped, a fractional part is truncated, and **anything
else at all makes the answer 0** — including a trailing space, which is what a
fixed-width `CHAR(1)` comes back with.

| Returned value | Interpreted as |
|:---:|:---|
| `1` | primary key |
| `0` | not a primary key |
| `'Y'` | **not a primary key** — non-numeric, parsed as 0 |
| `'true'` | **not a primary key** — non-numeric, parsed as 0 |
| `'1 '` | **not a primary key** — the trailing space is non-numeric |
| null | not a primary key |

Always return `0` or `1`, and cast rather than pad. If the table has no primary
key at all, return `0` for every row; do not drop the column.

### `DATA_TYPE` decides the generated types

`DATA_TYPE` is not merely informative. It is looked up in the mapping table of
the [template reference](template-reference.md#jdbc-and-java-type-mapping) to
derive `jdbcType` and `javaType`, so a wrong value produces wrong code:

- A value with no mapping — `TIMESTAMP_WITH_TIMEZONE` (2014), for instance —
  leaves both fields empty, and `${javaType}` renders as nothing.
- `0` is **not** a neutral value: it is `java.sql.Types.NULL`, which maps to the
  Java type `null`. Using it as a catch-all produces model fields declared
  `null`.

If you cannot classify a type, `1111` (`java.sql.Types.OTHER`) is a better
fallback than `0`.

Example for H2 — illustrative, not part of the shipped configuration; its
`CASE` covers a handful of types and whatever falls through to the `ELSE`
generates unusable Java types:

```sql
select TABLE_CATALOG as "TABLE_CAT",
       TABLE_SCHEMA as "TABLE_SCHEM",
       TABLE_NAME,
       COLUMN_NAME,
       CASE WHEN DATA_TYPE LIKE 'CHAR%' THEN 12
            WHEN DATA_TYPE='INTEGER' THEN 4
            WHEN DATA_TYPE='DATE' THEN 91
            WHEN DATA_TYPE='BIGINT' THEN -5
            WHEN DATA_TYPE='BOOLEAN' THEN 16
            ELSE 1111 END AS "DATA_TYPE",
       DATA_TYPE as "TYPE_NAME",
       CHARACTER_MAXIMUM_LENGTH as "COLUMN_SIZE",
       CASE WHEN IS_NULLABLE='YES' THEN 1 ELSE 0 END as "NULLABLE",
       REMARKS,
       COLUMN_DEFAULT as "COLUMN_DEF",
       CASE WHEN exists(select 1
                          from information_schema.index_columns B
                         where TABLE_CATALOG='${catalog}'
                           and TABLE_SCHEMA='${schema}'
                           and TABLE_NAME='${table}'
                           and COLUMN_NAME=A.COLUMN_NAME
                           and INDEX_NAME=(select INDEX_NAME from information_schema.indexes
                                            where TABLE_CATALOG='${catalog}'
                                              and TABLE_SCHEMA='${schema}'
                                              and TABLE_NAME='${table}'
                                              and INDEX_TYPE_NAME='PRIMARY KEY'))
           THEN 1 ELSE 0 END AS "IS_KEY"
  from information_schema.columns A
 where TABLE_CATALOG='${catalog}'
   and TABLE_SCHEMA='${schema}'
   and TABLE_NAME='${table}'
 order by ORDINAL_POSITION
```

---

## Contracts at a glance

| Query | Read by | Required | Effect of a missing row |
|:---|:---:|:---|:---|
| Table comments | position (1, 2) | 2 columns or more | Table keeps its previous comment |
| Column comments | position (1, 2) | 2 columns or more | Column keeps its previous comment (**Changed**) |
| Table list | label | `TABLE_CAT`, `TABLE_SCHEM`, `TABLE_NAME`, `TABLE_TYPE`, `REMARKS` | Table is not listed |
| Column list | label | `TABLE_CAT`, `TABLE_SCHEM`, `TABLE_NAME`, `COLUMN_NAME`, `DATA_TYPE`, `TYPE_NAME`, `COLUMN_SIZE`, `NULLABLE`, `REMARKS`, `COLUMN_DEF`, `IS_KEY` | Column is not listed |

For the label-based queries a missing **label** is fatal — the read fails and
names the first label it could not find. A missing **value** (null) is tolerated
for every field except the names themselves. For the positional queries a
result set with fewer than two columns is fatal, and anything past the second is
ignored.

Built-in drivers shipping with a custom query out of the box:

| Driver entry | Custom queries enabled |
|:---|:---|
| H2 Embedded | Table list |
| H2 Server | Table list |
| Microsoft SQL Server | Table comments, column comments |

Every other bundled driver relies entirely on standard JDBC metadata.

---

## Testing a query

Each of the four rows has a **Test** button, and above them three little
fields — **Test with**: catalog, schema, table — that the placeholders are
substituted from. Test runs the statement against a session open on this same
driver (or opens a temporary one from the connection form and closes it again),
reads back the labels of the result set and checks them against the contract.

The answer is one line under the row:

| Line | Meaning |
|:---|:---|
| `OK — 42 rows, and every column the reader needs is there.` | The shape is right. A row count of 0 with an OK shape means the statement is correct and this schema has nothing to say |
| `Missing columns: IS_KEY` | A label-read query is short of a label — the whole list of what is missing, not just the first |
| `The first 2 columns are read; only 1 came back.` | A positional query came back too narrow to read. Extra columns past the ones that are read are fine and are not reported |
| `The statement failed: …` | The database rejected it. An unsubstituted `${schema}` shows up here, as a syntax error naming the placeholder |
| `Connect to a database that uses this driver first…` | There is no session to run it on. A statement run against another product would fail for reasons that say nothing about the statement |

Only the shape is reported; the rows themselves are not shown.

The test asks exactly what the reader asks, no more: a label-read query must
carry every required label, and a positional one must be at least as wide as the
positions that are read. A comment query with a third column of its own passes,
because the reader takes the first two and ignores the rest.

## Writing queries for a new database

1. **Start with the smallest override.** Enable only the query whose data is
   actually wrong; leave the rest on the standard path.
2. **Run the SQL in a database client first**, with the placeholders filled in by
   hand. Confirm it works before pasting it in.
3. **Alias every column** so the labels match exactly. `TABLE_SCHEM` has no `A`;
   `COLUMN_DEF` is not `COLUMN_DEFAULT`.
4. **Check whether your database has catalogs and schemas.** If it has not, drop
   the placeholder rather than comparing against it — an empty one is left in
   the statement and the query fails.
5. **Return real `java.sql.Types` numbers** for `DATA_TYPE`, and cover every type
   your schema uses. Avoid `0`.
6. **Return `0`/`1` for `IS_KEY`**, never `'Y'`/`'N'`, and cast rather than pad.
7. **Order the column list** explicitly; the row order is the column order and
   the key order.
8. **Normalise `TABLE_TYPE` yourself** for anything that is not literally `TABLE`
   or `VIEW` and that you still want to see.
9. **Match letter case.** The comment queries join on names, and the join is
   case-sensitive.
10. **Use Test**, then watch the log while connecting: unsubstituted
    placeholders and unmatched names are reported there.

---

## Related documentation

- [User interface guide](ui-guide.md) — where the driver editor lives.
- [Template reference](template-reference.md) — the fields your queries populate
  and the `DATA_TYPE` mapping table.
- [Installation](installation.md) — where `drivers.json` is kept.
