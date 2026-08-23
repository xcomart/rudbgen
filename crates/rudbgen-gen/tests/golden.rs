//! The compatibility canary of M3: the three templates jdbgen ships, run
//! through a whole generation job against the **metadata model** and compared
//! byte for byte with what jdbgen itself wrote.
//!
//! `rudbgen-template`'s own golden test proves the engine renders the assets.
//! It does that against a fixture written for the test, so it says nothing
//! about the model a real run uses. This one closes that gap: the same
//! templates, the same expected bytes, but a hand-built
//! [`rudbgen_meta::Table`] and the real [`generate`] going all the way to a
//! file on disk. If the two models ever disagree — a member spelled
//! differently, a list in another order, a number rendered another way — this
//! is what says so.
//!
//! The fixture mirrors `crates/rudbgen-template/tests/golden.rs` member for
//! member, with one thing set by hand: `javaType`. The metadata reader derives
//! it from `java.sql.Types` through jdbgen's own table, where `DECIMAL` is
//! `Integer` and `TIMESTAMP` is `String` — deliberate oddities kept for
//! template compatibility (`rudbgen_meta::sqltypes`). jdbgen's `TestResultSet`
//! fixture, which the `.expected` files were captured from, carries
//! `java.math.BigDecimal` and `java.sql.Timestamp` instead, because it names
//! the Java types directly rather than deriving them. So `derive()` is not
//! called here and the four derived members are set to the fixture's values;
//! everything else is what a `DESCRIBE` would produce.

mod common;

use std::path::Path;

use common::{live, tree};
use rudbgen_gen::{Overwrite, Plan, Progress, TemplateSpec, generate, preview};
use rudbgen_meta::{Column, KIND_TABLE, Table};
use rudbgen_template::chrono::NaiveDate;

/// Where the three templates and their expected output are checked in.
fn golden_dir() -> &'static Path {
    Path::new(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../rudbgen-template/tests/golden"
    ))
}

/// One column of the fixture, as `DESCRIBE` would report it.
///
/// `java_type` is passed in rather than derived; see the module documentation.
fn column(name: &str, type_name: &str, length: i64, nullable: i64) -> Column {
    Column {
        catalog: String::new(),
        schema: "PUBLIC".to_string(),
        table: "TB_USER_ACCOUNT".to_string(),
        name: name.to_string(),
        type_name: type_name.to_string(),
        length,
        precision: length,
        nullable,
        ..Column::default()
    }
}

/// The fixture of `GoldenMain.java`, in the metadata model.
///
/// `no` is filled in by position, the way the reader numbers a column list.
fn fixture() -> Table {
    let columns = vec![
        Column {
            remarks: "사용자 ID".to_string(),
            java_type: "String".to_string(),
            key_seq: Some(1),
            ..column("USER_ID", "VARCHAR", 20, 0)
        },
        Column {
            remarks: "계정 번호".to_string(),
            java_type: "java.math.BigDecimal".to_string(),
            key_seq: Some(2),
            ..column("ACCT_NO", "DECIMAL", 18, 0)
        },
        Column {
            remarks: "이름".to_string(),
            java_type: "String".to_string(),
            ..column("USER_NAME", "VARCHAR", 50, 1)
        },
        Column {
            remarks: "등록 일시".to_string(),
            java_type: "java.sql.Timestamp".to_string(),
            ..column("REG_DATE", "TIMESTAMP", 0, 1)
        },
        Column {
            remarks: "수정 일시".to_string(),
            java_type: "java.sql.Timestamp".to_string(),
            ..column("UPDT_DT", "TIMESTAMP", 0, 0)
        },
    ];

    Table {
        catalog: String::new(),
        schema: "PUBLIC".to_string(),
        name: "TB_USER_ACCOUNT".to_string(),
        kind: KIND_TABLE.to_string(),
        remarks: "사용자 계정".to_string(),
        columns: columns
            .into_iter()
            .enumerate()
            .map(|(index, column)| Column {
                no: index + 1,
                ..column
            })
            .collect(),
        no: 1,
        ..Table::default()
    }
}

/// The three shipped templates, writing to a name of their own.
fn templates() -> Vec<TemplateSpec> {
    ["java_model.java", "mybatis_mapper.xml", "php_ci.php"]
        .into_iter()
        .map(|name| TemplateSpec::new(name, golden_dir().join(name), name))
        .collect()
}

/// The plan the `.expected` files were captured under: jdbgen's harness ran as
/// `tester`, with `author` set to `John Doe` and the clock of `now.txt`.
fn plan(out: &Path) -> Plan {
    let mut plan = Plan::new(vec![fixture()], templates(), out).with_clock(
        NaiveDate::from_ymd_opt(2026, 8, 23)
            .unwrap()
            .and_hms_opt(10, 22, 39)
            .unwrap(),
        "tester",
    );
    plan.author = "John Doe".to_string();
    plan
}

#[test]
fn a_whole_run_over_the_metadata_model_writes_what_jdbgen_wrote() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out");
    let plan = plan(&out);

    let outcome = generate(&plan, Overwrite::Overwrite, &live(), &|_: Progress| {});

    assert!(outcome.is_ok(), "{outcome:?}");
    assert!(
        outcome.diagnostics.is_empty(),
        "the shipped templates name a member the model does not answer: {:?}",
        outcome.diagnostics
    );
    assert_eq!(
        tree(&out),
        ["java_model.java", "mybatis_mapper.xml", "php_ci.php"]
    );

    for name in ["java_model.java", "mybatis_mapper.xml", "php_ci.php"] {
        let expected = std::fs::read(golden_dir().join(format!("{name}.expected")))
            .expect("the golden file is checked in");
        let written = std::fs::read(out.join(name)).expect("the run wrote it");

        assert_eq!(
            String::from_utf8_lossy(&written),
            String::from_utf8_lossy(&expected),
            "{name} is not what jdbgen wrote"
        );
        assert_eq!(written, expected, "{name} differs in its bytes");
    }
}

#[test]
fn the_preview_of_a_pair_is_the_file_the_run_would_write() {
    let dir = tempfile::tempdir().expect("tempdir");
    let out = dir.path().join("out");
    let plan = plan(&out);

    for (index, name) in ["java_model.java", "mybatis_mapper.xml", "php_ci.php"]
        .into_iter()
        .enumerate()
    {
        let preview = preview(&plan, 0, index).expect("a pair that exists");
        let expected = std::fs::read_to_string(golden_dir().join(format!("{name}.expected")))
            .expect("the golden file is checked in");
        assert_eq!(preview.content, expected, "{name}");
        assert_eq!(preview.path, out.join(name));
    }
}
