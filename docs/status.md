# Progress and handoff

This document exists so that another session — or another person — can pick
the work up. The design and the contracts all live in
[architecture.md](architecture.md); what is kept here is only **how far the
work has come, what is left, and how work is done in this repository**. It is
updated whenever a milestone ends.

Last updated: 2026-08-23 (M0, the workspace foundation and the app shell).

## Where things stand

| Milestone | State | What went in |
|---|---|---|
| M0 | done | The workspace foundation and the application shell, described below. The window opens on the welcome screen; the settings dialog, the theme editor, the about box and the update check all work end to end |
| M1 | not started | `rudbgen-template`: the ported jdbgen tests, the engine, diagnostics, byte-identical fixtures |
| M2 | not started | The trimmed bridge (D3) + `rudbgen-jdbc`, the merged stock driver store, Maven download, the connection dialog with Test, SSH wired up, the explorer tree, the inspector |
| M3 | not started | The Generate tab, template sets, the generation job (cancel, policy, summary), the status bar, preview and dry run |
| M4 | not started | Template tabs: the editor with its highlighter, the variable palette, live preview, diagnostics |
| M5 | not started | Custom queries with Test, the abbreviation rules dialog with D10 semantics, the jdbgen import wizard |
| M6 | not started | Packaging (jlink runtime, three platforms), the release workflow, `docs/template-reference.md`, `docs/ui-guide.md`, README |

### What is in the tree today

| Piece | Notes |
|---|---|
| `vendor/gpui{,_linux,_macos,_windows}` | Copied from rudbman **byte-identical**, `RULOGMAN PATCH` markers and all (D2). `diff -r` against rudbman's four directories is empty and must stay that way: three projects — rulogman, rudbman, rudbgen — share these trees so a fix moves between them as a plain `diff`. Nothing in them is rudbgen-specific, and nothing in them may become so |
| `Cargo.toml` | The workspace, the `[patch."https://github.com/zed-industries/zed"]` table pinning gpui to one revision of Zed's monorepo, `[workspace.dependencies]` for what the copied crates use, and rudbman's profile tables. Every dependency still carries its "why" comment, rewritten where the reason differs. `rudbgen-app` pulls in `gpui_platform`, so all four patches are live |
| `crates/rudbgen-ui` | rudbman's widget kit, whole and unchanged apart from the name: 17 modules, the theme and editor-theme layers, six chrome themes and six editor themes. `actions!(rudbman_input, …)` is now `rudbgen_input`. The grid tokens stay — the table inspector uses the grid (§2.1). No user-facing strings live here; the app layer owns every one of them |
| `crates/rudbgen-core` | rudbman's core with the fields rudbgen does not have removed and the ones it needs added — see the next table |
| `crates/rudbgen-ssh` | rudbman's tunnel crate, whole: russh, a bastion without a PTY, loopback binds, the trusted-host-key check against `known_hosts`. Tunnels are already wired into the connection dialog rudbgen inherits, so removing them would cost more than keeping them (§2.1) |
| `crates/rudbgen-app` | The binary, `rudbgen`. rudbman's bootstrap sequence, `actions!`, `bind_shortcuts`, menus and window-chrome helpers; its `app_settings`, `caption`, `icons`, `about_dialog`, `theme_editor`, `settings_dialog`, `update` and `update_dialog` with the names, URLs and settings that changed; `i18n.rs` and eight locale files trimmed to rudbgen's keys; a `Workspace` written from scratch around §4.2's sketch. See the third table below for what diverged |
| `assets/` | `icon.svg` — the master mark, a sheet of generated source with `</>` on it, rudbman's two colours swapped — with `icon-128.png`, `icon-256.png`, `icon.ico` and `icon.icns` rendered from it by `render.py`. `assets/drivers/` holds jdbgen's eleven driver icons, unused until M2 |
| `.github/workflows/ci.yml` | rudbman's `check` job — three platforms, `cargo fmt --check` once, `clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked`. The Gradle/JDK steps and the opt-in `containers` job are **deliberately absent until M2**; the file says why |

### How `rudbgen-core` differs from `rudbman-core`

The crate is a copy that has already diverged (D1). Everything below is a
deliberate change, not a port artefact:

