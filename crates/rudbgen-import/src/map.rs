//! From a decrypted jdbgen configuration to rudbgen's stores.
//!
//! Nothing here writes anything. [`map`] is a pure function from a
//! [`Decrypted`] configuration and two directory names to the values the app
//! would `upsert` into `connections.json`, `drivers.json`,
//! `template-sets.json` and `abbreviations.json` — plus the passwords, handed
//! back separately because their destination is the OS keychain and not a file
//! (architecture document, D5 and §5).
//!
//! The rules it applies, all of them consequences of the two products storing
//! the same facts differently:
//!
//! * **Stock drivers keep rudbgen's identity.** A driver jdbgen marked
//!   `stockItem` and whose `driverClass` names a product
//!   [`DriverDef::builtins`] knows is imported *as* that built-in — same id,
//!   same URL template with the `{host}`/`{port}` placeholders the connection
//!   dialog fills, same dialect — while the JAR path, the four SQL overrides
//!   and the properties come across from jdbgen, because those are the parts
//!   the user edited. Everything else becomes a driver of its own, with an id
//!   made from its name and a fresh UUID so that two configurations imported
//!   into one installation cannot collide.
//! * **URL templates are rewritten.** jdbgen writes `<databaseHost>`,
//!   `<database>` and `<database file>` where rudbgen writes `{host}`,
//!   `{database}` and `{file}`; any other `<x>` becomes `{x}`. A literal port
//!   after the host — jdbgen hard-codes one, and writes H2's as the optional
//!   `[:9092]` — becomes `{port}` and the driver's default port.
//! * **A connection's `driverType` is a driver *name***, and is resolved
//!   against the drivers this same import produced.
//! * **The URL and the user name stay in the profile**; only the password is
//!   separated out. They were encrypted in jdbgen because a master password
//!   existed, not because they are secrets (§5).
//! * **Relative paths are made absolute** against jdbgen's data directory, then
//!   its installation directory — whichever holds the file. A path found in
//!   neither is carried across unchanged, with a note, because inventing an
//!   absolute path for a file that has not been copied to this machine yet
//!   helps nobody.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use rudbgen_core::{
    AbbreviationRule, ConnectionProfile, CustomQueries, CustomQuery, DriverDef, GenerationProfile,
    KeepAlive, TemplateRef, TemplateSet,
};
use uuid::Uuid;

use crate::config::{JdbConnection, JdbDriver, JdbTemplate, JdbgenConfig};
use crate::notes::{Note, PathKind};

/// Where jdbgen kept the files a relative path in its configuration points at.
///
/// Both are directories rather than a search path on purpose: they are
/// jdbgen's own two, in jdbgen's own order (`AppDirs.resolve`), and adding a
/// third would make an import depend on where the wizard was started from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapOptions {
    /// jdbgen's user data directory — the one holding `config.json`.
    pub jdbgen_data_dir: PathBuf,
    /// The directory jdbgen was installed in, when it can be found.
    ///
    /// jdbgen resolves a relative path against its own installation as a
    /// fallback, which is where the shipped templates live for a copy that was
    /// unzipped rather than installed. `None` when the wizard has no candidate,
    /// and then a path that is not below the data directory is simply not
    /// resolved.
    pub jdbgen_install_dir: Option<PathBuf>,
}

impl MapOptions {
    /// Resolve relative paths against `data_dir` and nothing else.
    pub fn new(data_dir: impl Into<PathBuf>) -> Self {
        Self {
            jdbgen_data_dir: data_dir.into(),
            jdbgen_install_dir: None,
        }
    }

    /// Add the installation directory to fall back on.
    #[must_use]
    pub fn with_install_dir(mut self, install_dir: impl Into<PathBuf>) -> Self {
        self.jdbgen_install_dir = Some(install_dir.into());
        self
    }
}

