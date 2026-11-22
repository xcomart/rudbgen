//! The dialect profiles of `<config>/dialects`: loading them, and keeping the
//! ones rudbgen ships there.
//!
//! A profile is a YAML file describing a database product on top of a dialect
//! the applications know. The format, the directory's rules and the profiles
//! both applications ship are rudbkit's (`rudbkit_core::dialects`), and
//! rudbman reads the same files; its `docs/dialect-profiles.md` fixes them.
//! rudbgen takes two things from a profile:
//!
//! * the **driver definition** its `driver` section brings, offered to
//!   `drivers.json` once, the way a built-in released later is
//!   ([`crate::DriverStore`]);
//! * the **`bridge` half**, merged down its chain of `base`s, which a session
//!   of a driver naming the profile hands the bridge ([`Report::session_profile`]).
//!   rudbgen's bridge reads the keys it answers from — `detect`,
//!   `catalog_is_schema` and `comments` — and stands on the built-in the chain
//!   ends in for everything else.
//!
//! The `sql` half is rudbman's and is read without being looked at. A
//! profile's Groovy script is not run: rudbgen's bridge has no hooks.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use rudbkit_core::dialects::{
    self as shared, Listing, ParseError, ProfileFile, built_in, listing, pending_beside,
    profile_files, read_text,
};
use serde::de::IgnoredAny;

use crate::DriverDef;
use crate::paths::dialects_dir;

pub use rudbkit_core::dialects::{SHIPPED, Shipped, Synced, sync_shipped};

/// What one profile file came to.
#[derive(Debug, Clone)]
pub struct Entry {
    /// The profile's id, its file stem.
    pub id: String,
    /// The file.
    pub path: PathBuf,
    /// The name the file gives, once it reads.
    pub name: Option<String>,
    /// The base the file names, once it reads.
    pub base: Option<String>,
    /// Other names a driver's `dialect` may give the profile by.
    pub aliases: Vec<String>,
    /// Whether it loaded.
    pub state: State,
    /// The driver definition the profile brings, if it has a `driver`
    /// section.
    pub driver: Option<DriverDef>,
    /// The script `bridge.script` names, as written. Never run here.
    pub script: Option<String>,
    /// Newer shipped versions of its files, written beside the user's own
    /// (`<file>.new`).
    pub pending: Vec<PathBuf>,
    /// Its own `bridge` half, as JSON, before its base's is merged under it.
    bridge: serde_json::Value,
    /// What a session opened with the profile hands the bridge, once it has
    /// loaded: its id, the built-in its chain of bases ends in, and its
    /// `bridge` half merged along that chain.
    pub session: Option<serde_json::Value>,
}

impl Entry {
    /// Why it did not load, when it did not.
    pub fn failure(&self) -> Option<&str> {
        match &self.state {
            State::Loaded => None,
            State::Failed(message) => Some(message),
        }
    }

    /// The built-in dialect its chain of bases ends in, once it has loaded.
    pub fn built_in(&self) -> Option<&str> {
        self.session.as_ref()?["base"].as_str()
    }
}

/// Whether a profile loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum State {
    /// Loaded: its sessions open with it.
    Loaded,
    /// Not loaded, for this reason. Its drivers' sessions read metadata as
    /// though it were not there.
    Failed(String),
}

/// What loading `<config>/dialects` came to.
#[derive(Debug, Clone, Default)]
pub struct Report {
    /// The directory read, when there is one.
    pub dir: Option<PathBuf>,
    /// Every profile file, by id.
    pub entries: Vec<Entry>,
    /// What could not be read or kept apart from any one profile: the
    /// directory itself, or the shipped files.
    pub problems: Vec<String>,
}

