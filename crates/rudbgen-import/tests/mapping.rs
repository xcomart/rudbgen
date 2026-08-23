//! A whole jdbgen configuration, decrypted and mapped.
//!
//! `tests/vectors/config.json` is a synthetic configuration written by the same
//! Java that wrote the decryption vectors, so its `connectionUrl`, `userName`
//! and `userPassword` are ciphertext jdbgen produced rather than something this
//! crate encrypted for itself. It holds one of everything the mapping has to
//! decide about: a stock driver whose queries and JAR the user edited, a stock
//! driver whose class rudbgen does not know, a driver of the user's own, a
//! connection naming a driver that is not there, a keep-alive interval that is
//! not a number, paths that resolve in each of the two directories and one that
//! resolves in neither, and both encryption formats side by side.

use std::fs;
use std::path::{Path, PathBuf};

use rudbgen_import::{Error, MapOptions, Mapped, Note, PathKind, decrypt, map, preview, read};
use tempfile::TempDir;

/// The master password `tests/vectors/config.json` was written under.
const MASTER: &str = "correct horse battery staple";

/// jdbgen's two directories, laid out the way an import finds them.
struct Fixture {
    data: TempDir,
    install: TempDir,
}

impl Fixture {
    /// Write the checked-in configuration into a data directory, and put the
    /// files its paths point at where each of them belongs.
    fn new() -> Self {
        let data = TempDir::new().expect("a temporary data directory");
        let install = TempDir::new().expect("a temporary installation directory");
        fs::write(
            data.path().join("config.json"),
            include_str!("vectors/config.json"),
        )
        .unwrap();

        // In the data directory: the two JARs, one template and the output
        // directory.
        touch(&data.path().join("drivers/h2-2.4.240.jar"));
        touch(&data.path().join("drivers/warehouse-1.0.jar"));
        touch(&data.path().join("templates/java_model.java"));
        fs::create_dir_all(data.path().join("out")).unwrap();

        // In the installation directory only: the second template, which is
        // what makes the fallback observable.
        touch(&install.path().join("templates/mybatis_mapper.xml"));

        // `templates/never-copied.tpl` and `never-copied/out` are in neither,
        // on purpose.
        Self { data, install }
    }

    fn options(&self) -> MapOptions {
        MapOptions::new(self.data.path()).with_install_dir(self.install.path())
    }

    fn mapped(&self) -> Mapped {
        let config = read(&self.data.path().join("config.json")).unwrap();
        let opened = decrypt(&config, MASTER).unwrap();
        map(&opened, &self.options())
    }
}

fn touch(path: &Path) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, b"").unwrap();
}

fn driver<'a>(mapped: &'a Mapped, id: &str) -> &'a rudbgen_core::DriverDef {
    mapped
        .drivers
        .iter()
        .find(|def| def.id == id)
        .unwrap_or_else(|| panic!("no driver '{id}' among {:?}", ids(mapped)))
}

fn ids(mapped: &Mapped) -> Vec<&str> {
    mapped.drivers.iter().map(|def| def.id.as_str()).collect()
}

fn connection<'a>(
    mapped: &'a Mapped,
    name: &str,
) -> &'a (rudbgen_core::ConnectionProfile, rudbgen_import::Secret) {
    mapped
        .connections
        .iter()
        .find(|(profile, _)| profile.name == name)
        .unwrap_or_else(|| panic!("no connection '{name}'"))
}

#[test]
fn the_shipped_default_configuration_parses() {
    // jdbgen's own `defaultConfig.json`, copied verbatim from
    // `jdbgen/src/main/resources/`: ten stock drivers, no connections. It is
    // the file every first run of jdbgen starts from, so an import that cannot
    // read it can read nothing.
    let raw = include_str!("vectors/defaultConfig.json");
    let config: rudbgen_import::JdbgenConfig = serde_json::from_str(raw).unwrap();
    assert_eq!(config.drivers.len(), 10);
    assert!(config.connections.is_empty());
    assert!(config.drivers.iter().all(|driver| driver.stock_item));
    assert!(
        config
            .drivers
            .iter()
            .any(|driver| driver.use_tables && driver.tables_sql.is_some()),
        "H2 ships a table-list override"
    );
}

#[test]
fn a_configuration_written_in_both_formats_opens_with_one_password() {
    let fixture = Fixture::new();
    let config = read(&fixture.data.path().join("config.json")).unwrap();
    let opened = decrypt(&config, MASTER).unwrap();

    assert!(opened.legacy, "the fixture holds superseded values as well");
    let urls: Vec<&str> = opened
        .config
        .connections
        .iter()
        .map(|conn| conn.connection_url.as_str())
        .collect();
    assert_eq!(
        urls,
        [
            "jdbc:h2:./sample_h2.db",
            "jdbc:postgresql://db.example.com:5432/app",
            "jdbc:oracle:thin:@//ora.example.com:1521/ORCL",
            "jdbc:example://wh.example.com:7777/warehouse",
        ]
    );
}

