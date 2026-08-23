//! The three templates jdbgen ships, rendered against a fixed table and
//! compared byte for byte with what jdbgen itself wrote.
//!
//! `tests/golden/*.expected` was captured once by running jdbgen's own
//! `TemplateManager` over the same fixture; the harness that did it is checked
//! in beside them as `GoldenMain.java.txt`, so the files can be regenerated
//! against a future jdbgen. This is the compatibility canary of the port: the
//! ported unit tests say the engine follows the rules, these files say it
//! renders the assets.

use std::borrow::Cow;

use rudbgen_template::chrono::NaiveDate;
use rudbgen_template::{Model, RenderContext, Template, Value};

/// A column of the metadata, with the members jdbgen's `DBColumn` exposes to a
/// template.
struct Column {
    column: String,
    name: String,
    type_name: String,
    length: i64,
    nullable: i64,
    remarks: String,
    java_type: String,
    is_key: bool,
}

impl Model for Column {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "column" => Value::Str(Cow::Borrowed(&self.column)),
            "name" => Value::Str(Cow::Borrowed(&self.name)),
            "typeName" => Value::Str(Cow::Borrowed(&self.type_name)),
            "length" => Value::Int(self.length),
            "nullable" => Value::Int(self.nullable),
            "remarks" => Value::Str(Cow::Borrowed(&self.remarks)),
            "javaType" => Value::Str(Cow::Borrowed(&self.java_type)),
            "isKey" => Value::Bool(self.is_key),
            _ => return None,
        })
    }
}

/// A table of the metadata, holding its columns once and naming the primary
/// key ones by position.
struct Table {
    name: String,
    table: String,
    schema: String,
    kind: String,
    remarks: String,
    columns: Vec<Column>,
}

impl Table {
    fn subset(&self, keys: bool) -> Value<'_> {
        Value::List(
            self.columns
                .iter()
                .filter(|c| c.is_key == keys)
                .map(|c| c as &dyn Model)
                .collect(),
        )
    }
}

impl Model for Table {
    fn get(&self, key: &str) -> Option<Value<'_>> {
        Some(match key {
            "name" => Value::Str(Cow::Borrowed(&self.name)),
            "table" => Value::Str(Cow::Borrowed(&self.table)),
            "schema" => Value::Str(Cow::Borrowed(&self.schema)),
            "type" => Value::Str(Cow::Borrowed(&self.kind)),
            "remarks" => Value::Str(Cow::Borrowed(&self.remarks)),
            "columns" => Value::List(self.columns.iter().map(|c| c as &dyn Model).collect()),
            "keys" => self.subset(true),
            "notKeys" => self.subset(false),
            _ => return None,
        })
    }
}

fn column(
    name: &str,
    type_name: &str,
    length: i64,
    nullable: i64,
    remarks: &str,
    java_type: &str,
    is_key: bool,
) -> Column {
    Column {
        column: name.to_string(),
        name: name.to_string(),
        type_name: type_name.to_string(),
        length,
        nullable,
        remarks: remarks.to_string(),
        java_type: java_type.to_string(),
        is_key,
    }
}

/// The fixture of `GoldenMain.java`, member for member.
fn fixture() -> Table {
    Table {
        name: "TB_USER_ACCOUNT".to_string(),
        table: "TB_USER_ACCOUNT".to_string(),
        schema: "PUBLIC".to_string(),
        kind: "TABLE".to_string(),
        remarks: "사용자 계정".to_string(),
        columns: vec![
            column("USER_ID", "VARCHAR", 20, 0, "사용자 ID", "String", true),
            column(
                "ACCT_NO",
                "DECIMAL",
                18,
                0,
                "계정 번호",
                "java.math.BigDecimal",
                true,
            ),
            column("USER_NAME", "VARCHAR", 50, 1, "이름", "String", false),
            column(
                "REG_DATE",
                "TIMESTAMP",
                0,
                1,
                "등록 일시",
                "java.sql.Timestamp",
                false,
            ),
            column(
                "UPDT_DT",
                "TIMESTAMP",
                0,
                0,
                "수정 일시",
                "java.sql.Timestamp",
                false,
            ),
        ],
    }
}

/// The context the golden files were rendered with: the clock of `now.txt` and
/// the user the harness was run as.
fn context() -> RenderContext {
    RenderContext::new()
        .with_var("author", "John Doe")
        .with_user("tester")
        .with_now(
            NaiveDate::from_ymd_opt(2026, 8, 23)
                .unwrap()
                .and_hms_opt(10, 22, 39)
                .unwrap(),
        )
}

fn assert_golden(name: &str) {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
    let source = std::fs::read_to_string(format!("{dir}/{name}")).expect("template is checked in");
    let expected =
        std::fs::read_to_string(format!("{dir}/{name}.expected")).expect("golden is checked in");

    let template = Template::parse(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
    let rendered = template
        .render(&fixture(), &context())
        .unwrap_or_else(|e| panic!("{name}: {e}"));

    assert_eq!(
        rendered, expected,
        "{name} does not render what jdbgen renders"
    );
}

#[test]
fn the_java_model_template_renders_what_jdbgen_renders() {
    assert_golden("java_model.java");
}

#[test]
fn the_mybatis_mapper_template_renders_what_jdbgen_renders() {
    assert_golden("mybatis_mapper.xml");
}

#[test]
fn the_php_ci_template_renders_what_jdbgen_renders() {
    assert_golden("php_ci.php");
}

#[test]
fn the_shipped_templates_name_the_members_they_read() {
    let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/golden");
    let source = std::fs::read_to_string(format!("{dir}/mybatis_mapper.xml")).unwrap();
    let fields = Template::parse(&source).unwrap().fields_referenced();

    // this is what the variable palette of the editor lists
    for expected in [
        "name", "remarks", "columns", "column", "table", "keys", "notKeys",
    ] {
        assert!(
            fields.iter().any(|f| f == expected),
            "{expected} is missing from {fields:?}"
        );
    }
}
