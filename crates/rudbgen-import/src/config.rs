//! The shape of jdbgen's `config.json`, field for field.
//!
//! These are transcriptions of `comart.tools.jdbgen.types.JDBGenConfig` and its
//! neighbours, with jdbgen's spelling kept in the `serde` attributes and Rust's
//! in the field names. Nothing here interprets a value: `connection_url`,
//! `user_name` and `user_password` still hold ciphertext after [`read`], a
//! `jdbc_jar` is still whatever string the file carried, and `keep_alive_sec`
//! is still a string, because jdbgen stores it as one. Interpretation is
//! [`crate::map`]'s job.
//!
//! Reading is forgiving, for the same reason `rudbgen-core`'s stores are: this
//! file was hand-edited by whoever is now importing it. A missing array is an
//! empty one, a `null` is the default, a field this build does not know is
//! ignored, and a leading UTF-8 byte order mark is stripped. What is *not*
//! forgiven is a file that is not JSON at all — there is nothing to import
//! from it.
//!
//! [`read`]: crate::read

use std::collections::BTreeMap;

use serde::{Deserialize, Deserializer, Serialize};

/// A whole jdbgen configuration.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JdbgenConfig {
    /// Whether jdbgen was last shown in its dark theme.
    ///
    /// Carried to [`SettingsHint`](crate::SettingsHint); whether rudbgen adopts
    /// it is the app's decision, not this crate's.
    #[serde(rename = "isDarkUI")]
    #[serde(deserialize_with = "lenient")]
    pub is_dark_ui: bool,
    /// Whether the abbreviation rules were being applied.
    #[serde(deserialize_with = "lenient")]
    pub apply_abbr: bool,
    /// Language tag jdbgen was last run in, `null` for the system language.
    pub language: Option<String>,
    /// Geometry of jdbgen's main window.
    ///
    /// Read and ignored: rudbgen's window is a different window, and its
    /// geometry is already in `settings.json`. Kept as a value so that its
    /// presence does not look like a parse failure.
    pub main_window: Option<serde_json::Value>,
    /// Maven search endpoints.
    ///
    /// Read and ignored for the same reason: rudbgen's downloader has its own,
    /// and a stale mirror URL is not worth carrying across.
    pub maven: Option<serde_json::Value>,
    /// Driver definitions, stock and user-made.
    #[serde(deserialize_with = "lenient")]
    pub drivers: Vec<JdbDriver>,
    /// Saved connections.
    #[serde(deserialize_with = "lenient")]
    pub connections: Vec<JdbConnection>,
    /// Named template lists — rudbgen's template sets.
    #[serde(deserialize_with = "lenient")]
    pub presets: Vec<JdbPreset>,
    /// Abbreviation rules.
    #[serde(deserialize_with = "lenient")]
    pub abbrs: Vec<JdbAbbr>,
}

/// One driver definition (`JDBDriver`).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JdbDriver {
    /// Name, which is also what a connection's `driverType` refers to.
    #[serde(deserialize_with = "lenient")]
    pub name: String,
    /// Icon locator: `stock:h2.png`, a file path, `fa:`, `color:` or a URL.
    pub icon: Option<String>,
    /// Whether jdbgen shipped this entry rather than the user writing it.
    #[serde(deserialize_with = "lenient")]
    pub stock_item: bool,
    /// Fully qualified driver class.
    #[serde(deserialize_with = "lenient")]
    pub driver_class: String,
    /// URL skeleton with `<placeholder>` holes.
    #[serde(deserialize_with = "lenient")]
    pub url_template: String,
    /// JAR path, absolute or relative to jdbgen's data or install directory.
    pub jdbc_jar: Option<String>,
    /// Maven Central search term the driver manager pre-fills.
    ///
    /// Read and ignored: rudbgen's driver editor searches by coordinate.
    pub default_query: Option<String>,
    /// Maven coordinate, `group:artifact:version`.
    pub maven_artifact: Option<String>,
    /// Whether this product has no user and no password.
    #[serde(deserialize_with = "lenient")]
    pub no_auth: bool,
    /// Driver-wide connection properties.
    ///
    /// `None` when the file does not mention them at all, which is how a
    /// hand-written entry differs from one whose properties were emptied.
    pub props: Option<BTreeMap<String, String>>,
    /// Whether the table list is overridden.
    #[serde(deserialize_with = "lenient")]
    pub use_tables: bool,
    /// The overriding table list.
    pub tables_sql: Option<String>,
    /// Whether the column list is overridden.
    #[serde(deserialize_with = "lenient")]
    pub use_columns: bool,
    /// The overriding column list.
    pub columns_sql: Option<String>,
    /// Whether table comments are read by a query of their own.
    #[serde(deserialize_with = "lenient")]
    pub use_table_comments: bool,
    /// The table-comment query.
    pub table_comments_sql: Option<String>,
    /// Whether column comments are read by a query of their own.
    #[serde(deserialize_with = "lenient")]
    pub use_column_comments: bool,
    /// The column-comment query.
    pub column_comments_sql: Option<String>,
}