/// A jdbgen configuration whose three encrypted fields have been opened.
///
/// Holds plain text — a database password among it — so it lives for as long as
/// the wizard is on screen and no longer. Nothing in this crate serialises it,
/// and the [`Debug`] impl of [`Secret`] is what keeps the password out of a log
/// line written by whoever is debugging the wizard.
#[derive(Debug, Clone, PartialEq)]
pub struct Decrypted {
    /// The configuration, with `connectionUrl`, `userName` and `userPassword`
    /// in plain text.
    pub config: JdbgenConfig,
    /// Whether any value was written in the superseded encryption format.
    pub legacy: bool,
}

/// A password on its way to the OS keychain.
///
/// A type of its own rather than a `String` field on the profile, because
/// rudbgen's [`ConnectionProfile`] deliberately has no password field: the
/// secret goes to
/// [`SecretSlot::Connection`](rudbgen_core::SecretSlot::Connection) under the
/// profile's id, and a value that never reaches a struct that is serialised can
/// never be written to a file by accident.
#[derive(Clone, PartialEq, Eq)]
pub struct Secret {
    /// The database password, in plain text.
    pub password: String,
}

impl std::fmt::Debug for Secret {
    /// Renders the secret without it, and without its length.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Secret(<redacted>)")
    }
}

/// The two application settings jdbgen holds that rudbgen also has.
///
/// A *hint*, not a decision: whether an import changes the theme or the
/// language of the running application is the app's call, and it is the only
/// layer that knows whether the user has already chosen either.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SettingsHint {
    /// Language tag jdbgen was last run in; `None` for the system language.
    pub language: Option<String>,
    /// Whether jdbgen was last shown in its dark theme.
    pub dark_ui: bool,
}

/// Everything an import produces, ready to be written by the app.
#[derive(Debug, Clone, PartialEq)]
pub struct Mapped {
    /// Profiles and their passwords, in the order jdbgen listed them.
    pub connections: Vec<(ConnectionProfile, Secret)>,
    /// Driver definitions, both the built-ins that were matched and the ones
    /// this import invented.
    pub drivers: Vec<DriverDef>,
    /// jdbgen's presets.
    pub sets: Vec<TemplateSet>,
    /// jdbgen's abbreviation rules.
    pub rules: Vec<AbbreviationRule>,
    /// Whether the rules were being applied.
    pub apply_abbr: bool,
    /// Theme and language, for the app to accept or ignore.
    pub settings_hint: SettingsHint,
    /// What the wizard has to say out loud.
    pub notes: Vec<Note>,
}

/// Turn a decrypted configuration into rudbgen's stores.
///
/// Never fails: everything that cannot be carried across becomes a
/// [`Note`] beside the value that was carried across instead. A configuration
/// with nothing in it maps to empty stores and the one note that is always
/// there — D10's change to the abbreviation rules.
pub fn map(dec: &Decrypted, opts: &MapOptions) -> Mapped {
    let mut notes = Vec::new();

    // The one note that does not depend on the file: the behavioural break is
    // announced whether or not this configuration holds a rule today.
    notes.push(Note::AbbreviationCaseRule);
    if dec.legacy {
        notes.push(Note::LegacyEncryption);
    }

    let (drivers, by_name) = map_drivers(&dec.config.drivers, opts, &mut notes);
    let connections = dec
        .config
        .connections
        .iter()
        .map(|conn| map_connection(conn, &by_name, opts, &mut notes))
        .collect();
    let sets = dec
        .config
        .presets
        .iter()
        .map(|preset| {
            TemplateSet::new(
                preset.name.clone(),
                map_templates(&preset.templates, &preset.name, opts, &mut notes),
            )
        })
        .collect();
    let rules = dec
        .config
        .abbrs
        .iter()
        .map(|abbr| AbbreviationRule {
            enabled: abbr.check.unwrap_or(false),
            whole_name: abbr.total_name.unwrap_or(false),
            abbreviation: abbr.abbr.clone().unwrap_or_default(),
            replacement: abbr.replace_to.clone().unwrap_or_default(),
        })
        .collect();

    // The wizard shows the notes translated; this is the same list for whoever
    // is reading a log because the wizard did something they did not expect.
    for note in &notes {
        log::debug!("jdbgen import: {note}");
    }

    Mapped {
        connections,
        drivers,
        sets,
        rules,
        apply_abbr: dec.config.apply_abbr,
        settings_hint: SettingsHint {
            language: dec
                .config
                .language
                .as_deref()
                .map(str::trim)
                .filter(|tag| !tag.is_empty())
                .map(str::to_string),
            dark_ui: dec.config.is_dark_ui,
        },
        notes,
    }
}

