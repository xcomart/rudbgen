//! The model, read from a real H2 through a real JVM and the real bridge JAR.
//!
//! The unit tests prove the derivations. Only this file proves the thing the
//! crate exists for: that what a driver reports through `DESCRIBE`, and what a
//! custom statement reports through `EXECUTE`, becomes a table a template can
//! be rendered against.
//!
//! # One JVM, many test threads
//!
//! JNI allows one VM per process and `cargo test` runs this file's tests as
//! threads of one process, so [`Jvm::start`] is idempotent: whichever test gets
//! there first builds it. Each test then opens its own session against its own
//! database.
//!
//! # The H2 driver
//!
//! Looked up in `RUDBGEN_TEST_H2_JAR` and then in the Gradle cache the bridge's
//! own suite fills — and **not** silently skipped when it is missing. A test
//! that passes because it could not find the thing it tests is worse than no
//! test at all. The helpers below are the same ones `rudbgen-jdbc`'s H2 suite
//! uses; they are copied rather than shared because a test helper is not API
//! and a second copy is cheaper than publishing one.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

use rudbgen_core::{CustomQuery, DriverDef};
use rudbgen_jdbc::{ConnectionSpec, Jvm, JvmConfig, Session, StatementSpec, default_bridge_jar};
use rudbgen_meta::{Error, MetaReader, Schema, Table};
use rudbgen_template::{RenderContext, Template};

/// The process-wide JVM, started by whichever test needs it first.
fn jvm() -> &'static Jvm {
    Jvm::start(&JvmConfig::new(default_bridge_jar()).with_heap_mb(256))
        .expect("the JVM must start; build the bridge with `cd bridge && ./gradlew jar`")
}

/// Locates the H2 driver JAR, or fails with instructions.
fn h2_jar() -> PathBuf {
    if let Some(path) = std::env::var_os("RUDBGEN_TEST_H2_JAR") {
        let path = PathBuf::from(path);
        assert!(
            path.is_file(),
            "RUDBGEN_TEST_H2_JAR points at {}, which is not a file",
            path.display()
        );
        return path;
    }
    find_in_gradle_cache().unwrap_or_else(|| {
        panic!(
            "the H2 driver JAR was not found.\n\
             \n\
             These tests need it: they read metadata out of a real database.\n\
             Fetch it into the Gradle cache by running the bridge's own suite:\n\
             \n    cd bridge && ./gradlew test\n\
             \n\
             or point RUDBGEN_TEST_H2_JAR at an h2-*.jar you already have."
        )
    })
}

