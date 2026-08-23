//! Reads a [jdbgen](https://github.com/xcomart/jdbgen) `config.json` and turns
//! it into rudbgen's stores: the one-time import of the architecture document's
//! D5.
//!
//! # Why this crate exists
//!
//! jdbgen gates the whole application behind a master password in order to
//! protect three fields — a connection's URL, its user name and its password —
//! and its own documentation lists that as the first thing users trip over.
//! rudbgen has no master password. Secrets go to the OS keychain through
//! [`rudbgen_core::SecretStore`], where the operating system already guards
//! them with the credentials the user has anyway, and everything else is plain
//! JSON that can be read, diffed and hand-edited.
//!
//! That leaves exactly one moment where jdbgen's scheme has to be understood:
//! the import. The user is asked for the master password **once**, this crate
//! decrypts with jdbgen's exact scheme — the current AES-256-GCM/PBKDF2 form
//! and the superseded AES-128/CBC one — and hands back the profiles, the
//! drivers, the template sets, the abbreviation rules and, separately, the
//! passwords. The app writes the first four to disk and the last to the
//! keychain. Nothing here writes anything.
//!
//! # What it does not do
//!
//! * **It does not keep the master password.** [`Decryptor`] holds it for the
//!   length of the call and wipes it when it is dropped; no path in this crate
//!   writes it, logs it, or renders it in a [`Debug`] impl.
//! * **It does not touch the jdbgen configuration.** The file is opened read
//!   only and left exactly as it was found, so a user who imports and then
//!   changes their mind still has a working jdbgen.
//! * **It has no user interface.** The wizard of §4.6 is the app's, and every
//!   string this crate produces is data — a [`Note`], not a sentence — because
//!   the app is the layer that has the translations.
//!
//! # The shape of an import
//!
//! ```no_run
//! use rudbgen_import::{MapOptions, decrypt, locate, map, preview, read};
//!
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let Some(path) = locate() else {
//!     return Ok(()); // nothing to import from: the wizard is not offered
//! };
//! let config = read(&path)?;                       // parses; still encrypted
//! let opened = decrypt(&config, "the master password")?;
//!
//! let options = MapOptions::new(path.parent().unwrap());
//! let found = preview(&opened, &options);           // the checklist
//! println!("{} connections, {} drivers", found.connections.len(), found.drivers.len());
//!
//! let mapped = map(&opened, &options);              // what would be written
//! for (profile, secret) in &mapped.connections {
//!     let _ = (&profile.name, &secret.password);    // profile → JSON, secret → keychain
//! }
//! # Ok(())
//! # }
//! ```
//!
//! [`Note`]: crate::Note

#![warn(missing_docs)]

pub mod config;
pub mod crypt;
pub mod error;
pub mod map;
pub mod notes;
pub mod preview;

use std::fs;
use std::path::{Path, PathBuf};

pub use config::{JdbAbbr, JdbConnection, JdbDriver, JdbPreset, JdbTemplate, JdbgenConfig};
pub use crypt::Decryptor;
pub use error::{Error, Result};
pub use map::{Decrypted, MapOptions, Mapped, Secret, SettingsHint, map};
pub use notes::{Note, PathKind};
pub use preview::{
    ConnectionPreview, DriverPreview, Preview, RulePreview, SetPreview, from_mapped, preview,
};

/// Name jdbgen gives its user data directory on every platform.
const APP_NAME: &str = "jdbgen";
/// Name of the configuration file inside it.
const CONFIG_NAME: &str = "config.json";

/// jdbgen's user data directory, whether or not it exists.
///
/// The rule is jdbgen's `AppDirs.defaultUserDataDir`, which is
/// `BaseDirs::config_dir()` with `jdbgen` under it on all three platforms:
/// `%APPDATA%\jdbgen` on Windows, `~/Library/Application Support/jdbgen` on
/// macOS, `$XDG_CONFIG_HOME/jdbgen` or `~/.config/jdbgen` elsewhere. `None`
/// only when the platform has no home directory to speak of.
///
/// jdbgen also honours a `jdbgen.dataDir` system property, which is a JVM
/// notion and has no equivalent here; a user who moved their configuration
/// points the wizard's file chooser at it instead.
pub fn data_dir() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join(APP_NAME))
}