- **`paths`** — the app name is `rudbgen`, so the config directory is
  `~/.config/rudbgen` and its platform equivalents. `snippets_dir` and
  `erd_layouts_dir` are gone (rudbgen has neither); `template_sets_file`,
  `abbreviations_file` and `templates_dir` are new. The file list is §5's,
  exactly.
- **`secrets`** — the keychain service namespace is `rudbgen`. The two slots,
  `Connection` and `Tunnel`, are unchanged: rudbgen has the same two secrets
  to keep apart. The tests that touch a real keychain stay `#[ignore]`d.
- **`settings`** — `fetch_batch_rows`, `query_timeout_s`,
  `confirm_writes_default` and `erd_logical_names` are gone: nothing here
  streams rows or draws a diagram. `overwrite_policy` is new
  (`overwrite`/`skip`/`ask`, default `ask` — D11). It reads leniently: a
  missing key, a `null`, or a fourth word somebody invented all load as `ask`
  rather than failing the file.
- **`profile`** — `DriverDef`, `DriverStore`, `builtins()` (the same seven
  products), `ConnectionProfile`, `ConnectionStore` and the `Redacted` /
  `MaskedUrl` / `MaskedProps` `Debug` impls are carried over unchanged.
  `ConnectionProfile` gains `generation: GenerationProfile` — templates,
  output directory, author and custom variables, per connection (§4.4, §5) —
  which a `connections.json` written without it loads as the default. The
  four per-driver custom queries (D9) are **not** in `DriverDef` yet; the two
  comment queries rudbman already had are. That is M2 work.
- **`template_sets`** (new) — `TemplateSet` / `TemplateSetStore` over
  `template-sets.json`, jdbgen's presets.
- **`abbreviations`** (new) — `AbbreviationRule` / `AbbreviationStore` over
  `abbreviations.json`. It stores the rules and **does not apply them**:
  matching, and with it D10's case-insensitive word rules, belongs to the
  template engine, which is the only place that knows how a name splits into
  words.
- **`known_hosts`** — unchanged; it stays with SSH.

Both new stores follow the discipline the rest of the crate already had:
read-lenient (a missing file is a first run, a UTF-8 BOM is stripped, missing
keys default, unknown top-level keys are round-tripped through `extra`) and
write-atomic (`paths::write_atomic`).

### How `rudbgen-app` differs from `rudbman-app`

- **`main.rs` is new.** rudbman's 8,400-line `Workspace` is not copied (§2.1).
  What came over is the bootstrap sequence — `env_logger` → `update::apply_pending`
  → `application().with_assets(Icons)` → keychain → settings → `i18n::apply` →
  `rudbgen_ui::init` → shortcuts → menus → themes → the window-closed save →
  `open_window` — and every window-chrome helper: `draws_own_titlebar`,
  `titlebar_gestures`, `client_tiling`, `render_resize_edges`,
  `window_appearance`, `window_bounds`, `record_window_geometry`, the caption
  theme and the two `WindowControls` strips. The shell built on top of them is
  §4.2's: a title bar (mark, connection selector, ⚙, ?), the welcome screen as
  the whole body, and a status bar. `actions!(rudbgen, [Quit, NewConnection,
  OpenSettings, ShowAbout, CheckUpdates, DismissDialog, ToggleExplorer])`.
- **The welcome screen (§4.3)** lists what `connections.json` holds — name and
  driver, display only — and offers *New connection…*, *Import from jdbgen…*
  and *Open a template…*. All three are drawn **disabled with a tooltip that
  says they are not built yet**: they belong to M2 and M5, and a button that
  looks live and does nothing is worse than one that says so. `NewConnection`
  and `ToggleExplorer` are bound and dispatched, and their handlers log and
  return.
- **The explorer and the inspector are out of the frame**, not empty: with no
  connection there is nothing to put in either (§4.3).
- **`settings_dialog`** loses the rows for the fields `rudbgen-core` dropped
  (rows per batch, statement timeout, confirm writes) and its "Database"
  section becomes **"Generation"**, holding the overwrite policy as a
  `Segmented` of overwrite / skip / ask (D11). Everything else — the two theme
  pickers with their management rows, the live preview, the colour editor, the
  language switch — is unchanged.