/// Walks `<gradle home>/caches/modules-2/files-2.1/com.h2database/h2/*/*/h2-*.jar`.
fn find_in_gradle_cache() -> Option<PathBuf> {
    let gradle_home = std::env::var_os("GRADLE_USER_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".gradle")))?;
    let root = gradle_home.join("caches/modules-2/files-2.1/com.h2database/h2");

    let mut newest: Option<(String, PathBuf)> = None;
    for version in std::fs::read_dir(&root).ok()?.flatten() {
        let number = version.file_name().to_string_lossy().into_owned();
        // Only the binary artefact: the same directory holds the javadoc and
        // sources archives, and loading one of those gets a class loader with
        // no driver in it.
        let wanted = format!("h2-{number}.jar");
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

/// A URL for a database no other test shares.
fn fresh_url() -> String {
    static SEQ: AtomicU32 = AtomicU32::new(0);
    format!(
        "jdbc:h2:mem:rudbgenmeta{};DB_CLOSE_DELAY=-1",
        SEQ.fetch_add(1, Ordering::Relaxed)
    )
}

/// Runs a statement that returns no rows.
fn exec(session: &Session, sql: &str) {
    session
        .execute(&StatementSpec::new(sql))
        .unwrap_or_else(|error| panic!("{sql}: {error}"));
}

/// A session against a fresh database holding the fixture schema.
///
/// One album, one track referencing it, one view, one index, and comments on
/// three of them. The primary key of `T_TRACK` is composite and is declared in
/// the order the driver does *not* report it in — `getPrimaryKeys` answers by
/// column name — which is what makes the key order testable at all.
fn fixture() -> Session {
    let session = Session::open(
        jvm(),
        &ConnectionSpec::new(fresh_url(), "org.h2.Driver")
            .with_credentials("sa", "")
            .with_jars([h2_jar()]),
    )
    .expect("H2 accepts the connection");

    exec(
        &session,
        "create table T_ALBUM (\
           ID integer not null,\
           TITLE varchar(120) not null,\
           RELEASED date,\
           constraint PK_ALBUM primary key (ID))",
    );
    exec(&session, "comment on table T_ALBUM is 'an album'");
    exec(&session, "comment on column T_ALBUM.ID is 'the id'");
    exec(&session, "comment on column T_ALBUM.TITLE is 'the title'");
    exec(
        &session,
        "create table T_TRACK (\
           ALBUM_ID integer not null,\
           TRACK_NO integer not null,\
           NAME varchar(200),\
           DURATION_S integer default 0,\
           constraint PK_TRACK primary key (TRACK_NO, ALBUM_ID),\
           constraint FK_TRACK_ALBUM foreign key (ALBUM_ID) \
             references T_ALBUM(ID) on delete cascade)",
    );
    exec(&session, "comment on table T_TRACK is 'a track'");
    exec(&session, "create index IX_TRACK_NAME on T_TRACK(NAME)");
    exec(
        &session,
        "create view V_ALBUM as select ID, TITLE from T_ALBUM",
    );
    session
}

/// The `PUBLIC` schema of a reader's database.
fn public(reader: &MetaReader<'_>) -> Schema {
    reader
        .schemas()
        .expect("H2 answers its schemas")
        .into_iter()
        .find(|schema| schema.schema == "PUBLIC")
        .expect("H2 always has a PUBLIC schema")
}

/// The loaded table of that name.
fn table(reader: &MetaReader<'_>, name: &str) -> Table {
    let schema = public(reader);
    let entry = reader
        .tables(&schema, true)
        .expect("the table list")
        .into_iter()
        .find(|entry| entry.name == name)
        .unwrap_or_else(|| panic!("{name} is in the fixture"));
    reader.table(&entry).expect("the table loads")
}

/// A driver definition with one custom query switched on.
fn with_query(set: impl FnOnce(&mut DriverDef, CustomQuery)) -> impl FnOnce(&str) -> DriverDef {
    move |sql| {
        let mut driver = DriverDef::default();
        set(&mut driver, CustomQuery::on(sql));
        driver
    }
}

/// The stock `h2-embedded` definition, custom table query and all.
fn h2_builtin() -> DriverDef {
    DriverDef::builtins()
        .into_iter()
        .find(|driver| driver.id == "h2-embedded")
        .expect("H2 Embedded is a built-in")
}

// --- the schema tree -------------------------------------------------------

#[test]
fn the_schema_tree_carries_a_catalog_and_is_never_empty() {
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);

    let schemas = reader.schemas().expect("H2 answers its schemas");
    assert!(!schemas.is_empty(), "the list is never empty");
    let names: Vec<&str> = schemas.iter().map(|s| s.name.as_str()).collect();
    assert!(names.contains(&"PUBLIC"), "{names:?}");
    assert!(names.contains(&"INFORMATION_SCHEMA"), "{names:?}");

    let public = public(&reader);
    assert_eq!(public.schema, public.name);
    assert!(
        !public.catalog.is_empty(),
        "H2 reports a catalog and it has to reach the schema: {public:?}"
    );
}

// --- tables and views ------------------------------------------------------

#[test]
fn views_are_listed_only_when_they_are_asked_for() {
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);
    let schema = public(&reader);

    let tables = reader.tables(&schema, false).expect("the table list");
    let names: Vec<&str> = tables.iter().map(|entry| entry.name.as_str()).collect();
    assert!(
        names.contains(&"T_ALBUM") && names.contains(&"T_TRACK"),
        "{names:?}"
    );
    assert!(
        !names.contains(&"V_ALBUM"),
        "a view is not a table: {names:?}"
    );
    assert!(
        tables.iter().all(|entry| entry.kind == "TABLE"),
        "{tables:?}"
    );
    // One-based and gapless, whatever the driver dropped on the way.
    let numbers: Vec<usize> = tables.iter().map(|entry| entry.no).collect();
    assert_eq!(numbers, (1..=tables.len()).collect::<Vec<_>>());

    let with_views = reader.tables(&schema, true).expect("the table list");
    let view = with_views
        .iter()
        .find(|entry| entry.name == "V_ALBUM")
        .expect("the view is listed now");
    assert_eq!(view.kind, "VIEW");
    assert_eq!(view.icon(), "fa:EYE");
    assert!(view.is_view());

    let album = with_views
        .iter()
        .find(|entry| entry.name == "T_ALBUM")
        .expect("the table is still listed");
    assert_eq!(album.remarks, "an album", "H2 reports its own comments");
    assert_eq!(album.icon(), "fa:TABLE");
}

// --- columns, keys, foreign keys, indexes ----------------------------------

#[test]
fn the_columns_arrive_in_order_numbered_and_with_their_types_derived() {
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);
    let track = table(&reader, "T_TRACK");

    let names: Vec<&str> = track.columns.iter().map(|c| c.name.as_str()).collect();
    assert_eq!(names, vec!["ALBUM_ID", "TRACK_NO", "NAME", "DURATION_S"]);
    let numbers: Vec<usize> = track.columns.iter().map(|c| c.no).collect();
    assert_eq!(numbers, vec![1, 2, 3, 4]);

    let name = track.column("NAME").expect("NAME is a column");
    assert_eq!(name.nullable, 1, "NAME has no NOT NULL");
    assert_eq!(name.length, 200);
    assert_eq!(name.type_string, "CHARACTER VARYING(200)");
    assert!(name.is_char_type);
    assert_eq!(name.jdbc_type, "VARCHAR");
    assert_eq!(name.java_type, "String");
    assert!(!name.is_key());

    let album_id = track.column("ALBUM_ID").expect("ALBUM_ID is a column");
    assert_eq!(album_id.nullable, 0, "ALBUM_ID is NOT NULL");
    assert_eq!(album_id.jdbc_type, "INTEGER");
    assert_eq!(album_id.java_type, "Integer");
    assert!(!album_id.is_char_type);

    let duration = track.column("DURATION_S").expect("DURATION_S is a column");
    assert_eq!(duration.def_val, "0", "the default reaches the model");

    // The comment the fixture put on the table itself.
    let album = table(&reader, "T_ALBUM");
    assert_eq!(album.remarks, "an album");
    assert_eq!(album.column("TITLE").expect("TITLE").remarks, "the title");
}

