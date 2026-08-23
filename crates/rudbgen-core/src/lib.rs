//! Platform-independent core of rudbgen: configuration paths, application
//! settings, saved connection profiles with their generation options, JDBC
//! driver definitions, template sets, abbreviation rules, OS keychain access,
//! and the trusted host key database.
//!
//! This crate owns everything rudbgen persists on disk or in the system
//! credential store. It knows nothing about JNI, template rendering, SSH
//! transport, or the GUI, so it can be exercised entirely from tests. In
//! particular it stores the abbreviation rules without applying them and names
//! the templates without reading them: matching and rendering belong to the
//! template engine, which is the only place that knows how a name splits into
//! words.
//!
//! Two rules run through all of it. Loading is forgiving — every file here is
//! meant to be hand-editable, so a missing file is a first run, a UTF-8 byte
//! order mark is stripped, missing keys default and out-of-range numbers are
//! clamped. Writing is atomic — the data lands in a temporary sibling file that
//! is renamed over the destination, so a crash mid-save cannot leave a
//! truncated configuration behind.
//!
//! ```no_run
//! use rudbgen_core::{ConnectionProfile, ConnectionStore, DriverStore};
//!
//! # fn main() -> anyhow::Result<()> {
//! rudbgen_core::init_secrets().ok(); // a missing keychain is not fatal
//!
//! let drivers = DriverStore::load()?; // built-in definitions on a first run
//! let postgres = drivers.get("postgresql").expect("built-in driver");
//!
//! let mut store = ConnectionStore::load()?;
//! store.upsert(ConnectionProfile::new(
//!     "staging",
//!     &postgres.id,
//!     "jdbc:postgresql://db.example.com:5432/app",
//!     "alice",
//! ));
//! store.save()?;
//! # Ok(())
//! # }
//! ```

#![warn(missing_docs)]

pub mod abbreviations;
pub mod known_hosts;
pub mod paths;
pub mod profile;
pub mod secrets;
pub mod settings;
pub mod template_sets;

pub use abbreviations::{AbbreviationRule, AbbreviationStore};
pub use known_hosts::{HostKeyStatus, KnownHosts};
pub use paths::{
    abbreviations_file, config_dir, connections_file, drivers_dir, drivers_file, editor_themes_dir,
    known_hosts_file, settings_file, template_sets_file, templates_dir, ui_themes_dir,
};
pub use profile::{
    ConnectionProfile, ConnectionStore, CustomQueries, CustomQuery, CustomQueryKind, DriverDef,
    DriverStore, GenerationProfile, KeepAlive, TemplateRef, TunnelAuth, TunnelConfig,
};
pub use secrets::{SecretSlot, SecretStore, init as init_secrets};
pub use settings::{AppSettings, OverwritePolicy, TitlebarStyle, WindowState};
pub use template_sets::{TemplateSet, TemplateSetStore};
