# Contributing to rudbgen

Contributions to code generation, templates, database metadata, imports,
translations, tests and documentation are welcome. Start with the
[README](README.md), [architecture](docs/architecture.md) and
[template reference](docs/template-reference.md).

## Reporting a problem or proposing a change

Search the [issues](https://github.com/xcomart/rudbgen/issues) first. Include the
application version or commit, OS, database product and version, JDBC driver
coordinate or version, reproduction steps, and expected and actual results.
For generation problems, provide a minimal schema, template and expected output.
For jdbgen compatibility reports, also identify the original template's behavior.

Remove credentials and private schema or generated-code contents from examples,
logs and screenshots. Discuss changes to the template language, metadata model
or generation workflow before implementing them.

## Set up a development checkout

Use Git, stable Rust with `rustfmt` and `clippy` (CI and the release builds pin
Rust 1.98.0), a native compiler toolchain, and JDK 17 or newer. CI uses JDK 17.
Set `JAVA_HOME` to the JDK and put its `bin` directory on `PATH`. The Gradle wrapper is included; a separate Gradle install
is unnecessary. Windows needs MSVC and the Windows SDK; macOS needs Xcode
command line tools.

On Debian or Ubuntu, install the native libraries used by CI:

```sh
sudo apt-get update
sudo apt-get install -y --no-install-recommends \
  libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libfontconfig1-dev
```

From a checkout of `dev`, build the Java bridge before Rust:

```sh
rustup component add rustfmt clippy
cd bridge
./gradlew build --no-daemon
cd ..
cargo build --workspace --locked
cargo run -p rudbgen-app --locked
```

In Windows PowerShell, use `.\gradlew.bat build --no-daemon` inside `bridge`.
The first build needs network access for Cargo dependencies and Maven artifacts.
Running the app needs a graphical session.

Cargo checks for `bridge/build/libs/rudbgen-bridge.jar` but does not run Gradle.
Rebuild the bridge after Java changes, even if the old JAR still exists. Source
builds do not bundle Java: runtime discovery uses `RUDBGEN_JAVA_HOME`, then
`JAVA_HOME`, then system locations. Windows release builds also need the Windows
SDK shader compiler `fxc.exe`; `GPUI_FXC_PATH` can select it explicitly.

## Where changes belong

| Location | Responsibility |
| --- | --- |
| `crates/rudbgen-app` | Explorer, template editor integration, previews and generation UI |
| `crates/rudbgen-core` | Profiles, settings, paths and secret storage |
| `crates/rudbgen-template` | Template parsing, rendering and compatibility with jdbgen |
| `crates/rudbgen-meta` | Metadata model and the fields available to templates |
| `crates/rudbgen-gen` | Generation jobs, output paths, overwrite handling and cancellation |
| `crates/rudbgen-import` | Importing jdbgen configuration and encrypted data |
| `crates/rudbgen-jdbc` and `bridge` | Rust/Java protocol, JDBC sessions and metadata reads |
| `crates/rudbgen-ssh` | Compatibility facade over shared SSH transport |
| `templates` and `crates/rudbgen-app/locales` | Shipped templates and interface translations |

Shared widgets, editor and shell belong in
[rugpui](https://github.com/xcomart/rugpui/blob/dev/CONTRIBUTING.md); shared
connection validation, tunnel ownership and JVM startup belong in
[rudbkit](https://github.com/xcomart/rudbkit/blob/dev/CONTRIBUTING.md).
Those guides explain testing sibling checkouts through temporary Cargo patches.
Keep all references to each shared repository on one revision, including
rugpui's gpui patches. Commit intentional `Cargo.lock` changes with dependency
updates, and remove local path overrides before submitting.

## Validate a change

Run the baseline checks from the repository root:

```sh
cd bridge
./gradlew build --no-daemon
cd ..
cargo fmt --all --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
```

The Java suite and Rust integration tests use real in-memory H2 databases. The
H2 driver is normally found in the Gradle cache populated by the bridge build;
set `RUDBGEN_TEST_H2_JAR` to its absolute JAR path if discovery fails. An absent
bridge, driver or usable JVM is a setup problem, not a reason to skip these tests.

For template, metadata or output changes, use the focused suites while iterating:

```sh
cargo test -p rudbgen-template --locked
cargo test -p rudbgen-meta --locked
cargo test -p rudbgen-gen --locked
cargo test -p rudbgen-import --locked
```

The template and generation golden tests protect existing output. Investigate a
changed fixture before updating it, and describe any intentional compatibility
change in the [template reference](docs/template-reference.md).

### Database-specific checks

The container suite is opt-in: tests return without exercising a server when its
`RUDBGEN_TEST_*_URL` is unset. For a PostgreSQL change, for example:

```sh
cd bridge
./gradlew jar drivers --no-daemon
cd ..
docker compose -f docker/compose.yml up -d postgres
docker compose -f docker/compose.yml ps
# Wait for postgres to become healthy before continuing.
export RUDBGEN_TEST_PG_URL='jdbc:postgresql://127.0.0.1:55432/rudbgen'
cargo test -p rudbgen-jdbc --test containers --locked -- --nocapture
docker compose -f docker/compose.yml down
```

These examples use a POSIX shell; PowerShell environment variables use
`$env:NAME = 'value'`. See [docker/compose.yml](docker/compose.yml) for the MySQL,
MariaDB, SQL Server and Oracle URLs and credentials, and
[CI](.github/workflows/ci.yml) for the complete database matrix. Use disposable
test databases: the suites create and drop objects. rudbgen and rudbman use the
same local test ports, so run their Compose environments one at a time.

SQLite needs no Docker server. After downloading the drivers, set
`RUDBGEN_TEST_SQLITE_URL` to `jdbc:sqlite:/absolute/path/to/a/test.db` in a
disposable directory and run
`cargo test -p rudbgen-jdbc --test sqlite --locked -- --nocapture`.
Driver discovery can be overridden with `RUDBGEN_TEST_PG_JAR`,
`RUDBGEN_TEST_SQLITE_JAR` and the other `*_JAR` variables listed in CI.

Tests that access the OS keychain, Maven Central or a real SSH bastion are
ignored by default. Read each test's setup requirements before selecting it
with `--ignored`; ordinary SSH tests use loopback servers.

## Change guidelines

- Preserve jdbgen template compatibility, including filename rendering and
  documented quirks. Add regression coverage for parsing or rendering changes.
- Keep previews, dry runs and generated output consistent. Exercise overwrite,
  skip and cancel paths when changing generation, using a temporary directory.
- For bridge protocol changes, update the Java implementation, Rust decoder,
  tests on both sides and the [protocol documentation](bridge/README.md).
  Check whether the related rudbman bridge needs the same fix.
- Keep blocking database, file and network work off the UI thread. Keep secrets
  in the OS credential store and imported source configuration read-only.
- Add user-visible strings to the locale files, retaining matching keys and
  placeholders across all eight languages. Test keyboard navigation, long
  labels, light/dark themes and the affected user flow.
- Follow the surrounding style, use `cargo fmt --all`, and add focused tests for
  changed behavior. Update the relevant [documentation](docs/README.md).

## Submit your contribution

`dev` is the integration branch. Fork if necessary, branch from the latest
upstream `dev`, and open a focused pull request targeting `dev`. Maintainers
working directly in this workspace validate and commit on `dev`, then push to
`origin/dev`.

Use `type: summary` for new commit subjects, with a concise imperative summary.
Choose `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci` or `chore`
to describe the change, for example `fix: handle empty tables in template previews`
or `docs: add contribution guide`. Release and merge commits also use a prefix,
such as `chore: release <version>` or `chore: merge dev into main`. This convention
applies across all five repositories; do not rewrite existing history to rename
older commits.

Explain the problem, resulting behavior and validation in the pull request.
Include a minimal reproduction, screenshots for UI changes, database/driver
versions when relevant, and any checks you could not run. Exclude build output,
credentials, local Cargo patches and unrelated formatting from the diff.

CI runs on pull requests and on pushes to `dev` and `main`; validate locally
before a direct `dev` push all the same. Routine work ends at `origin/dev`. A release is a separate
maintainer action: merge the `dev` → `main` PR after required checks, fetch
`origin/main`, then tag its verified latest commit and publish. Do not move an
existing release tag.

Contributions are covered by the [MIT license](LICENSE). Preserve third-party
license notices, including those attached to inherited code and templates.
