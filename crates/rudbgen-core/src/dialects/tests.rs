use std::fs;

use super::*;

fn write(dir: &Path, name: &str, text: &str) {
    fs::write(dir.join(name), text).expect("write");
}

/// The shipped profiles, brought into a folder of their own and read.
fn shipped() -> (tempfile::TempDir, Report) {
    let dir = tempfile::tempdir().expect("tempdir");
    sync_shipped(dir.path(), SHIPPED).expect("sync");
    let report = load_from(dir.path());
    (dir, report)
}

#[test]
fn a_directory_that_is_not_there_holds_no_profiles() {
    let dir = tempfile::tempdir().expect("tempdir");
    let report = load_from(&dir.path().join("dialects"));
    assert!(report.entries.is_empty());
    assert!(report.problems.is_empty(), "{:?}", report.problems);
}

/// Every profile rudbkit ships loads here, its `sql` half unread, and brings
/// its driver definition.
#[test]
fn every_shipped_profile_loads_with_its_driver() {
    let (_dir, report) = shipped();
    assert!(!report.entries.is_empty(), "the shipped profiles");
    for entry in &report.entries {
        assert_eq!(entry.failure(), None, "{}", entry.id);
    }
    let drivers: Vec<(String, String, String)> = report
        .drivers()
        .into_iter()
        .map(|driver| (driver.id, driver.class, driver.dialect))
        .collect();
    for (id, class) in [
        ("cockroachdb", "org.postgresql.Driver"),
        ("firebird", "org.firebirdsql.jdbc.FBDriver"),
        ("tidb", "com.mysql.cj.jdbc.Driver"),
    ] {
        assert!(
            drivers.contains(&(id.to_string(), class.to_string(), id.to_string())),
            "{id}: {drivers:?}"
        );
    }
}

/// A session of a profile hands the bridge the built-in its chain ends in and
/// its `bridge` half, and never a script: rudbgen runs none.
#[test]
fn a_session_profile_names_its_built_in_and_carries_no_script() {
    let (_dir, report) = shipped();
    let tidb = report.session_profile("TiDB").expect("by id, in any case");
    assert_eq!(tidb["id"], "tidb");
    assert_eq!(tidb["base"], "mysql");
    let firebird = report.session_profile("firebird").expect("loaded");
    assert_eq!(firebird["base"], "generic");
    assert_eq!(firebird["bridge"]["detect"][0], "firebird");
    assert!(firebird.get("script").is_none(), "{firebird}");
    assert_eq!(
        report
            .entry_for("firebird")
            .and_then(|e| e.script.as_deref()),
        Some("firebird.groovy"),
        "named, and left alone"
    );
    assert!(
        report
            .detect_profiles()
            .iter()
            .any(|profile| profile["id"] == "firebird"),
        "Firebird is found by its product name"
    );
}

const ON_TIDB: &str = "\
api: 1
name: TiDB on TiDB
base: tidb
aliases: [deeper]
bridge:
  comments:
    hides_comments: false
";

/// A profile on a profile merges its `bridge` half over its base's, and is
/// found by its alias.
#[test]
fn a_profile_on_a_profile_merges_down_its_chain() {
    let dir = tempfile::tempdir().expect("tempdir");
    sync_shipped(dir.path(), SHIPPED).expect("sync");
    write(dir.path(), "deep.yml", ON_TIDB);
    write(
        dir.path(),
        "tidb.yml",
        "api: 1\nname: TiDB\nbase: mysql\nbridge:\n  catalog_is_schema: true\n",
    );
    let report = load_from(dir.path());
    let deep = report.session_profile("Deeper").expect("by alias");
    assert_eq!(deep["base"], "mysql", "where the chain ends");
    assert_eq!(deep["bridge"]["catalog_is_schema"], true, "the base's key");
    assert_eq!(
        deep["bridge"]["comments"]["hides_comments"], false,
        "its own"
    );
}

/// A profile that does not load says why, and brings nothing.
#[test]
fn a_profile_that_fails_says_why_and_brings_nothing() {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "typo.yml",
        "api: 1\nname: T\nbase: mysql\nbridge:\n  detekt: [x]\n",
    );
    write(dir.path(), "nowhere.yml", "api: 1\nname: N\nbase: nosuch\n");
    write(dir.path(), "loop_a.yml", "api: 1\nname: A\nbase: loop_b\n");
    write(dir.path(), "loop_b.yml", "api: 1\nname: B\nbase: loop_a\n");
    write(dir.path(), "on_typo.yml", "api: 1\nname: O\nbase: typo\n");
    write(dir.path(), "mysql.yml", "api: 1\nname: M\nbase: generic\n");
    write(
        dir.path(),
        "driven.yml",
        "api: 1\nname: D\nbase: nosuch\ndriver:\n  class: c\n  url_template: u\n",
    );
    let report = load_from(dir.path());
    let failure = |id: &str| {
        report
            .entry_for(id)
            .and_then(Entry::failure)
            .map(str::to_string)
    };
    assert!(
        failure("typo").is_some_and(|m| m.contains("detekt")),
        "{:?}",
        failure("typo")
    );
    assert!(failure("nowhere").is_some_and(|m| m.contains("nosuch")));
    assert!(failure("loop_a").is_some_and(|m| m.contains("its own base")));
    assert_eq!(failure("on_typo").as_deref(), Some("`typo` did not load"));
    assert!(failure("mysql").is_some_and(|m| m.contains("built-in")));
    assert!(
        report.drivers().is_empty(),
        "a failed profile brings no driver"
    );
    assert!(report.session_profile("driven").is_none());
}

/// A newer shipped version waiting beside a changed profile is noticed.
#[test]
fn a_pending_update_is_noticed() {
    let dir = tempfile::tempdir().expect("tempdir");
    sync_shipped(dir.path(), SHIPPED).expect("sync");
    write(
        dir.path(),
        "tidb.yml.new",
        "api: 1\nname: TiDB\nbase: mysql\n",
    );
    let report = load_from(dir.path());
    let tidb = report.entry_for("tidb").expect("listed");
    assert_eq!(tidb.pending, [dir.path().join("tidb.yml.new")]);
    assert_eq!(tidb.failure(), None, "the profile itself still loads");
}
