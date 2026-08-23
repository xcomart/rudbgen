//! Named lists of templates — jdbgen's presets — persisted as
//! `template-sets.json`.
//!
//! A [`TemplateSet`] is a name and an ordered list of [`TemplateRef`]s. It is
//! *not* where a run's ticks live: those belong to the connection, in
//! [`GenerationProfile`](crate::GenerationProfile). A set is the thing the
//! Generate tab's selector loads *into* that profile, so the same list can be
//! applied to a second database without retyping it.
//!
//! The file follows the same discipline as `settings.json`: a missing file is a
//! first run rather than an error, a UTF-8 BOM is tolerated, missing keys fall
//! back to their defaults, top-level keys this build does not know are kept
//! verbatim so a newer build's file survives a save from here, and every write
//! is atomic.
//!
//! Nothing here reads a template file. A set names templates; rendering them is
//! the engine's job and resolving their paths is the app layer's.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::paths::{strip_bom, template_sets_file, write_atomic};
use crate::profile::TemplateRef;

/// A named, ordered list of templates.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateSet {
    /// Stable identifier.
    ///
    /// An id rather than the name, because a set is referenced while it is
    /// being renamed, and two sets may briefly share a name while the user is
    /// typing one.
    pub id: Uuid,
    /// Name shown in the Generate tab's selector.
    pub name: String,
    /// The templates, in the order the list shows them.
    ///
    /// [`TemplateRef::selected`] is meaningful here too: a set may ship a
    /// template unticked, which is how jdbgen's presets offer an optional
    /// output without forcing it.
    pub templates: Vec<TemplateRef>,
}

impl Default for TemplateSet {
    /// An empty set with a fresh id.
    fn default() -> Self {
        Self {
            id: Uuid::new_v4(),
            name: String::new(),
            templates: Vec::new(),
        }
    }
}

impl TemplateSet {
    /// Create a set with a freshly generated identifier.
    pub fn new(name: impl Into<String>, templates: Vec<TemplateRef>) -> Self {
        Self {
            name: name.into(),
            templates,
            ..Self::default()
        }
    }
}

/// Every saved [`TemplateSet`], persisted as `template-sets.json`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct TemplateSetStore {
    /// Schema version of the file; see [`TemplateSetStore::CURRENT_VERSION`].
    pub version: u32,
    /// Sets in the order the selector shows them.
    pub sets: Vec<TemplateSet>,
    /// Top-level keys this build does not know, kept verbatim.
    ///
    /// The same round-tripping [`AppSettings::extra`](crate::AppSettings) does,
    /// and for the same reason: running a beta beside a release against one
    /// config directory must not be destructive.
    #[serde(flatten)]
    pub extra: BTreeMap<String, serde_json::Value>,
}

impl Default for TemplateSetStore {
    fn default() -> Self {
        Self {
            version: Self::CURRENT_VERSION,
            sets: Vec::new(),
            extra: BTreeMap::new(),
        }
    }
}

impl TemplateSetStore {
    /// Schema version written by this build.
    ///
    /// A file carrying a different number still loads: unknown keys are kept
    /// and missing ones default, so the version is informational until a real
    /// migration is needed.
    pub const CURRENT_VERSION: u32 = 1;

    /// Load the store from the default configuration file.
    ///
    /// A missing file yields an empty store, which is what a first run looks
    /// like — the built-in set is copied in by the app layer, not invented
    /// here.
    ///
    /// # Errors
    ///
    /// Fails when the configuration directory cannot be determined, the file
    /// cannot be read, or its contents are not valid JSON.
    pub fn load() -> Result<Self> {
        Self::load_from(&template_sets_file()?)
    }

