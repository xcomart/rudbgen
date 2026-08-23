# rudbgen architecture

A database code generator: point it at a database, pick tables, pick
templates, and it writes one file per table per template. It is the Rust +
gpui successor of [jdbgen](https://github.com/xcomart/jdbgen) (Java/Swing),
built on the foundation of [rudbman](https://github.com/xcomart/rudbman) —
the same JDBC-over-JNI bridge, the same widget kit, the same settings, theme,
i18n and self-update machinery.

This document fixes the boundaries before any code is written, in the way
rudbman's `docs/architecture.md` does. What it does not decide, the
implementer may decide; what it does decide never changes without changing
this document.

---

## 1. Decision summary

| # | Decision | Why |
|---|---|---|
| D1 | **Copy** rudbman's crates into this repository (not a path or git dependency) and let them evolve on their own | The same rule rudbman applied to logman. A path dependency ties two release cycles together and a git dependency cannot carry the `vendor/` gpui patches. Copies diverge where rudbgen needs them to (driver editor, explorer) and stay `diff`-able where they do not |
| D2 | **Vendor rudbman's patched gpui** byte-identical, `RULOGMAN PATCH` markers and all | Same patches, same reasons: live title-bar switch, X11 re-entrancy panic, X11 CSD transparency, KWin blur. Kept identical so a fix moves between the three projects as a plain diff |
| D3 | The **JDBC bridge is inherited as a copy** (`bridge/`, package `comart.rudbgen.bridge`) and **trimmed**: `job/` (Backup, Transfer, Extract) and the LOB path go; `meta/`, `codec/`, `Session`, `Loaders`, `DriverProbe` stay | rudbgen never ferries row data. What it needs — open a session, `DESCRIBE` everything, run a handful of metadata SQL — is the bridge's M1/M2 surface. Trimming removes ~40% of the Java and every job-frame op |
| D4 | The **template engine is ported to Rust** as the pure crate `rudbgen-template`, byte-compatible with jdbgen's assets | rudbman kept it in Java because rows flow through the JVM there. Here nothing flows: the model is metadata already in Rust, and a Rust engine is what live preview, syntax highlighting and unknown-field diagnostics in the editor need without a JNI round trip per keystroke. jdbgen's three engine test classes (~1,350 lines) are ported first and are the compatibility canary. Padding width keeps jdbgen's EUC-KR byte count via `encoding_rs` |
| D5 | **No master password.** Secrets go to the OS keychain through `rudbgen-core::secrets`; everything else is plain JSON | The master password gates the whole app to protect three fields. jdbgen's own docs list it as the first thing users trip over. A one-time **import from jdbgen** asks for the master password once, decrypts with jdbgen's exact scheme (AES-256-GCM/PBKDF2 v2 and the legacy CBC form), and moves the secrets into the keychain |
| D6 | **One window, no startup modal.** With no connection open, a welcome screen lists the saved connections; the update check runs in the background and surfaces as a banner | jdbgen blocks three times before the first click (update check up to 60 s, password, mandatory Connection Manager whose Cancel quits). Every one of those becomes a non-blocking state of the main window |
| D7 | **Edits are transactional.** Every dialog edits a draft; `Save` commits it to the store and disk in one step, `Cancel` discards it. The store is never touched during editing | jdbgen's `+`/clone/import leak into the saved configuration when the dialog is cancelled — a documented known issue. A draft model makes the bug impossible rather than fixed |
| D8 | **Metadata is richer than jdbgen's**: primary keys, foreign keys (both directions), indexes and unique constraints are read through the bridge's `DESCRIBE` and exposed to templates as new fields. jdbgen's fields keep their names and meaning | A relational code generator that cannot see relations cannot generate joins, navigation properties or fixture order. The bridge already answers all of it |
| D9 | **Custom queries stay** (the four per-driver SQL overrides) and gain a **Test** button that runs the SQL against a chosen connection and checks the result labels | They are the only way some drivers yield a table list at all. Their failure mode today — one missing label fails the whole run, silently — is what the test button is for |
| D10 | **Abbreviation word rules match case-insensitively.** jdbgen's word rules only ever matched lower-case segments, which made them useless against upper-case identifiers | This is the one deliberate behavioural break from jdbgen. It is called out in the import wizard and in the template reference |
| D11 | **Generation is a cancellable job with an overwrite policy** (overwrite / skip existing / ask) and a **result summary** listing every file written, skipped or failed. A **dry run** renders to memory only | jdbgen's progress window has no cancel, no close, and a failed run leaves half the files on disk with no list of which |
| D12 | **Templates are first-class documents**: an in-app editor with template-language highlighting, a variable palette, diagnostics for unknown fields, and a live preview against a selected table | This is where the port earns its keep. jdbgen stores a file path and nothing else; a typo in a field name is a silent empty string found in the output |

---

## 2. Inherited assets

### 2.1 rudbman (`~/Work/rudbman`) — the foundation

| Source | Destination | Notes |
|---|---|---|
| `vendor/gpui{,_linux,_macos,_windows}` | same | Byte-identical. `RULOGMAN PATCH` markers untouched |
| `Cargo.toml` profile tables, `[patch]` table, `.gitattributes` | same | Dependency comments rewritten where the reason differs |
| `crates/rudbman-ui/` (17 modules) | `crates/rudbgen-ui/` | Whole. `actions!(rudbman_input, …)` → `rudbgen_input`. Grid tokens stay (the table inspector uses the grid) |
| `crates/rudbman-core/` | `crates/rudbgen-core/` | `paths`, `secrets`, `settings` skeleton, `profile`'s `DriverDef`/`DriverStore`/`builtins()` and the `Redacted`/`MaskedUrl`/`MaskedProps` Debug impls. `ConnectionProfile` gains generation fields (§5). `known_hosts` stays with SSH |
| `crates/rudbman-ssh/` | `crates/rudbgen-ssh/` | Whole. Tunnels are already wired into the connection dialog; removing them costs more than keeping them |
| `crates/rudbman-jdbc/` | `crates/rudbgen-jdbc/` | `jvm`, `session`, `protocol`, `codec`, `error`, `response`; `spec` loses `ExtractSpec`/`TransferSpec`/`BackupSpec`; `Op` loses `Job*` and `LobRead` |
| `bridge/` | `bridge/` | Per D3. Package renamed, `template/` dropped (it moves to Rust, D4) |
| `crates/rudbman-grid/` | `crates/rudbgen-grid/` | Whole. Used by the table inspector and the custom-query test result |
| `crates/rudbman-editor/` | `crates/rudbgen-editor/` | The `Highlighter` is made pluggable (a trait over a token stream); the SQL highlighter's dependency on `rudbman-sql` is replaced by a small lexer of its own for custom-query SQL, and a template-language highlighter is added (§8) |
| `crates/rudbman-app/src/`: `i18n.rs` + `locales/*.yml`, `app_settings.rs`, `caption.rs`, `icons.rs`, `about_dialog.rs`, `context_menu.rs`, `pane_tree.rs`, `theme_editor.rs`, `settings_dialog.rs`, `update.rs`, `update_dialog.rs`, `maven.rs`, `connection.rs`, `connection_dialog.rs`, `driver_manager.rs`, `explorer.rs` | `crates/rudbgen-app/src/` | Keys, strings, URLs and asset names replaced. The `Workspace` shell in `main.rs` is **not** copied — its bootstrap sequence, `actions!`, `bind_shortcuts`, menus and window-chrome helpers are |
| `.github/workflows/`, `packaging/`, `assets/render.py`, `docker/compose.yml` | same | Names and targets replaced. Container tests are opt-in exactly as in rudbman |

Not brought over: `rudbman-sql` (DDL/DML planners, dialects — nothing here writes SQL to a server), `rudbman-erd`, the data/struct panes, backup/transfer/extract dialogs, `row_apply`, `data_edit`, `struct_edit`, `query*`.

### 2.2 jdbgen (`~/Work/jdbgen`) — the specification

- `template/TemplateManager.java` and its three test classes — the **behavioural spec** of `rudbgen-template` (§7). The code, not `docs/template-reference.md`, is authoritative where they disagree (they do: `${no}` counts rendered elements only).
- `utils/StrUtils.java` — the case conversions (`toCamelCase` …) and the v2/legacy decryption, needed by the import wizard.
- `types/JDBGenConfig.java`, `JDBConnection`, `JDBDriver`, `JDBTemplate`, `JDBPreset`, `JDBAbbr` — the shape of `config.json` the import wizard reads.
- `types/db/DBMeta.java` — the custom-query contract: which labels the four SQLs must return (`TABLE_CAT`, `TABLE_SCHEM`, `TABLE_NAME`, `TABLE_TYPE`, `REMARKS`; `COLUMN_NAME`, `DATA_TYPE`, `TYPE_NAME`, `COLUMN_SIZE`, `NULLABLE`, `REMARKS`, `COLUMN_DEF`, `IS_KEY`; and the positional name/comment pairs).
- `types/db/SqlTypes.java` — the `java.sql.Types` → `jdbcType`/`javaType` table, kept verbatim for template compatibility (including its oddities: `TIMESTAMP`→`String`, `DECIMAL`→`Integer`). A second, saner mapping is offered as new fields, never by changing these.
- `templates/*.{java,xml,php}` — shipped as the built-in template set; `src/main/resources/icons/*.png` — driver icons; `defaultConfig.json` — the ten stock drivers (merged with rudbman's seven: same product → one entry, jdbgen's custom queries and props carried over).

---

## 3. Repository layout

```
rudbgen/
├── Cargo.toml
├── docs/architecture.md        this document
├── docs/status.md              progress and handoff
├── docs/template-reference.md  the template language (ported from jdbgen, corrected)
├── vendor/gpui{,_linux,_macos,_windows}/
├── bridge/                     Gradle → rudbgen-bridge.jar
├── assets/                     icon, driver icons
├── templates/                  built-in template set (copied to the config dir on first run)
├── packaging/
└── crates/
    ├── rudbgen-core/           settings, profiles, drivers, secrets, paths, known_hosts
    ├── rudbgen-ui/             gpui widget kit + themes
    ├── rudbgen-ssh/            SSH local port forwarding
    ├── rudbgen-jdbc/           JNI: JVM bootstrap, session worker, DESCRIBE, EXECUTE
    ├── rudbgen-template/       the template engine — pure, no gpui, no JNI
    ├── rudbgen-meta/           the table model: DESCRIBE + custom queries → template Model
    ├── rudbgen-gen/            the generation job: plan → render → write (§9), no gpui
    ├── rudbgen-editor/         code editor widget with pluggable highlighting
    ├── rudbgen-grid/           virtualized grid widget
    └── rudbgen-app/            the binary
```

Dependency direction (no cycles, no back-edges):

```
rudbgen-app
 ├─→ rudbgen-grid ───┐
 ├─→ rudbgen-editor ─┼─→ rudbgen-ui ─→ gpui
 ├─→ rudbgen-template          (pure: encoding_rs, regex, chrono only)
 ├─→ rudbgen-meta ─→ rudbgen-jdbc, rudbgen-template, rudbgen-core
 ├─→ rudbgen-gen ─→ rudbgen-meta, rudbgen-template, rudbgen-core
 ├─→ rudbgen-jdbc ─→ rudbgen-core
 ├─→ rudbgen-ssh  ─→ rudbgen-core
 └─→ rudbgen-core
```

`rudbgen-template` knows nothing of JDBC: it renders a `Model` trait (§7.3) that the app implements over metadata. This is what makes it testable with the ported jdbgen fixtures and reusable for the output-file-name templates.

---

## 4. The UI

### 4.1 Principles

1. **Nothing blocks the first click.** Startup is: settings → theme → window. Update check, keychain and JVM start happen after the window is on screen.
2. **One window.** Connections are not a modal gate; they are the first thing the welcome screen offers. Dialogs are entities rendered over the workspace (rudbman's `modal()`), never nested.
3. **Selection lives in the explorer, options live in the work area, the verdict lives in the status bar.** The three never overlap.
4. **Every edit is a draft** (D7). A dialog's `Save` is the only write.
5. **What the generator will do is visible before it does it**: the status bar counts tables × templates, the preview renders one of them, the dry run renders all of them.

### 4.2 Window structure

```
┌ [⌂ rudbgen]  [● Sample H2 ▾]                      [⚙] [?]  ─ □ ✕ ┐
├──────────────┬─────────────────────────────────┬─────────────────┤
│ EXPLORER     │ WORK AREA (tabs)                │ INSPECTOR       │
│ 🔍 filter    │ ┌ Generate ┐┌ java_model ┐┌ … ┐ │ T_SAMPLE_ALBUM  │
│ ☐ views      │ │                                │ Columns │Keys│FK│
│ ▾ PUBLIC     │ │  Template set  [Java + MyBatis▾]│ # name  type  K │
│   ☑ T_ALBUM  │ │  ☑ Java Model  ${…}Model.java  │ 1 ALBUM_ID INT ● │
│   ☑ T_ARTIST │ │  ☑ Mapper XML  ${…}-mapper.xml │ 2 NAME  VARCHAR  │
│   ☐ T_TRACK  │ │  ☐ PHP CI      …               │ …               │
│ ▸ INFO_SCHEMA│ │                                │                 │
│              │ │  Output   ~/out/src      [...]  │                 │
│              │ │  Author   comart                │                 │
│              │ │  Variables  package=com.abc.x   │                 │
│              │ │  ☑ apply abbreviations  [rules…]│                 │
│              │ └────────────────────────────────┘                 │
├──────────────┴─────────────────────────────────┴─────────────────┤
│ 2 tables · 2 templates → 4 files into ~/out/src   [Preview] [Dry run] [Generate ▶] │
└──────────────────────────────────────────────────────────────────┘
```

- **Title bar**: the connection selector (a `TabBar`-less `Select` with the status dot: connecting / connected / error), settings and help. Switching the connection swaps the explorer and the Generate tab's options (they are per connection), never the open template tabs.
- **Explorer** (left, collapsible): catalog → schema → table tree with a filter box (contains, case-insensitive — jdbgen's `filterTables` rule), a *views* toggle, and a **checkbox per table row**. Selection is the set of ticked rows, which survives filtering and schema switching. Context menu: select all visible, clear, invert, open in inspector.
- **Work area** (center): a tab strip. The **Generate** tab is permanent. Template tabs open from the template list (§4.4). A **Preview** tab opens from the status bar.
- **Inspector** (right, collapsible): the table under the cursor or the last ticked one — columns (ordinal, name, type, nullable, default, comment, PK marker), keys, foreign keys in and out, indexes. This replaces jdbgen's Table View modal. When a template tab is active it shows the **variable palette** instead (§4.5).
- **Status bar**: the arithmetic of the run and the three actions. `Generate` is disabled with a reason (`no connection`, `no tables ticked`, `no templates ticked`) in a tooltip rather than an error box after the click.

### 4.3 Welcome screen

Shown in the work area when no connection is open: saved connections as a list (icon, name, driver, last used), **New connection**, **Import from jdbgen…** (when `~/.config/jdbgen/config.json` or its platform equivalent exists), and **Open a template** (templates can be edited without a database). The explorer and inspector stay out of the frame until a connection opens.

### 4.4 Generate tab

- **Template set** selector (jdbgen's presets, renamed): `Built-in: Java + MyBatis`, user sets, and `Custom` once the list diverges. `Save as set…` stores the current list.
- **Template list**: one row per template — tick, name, output name template, and an **Edit** glyph that opens the template tab. Inline add/remove; the file picker starts in `<config>/templates`. The ticks are stored per connection as in jdbgen.
- **Options**: output directory (with the `...` chooser), author, custom variables (key/value table with the trailing empty row rule), the abbreviation toggle with a link to the rules editor.
- All of it is the connection's generation profile, saved when it changes (debounced) — there is no second place to edit it.

### 4.5 Template tab

The editor (`rudbgen-editor`) with the template highlighter, split with a **live preview** rendered against the first ticked table (selectable from a dropdown in the preview header). The inspector becomes the **variable palette**: every field the current model offers (table fields, column fields inside a `for`, custom variables, statements, decorators) — clicking inserts `${…}` at the caret. Diagnostics are gutter marks with a message: parse errors from the engine, and *unknown field* warnings computed against the preview model. `Ctrl+S` writes the file; the tab shows a dirty marker.

### 4.6 Dialogs

| Dialog | From | Notes |
|---|---|---|
| Connection | welcome, title bar | rudbman's connection dialog: name, driver, URL with placeholder substitution, credentials (keychain), properties, keep-alive, SSH tunnel, **Test**. The driver editor replaces the form body (one tab ring, never nested) |
| Driver editor | connection dialog | rudbman's driver manager plus a **Custom queries** section: four `[enable] [SQL editor] [Test]` rows. Test runs the SQL on a chosen open connection and reports the labels it found against the labels required |
| Abbreviation rules | Generate tab | The four-column table (apply, whole name, abbreviation, replacement), the trailing empty row, duplicate rejection, and a table-name picker for whole-name rows |
| Settings | title bar | Theme, editor theme, language (live, no restart), fonts, JVM heap, update channel |
| Generation progress | Generate | Bar + log + **Cancel**. On completion a **result summary**: files written / skipped / failed with paths, `Open output directory`, and for conflicts under the *ask* policy a per-file choice |
| Import from jdbgen | welcome | Asks for the master password once, shows what it found (connections, drivers, sets, rules) with checkboxes, then writes the stores and the keychain. Reports the D10 behaviour change |
| About | title bar | rudbman's |

---

## 5. Configuration

`directories::ProjectDirs::from("", "", "rudbgen")` — the same layout rule as rudbman, read-leniently / write-atomically:

```
settings.json          AppSettings (theme, language, fonts, jvm, window, overwrite policy default)
connections.json       ConnectionStore — profiles with a GenerationProfile each
drivers.json           DriverStore — incl. custom queries
template-sets.json     TemplateSetStore (jdbgen's presets)
abbreviations.json     AbbreviationStore
templates/             template files (built-ins copied here on first run, never overwritten)
drivers/               downloaded JDBC JARs
themes/, editor-themes/
known_hosts
```

`GenerationProfile { templates: Vec<TemplateRef { name, file, out_template, selected }>, output_dir, author, custom_vars: IndexMap }` lives inside `ConnectionProfile`. Paths are stored relative to the config dir when below it, absolute otherwise (jdbgen's rule; the install-dir fallback goes — a Rust binary has no `templates/` beside it).

Secrets: `SecretSlot::Connection(uuid)` for the database password, `SecretSlot::Tunnel(uuid)` for the bastion. The URL and user name are **not** secrets here (they were encrypted in jdbgen only because the master password existed); `MaskedUrl` still keeps inline credentials out of logs.

---

## 6. Metadata

All through the bridge's `DESCRIBE` (`catalogs`, `schemas`, `tables`, `columns`, `primary_keys`, `imported_keys`, `exported_keys`, `indexes`). The `rudbgen-meta` crate builds the template model (and implements `rudbgen_template::Model` for it), so it is testable against H2 without a window:

- `Table { catalog, schema, name, table, title, type, remarks, columns, keys, not_keys, imports, exports, indexes, no }` — the first nine exactly as jdbgen; `imports`/`exports`/`indexes` new.
- `Column { … jdbgen's 18 fields …, precision, scale, auto_increment, fk: Option<ForeignKeyRef> }`.
- `ForeignKey { name, columns[], ref_table, ref_columns[], on_update, on_delete }`, `Index { name, unique, columns[] }`.

Custom queries (D9) run through `EXECUTE` on the session, **on the Rust side**, with jdbgen's contract exactly — so a driver definition imported from jdbgen works unedited: the table-list and column-list queries are read **by label** (`TABLE_CAT, TABLE_SCHEM, TABLE_NAME, TABLE_TYPE, REMARKS` / the eleven column labels including `IS_KEY`), the two comment queries are read **positionally** (1 = name, 2 = comment), and the parameters are `${catalog}`, `${schema}` and, for the per-table ones, `${table}`. The comment queries apply only to names they return (jdbgen's column-comment path overwrote comments it did not return — that is a bug, not a contract, and is not reproduced). The bridge's own comment enrichment (`meta/Comments.java`, inherited from rudbman and scoped per schema) is an internal detail of `DESCRIBE` and is not exposed as a user-editable query; a custom comment query, when enabled, replaces its result for the names it returns.

Every catalogue read is one round trip on the session worker; the UI thread never waits.

---

## 7. The template engine (`rudbgen-template`)

### 7.1 Scope

jdbgen's language, exactly: `${item}` shorthand and long forms, `super`, `if/elif/else/endif` with the ten conditions, `for/endfor` with `inStr`/`indent`/`skipList`, `date`, `user`, `author`, literal escapes, the twelve decorators, the five extra decorators, the quoting and escaping rules, the error messages with line numbers. Parse once, render many; the parsed template is immutable.

### 7.2 Compatibility canary

`TemplateManagerTest`, `TemplateManagerSyntaxTest`, `TemplateManagerErrorTest` are ported case for case before the engine is written; the three shipped templates rendered against jdbgen's `TestResultSet` fixture must be byte-identical to jdbgen's output (captured once from the Java build and checked in as fixtures).

Known deliberate differences, listed in `docs/template-reference.md`:
- D10 — word-level abbreviation rules match case-insensitively.
- Column-comment custom queries do not erase comments they do not return.

### 7.3 Model

```rust
pub trait Model {
    fn get(&self, key: &str) -> Option<Value>;          // Value: Str | Int | Bool | List(Vec<Box<dyn Model>>) | Null
    fn set_no(&mut self, no: usize);                    // the for-loop counter
}
```
The app implements it for `Table`, `Column`, `ForeignKey`, `Index` and for the custom-variable map (the fallback, looked up after the model, as in jdbgen). `Diagnostics` collects unknown-field warnings with source spans so the editor can mark them.

### 7.4 Width

Padding and `indent` count **EUC-KR bytes** of the decorated value, as jdbgen does, via `encoding_rs::EUC_KR` (unencodable characters count 1, the Java fallback's behaviour).

---

## 8. The editor and highlighting

`rudbgen-editor` is rudbman's editor with `Highlighter` behind a trait:

```rust
pub trait Highlighter: Send + Sync { fn tokens(&self, line: &str, state: State) -> (Vec<Span>, State); }
```
Two implementations: `TemplateHighlighter` (text, `${`…`}` statement, statement name, option name, value, decorator, literal; unbalanced `}` as error) and a small `SqlHighlighter` (keywords, strings, comments, `${placeholder}`) for the custom-query editors. The editor theme's twelve token colours are mapped onto both.

---

## 9. Generation

`rudbgen-gen`'s `generate(plan: Plan, policy: Overwrite, cancel: CancelToken, progress: Sender<Progress>) -> Outcome`, run by the app on a background thread:

1. Load columns (and keys/FKs/indexes) for every ticked table — one `DESCRIBE` each, cached per connection until refresh.
2. Parse every ticked template and every output-name template once; a parse error aborts **before** any file is written.
3. For each table × template: render the name, resolve against the output dir (and refuse anything that escapes it), apply the policy, render the body, write atomically.
4. `Outcome { written, skipped, failed: Vec<(path, reason)>, cancelled }`.

Dry run is the same function with a `Sink::Memory` and no policy. Preview is a dry run of one pair.

---

## 10. Milestones

| | Scope | Done when |
|---|---|---|
| **M0** | Workspace shell: vendored gpui, `rudbgen-ui`, `rudbgen-core` (settings/paths/secrets), i18n (8 locales, rudbgen keys), themes, settings dialog, about, update, window chrome, welcome screen placeholder | `cargo test --workspace` green on CI's three platforms; the window opens with the welcome screen |
| **M1** | `rudbgen-template`: ported tests, engine, diagnostics, byte-identical fixtures | All ported tests pass; the three shipped templates match jdbgen's output |
| **M2** | Bridge (trimmed, renamed) + `rudbgen-jdbc`, driver store with merged stock drivers, Maven download, connection dialog with test, SSH, explorer tree with ticks and filter, inspector | Connect to the sample H2, tick tables, read columns/keys/FKs in the inspector |
| **M3** | Generate tab, template sets, generation job (cancel, policy, summary), status bar, preview & dry run | End-to-end generation against H2 produces jdbgen's fixture output |
| **M4** | Template tabs: editor with highlighter, variable palette, live preview, diagnostics | Editing a template updates the preview; an unknown field is marked |
| **M5** | Custom queries with Test, abbreviation rules dialog with D10 semantics, jdbgen import wizard (both encryption forms) | A jdbgen `config.json` imports with every connection usable |
| **M6** | Packaging (jlink runtime, three platforms), release workflow, `docs/template-reference.md`, `docs/ui-guide.md`, README | A tagged build installs and runs without a JDK on each platform |

Each milestone is a PR onto `main` from `dev` (rudbman's flow); `docs/status.md` is updated when one ends.

---

## 11. Open questions

1. Should the output-name template and the body template be allowed to come from the same file (a front-matter header), so a template is one file rather than a file plus a row in a set? Decided in M4 at the latest.
2. A second, saner `javaType` mapping (`DECIMAL`→`BigDecimal`, `TIMESTAMP`→`LocalDateTime`) is offered under a new field name — which name? (`javaType2` is ugly; `jtype`? `modernJavaType`?)

---

## Appendix A — pitfalls carried over from rudbman

Everything in rudbman's Appendix A applies unchanged: do not drop the gpui patches; never `DriverManager`; `Driver.connect` returning `null` is "not my URL"; `-Xrs` or the window will not close; one `URLClassLoader` per JAR set, not per session; never fetch on the UI thread; reclaim focus in the same update that hides a subtree; never call back into gpui with a `RefCell` borrowed.

Two of jdbgen's own:
- `${a.b}` is a **processor chain**, not a nested path. `Models.getValue` supports dotted paths but the template syntax never passes one through; the Rust `Model::get` takes a single key.
- `endif`/`endfor`/`else`/`elif:` are matched **lower-case only** while everything else is case-insensitive; `${ELSE}` silently becomes an item lookup. Reproduce it (assets depend on it) and let the highlighter warn.
