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
//!
//! # The driver comes with it
//!
//! A connection is only half of what it takes to open the sample: a
//! [`DriverDef`](rudbgen_core::DriverDef) with no JAR on its class path fails
//! with `NoDriverJar` before it reaches JDBC at all. Every other driver rudbgen
//! knows about is downloaded from Maven Central on demand, but H2 is dual
//! licensed under MPL 2.0 and EPL 1.0 and may be redistributed, so the release
//! ships `h2-2.4.240.jar` beside the executable — jdbgen bundled it for the
//! same reason. [`seed_driver`] copies it into
//! [`drivers_dir`](rudbgen_core::drivers_dir) and puts the copy on the
//! `h2-embedded` class path, so the seeded connection opens on a first run with
//! no network at all.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use rudbgen_core::{ConnectionProfile, ConnectionStore, DriverStore};

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

/// Name of the directory the bundled driver JAR is installed in, beside the
/// executable — and, not by accident, the name of the directory inside the
/// configuration that the driver downloader writes to.
const DRIVERS_DIR: &str = "drivers";

/// File name of the bundled H2 driver JAR.
///
/// It has to be exactly what the `h2-embedded` coordinate resolves to: the
/// release workflow fetches that artefact from Maven Central under this name,
/// and [`rudbgen_core::drivers_dir`] is also where the in-app downloader would
/// put it, so a later download finds the file already there rather than
/// fetching a second copy. `the_bundled_jar_matches_the_builtin_coordinate`
/// is what keeps the two in step when the version moves.
const BUNDLED_DRIVER_JAR: &str = "h2-2.4.240.jar";

/// The `sample/` directory of a checkout, for `cargo run` and `cargo test`.
///
/// The same compile-time fallback `rudbgen_jdbc::default_bridge_jar` ends on:
/// nothing is installed beside a test binary under `target/debug/deps`, and
/// without this a developer's first run would seed no sample at all.
const CHECKOUT_SAMPLE_DIR: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/sample");

