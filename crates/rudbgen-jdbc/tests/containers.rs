//! Opt-in metadata tests against real PostgreSQL, MySQL, MariaDB, SQL Server
//! and Oracle servers.
//!
//! `tests/h2.rs` proves that Rust reads what Java wrote. It cannot prove the
//! other half of what rudbgen depends on: that `DESCRIBE` answers the *same
//! shape* whichever driver is underneath. Every field the template model is
//! built from — a column's type name and nullability, a primary key's order, a
//! foreign key's two ends, an index's uniqueness (architecture.md §6, D8) —
//! comes out of `DatabaseMetaData`, and every driver has its own habits about
//! it. Which of catalog and schema a database even is, whether `getSchemas`
//! answers at all, whether an unquoted name comes back folded: none of that is
//! visible from H2.
//!
//! So each test here creates a small fixture — a parent, a child that
//! references it, a unique index — and **reads the catalogue back** through
//! `DescribeRequest`. Nothing is written by a planner and nothing is asserted
//! against a string somebody reasoned out; the assertions are about what the
//! server says it holds.
//!
//! rudbman's version of this file drove its DDL and transfer planners against
//! the same five servers. Those planners are not in rudbgen (architecture.md
//! D3), so what is left is the reading — which is all this application ever
//! does to a database anyway.
//!
//! # Opt-in, and silent when it is out
//!
//! Every test here passes by doing nothing when its server's URL is unset, and
//! says so in one line. That is deliberately *not* what `h2_jar()` does: H2 is
//! a dependency the repository guarantees, so a missing H2 is a broken
//! checkout and a panic; a database server is a container the developer chose
//! to start. `cargo test --workspace` must stay green on a machine with no
//! Docker — and a developer who started one container out of five gets that
//! container's tests and four skips.
//!
//! ```text
//! docker compose -f docker/compose.yml up -d      # then wait for "healthy"
//! export RUDBGEN_TEST_PG_URL='jdbc:postgresql://127.0.0.1:55432/rudbgen'
//! export RUDBGEN_TEST_MYSQL_URL='jdbc:mysql://127.0.0.1:33306/rudbgen?allowPublicKeyRetrieval=true&useSSL=false'
//! export RUDBGEN_TEST_MARIADB_URL='jdbc:mariadb://127.0.0.1:53306/rudbgen'
//! export RUDBGEN_TEST_MSSQL_URL='jdbc:sqlserver://127.0.0.1:51433;encrypt=false'
//! export RUDBGEN_TEST_ORACLE_URL='jdbc:oracle:thin:@//127.0.0.1:51521/FREEPDB1'
//! cargo test -p rudbgen-jdbc --test containers
//! ```
//!
//! The user and password default to `rudbgen`/`rudbgen`, which is what
//! `docker/compose.yml` sets up for four of the five; `RUDBGEN_TEST_PG_USER`,
//! `RUDBGEN_TEST_PG_PASSWORD` and their `MYSQL`, `MARIADB`, `MSSQL` and
//! `ORACLE` counterparts override them. SQL Server is the exception, and not by
//! choice: its `sa` password has a complexity rule that `rudbgen` fails, so the
//! default there is `sa`/`Rudbgen!Passw0rd` and the URL names no database at
//! all — the image ships none but `master`, and every table here is named
//! uniquely enough to live in it.
//!
//! # The driver JARs
//!
//! Found the way `tests/h2.rs` finds H2's, and for the same reason — the
//! Gradle cache is already the one place in this checkout where a driver
//! lives. What fills it is `cd bridge && ./gradlew drivers`, a task whose only
//! job is to resolve the five drivers; they are *not* `testImplementation`,
//! because no Java test in this project loads any of them.
//! `RUDBGEN_TEST_PG_JAR` and its four counterparts override the search.
//!
//! A missing JAR **is** a panic, unlike a missing server: by the time one is
//! looked for the developer has already set a URL and asked for the test to
//! run, and a test that passes because it could not find its driver is the
//! thing this file exists to avoid.
//!
//! # Case folding
//!
//! Every identifier this file writes is unquoted, so a product that folds
//! unquoted names to upper case stores them shouted and answers `getColumns`
//! shouted. [`Server::name`] applies that fold to the names the test then
//! looks for, which keeps the assertions about the metadata rather than about
//! Oracle's case rules.
//!
//! # Cleaning up
//!
//! Every table is named `rb_<what>_<pid>_<n>`, so two runs — or two developers
//! against one server — cannot collide, and every one of them is registered
//! with the [`Server`] that drops it however the test ends. Tables go in
//! reverse order of creation, so a child goes before the parent it references.