/// Map every driver, and build the name → id table the connections need.
fn map_drivers(
    drivers: &[JdbDriver],
    opts: &MapOptions,
    notes: &mut Vec<Note>,
) -> (Vec<DriverDef>, BTreeMap<String, String>) {
    let builtins = DriverDef::builtins();
    let mut mapped: Vec<DriverDef> = Vec::with_capacity(drivers.len());
    let mut by_name: BTreeMap<String, String> = BTreeMap::new();

    for driver in drivers {
        let def = match driver
            .stock_item
            .then(|| match_builtin(&builtins, driver))
            .flatten()
        {
            Some(builtin) => {
                notes.push(Note::StockDriverMatched {
                    driver: driver.name.clone(),
                    builtin: builtin.id.clone(),
                });
                from_builtin(builtin, driver, opts, notes)
            }
            None => {
                if driver.stock_item {
                    notes.push(Note::StockDriverUnknown {
                        driver: driver.name.clone(),
                        class: driver.driver_class.clone(),
                    });
                }
                fresh_driver(driver, opts, notes)
            }
        };

        // Two entries naming one product — a stock driver duplicated by hand,
        // say — must not become two definitions sharing an id. The first wins,
        // and the second name points at it.
        if !mapped.iter().any(|existing| existing.id == def.id) {
            by_name.entry(driver.name.clone()).or_insert(def.id.clone());
            mapped.push(def);
        } else {
            by_name.entry(driver.name.clone()).or_insert(def.id);
        }
    }

    (mapped, by_name)
}

/// The id of the built-in definition a driver maps onto, for the checklist.
///
/// The same question [`map_drivers`] asks, asked again by
/// [`crate::preview::from_mapped`] so that the row a user sees and the
/// definition the import writes can never disagree.
pub(crate) fn matched_builtin_id(driver: &JdbDriver) -> Option<String> {
    driver
        .stock_item
        .then(|| match_builtin(&DriverDef::builtins(), driver).map(|def| def.id.clone()))
        .flatten()
}

/// The built-in definition of the product `driver` names, if there is one.
///
/// Matched on the driver *class*, which is the one field a stock entry cannot
/// have changed and still work. Two built-ins share a class — H2's embedded and
/// server forms are one JAR and two URLs — so the name breaks the tie, and a
/// name that matches neither takes the first, which is the embedded form and
/// the one jdbgen lists first as well.
fn match_builtin<'a>(builtins: &'a [DriverDef], driver: &JdbDriver) -> Option<&'a DriverDef> {
    let class = driver.driver_class.trim();
    if class.is_empty() {
        return None;
    }
    let candidates: Vec<&DriverDef> = builtins
        .iter()
        .filter(|builtin| builtin.class == class)
        .collect();
    match candidates.len() {
        0 => None,
        1 => Some(candidates[0]),
        _ => candidates
            .iter()
            .find(|builtin| builtin.name.eq_ignore_ascii_case(driver.name.trim()))
            .or(candidates.first())
            .copied(),
    }
}