#[test]
fn the_wrong_master_password_opens_nothing_at_all() {
    let fixture = Fixture::new();
    let config = read(&fixture.data.path().join("config.json")).unwrap();
    assert!(matches!(
        decrypt(&config, "not the master password"),
        Err(Error::WrongPassword)
    ));
}

#[test]
fn a_stock_driver_keeps_rudbgens_identity_and_the_users_edits() {
    let mapped = Fixture::new().mapped();
    let h2 = driver(&mapped, "h2-embedded");

    // rudbgen's, because they are facts about the product.
    assert_eq!(h2.name, "H2 Embedded");
    assert_eq!(h2.url_template, "jdbc:h2:{file}");
    assert_eq!(h2.dialect, "h2");
    assert!(h2.no_auth);

    // jdbgen's, because they are what the user edited.
    assert_eq!(h2.jars.len(), 1);
    assert!(h2.jars[0].ends_with("drivers/h2-2.4.240.jar"));
    assert!(h2.jars[0].is_absolute());
    assert_eq!(h2.props.get("MODE").map(String::as_str), Some("Oracle"));
    assert!(h2.custom_queries.tables.enabled);
    assert_eq!(
        h2.custom_queries.tables.sql,
        "select 1 from information_schema.tables"
    );
}

#[test]
fn a_stock_driver_that_says_nothing_about_its_properties_keeps_the_built_ins() {
    let mapped = Fixture::new().mapped();
    let oracle = driver(&mapped, "oracle-thin");
    assert_eq!(
        oracle.props.get("remarksReporting").map(String::as_str),
        Some("true"),
        "the built-in property survives an entry that never mentioned it"
    );
    assert!(oracle.jars.is_empty(), "the fixture names no Oracle JAR");
}

#[test]
fn a_driver_the_user_wrote_becomes_one_of_its_own() {
    let mapped = Fixture::new().mapped();
    let ours = mapped
        .drivers
        .iter()
        .find(|def| def.name == "Our Warehouse")
        .expect("the user's own driver");

    assert!(ours.id.starts_with("our-warehouse-"), "{}", ours.id);
    assert_eq!(ours.class, "com.example.warehouse.Driver");
    assert_eq!(
        ours.url_template,
        "jdbc:example://{host}:{port}/{warehouse}"
    );
    assert_eq!(ours.default_port, Some(7777));
    assert_eq!(ours.dialect, "generic");
    assert_eq!(
        ours.maven.as_deref(),
        Some("com.example:warehouse-jdbc:1.0")
    );
    assert!(ours.custom_queries.columns.enabled);
    assert_eq!(
        ours.icon, None,
        "a Font Awesome glyph has no equivalent here"
    );
    assert!(ours.jars[0].ends_with("drivers/warehouse-1.0.jar"));
}

#[test]
fn a_stock_driver_rudbgen_ships_nothing_for_is_imported_and_reported() {
    let mapped = Fixture::new().mapped();
    let gone = mapped
        .drivers
        .iter()
        .find(|def| def.name == "Gone Product")
        .expect("the unknown stock driver");
    assert!(gone.id.starts_with("gone-product-"), "{}", gone.id);
    assert_eq!(gone.url_template, "jdbc:gone:{host}");
    assert!(mapped.notes.contains(&Note::StockDriverUnknown {
        driver: "Gone Product".into(),
        class: "com.gone.Driver".into(),
    }));
}

#[test]
fn every_stock_driver_that_was_recognised_says_which_built_in_it_became() {
    let mapped = Fixture::new().mapped();
    for (driver, builtin) in [
        ("H2 Embedded", "h2-embedded"),
        ("PostgreSQL", "postgresql"),
        ("Oracle", "oracle-thin"),
    ] {
        assert!(
            mapped.notes.contains(&Note::StockDriverMatched {
                driver: driver.into(),
                builtin: builtin.into(),
            }),
            "no note for {driver}"
        );
    }
}

#[test]
fn a_connection_carries_its_url_and_user_and_leaves_its_password_behind() {
    let mapped = Fixture::new().mapped();
    let (profile, secret) = connection(&mapped, "Warehouse");

    assert_eq!(profile.driver_id, "postgresql");
    assert_eq!(profile.url, "jdbc:postgresql://db.example.com:5432/app");
    assert_eq!(profile.username, "alice");
    assert_eq!(secret.password, "hunter2");
    assert_eq!(
        profile.props.get("ApplicationName").map(String::as_str),
        Some("jdbgen")
    );
    assert_eq!(profile.generation.author, "alice");

    // The password is nowhere in the profile, and the profile is what gets
    // serialised into `connections.json`.
    let json = serde_json::to_string(profile).unwrap();
    assert!(!json.contains("hunter2"), "{json}");
    assert!(!format!("{secret:?}").contains("hunter2"));
}