impl Report {
    /// Where among [`Self::entries`] the profile a driver's `dialect` names
    /// is: by its id or one of its aliases, ignoring ASCII case.
    pub fn find(&self, dialect: &str) -> Option<usize> {
        let dialect = dialect.trim();
        self.entries.iter().position(|entry| {
            entry.id.eq_ignore_ascii_case(dialect)
                || entry
                    .aliases
                    .iter()
                    .any(|alias| alias.eq_ignore_ascii_case(dialect))
        })
    }

    /// The profile a driver's `dialect` names; see [`Self::find`].
    pub fn entry_for(&self, dialect: &str) -> Option<&Entry> {
        self.find(dialect).map(|at| &self.entries[at])
    }

    /// The driver definitions the loaded profiles bring.
    pub fn drivers(&self) -> Vec<DriverDef> {
        self.entries
            .iter()
            .filter(|entry| entry.state == State::Loaded)
            .filter_map(|entry| entry.driver.clone())
            .collect()
    }

    /// What a session whose driver names `dialect` opens with, when that is
    /// a profile that loaded.
    pub fn session_profile(&self, dialect: &str) -> Option<serde_json::Value> {
        self.entry_for(dialect)?.session.clone()
    }

    /// The loaded profiles that name products by their product name
    /// (`bridge.detect`), for a session whose driver names no profile.
    pub fn detect_profiles(&self) -> Vec<serde_json::Value> {
        self.entries
            .iter()
            .filter_map(|entry| entry.session.clone())
            .filter(|session| {
                session["bridge"]["detect"]
                    .as_array()
                    .is_some_and(|words| !words.is_empty())
            })
            .collect()
    }

    /// Settles every read profile: one whose chain of bases ends in a
    /// built-in loads, with its `bridge` half merged along the chain; any
    /// other fails, saying where the chain broke.
    fn resolve(&mut self) {
        for at in 0..self.entries.len() {
            if self.entries[at].state != State::Loaded {
                continue;
            }
            let mut halves = vec![self.entries[at].bridge.clone()];
            let mut seen = vec![at];
            let mut base = self.entries[at].base.clone().unwrap_or_default();
            let outcome = loop {
                // A profile of a built-in's name is refused below, so the
                // built-in is what a name of one means.
                if let Some(found) = built_in(&base) {
                    break Ok(found);
                }
                let Some(next) = self.find(&base) else {
                    break Err(format!(
                        "base: no built-in dialect or profile is called `{base}`"
                    ));
                };
                if seen.contains(&next) {
                    break Err(format!("base: `{base}` is its own base, through its chain"));
                }
                if self.entries[next].name.is_none() {
                    break Err(format!("`{base}` did not load"));
                }
                seen.push(next);
                halves.push(self.entries[next].bridge.clone());
                base = self.entries[next].base.clone().unwrap_or_default();
            };
            let entry = &mut self.entries[at];
            match outcome {
                Err(message) => entry.state = State::Failed(message),
                Ok(builtin) => {
                    let bridge = halves
                        .iter()
                        .rev()
                        .fold(serde_json::json!({}), |under, over| {
                            shared::bridge::merged(&under, over)
                        });
                    entry.session = Some(serde_json::json!({
                        "id": entry.id,
                        "base": builtin,
                        "bridge": bridge,
                    }));
                }
            }
        }
    }
}