/// A built-in definition wearing the parts of the jdbgen entry the user edited.
fn from_builtin(
    builtin: &DriverDef,
    driver: &JdbDriver,
    opts: &MapOptions,
    notes: &mut Vec<Note>,
) -> DriverDef {
    let mut def = builtin.clone();
    def.jars = jar_paths(driver, opts, notes);
    if let Some(props) = &driver.props {
        // Present but empty is a user who removed them; absent is a
        // hand-written entry that never mentioned them, and then the built-in's
        // own properties are the better answer.
        def.props = props.clone();
    }
    if let Some(maven) = non_empty(driver.maven_artifact.as_deref()) {
        def.maven = Some(maven.to_string());
    }
    def.custom_queries = merge_queries(&def.custom_queries, driver);
    def
}

/// A definition for a driver rudbgen ships nothing for.
fn fresh_driver(driver: &JdbDriver, opts: &MapOptions, notes: &mut Vec<Note>) -> DriverDef {
    let (url_template, default_port) = convert_url_template(&driver.url_template);
    let (icon, dropped) = icon_stem(driver.icon.as_deref());
    if let Some(icon) = dropped {
        notes.push(Note::IconDropped {
            owner: driver.name.clone(),
            icon,
        });
    }
    DriverDef {
        id: fresh_id(&driver.name),
        name: driver.name.clone(),
        icon,
        class: driver.driver_class.trim().to_string(),
        jars: jar_paths(driver, opts, notes),
        maven: non_empty(driver.maven_artifact.as_deref()).map(str::to_string),
        url_template,
        default_port,
        dialect: dialect_of(&driver.driver_class, &driver.url_template).to_string(),
        no_auth: driver.no_auth,
        props: driver.props.clone().unwrap_or_default(),
        custom_queries: merge_queries(&CustomQueries::default(), driver),
    }
}

/// The four SQL overrides, jdbgen's winning wherever jdbgen says anything.
///
/// "Says anything" is the flag *or* the text: a query switched off with its SQL
/// still in the box is a deliberate edit and comes across switched off, while
/// an entry that mentions neither leaves whatever the built-in shipped — which
/// is how a hand-written stock entry keeps H2's table list.
fn merge_queries(base: &CustomQueries, driver: &JdbDriver) -> CustomQueries {
    let mut merged = base.clone();
    let incoming = driver.custom_queries();
    for (kind, (enabled, sql)) in rudbgen_core::CustomQueryKind::ALL.iter().zip(incoming) {
        if enabled || !sql.trim().is_empty() {
            *merged.get_mut(*kind) = CustomQuery {
                enabled,
                sql: sql.to_string(),
            };
        }
    }
    merged
}

/// The driver's JAR, resolved, as the one-element list `DriverDef` holds.
fn jar_paths(driver: &JdbDriver, opts: &MapOptions, notes: &mut Vec<Note>) -> Vec<PathBuf> {
    resolve(
        driver.jdbc_jar.as_deref(),
        PathKind::DriverJar,
        &driver.name,
        opts,
        notes,
    )
    .into_iter()
    .collect()
}

/// One saved connection, and the password that leaves the profile behind.
fn map_connection(
    conn: &JdbConnection,
    by_name: &BTreeMap<String, String>,
    opts: &MapOptions,
    notes: &mut Vec<Note>,
) -> (ConnectionProfile, Secret) {
    let driver_id = lookup_driver(by_name, &conn.driver_type).unwrap_or_else(|| {
        notes.push(Note::UnknownDriver {
            connection: conn.name.clone(),
            driver_type: conn.driver_type.clone(),
        });
        String::new()
    });

    let keep_alive = conn.use_keep_alive.then(|| {
        let raw = conn
            .keep_alive_sec
            .as_deref()
            .unwrap_or("")
            .trim()
            .to_string();
        match raw.parse::<u32>() {
            Ok(interval_s) if interval_s > 0 => Some(KeepAlive {
                interval_s,
                query: conn.keep_alive_query.clone().unwrap_or_default(),
            }),
            _ => {
                notes.push(Note::KeepAliveNotANumber {
                    connection: conn.name.clone(),
                    value: raw,
                });
                None
            }
        }
    });

    let profile = ConnectionProfile {
        name: conn.name.clone(),
        driver_id,
        url: conn.connection_url.clone(),
        username: conn.user_name.clone(),
        props: conn.connection_props.clone(),
        keep_alive: keep_alive.flatten(),
        generation: GenerationProfile {
            templates: map_templates(&conn.templates, &conn.name, opts, notes),
            output_dir: resolve(
                conn.output_dir.as_deref(),
                PathKind::OutputDir,
                &conn.name,
                opts,
                notes,
            ),
            author: conn.author.clone().unwrap_or_default(),
            custom_vars: conn.custom_vars.clone(),
        },
        ..ConnectionProfile::default()
    };

    (
        profile,
        Secret {
            password: conn.user_password.clone(),
        },
    )
}