#[test]
fn a_keep_alive_interval_is_read_as_seconds_or_switched_off() {
    let mapped = Fixture::new().mapped();

    let (warehouse, _) = connection(&mapped, "Warehouse");
    let keep_alive = warehouse.keep_alive.as_ref().expect("a numeric interval");
    assert_eq!(keep_alive.interval_s, 120);
    assert_eq!(keep_alive.query, "select 1");

    let (oracle, _) = connection(&mapped, "Legacy Oracle");
    assert!(oracle.keep_alive.is_none(), "'30 sec' is not a number");
    assert!(mapped.notes.contains(&Note::KeepAliveNotANumber {
        connection: "Legacy Oracle".into(),
        value: "30 sec".into(),
    }));
}

#[test]
fn a_connection_naming_a_driver_that_is_not_there_still_imports() {
    let mapped = Fixture::new().mapped();
    let (nowhere, _) = connection(&mapped, "Nowhere");
    assert_eq!(nowhere.driver_id, "");
    assert!(mapped.notes.contains(&Note::UnknownDriver {
        connection: "Nowhere".into(),
        driver_type: "Retired Product".into(),
    }));
}

#[test]
fn a_relative_path_is_resolved_against_the_data_directory_then_the_installation() {
    let fixture = Fixture::new();
    let mapped = fixture.mapped();
    let (h2, _) = connection(&mapped, "Sample H2");

    let files: Vec<&PathBuf> = h2.generation.templates.iter().map(|t| &t.file).collect();
    assert_eq!(
        files[0],
        &fixture.data.path().join("templates/java_model.java")
    );
    assert_eq!(
        files[1],
        &fixture.install.path().join("templates/mybatis_mapper.xml"),
        "the second template is only in the installation directory"
    );
    assert_eq!(
        h2.generation.output_dir.as_ref().unwrap(),
        &fixture.data.path().join("out")
    );
}

#[test]
fn a_path_in_neither_directory_is_carried_across_as_written_and_reported() {
    let mapped = Fixture::new().mapped();
    let (nowhere, _) = connection(&mapped, "Nowhere");

    assert_eq!(
        nowhere.generation.templates[0].file,
        PathBuf::from("templates/never-copied.tpl")
    );
    assert_eq!(
        nowhere.generation.output_dir,
        Some(PathBuf::from("never-copied/out"))
    );
    assert!(mapped.notes.contains(&Note::UnresolvedPath {
        kind: PathKind::TemplateFile,
        owner: "Nowhere".into(),
        path: "templates/never-copied.tpl".into(),
    }));
    assert!(mapped.notes.contains(&Note::UnresolvedPath {
        kind: PathKind::OutputDir,
        owner: "Nowhere".into(),
        path: "never-copied/out".into(),
    }));
}

#[test]
fn an_absolute_path_is_left_exactly_as_it_is() {
    // Built here rather than checked into the fixture, because what counts as
    // an absolute path is a platform question and the suite runs on three.
    let elsewhere = TempDir::new().unwrap();
    let raw = format!(
        r#"{{"connections":[{{"name":"n","outputDir":{}}}]}}"#,
        serde_json::to_string(&elsewhere.path().join("generated")).unwrap()
    );
    let config: rudbgen_import::JdbgenConfig = serde_json::from_str(&raw).unwrap();
    let opened = decrypt(&config, MASTER).unwrap();
    let mapped = map(&opened, &MapOptions::new(elsewhere.path()));

    assert_eq!(
        mapped.connections[0].0.generation.output_dir,
        Some(elsewhere.path().join("generated"))
    );
    assert!(
        !mapped
            .notes
            .iter()
            .any(|note| matches!(note, Note::UnresolvedPath { .. })),
        "an absolute path resolves without being looked for"
    );
}

#[test]
fn the_generation_profile_keeps_the_ticks_and_the_variable_order() {
    let mapped = Fixture::new().mapped();
    let (h2, _) = connection(&mapped, "Sample H2");

    assert_eq!(h2.generation.templates.len(), 2);
    assert_eq!(h2.generation.templates[0].name, "Java Model");
    assert_eq!(
        h2.generation.templates[0].out_template,
        "${table:camel}Model.java"
    );
    assert!(h2.generation.templates[0].selected);
    assert!(!h2.generation.templates[1].selected);
    assert_eq!(
        h2.generation.custom_vars,
        vec![
            ("package".to_string(), "com.abc.sample".to_string()),
            ("vendor".to_string(), "acme".to_string()),
        ]
    );
}