/// The places a shipped directory could be, in the order they are tried:
/// `<exe_dir>/<name>` — the flat tree Windows and Linux install — then
/// `<exe_dir>/../Resources/<name>`, which is the macOS bundle, whose executable
/// sits in `Contents/MacOS/`.
fn install_tree_candidates(exe: &Path, name: &str) -> Vec<PathBuf> {
    let Some(dir) = exe.parent() else {
        return Vec::new();
    };
    let mut candidates = vec![dir.join(name)];
    if let Some(above) = dir.parent() {
        candidates.push(above.join("Resources").join(name));
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
        .map(|exe| install_tree_candidates(&exe, SAMPLE_DIR))
        .unwrap_or_default()
        .into_iter()
        .chain(std::iter::once(PathBuf::from(CHECKOUT_SAMPLE_DIR)))
        .map(|dir| dir.join(SAMPLE_DB_FILE))
        .find(|candidate| candidate.is_file())
}

/// The bundled H2 driver JAR, or `None` when this build ships none.
///
/// No checkout fallback, unlike [`locate`]: a JAR is not in the repository and
/// is not going to be, so a developer running from `cargo run` installs H2 the
/// same way they install every other driver — the download button in the driver
/// manager, which lands the identical file in the identical place.
pub fn locate_driver_jar() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .map(|exe| install_tree_candidates(&exe, DRIVERS_DIR))
        .unwrap_or_default()
        .into_iter()
        .map(|dir| dir.join(BUNDLED_DRIVER_JAR))
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

/// Puts the bundled driver JAR where the sample connection can load it.
///
/// `source` is the JAR as [`locate_driver_jar`] found it; `None` — or a path
/// that is not a file — is a build without it, which is not an error: it is a
/// `cargo run` from a checkout, and the driver manager's download button is
/// what fills the gap. Answers whether `drivers_file` was written.
///
/// The copy is skipped when `drivers_dir` already holds a file of that name,
/// which is the case where the user downloaded the same artefact earlier: the
/// file name carries the version, so the two are the same bytes. A JAR already
/// on the driver's class path is not added twice, and the order of the ones
/// that are there is not disturbed — the same rule
/// [`crate::driver_manager`]'s `install_jars` follows, for the same reason:
/// with two JARs carrying one class, the first on the path wins.
///
/// Writing `drivers.json` freezes the built-in list as it stands today, since
/// [`DriverStore::load_from`] takes an existing file at its word. That is
/// already what the first download through the driver manager does, and the
/// alternative here is a seeded connection that cannot open.
///
/// # Errors
///
/// Fails when the drivers directory cannot be created or the driver store
/// cannot be read or written.
pub fn seed_driver(drivers_dir: &Path, drivers_file: &Path, source: Option<&Path>) -> Result<bool> {
    let Some(source) = source.filter(|path| path.is_file()) else {
        log::info!("no driver JAR is bundled with this build; none was installed");
        return Ok(false);
    };

    std::fs::create_dir_all(drivers_dir)
        .with_context(|| format!("failed to create directory {}", drivers_dir.display()))?;

    let target = drivers_dir.join(BUNDLED_DRIVER_JAR);
    if !target.exists() {
        match std::fs::copy(source, &target) {
            Ok(_) => log::info!("copied the bundled driver JAR to {}", target.display()),
            Err(error) => {
                // Unlike the database, a path that names nothing is worse than
                // no path at all: it would put the class loader on a file that
                // is not there and report a missing class.
                log::warn!(
                    "cannot copy the bundled driver JAR {} to {}: {error}",
                    source.display(),
                    target.display()
                );
                return Ok(false);
            }
        }
    }

    let mut store = DriverStore::load_from(drivers_file)?;
    let Some(mut driver) = store.get(SAMPLE_DRIVER_ID).cloned() else {
        log::warn!("no {SAMPLE_DRIVER_ID} driver to put the bundled JAR on");
        return Ok(false);
    };
    if driver.jars.contains(&target) {
        return Ok(false);
    }
    driver.jars.push(target);
    store.upsert(driver);
    store.save_to(drivers_file)?;
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
        Ok(true) => {
            log::info!(
                "seeded the sample connection into {}",
                connections_file.display()
            );
            // Only behind a connection that was actually seeded: the JAR is
            // installed for it, and a run that seeded nothing is either a
            // second run or one with no sample to open.
            install_driver_jar();
        }
        Ok(false) => {}
        Err(error) => log::error!("could not seed the sample connection: {error:#}"),
    }
}