/// The id of the driver a `driverType` names.
///
/// Exact first, then case-insensitively: jdbgen matches the name verbatim, but
/// a configuration edited by hand is the normal case for an import and `"h2
/// embedded"` should not lose its driver over a capital letter.
fn lookup_driver(by_name: &BTreeMap<String, String>, driver_type: &str) -> Option<String> {
    let wanted = driver_type.trim();
    if wanted.is_empty() {
        return None;
    }
    by_name.get(wanted).cloned().or_else(|| {
        by_name
            .iter()
            .find(|(name, _)| name.trim().eq_ignore_ascii_case(wanted))
            .map(|(_, id)| id.clone())
    })
}

/// jdbgen's templates, with their bodies resolved.
fn map_templates(
    templates: &[JdbTemplate],
    owner: &str,
    opts: &MapOptions,
    notes: &mut Vec<Note>,
) -> Vec<TemplateRef> {
    templates
        .iter()
        .map(|template| TemplateRef {
            name: template.name.clone(),
            file: resolve(
                Some(template.template_file.as_str()),
                PathKind::TemplateFile,
                owner,
                opts,
                notes,
            )
            .unwrap_or_default(),
            out_template: template.out_template.clone(),
            selected: template.selected,
        })
        .collect()
}

/// jdbgen's `AppDirs.resolve`, minus its habit of guessing.
///
/// An absolute path is itself; a relative one is looked for below the data
/// directory and then below the installation directory. jdbgen falls back to
/// the data directory even when the file is in neither, which produces an
/// absolute path to a file that does not exist; this keeps the path as written
/// and says so, because the wizard can ask about it and a fabricated absolute
/// path cannot be told from a real one later.
fn resolve(
    raw: Option<&str>,
    kind: PathKind,
    owner: &str,
    opts: &MapOptions,
    notes: &mut Vec<Note>,
) -> Option<PathBuf> {
    let raw = non_empty(raw)?.trim();
    let path = Path::new(raw);
    if path.is_absolute() {
        return Some(path.to_path_buf());
    }
    let in_data = opts.jdbgen_data_dir.join(path);
    if in_data.exists() {
        return Some(in_data);
    }
    if let Some(install) = &opts.jdbgen_install_dir {
        let in_install = install.join(path);
        if in_install.exists() {
            return Some(in_install);
        }
    }
    notes.push(Note::UnresolvedPath {
        kind,
        owner: owner.to_string(),
        path: raw.to_string(),
    });
    Some(path.to_path_buf())
}

/// jdbgen's `<placeholder>` URL skeleton in rudbgen's `{placeholder}` spelling,
/// with the hard-coded port lifted out of it.
///
/// The three names that differ are translated — `databaseHost` is `host`, and
/// `database file` is `file` — and anything else keeps its own name. The port
/// is the interesting half: jdbgen writes it as a literal (`<databaseHost>:1521`)
/// or, for H2, as the optional `[:9092]`, while rudbgen's connection dialog
/// fills a `{port}` hole and pre-fills it from the driver's default. Both forms
/// become `{port}`, and the number becomes that default.
fn convert_url_template(raw: &str) -> (String, Option<u16>) {
    let mut out = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>').map(|index| open + index) else {
            break;
        };
        out.push_str(&rest[..open]);
        out.push('{');
        out.push_str(match rest[open + 1..close].trim() {
            "databaseHost" => "host",
            "database file" => "file",
            other => other,
        });
        out.push('}');
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    lift_port(out)
}