#[test]
fn a_composite_key_keeps_the_order_it_was_declared_in() {
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);
    let track = table(&reader, "T_TRACK");

    // Declared `primary key (TRACK_NO, ALBUM_ID)`; `getPrimaryKeys` answers by
    // column name, so a reader that kept the driver's order would say the
    // other way round and generate a predicate that does not match the key.
    let keys: Vec<&str> = track.keys().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(keys, vec!["TRACK_NO", "ALBUM_ID"]);
    let rest: Vec<&str> = track.not_keys().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(rest, vec!["NAME", "DURATION_S"]);
    assert_eq!(track.column("TRACK_NO").expect("TRACK_NO").key_seq, Some(1));
    assert_eq!(track.column("ALBUM_ID").expect("ALBUM_ID").key_seq, Some(2));
}

#[test]
fn a_foreign_key_is_seen_from_both_of_its_ends() {
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);

    let track = table(&reader, "T_TRACK");
    assert_eq!(track.imports.len(), 1, "{:?}", track.imports);
    let import = &track.imports[0];
    assert_eq!(import.name, "FK_TRACK_ALBUM");
    assert_eq!(names(&import.columns), vec!["ALBUM_ID"]);
    assert_eq!(import.ref_table, "T_ALBUM");
    assert_eq!(names(&import.ref_columns), vec!["ID"]);
    assert_eq!(import.on_delete, "CASCADE");
    assert!(!import.ref_schema.is_empty(), "the other end is qualified");
    assert!(track.exports.is_empty(), "nothing references a track");

    // The column knows its target without walking the list.
    let fk = track
        .column("ALBUM_ID")
        .expect("ALBUM_ID")
        .fk
        .as_ref()
        .expect("ALBUM_ID is a foreign key");
    assert_eq!((fk.table.as_str(), fk.column.as_str()), ("T_ALBUM", "ID"));
    assert!(track.column("NAME").expect("NAME").fk.is_none());

    // And the same constraint from the parent's side, with the two ends
    // swapped over.
    let album = table(&reader, "T_ALBUM");
    assert!(album.imports.is_empty(), "an album references nothing");
    assert_eq!(album.exports.len(), 1, "{:?}", album.exports);
    let export = &album.exports[0];
    assert_eq!(export.name, "FK_TRACK_ALBUM");
    assert_eq!(names(&export.columns), vec!["ID"]);
    assert_eq!(export.ref_table, "T_TRACK");
    assert_eq!(names(&export.ref_columns), vec!["ALBUM_ID"]);
}

