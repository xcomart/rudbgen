//! The sample H2 database rudbgen ships, and the connection that opens it.
//!
//! jdbgen carried a small H2 database — `T_SAMPLE_ALBUM` and `T_SAMPLE_MUSIC`,
//! both commented — and put a connection to it into a fresh configuration, so
//! that a first run had something to generate from before the user had a
//! server to point at. rudbgen ships the same file (`assets/sample/`, installed
//! beside the executable) and does the same thing here.
//!
//! Two rules, both taken from jdbgen's `JDBGenConfig.sampleDatabaseUrlPath`:
//!
//! * the database is **copied** into the configuration directory before it is
//!   named in a URL. H2 opens an embedded database read-write and takes a lock
//!   on it, and the installed tree is exactly where that is not allowed —
//!   `C:\Program Files`, `/opt`, a signed `.app` bundle;
//! * a copy that is already there is **never** overwritten, so the rows a user
//!   has changed in the sample survive an update.
//!
//! Seeding runs only when `connections.json` does not exist at all. An empty
//! list in a file that *does* exist is a user who deleted the sample, and
//! handing it back on the next launch would be the same surprise
//! [`crate::builtin_templates::seed`] guards against with its flag.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rudbgen_core::{ConnectionProfile, ConnectionStore};

/// File name of the shipped database, as it is installed and as it is copied.
///
/// The `.mv.db` suffix is H2's own storage suffix; it is part of the file name
/// but never part of the URL.
const SAMPLE_DB_FILE: &str = "sample_h2.db.mv.db";

/// The database name an H2 URL carries — the file name without `.mv.db`, which
/// H2 appends itself.
const SAMPLE_DB_NAME: &str = "sample_h2.db";

/// Name of the directory the database is installed in, beside the executable.
const SAMPLE_DIR: &str = "sample";

/// What the connection list calls the seeded profile.
///
/// jdbgen's name for the same connection, and not translated, for the reason
/// [`crate::builtin_templates::Builtin::name`] gives: it is a name the user may
/// rename, and one that is written into `connections.json` the moment it is
/// saved — a label that changed with the interface language would leave a
/// profile carrying a name from whichever language it was created in.
const SAMPLE_CONNECTION_NAME: &str = "Sample H2 Embedded";

/// Id of the built-in driver the sample opens through.
///
/// `h2-embedded` is `no_auth`, which is why the profile carries no user name.
const SAMPLE_DRIVER_ID: &str = "h2-embedded";

/// The `sample/` directory of a checkout, for `cargo run` and `cargo test`.
///
/// The same compile-time fallback `rudbgen_jdbc::default_bridge_jar` ends on:
/// nothing is installed beside a test binary under `target/debug/deps`, and
/// without this a developer's first run would seed no sample at all.
const CHECKOUT_SAMPLE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sample");

/// The places the shipped database could be, in the order they are tried:
/// `<exe_dir>/sample` — the flat tree Windows and Linux install — then
/// `<exe_dir>/../Resources/sample`, which is the macOS bundle, whose executable
/// sits in `Contents/MacOS/`.
fn sample_dir_candidates(exe: &Path) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![dir.join(SAMPLE_DIR)];
    if let Some(above) = dir.parent() {
        candidates.push(above.join("Resources").join(SAMPLE_DIR));
    }
    candidates
}

/// The shipped database, or `None` when this build has none beside it.
///
/// A release without the sample is not an error: it only means the first run
/// starts with an empty connection list, which is what it did before.
pub fn locate() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|exe| sample_dir_candidates(&exe))
        .unwrap_or_default()
        .into_iter()
        .chain(std::iter::once(PathBuf::from(CHECKOUT_SAMPLE_DIR)))
        .map(|dir| dir.join(SAMPLE_DB_FILE))
        .find(|candidate| candidate.is_file())
}

/// The path an H2 URL carries for the copy under `config_dir`.
///
/// Without the `.mv.db` suffix, and with `/` separators — which H2 understands
/// on every platform, where a Windows `\` inside a JDBC URL does not survive
/// every parser it passes through. jdbgen spells the same two rules in
/// `JDBGenConfig.java:442`.
fn database_url_path(config_dir: &Path) -> String {
    config_dir
        .join(SAMPLE_DB_NAME)
        .to_string_lossy()
        .replace('\\', "/")
}

/// Puts the sample database and the connection that opens it in place, once.
///
/// `source` is the installed database, as [`locate`] found it; `None` — or a
/// path that is not a file — means this build ships none, and nothing is
/// seeded. Answers whether `connections_file` was written.
///
/// A copy that fails is logged and does not stop the profile from being
/// written: the connection then reports the missing file when it is used,
/// which is a great deal easier to understand than a connection list that is
/// silently empty. jdbgen behaves the same way.
///
/// # Errors
///
/// Fails when the configuration directory cannot be created or the connection
/// store cannot be written.
pub fn seed(config_dir: &Path, connections_file: &Path, source: Option<&Path>) -> Result<bool> {
    if connections_file.exists() {
        return Ok(false);
    }
    let Some(source) = source.filter(|path| path.is_file()) else {
        log::warn!("no sample database is installed with this build; none was seeded");
        return Ok(false);
    };

    std::fs::create_dir_all(config_dir)
        .with_context(|| format!("failed to create directory {}", config_dir.display()))?;

    let target = config_dir.join(SAMPLE_DB_FILE);
    if !target.exists() {
        match std::fs::copy(source, &target) {
            Ok(_) => log::info!("copied the sample database to {}", target.display()),
            Err(error) => log::warn!(
                "cannot copy the sample database {} to {}: {error}",
                source.display(),
                target.display()
            ),
        }
    }

    let mut store = ConnectionStore::default();
    // `new` and nothing else: every switch the profile carries is whatever the
    // `Default` impl answers, which is the one place they are decided.
    store.upsert(ConnectionProfile::new(
        SAMPLE_CONNECTION_NAME,
        SAMPLE_DRIVER_ID,
        format!("jdbc:h2:{}", database_url_path(config_dir)),
        // `h2-embedded` is `no_auth`: an embedded database has no login.
        "",
    ));
    store.save_to(connections_file)?;
    Ok(true)
}