/// Replace the literal port after `{host}` with `{port}` and return it.
fn lift_port(template: String) -> (String, Option<u16>) {
    const HOST: &str = "{host}";
    let Some(after) = template.find(HOST).map(|index| index + HOST.len()) else {
        return (template, None);
    };
    let tail = &template[after..];
    // `[:9092]` — jdbgen's way of writing "and this one is optional" — and the
    // plain `:1521` every other stock entry uses.
    let (digits, consumed) = match tail.strip_prefix("[:") {
        Some(bracketed) => {
            let digits: String = bracketed.chars().take_while(char::is_ascii_digit).collect();
            match bracketed[digits.len()..].starts_with(']') {
                true => (digits.clone(), digits.len() + 3),
                false => (String::new(), 0),
            }
        }
        None => match tail.strip_prefix(':') {
            Some(plain) => {
                let digits: String = plain.chars().take_while(char::is_ascii_digit).collect();
                let length = digits.len();
                (digits, length + 1)
            }
            None => (String::new(), 0),
        },
    };
    let Ok(port) = digits.parse::<u16>() else {
        return (template, None);
    };
    let mut out = String::with_capacity(template.len());
    out.push_str(&template[..after]);
    out.push_str(":{port}");
    out.push_str(&template[after + consumed..]);
    (out, Some(port))
}

/// The asset stem behind a jdbgen icon locator, when there is one.
///
/// `stock:h2.png` is the only form with an equivalent here — `assets/drivers/`
/// holds jdbgen's eleven icons under those very stems. A file path, a Font
/// Awesome glyph, a colour swatch or a URL comes across as no icon and is
/// returned as the second half of the pair so the caller can note it.
fn icon_stem(locator: Option<&str>) -> (Option<String>, Option<String>) {
    let Some(locator) = non_empty(locator).map(str::trim) else {
        return (None, None);
    };
    match locator.strip_prefix("stock:") {
        Some(file) => {
            let stem = file.strip_suffix(".png").unwrap_or(file);
            match stem.is_empty() {
                true => (None, Some(locator.to_string())),
                false => (Some(stem.to_string()), None),
            }
        }
        None => (None, Some(locator.to_string())),
    }
}

/// The dialect id for a product, read off its class and then its URL scheme.
///
/// Only the seven the SQL layer knows are named; everything else is `generic`,
/// which degrades to standard behaviour rather than failing. The class is asked
/// first because a URL template can be edited into anything, while a class that
/// is wrong does not load at all.
fn dialect_of(class: &str, url_template: &str) -> &'static str {
    const BY_CLASS: &[(&str, &str)] = &[
        ("oracle.jdbc", "oracle"),
        ("org.postgresql", "postgres"),
        ("com.mysql", "mysql"),
        ("org.mariadb", "mariadb"),
        ("org.sqlite", "sqlite"),
        ("org.h2", "h2"),
        ("com.microsoft.sqlserver", "mssql"),
    ];
    const BY_SCHEME: &[(&str, &str)] = &[
        ("oracle", "oracle"),
        ("postgresql", "postgres"),
        ("mysql", "mysql"),
        ("mariadb", "mariadb"),
        ("sqlite", "sqlite"),
        ("h2", "h2"),
        ("sqlserver", "mssql"),
    ];
    let class = class.trim();
    for (prefix, dialect) in BY_CLASS {
        if class.starts_with(prefix) {
            return dialect;
        }
    }
    let scheme = url_template
        .strip_prefix("jdbc:")
        .and_then(|rest| rest.split(':').next())
        .unwrap_or_default();
    for (name, dialect) in BY_SCHEME {
        if scheme.eq_ignore_ascii_case(name) {
            return dialect;
        }
    }
    "generic"
}

