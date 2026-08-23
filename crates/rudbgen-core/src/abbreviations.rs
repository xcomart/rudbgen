//! Abbreviation rules, persisted as `abbreviations.json`.
//!
//! A rule rewrites a piece of an identifier on its way into a generated name:
//! `EMP` → `Employee`, `NO` → `Number`. jdbgen keeps them in one global list
//! rather than per connection, and so does this — a shop's vocabulary is a fact
//! about the shop, not about which database is open.
//!
//! **This module stores the rules; it does not apply them.** Matching — which
//! is where the one deliberate break from jdbgen lives (architecture document,
//! D10: word rules match case-insensitively) — belongs to the template engine,
//! which is the only place that knows how a name was split into words. Keeping
//! the two apart is what lets the rules editor be tested without an engine and
//! the engine be tested without a config directory.
//!
//! The file follows the same discipline as `settings.json`: a missing file is a
//! first run rather than an error, a UTF-8 BOM is tolerated, missing keys fall
//! back to their defaults, top-level keys this build does not know are kept
//! verbatim, and every write is atomic.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::paths::{abbreviations_file, strip_bom, write_atomic};

/// One rewrite of an identifier or of a word inside one.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AbbreviationRule {
    /// Whether this rule takes part in a run.
    ///
    /// A tick, not a deletion: switching a rule off for one project must not
    /// cost the user the rule.
    pub enabled: bool,
    /// Whether [`AbbreviationRule::abbreviation`] is a whole identifier rather
    /// than a word inside one.
    ///
    /// A whole-name rule replaces the entire table name and nothing else —
    /// which is how a table whose name is an acronym gets a readable class —
    /// while a word rule replaces one segment wherever it appears.
    pub whole_name: bool,
    /// What to look for: a whole identifier, or a word inside one.
    pub abbreviation: String,
    /// What to put in its place.
    pub replacement: String,
}