    /// Load the store from an explicit path.
    ///
    /// A missing file yields [`TemplateSetStore::default`]. A leading UTF-8
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
            .with_context(|| format!("failed to parse template sets from {}", path.display()))
    }

    /// Write the store to the default configuration file.
    ///
    /// # Errors
    ///
    /// Fails when the configuration directory cannot be determined or created,
    /// or when the file cannot be written.
    pub fn save(&self) -> Result<()> {
        self.save_to(&template_sets_file()?)
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
        let json = serde_json::to_vec_pretty(self).context("failed to serialize template sets")?;
        write_atomic(path, &json)
    }

    /// Look up a set by identifier.
    pub fn get(&self, id: Uuid) -> Option<&TemplateSet> {
        self.sets.iter().find(|set| set.id == id)
    }

    /// Insert `set`, replacing an existing entry with the same identifier.
    ///
    /// Replacement keeps the original position in the list.
    pub fn upsert(&mut self, set: TemplateSet) {
        match self.sets.iter_mut().find(|existing| existing.id == set.id) {
            Some(slot) => *slot = set,
            None => self.sets.push(set),
        }
    }

    /// Remove the set with the given identifier and return it.
    pub fn remove(&mut self, id: Uuid) -> Option<TemplateSet> {
        let index = self.sets.iter().position(|set| set.id == id)?;
        Some(self.sets.remove(index))
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn sample(name: &str) -> TemplateSet {
        TemplateSet::new(
            name,
            vec![TemplateRef {
                name: "Java Model".to_string(),
                file: PathBuf::from("templates/java_model.java"),
                out_template: "${table:camel}Model.java".to_string(),
                selected: true,
            }],
        )
    }

    #[test]
    fn a_new_set_gets_a_fresh_identifier() {
        assert_ne!(sample("a").id, sample("a").id);
    }

    #[test]
    fn load_from_missing_file_is_an_empty_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = TemplateSetStore::load_from(&dir.path().join("absent.json")).expect("load");
        assert_eq!(store, TemplateSetStore::default());
        assert!(store.sets.is_empty());
        assert_eq!(store.version, 1);
    }

    #[test]
    fn save_to_load_from_round_trip_keeps_the_order() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("cfg").join("template-sets.json");

        let mut store = TemplateSetStore::default();
        store.upsert(sample("Java + MyBatis"));
        store.upsert(sample("PHP CI"));
        store.save_to(&path).expect("save");

        let loaded = TemplateSetStore::load_from(&path).expect("load");
        assert_eq!(loaded, store);
        assert_eq!(
            loaded
                .sets
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>(),
            ["Java + MyBatis", "PHP CI"]
        );

        // Saving over an existing file must work too.
        loaded.save_to(&path).expect("overwrite");
        assert_eq!(TemplateSetStore::load_from(&path).expect("reload"), loaded);
    }

    #[test]
    fn upsert_replaces_in_place_and_remove_returns_the_set() {
        let mut store = TemplateSetStore::default();
        let keep = sample("keep");
        let mut edited = sample("original");
        store.upsert(keep.clone());
        store.upsert(edited.clone());

        edited.name = "renamed".to_string();
        store.upsert(edited.clone());
        assert_eq!(store.sets.len(), 2);
        assert_eq!(store.sets[0].id, keep.id);
        assert_eq!(
            store.get(edited.id).map(|s| s.name.as_str()),
            Some("renamed")
        );

        assert_eq!(store.remove(edited.id), Some(edited.clone()));
        assert_eq!(store.remove(edited.id), None);
        assert_eq!(store.get(edited.id), None);
    }

    #[test]
    fn a_hand_written_file_fills_the_rest_in() {
        let json = r#"{"sets":[{"name":"minimal","templates":[{"file":"a.java"}]}]}"#;
        let store: TemplateSetStore = serde_json::from_str(json).expect("parse");
        assert_eq!(store.version, 1, "a missing version defaults");
        let set = &store.sets[0];
        assert_eq!(set.name, "minimal");
        assert_ne!(set.id, Uuid::nil(), "a missing id gets a fresh one");
        assert_eq!(set.templates[0].file, PathBuf::from("a.java"));
        assert!(!set.templates[0].selected);
    }

    #[test]
    fn load_from_tolerates_a_utf8_bom() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("template-sets.json");

        let mut store = TemplateSetStore::default();
        store.upsert(sample("bom"));
        store.save_to(&path).expect("save");

        let saved = fs::read(&path).expect("read");
        let mut with_bom = b"\xEF\xBB\xBF".to_vec();
        with_bom.extend_from_slice(&saved);
        fs::write(&path, with_bom).expect("write");

        assert_eq!(
            TemplateSetStore::load_from(&path).expect("load").sets[0].name,
            "bom"
        );
    }

    #[test]
    fn unknown_keys_survive_a_load_and_save() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("template-sets.json");
        fs::write(
            &path,
            br#"{"version": 99, "sets": [], "from_the_future": {"a": [1, 2]}}"#,
        )
        .expect("write");

        let store = TemplateSetStore::load_from(&path).expect("load");
        assert_eq!(store.version, 99);
        assert_eq!(
            store.extra.get("from_the_future"),
            Some(&serde_json::json!({"a": [1, 2]}))
        );

        store.save_to(&path).expect("save");
        let text = fs::read_to_string(&path).expect("read");
        assert!(text.contains("from_the_future"), "got {text}");
        assert_eq!(TemplateSetStore::load_from(&path).expect("reload"), store);
    }

    #[test]
    fn load_from_invalid_json_fails_without_touching_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("template-sets.json");
        fs::write(&path, b"{ nope").expect("write");

        let err = TemplateSetStore::load_from(&path).expect_err("must be an error");
        assert!(
            err.to_string().contains("failed to parse template sets"),
            "unhelpful error: {err:#}"
        );
        assert_eq!(fs::read_to_string(&path).unwrap(), "{ nope");
    }
}
