#![allow(dead_code)] // each test binary uses a different part of it

//! The fixture the run tests share: a temporary output tree, a couple of
//! tables, and templates written to disk.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rudbgen_gen::{CancelToken, Plan, Progress, TemplateSpec};
use rudbgen_meta::{Column, Table};
use rudbgen_template::chrono::NaiveDate;

/// A table with two columns, one of them the key.
pub fn table(name: &str) -> Table {
    let mut key = Column {
        name: "ID".to_string(),
        type_name: "VARCHAR".to_string(),
        length: 20,
        nullable: 0,
        key_seq: Some(1),
        data_type: 12,
        no: 1,
        ..Column::default()
    };
    key.derive();
    let mut value = Column {
        name: "USER_NAME".to_string(),
        type_name: "VARCHAR".to_string(),
        length: 50,
        nullable: 1,
        data_type: 12,
        no: 2,
        ..Column::default()
    };
    value.derive();

    Table {
        schema: "PUBLIC".to_string(),
        name: name.to_string(),
        kind: rudbgen_meta::KIND_TABLE.to_string(),
        remarks: format!("the {name} table"),
        columns: vec![key, value],
        no: 1,
        ..Table::default()
    }
}

/// Write a template file into `dir` and name it in a spec.
pub fn template(dir: &Path, file: &str, body: &str, out_template: &str) -> TemplateSpec {
    let path = dir.join(file);
    std::fs::write(&path, body).expect("template file");
    TemplateSpec::new(file, path, out_template)
}

/// A plan with a pinned clock, so every test renders the same bytes twice.
pub fn plan(tables: Vec<Table>, templates: Vec<TemplateSpec>, out: &Path) -> Plan {
    Plan::new(tables, templates, out).with_clock(
        NaiveDate::from_ymd_opt(2026, 8, 23)
            .unwrap()
            .and_hms_opt(10, 22, 39)
            .unwrap(),
        "tester",
    )
}

/// A token that is not cancelled.
pub fn live() -> CancelToken {
    CancelToken::new()
}

/// Collects every progress event a run reports.
#[derive(Clone, Default)]
pub struct Events(Arc<Mutex<Vec<Progress>>>);

impl Events {
    pub fn new() -> Self {
        Events::default()
    }

    /// The callback to hand to a run.
    pub fn sink(&self) -> impl Fn(Progress) + '_ {
        move |event| self.0.lock().unwrap().push(event)
    }

    pub fn all(&self) -> Vec<Progress> {
        self.0.lock().unwrap().clone()
    }

    /// One label per event, which is what the order assertions read.
    pub fn labels(&self) -> Vec<String> {
        self.all()
            .iter()
            .map(|event| match event {
                Progress::Started { total } => format!("started {total}"),
                Progress::Parsed { templates } => format!("parsed {templates}"),
                Progress::File {
                    index,
                    total,
                    table,
                    status,
                    ..
                } => format!("file {index}/{total} {table} {status:?}"),
                Progress::Finished(outcome) => {
                    format!("finished {}", outcome.written.len())
                }
            })
            .collect()
    }
}

/// Every file below `dir`, relative to it and sorted, `/`-separated.
pub fn tree(dir: &Path) -> Vec<String> {
    let mut found = Vec::new();
    walk(dir, dir, &mut found);
    found.sort();
    found
}

fn walk(root: &Path, dir: &Path, into: &mut Vec<String>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            walk(root, &path, into);
        } else {
            into.push(
                path.strip_prefix(root)
                    .unwrap()
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/"),
            );
        }
    }
}

/// The text of a file below `dir`.
pub fn read(dir: &Path, name: &str) -> String {
    std::fs::read_to_string(dir.join(name)).unwrap_or_else(|e| panic!("{name}: {e}"))
}

/// A path below `dir`.
pub fn at(dir: &Path, name: &str) -> PathBuf {
    dir.join(name)
}