use std::cell::RefCell;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rudbgen_jdbc::{
    Batch, ConnectionSpec, DescribeRequest, Jvm, JvmConfig, Session, StatementSpec, Value,
    default_bridge_jar,
};

/// The process-wide JVM, started by whichever test needs it first.
fn jvm() -> &'static Jvm {
    Jvm::start(&JvmConfig::new(default_bridge_jar()).with_heap_mb(256))
        .expect("the JVM must start; build the bridge with `cd bridge && ./gradlew jar`")
}

// --- finding the drivers ---------------------------------------------------

/// Locates a driver JAR, or fails with instructions.
///
/// Only ever called once a server URL has been set, so a JAR that is not there
/// is a panic rather than a skip — see the module documentation.
fn driver_jar(env: &str, group: &str, artifact: &str) -> PathBuf {
    if let Some(path) = std::env::var_os(env) {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "{env} points at {}, which is not a file",
            path.display()
        );
        return path;
    }
    find_in_gradle_cache(group, artifact).unwrap_or_else(|| {
        panic!(
            "the {group}:{artifact} driver JAR was not found.\n\
             \n\
             A server URL is set, so this test was asked to run, and it needs a driver.\n\
             Fetch the drivers into the Gradle cache with:\n\
             \n    cd bridge && ./gradlew drivers\n\
             \n\
             or point {env} at a JAR you already have."
        )
    })
}

/// Walks `<gradle home>/caches/modules-2/files-2.1/<group>/<artifact>/*/*/<artifact>-<version>.jar`.
///
/// The same two-level walk `tests/h2.rs` does, with the coordinates as
/// arguments rather than baked in: the shape of the cache is fixed, and one
/// crate with two copies of it would be two places for the same mistake.
fn find_in_gradle_cache(group: &str, artifact: &str) -> Option<PathBuf> {
    let gradle_home = std::env::var_os("GRADLE_USER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".gradle")))?;
    let root = gradle_home
        .join("caches/modules-2/files-2.1")
        .join(group)
        .join(artifact);

    let mut newest: Option<(String, PathBuf)> = None;
    for version in std::fs::read_dir(&root).ok()?.flatten() {
        let number = version.file_name().to_string_lossy().into_owned();
        // Only the binary artefact. The same directory also holds
        // `-javadoc.jar` and `-sources.jar`, and picking one of those gets a
        // class loader with no classes in it — a ClassNotFoundException a long
        // way from its cause.
        let wanted = format!("{artifact}-{number}.jar");
        for hash in std::fs::read_dir(version.path()).ok()?.flatten() {
            for file in std::fs::read_dir(hash.path()).ok()?.flatten() {
                if file.file_name().to_string_lossy() == wanted
                    && newest
                        .as_ref()
                        .is_none_or(|(best, _)| best.as_str() < number.as_str())
                {
                    newest = Some((number.clone(), file.path()));
                }
            }
        }
    }
    newest.map(|(_, path)| path)
}

// --- the five products -----------------------------------------------------

/// Which server a test wants.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Product {
    Postgres,
    MySql,
    MariaDb,
    MsSql,
    Oracle,
}