/// Every saved [`AbbreviationRule`], persisted as `abbreviations.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AbbreviationStore {
    /// Schema version of the file; see [`AbbreviationStore::CURRENT_VERSION`].
    pub version: u32,
    /// Whether the rules are applied at all.
    ///
    /// The Generate tab's single switch, above the list: turning the whole
    /// mechanism off is one click rather than unticking every rule, and the
    /// rules are still there when it is turned back on. Off by default,
    /// because a name rewritten by a rule the user never wrote is a surprise
    /// in the output.
    pub apply_to_names: bool,
    /// The rules, in the order the editor shows them.
    ///
    /// Order is meaningful: word rules are applied in it, so a longer
    /// abbreviation placed above a shorter one that is its prefix is how the
    /// user resolves the overlap. Nothing here reorders them.
    pub rules: Vec<AbbreviationRule>,
    /// Top-level keys this build does not know, kept verbatim.
    ///
    /// The same round-tripping [`AppSettings::extra`](crate::AppSettings) does.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for AbbreviationStore {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            apply_to_names: false,
            rules: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl AbbreviationStore {
    /// Schema version written by this build.
    ///
    /// A file carrying a different number still loads: unknown keys are kept
    /// and missing ones default, so the version is informational until a real
    /// migration is needed.
    pub const CURRENT_VERSION: u32 = 1;

    /// Load the store from the default configuration file.
    ///
    /// A missing file yields [`AbbreviationStore::default`] — no rules, and the
    /// mechanism off.
    ///
    /// # Errors
    ///
    /// Fails when the configuration directory cannot be determined, the file
    /// cannot be read, or its contents are not valid JSON.
    pub fn load() -> Result<Self> {
        Self::load_from(&abbreviations_file()?)
    }

    /// Load the store from an explicit path.
    ///
    /// A missing file yields [`AbbreviationStore::default`]. A leading UTF-8
    /// byte order mark is tolerated and unknown keys are preserved.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or does not contain valid JSON.
    pub fn load_from(path: &Path) -> Result<Self> {
        let data = match fs::read(path) {
            Ok(data) => data,
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(Self::default()),
            Err(err) => {
                return Err(err).with_context(|| format!("failed to read {}", path.display()));
            }
        };
        serde_json::from_slice(strip_bom(&data))
            .with_context(|| format!("failed to parse abbreviations from {}", path.display()))
    }

    /// Write the store to the default configuration file.
    ///
    /// # Errors
    ///
    /// Fails when the configuration directory cannot be determined or created,
    /// or when the file cannot be written.
    pub fn save(&self) -> Result<()> {
        self.save_to(&abbreviations_file()?)
    }

    /// Write the store to an explicit path, creating parent directories.
    ///
    /// The write is atomic: the data lands in a temporary sibling file that is
    /// then renamed over `path`.
    ///
    /// # Errors
    ///
    /// Fails when the parent directory cannot be created or the file cannot be
    /// written.
    pub fn save_to(&self, path: &Path) -> Result<()> {
        let json = serde_json::to_vec_pretty(self).context("failed to serialize abbreviations")?;
        write_atomic(path, &json)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rule(abbreviation: &str, replacement: &str) -> AbbreviationRule {
        AbbreviationRule {
            enabled: true,
            whole_name: false,
            abbreviation: abbreviation.to_string(),
            replacement: replacement.to_string(),
        }
    }

    #[test]
    fn the_defaults_leave_names_alone() {
        let store = AbbreviationStore::default();
        assert_eq!(store.version, 1);
        assert!(!store.apply_to_names);
        assert!(store.rules.is_empty());
    }

    #[test]
    fn load_from_missing_file_is_default() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = AbbreviationStore::load_from(&dir.path().join("absent.json")).expect("load");
        assert_eq!(store, AbbreviationStore::default());
    }

    #[test]
    fn save_to_load_from_round_trip_keeps_the_rule_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cfg").join("abbreviations.json");

        let store = AbbreviationStore {
            apply_to_names: true,
            rules: vec![
                rule("EMP", "Employee"),
                rule("NO", "Number"),
                AbbreviationRule {
                    enabled: false,
                    whole_name: true,
                    abbreviation: "T_SAMPLE_ALBUM".to_string(),
                    replacement: "Album".to_string(),
                },
            ],
            ..AbbreviationStore::default()
        };

        store.save_to(&path).expect("save");
        let loaded = AbbreviationStore::load_from(&path).expect("load");
        assert_eq!(loaded, store);
        assert_eq!(
            loaded
                .rules
                .iter()
                .map(|r| r.abbreviation.as_str())
                .collect::<Vec<_>>(),
            ["EMP", "NO", "T_SAMPLE_ALBUM"],
            "the order the user arranged is what resolves overlaps"
        );

        // Saving over an existing file must work too.
        loaded.save_to(&path).expect("overwrite");
        assert_eq!(AbbreviationStore::load_from(&path).expect("reload"), loaded);
    }

    #[test]
    fn a_hand_written_rule_fills_the_rest_in() {
        let json = r#"{"rules":[{"abbreviation":"EMP","replacement":"Employee"}]}"#;
        let store: AbbreviationStore = serde_json::from_str(json).expect("parse");
        assert_eq!(store.version, 1, "a missing version defaults");
        assert!(!store.apply_to_names);
        let parsed = &store.rules[0];
        assert_eq!(parsed.abbreviation, "EMP");
        assert_eq!(parsed.replacement, "Employee");
        assert!(!parsed.enabled, "a half-typed row must not join a run");
        assert!(!parsed.whole_name);
    }

    #[test]
    fn load_from_tolerates_a_utf8_bom() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("abbreviations.json");

        let mut with_bom = b"\xEF\xBB\xBF".to_vec();
        with_bom.extend_from_slice(br#"{"apply_to_names":true}"#);
        fs::write(&path, with_bom).expect("write");

        assert!(
            AbbreviationStore::load_from(&path)
                .expect("load")
                .apply_to_names
        );
    }

    #[test]
    fn unknown_keys_survive_a_load_and_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("abbreviations.json");
        fs::write(
            &path,
            br#"{"version": 99, "rules": [], "from_the_future": {"a": [1, 2]}}"#,
        )
        .expect("write");

        let store = AbbreviationStore::load_from(&path).expect("load");
        assert_eq!(store.version, 99);
        assert_eq!(
            store.extra.get("from_the_future"),
            Some(&serde_json::json!({"a": [1, 2]}))
        );

        store.save_to(&path).expect("save");
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("from_the_future"), "got {text}");
        assert_eq!(AbbreviationStore::load_from(&path).expect("reload"), store);
    }

    #[test]
    fn load_from_invalid_json_fails_without_touching_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("abbreviations.json");
        fs::write(&path, b"{ nope").expect("write");

        let err = AbbreviationStore::load_from(&path).expect_err("must be an error");
        assert!(
            err.to_string().contains("failed to parse abbreviations"),
            "unhelpful error: {err:#}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ nope");
    }
}
