# Template reference

A rudbgen template is an ordinary text file with placeholders in it. rudbgen
reads the metadata of every table you ticked, renders the template once per
table, and writes the result to a file whose name is itself a small template.
This page documents every statement the engine understands, every field it
exposes, and what happens when something goes wrong.

The language is [jdbgen](https://github.com/xcomart/jdbgen)'s, ported to Rust
and kept byte-compatible: jdbgen's three engine test classes are ported case
for case, and the three shipped templates rendered against jdbgen's own fixture
must come out byte for byte identical. Where this page differs from jdbgen's
documentation it is because the *code* differed from it — jdbgen's engine is the
specification, not its manual. Everything corrected that way is marked
**Corrected**; the three deliberate behaviour changes are marked
**Changed**.

[← Documentation index](README.md)

- [Quick example](#quick-example)
- [Syntax overview](#syntax-overview)
- [`item` statement](#item-statement)
- [The model: table, column, key and index fields](#the-model)
- [Control statements](#control-statements)
- [Other statements](#other-statements)
- [Custom variables](#custom-variables)
- [Abbreviations](#abbreviations)
- [Error handling](#error-handling)
- [Recipes](#recipes)

## Quick example

Every example on this page uses the same table. It is the one in the sample H2
database that ships beside the program (`sample/sample_h2.db.mv.db`):

```sql
create table t_sample_album (
  album_id int not null,
  album_name varchar(256) not null,
  artist_name varchar(512) not null,
  publish_date DATE,
  primary key (album_id)
);
comment on table t_sample_album is 'Music Album';
comment on column t_sample_album.album_id is 'Album identifier';
comment on column t_sample_album.album_name is 'Album display name';
comment on column t_sample_album.artist_name is 'Creator artist name';
comment on column t_sample_album.publish_date is 'Published date';
```

With this template:

```java
/**
 * ${remarks} Model class
 *
 * @author ${author}
 * @version 1.0 ${date:yyyy-MM-dd}
 */
class ${table.suffix.pascal}Model {
    ${for:item=columns}// ${remarks}
    private ${item:key=javaType, padSize=10, padDir=right} ${name.camel};
    ${endfor}
    // Getters and Setters
    ${for:item=columns}
    // get ${remarks}
    public ${javaType} ${if:item=javaType, equals='boolean'}is${else}get${endif}${name.pascal}() {
        return ${name.camel};
    }

    // set ${remarks}
    public void set${name.pascal}(${javaType} ${name.camel}) {
        this.${name.camel} = ${name.camel};
    }
    ${endfor}
}
```

rudbgen generates:

```java
/**
 * Music Album Model class
 *
 * @author John Doe
 * @version 1.0 2026-08-23
 */
class SampleAlbumModel {
    // Album identifier
    private Integer    albumId;
    // Album display name
    private String     albumName;
    // Creator artist name
    private String     artistName;
    // Published date
    private Date       publishDate;
    
    // Getters and Setters
    ...
}
```

Note the lines that contain nothing but the four spaces preceding `${endfor}`:
the body of a `for` loop is copied out exactly as written, line breaks and
indentation included. See [`for` statement](#for-statement) for the details, and
[Recipes](#recipes) for layouts that avoid stray blank lines.

The **Preview** tab renders exactly this — pick a table and a template from its
two dropdowns — and **Dry run** renders every pair without writing anything. Both
are on the status bar; see the [UI guide](ui-guide.md#preview-and-dry-run).

## Syntax overview

A statement starts with `${` and ends at the **first** `}` after it. Everything
outside statements is copied to the output byte for byte.

```
${<statement type>:<name>=<value>[, <name>=<value> ...]}
${<field name>[<processors>]}
```

| Statement | Form | Purpose |
|:---|:---|:---|
| [`item`](#item-statement) | `${<field>[<processors>]}`<br>`${item:key=<field>[<processors>][, <decorations>]}` | Insert a table field, a column field or a custom variable |
| [`super`](#super) | `${super:key=<table field>[<processors>][, <decorations>]}` | Reach the enclosing model from inside a `for` loop |
| [`if`](#if-statement) | `${if:item=<field>, <condition>}` … `${elif:…}` … `${else}` … `${endif}` | Branch on a field value |
| [`for`](#for-statement) | `${for:item=<collection field>[, <controls>]}` … `${endfor}` | Repeat over a collection |
| [`author`](#author-user-and-date) | `${author[:<decorations>]}` | The **Author** field of the Generate tab |
| [`user`](#author-user-and-date) | `${user[:<decorations>]}` | The OS login user id |
| [`date`](#author-user-and-date) | `${date[:<format>]}`, `${date[:format=<format>[, <decorations>]]}` | The current date |
| [Text escape](#text-escape) | `${'any text'}`, `${"any text"}` | Emit text that itself contains `${` or `}` |

A statement without a `:` is shorthand for `item:key=`, so `${name.camel}` and
`${item:key=name.camel}` are the same thing.

### Whitespace, quoting and escapes

- Whitespace inside a statement is insignificant, including line breaks. Both
  `${item:key=name}` and

  ```
  ${
    item : key = name , padSize = 15
  }
  ```

  behave identically — the line breaks inside the braces do not reach the
  output. Long `for` headers in the shipped templates use this to stay readable.
- Attribute values may be quoted with `'` or `"`. Exactly one matching pair of
  surrounding quotes is removed, so `equals='TABLE'`, `equals="TABLE"` and
  `equals=TABLE` are equivalent. Quote a value when it contains a space, a comma
  or an `=`. A `replace(...)` argument list keeps its parentheses.
- Inside an attribute value, `\n`, `\r` and `\t` become the real control
  characters, and `\<any other char>` becomes that character. This is what makes
  `inStr="\n,"` work.

  > **Note**
  > A [text escape](#text-escape) statement uses a *different* rule: there,
  > `\<char>` always yields the character itself, so `${'a\nb'}` prints `anb`,
  > not `a`, newline, `b`.

- A `}` inside a quoted attribute value still ends the statement, because the
  engine looks for the first `}` without tracking quotes. Use a text escape
  statement to emit a literal `}`.

### Case sensitivity

| Element | Case |
|:---|:---|
| Statement names (`item`, `super`, `if`, `for`, `date`, `user`, `author`) | Ignored — `${IF:ITEM=type, EQUALS='TABLE'}` works |
| Attribute names (`key`, `item`, `padSize`, `inStr`, `indent`, …) | Ignored |
| Processor names (`.suffix`, `.pascal`, …) | Ignored — `${name.SUFFIX.PASCAL}` works |
| Field names (`remarks`, `javaType`, …) | **Respected** (**Corrected**, see below) |
| `endif`, `endfor`, `else`, `elif:` | **Lower case only** |

> **Corrected**
> jdbgen resolved field names through Java reflection, which made them look
> case-insensitive. rudbgen's model answers a fixed set of names, spelled as the
> tables below spell them: `${javaType}` works, `${javatype}` does not — it is
> an unknown field, so it warns and renders empty. The shipped templates and
> every jdbgen template seen in the wild use the documented spelling, so this
> only bites a template that was relying on the accident.

> **Note**
> `${ENDIF}` and `${ENDFOR}` are not recognised as terminators, so the statement
> stays open and parsing fails with `if statements not closed` /
> `for statements not closed`. `${ELSE}` is worse: it is silently parsed as an
> `item` statement for a field named `ELSE`, which resolves to an empty string,
> so the `else` branch simply disappears without an error. Always write these
> four in lower case.

### Escaping `${`

Anything a template must emit literally — including `${` and `}` — goes into a
text escape statement:

| Template | Output |
|:---|:---|
| `${'Test sample with ${author}'}` | `Test sample with ${author}` |
| `${"literal ${author} and }"}` | `literal ${author} and }` |

## `item` statement

`item` inserts the value of a field of the model the template is currently
rendered against — the table, or the column when inside a
[`for` loop](#for-statement).

```
${<field name>[<processors>]}
${item:key=<field name>[<processors>][, <decorations>]}
${item:item=<field name>…}
```

`key=` and `item=` are interchangeable in **every** statement that takes a field
name (`item`, `super`, `if` and `for`). The shorthand form cannot take
decorations; use the `item:key=` form for those.

> **Note**
> The shorthand form must not contain a `:`, because the engine splits the
> statement at the first colon to find the statement type.
> `${name.replace('_',':')}` fails with `Unknown template: …`; write
> `${item:key=name.replace('_',':')}` instead.

### Processors

Processors are appended to the field name with a `.` and are applied left to
right, each one receiving the previous one's result.

> **`${a.b}` is a chain, never a path.** The dot does not walk into a nested
> object. `${fk.table}` does not read the `table` of the `fk` object; it reads
> the field `fk` and then looks for a processor called `table`, which does not
> exist, and fails. Walk into a sub-object with a [`for` loop](#for-statement)
> — that is what `fk`, `imports`, `exports` and `indexes` are for.

| Processor | Description |
|:---|:---|
| `.abbr` | Apply the [abbreviation rules](#abbreviations) to the value |
| `.suffix` | Remove everything up to and including the **first** `_` (`T_SAMPLE_ALBUM` → `SAMPLE_ALBUM`) |
| `.prefix` | Remove everything from the **last** `_` onwards (`SAMPLE_ALBUM_T` → `SAMPLE_ALBUM`) |
| `.camel` | Camel case (`SAMPLE_ALBUM` → `sampleAlbum`) |
| `.pascal` | Pascal case (`SAMPLE_ALBUM` → `SampleAlbum`) |
| `.snake` | Snake case (`SAMPLE_ALBUM` → `sample_album`) |
| `.screaming` | Screaming snake case (`sampleAlbum` → `SAMPLE_ALBUM`) |
| `.skewer` | Skewer case (`SAMPLE_ALBUM` → `sample-album`) |
| `.kebab` | Alias of `.skewer` |
| `.lower` | Lower case |
| `.upper` | Upper case |
| `.replace(<find>, <replacement>)` | Replace every occurrence of `<find>` with `<replacement>` |

`.suffix` and `.prefix` return the value **unchanged** when it contains no `_`,
so `${type.suffix}` on a `TABLE` type yields `TABLE`.

`.replace` takes its two arguments quoted, unquoted or mixed:
`${name.replace('SAMPLE','TEST')}`, `${name.replace(_, -)}` and
`${name.replace(ghi, 'xyz')}` all work.

Any processor may be repeated and combined with any other — the chain is a plain
pipeline with no restrictions. `${name.upper.lower}` and
`${name.replace('SAMPLE','TEST').suffix.pascal}` are both valid.

Examples on `T_SAMPLE_ALBUM`:

| Template | Output |
|:---|:---|
| `${name.suffix}` | `SAMPLE_ALBUM` |
| `${name.prefix}` | `T_SAMPLE` |
| `${name.suffix.pascal}` | `SampleAlbum` |
| `${name.suffix.camel}` | `sampleAlbum` |
| `${name.suffix.kebab}` | `sample-album` |
| `${name.replace('SAMPLE','TEST').suffix.pascal}` | `TestAlbum` |
| `${item:key=name.suffix.camel.screaming}` | `SAMPLE_ALBUM` |

### Decorations

Decorations are comma-separated attributes of the `item:key=` form. They also
work on [`super`](#super) and on [`author`, `user` and
`date`](#author-user-and-date).

| Decoration | Description |
|:---|:---|
| `padSize=<size>` | Pad the value with spaces up to `<size>` display columns |
| `padDir=<direction>` | `left` or `right` — the side the spaces go to. Default `right` |
| `quote=<quote>` | Wrap the value in `<quote>` on both sides |
| `prepend=<text>` | Put `<text>` in front of the value |
| `postpend=<text>` | Put `<text>` behind the value |

Order of application:

1. `prepend` (or `quote`) is put in front, `postpend` (or `quote`) behind.
   `prepend`/`postpend` **override** `quote` on their side, so
   `${item:key=type, quote='"', prepend='<'}` yields `<TABLE"`.
2. Padding is then computed on the **already decorated** string — quotes count
   towards `padSize`.

| Template | Output |
|:---|:---|
| `[${item:key=name.suffix, padSize=20, quote="'", padDir=right}]` | `['SAMPLE_ALBUM'      ]` |
| `[${item:key=type, padSize=10, padDir=left}]` | `[     TABLE]` |
| `[${item:key=type, padSize=10}]` | `[TABLE     ]` |
| `[${item:key=name, padSize=3}]` | `[T_SAMPLE_ALBUM]` |

> **Changed — padding counts display columns, not EUC-KR bytes.**
> jdbgen measured `padSize` and the `for` indentation base in EUC-KR bytes.
> rudbgen measures **display columns**, the wcwidth-compatible width a
> fixed-width font actually draws (architecture document §7.4): a wide character
> counts two, a zero-width character counts none.
>
> On everything the shipped templates and the golden fixtures contain — ASCII,
> Hangul, Hanja — the two measures agree exactly, which is what keeps the
> byte-identity canary green. They part on characters EUC-KR happens to store in
> two bytes but a terminal draws in one (Cyrillic, Greek, box drawing), where
> jdbgen over-counted, and on characters EUC-KR cannot encode at all (emoji,
> most of CJK Extension B), where jdbgen's count was a guess. The number rudbgen
> uses is the one the generated file lines up by.
>
> `[${item:key=remarks, padSize=10}]` on a table commented `음반` still produces
> `[음반      ]` — two characters, four columns, six spaces.

A value longer than `padSize` is never truncated.

An unknown or empty field still goes through the decorations, because it is
resolved to an empty string first: `[${item:key=nvlColName, padSize=8,
quote='#'}]` produces `[##      ]`.

### Resolution order

For every `item` (and `super`) statement the engine looks for the name in this
order:

1. a field of the current model (table, column, foreign key, index — see
   [The model](#the-model)),
2. a [custom variable](#custom-variables),
3. otherwise it records a warning and substitutes an **empty string**.

An unknown *field* therefore never stops generation; an unknown *processor*
does. See [Error handling](#error-handling).

Every warning of step 3 carries the line it came from, and the result summary of
a run counts them per table × template. The template tab marks them in the gutter
as you type, against the table the live preview is rendering — which is the
fastest way to find a misspelled field. See the [user interface
guide](ui-guide.md#the-template-tab).

## The model

### Table fields

| Field | Type | Description |
|:---|:---|:---|
| `catalog` | text | Database catalog containing this table |
| `schema` | text | Database schema containing this table |
| `name` | text | Table name |
| `table` | text | Alias of `name` |
| `title` | text | Alias of `name` |
| `type` | text | `TABLE` or `VIEW` |
| `remarks` | text | Table comment |
| `columns` | list of column | All columns, in ordinal order |
| `keys` | list of column | Primary key columns, **in the order the key was declared in** |
| `notKeys` | list of column | All columns except the primary keys, in ordinal order |
| `imports` | list of foreign key | Foreign keys **from** this table to others (D8, new) |
| `exports` | list of foreign key | Foreign keys **from** other tables to this one (D8, new) |
| `indexes` | list of index | Indexes of this table (D8, new) |
| `icon` | text | The icon key the explorer uses (`fa:TABLE` or `fa:EYE`) |
| `no` | int | 1-based position of this table in the generation run (**Corrected**) |
| `source` | text | Declared for jdbgen compatibility — **always empty** |

> **Corrected — `no` on a table.**
> jdbgen's documentation says a table's `no` is always `0` because nothing
> assigned it. rudbgen numbers the tables of a run from 1, so `${no}` outside a
> loop is the table's position in the list you ticked.

> **Note**
> The table comment field is `remarks`, **not** `remark`. `${remark}` is not a
> field, so it warns and expands to an empty string.

> **`keys` is in key order, not column order.** A composite key declared
> `primary key (B, A)` lists `B` first, because a `WHERE B = ? AND A = ?`
> generated from it has to match the declaration.

### Column fields

| Field | Type | Description |
|:---|:---|:---|
| `catalog` | text | Database catalog containing this column |
| `schema` | text | Database schema containing this column |
| `table` | text | Table this column belongs to |
| `name` | text | Column name |
| `column` | text | Alias of `name` |
| `typeName` | text | Database type name as the driver reports it (`VARCHAR`, `INT`, …) |
| `typeString` | text | `typeName` in upper case with `(<length>)` appended for `CHAR`/`BINARY` types (`VARCHAR(256)`). Lengths above 1,000,000 render as `(max)` |
| `length` | int | `COLUMN_SIZE` |
| `precision` | int | `COLUMN_SIZE` again, under the name a numeric column wants (D8, new) |
| `scale` | int | `DECIMAL_DIGITS`; `0` for a type that has none (D8, new) |
| `nullable` | int | `0` not nullable, `1` nullable, `2` unknown |
| `isKey` | bool | `true` when the column is part of the primary key |
| `key` | bool | Alias of `isKey` |
| `keySeq` | int or empty | 1-based position inside the primary key; empty when the column is not part of it (D8, new) |
| `isCharType` | bool | `true` when `typeName` contains `CHAR`, `CLOB` or `TEXT` |
| `charType` | bool | Alias of `isCharType` |
| `autoIncrement` | bool | `true` when the database fills this column in itself (D8, new) |
| `remarks` | text | Column comment |
| `defVal` | text | Default value (`COLUMN_DEF`) |
| `dataType` | int | Raw `java.sql.Types` constant (`4`, `12`, `91`, …) |
| `jdbcType` | text | JDBC type name derived from `dataType` |
| `javaType` | text | Java type name derived from `dataType` |
| `fk` | list of 0 or 1 reference | What this column points at, when exactly one foreign key uses it (D8, new) |
| `no` | int | 1-based position, see [`for` statement](#for-statement) |
| `nvlColName` | text | Declared for jdbgen compatibility — **always empty** |

`key` and `charType` are jdbgen's accidental second spellings — its reflection
tried `is<Property>()` last, so `${key}` found `isKey()`. Templates exist that
use both, so both answer.

For the sample table:

| Template | Output |
|:---|:---|
| `${remarks}` | `Music Album` |
| `${name}` / `${table}` / `${type}` | `T_SAMPLE_ALBUM` / `T_SAMPLE_ALBUM` / `TABLE` |
| `${for:item=columns, inStr=","}${name}:${javaType}:${jdbcType}:${typeString}${endfor}` | `ALBUM_ID:Integer:INTEGER:INT,ALBUM_NAME:String:VARCHAR:VARCHAR(256),ARTIST_NAME:String:VARCHAR:VARCHAR(512),PUBLISH_DATE:Date:DATE:DATE` |

### Foreign key fields (D8, new)

`imports` and `exports` are lists of these. Loop over them with `for`.

| Field | Type | Description |
|:---|:---|:---|
| `name` | text | Constraint name |
| `columns` | list of key column | The columns of *this* table the key is made of, in key order |
| `refCatalog` | text | Catalog of the table at the other end |
| `refSchema` | text | Schema of the table at the other end |
| `refTable` | text | The table at the other end |
| `refColumns` | list of key column | The columns of that table, aligned with `columns` |
| `onUpdate` | text | `CASCADE`, `RESTRICT`, `SET NULL`, `NO ACTION`, `SET DEFAULT`, or empty |
| `onDelete` | text | As `onUpdate` |
| `no` | int | 1-based position in the list |

A **key column** — the element of `columns`, `refColumns` and an index's
`columns` — has just two fields, `name` and `no`. It is a model rather than a
bare string so that `${for}` can walk it and `${name}` read it.

### Index fields (D8, new)

| Field | Type | Description |
|:---|:---|:---|
| `name` | text | Index name |
| `unique` | bool | Whether it is a unique index |
| `columns` | list of key column | Its columns, in index order |
| `no` | int | 1-based position in the list |

### The `fk` of a column (D8, new)

`fk` is the foreign key this one column takes part in, when exactly one uses it
— the convenience a navigation property wants, so that a template does not have
to walk `imports` looking for the column. A column inside a composite key gets
the key it takes part in; a column used by two different foreign keys gets
**nothing**, because there is no single answer to give.

It is reached with a loop, because a `.` is a
[processor chain and not a path](#processors):

```java
${for:item=fk}    private ${table.suffix.pascal} ${table.suffix.camel};   // -> ${table}.${column}
${endfor}
```

Its fields are `name` (the constraint), `catalog`, `schema`, `table` and
`column` — the table and column it points at.

> **Known limitation.** A column that has no foreign key answers `fk` as *null*
> rather than as an empty list, and a `for` over a null fails the render with
> `'fk' is not a collection but Null`. There is no `if` that guards it either:
> `contains` on a null fails the same way. So today `${for:item=fk}` is only
> safe on a table where **every** column has exactly one foreign key, which in
> practice means it is not usable. Loop over `${imports}` instead — that list is
> always a list, empty when there is none — and match on `${name}` inside it.
> The one-line fix (answering an empty list) is tracked in `docs/status.md`;
> when it lands, the loop above starts working and no template that used
> `imports` breaks.

### JDBC and Java type mapping

`jdbcType` and `javaType` are looked up from the numeric `DATA_TYPE` the JDBC
driver reports (`dataType`), *not* from `typeName`. The table is jdbgen's,
carried over unchanged — including its oddities — because changing it would
change the output of every template written against it.

| `java.sql.Types` | `dataType` | `jdbcType` | `javaType` |
|:---|---:|:---|:---|
| `ARRAY` | 2003 | `ARRAY` | `array` |
| `BIGINT` | -5 | `BIGINT` | `Long` |
| `BINARY` | -2 | `BINARY` | `byte[]` |
| `BIT` | -7 | `BIT` | `Boolean` |
| `BLOB` | 2004 | `BLOB` | `byte[]` |
| `BOOLEAN` | 16 | `BOOLEAN` | `Boolean` |
| `CHAR` | 1 | `CHAR` | `String` |
| `CLOB` | 2005 | `CLOB` | `String` |
| `DATALINK` | 70 | `DATALINK` | `String` |
| `DATE` | 91 | `DATE` | `Date` |
| `DECIMAL` | 3 | `DECIMAL` | `Integer` |
| `DISTINCT` | 2001 | `DISTINCT` | `String` |
| `DOUBLE` | 8 | `DOUBLE` | `Double` |
| `FLOAT` | 6 | `FLOAT` | `Float` |
| `INTEGER` | 4 | `INTEGER` | `Integer` |
| `JAVA_OBJECT` | 2000 | `JAVA_OBJECT` | `String` |
| `LONGNVARCHAR` | -16 | `LONGNVARCHAR` | `String` |
| `LONGVARBINARY` | -4 | `LONGVARBINARY` | `byte[]` |
| `LONGVARCHAR` | -1 | `LONGVARCHAR` | `String` |
| `NCHAR` | -15 | `NCHAR` | `String` |
| `NCLOB` | 2011 | `NCLOB` | `String` |
| `NULL` | 0 | `NULL` | `null` |
| `NUMERIC` | 2 | `NUMERIC` | `Integer` |
| `NVARCHAR` | -9 | `NVARCHAR` | `String` |
| `OTHER` | 1111 | `OTHER` | `String` |
| `REAL` | 7 | `REAL` | `Float` |
| `REF` | 2006 | `REF` | `ref` |
| `ROWID` | -8 | `ROWID` | `Integer` |
| `SMALLINT` | 5 | `SMALLINT` | `Short` |
| `SQLXML` | 2009 | `SQLXML` | `String` |
| `STRUCT` | 2002 | `STRUCT` | `struct` |
| `TIME` | 92 | `TIME` | `Time` |
| `TIMESTAMP` | 93 | `TIMESTAMP` | `String` |
| `TINYINT` | -6 | `TINYINT` | `Short` |
| `VARBINARY` | -3 | `VARBINARY` | `byte[]` |
| `VARCHAR` | 12 | `VARCHAR` | `String` |

> **Note**
> The Java mapping has sharp edges to work around in your templates:
>
> - `TIMESTAMP` maps to `String`, not to a date type. `DATE` maps to `Date` and
>   `TIME` to `Time`, neither qualified with a package.
> - `DECIMAL` and `NUMERIC` map to `Integer`, ignoring precision and scale, so a
>   `NUMERIC(18,2)` column produces `Integer`. `precision` and `scale` are new
>   fields precisely so a template can do better: branch on `${scale}`.
> - `TINYINT` maps to `Short`, `ROWID` to `Integer`.
> - `ARRAY`, `REF`, `STRUCT` and `NULL` map to the lower-case words `array`,
>   `ref`, `struct` and `null`, which are not Java types at all.
>
> Codes outside the table — `TIME_WITH_TIMEZONE` (2013),
> `TIMESTAMP_WITH_TIMEZONE` (2014), `REF_CURSOR` (2012) and vendor-specific
> codes — have no mapping, so both `${jdbcType}` and `${javaType}` expand to an
> empty string.
>
> Where the mapping does not suit you, branch on `typeName` or `dataType`:
> `${if:item=typeName, startsWith='timestamp'}LocalDateTime${else}${javaType}${endif}`.

## Control statements

### `if` statement

```
${if:item=<field>[<processors>], <condition>[, <condition> ...]}
 ...                                   // every condition met
[${elif:item=<field>, <condition>[, <condition> ...]}]
 ...                                   // another one met, repeatable
[${else}]
 ...                                   // nothing matched
${endif}
```

`item=` and `key=` are interchangeable, and the field name may carry
[processors](#processors): `${if:item=name.suffix.camel, startsWith='sample'}`.
Multiple conditions in one `if` are combined with **AND**. `elif` may be
repeated, `else` is optional, and `endif` is mandatory.

| Condition | True when the value … |
|:---|:---|
| `equals=<value>` / `value=<value>` | equals `<value>` |
| `notEquals=<value>` | does not equal `<value>` |
| `startsWith=<prefix>` | starts with `<prefix>` |
| `notStartsWith=<prefix>` | does **not** start with `<prefix>` |
| `endsWith=<suffix>` | ends with `<suffix>` |
| `notEndsWith=<suffix>` | does **not** end with `<suffix>` |
| `contains=<value>` | is a collection holding an element whose `name` is `<value>`, or is a string equal to one of the comma-separated tokens of `<value>` |
| `notContains=<value>` | the negation of `contains` |
| `matches=<regex>` | matches the regular expression `<regex>` **entirely** |
| `notMatches=<regex>` | does not match `<regex>` entirely |

#### Case sensitivity of conditions

| Condition | Case | Matching |
|:---|:---|:---|
| `equals`, `value`, `notEquals` | **ignored** | whole value |
| `startsWith`, `notStartsWith` | **ignored** | prefix |
| `endsWith`, `notEndsWith` | **ignored** | suffix |
| `contains`, `notContains` | **ignored** | collection: each element's `name`; string: each `,`-separated token, compared whole |
| `matches`, `notMatches` | **respected** | whole value, anchored at both ends |
| `skipList` of [`for`](#for-statement) | **respected** | exact, whole element name |

> **Note**
> `matches` anchors at both ends. `matches='SAMPLE'` does **not** match
> `T_SAMPLE_ALBUM`; write `matches='.*SAMPLE.*'`. And unlike every other
> condition, it is case-sensitive: `matches='t_sample_album'` does not match
> `T_SAMPLE_ALBUM` either.
>
> The dialect is Rust's `regex` crate rather than `java.util.regex`. The two
> agree on everything a template realistically writes; what Rust's has not got
> is backreferences and lookaround, which it refuses at parse time rather than
> matching slowly. A pattern that does not compile is reported when the
> condition is evaluated.

For a string value, `contains` reads as *"is one of"*:
`${if:item=type, contains='TABLE,VIEW'}` is true when `type` is exactly `TABLE`
or exactly `VIEW`. Tokens are trimmed, so `'TABLE, VIEW'` works too.

#### Multiple conditions

Conditions are stored under their name, so **the same condition name twice keeps
only the last one**:

```
${if:item=type, equals='VIEW', equals='TABLE'}A${else}B${endif}
```

evaluates only `equals='TABLE'` and prints `A` for a table. Use `elif` or
`contains='VIEW,TABLE'` when you mean "or".

Any attribute name that is neither a condition, `key` nor `item` is rejected
while parsing — `if` accepts no [decorations](#decorations):

```
${if:item=type, weird='x'}A${endif}
→ parse error: Unknown if condition: item=type, weird='x'
```

#### Examples

| Template | Output |
|:---|:---|
| `${if:item=type, equals='table'}YES${else}NO${endif}` | `YES` (case is ignored) |
| `${if:item=type, value='TABLE'}YES${endif}` | `YES` |
| `${if:item=type, equals='VIEW'}V${elif:item=type, equals='TABLE'}T${else}X${endif}` | `T` |
| `${if:item=name, startsWith='t_'}YES${endif}` | `YES` |
| `${if:item=columns, contains='album_id'}HAS${else}NO${endif}` | `HAS` |
| `${if:item=name, matches='SAMPLE'}FULL${else}NOTFULL${endif}` | `NOTFULL` |
| `${if:item=name, matches='.*SAMPLE.*'}M${else}N${endif}` | `M` |

### `for` statement

```
${for:item=<collection field>[, <controls>]}
 ...    // repeated once per element
${endfor}
```

Inside the loop the current model is the **element**, so `${name}` is the column
name. The enclosing model is still reachable through [`super`](#super).

| Control | Description |
|:---|:---|
| `inStr=<separator>` | Text inserted **between** iterations (not before the first or after the last) |
| `indent=<spaces>` | Integer, may be negative — adjusts the indentation applied to `inStr` fragments |
| `skipList=<names>` | Comma-separated element names to skip; matched case-**sensitively**, whole name, tokens trimmed |

`item=` and `key=` are interchangeable here too. The collection field is any
field whose value is a list:

| On | Collection fields |
|:---|:---|
| table | `columns`, `keys`, `notKeys`, `imports`, `exports`, `indexes` |
| column | `fk` |
| foreign key | `columns`, `refColumns` |
| index | `columns` |

A field that exists but is not a list (`${for:item=type}`) fails with
`'type' is not a collection`; a field that does not exist at all fails with
`Model has no '<name>' member`. Both name the line.

#### `${no}` — the iteration counter

At every iteration the engine hands the element its 1-based position, so `${no}`
numbers the loop:

| Template | Output |
|:---|:---|
| `${for:item=columns, inStr=","}${no}:${name}${endfor}` | `1:ALBUM_ID,2:ALBUM_NAME,3:ARTIST_NAME,4:PUBLISH_DATE` |
| `${for:item=notKeys, inStr=","}${no}:${name}${endfor}` | `1:ALBUM_NAME,2:ARTIST_NAME,3:PUBLISH_DATE` |

The number is the position inside the collection **being iterated**, so in a
`notKeys` loop `ALBUM_NAME` is `1`, even though it is the second column of the
table.

> **Corrected — `${no}` counts rendered elements, so `skipList` leaves no gaps.**
> jdbgen's documentation claims the counter is assigned before `skipList` is
> applied and that skipping therefore produces `1,3,4`. The engine does not do
> that, and neither does this port: the counter advances only for an element
> that is actually rendered.
>
> `${for:item=columns, inStr=',', skipList='ALBUM_NAME'}${no}:${name}${endfor}`
> yields `1:ALBUM_ID,2:ARTIST_NAME,3:PUBLISH_DATE`.
>
> This is what a template wants — a numbered list with a hole in it is not a
> numbered list — and it is what a `?1, ?2, ?3` parameter list needs to be
> correct.

> **Changed — a model with no `no` of its own.**
> The counter travels beside the model rather than being written into it, so
> that the metadata tree stays shareable. The only visible difference is for a
> model that has no `no` field at all — a custom-variable map used as a loop
> element — where jdbgen rendered nothing and rudbgen renders the position.

#### Indentation — read this before laying out a loop

`indent` does **not** re-indent the loop body. It only affects line breaks that
are part of `inStr`.

- Line breaks *inside the loop body* are emitted exactly as they appear in the
  template. No indentation is added and none is removed.
- Each line break *inside `inStr`* is normalised to the template's own line
  ending — rudbgen decides between `\r\n` and `\n` by looking at the **first**
  line break in the template file, so a CRLF template keeps producing CRLF — and
  the fragment after it is indented to
  `<output column where the for statement started> + <indent>`. The column is
  measured in display columns (see [Decorations](#decorations)).
- `indent` defaults to `0`, and a non-numeric value fails the render with
  `indent is not a number`.

This is why the idiomatic layout puts the whole body on the same line as the
`for` header and moves the line break into `inStr`:

```sql
SELECT  ${for:item=columns, inStr="\n,", indent=-1}${column} AS "${name.camel}"${endfor}
```

```sql
SELECT  ALBUM_ID AS "albumId"
       ,ALBUM_NAME AS "albumName"
       ,ARTIST_NAME AS "artistName"
       ,PUBLISH_DATE AS "publishDate"
```

The `for` starts at output column 8, `indent=-1` pulls the continuation lines
back to column 7, and the `,` of `inStr` lands under the second space of
`SELECT  `. The same pattern with `inStr="\nAND ", indent=-4` produces a
`WHERE` clause:

```sql
 WHERE ${for:item=keys, inStr="\nAND ", indent=-4}${column} = #{${name.camel}}${endfor}
```

```sql
 WHERE ALBUM_ID = #{albumId}
   AND TRACK_NO = #{trackNo}
```

> **Note**
> Writing the body on its own line, as in
> `SELECT ${for:item=columns, inStr=","}` ⏎ `       ${name}…${endfor}`, keeps the
> leading line break of the body in the output — the generated statement starts
> with an empty line after `SELECT`, and the closing `${endfor}` line contributes
> its own indentation. It is not an error, but it is rarely what you want.

Since line breaks inside a statement are insignificant, a long `for` header can
be broken up without affecting the output. The shipped MyBatis template uses
this to keep an `INSERT` readable:

```xml
               ${for:item=columns,inStr="\n,",indent=-1
               }${
               if:item=name,endsWith="date"
               }CURRENT_DATE()${
               else
               }#{${name.camel}}${
               endif}${
               endfor}
```

## Other statements

### `super`

```
${super:key=<field>[<processors>][, <decorations>]}
```

Inside a `for` loop, `super` resolves against the **enclosing** model instead of
the current element:

| Template | Output |
|:---|:---|
| `${for:item=keys}${super:key=name.suffix.pascal}.${name.camel}${endfor}` | `SampleAlbum.albumId` |
| `${for:item=keys}${super:key=remarks}${endfor}` | `Music Album` |

Nesting is one level: `super` always means "one loop out". In
`${for:item=imports}${for:item=columns}…` the inner `super` is the foreign key,
not the table — there is no way to reach two levels up, so read what you need
from the table before the outer loop, or into a custom variable.

Used **outside** a loop, `super` is not an error — it falls back to
[custom variables](#custom-variables) and, failing that, to an empty string.

> **Note**
> There is no `super` version of `if`. Inside a `for` loop you cannot branch on a
> table field directly; `${if:super=type, …}` is rejected as an unknown `if`
> condition. Move the `if` outside the loop, or duplicate the loop in both
> branches.

### `author`, `user` and `date`

| Statement | Value |
|:---|:---|
| `${author}` | The **Author** field of the Generate tab |
| `${user}` | The OS login user id |
| `${date}` | The current date |

All three accept [decorations](#decorations):

| Template | Output |
|:---|:---|
| `${author:quote='"'}` | `"John Doe"` |
| `${user:prepend='@'}` | `@comart` |

`date` takes a Java `SimpleDateFormat` pattern in either of two ways:

| Template | Meaning |
|:---|:---|
| `${date}`, `${date:}` | Default format `yyyy-MM-dd` |
| `${date:yyyy/MM/dd HH:mm}` | Shorthand — the whole attribute string is the format |
| `${date:format=yyyy/MM/dd}` | Explicit form |
| `${date:format=yyyy, quote='!'}` | Explicit form with decorations → `!2026!` |

> **Note**
> The shorthand and decorations cannot be mixed, and the shorthand must not
> contain a `,` or an `=`. `${date:yyyy, quote='!'}` and
> `${date:EEE, d MMM yyyy}` both fail with `Name value pair not matched`. Use
> the explicit form and quote the pattern:
> `${date:format='EEE, d MMM yyyy'}`.

> **Changed — month and weekday names are always English.**
> The pattern letters are reproduced; the locale is not. `MMMM` renders
> `August` whatever the machine's language and whatever the interface language
> is set to, where Java followed the default locale. Every shipped template uses
> numeric fields only, where the two agree exactly. If a localised month name
> matters, put it in a custom variable.

### Text escape

```
${'any text'}
${"any text"}
```

The text is emitted verbatim, `${` and `}` included. A `\` makes the next
character literal, which is how you embed the closing quote.

| Template | Output |
|:---|:---|
| `${'Test sample with ${author}'}` | `Test sample with ${author}` |
| `${'It\'s a test'}` | `It's a test` |
| `${'a\\b'}` | `a\b` |
| `${"say \"hi\""}` | `say "hi"` |
| `${'a\nb'}` | `anb` |
| `${''}` | (nothing) |

> **Note**
> Two traps. First, `\n` here is **not** a line break — the backslash simply
> escapes the `n` (attribute values behave the opposite way, see
> [Whitespace, quoting and escapes](#whitespace-quoting-and-escapes)). Second,
> anything between the closing quote and the `}` is silently discarded:
> `${'abc' ignored }` prints `abc`.

## Custom variables

Custom variables are the name/value pairs of the Generate tab. They are stored
per connection, alongside the output directory and the template ticks.

They are used exactly like fields, processors included:

```
${package}                        → com.abc.sample
${package.replace('.','/')}       → com/abc/sample
```

Points to keep in mind:

- **Fields win.** The lookup order is model field first, custom variable
  second. A variable named `name` or `remarks` is shadowed by the table and
  column fields of the same name and can never be read.
- **`author`, `user` and `date` are statement names.** `${date}` is always the
  date statement, never your variable. The `item` form still reaches the
  variable: `${item:key=date}`.
- `${author}` *is* a custom variable — the Generate tab stores its **Author**
  field under that name — which is why it also answers to `${item:key=author}`.
- Custom variables apply to the **output file name template** as well as to the
  template body. The shipped set uses `${name.suffix.pascal}Model.java`,
  `${name.suffix.camel}-mapper.xml` and `${name.suffix.lower}_ci_model.php`,
  which for the sample table produce `SampleAlbumModel.java`,
  `sampleAlbum-mapper.xml` and `sample_album_ci_model.php`. A file-name template
  such as `${package.replace('.','/')}/${name.suffix.pascal}.java` writes into a
  package directory tree; the directories are created for you.
- An undefined name is not an error: it warns and expands to an empty string.

The inspector lists the same fields the model offers for the table under the
cursor, which is the quickest way to check a name you are unsure of. **Next
release**: with a template tab open the inspector becomes a *variable palette*
proper — every name including your custom variables, clickable to insert `${…}`
at the caret.

## Abbreviations

Abbreviation rules live in `abbreviations.json` in your [configuration
directory](installation.md#where-rudbgen-keeps-your-data). Each replaces a word —
or a whole name — with a replacement string, and each can be turned off without
being deleted. The `.abbr` processor applies them in two steps:

1. **Whole-name rules** are looked up with the value lower-cased. On a hit, the
   entire value is replaced and processing stops.
2. Otherwise the value is split on `_` and `-`, and each word is looked up in
   the word rules. The separators stay where they were.

> **Changed — D10: word rules match ignoring case.**
> jdbgen stored rule keys lower-cased but looked words up **as written**, so a
> word rule never fired on an upper-case identifier — which is what database
> identifiers usually are. With a rule `usr` → `user`, jdbgen left `TB_USR`
> alone and only `tb_usr` was rewritten.
>
> rudbgen lower-cases both sides: `TB_USR` becomes `TB_user`, and `Usr` becomes
> `user`. Only the *matching* changed; the replacement is inserted exactly as
> the rule spells it, so control the case of the output through the rule and
> through the case processors after it (`${name.abbr.pascal}`).
>
> It is the one change that can alter the output of a template that used to
> work, which is why the jdbgen import reports it whether or not the file it
> read holds a single rule.

The Generate tab has a switch, **apply abbreviations**. When it is on, `.abbr`
is inserted automatically as the **first** processor of every `item` reference
whose first key is `name` — you do not have to write `.abbr` at all, and
`${name.lower}` silently becomes `${name.abbr.lower}`. Aliases are not covered:
`${table}` and `${column}` are never abbreviated.

The **Rules…** button beside the switch opens the [rules
editor](ui-guide.md#abbreviation-rules), which is where the list is normally
kept: four columns, a trailing empty row, a table-name picker for the whole-name
rows, and a refusal to save two enabled rules of one kind that look for the same
thing. The file behind it is `abbreviations.json`, and it can equally be edited
by hand:

```json
{
  "version": 1,
  "apply_to_names": true,
  "rules": [
    { "enabled": true, "whole_name": false, "abbreviation": "EMP", "replacement": "Employee" },
    { "enabled": true, "whole_name": true,  "abbreviation": "T_SAMPLE_ALBUM", "replacement": "Album" }
  ]
}
```

`apply_to_names` is the switch itself. A word rule matches a **whole** segment
between `_` or `-` separators, never a prefix of one, so there is nothing for two
overlapping rules to fight over; where two rules do name the same abbreviation —
which the editor refuses to save — the later one wins.

## Error handling

| Situation | Behaviour |
|:---|:---|
| Unknown field or variable | Warning with a line number, expands to an empty string — not an error |
| Unknown processor | Render error listing the valid processors |
| `replace` with fewer than 2 arguments | Render error |
| Unknown condition name in `if` | Parse error |
| Missing `key`/`item` | Parse error |
| `for` over a field that does not exist | Render error: `Model has no 'x' member` |
| `for` over a field that is not a list | Render error: `'x' is not a collection` |
| `contains`/`notContains` on a value that is neither a list nor a string | Render error |
| Unterminated `}` | Parse error: `'}' not found, before: …` |
| Missing `endif` / `endfor` | Parse error: `if statements not closed` / `for statements not closed` |
| Unknown statement type | Parse error: `Unknown template: …` |
| Non-numeric `padSize` or `indent` | Render error |

**Parse errors** are found before any file is written: every template body and
every output-name template of a run is parsed first, so a template with a syntax
error costs nothing — the run stops and names the template, which half of it, and
the line. **Render errors** depend on the model, so they surface per table; the
run reports the file that failed, carries on with the rest, and lists the
failures in the result summary. Nothing already written is rolled back.

> **Changed — errors carry a line number.**
> jdbgen reported some of these with a character offset into the file, which is
> not something you can act on. Every error here names the line, and the editor
> puts a mark on it.

Because a missing field is silent, an unexpectedly empty spot in the output is
almost always a misspelled field name. The warning behind it reads
`cannot find 'x' information from database/custom variables` and names the line;
the result summary counts them, and a **Dry run** surfaces them without writing
anything.

## Recipes

The three templates in `templates/` (copied into your configuration directory on
first run — see [Installation](installation.md)) are working examples; the
patterns below are taken from them.

### A column list, one per line

```sql
        SELECT  ${for:item=columns, inStr="\n,", indent=-1}${column} AS "${name.camel}"${endfor}
          FROM ${table} A
```

```sql
        SELECT  ALBUM_ID AS "albumId"
               ,ALBUM_NAME AS "albumName"
               ,ARTIST_NAME AS "artistName"
               ,PUBLISH_DATE AS "publishDate"
          FROM T_SAMPLE_ALBUM A
```

### A primary key predicate

```sql
         WHERE ${for:item=keys, inStr="\nAND ", indent=-4}${column} = #{${name.camel}}${endfor}
```

For a two-column primary key:

```sql
         WHERE ALBUM_ID = #{albumId}
           AND TRACK_NO = #{trackNo}
```

### An UPDATE that leaves the key columns alone

```sql
        UPDATE ${table}
           SET  ${for:item=notKeys, inStr="\n,", indent=-1}${column} = #{${name.camel}}${endfor}
         WHERE ${for:item=keys, inStr="\nAND ", indent=-4}${column} = #{${name.camel}}${endfor}
```

### An INSERT that skips the generated key (D8)

`autoIncrement` is new, and this is what it is for: a column the database fills
in must not appear in the `INSERT`.

`autoIncrement` renders as the text `true` or `false`, so it is compared with
`equals`:

```sql
INSERT INTO ${table} (${for:item=columns, inStr=", "}${if:item=autoIncrement, equals='false'}${column}${endif}${endfor})
```

The separator is written between *iterations*, not between the fragments an
`if` decided to emit, so a skipped column still leaves its `, ` behind. Where
the list has to come out exactly right, name the generated column in
`skipList` instead — or loop over `notKeys`, which is what the shipped MyBatis
template does.

### A Java field per outgoing foreign key (D8)

```java
    ${for:item=imports}// -> ${refTable}
    private ${item:key=refTable.suffix.pascal} ${refTable.suffix.camel};
    ${endfor}
```

The column's own `fk` would be the shorter way round, but see the limitation
under [The `fk` of a column](#the-fk-of-a-column-d8-new) before reaching for it.

### A unique-index lookup method per index (D8)

```java
    ${for:item=indexes}${if:item=unique, equals='true'}
    public ${super:key=name.suffix.pascal}Model findBy${name.suffix.pascal}(${for:item=columns, inStr=", "}String ${name.camel}${endfor}) { … }
    ${endif}${endfor}
```

### A numeric column that keeps its precision (D8)

```java
    ${if:item=jdbcType, contains='DECIMAL,NUMERIC'}${if:item=scale, equals='0'}Long${else}java.math.BigDecimal${endif}${else}${javaType}${endif}
```

### A qualified class name from a custom variable

Define a custom variable `package` = `com.abc.sample` in the Generate tab, then
use it in both the template and the output file name:

| Template | Output |
|:---|:---|
| `package ${package}.${name.suffix.camel};` | `package com.abc.sample.sampleAlbum;` |
| Output file name `${package.replace('.','/')}/${name.suffix.pascal}.java` | `com/abc/sample/SampleAlbum.java` |

---

## Related documentation

- [User interface guide](ui-guide.md) — where templates are edited and
  previewed.
- [Custom queries](custom-queries.md) — the fields your own metadata SQL
  populates.
- [Installation](installation.md) — where the templates live on disk.