#[test]
fn the_indexes_come_back_by_name_with_their_columns_in_index_order() {
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);
    let track = table(&reader, "T_TRACK");

    let by_name: Vec<&str> = track.indexes.iter().map(|i| i.name.as_str()).collect();
    let mut sorted = by_name.clone();
    sorted.sort_unstable();
    assert_eq!(by_name, sorted, "the list is ordered by name");

    let named = track
        .indexes
        .iter()
        .find(|index| index.name == "IX_TRACK_NAME")
        .unwrap_or_else(|| panic!("the fixture's index is missing: {by_name:?}"));
    assert!(!named.unique);
    assert_eq!(names(&named.columns), vec!["NAME"]);

    // The primary key has an index of its own and the database reports it;
    // a template writing DDL needs to see it.
    let unique = track
        .indexes
        .iter()
        .find(|index| index.unique)
        .unwrap_or_else(|| panic!("the primary key index is missing: {by_name:?}"));
    assert_eq!(names(&unique.columns), vec!["TRACK_NO", "ALBUM_ID"]);
    let numbers: Vec<usize> = track.indexes.iter().map(|index| index.no).collect();
    assert_eq!(numbers, (1..=track.indexes.len()).collect::<Vec<_>>());
}

/// The names of a key or index column list, in list order.
fn names(columns: &[rudbgen_meta::KeyColumn]) -> Vec<&str> {
    columns.iter().map(|column| column.name.as_str()).collect()
}

// --- the custom queries (D9) -----------------------------------------------

#[test]
fn the_stock_h2_table_query_is_used_instead_of_the_driver_list() {
    let session = fixture();
    // Not a statement written for this test: the definition rudbgen ships,
    // unedited, holes and all. H2 answers `BASE TABLE` where the label
    // contract says `TABLE`, and that CASE is why the query exists.
    let driver = h2_builtin();
    assert!(driver.tables_query().is_some(), "the built-in ships it on");
    let reader = MetaReader::new(&session, &driver);
    let schema = public(&reader);

    let tables = reader.tables(&schema, true).expect("the custom table list");
    let names: Vec<&str> = tables.iter().map(|entry| entry.name.as_str()).collect();
    assert!(names.contains(&"T_ALBUM"), "{names:?}");
    assert!(names.contains(&"V_ALBUM"), "{names:?}");

    let album = tables
        .iter()
        .find(|entry| entry.name == "T_ALBUM")
        .expect("T_ALBUM");
    assert_eq!(album.kind, "TABLE", "BASE TABLE has to normalise to TABLE");
    assert_eq!(album.remarks, "an album");
    assert!(!album.catalog.is_empty() && album.schema == "PUBLIC");
    assert_eq!(
        tables
            .iter()
            .find(|entry| entry.name == "V_ALBUM")
            .expect("V_ALBUM")
            .kind,
        "VIEW"
    );

    // And the tables it lists still load: the catalog and schema the statement
    // answered are what the column read is filtered by.
    let loaded = reader.table(album).expect("the table loads");
    assert_eq!(loaded.columns.len(), 3, "{:?}", loaded.columns);
}