/// One saved connection (`JDBConnection`).
///
/// The three encrypted fields hold ciphertext until [`crate::decrypt`] has run.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JdbConnection {
    /// Name shown in jdbgen's connection list.
    #[serde(deserialize_with = "lenient")]
    pub name: String,
    /// Icon locator, as [`JdbDriver::icon`].
    pub icon: Option<String>,
    /// **Name** of the driver this connection uses — not an id.
    #[serde(deserialize_with = "lenient")]
    pub driver_type: String,
    /// JDBC URL. Encrypted on disk.
    #[serde(deserialize_with = "lenient")]
    pub connection_url: String,
    /// Login user. Encrypted on disk.
    #[serde(deserialize_with = "lenient")]
    pub user_name: String,
    /// Password. Encrypted on disk.
    #[serde(deserialize_with = "lenient")]
    pub user_password: String,
    /// Per-connection JDBC properties.
    #[serde(deserialize_with = "lenient")]
    pub connection_props: BTreeMap<String, String>,
    /// Whether the idle keep-alive probe is on.
    #[serde(deserialize_with = "lenient")]
    pub use_keep_alive: bool,
    /// Seconds between probes — a *string*, as jdbgen stores it.
    pub keep_alive_sec: Option<String>,
    /// The probe statement.
    pub keep_alive_query: Option<String>,
    /// Templates offered for this connection, with their ticks.
    #[serde(deserialize_with = "lenient")]
    pub templates: Vec<JdbTemplate>,
    /// Where generated files are written.
    pub output_dir: Option<String>,
    /// Value of the template language's `author` item.
    pub author: Option<String>,
    /// Custom variables, in the order the file lists them.
    #[serde(deserialize_with = "ordered_pairs")]
    pub custom_vars: Vec<(String, String)>,
}

/// One template in a connection or a preset (`JDBTemplate`).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JdbTemplate {
    /// Name shown in the template list.
    #[serde(deserialize_with = "lenient")]
    pub name: String,
    /// Path of the template body, absolute or relative.
    #[serde(deserialize_with = "lenient")]
    pub template_file: String,
    /// Template rendered to get the output file name.
    #[serde(deserialize_with = "lenient")]
    pub out_template: String,
    /// Whether this template takes part in a run.
    #[serde(deserialize_with = "lenient")]
    pub selected: bool,
}

/// One named template list (`JDBPreset`).
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JdbPreset {
    /// Name of the preset.
    #[serde(deserialize_with = "lenient")]
    pub name: String,
    /// Icon locator, as [`JdbDriver::icon`].
    pub icon: Option<String>,
    /// The templates it holds.
    #[serde(deserialize_with = "lenient")]
    pub templates: Vec<JdbTemplate>,
}

/// One abbreviation rule (`JDBAbbr`).
///
/// The three booleans are `Boolean` in jdbgen and are genuinely absent in old
/// files, which is why they are [`Option`] here: `totalName` missing means a
/// word rule, and `check` missing means the rule is off.
#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct JdbAbbr {
    /// Whether the rule is applied.
    pub check: Option<bool>,
    /// Whether it matches a whole identifier rather than a word inside one.
    pub total_name: Option<bool>,
    /// What to look for.
    pub abbr: Option<String>,
    /// What to put in its place.
    pub replace_to: Option<String>,
}

/// Read `null` as the default rather than as an error.
///
/// Gson omits a `null` field when it writes, but a configuration that has been
/// hand-edited — which is the normal case for a file old enough to be imported
/// — carries them, and `#[serde(default)]` covers a *missing* key rather than a
/// present `null`. Every non-optional field below uses this, so
/// `"connections": null` is no connections and `"name": null` is no name.
fn lenient<'de, D, T>(deserializer: D) -> Result<T, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de> + Default,
{
    Ok(Option::<T>::deserialize(deserializer)?.unwrap_or_default())
}