/// Resolves the paths [`seed_driver`] needs and installs the bundled JAR.
fn install_driver_jar() {
    let (Ok(drivers_dir), Ok(drivers_file)) =
        (rudbgen_core::drivers_dir(), rudbgen_core::drivers_file())
    else {
        log::error!("no configuration directory for the bundled driver JAR");
        return;
    };
    match seed_driver(&drivers_dir, &drivers_file, locate_driver_jar().as_deref()) {
        Ok(true) => log::info!(
            "put the bundled driver JAR on the {SAMPLE_DRIVER_ID} class path in {}",
            drivers_file.display()
        ),
        Ok(false) => {}
        Err(error) => log::error!("could not install the bundled driver JAR: {error:#}"),
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
        let exe = Path::new("/Apps/rudbgen.app/Contents/MacOS/rudbgen");
        assert_eq!(
            install_tree_candidates(exe, SAMPLE_DIR),
            vec![
                PathBuf::from("/Apps/rudbgen.app/Contents/MacOS/sample"),
                PathBuf::from("/Apps/rudbgen.app/Contents/Resources/sample"),
            ]
        );
        // The driver JAR is packaged the same way, and looked for the same way.
        assert_eq!(
            install_tree_candidates(exe, DRIVERS_DIR),
            vec![
                PathBuf::from("/Apps/rudbgen.app/Contents/MacOS/drivers"),
                PathBuf::from("/Apps/rudbgen.app/Contents/Resources/drivers"),
            ]
        );
    }

    /// A directory holding a file that stands in for the bundled driver JAR.
    fn bundled_jar(dir: &Path) -> PathBuf {
        let source = dir.join(BUNDLED_DRIVER_JAR);
        std::fs::write(&source, b"PK-jar").expect("write jar");
        source
    }

    /// The `h2-embedded` definition as `drivers_file` now records it.
    fn h2_jars(drivers_file: &Path) -> Vec<PathBuf> {
        DriverStore::load_from(drivers_file)
            .expect("load")
            .get(SAMPLE_DRIVER_ID)
            .expect("built-in H2")
            .jars
            .clone()
    }

    #[test]
    fn a_bundled_jar_is_copied_and_put_on_the_h2_class_path() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = bundled_jar(dir.path());
        let config = dir.path().join("config");
        let drivers = config.join("drivers");
        let drivers_file = config.join("drivers.json");

        assert!(seed_driver(&drivers, &drivers_file, Some(&source)).expect("seed driver"));

        let copy = drivers.join(BUNDLED_DRIVER_JAR);
        assert_eq!(std::fs::read(&copy).expect("copy"), b"PK-jar");
        assert_eq!(h2_jars(&drivers_file), vec![copy]);

        // A second run adds nothing: the JAR is already on the class path, and
        // a duplicate would be a second entry pointing at the same file.
        assert!(!seed_driver(&drivers, &drivers_file, Some(&source)).expect("again"));
        assert_eq!(h2_jars(&drivers_file).len(), 1);
    }

    #[test]
    fn a_build_without_the_jar_seeds_the_profile_and_touches_no_driver() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = install_tree(dir.path());
        let config = dir.path().join("config");
        let connections = config.join("connections.json");
        let drivers = config.join("drivers");
        let drivers_file = config.join("drivers.json");

        // The sample connection is seeded whether or not a JAR came with it.
        assert!(seed(&config, &connections, Some(&source)).expect("seed"));
        assert!(!seed_driver(&drivers, &drivers_file, None).expect("no jar"));

        // Nothing was written, so the built-in list is still whatever this
        // build says it is rather than a copy frozen on disk.
        assert!(!drivers_file.exists());
        assert!(!drivers.exists());
        assert_eq!(
            ConnectionStore::load_from(&connections)
                .expect("load")
                .len(),
            1
        );

        // A path that names nothing is the same case, not a panic.
        let missing = dir.path().join("nowhere").join(BUNDLED_DRIVER_JAR);
        assert!(!seed_driver(&drivers, &drivers_file, Some(&missing)).expect("missing jar"));
        assert!(!drivers_file.exists());
    }

    #[test]
    fn a_jar_already_in_the_drivers_directory_is_never_overwritten() {
        let dir = tempfile::tempdir().expect("tempdir");
        let source = bundled_jar(dir.path());
        let config = dir.path().join("config");
        let drivers = config.join("drivers");
        std::fs::create_dir_all(&drivers).expect("drivers dir");
        // The same artefact, downloaded through the driver manager earlier.
        let copy = drivers.join(BUNDLED_DRIVER_JAR);
        std::fs::write(&copy, b"downloaded").expect("existing jar");
        let drivers_file = config.join("drivers.json");

        assert!(seed_driver(&drivers, &drivers_file, Some(&source)).expect("seed driver"));

        assert_eq!(std::fs::read(&copy).expect("copy"), b"downloaded");
        // The class path still has to name it: the store is what is missing
        // here, not the file.
        assert_eq!(h2_jars(&drivers_file), vec![copy]);
    }

    #[test]
    fn the_bundled_jar_matches_the_builtin_coordinate() {
        // The release workflow fetches the artefact this coordinate names and
        // installs it under this file name; a version bump in `profile.rs` that
        // left either behind would ship a JAR nothing looks for.
        let maven = DriverStore::default()
            .get(SAMPLE_DRIVER_ID)
            .expect("built-in H2")
            .maven
            .clone()
            .expect("H2 has a coordinate");
        let coordinate = crate::maven::Coordinate::parse(&maven).expect("parses");
        assert_eq!(coordinate.file_name(), BUNDLED_DRIVER_JAR);
        assert_eq!(
            coordinate.jar_url(),
            format!(
                "https://repo1.maven.org/maven2/com/h2database/h2/{}/{BUNDLED_DRIVER_JAR}",
                coordinate.version
            )
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