/// Reads every profile in `dir` and resolves them together.
///
/// A directory that does not exist holds no profiles; one that cannot be read
/// is a problem of the report.
pub fn load_from(dir: &Path) -> Report {
    let mut report = Report {
        dir: Some(dir.to_path_buf()),
        ..Report::default()
    };
    let files = match profile_files(dir) {
        Ok(files) => files,
        Err(error) => {
            if error.kind() != io::ErrorKind::NotFound {
                report.problems.push(format!("{}: {error}", dir.display()));
            }
            return report;
        }
    };
    for (id, path) in files {
        let mut entry = Entry {
            id: id.clone(),
            path: path.clone(),
            name: None,
            base: None,
            aliases: Vec::new(),
            state: State::Loaded,
            driver: None,
            script: None,
            pending: Vec::new(),
            bridge: serde_json::json!({}),
            session: None,
        };
        let read: Result<ProfileFile<IgnoredAny>, String> =
            read_text(&path).and_then(|(_, text)| {
                shared::parse(&id, &text).map_err(|error: ParseError| error.message)
            });
        match read {
            Err(message) => entry.state = State::Failed(message),
            Ok(_) if built_in(&id).is_some() => {
                entry.state = State::Failed(format!(
                    "`{id}` is a built-in dialect's name; a profile of it needs a name of its own"
                ));
            }
            Ok(file) => {
                entry.script = file.script().map(str::to_string);
                if let Some(bridge) = &file.bridge {
                    entry.bridge =
                        serde_json::to_value(bridge).unwrap_or_else(|_| serde_json::json!({}));
                }
                entry.driver = file
                    .driver
                    .as_ref()
                    .map(|driver| DriverDef::of_profile(driver, &id, &file.name));
                entry.aliases = file.aliases;
                entry.name = Some(file.name);
                entry.base = Some(file.base);
            }
        }
        entry.pending = pending_beside(dir, &entry.path, &entry.id, entry.script.as_deref());
        report.entries.push(entry);
    }
    report.resolve();
    report
}

/// What the last [`reload`] came to, and the directory as it stood then.
static CURRENT: RwLock<Option<(Arc<Report>, Listing)>> = RwLock::new(None);

/// What [`start_up`]'s sync could not do, kept for every later [`reload`].
static SYNC_PROBLEMS: RwLock<Vec<String>> = RwLock::new(Vec::new());

/// What the last [`reload`] came to; an empty report before the first.
pub fn current() -> Arc<Report> {
    CURRENT
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        .map(|(report, _)| report.clone())
        .unwrap_or_default()
}

/// Loads `<config>/dialects` in place of the last load — unless no file of
/// the directory has changed since, when that load's report stands.
///
/// The problems of the last [`start_up`]'s sync are kept: they are about the
/// directory, not about this read of it.
pub fn reload() -> Arc<Report> {
    let dir = dialects_dir();
    let now = dir.as_deref().map(listing).unwrap_or_default();
    if let Some((report, then)) = CURRENT
        .read()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .as_ref()
        && *then == now
    {
        return report.clone();
    }
    let mut report = match dir {
        Ok(dir) => load_from(&dir),
        Err(error) => Report {
            problems: vec![format!("{error:#}")],
            ..Report::default()
        },
    };
    report.problems.extend(
        SYNC_PROBLEMS
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .iter()
            .cloned(),
    );
    for entry in &report.entries {
        if let Some(message) = entry.failure() {
            log::warn!("dialect profile {} did not load: {message}", entry.id);
        }
    }
    let report = Arc::new(report);
    *CURRENT
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some((report.clone(), now));
    report
}

/// Brings the shipped profiles into `<config>/dialects` and loads the
/// directory: what the application does once, before the driver definitions
/// are read.
pub fn start_up() -> Arc<Report> {
    let synced = dialects_dir().and_then(|dir| sync_shipped(&dir, SHIPPED));
    let problems = match synced {
        Ok(synced) => {
            for name in &synced.written {
                log::info!("dialect profile file {name} written");
            }
            for name in &synced.replaced {
                log::info!("dialect profile file {name} updated");
            }
            for name in &synced.beside {
                log::info!(
                    "dialect profile file {name} changed by its user; its update is beside it"
                );
            }
            Vec::new()
        }
        Err(error) => {
            log::warn!("shipped dialect profiles not kept: {error:#}");
            vec![format!("{error:#}")]
        }
    };
    *SYNC_PROBLEMS
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = problems;
    *CURRENT
        .write()
        .unwrap_or_else(|poisoned| poisoned.into_inner()) = None;
    reload()
}

#[cfg(test)]
mod tests;