impl Product {
    /// The environment variable that both enables the product and says where
    /// it is.
    fn url_var(self) -> &'static str {
        match self {
            Product::Postgres => "RUDBGEN_TEST_PG_URL",
            Product::MySql => "RUDBGEN_TEST_MYSQL_URL",
            Product::MariaDb => "RUDBGEN_TEST_MARIADB_URL",
            Product::MsSql => "RUDBGEN_TEST_MSSQL_URL",
            Product::Oracle => "RUDBGEN_TEST_ORACLE_URL",
        }
    }

    fn driver_class(self) -> &'static str {
        match self {
            Product::Postgres => "org.postgresql.Driver",
            Product::MySql => "com.mysql.cj.jdbc.Driver",
            Product::MariaDb => "org.mariadb.jdbc.Driver",
            Product::MsSql => "com.microsoft.sqlserver.jdbc.SQLServerDriver",
            Product::Oracle => "oracle.jdbc.OracleDriver",
        }
    }

    fn jar(self) -> PathBuf {
        match self {
            Product::Postgres => driver_jar("RUDBGEN_TEST_PG_JAR", "org.postgresql", "postgresql"),
            Product::MySql => {
                driver_jar("RUDBGEN_TEST_MYSQL_JAR", "com.mysql", "mysql-connector-j")
            }
            Product::MariaDb => driver_jar(
                "RUDBGEN_TEST_MARIADB_JAR",
                "org.mariadb.jdbc",
                "mariadb-java-client",
            ),
            Product::MsSql => driver_jar(
                "RUDBGEN_TEST_MSSQL_JAR",
                "com.microsoft.sqlserver",
                "mssql-jdbc",
            ),
            Product::Oracle => driver_jar(
                "RUDBGEN_TEST_ORACLE_JAR",
                "com.oracle.database.jdbc",
                "ojdbc11",
            ),
        }
    }

    /// Whether this product folds an unquoted identifier to upper case.
    fn folds_upper(self) -> bool {
        self == Product::Oracle
    }

    /// `(user, password)`, from the environment or the compose file's defaults.
    fn credentials(self) -> (String, String) {
        let (user_var, password_var) = match self {
            Product::Postgres => ("RUDBGEN_TEST_PG_USER", "RUDBGEN_TEST_PG_PASSWORD"),
            Product::MySql => ("RUDBGEN_TEST_MYSQL_USER", "RUDBGEN_TEST_MYSQL_PASSWORD"),
            Product::MariaDb => ("RUDBGEN_TEST_MARIADB_USER", "RUDBGEN_TEST_MARIADB_PASSWORD"),
            Product::MsSql => ("RUDBGEN_TEST_MSSQL_USER", "RUDBGEN_TEST_MSSQL_PASSWORD"),
            Product::Oracle => ("RUDBGEN_TEST_ORACLE_USER", "RUDBGEN_TEST_ORACLE_PASSWORD"),
        };
        // SQL Server's `sa` password has a complexity rule `rudbgen` fails, so
        // that one image gets a login of its own; `docker/compose.yml` says the
        // same thing in its header.
        let (user, password) = match self {
            Product::MsSql => ("sa", "Rudbgen!Passw0rd"),
            _ => ("rudbgen", "rudbgen"),
        };
        (
            std::env::var(user_var).unwrap_or_else(|_| user.to_string()),
            std::env::var(password_var).unwrap_or_else(|_| password.to_string()),
        )
    }
}

/// A connected server, with the two names a `DescribeRequest` has to be
/// narrowed by.
///
/// The products disagree about which of the two a database even is. PostgreSQL
/// and SQL Server put a table in a *schema* of a catalog; MySQL's and
/// MariaDB's drivers report the database as the *catalog* and no schema at all;
/// Oracle has no catalog and calls the owning user the schema. Every describe
/// below goes through [`Server::describe`] so that difference is written once.
struct Server {
    product: Product,
    session: Session,
    catalog: Option<String>,
    schema: Option<String>,
    /// Tables to drop when the test ends, in creation order.
    tables: RefCell<Vec<String>>,
}