/// Read a JSON object into pairs, keeping the order the document lists them in.
///
/// `customVars` is a JSON object, and the order of its keys is the order the
/// user arranged their variable table in. A `BTreeMap` would sort it and a
/// `HashMap` would scatter it, so the object is walked by hand into the
/// `Vec<(String, String)>` that
/// [`GenerationProfile::custom_vars`](rudbgen_core::GenerationProfile) already
/// is. A `null` is an empty list, which is how a connection that never had a
/// variable is written.
fn ordered_pairs<'de, D>(deserializer: D) -> Result<Vec<(String, String)>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de::{MapAccess, Visitor};
    use std::fmt;

    struct Pairs;

    impl<'de> Visitor<'de> for Pairs {
        type Value = Vec<(String, String)>;

        fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str("an object of string values, or null")
        }

        fn visit_unit<E>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_none<E>(self) -> Result<Self::Value, E> {
            Ok(Vec::new())
        }

        fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
        where
            D: Deserializer<'de>,
        {
            deserializer.deserialize_map(Pairs)
        }

        fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
        where
            M: MapAccess<'de>,
        {
            let mut pairs = Vec::with_capacity(map.size_hint().unwrap_or(0));
            while let Some((key, value)) = map.next_entry::<String, Option<String>>()? {
                pairs.push((key, value.unwrap_or_default()));
            }
            Ok(pairs)
        }
    }

    deserializer.deserialize_option(Pairs)
}

impl JdbDriver {
    /// The four SQL overrides as `(enabled, sql)` pairs, in
    /// [`CustomQueryKind::ALL`](rudbgen_core::CustomQueryKind) order.
    ///
    /// Which is tables, columns, table comments, column comments — the order
    /// the driver editor draws them in, so a mapping walking both lists at once
    /// cannot get them out of step.
    pub(crate) fn custom_queries(&self) -> [(bool, &str); 4] {
        [
            (self.use_tables, self.tables_sql.as_deref().unwrap_or("")),
            (self.use_columns, self.columns_sql.as_deref().unwrap_or("")),
            (
                self.use_table_comments,
                self.table_comments_sql.as_deref().unwrap_or(""),
            ),
            (
                self.use_column_comments,
                self.column_comments_sql.as_deref().unwrap_or(""),
            ),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_object_is_a_configuration_with_nothing_in_it() {
        let cfg: JdbgenConfig = serde_json::from_str("{}").unwrap();
        assert_eq!(cfg, JdbgenConfig::default());
        assert!(cfg.drivers.is_empty());
        assert!(cfg.connections.is_empty());
    }

    #[test]
    fn a_null_array_reads_as_an_empty_one() {
        let cfg: JdbgenConfig =
            serde_json::from_str(r#"{"drivers":null,"connections":null,"abbrs":null}"#).unwrap();
        assert!(cfg.drivers.is_empty());
        assert!(cfg.abbrs.is_empty());
    }

    #[test]
    fn a_field_this_build_does_not_know_is_ignored() {
        let cfg: JdbgenConfig =
            serde_json::from_str(r#"{"somethingNewer": {"a": 1}, "applyAbbr": true}"#).unwrap();
        assert!(cfg.apply_abbr);
    }

    #[test]
    fn custom_variables_keep_the_order_the_file_lists_them_in() {
        let conn: JdbConnection =
            serde_json::from_str(r#"{"customVars":{"package":"com.abc","zeta":"1","alpha":"2"}}"#)
                .unwrap();
        assert_eq!(
            conn.custom_vars,
            vec![
                ("package".to_string(), "com.abc".to_string()),
                ("zeta".to_string(), "1".to_string()),
                ("alpha".to_string(), "2".to_string()),
            ]
        );
    }

    #[test]
    fn a_missing_or_null_variable_map_is_no_variables() {
        let absent: JdbConnection = serde_json::from_str("{}").unwrap();
        let null: JdbConnection = serde_json::from_str(r#"{"customVars":null}"#).unwrap();
        assert!(absent.custom_vars.is_empty());
        assert!(null.custom_vars.is_empty());
    }

    #[test]
    fn a_variable_with_a_null_value_reads_as_an_empty_one() {
        let conn: JdbConnection = serde_json::from_str(r#"{"customVars":{"k":null}}"#).unwrap();
        assert_eq!(conn.custom_vars, vec![("k".to_string(), String::new())]);
    }

    #[test]
    fn the_four_overrides_come_out_in_the_order_the_editor_draws_them() {
        let driver = JdbDriver {
            use_tables: true,
            tables_sql: Some("t".into()),
            use_column_comments: true,
            column_comments_sql: Some("cc".into()),
            ..JdbDriver::default()
        };
        assert_eq!(
            driver.custom_queries(),
            [(true, "t"), (false, ""), (false, ""), (true, "cc")]
        );
    }
}