#[test]
fn presets_become_template_sets() {
    let mapped = Fixture::new().mapped();
    assert_eq!(mapped.sets.len(), 1);
    assert_eq!(mapped.sets[0].name, "Java + MyBatis");
    assert_eq!(mapped.sets[0].templates.len(), 2);
    assert!(mapped.sets[0].templates.iter().all(|t| t.selected));
    assert_ne!(mapped.sets[0].id, uuid::Uuid::nil());
}

#[test]
fn abbreviations_come_across_with_their_two_flags() {
    let mapped = Fixture::new().mapped();
    let rules = &mapped.rules;
    assert_eq!(rules.len(), 4);

    assert!(rules[0].enabled && !rules[0].whole_name);
    assert_eq!(rules[0].abbreviation, "emp");
    assert_eq!(rules[0].replacement, "Employee");

    assert!(rules[1].enabled && rules[1].whole_name);
    assert_eq!(rules[1].abbreviation, "T_ORG");

    assert!(!rules[2].enabled, "check:false is a rule that is off");
    assert!(
        !rules[3].enabled && !rules[3].whole_name,
        "an absent flag is off, and an absent totalName is a word rule"
    );
    assert!(mapped.apply_abbr);
}

#[test]
fn the_case_rule_change_is_reported_whatever_the_configuration_holds() {
    let mapped = Fixture::new().mapped();
    assert!(mapped.notes.contains(&Note::AbbreviationCaseRule));

    let empty: rudbgen_import::JdbgenConfig = serde_json::from_str("{}").unwrap();
    let opened = decrypt(&empty, MASTER).unwrap();
    let mapped = map(&opened, &MapOptions::new("."));
    assert_eq!(
        mapped.notes,
        vec![Note::AbbreviationCaseRule],
        "a configuration with nothing in it still gets the one note"
    );
}

#[test]
fn a_superseded_value_anywhere_in_the_file_is_reported_once() {
    let mapped = Fixture::new().mapped();
    let legacy = mapped
        .notes
        .iter()
        .filter(|note| **note == Note::LegacyEncryption)
        .count();
    assert_eq!(legacy, 1);
}

#[test]
fn the_theme_and_the_language_are_a_hint_and_not_a_decision() {
    let mapped = Fixture::new().mapped();
    assert_eq!(mapped.settings_hint.language.as_deref(), Some("ko"));
    assert!(mapped.settings_hint.dark_ui);
}

#[test]
fn the_checklist_shows_every_row_and_no_secret() {
    let fixture = Fixture::new();
    let config = read(&fixture.data.path().join("config.json")).unwrap();
    let opened = decrypt(&config, MASTER).unwrap();
    let found = preview(&opened, &fixture.options());

    assert_eq!(found.connections.len(), 4);
    assert_eq!(found.drivers.len(), 5);
    assert_eq!(found.sets.len(), 1);
    assert_eq!(found.rules.len(), 4);
    assert_eq!(found.sets[0].templates, 2);

    let h2 = &found.drivers[0];
    assert_eq!(h2.name, "H2 Embedded");
    assert!(h2.stock);
    assert_eq!(h2.matched_builtin.as_deref(), Some("h2-embedded"));

    let ours = &found.drivers[3];
    assert!(!ours.stock);
    assert_eq!(ours.matched_builtin, None);

    let gone = &found.drivers[4];
    assert!(gone.stock);
    assert_eq!(gone.matched_builtin, None, "stock, but nothing to match");

    assert_eq!(found.connections[1].driver, "PostgreSQL");
    assert_eq!(
        found.connections[1].url,
        "jdbc:postgresql://db.example.com:5432/app"
    );
    let rendered = format!("{found:?}");
    assert!(!rendered.contains("hunter2"), "{rendered}");
}

#[test]
fn a_configuration_missing_everything_optional_maps_to_empty_stores() {
    let config: rudbgen_import::JdbgenConfig =
        serde_json::from_str(r#"{"drivers":null,"connections":[{}],"presets":null}"#).unwrap();
    let opened = decrypt(&config, MASTER).unwrap();
    let mapped = map(&opened, &MapOptions::new("."));

    assert!(mapped.drivers.is_empty());
    assert!(mapped.sets.is_empty());
    assert!(mapped.rules.is_empty());
    assert!(!mapped.apply_abbr);
    assert_eq!(
        mapped.settings_hint,
        rudbgen_import::SettingsHint::default()
    );

    let (profile, secret) = &mapped.connections[0];
    assert_eq!(profile.name, "");
    assert_eq!(profile.url, "");
    assert_eq!(secret.password, "");
    assert!(profile.keep_alive.is_none());
    assert!(profile.generation.templates.is_empty());
    assert_eq!(profile.generation.output_dir, None);
}