impl Drop for Server {
    fn drop(&mut self) {
        // Reverse order, so a child that references a parent goes first. Every
        // drop is `IF EXISTS` and every failure is ignored: this runs on the way
        // out of a panicking test too, and a cleanup that panicked would replace
        // the real failure with its own.
        for table in self.tables.borrow().iter().rev() {
            let _ = self
                .session
                .execute(&StatementSpec::new(format!("DROP TABLE IF EXISTS {table}")));
        }
    }
}

impl Server {
    /// Opens the product's server, or answers `None` and says how to get one.
    fn open(product: Product) -> Option<Server> {
        let Ok(url) = std::env::var(product.url_var()) else {
            println!(
                "skipped: no {product:?} server. Start one with \
                 `docker compose -f docker/compose.yml up -d` and set {} \
                 (the URL is in that file's header).",
                product.url_var()
            );
            return None;
        };
        let session = connect(product, &url);

        // Asked of the server rather than parsed out of the URL: the URL may
        // leave the database to the driver's default, and a catalogue request
        // has to name the one the session really landed in.
        let (catalog, schema) = match product {
            Product::Postgres => (None, Some(scalar(&session, "select current_schema()"))),
            Product::MySql | Product::MariaDb => {
                (Some(scalar(&session, "select database()")), None)
            }
            // The one product here that has both, and the only URL that names
            // no database: `master` is where a session with nothing else asked
            // for lands, and `db_name()` is how it says so.
            Product::MsSql => (
                Some(scalar(&session, "select db_name()")),
                Some(scalar(&session, "select schema_name()")),
            ),
            // Oracle's `getTables` takes no catalog at all, and the schema is
            // the connected user's — asked of the session rather than assumed
            // from the credentials, since `ALTER SESSION` can move it.
            Product::Oracle => (
                None,
                Some(scalar(
                    &session,
                    "select sys_context('userenv', 'current_schema') from dual",
                )),
            ),
        };
        Some(Server {
            product,
            session,
            catalog,
            schema,
            tables: RefCell::new(Vec::new()),
        })
    }

    /// Runs a statement that returns no rows.
    fn exec(&self, sql: &str) {
        self.session
            .execute(&StatementSpec::new(sql.to_string()))
            .unwrap_or_else(|error| panic!("{:?}: {sql}: {error}", self.product));
    }

