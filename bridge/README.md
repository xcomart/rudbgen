# rudbgen-bridge

The Java side of the JNI boundary. One JAR, one entry point, no framework.

Everything `rudbgen-jdbc` can ask a database is routed through a single static
method, and every answer comes back as a length-prefixed byte array. This file is
the contract; `docs/architecture.md` is the design behind it.

This is [rudbman](https://github.com/xcomart/rudbman)'s bridge, copied and
trimmed (architecture.md D3). rudbgen writes files from metadata and never
ferries row data, so the `job/` layer (extract, backup, transfer), the LOB
reference path, the `template/` engine (ported to Rust as `rudbgen-template`,
D4) and the four `DESCRIBE` kinds that needed hand-written vendor catalogue
queries (`ddl`, `procedures`, `functions`, `sequences`) are gone — about 40% of
the Java. **Their operation codes are retired, not reused**: `0x25` and
`0x40`–`0x42` stay spent so this table and rudbman's keep lining up.

## Build

```
cd bridge
./gradlew build          # compile + test
./gradlew jar            # just the artifact
```

Output: `bridge/build/libs/rudbgen-bridge.jar`.

Requires an ambient JDK 17 or newer (`--release 17` pins the bytecode level).
The Gradle wrapper is pinned to 8.14.3. First build needs network access to
Maven Central for Gson, H2 and JUnit.

Gson is merged into the JAR rather than shipped beside it, because the JVM is
booted with `-Djava.class.path=<bridge.jar>` and nothing else. It stays under its
own `com.google.gson` package — it is not relocated. Driver JARs load through a
child loader whose parent is the bridge loader, so a driver that bundles its own
Gson will see the bridge's copy. No known JDBC driver does; if one ever does, the
fix is a shading plugin, not a classpath change.

`cargo build` never runs Gradle. `rudbgen-jdbc/build.rs` only checks that the JAR
exists.

## Entry point

```java
package comart.rudbgen.bridge;

public final class Bridge {
    public static byte[] call(int op, long handle, long arg, byte[] req);
}
```

- `op` — operation code, below
- `handle` — session or cursor handle; `0` when the operation takes none
- `arg` — integer argument for hot paths, so `FETCH` parses no JSON
- `req` — request body as UTF-8 JSON, or `null`
- returns — response envelope, never `null`

**`call` never throws.** Every failure, `Throwable` included, comes back as an
ERROR envelope. `NoClassDefFoundError` out of an incomplete driver JAR is a
message for the user, not a reason to take the process down. This is what lets
the Rust side drop `ExceptionCheck` from the normal path.

## Operations

| Code | Name | handle | arg | req | resp |
|---|---|---|---|---|---|
| `0x01` | `OPEN_SESSION` | — | — | JSON connection spec | JSON `{session}` |
| `0x02` | `CLOSE_SESSION` | session | — | — | — |
| `0x03` | `PING` | session | — | — | JSON `{ok, elapsed_ms}` |
| `0x04` | `SESSION_INFO` | session | — | — | JSON product / driver / capability facts |
| `0x10` | `DESCRIBE` | session | — | JSON `{kind, …}` | JSON `{kind, items[]}` |
| `0x20` | `EXECUTE` | session | — | JSON statement spec | JSON `{cursor, columns[], update_count, has_result_set, has_more}` |
| `0x21` | `FETCH` | cursor | max rows | — | binary `RDB1` batch |
| `0x22` | `MORE_RESULTS` | cursor | — | — | JSON, same shape as `EXECUTE` |
| `0x23` | `CLOSE_CURSOR` | cursor | — | — | — |
| `0x24` | `CANCEL` | session | — | — | JSON `{cancelled}` |
| `0x30` | `SET_AUTOCOMMIT` | session | 0/1 | — | — |
| `0x31` | `COMMIT` | session | — | — | — |
| `0x32` | `ROLLBACK` | session | — | — | — |
| `0x50` | `PROBE_DRIVER` | — | — | JSON `{jars[]}` | JSON `{classes[], services[]}` |

`0x25` (rudbman's `LOB_READ`) and `0x40`–`0x42` (its `JOB_START` / `JOB_POLL` /
`JOB_CANCEL`) are retired here and answer as unknown operations. No code is ever
reassigned.

## Response envelope

```
u8  tag       0 = OK, 1 = ERROR
    payload   OK: operation body (JSON or binary), ERROR: JSON
```

Operations with no response body return the single byte `0x00`.

Error JSON, every member always present:

```json
{
  "kind": "sql | driver | io | protocol | interrupted | internal",
  "sql_state": "42S02",
  "vendor_code": 942,
  "message": "…",
  "causes": ["java.net.ConnectException: Connection refused", "…"],
  "stack": "…"
}
```

- `sql_state` / `vendor_code` come from `SQLException`. Both chains are walked —
  `getCause` and `getNextException` — because drivers routinely hide the real
  reason in the second exception. Both walks are cycle-guarded and depth-capped
  at 16.
- `causes[]` is that same flattened chain, excluding the root message.
- `stack` is for the debug log. Never show it to the user.
- `kind: "driver"` covers a missing driver class, a missing JAR, linkage errors
  and *"this driver does not accept this URL"* — the `null` return from
  `Driver.connect`, which the JDBC spec defines as "I do not understand this
  URL" and which is not an exception.

All responses use `serializeNulls`: a member the driver had nothing to say about
is JSON `null`, never absent. Rust `Option<T>` fields therefore need no
`#[serde(default)]`.

## Requests

### `OPEN_SESSION`

```json
{
  "url": "jdbc:postgresql://localhost:5432/app",
  "driver_class": "org.postgresql.Driver",
  "jars": ["/home/u/.config/rudbman/drivers/postgresql-42.7.4.jar"],
  "username": "app",
  "password": "…",
  "props": { "ApplicationName": "rudbman" },
  "read_only": false,
  "auto_commit": true,
  "login_timeout_s": 10,
  "keep_alive": { "enabled": true, "interval_s": 300, "query": "select 1" }
}
```

`url` and `driver_class` are required. An empty or absent `jars` resolves the
driver class from the bridge's own loader — that is how a driver baked into the
jlink image, or H2 on the test classpath, is reached.

`login_timeout_s` is passed through as a `loginTimeout` connection property.
`java.sql.Driver` has no login timeout of its own, and `DriverManager`'s is
global mutable state this bridge stays away from. A caller that knows its driver
should set the real property in `props`.

### `EXECUTE`

```json
{
  "sql": "select * from t where id = ? and amount > ?",
  "params": [42, { "type": "decimal", "value": "123456789012.12345678" }],
  "fetch_size": 500,
  "max_rows": 0,
  "timeout_s": 30
}
```

A parameter is either a bare JSON scalar (`null`, boolean, number, string) or an
object `{"type": …, "value": …}`. Types: `null`, `bool`, `i64`, `f64`,
`string`, `decimal`, `date`, `time`, `timestamp`, `bytes` (base64).

The typed form exists because JSON has one numeric type and no date type. A
`DECIMAL(20,8)` sent as a JSON number arrives rounded — the same mistake the
batch codec refuses to make in the other direction.

Omitting `params`, or sending an empty array, uses a plain `Statement` instead of
a `PreparedStatement`.

`EXECUTE` **always** returns a non-zero `cursor`, even for an `UPDATE` that
produced only a row count, so `MORE_RESULTS` always has something to advance and
`CLOSE_CURSOR` always has something to close. `FETCH` on such a cursor returns an
empty terminal batch rather than an error.

`has_more` is a **hint**, not a fact. JDBC offers no way to look ahead without
consuming the current result, so it means *"`MORE_RESULTS` may still return
something"*. Keep calling `MORE_RESULTS` until it answers `has_more: false`
(which comes with `cursor` set, `update_count: -1` and no columns).

### `DESCRIBE`

```json
{ "kind": "columns", "catalog": null, "schema": "APP", "table": "CHILD" }
```

| kind | needs | notes |
|---|---|---|
| `catalogs` | — | |
| `schemas` | — | |
| `tables` | — | `types[]` filters by `TABLE`, `VIEW`, … |
| `columns` | — | |
| `primary_keys` | exact `table` | |
| `imported_keys` | exact `table` | |
| `exported_keys` | exact `table` | |
| `indexes` | exact `table` | `unique_only`, `approximate` |
| `type_info` | — | |

Every response is `{ "kind": "...", "items": [ … ] }`. Item keys are
fixed snake_case chosen here, **not** the driver's metadata labels, so the Rust
structs stay stable across drivers. Optional metadata columns that a driver omits
come back as `null`; the reader collects the available labels once instead of
asking for a missing one and catching the exception per cell.

- `schema` is an exact name, `schema_pattern` a LIKE pattern; likewise
  `table` / `table_pattern` and `column` / `column_pattern`.
- `imported_keys` and `exported_keys` share one item shape with `pk_`/`fk_`
  prefixes; only the direction of the query differs.
- `indexes` accepts `unique_only` (default false) and `approximate`
  (default **true** — a statistics refresh on a large schema is the difference
  between instant and a minute).

### `FETCH`

`arg` is the maximum row count. `arg <= 0` means the default 500; values above
1,000,000 are clamped. No JSON is parsed on this path.

## `RDB1` batch codec

All integers little-endian.

```
Batch  := Header Column*
Header := "RDB1"(4B) | u32 col_count | u32 row_count | u8 flags
          flags bit0 = this is the last batch
Column := u8 kind | u32 payload_len | payload
```

`payload` **always** starts with a validity bitmap of `ceil(row_count/8)` bytes.
Bits are **LSB-first**: row `i` is byte `i >> 3`, bit `i & 7`. A set bit means
**non-null**. `payload_len` covers the bitmap and the values together.

| kind | name | value area |
|---|---|---|
| 0 | `NULLS` | none — every row is NULL |
| 1 | `I64` | `row_count × i64` |
| 2 | `F64` | `row_count × f64` (raw IEEE-754 bits, so NaN and ±∞ survive) |
| 3 | `BOOL` | packed bits, LSB-first, `ceil(row_count/8)` bytes |
| 4 | `STR` | `u32 offsets[row_count+1]` then UTF-8 bytes |
| 5 | `BIN` | same layout, raw bytes |

Kind `6` was rudbman's `LOB` reference column,
`row_count × (u64 lob_id, u64 size)`, resolved by a follow-up `LOB_READ`. It is
retired with that op and the number is not reused, so a batch never means one
thing in rudbman and another here.

Points a decoder has to get right:

- **The kind of a column can change between batches of the same cursor.** A
  batch in which a column is entirely NULL is emitted as kind 0 regardless of the
  column's declared type, so a 500-row all-null string column costs 63 bytes
  instead of kilobytes of zero offsets. Switch on the kind byte per batch, not
  once per cursor. The stable type is the logical one in `columns[]`.
- **Kind 0 still carries the bitmap** (all zeros), because the payload always
  starts with one. Only the value area is omitted.
- **NULL rows still occupy a slot** in every fixed-width value area, filled with
  zero, so indexes line up with the bitmap without a rank computation.
- **A NULL and an empty string produce the same zero-length slice.** Only the
  bitmap tells them apart. The grid must distinguish them; too many tools do not.
- **`row_count` may be 0.** The bitmap is then 0 bytes, and every column is
  kind 0 with an empty payload. A cursor that produced no result set at all
  yields `col_count: 0` too.
- The last batch is only recognised as such when the driver runs out of rows. A
  batch that filled its row limit exactly reports `flags = 0`; the next `FETCH`
  returns a 0-row batch with bit0 set.

### Type mapping

| JDBC type | kind |
|---|---|
| `TINYINT` `SMALLINT` `INTEGER` `BIGINT` | `I64` |
| `REAL` `FLOAT` `DOUBLE` | `F64` |
| `BOOLEAN`, `BIT` with precision ≤ 1 | `BOOL` |
| `BINARY` `VARBINARY` `LONGVARBINARY` `BLOB`, `BIT` with precision > 1 | `BIN` |
| `CLOB` `NCLOB` | `STR` |
| everything else | `STR` |

`DECIMAL`, `NUMERIC`, `DATE`, `TIME`, `TIMESTAMP`, `UUID`, `INTERVAL`, arrays,
`SQLXML` and vendor types all travel as `STR`:

- the grid displays text in the end;
- flattening a `BigDecimal` into an `f64` cannot be undone — `DECIMAL` goes
  through `BigDecimal.toPlainString()`, never through a double and never through
  exponent notation;
- the driver's own text is the only authority on which time zone was applied, so
  time-zone handling stays on this side of the boundary;
- sorting is the server's job via `ORDER BY`.

`BIT` is split by precision because MySQL reports `BIT(n>1)` as `Types.BIT` while
handing back a byte string, not a boolean.

Presentation — right alignment, NULL rendering, copy format — is decided by the
**logical** type in `columns[]` (`type`, `jdbc_type`, `type_name`, `precision`,
`scale`, `nullable`, `auto_increment`). The `kind` is transport only. Each entry
in `columns[]` also carries a `kind` hint: the encoding a full batch would use.

### LOBs

rudbman never inlined a `BLOB`, `CLOB` or `NCLOB`: each cell contributed a
16-byte `(id, size)` pair and the UI pulled the bytes on demand with `LOB_READ`.
rudbgen has no `LOB_READ` to pull with, so **LOBs are materialised** — `BLOB` as
`BIN`, `CLOB` and `NCLOB` as `STR`.

That is safe here because of what runs through `EXECUTE`. The only statements
rudbgen sends are the four custom catalogue queries of architecture.md D9, whose
values are table names, column names and comments — and Oracle hands a comment
back as a `CLOB`. Nothing streams a user's row data through this bridge; the
explorer and the inspector read `DESCRIBE`, which returns JSON.

The sharp edge rudbman deferred is therefore inherited rather than fixed: a
custom query that selects a genuine multi-megabyte LOB column will carry it
across JNI. Do not point one at a document table.

## Concurrency

- One JDBC connection per session, one Rust worker thread per session. The worker
  serialises commands, which is what makes a non-thread-safe `Connection` safe.
- The session still holds a `ReentrantLock` around every use of the connection,
  because the keep-alive timer runs concurrently with the worker. The timer uses
  `tryLock` and skips its round rather than queueing — a statement already in
  flight keeps the connection just as alive.
- **`CANCEL` deliberately takes no lock.** It arrives on a different thread while
  the worker holds the lock inside the blocking `execute` it is meant to
  interrupt. The cursor table is a `ConcurrentHashMap` for exactly this, and
  cursors are registered *before* the statement executes. `Statement.cancel()` is
  the one JDBC method documented as callable from another thread.
- Handles are never reused, so a stale handle is always reported as a stale
  handle and never mistaken for a live object.

## Driver isolation

One child `URLClassLoader` per set of driver JARs, parented to the bridge loader.
`Class.forName(cls, true, child)` then `Driver.connect(url, props)` directly —
**never `DriverManager`**, which is a global registry where two drivers claiming
the same URL prefix produce an undefined winner.

Loaders are cached by the JAR path list and reference counted; the loader closes
when its last session does. A fresh loader per session re-runs the driver's
static initialisers and leaks everything they loaded.

The cache key preserves the caller's JAR order, because classpath order decides
which JAR wins when two ship the same class — two orderings are genuinely two
different class paths.

`PROBE_DRIVER` uses a throwaway loader and `Class.forName(cls, false, …)`:
probing must not run a driver's static initialiser, which may open sockets or
load native libraries, and must not pin a JAR the user is about to replace.

## Layout

```
src/main/java/comart/rudbgen/bridge/
├── Bridge.java            the single JNI entry point; op dispatch, Throwable → envelope
├── Ops.java               operation codes
├── Envelope.java          response envelope and error mapping
├── Json.java              Gson tree helpers
├── BridgeException.java   failures that carry their own envelope kind
├── Registry.java          handle ↔ object table
├── Session.java           Connection + loader lease + keep-alive
├── Loaders.java           URLClassLoader cache
├── Cursor.java            Statement + ResultSet + batch encoding
├── Params.java            EXECUTE parameter binding
├── DriverProbe.java       PROBE_DRIVER
├── codec/                 RDB1 encoder
└── meta/
    ├── Describe.java      DESCRIBE dispatch, the DatabaseMetaData kinds
    ├── Comments.java      table and column comments from the vendor catalogue
    ├── Dialect.java       product name → the vendor paths that apply
    ├── Ident.java         identifier quoting, only where it is needed
    ├── Attempt.java       savepoint-fenced query that is allowed to fail
    ├── RsView.java        reader for metadata result sets with optional columns
    ├── SessionInfo.java   SESSION_INFO
    └── SqlTypes.java      java.sql.Types names
```

rudbman additionally has `job/` (extract, backup, transfer), `template/` and the
`meta/` classes behind the retired `DESCRIBE` kinds (`Ddl`, `Routines`,
`Sequences`) plus `meta/Upsert` and `codec/LobSink`. Everything that remains here
is byte-for-byte rudbman's apart from the package name and the cuts this file
lists, so a fix in one is a plain diff into the other.

## Inherited code

From [jdbgen](https://github.com/comart/jdbgen) (MIT, Dennis Soungjin Park):

- `types/db/DBMeta.java` → `Session.java`, `Loaders.java` — the child class
  loader, the deliberate avoidance of `DriverManager`, the explicit `null` check
  on `Driver.connect`, the connection lock, the keep-alive scheduler
- `types/db/SqlTypes.java` → `meta/SqlTypes.java`
- `utils/ClassUtils.java` → `DriverProbe.java`

Beyond what jdbgen had: primary keys, foreign keys, indexes, `type_info`, the
`RDB1` codec, the handle registry, cancellation and the error envelope — all of
it inherited from rudbman.

## Tests

```
./gradlew test
```

H2 in-memory is the reference database. These earn their keep by round tripping
rather than by asserting on strings:

- `FetchRoundTripTest` asserts through `support/Batch.java`, a decoder written
  from the format description above rather than from the encoder. That round trip
  is the only thing that proves the format the Rust decoder has to read.
- `DescribeTest` reads a fixture schema back through every surviving kind, which
  is the whole of what rudbgen asks a database for.

rudbman's `DdlTest`, `DescribeRoutinesTest`, `ExtractJobTest`, `BackupJobTest`,
`TransferJobTest` and `TemplateManagerTest` went with the code they covered.

## Known gaps

- **LOBs are materialised, not referenced.** A custom catalogue query that
  selects a large `BLOB` or `CLOB` column carries the whole value across JNI.
  See "LOBs" above for why that is an acceptable trade here and not in rudbman.
- `LONGVARCHAR` / `LONGVARBINARY` inline too; on MySQL a `LONGTEXT` reaches 4GB
  under `Types.LONGVARCHAR`.
- `DESCRIBE` reads what `DatabaseMetaData` reports. `CHECK` constraints,
  triggers, partitioning, collations and generated-column expressions are not in
  it, so a template cannot see them. `UNIQUE` constraints arrive as unique
  indexes.
- A view reaches `columns` as a bare column list; JDBC metadata does not carry
  the view's query text.