/// An id for a driver rudbgen has no definition of.
///
/// The name in a form a JSON key and a URL both tolerate, and a UUID's first
/// eight characters after it: two people importing two configurations into one
/// installation both have a driver called *Our Warehouse*, and only one of them
/// may own the id `our-warehouse`.
fn fresh_id(name: &str) -> String {
    let mut slug = String::with_capacity(name.len());
    let mut hyphen = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.extend(ch.to_lowercase());
            hyphen = false;
        } else if !hyphen && !slug.is_empty() {
            slug.push('-');
            hyphen = true;
        }
    }
    let slug = slug.trim_end_matches('-');
    let unique = Uuid::new_v4().simple().to_string();
    match slug.is_empty() {
        true => format!("driver-{}", &unique[..8]),
        false => format!("{slug}-{}", &unique[..8]),
    }
}

/// The string, unless there is nothing in it.
fn non_empty(value: Option<&str>) -> Option<&str> {
    value.filter(|text| !text.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_three_named_placeholders_are_translated_and_the_rest_keep_their_names() {
        assert_eq!(
            convert_url_template("jdbc:postgresql://<databaseHost>:5432/<database>"),
            (
                "jdbc:postgresql://{host}:{port}/{database}".to_string(),
                Some(5432)
            )
        );
        assert_eq!(
            convert_url_template("jdbc:sqlite:<database file>"),
            ("jdbc:sqlite:{file}".to_string(), None)
        );
        assert_eq!(
            convert_url_template("jdbc:example://<databaseHost>/<serviceName>"),
            ("jdbc:example://{host}/{serviceName}".to_string(), None)
        );
    }

    #[test]
    fn h2s_optional_port_becomes_the_same_hole_as_everyone_elses() {
        assert_eq!(
            convert_url_template("jdbc:h2:tcp://<databaseHost>[:9092]/<database file>"),
            ("jdbc:h2:tcp://{host}:{port}/{file}".to_string(), Some(9092))
        );
    }

    #[test]
    fn cubrids_colon_grammar_survives_the_port_being_lifted_out() {
        assert_eq!(
            convert_url_template("jdbc:cubrid:<databaseHost>:33000:<database>:public::"),
            (
                "jdbc:cubrid:{host}:{port}:{database}:public::".to_string(),
                Some(33000)
            )
        );
    }

    #[test]
    fn a_template_with_no_host_or_no_port_keeps_its_shape() {
        assert_eq!(
            convert_url_template("jdbc:h2:<database file>"),
            ("jdbc:h2:{file}".to_string(), None)
        );
        assert_eq!(
            convert_url_template("jdbc:x://<databaseHost>/db"),
            ("jdbc:x://{host}/db".to_string(), None)
        );
    }

    #[test]
    fn a_port_that_is_not_a_port_is_left_where_it_is() {
        assert_eq!(
            convert_url_template("jdbc:x://<databaseHost>:99999/db"),
            ("jdbc:x://{host}:99999/db".to_string(), None)
        );
    }

    #[test]
    fn an_unclosed_placeholder_stops_the_rewrite_rather_than_eating_the_url() {
        assert_eq!(
            convert_url_template("jdbc:x://<databaseHost>/<database"),
            ("jdbc:x://{host}/<database".to_string(), None)
        );
    }

    #[test]
    fn a_stock_icon_becomes_an_asset_stem_and_everything_else_becomes_nothing() {
        assert_eq!(icon_stem(Some("stock:h2.png")), (Some("h2".into()), None));
        assert_eq!(icon_stem(Some("stock:mssql")), (Some("mssql".into()), None));
        assert_eq!(icon_stem(None), (None, None));
        assert_eq!(icon_stem(Some("  ")), (None, None));
        assert_eq!(
            icon_stem(Some("fa:database")),
            (None, Some("fa:database".into()))
        );
        assert_eq!(
            icon_stem(Some("/home/me/icon.png")),
            (None, Some("/home/me/icon.png".into()))
        );
    }

    #[test]
    fn the_dialect_is_read_off_the_class_and_then_off_the_scheme() {
        assert_eq!(dialect_of("org.h2.Driver", ""), "h2");
        assert_eq!(
            dialect_of("com.microsoft.sqlserver.jdbc.SQLServerDriver", ""),
            "mssql"
        );
        assert_eq!(dialect_of("", "jdbc:postgresql://x/y"), "postgres");
        assert_eq!(
            dialect_of("com.example.Driver", "jdbc:example://x"),
            "generic"
        );
    }

    #[test]
    fn an_invented_id_is_the_name_and_something_that_is_not_the_name() {
        let first = fresh_id("Our Warehouse!");
        let second = fresh_id("Our Warehouse!");
        assert!(first.starts_with("our-warehouse-"), "{first}");
        assert_ne!(first, second);
        assert!(fresh_id("!!!").starts_with("driver-"));
    }

    #[test]
    fn a_driver_type_is_found_however_it_is_capitalised() {
        let mut by_name = BTreeMap::new();
        by_name.insert("H2 Embedded".to_string(), "h2-embedded".to_string());
        assert_eq!(
            lookup_driver(&by_name, "H2 Embedded").as_deref(),
            Some("h2-embedded")
        );
        assert_eq!(
            lookup_driver(&by_name, " h2 embedded ").as_deref(),
            Some("h2-embedded")
        );
        assert_eq!(lookup_driver(&by_name, "  "), None);
        assert_eq!(lookup_driver(&by_name, "Gone"), None);
    }

    #[test]
    fn h2s_two_built_ins_are_told_apart_by_name() {
        let builtins = DriverDef::builtins();
        let embedded = JdbDriver {
            name: "H2 Embedded".into(),
            driver_class: "org.h2.Driver".into(),
            ..JdbDriver::default()
        };
        let server = JdbDriver {
            name: "H2 Server".into(),
            driver_class: "org.h2.Driver".into(),
            ..JdbDriver::default()
        };
        let neither = JdbDriver {
            name: "H2 Whatever".into(),
            driver_class: "org.h2.Driver".into(),
            ..JdbDriver::default()
        };
        assert_eq!(
            match_builtin(&builtins, &embedded).unwrap().id,
            "h2-embedded"
        );
        assert_eq!(match_builtin(&builtins, &server).unwrap().id, "h2-server");
        assert_eq!(
            match_builtin(&builtins, &neither).unwrap().id,
            "h2-embedded"
        );
    }

    #[test]
    fn a_class_no_built_in_names_matches_nothing() {
        let builtins = DriverDef::builtins();
        let unknown = JdbDriver {
            name: "Ours".into(),
            driver_class: "com.example.Driver".into(),
            ..JdbDriver::default()
        };
        let classless = JdbDriver {
            name: "Ours".into(),
            ..JdbDriver::default()
        };
        assert!(match_builtin(&builtins, &unknown).is_none());
        assert!(match_builtin(&builtins, &classless).is_none());
    }

    #[test]
    fn an_override_that_is_switched_off_but_written_still_comes_across() {
        let base = CustomQueries {
            tables: CustomQuery::on("built in"),
            ..CustomQueries::default()
        };
        let driver = JdbDriver {
            use_tables: false,
            tables_sql: Some("mine".into()),
            ..JdbDriver::default()
        };
        let merged = merge_queries(&base, &driver);
        assert!(!merged.tables.enabled);
        assert_eq!(merged.tables.sql, "mine");
    }

    #[test]
    fn an_entry_that_mentions_no_override_leaves_the_built_ins_alone() {
        let base = CustomQueries {
            tables: CustomQuery::on("built in"),
            ..CustomQueries::default()
        };
        let merged = merge_queries(&base, &JdbDriver::default());
        assert_eq!(merged, base);
    }
}