    /// A table name no other test and no other run of this one uses,
    /// registered for cleanup and folded the way this product will store it.
    fn table(&self, what: &str) -> String {
        static SEQ: AtomicU32 = AtomicU32::new(0);
        let name = self.name(&format!(
            "rb_{what}_{}_{}",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        self.tables.borrow_mut().push(name.clone());
        name
    }

    /// A name in the case this product's catalogue will hold it.
    ///
    /// Everything this file writes is unquoted, so on a folding product the
    /// server stores the upper-case spelling and `getColumns` answers it. The
    /// fold belongs here rather than in each assertion.
    fn name(&self, name: &str) -> String {
        if self.product.folds_upper() {
            name.to_uppercase()
        } else {
            name.to_string()
        }
    }

    /// A describe request narrowed to this session's catalog and schema.
    fn narrowed(&self, kind: &str) -> DescribeRequest {
        let mut request = DescribeRequest::new(kind);
        if let Some(catalog) = &self.catalog {
            request = request.with_catalog(catalog);
        }
        if let Some(schema) = &self.schema {
            request = request.with_schema(schema);
        }
        request
    }

    /// Runs a describe of `kind`, narrowed to this session.
    fn describe(&self, kind: &str) -> Vec<serde_json::Map<String, serde_json::Value>> {
        self.run(self.narrowed(kind), kind, "")
    }

    /// Runs a describe of `kind` for one table, narrowed to this session.
    fn describe_table(
        &self,
        kind: &str,
        table: &str,
    ) -> Vec<serde_json::Map<String, serde_json::Value>> {
        self.run(self.narrowed(kind).with_table(table), kind, table)
    }

    fn run(
        &self,
        request: DescribeRequest,
        kind: &str,
        what: &str,
    ) -> Vec<serde_json::Map<String, serde_json::Value>> {
        self.session
            .describe(&request)
            .unwrap_or_else(|error| panic!("{:?}: describe {kind} {what}: {error}", self.product))
            .items
    }

    /// The `describe tables` row for exactly `table`, or a panic naming what
    /// came back instead.
    ///
    /// The row has to be picked out by name rather than taken as the only one:
    /// `getTables` takes its table argument as a `LIKE` pattern, and every name
    /// this file makes has underscores in it — each of which matches any single
    /// character, so a request for `rb_parent_9_0` can quite legitimately come
    /// back with somebody else's table beside it.
    fn table_item(&self, table: &str) -> serde_json::Map<String, serde_json::Value> {
        let items = self.describe_table("tables", table);
        items
            .iter()
            .find(|item| string(item, "name") == table)
            .unwrap_or_else(|| {
                panic!(
                    "{:?}: describe tables did not list {table}: {items:?}",
                    self.product
                )
            })
            .clone()
    }

    /// One table's columns, as the catalogue holds them, in ordinal order.
    fn columns(&self, table: &str) -> Vec<CatalogColumn> {
        let mut columns: Vec<CatalogColumn> = self
            .describe_table("columns", table)
            .iter()
            .map(|item| {
                // The wire contract, asserted where every reader of it passes:
                // the key is always there, carrying a value or a JSON null. A
                // driver with no REMARKS column would otherwise read as "no
                // comment", and one with no ORDINAL_POSITION as "column 0".
                for key in ["remarks", "ordinal", "is_nullable", "data_type"] {
                    assert!(
                        item.contains_key(key),
                        "{:?}: no `{key}` in a columns item: {item:?}",
                        self.product
                    );
                }
                CatalogColumn {
                    name: string(item, "name"),
                    type_name: string(item, "type_name").to_lowercase(),
                    nullable: item.get("is_nullable").and_then(serde_json::Value::as_bool),
                    ordinal: item
                        .get("ordinal")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                }
            })
            .collect();
        columns.sort_by_key(|column| column.ordinal);
        columns
    }

    /// The primary key's columns, in key order.
    fn primary_key(&self, table: &str) -> Vec<String> {
        let mut rows: Vec<(i64, String)> = self
            .describe_table("primary_keys", table)
            .iter()
            .map(|item| {
                (
                    item.get("seq")
                        .and_then(serde_json::Value::as_i64)
                        .unwrap_or(0),
                    string(item, "column"),
                )
            })
            .collect();
        rows.sort();
        rows.into_iter().map(|(_, column)| column).collect()
    }

    /// Every foreign key of `table`, as `(this column, referenced table,
    /// referenced column)`, sorted.
    fn references(&self, table: &str) -> Vec<(String, String, String)> {
        let mut rows: Vec<_> = self
            .describe_table("imported_keys", table)
            .iter()
            .map(|item| {
                (
                    string(item, "fk_column"),
                    string(item, "pk_table"),
                    string(item, "pk_column"),
                )
            })
            .collect();
        rows.sort();
        rows
    }

    /// Whether `table` carries a unique index called `name`.
    fn has_unique_index(&self, table: &str, name: &str) -> bool {
        self.describe_table("indexes", table).iter().any(|item| {
            item.get("name").and_then(serde_json::Value::as_str) == Some(name)
                && item.get("non_unique").and_then(serde_json::Value::as_bool) == Some(false)
        })
    }
}

/// Opens one connection to `product` at `url`.
fn connect(product: Product, url: &str) -> Session {
    let (user, password) = product.credentials();
    let spec = ConnectionSpec::new(url.to_string(), product.driver_class())
        .with_credentials(user, password)
        .with_jars([product.jar()]);
    Session::open(jvm(), &spec).unwrap_or_else(|error| {
        panic!("{} is set but does not connect: {error}", product.url_var())
    })
}

/// A column as the catalogue reports it, which is the only reading these tests
/// trust.
#[derive(Debug, Clone, PartialEq, Eq)]
struct CatalogColumn {
    name: String,
    /// Lower-cased: PostgreSQL says `int4` and MySQL says `INT`, and the case
    /// is the driver's habit rather than anything worth asserting.
    type_name: String,
    /// `IS_NULLABLE`, as JDBC's tri-state. `None` is "the driver does not
    /// know", which none of these five ever answers.
    nullable: Option<bool>,
    /// `ORDINAL_POSITION`. The template model numbers columns by it, so a
    /// driver that answered them out of order would reorder a generated class.
    ordinal: i64,
}

/// A string field of a describe item, or a panic.
fn string(item: &serde_json::Map<String, serde_json::Value>, key: &str) -> String {
    item.get(key)
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("no `{key}` in {item:?}"))
        .to_string()
}

/// The first column of the first row of `sql`, as text.
fn scalar(session: &Session, sql: &str) -> String {
    let batch = session
        .execute(&StatementSpec::new(sql.to_string()))
        .unwrap_or_else(|error| panic!("{sql}: {error}"))
        .fetch(200)
        .expect("the batch decodes");
    text(&batch, 0, 0).unwrap_or_else(|| panic!("{sql} answered NULL"))
}

/// One cell, as text.
fn text(batch: &Batch, row: usize, column: usize) -> Option<String> {
    match batch.value(row, column)? {
        Value::Null => None,
        Value::Str(text) => Some(text.to_string()),
        Value::I64(value) => Some(value.to_string()),
        other => Some(format!("{other:?}")),
    }
}

/// Opens the product's server, or returns from the test having said why not.
///
/// A macro rather than a function because the `return` has to happen in the
/// test body: a skipped test is one that passed by doing nothing, and that is
/// what keeps this file out of the way of a checkout with no Docker.
macro_rules! server {
    ($product:expr) => {
        match Server::open($product) {
            Some(server) => server,
            None => return,
        }
    };
}

// --- the fixture -----------------------------------------------------------

/// A parent, a child that references it, and a unique index — spelled in the
/// subset of SQL all five products accept unchanged.
///
/// `integer` and `varchar(n)` are synonyms every one of them takes, and the key
/// and reference clauses are written out as table constraints rather than
/// inline: MySQL parses a column-level `REFERENCES` and then silently ignores
/// it, which would make [`imported_keys_carry_both_ends_of_the_reference`] pass
/// against nothing at all on two of the five.
fn create_fixture(server: &Server) -> (String, String, String) {
    let parent = server.table("parent");
    let child = server.table("child");
    server.exec(&format!(
        "create table {parent} (\
           id integer not null, \
           code varchar(20) not null, \
           label varchar(40), \
           primary key (id))"
    ));
    server.exec(&format!(
        "create table {child} (\
           id integer not null, \
           parent_id integer not null, \
           note varchar(40), \
           primary key (id), \
           foreign key (parent_id) references {parent} (id))"
    ));
    let unique = server.name(&format!("{parent}_ux"));
    server.exec(&format!("create unique index {unique} on {parent} (code)"));
    (parent, child, unique)
}

/// Everything the template model is built from, read back off one server.
fn metadata_round_trip(server: &Server) {
    let (parent, child, unique) = create_fixture(server);

    // `schemas` and `catalogs`: whichever of the two this product files a table
    // under has to list the one the session is actually in, or every narrowed
    // request below would be narrowing to a name the driver never answers.
    if let Some(schema) = &server.schema {
        let names: Vec<String> = server
            .describe("schemas")
            .iter()
            .map(|item| string(item, "name"))
            .collect();
        assert!(
            names.contains(schema),
            "{:?}: getSchemas does not list {schema}: {names:?}",
            server.product
        );
    }
    if let Some(catalog) = &server.catalog {
        let names: Vec<String> = server
            .describe("catalogs")
            .iter()
            .map(|item| string(item, "name"))
            .collect();
        assert!(
            names.contains(catalog),
            "{:?}: getCatalogs does not list {catalog}: {names:?}",
            server.product
        );
    }

    // `tables`: the explorer tree draws the type, and the *views* toggle filters
    // on it, so a product that answered something other than TABLE here would
    // hide every table behind that toggle.
    let item = server.table_item(&parent);
    assert_eq!(
        item.get("type").and_then(serde_json::Value::as_str),
        Some("TABLE"),
        "{:?}: {item:?}",
        server.product
    );
    assert!(
        item.contains_key("remarks"),
        "{:?}: no `remarks` in a tables item: {item:?}",
        server.product
    );

    // `columns`: names, order and nullability, which is most of what a template
    // renders per column.
    let columns = server.columns(&child);
    let names: Vec<&str> = columns.iter().map(|column| column.name.as_str()).collect();
    assert_eq!(
        names,
        [
            server.name("id"),
            server.name("parent_id"),
            server.name("note")
        ]
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>(),
        "{:?}",
        server.product
    );
    assert_eq!(columns[0].nullable, Some(false), "{:?}", server.product);
    assert_eq!(columns[2].nullable, Some(true), "{:?}", server.product);
    assert!(
        !columns[2].type_name.is_empty(),
        "{:?}: a column with no type name",
        server.product
    );
    // Ordinals are one-based in JDBC and the model relies on it: a zero-based
    // driver would put `${no}` one out on every generated file.
    assert_eq!(columns[0].ordinal, 1, "{:?}", server.product);

    // `primary_keys`: in KEY_SEQ order, which is what makes a composite key
    // usable at all.
    assert_eq!(
        server.primary_key(&child),
        vec![server.name("id")],
        "{:?}",
        server.product
    );

    // `imported_keys`: both ends, which is the whole of D8.
    assert_eq!(
        server.references(&child),
        vec![(server.name("parent_id"), parent.clone(), server.name("id"))],
        "{:?}",
        server.product
    );
    // And the child has no exported keys of its own — the direction really is
    // a different query and not the same rows relabelled.
    assert!(
        server.describe_table("exported_keys", &child).is_empty(),
        "{:?}: the child is referenced by nothing",
        server.product
    );

    // `indexes`: uniqueness, which the inspector shows and a template can key
    // a lookup method off.
    assert!(
        server.has_unique_index(&parent, &unique),
        "{:?}: {unique} is missing or not unique in {:?}",
        server.product,
        server.describe_table("indexes", &parent)
    );
}

// --- one test per product --------------------------------------------------

#[test]
fn postgres_answers_every_metadata_kind_the_model_needs() {
    let server = server!(Product::Postgres);
    metadata_round_trip(&server);
}

#[test]
fn mysql_answers_every_metadata_kind_the_model_needs() {
    let server = server!(Product::MySql);
    metadata_round_trip(&server);
}

#[test]
fn mariadb_answers_every_metadata_kind_the_model_needs() {
    let server = server!(Product::MariaDb);
    metadata_round_trip(&server);
}

#[test]
fn mssql_answers_every_metadata_kind_the_model_needs() {
    let server = server!(Product::MsSql);
    metadata_round_trip(&server);
}

#[test]
fn oracle_answers_every_metadata_kind_the_model_needs() {
    let server = server!(Product::Oracle);
    metadata_round_trip(&server);
}