- **`context_menu.rs`** keeps the `MenuRow` → `MenuEntry` skeleton and its test
  helpers; the grid copy/heading helpers went with the grid. **`pane_tree.rs`**
  keeps the whole generic `PaneTree<T>` and its mechanics tests; `PaneItem`
  becomes rudbgen's three tabs — `Generate`, `Template { file, title, dirty }`,
  `Preview` — carrying identities rather than views, since the views arrive in
  M3/M4. Both modules are `#[allow(dead_code)]` at their declaration, with the
  reason written there.
- **`update.rs`** carries a **fresh `ProductCode` GUID**: a product code names a
  product, and sharing rudbman's would have winget treat an installed rudbman
  as an installed rudbgen. `must_defer()` answers `false` until M2 — the
  question it asks is "is a JVM loaded into this process", and nothing here
  loads one yet. The test that checked the GUID against
  `packaging/windows/rudbgen.iss` comes back with `packaging/` in M6.
- **`locales/*.yml`** are rudbman's, trimmed to 135 keys per language across
  ten namespaces (`_version`, `language`, `common`, `menu`, `titlebar`,
  `welcome`, `statusbar`, `about`, `settings`, `update`). Every rudbman-only
  namespace — query, explorer, grid, erd, builder, extract, transfer, backup,
  data, struct, connect, driver, detail, tab, empty, context — is gone. The new
  keys (`welcome.*`, `titlebar.*`, `settings.overwrite_*`,
  `settings.section.generation`, `statusbar.no_selection`, and the two
  rewritten taglines) are translated into all eight languages. `i18n.rs`'s five
  tests are unchanged apart from the `PROBES` list, which now names one key per
  surviving namespace.

## How work is done here

- Repository: <https://github.com/xcomart/rudbgen> (MIT, same author as
  rudbman and rulogman).
- **The branch flow is rudbman's: work on `dev`, `main` takes PR merge commits
  only.** Each milestone is one PR onto `main`, and this document is updated
  when the milestone ends.
- Every library crate carries `#![warn(missing_docs)]`; `lib.rs` is a module
  list and its re-exports and nothing else. Pure modules and gpui modules stay
  in separate files. Test names are sentences.
- The widget crates carry no user-facing strings — the app layer owns every
  one of them, because it is the layer that has the translations.

### Building and testing

```sh
cargo build --workspace
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

The first build is long: gpui and its dependency tree come from a git
revision and are compiled from source. On Linux the native libraries gpui
links against have to be present — on Debian/Ubuntu that is
`libxkbcommon-dev libxkbcommon-x11-dev libwayland-dev libfontconfig1-dev`;
CI installs exactly those.

No JDK is needed yet. It becomes a requirement in M2, when the bridge and
`rudbgen-jdbc` arrive: `rudbgen-jdbc`'s build script will refuse to compile
without the bridge JAR, so Gradle runs before cargo from then on.

Tests that touch the real OS keychain are `#[ignore]`d, so a headless machine
runs the suite green. Run them by hand with:

```sh
cargo test -p rudbgen-core -- --ignored
```

Test count as of this entry: 370 passing plus 5 ignored, across
`rudbgen-core` (89), `rudbgen-ssh` (25, of which the in-process russh server
suite is the bulk), `rudbgen-ui` (121) and `rudbgen-app` (133), plus the doc
tests. `rudbgen-template`'s own suite is counted in its milestone's entry.

## What is next

M0 is done on Linux: `cargo build --workspace`, `cargo test --workspace`,
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all --check` are all green, and `cargo run -p rudbgen-app` opens
the window on the welcome screen — see `docs/screenshots/welcome.png`. What
closes it formally is the same suite green on CI's other two platforms, which
only CI can say.

Two things the milestone deliberately left for later, written down so they are
not mistaken for oversights:

- The welcome screen's three buttons and the `NewConnection` / `ToggleExplorer`
  actions are inert, by the reasoning above. They come alive in M2 and M5.
- `packaging/` and the release workflow are M6, so the Windows installer's
  `AppId`, the winget manifests' `ProductCode` and `update::ARP_KEY` are only
  one corner of a triangle today; the test that checks the other two comes back
  with the directory.

M1 is next: `rudbgen-template`.
