# Progress and handoff

This document exists so that another session — or another person — can pick
the work up. The design and the contracts all live in
[architecture.md](architecture.md); what is kept here is only **how far the
work has come, what is left, and how work is done in this repository**. It is
updated whenever a milestone ends.

Last updated: 2026-08-25 (the application-level shell extracted to [rugpui](https://github.com/xcomart/rugpui) as `rugpui-shell`, D13a — `caption.rs`, `about_dialog.rs`, `context_menu.rs`, `pane_tree.rs`, `theme_editor.rs`, `update.rs` and `update_dialog.rs` are gone from `rudbgen-app`, along with the halves of `i18n.rs`, `app_settings.rs`, `icons.rs` and `settings_dialog.rs` that were not about rudbgen; what the shell has to be told is injected in `main` through `AppIdentity`, `Strings` and `UpdatePolicy`. Before that: the widget layer extracted to the same repository, D13 — `rudbgen-ui`, `rudbgen-grid`, `rudbgen-editor` and `vendor/` come back as `rugpui`, `rugpui-grid` and `rugpui-editor`; the template highlighter stayed, in `rudbgen-app`. Before that: M5 done — the abbreviation rules editor, the jdbgen import wizard and the custom-query Test's remaining half. Every milestone M0–M6 is now closed on Linux).

## Where things stand

| Milestone | State | What went in |
|---|---|---|
| M0 | done | The workspace foundation and the application shell, described below. The window opens on the welcome screen; the settings dialog, the theme editor, the about box and the update check all work end to end |
| M1 | done | `rudbgen-template`: the ported jdbgen tests, the engine, diagnostics, byte-identical fixtures. 130 tests, three of jdbgen's own test classes among them |
| M2 | done | The trimmed bridge (D3) + `rudbgen-jdbc`, the merged stock driver store, Maven download, the connection dialog with Test, SSH wired up, the explorer tree, the inspector. In the tree so far: `DriverDef` carries jdbgen's four custom queries (D9), its driver-wide properties and `noAuth`, and `builtins()` is the merge of rudbman's seven products with jdbgen's ten — H2 split into the embedded and the server form, MongoDB and CUBRID added, every Maven coordinate checked against Central; `rudbgen-grid` copied from rudbman whole; `maven.rs` copied into `rudbgen-app` and declared, waiting for the driver editor to press it; **`bridge/` and `crates/rudbgen-jdbc/` copied and trimmed per D3** — see below; **`crates/rudbgen-meta/` written**: `MetaReader` turns `DESCRIBE` and the four custom queries (D9) into `Table`/`Column`/`ForeignKey`/`Index`/`Schema`, which implement `rudbgen_template::Model` with jdbgen's member names and D8's new fields (`imports`, `exports`, `indexes`, `precision`, `scale`, `autoIncrement`, a column's `fk`) — 36 unit tests over the pure rules ported from jdbgen's `DBColumnTest`/`DBTableTest`/`SqlTypesTest`, 13 H2 integration tests including an end-to-end render of the shipped `java_model.java`. It caches nothing: the app owns the cache | **The connection path is wired end to end**: `connection.rs` (tunnel → JVM → `OPEN_SESSION`, the shared tunnel registry, `ConnectError` and its hints), `connection_dialog.rs` (D7: the dialog edits a draft and only `Save` writes `connections.json` and the keychain) and `driver_manager.rs` copied from rudbman and adapted — the driver editor replaces the dialog's form body, never nests. The editor gains **D9's Custom queries section**: the four rows in `CustomQueryKind::ALL` order, each a tick, a multiline SQL field, a **Test** button and the contract it is checked against, with one shared row of `${catalog}`/`${schema}`/`${table}` test values. Test runs the statement on the open session for that driver, or on a temporary one opened from the connection form, and reads the result's labels back against `required_labels()`/`positional_columns()`. `rudbgen-ui`'s `TextInput` grew a `multiline(rows)` mode for those fields (wrapping, `Up`/`Down`, click-to-place, `Enter` breaks the line) — replaced by `rudbgen-editor`'s SQL highlighter in M4/M5. The `Workspace` owns the session as a `ConnectionState` (idle / connecting / open / failed): the title-bar selector lists the saved profiles with a status dot and a **Disconnect** row, a welcome row connects, `Ctrl+N` and the two menu rows open the dialog, a failure lands on the status bar and a banner rather than in a box, a tunnel that dies takes its session with it, and the session is closed on disconnect and on quit. **The explorer and the inspector close the milestone** — see `docs/screenshots/explorer.png`. `explorer.rs` is the left sidebar: a `TreeView` over catalog → schema → table, the catalog level skipped when there is only one, a schema's tables fetched lazily on the first expansion and cached per schema **views included whatever the toggle says**, so flipping *Show views* costs no round trip. Over the tree sit jdbgen's `filterTables` box (contains, case-insensitive, a pure function with its own tests) and that toggle; on every schema and table row sits a tick box that swallows its own press so ticking is never selecting. The selection is a `BTreeSet<TableKey>` and survives the filter, the toggle, a collapse and a refresh — it is only emptied by a new connection — while a schema's box is three-state and acts on the rows *on screen*, as do the right-click menu's `select all shown` / `clear` / `invert`, beside `open in inspector` and `refresh`. `inspector.rs` is the right panel: the table under the cursor, fetched in the background and cached per table so walking the tree costs one round trip each, with four tabs — Columns as a `rudbgen-grid` `GridView` (#, name, type, nullability, default, `PK`*n*, comment), Keys in *key* order, Foreign keys in both directions, Indexes — and its own loading, empty and failed states. The `Workspace` lays the three columns of §4.2 out with a draggable divider either side, remembers `explorer_width`/`inspector_width` and both visibilities in `AppSettings`, hides both panels when nothing is connected, reclaims the keyboard in the same update that hides one, counts the ticks on the status bar and runs every `MetaReader` call on a background task keyed by a connection epoch, so an answer that outlives its session is dropped rather than drawn. `Ctrl+B` and `Ctrl+I` toggle the two. 43 new tests, 22 of them driving the real widgets under gpui's test support, and one end-to-end against a real H2 through a real JVM: connect, list, tick, and read the columns, the primary key and the foreign key back out of the inspector
| M3 | done | The Generate tab, template sets, the generation job (cancel, policy, summary), the status bar, preview and dry run — see `docs/screenshots/generate.png`, `summary.png` and `overwrite.png`. **`crates/rudbgen-gen/`** is §9's job end to end, with no gpui and no JNI in it: `Plan` (tables already loaded, templates, output dir, author, custom variables, abbreviations, clock and user) → `generate` / `dry_run` / `preview`. Every body and every output-name template is parsed first, so one parse error writes no file at all and names the template, the half and the line; an output name is rendered per table and resolved against the output directory, where an absolute path or any `..` is refused; files are written atomically through `rudbgen_core::paths::write_atomic`, creating the directories a name like `${package.replace('.','/')}/${name.pascal}.java` implies; the policy is `Overwrite`/`Skip`/`Ask(callback)` with `Overwrite`/`Skip`/`OverwriteAll`/`SkipAll`/`Cancel` answers, cancellation is checked at file boundaries through a `CancelToken`, and `Outcome` lists every file written, skipped and failed together with the engine's unknown-field warnings per table × template. 55 tests, among them the **M3 compatibility check**: a whole run against a hand-built `rudbgen_meta::Table` writes the three shipped templates byte-identical to `crates/rudbgen-template/tests/golden/*.expected`, with no unknown field left over. **The application half closes the milestone.** `templates/` holds jdbgen's three shipped templates, CRLF and all (`.gitattributes` marks the directory binary — the engine takes a template's line ending from its first newline); `builtin_templates.rs` compiles them in with `include_bytes!`, copies them into `<config>/templates` on a first run **without ever overwriting one that is there** (§5), and seeds the two built-in sets — `Java + MyBatis` and `PHP CodeIgniter`, under fixed UUIDs — exactly once, `TemplateSetStore::builtins_seeded` being the field that makes a set the user deleted stay deleted. `generate_pane.rs` is §4.4's tab and **the only editor of a connection's `GenerationProfile`**: the set selector (the saved sets plus `Custom` once the list matches none of them, an ordered field-by-field comparison with the ticks counted, and `Save as set…`), the template list (tick, name, file, output-name template, edit and remove, `Add template…` through the platform file picker), and the options — output directory with its chooser, author, the custom-variable table with the trailing empty row rule, and the abbreviation switch beside a `Rules…` button disabled until M5. Every edit is written back to `connections.json` **debounced and diffed**: a text field notifies when the caret moves as well as when the text changes, so the panel rebuilds the profile and writes only when it differs from what it last wrote. The status bar carries §4.2's arithmetic — *n* tables · *m* templates → *n×m* files into the directory — and `[Preview] [Dry run] [Generate ▶]`, each greyed with the **first** thing missing in a tooltip rather than an error box after the click. `generate_job.rs` runs the job on a thread of its own and pumps its `Progress` back over a channel into the progress dialog (bar, log, Cancel); under the *ask* policy the run's callback puts the path on that same channel and blocks on the answer, which is the **overwrite dialog** — Overwrite / Skip / Overwrite all / Skip all / Cancel — and the run ends in a **result summary** listing every file written, skipped and failed with the failing template's line, the warning count, and *Open output directory*. `preview_pane.rs` is the Preview tab: one pair chosen by two dropdowns in the header, or a whole dry run as a file list (path, whether it replaces something, how big it would be) with the text of the selected row underneath, read-only monospace until M4 puts an `EditorView` there. The work area is now `pane_tree::Pane` behind a `TabBar` — Generate permanent, one Preview tab reused — and `Generate` is on `Ctrl+G`, with `Preview` and `DryRun` in the menus. 74 new i18n keys in all eight languages (`generate.*`, `progress.*`, `summary.*` plus the new `menu.*` and `statusbar.*` rows), with three of them added to the translation probes. 18 new tests: the pure rules (the set match, the variable table's two rules, the status-bar arithmetic and the order its reasons are reported in, the config-relative path round trip, the built-in installer's two refusals to overwrite, the ask channel in both directions) and one end-to-end against a real H2 through a real JVM — tick two tables, plan, generate, two files on disk from the shipped template, then a second run over the same directory that stops on the overwrite question and skips them all, then a preview and a dry run into the tab |
| M4 | done | Template tabs: the editor with its highlighter, the variable palette, live preview, diagnostics — see `docs/screenshots/template-editor.png` and `completion.png`. **`crates/rudbgen-editor/` was copied from rudbman with the highlighter pulled out from under it** (§2.1, §8). `Highlighter` is now a trait — `fn line(&self, text: &str, state: LineState) -> (Vec<Span>, LineState)` — behind an `Arc<dyn>`, and the incremental cache that used to *be* the highlighter is `SyntaxCache`: still one `LineState` per line, still re-lexing from an edit down to the first line whose end state is unchanged. `LineState` is one opaque `u32` rather than an associated type, which is what keeps the trait object safe; `Span`'s `Token` is exactly the editor palette's twelve token slots, so mapping a span to a colour is total, and spans may leave gaps (the element fills them with the foreground colour) so that a highlighter for a language that is mostly prose does not have to invent a token for prose. `rudbman-sql` is gone with it: `sql_syntax::SqlHighlighter` is a two hundred line dialect-free lexer for the custom-query editors, and `syntax.rs`'s statement splitter and bracket matcher now ask the *highlighter* which `;` and which `(` are real instead of a SQL lexer — so both work over a template too. **`template_syntax::TemplateHighlighter`** tokenizes jdbgen's grammar a second time, line by line and never failing, because the engine's parser either parses a whole template or returns a `ParseError` and an editor whose colours vanish on every other keystroke is worse than one with none; it reproduces the oddities — the first `}` closes a statement even inside a quoted value, `${'…'}` is a literal, `endif`/`endfor`/`else`/`elif:` are lower case only (`${ENDIF}` is painted as a *warning*, per appendix A), a statement with no `:` is an item, and a statement may run over line breaks. **`composite::CompositeHighlighter`** layers that over a base language — Java, XML, PHP, C-like — chosen from the file's extension by `lang::template_highlighter_for_path`, the base's spans cut where a statement stands and both states packed into one `LineState` (16 bits each). Two of §8's error cases are deliberately **not** painted — a `}` in the text between statements (a Java or XML template is full of them) and a `${` that is not closed yet — because both are whole-document verdicts, which is what the gutter marks are for. **The application half closes the milestone.** `template_pane.rs` is §4.5's tab, one per open file: the buffer (`\n` only, whatever the file holds — the CRLF the shipped templates are written with is remembered on load, restored on save, and restored again for every render, because the engine takes a template's line ending from its first newline), a **300 ms debounce** off the keystroke path, and two verdicts that land in one list and one set of gutter marks — the **parse**, which is local and is a single error with a line, and the **render**, which needs a table, comes back from the shell and is a list of unknown fields with a span each. A parse error suppresses the render, since a template that does not parse renders to nothing. Clicking a row in the list moves the caret to it. The tab splits down the middle (draggable, and the preview half toggles away) and the right half is the **live preview**: the shell renders the *buffer* — `TemplateSpec::source`, so the file on disk is never read — against the ticked table the header's dropdown picks, through `rudbgen_gen::preview` with a **literal** output name, so that every warning it reports is the body's and its span therefore an offset into the buffer the marks are drawn on; the name the run would really give the file is rendered separately, for the header alone. The preview is a read-only `EditorView` coloured by the *output* file's language, and so is the Preview tab's text now. `Ctrl+S` writes the file atomically, the tab wears a dot until it does, and closing one with edits in it asks **Save / Don't save / Cancel**. **`palette.rs`** is the pure half of §4.5: 98 entries — every key `rudbgen-meta`'s four `Model` impls answer, the eleven statements and three `for` controls, the twelve chain processors and five extra decorators, the ten conditions and the three key options — each with a description summarised from jdbgen's template reference (i18n, not English in the source) and, when a table has been read, an example of what it renders to. It also holds the completion rule: `completion_at` reads the caret's context out of the text before it on its line — text, just inside a `${`, after a `.`, after a `:` or `,`, or the value of a `key=` — and `matching` filters and ranks the entries that may be written there, one row per thing that can be *typed* (the four models' `name` is four entries in the panel and one suggestion in the popup). **`variable_palette.rs`** is what the inspector's column becomes while a template tab is on top: the eight sections over a filter box that matches names *and* descriptions, a click writing `${…}` at the caret. The **completion popup** is anchored to `caret_bounds`, `deferred` over everything, and driven by keys the editor hands over: `EditorView::set_intercept` turns `Up`/`Down`/`Enter`/`Tab`/`Escape` into `EditorEvent::Intercepted` while it is up, because those five are bound on the editor's own key context — the innermost node of the dispatch path — and nothing the host wraps around the editor could take them first. `Escape` still belongs to the find bar while *it* is open. `EditorView` also gained `set_marks` and the element paints them: a bar at the left of the gutter and a wash across the row, in the editor palette's `error` and `warning`. A template tab **survives a reconnection** (§4.2) and opens with no connection at all — the welcome screen's *Open a template* is live, as is a double click on a template row's path — so the work area is drawn whenever a connection *or* a template is open, and the Generate tab shows the welcome screen while nothing is connected. 134 new i18n keys in all eight languages (`template.*`, `diagnostics.*`, `completion.*`, `palette.*` and five `menu.*` rows), three of them added to the translation probes, and the M0 placeholder namespace `workarea` is gone. 32 new tests: the pure rules (the completion contexts, the scope and ranking of what they offer, the palette's entries against the model's own keys, the line-ending round trip, a diagnostic's line and column, the gutter marks' one-verdict-per-line rule and the intercepted keys) and three end to end — a template opened, edited, diagnosed, saved and closed with no database at all, the popup offering and accepting, and against a real H2 through a real JVM: tick a table, open a template, see it rendered, then type `${nmae}` into the *buffer* and watch the warning land on line 1 while the file on disk stays as it was |
| M5 | done | Custom queries with Test, the abbreviation rules dialog with D10 semantics, the jdbgen import wizard. In the tree so far: **`crates/rudbgen-import/` written** — D5's one-time import, pure, with no gpui in it and no way to write anything. `locate` finds jdbgen's `config.json` by jdbgen's own directory rule (`BaseDirs::config_dir()/jdbgen` on all three platforms); `read` parses it leniently — a `null` array is an empty one, a BOM is stripped, an unknown key is ignored — leaving the three encrypted fields as ciphertext, so the wizard can say what it found before asking for anything; `decrypt` opens `connectionUrl`, `userName` and `userPassword` with **both** of jdbgen's schemes, the current `ENC2:` form (AES-256-GCM under a PBKDF2-HMAC-SHA256 key of 210,000 iterations, salt and nonce carried in the value) and the superseded one (AES-128/CBC/PKCS#5 keyed with the two halves of SHA-256(master)), a file holding a mixture included. A `Decryptor` caches the derived key per salt as jdbgen's own `KEY_CACHE` does — twenty connections cost one derivation, not sixty — and wipes the master password when it is dropped: nothing here stores it, logs it or renders it in a `Debug`. Every cryptographic failure is one `Error::WrongPassword`, whether the GCM tag or the CBC padding said so; only a damaged envelope (not Base64, shorter than salt+nonce+tag) is told apart, because no password produces that. `map` is the pure function to the stores: a stock driver whose `driverClass` a built-in names keeps **rudbgen's** id, URL template and dialect while jdbgen's JAR, four SQL overrides and properties ride along (H2's two forms are told apart by name); everything else becomes a `DriverDef` of its own with an id of slug + UUID. `<databaseHost>`/`<database>`/`<database file>` become `{host}`/`{database}`/`{file}` and any other `<x>` becomes `{x}`, while the hard-coded port after the host — jdbgen's `:1521`, and H2's optional `[:9092]` — becomes `{port}` and the driver's `default_port`. A connection's `driverType` is a driver *name* and is resolved against the drivers the same import produced; the URL and the user name stay in the profile (§5: they were encrypted only because a master password existed) and only the password comes back separately, as a `Secret` bound for the keychain. `keepAliveSec` is a string in jdbgen, so it is parsed and the probe is switched **off** with a note when it is not a number. Relative paths — JAR, template, output directory — are resolved against jdbgen's data directory and then its installation directory, and a path in neither is carried across as written with a note rather than fabricated. Presets become `TemplateSet`s, `abbrs` become `AbbreviationRule`s, and `language`/`isDarkUI` come back as a `settings_hint` for the app to accept or ignore. Everything the import cannot carry across cleanly is a `Note` — data, never a sentence, because the app owns the translations — and **D10's case-insensitive word rules are noted always**, whether or not the file holds a rule. `preview` is the same mapping read back as the wizard's checklist, with the URLs masked the way `MaskedUrl` masks a log line. 68 tests: the decryption vectors in `tests/vectors/decrypt.json` were produced by jdbgen's own `StrUtils` and by the `legacyEncrypt` recipe of its `EncryptionTest` (jdbgen fixes no ciphertext of its own — its suite encrypts and decrypts inside one run), `tests/vectors/config.json` is a synthetic configuration written by that same Java, and `tests/vectors/defaultConfig.json` is jdbgen's shipped stock file copied verbatim. **The application half closes the milestone** — see `docs/screenshots/abbreviations.png` and `import.png`. `abbreviation_dialog.rs` is §4.6's rules editor over the one global `abbreviations.json`: four columns (apply, whole name, abbreviation, replacement) and a delete glyph, the trailing empty row rule the Generate tab's variable table already follows, and the **apply** switch itself — one value with two controls, filled from the Generate tab when the dialog opens and handed back to it on `Save`, so the panel that a run reads the rules from is never a second answer. D7 to the letter: the rows are a draft, `Save` is the only write, and `Cancel`, `Escape` and the backdrop throw it away. Beside a whole-name row's abbreviation sits a **table-name picker** offering what the explorer has loaded (`Explorer::loaded_table_names`, the whole index rather than the visible rows — a rule is about a name, not about what the filter box is showing), drawn only while something is connected. A duplicate is refused per *kind*: the engine keys its dictionary by the lower-cased abbreviation with whole names in one map and words in another, so two enabled rules that agree on both would end as one entry and which survived would be invisible — `Save` is disabled and the offending rows outlined, while a whole-name `EMP` beside a word `EMP` is two useful rules and is left alone. `import_dialog.rs` is the wizard of D5, in three steps over `rudbgen-import`: the file (`locate`, or one chosen with *Other file…*) and the master password, masked, read out of the field once into a `Zeroizing<String>` and decrypted on a **background** task because PBKDF2 is meant to be slow, with `Error::WrongPassword` coming back as a message beside the field rather than as a failed import; the **checklist**, a tick per connection, driver and set plus the rules, jdbgen's language and theme, the name-conflict policy and every `Note` the mapping produced (the D10 announcement always among them); and the **result**. The merge itself — `merge(&Mapped, &Selection, OnConflict, &mut Stores)` — is a pure function with no disk and no keychain in it, which is what makes it provable: drivers first, because a renamed driver id has to reach the profiles that name it before they are written; names compared case-insensitively against both the store and what the same import has already added; `(imported)` after the name, numbered on a second clash, or the entry left behind; a stock driver that lands on a built-in rudbgen already ships is the common case and *keep both* repoints the imported connections at the imported definition; a rule that repeats an enabled rule's abbreviation is always skipped, because it could never fire; and `apply_to_names` is only ever turned **on**, never off behind the user's back. The passwords go through a `SecretSink` rather than to `SecretStore` directly — the trait exists so a test never writes into the developer's own login keyring — and a machine with no credential store is not a failed import: those profiles are saved without their password and named on the last step. The settings hint is applied by the *shell*, never by the dialog. The driver rows of the checklist come from the **mapping** rather than from `Preview`, and there is a test that says why: two jdbgen entries naming one product collapse into one `DriverDef`, so the preview — one row per entry in the file — is longer than the list the ticks index into, and a checklist drawn from it would untick the wrong driver. The welcome screen's **Import from jdbgen…** and the Generate tab's **Rules…** are live, `soon()` and its tooltip are gone — the import button is drawn only when `locate()` found a file, as §4.3 asks, and the menu row is live either way so that a configuration copied from another machine still has a door, and both dialogs have a menu row and an action of their own (`EditAbbreviations`, `ImportJdbgen`) in the in-app menu and the macOS menu bar. The custom-query **Test**'s remaining half is the width check: a positionally read result was rejected unless it had *exactly* the two columns, while `rudbgen-meta`'s reader takes the first two and ignores the rest — so `select name, comment, owner from …` failed a test and generated perfectly good comments. Test now complains only when the result is too **narrow**, and both the message and the contract line say *the first n columns are read*. 61 new i18n keys in all eight languages (11 `abbr.*`, 48 `import.*` and two `menu.*` rows), two of them added to the translation probes; `welcome.tip_soon` and `generate.tip_rules_soon` are gone. 33 new tests: the pure rules (the duplicate verdict per kind and ignoring case, the trailing empty row, the blank-row drop and the trim, the name-conflict policy in both directions, the driver repoint, the rule merge, the `Imported` `Debug` that carries no password, every note and every failure message translated) — among them the whole of `tests/vectors/config.json` decrypted, mapped, merged and read back out of four store files in a temporary directory — the wizard's own step 1 driven under gpui's test support — a wrong master password comes back as a message beside the field and another go, the right one opens the checklist, and a file that is not a configuration says so rather than asking again — and one end to end against a real H2 through a real JVM: *Rules…* opens over the panel's store with the loaded tables in the picker, `album` → `Disc` is typed into the trailing row, and `${name.abbr}` in a template tab renders `T_SAMPLE_Disc` — which is D10, since the segment is `ALBUM` |
| M6 | done | Packaging (jlink runtime, three platforms), the release workflow, `docs/`: the [user interface guide](ui-guide.md), the [template reference](template-reference.md), [custom queries](custom-queries.md), [installation](installation.md) and the README. Brought forward ahead of M5 and brought up to date by it: the guide describes the rules editor, the import wizard and the template tab as they are, and nothing in `docs/` is marked *next release* any more |

### What is in the tree today

| Piece | Notes |
|---|---|
| `rugpui`, `rugpui-grid`, `rugpui-editor` | Not in this tree at all (D13). The widget kit, the virtualised grid and the code editor live in [rugpui](https://github.com/xcomart/rugpui), together with the four patched gpui crates they are written against — `RULOGMAN PATCH` markers and all — because rulogman, rudbman and rudbgen were carrying byte-identical copies of every one of them. They come back as dependencies at one pinned revision, and the `[patch."https://github.com/zed-industries/zed"]` table has to name that same revision or two gpui crates end up in one binary. Nothing in rugpui is rudbgen-specific and nothing in it may become so: the theme store is handed a `ThemeDirs` rather than knowing a configuration directory, and the editor composes an `Overlay` the host supplies rather than knowing a template grammar |
| `Cargo.toml` | The workspace, the `[patch."https://github.com/zed-industries/zed"]` table pinning gpui to one revision of Zed's monorepo — pointed at rugpui's vendored copies (D13) — `[workspace.dependencies]` for what the crates use, and rudbman's profile tables. Every dependency still carries its "why" comment, rewritten where the reason differs. `rudbgen-app` pulls in `gpui_platform`, so all five patches are live |
| `crates/rudbgen-core` | rudbman's core with the fields rudbgen does not have removed and the ones it needs added — see the next table |
| `crates/rudbgen-ssh` | rudbman's tunnel crate, whole: russh, a bastion without a PTY, loopback binds, the trusted-host-key check against `known_hosts`. Tunnels are already wired into the connection dialog rudbgen inherits, so removing them would cost more than keeping them (§2.1) |
| `crates/rudbgen-app` | The binary, `rudbgen`. rudbman's bootstrap sequence, `actions!`, `bind_shortcuts`, menus and window-chrome helpers; its `app_settings`, `caption`, `icons`, `about_dialog`, `theme_editor`, `settings_dialog`, `update` and `update_dialog` with the names, URLs and settings that changed; `i18n.rs` and eight locale files trimmed to rudbgen's keys; a `Workspace` written from scratch around §4.2's sketch. Most of that shell has since gone to `rugpui-shell` (D13a); see the third table below for what diverged and what is left |
| `bridge/` | rudbman's JDBC bridge, package `comart.rudbgen.bridge`, artefact `rudbgen-bridge.jar`, **trimmed per D3** — see the table below for what went. 58 JUnit tests against in-memory H2; `cd bridge && ./gradlew build` runs them |
| `crates/rudbgen-jdbc` | rudbman's JNI crate against that JAR: `jvm`, `session`, `protocol`, `codec`, `error`, `response`, `spec`, minus the data plane (D3). Env names are `RUDBGEN_JAVA_HOME`, `RUDBGEN_BRIDGE_JAR`, `RUDBGEN_TEST_H2_JAR`. 46 unit tests, 22 H2 integration tests that boot a real JVM, and two opt-in suites (five servers, SQLite) |
| `crates/rudbgen-app/src/template_syntax.rs` | The one piece of the editor that did not go to rugpui (D13): jdbgen's template grammar, tokenized a second time — line by line and never failing, because the engine's parser either parses a whole template or returns a `ParseError`, and an editor whose colours vanish on every other keystroke is worse than one with none. It implements `rugpui_editor::Overlay` — spans plus the byte ranges it took charge of — and `template_highlighter_for_path` composes it over the base language the file's extension names, through `rugpui_editor::CompositeHighlighter`. 27 tests. See the M4 row above and §8 |
| `docker/compose.yml` | The five servers the opt-in `containers` suite runs against, on non-standard ports so a developer's own server is never shadowed. `rudbgen`/`rudbgen` everywhere but SQL Server, whose `sa` complexity rule forces `sa`/`Rudbgen!Passw0rd` |
| `assets/` | `icon.svg` — the master mark, a sheet of generated source with `</>` on it, rudbman's two colours swapped — with `icon-128.png`, `icon-256.png`, `icon.ico` and `icon.icns` rendered from it by `render.py`. `assets/drivers/` holds jdbgen's eleven driver icons, unused until M2 |
| `.github/workflows/ci.yml` | rudbman's `check` job — three platforms, `cargo fmt --check` once, `clippy --workspace --all-targets --locked -- -D warnings`, `cargo test --workspace --locked` — now with the JDK/Gradle steps in front of it (`cd bridge && ./gradlew build`, then the H2 JAR handed to the Rust tests through `RUDBGEN_TEST_H2_JAR`), plus the opt-in `containers` job: five servers from `docker/compose.yml`'s ports, SQLite riding along, all of them reading metadata only |

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
  What came over is the bootstrap sequence — `env_logger` →
  `rugpui_shell::init_process_identity` → `update::apply_pending` →
  `application().with_assets(Icons)` → keychain → settings → `i18n::apply` →
  `rugpui::init` → shortcuts → menus → themes → the window-closed save →
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
- **`context_menu.rs`** and **`pane_tree.rs`** have since gone to `rugpui-shell`
  (D13a) as `menu_rows` and `pane`; what stayed is `pane_item.rs` — rudbgen's
  three tabs, `Generate`, `Template { file, title, dirty }` and `Preview`,
  carrying identities rather than views — and the two lookups over them, written
  as an extension trait over `rugpui_shell::Pane::position`.
- **The updater** carries a **fresh `ProductCode` GUID**: a product code names a
  product, and sharing rudbman's would have winget treat an installed rudbman
  as an installed rudbgen. It is `IDENTITY.windows_arp_key` in `main.rs` now,
  handed to `rugpui-shell` along with the payload, the release endpoints and
  `must_defer`, which answers `false` — the question it asks is "is a JVM loaded
  into this process", and no path reachable from an update check loads one. The
  test that checked the GUID against `packaging/windows/rudbgen.iss` comes back
  with `packaging/` in M6.
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

### What the bridge and `rudbgen-jdbc` dropped (D3)

The two are rudbman's, copied and cut down. rudbgen reads a schema and writes
files from it; it never ferries row data, so the whole data plane went. What is
left is byte-for-byte rudbman's apart from the package name, so a fix moves
between the two as a plain `diff`.

| Gone | Java | Rust |
|---|---|---|
| The job layer — extract, backup, transfer | `job/` (7 classes), `meta/Upsert` | `Op::{JobStart, JobPoll, JobCancel}`, `Job`, `Session::start_job`/`start_transfer`/`start_backup`, `ExtractSpec`/`TransferSpec`/`BackupSpec` and their option types, `JobProgress`, `JobState` |
| The LOB reference path | `codec/LobSink`, `ColumnWriters.LobWriter`, `Cursor.LobRef` | `Op::LobRead`, `ColumnKind::Lob`, `Value::Lob` |
| The four vendor-catalogue `DESCRIBE` kinds — `ddl`, `procedures`, `functions`, `sequences` | `meta/Ddl`, `meta/Routines`, `meta/Sequences` | `Session::describe_ddl`, `DdlSource`, `DdlResult` |
| The template engine (it is `rudbgen-template`, D4) | `template/` | — |

Two consequences worth knowing:

- **Retired operation codes are never reused.** `0x25` and `0x40`–`0x42` stay
  spent on both sides of the boundary, and so does batch kind `6`. Both tables
  carry a comment saying so, and `protocol.rs`'s own test asserts that no `Op`
  variant claims one. The point is that rudbman's op table and rudbgen's keep
  lining up line for line.
- **LOBs are now materialised** — `BLOB` as `BIN`, `CLOB`/`NCLOB` as `STR` —
  because there is no `LOB_READ` left to resolve a reference with. That is safe
  only because the sole statements rudbgen executes are the four custom
  catalogue queries (D9), whose values are names and comments, and Oracle hands
  a comment back as a `CLOB`. It is written down in `bridge/README.md`'s *Known
  gaps*: a custom query pointed at a document table would carry the document
  across JNI.

`DESCRIBE` keeps `catalogs`, `schemas`, `tables`, `columns`, `primary_keys`,
`imported_keys`, `exported_keys`, `indexes` and `type_info` — §6's list, which
is everything the template model is built from — and `meta/Comments` stays,
because a driver that reports no comment where the server holds one is the
normal case rather than the exception.

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

A JDK 17 or newer is required from M2 on. `rudbgen-jdbc`'s build script
refuses to compile without the bridge JAR — it deliberately does **not** invoke
Gradle itself, because a JVM start-up per Rust edit is not a trade worth
making — so the Java half is built first:

```sh
cd bridge && ./gradlew build     # compile + test -> build/libs/rudbgen-bridge.jar
```

The H2 driver the JNI tests boot against is found in the Gradle cache that
build fills, or named explicitly with `RUDBGEN_TEST_H2_JAR`. The container and
SQLite suites are opt-in and pass by doing nothing with their
`RUDBGEN_TEST_*_URL` unset:

```sh
docker compose -f docker/compose.yml up -d      # then wait for "healthy"
cd bridge && ./gradlew drivers                  # the six JDBC drivers
# then export the URLs listed in docker/compose.yml's header
cargo test -p rudbgen-jdbc --test containers --test sqlite -- --nocapture
```

Tests that touch the real OS keychain are `#[ignore]`d, so a headless machine
runs the suite green. Run them by hand with:

```sh
cargo test -p rudbgen-core -- --ignored
```

Test count as of this entry: **726 passing** plus the 7 ignored ones, across
`rudbgen-core` (97), `rudbgen-ssh` (26, of which the in-process russh server
suite is the bulk), `rudbgen-template` (130), `rudbgen-meta` (50),
`rudbgen-jdbc` (75), `rudbgen-gen` (55), `rudbgen-import` (68) and
`rudbgen-app` (225), doc tests included. The ones that need a JVM run against a
real H2 and are counted above; the ones that need a server or a keychain are
`#[ignore]`d. The widget layer's own tests — some 350 of them — moved to rugpui
with the code (D13) and run in that workspace; the 95 that covered the
application shell followed them there with `rugpui-shell` (D13a), the updater's
46 among them. The 27 that cover the template grammar stayed and are counted in
`rudbgen-app`.

## What is next

Every milestone M0–M6 is done on Linux: `cargo build --workspace`,
`cargo test --workspace` (726 passing, 7 `#[ignore]`d, plus rugpui's own suite),
`cargo clippy --workspace --all-targets -- -D warnings` and
`cargo fmt --all --check` are all green, and `cargo run -p rudbgen-app` opens
the sample H2 connection, generates against it, edits a template with the live
preview beside it, writes an abbreviation rule that the preview then applies,
and imports a jdbgen `config.json` end to end — see
`docs/screenshots/welcome.png`, `explorer.png`, `generate.png`, `summary.png`,
`overwrite.png`, `template-editor.png`, `completion.png`, `abbreviations.png`
and `import.png`. What closes each milestone formally is the same suite green
on CI's other two platforms, which only CI can say — and that is now the whole
of what is left before a first tag.

Nothing in `docs/` is marked *next release* any more: the
[user interface guide](ui-guide.md) describes every surface §4 asks for.

Decisions taken along the way, written down so they are not mistaken for
oversights:

- **Open question 1** of the architecture document — whether a template's
  output name may live in a front-matter header in the file itself, rather than
  in a row of the set — was to be decided in M4 at the latest, and is decided
  **no**: the name is rendered per table with the same engine as the body, the
  live preview renders both, and moving it into the file would make a template
  unreadable to jdbgen for no gain the template tab does not already give.
- **Open question 2** — the name of the second, saner `javaType` mapping
  (`DECIMAL` → `BigDecimal`, `TIMESTAMP` → `LocalDateTime`) — is still open, and
  is the one piece of §6 that has no code behind it yet. Nothing depends on it:
  `javaType` keeps jdbgen's table verbatim, as D8 requires, so the new field can
  be added later without changing a single shipped template.
- The **abbreviation duplicate check is per kind** — a whole-name rule and a
  word rule may look for the same thing — because that is exactly the pair the
  engine keeps in two dictionaries. The architecture document says "duplicate
  rejection" and jdbgen's own check was flat; the narrower rule is the one that
  matches what the engine does, and it is documented in
  `abbreviation_dialog.rs`.
- The **import's keychain writes go through a `SecretSink`**, not through
  `SecretStore` directly. There is one implementation in the binary and one in
  the tests, and the reason is blunt: a test that reached the real credential
  store would write into the developer's own login keyring.
- **The custom-query Test now matches the reader**, not the contract as
  written: a positionally read statement may come back wider than the two
  columns that are read, and only a *narrow* result is a complaint. The
  contract line under each editor says *the first n columns are read* for the
  same reason.

Two things a first tag will want that no milestone owns:

- `packaging/` and the release workflow have now been exercised once: v0.1.0
  built and published on all three platforms from the tag alone. The Windows
  installer's `AppId`, the winget manifests' `ProductCode` and the uninstall
  key the shell's updater reads are checked against each other by a test, but
  the round trip — install, then self-update — has only been walked on Linux.
- The screenshots in `docs/screenshots/` are captures of the whole window
  *surface*, which under client-side decorations carries a 12 px transparent
  band for the drop shadow all round. Every one of them is now cropped to the
  window's own frame — the fully opaque part of an RGBA capture, which is
  exactly the window — and a new capture has to be cropped the same way, or a
  strip of whatever was behind the window composites into the edge. All twelve
  were re-taken together after M5 — in the **English** interface, where earlier
  captures were of the Korean one — and every one now carries that crop,
  `connection-dialog.png` included, which used to owe it. `import.png` is staged
  from `crates/rudbgen-import/tests/vectors/config.json` (the synthetic fixture)
  rather than from a real jdbgen configuration, so its checklist has something
  worth showing; the master password it unlocks with is the one in
  `import_dialog.rs`'s tests.