/// A column list for H2 2.x, whose `INFORMATION_SCHEMA` follows the standard:
/// `DATA_TYPE` is the type *name* there, so the numeric code the contract
/// wants has to be written out.
const H2_COLUMNS_SQL: &str = "select TABLE_CATALOG as \"TABLE_CAT\", \
     TABLE_SCHEMA as \"TABLE_SCHEM\", TABLE_NAME, COLUMN_NAME, \
     CASE WHEN DATA_TYPE = 'INTEGER' THEN 4 ELSE 12 END as \"DATA_TYPE\", \
     DATA_TYPE as \"TYPE_NAME\", \
     COALESCE(CHARACTER_MAXIMUM_LENGTH, NUMERIC_PRECISION, 0) as \"COLUMN_SIZE\", \
     CASE WHEN IS_NULLABLE = 'YES' THEN 1 ELSE 0 END as \"NULLABLE\", \
     REMARKS, COLUMN_DEFAULT as \"COLUMN_DEF\", {IS_KEY} as \"IS_KEY\" \
     from INFORMATION_SCHEMA.COLUMNS \
     where TABLE_CATALOG = '${catalog}' and TABLE_SCHEMA = '${schema}' \
       and TABLE_NAME = '${table}' order by ORDINAL_POSITION";

#[test]
fn a_custom_column_query_reports_the_key_itself_in_the_order_it_answers() {
    let session = fixture();
    let driver =
        with_query(|driver, query| driver.custom_queries.columns = query)(&H2_COLUMNS_SQL.replace(
            "{IS_KEY}",
            "CASE WHEN COLUMN_NAME IN ('TRACK_NO', 'ALBUM_ID') THEN 1 ELSE 0 END",
        ));
    let reader = MetaReader::new(&session, &driver);
    let track = table(&reader, "T_TRACK");

    // The `DESCRIBE` path answers TRACK_NO, ALBUM_ID — that is KEY_SEQ. Here
    // the key order is the order the *statement* answered in, which is the
    // ordinal one, and that difference is the proof that `getPrimaryKeys` was
    // not consulted at all.
    let keys: Vec<&str> = track.keys().iter().map(|c| c.name.as_str()).collect();
    assert_eq!(keys, vec!["ALBUM_ID", "TRACK_NO"]);

    let name = track.column("NAME").expect("NAME");
    assert_eq!(name.length, 200);
    assert_eq!(name.nullable, 1);
    assert_eq!(name.java_type, "String", "DATA_TYPE 12 reached the model");
    assert!(!name.is_key());
    assert_eq!(track.columns.len(), 4);
    assert_eq!(track.column("ALBUM_ID").expect("ALBUM_ID").no, 1);
}

#[test]
fn a_custom_column_query_answering_yes_and_no_marks_no_key_at_all() {
    // jdbgen reads IS_KEY with `toInt`, so `'Y'` is zero. It is a trap, it is
    // the contract, and a definition carried over from jdbgen was written
    // against it — so it has to behave the same here.
    let session = fixture();
    let driver =
        with_query(|driver, query| driver.custom_queries.columns = query)(&H2_COLUMNS_SQL.replace(
            "{IS_KEY}",
            "CASE WHEN COLUMN_NAME IN ('TRACK_NO', 'ALBUM_ID') THEN 'Y' ELSE 'N' END",
        ));
    let reader = MetaReader::new(&session, &driver);
    let track = table(&reader, "T_TRACK");

    assert!(track.keys().is_empty(), "{:?}", track.keys());
    assert_eq!(track.not_keys().len(), 4);
}