/// Resolves the three paths [`seed`] needs and seeds, on a first run.
///
/// Every failure is logged rather than fatal, for the reason
/// [`crate::install_builtins`] gives: an application that could not write a
/// sample connection is still an application.
pub fn install() {
    let (Ok(config_dir), Ok(connections_file)) =
        (rudbgen_core::config_dir(), rudbgen_core::connections_file())
    else {
        log::error!("no configuration directory for the sample database");
        return;
    };
    match seed(&config_dir, &connections_file, locate().as_deref()) {
        Ok(true) => log::info!(
            "seeded the sample connection into {}",
            connections_file.display()
        ),
        Ok(false) => {}
        Err(error) => log::error!("could not seed the sample connection: {error:#}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A directory holding a file that stands in for the shipped database.
    fn install_tree(dir: &Path) -> PathBuf {
        let source = dir.join(SAMPLE_DB_FILE);
        std::fs::write(&source, b"shipped").expect("write source");
        source
    }

    #[test]
    fn a_first_run_copies_the_database_and_seeds_one_profile() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = install_tree(dir.path());
        let config = dir.path().join("config");
        let connections = config.join("connections.json");

        assert!(seed(&config, &connections, Some(&source)).expect("seed"));

        assert_eq!(
            std::fs::read(config.join(SAMPLE_DB_FILE)).expect("copy"),
            b"shipped"
        );
        let store = ConnectionStore::load_from(&connections).expect("load");
        assert_eq!(store.len(), 1);
        let profile = &store.connections()[0];
        assert_eq!(profile.name, SAMPLE_CONNECTION_NAME);
        assert_eq!(profile.driver_id, SAMPLE_DRIVER_ID);
        assert_eq!(profile.username, "");
        // The URL names the copy, without the storage suffix and with `/`.
        assert_eq!(
            profile.url,
            format!("jdbc:h2:{}", database_url_path(&config))
        );
        assert!(profile.url.ends_with(&format!("/{SAMPLE_DB_NAME}")));
        assert!(!profile.url.contains('\\'));
    }

    #[test]
    fn an_existing_connections_file_is_left_alone() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = install_tree(dir.path());
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).expect("config dir");
        let connections = config.join("connections.json");
        // A user who deleted the sample: the file exists and lists nothing.
        ConnectionStore::default()
            .save_to(&connections)
            .expect("empty store");

        assert!(!seed(&config, &connections, Some(&source)).expect("seed"));

        assert!(
            ConnectionStore::load_from(&connections)
                .expect("load")
                .is_empty()
        );
        // Nor is the database copied: there is no connection to open it.
        assert!(!config.join(SAMPLE_DB_FILE).exists());
    }

    #[test]
    fn a_build_without_the_database_seeds_nothing() {
        let dir = tempfile::tempdir().expect("tempdir");
        let config = dir.path().join("config");
        let connections = config.join("connections.json");

        assert!(!seed(&config, &connections, None).expect("no source"));
        assert!(!connections.exists());

        // A path that names nothing is the same case, not a panic.
        let missing = dir.path().join("nowhere").join(SAMPLE_DB_FILE);
        assert!(!seed(&config, &connections, Some(&missing)).expect("missing source"));
        assert!(!connections.exists());
    }

    #[test]
    fn an_existing_copy_of_the_database_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = install_tree(dir.path());
        let config = dir.path().join("config");
        std::fs::create_dir_all(&config).expect("config dir");
        // The user's own rows, from a previous release.
        std::fs::write(config.join(SAMPLE_DB_FILE), b"edited").expect("existing copy");
        let connections = config.join("connections.json");

        assert!(seed(&config, &connections, Some(&source)).expect("seed"));

        assert_eq!(
            std::fs::read(config.join(SAMPLE_DB_FILE)).expect("copy"),
            b"edited"
        );
        // The profile is still written: it is what opens the database that is
        // already there.
        assert_eq!(
            ConnectionStore::load_from(&connections)
                .expect("load")
                .len(),
            1
        );
    }

    #[test]
    fn the_macos_bundle_is_among_the_candidates() {
        let candidates =
            sample_dir_candidates(Path::new("/Apps/rudbgen.app/Contents/MacOS/rudbgen"));
        assert_eq!(
            candidates,
            vec![
                PathBuf::from("/Apps/rudbgen.app/Contents/MacOS/sample"),
                PathBuf::from("/Apps/rudbgen.app/Contents/Resources/sample"),
            ]
        );
    }

    #[test]
    fn the_checkout_ships_the_sample_database() {
        // The compile-time fallback is what makes `cargo run` from a checkout
        // seed anything at all; a move of `assets/sample` has to be noticed.
        assert!(
            Path::new(CHECKOUT_SAMPLE_DIR)
                .join(SAMPLE_DB_FILE)
                .is_file()
        );
    }
}