/// The jdbgen configuration to offer an import from, if there is one.
///
/// `Some` only when the file is actually there: the welcome screen offers
/// *Import from jdbgen…* on the strength of this answer (§4.3), and an entry
/// that opens a dialog saying "no configuration found" would be worse than no
/// entry.
pub fn locate() -> Option<PathBuf> {
    data_dir()
        .map(|dir| dir.join(CONFIG_NAME))
        .filter(|path| path.is_file())
}

/// Parse a jdbgen configuration, encrypted fields and all.
///
/// Reading is separate from decrypting so the wizard can say *what* it found
/// before asking for a password — a file holding no connections needs no
/// password at all. The three encrypted fields are still ciphertext in the
/// value this returns.
///
/// A leading UTF-8 byte order mark is tolerated, missing keys default, and
/// keys this build does not know are ignored.
///
/// # Errors
///
/// [`Error::Read`] when the file cannot be read and [`Error::Parse`] when it is
/// not JSON.
pub fn read(path: &Path) -> Result<JdbgenConfig> {
    let bytes = fs::read(path).map_err(|source| Error::Read {
        path: path.to_path_buf(),
        source,
    })?;
    serde_json::from_slice(rudbgen_core::paths::strip_bom(&bytes)).map_err(|source| Error::Parse {
        path: path.to_path_buf(),
        source,
    })
}

/// Open the three encrypted fields of every connection.
///
/// The whole configuration is decrypted at once, so a wrong master password is
/// discovered before anything is shown rather than one connection at a time.
/// Each connection carries `connectionUrl`, `userName` and `userPassword`, in
/// whichever of the two formats the value was written — a file that has been
/// through both jdbgen releases holds a mixture, and this reads it.
///
/// The master password is not kept: the [`Decryptor`] built here is dropped
/// before this function returns, and wipes it.
///
/// # Errors
///
/// [`Error::WrongPassword`] when the password does not open the file, and
/// [`Error::Malformed`] when a value is damaged rather than merely unreadable —
/// no password produces that, so it is not worth retrying.
pub fn decrypt(cfg: &JdbgenConfig, master: &str) -> Result<Decrypted> {
    let decryptor = Decryptor::new(master);
    let mut config = cfg.clone();
    let mut legacy = false;

    for conn in &mut config.connections {
        for (field, value) in [
            ("connectionUrl", &mut conn.connection_url),
            ("userName", &mut conn.user_name),
            ("userPassword", &mut conn.user_password),
        ] {
            legacy |= Decryptor::is_legacy(value);
            *value = decryptor.decrypt(value, field)?;
        }
    }

    Ok(Decrypted { config, legacy })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_data_directory_is_jdbgens_and_not_rudbgens() {
        let Some(dir) = data_dir() else {
            return; // a machine with no home directory; nothing to assert
        };
        assert_eq!(dir.file_name().unwrap(), APP_NAME);
    }

    #[test]
    fn a_file_that_is_not_there_is_not_read() {
        let error = read(Path::new("/nonexistent/jdbgen/config.json")).unwrap_err();
        assert!(matches!(error, Error::Read { .. }), "{error:?}");
    }

    #[test]
    fn a_file_that_is_not_json_says_so_rather_than_importing_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_NAME);
        std::fs::write(&path, b"not json at all").unwrap();
        assert!(matches!(read(&path), Err(Error::Parse { .. })));
    }

    #[test]
    fn a_byte_order_mark_is_not_a_parse_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(CONFIG_NAME);
        std::fs::write(&path, b"\xef\xbb\xbf{\"applyAbbr\": true}").unwrap();
        assert!(read(&path).unwrap().apply_abbr);
    }

    #[test]
    fn a_configuration_with_no_connections_needs_no_password() {
        let cfg = JdbgenConfig::default();
        let opened = decrypt(&cfg, "").unwrap();
        assert!(opened.config.connections.is_empty());
        assert!(!opened.legacy);
    }
}