#[test]
fn a_comment_query_only_touches_the_names_it_returns() {
    // jdbgen overwrote every comment with the answer of this query, so a
    // statement naming one column blanked all the others. Not reproduced
    // (architecture.md §6), and this is what says so.
    let session = fixture();
    let mut driver = DriverDef::default();
    driver.custom_queries.table_comments =
        CustomQuery::on("select 'T_ALBUM' as N, 'from the query' as C");
    driver.custom_queries.column_comments =
        CustomQuery::on("select 'TITLE' as N, 'a better title' as C");
    let reader = MetaReader::new(&session, &driver);

    let schema = public(&reader);
    let tables = reader.tables(&schema, false).expect("the table list");
    let comment = |name: &str| {
        tables
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name}"))
            .remarks
            .clone()
    };
    assert_eq!(comment("T_ALBUM"), "from the query", "the named one wins");
    assert_eq!(
        comment("T_TRACK"),
        "a track",
        "a table the query did not name keeps the driver's comment"
    );

    let album = table(&reader, "T_ALBUM");
    assert_eq!(
        album.column("TITLE").expect("TITLE").remarks,
        "a better title"
    );
    assert_eq!(
        album.column("ID").expect("ID").remarks,
        "the id",
        "a column the query did not name keeps its comment"
    );
}

#[test]
fn a_custom_query_missing_a_label_says_which_one() {
    let session = fixture();
    // Everything but TABLE_SCHEM, which is where jdbgen would have failed the
    // whole run with nothing to look at.
    let driver = with_query(|driver, query| driver.custom_queries.tables = query)(
        "select 'A' as TABLE_CAT, 'B' as TABLE_NAME, 'TABLE' as TABLE_TYPE, '' as REMARKS",
    );
    let reader = MetaReader::new(&session, &driver);
    let schema = public(&reader);

    let error = reader
        .tables(&schema, false)
        .expect_err("a table list without TABLE_SCHEM is not a table list");
    match &error {
        Error::MissingLabel { label, .. } => assert_eq!(label, "TABLE_SCHEM"),
        other => panic!("{other:?}"),
    }
    let message = error.to_string();
    assert!(message.contains("TABLE_SCHEM"), "{message}");
    assert!(message.contains("table-list"), "{message}");
}

#[test]
fn a_comment_query_with_one_column_is_refused_before_it_is_read() {
    let session = fixture();
    let driver = with_query(|driver, query| driver.custom_queries.table_comments = query)(
        "select 'T_ALBUM' as N",
    );
    let reader = MetaReader::new(&session, &driver);
    let schema = public(&reader);

    let error = reader
        .tables(&schema, false)
        .expect_err("a comment query is read positionally and needs two columns");
    match &error {
        Error::Shape {
            expected, found, ..
        } => assert_eq!((*expected, *found), (2, 1)),
        other => panic!("{other:?}"),
    }
}

// --- end to end ------------------------------------------------------------

#[test]
fn a_loaded_table_renders_the_template_that_ships_with_rudbgen() {
    // The point of the crate, in one test: metadata read from a database, a
    // shipped template parsed, and a Java file coming out of the two without
    // anything in between.
    let session = fixture();
    let driver = DriverDef::default();
    let reader = MetaReader::new(&session, &driver);
    let track = table(&reader, "T_TRACK");

    let source = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rudbgen-template/tests/golden/java_model.java"
    ))
    .expect("the shipped template is checked in beside the engine");
    let template = Template::parse(&source).expect("the shipped template parses");
    let rendered = template
        .render(&track, &RenderContext::new().with_user("tester"))
        .expect("a loaded table answers everything the template asks for");

    // `${name.suffix.pascal}` of T_TRACK.
    assert!(rendered.contains("public class TrackModel"), "{rendered}");
    assert!(
        rendered.contains("package com.abc.sample.track;"),
        "{rendered}"
    );
    assert!(rendered.contains("@Alias(\"track\")"), "{rendered}");
    assert!(rendered.contains("@author tester"), "{rendered}");
    // A key column, its javaType padded to ten columns, and `${name.camel}`.
    assert!(
        rendered.contains("private Integer    trackNo;"),
        "{rendered}"
    );
    assert!(
        rendered.contains("private Integer    albumId;"),
        "{rendered}"
    );
    // A non-key column, and the `@Size` its `startsWith=char` branch adds.
    assert!(rendered.contains("private String     name;"), "{rendered}");
    assert!(
        rendered.contains("@Size(max=200"),
        "the character type carries its length: {rendered}"
    );
    // Nothing was left unanswered.
    assert!(!rendered.contains("${"), "{rendered}");
}
