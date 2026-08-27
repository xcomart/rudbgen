# User interface guide

rudbgen is **one window**. There is no modal gate to get through before you can
see anything, no separate generator dialog and no master password: the window
opens on a welcome screen, and everything else happens inside the same frame.

This guide describes the interface as it is in the current release, which is
every surface the [architecture document](architecture.md) §4 describes.

[← Documentation index](README.md)

- [The welcome screen](#the-welcome-screen)
- [Connecting](#connecting)
- [The driver editor](#the-driver-editor)
- [The window](#the-window)
- [The explorer](#the-explorer)
- [The inspector](#the-inspector)
- [The Generate tab](#the-generate-tab)
- [Abbreviation rules](#abbreviation-rules)
- [The template tab](#the-template-tab)
- [Preview and dry run](#preview-and-dry-run)
- [Running the generation](#running-the-generation)
- [Importing from jdbgen](#importing-from-jdbgen)
- [Settings](#settings)
- [Keyboard shortcuts](#keyboard-shortcuts)

## The welcome screen

![The welcome screen](screenshots/welcome.png)

With nothing connected, the work area is the welcome screen: your saved
connections as a list — icon, name, driver, when it was last used — and a **New
connection** button. Clicking a row connects. The explorer and the inspector
stay out of the frame until a connection opens, so there is nothing on screen
that has nothing to say.

**Open a template…** under it opens a template file in a tab of its own, with no
database in the picture at all: editing a template needs no connection, and only
the live preview does. **Import from jdbgen…** appears beside it when a jdbgen
`config.json` is where jdbgen keeps it — see [Importing from
jdbgen](#importing-from-jdbgen); the same command is in the menu whether or not
it is, so a configuration copied from another machine still has a door.

## Connecting

![The connection dialog](screenshots/connection-dialog.png)

**New connection** (or <kbd>Ctrl</kbd>/<kbd>Cmd</kbd>+<kbd>N</kbd>) opens the
connection dialog: a name, a driver, the URL, credentials, extra properties, a
keep-alive probe and an SSH tunnel.

- **The driver** picks the URL shape. Choosing one fills the URL with its
  template — `jdbc:postgresql://{host}:{port}/{database}` — and the fields under
  it fill the holes; the URL stays editable, so an option the form has no field
  for is typed straight into it. A driver marked as needing no authentication (a
  file-backed H2 or SQLite) hides the credential fields instead of asking a
  question with no answer.
- **The password** goes to the OS keychain, never to `connections.json`. See
  [Installation](installation.md#passwords-and-the-os-keychain).
- **Test** opens the connection, reports what came back, and closes it again.
- **SSH tunnel** takes a bastion host, a password or a private key, and binds
  only on loopback. It does not ask for a PTY, so a `nologin` jump host works.
- **Save** is the only thing that writes. Every field is a draft until then —
  cancel and nothing changed, including the keychain.

The title bar carries the connection selector: the open connection with a status
dot (connecting, connected, failed), every saved connection under it, and a
**Disconnect** row. Switching connections swaps the explorer and the Generate
tab's options, because those belong to the connection.

![Connected](screenshots/connected.png)

A connection that fails says so on the status bar and in a banner over the work
area — never in a box you have to dismiss before you can read the URL that was
wrong.

## The driver editor

![The driver editor's custom queries](screenshots/driver-custom-queries.png)

**Edit** beside the driver picker replaces the connection dialog's body with the
driver editor — one tab ring, never a dialog inside a dialog. It holds the
driver's class, its JARs, its URL template, its default port and the properties
handed to every connection that uses it.

- **JARs** come from Maven Central by coordinate (`group:artifact:version`, with
  a progress bar and a Cancel) or from disk.
- **The driver class is detected** from the JARs rather than typed: rudbgen asks
  the bridge which `java.sql.Driver` implementations they hold, without running
  a single static initialiser. Picking a sources or javadoc JAR by mistake says
  so in those words.
- **Custom queries** is the section at the bottom: four rows — table list,
  column list, table comments, column comments — each a tick, a SQL field, the
  contract it has to satisfy, and a **Test** button. One shared row of test
  values above them supplies `${catalog}`, `${schema}` and `${table}`.

  This is jdbgen's four SQL overrides, for products whose driver reports its
  metadata badly or not at all. [Custom queries](custom-queries.md) is the
  reference; the short version is that Test runs the statement on a session
  using this driver and tells you which required label is missing, rather than
  letting you find out in the middle of a generation run.

## The window

Once connected the window is three columns over a status bar:

```
┌ [⌂ rudbgen] [☰] [● Sample H2 ▾]                              ─ □ ✕ ┐
├──────────────┬─────────────────────────────────┬─────────────────┤
│ EXPLORER     │ WORK AREA (tabs)                │ INSPECTOR       │
│ 🔍 filter    │ ┌ Generate ┐┌ Preview ┐         │ T_SAMPLE_ALBUM  │
│ ☐ views      │ │                                │ Columns│Keys│FK│
│ ▾ PUBLIC     │ │  Template set  [Java + MyBatis▾]│ # name  type  K │
│   ☑ T_ALBUM  │ │  ☑ Java Model  ${…}Model.java  │ 1 ALBUM_ID INT ● │
│   ☑ T_ARTIST │ │  ☑ Mapper XML  ${…}-mapper.xml │ 2 NAME  VARCHAR  │
│   ☐ T_TRACK  │ │  ☐ PHP CI      …               │ …               │
│ ▸ INFO_SCHEMA│ │  Output   ~/out/src      [...]  │                 │
│              │ │  Author   comart                │                 │
│              │ │  Variables  package=com.abc.x   │                 │
├──────────────┴─────────────────────────────────┴─────────────────┤
│ 2 tables · 2 templates → 4 files into ~/out/src  [Preview][Dry run][Generate ▶]│
└──────────────────────────────────────────────────────────────────┘
```

Both side panels have a draggable divider and remember their width; both toggle
with a shortcut (<kbd>Ctrl</kbd>+<kbd>B</kbd>, <kbd>Ctrl</kbd>+<kbd>I</kbd>) and
both disappear when nothing is connected.

The rule the layout follows: **selection lives in the explorer, options live in
the work area, the verdict lives in the status bar.** The three never overlap.

## The explorer

![The explorer](screenshots/explorer.png)

Catalog → schema → table, fetched lazily: a schema's tables are read the first
time you expand it and cached after that.

- **The filter box** is a contains match, case-insensitive — jdbgen's rule.
- **The views toggle** shows or hides views. Both forms are fetched either way,
  so flipping it costs no round trip.
- **A checkbox on every row.** The ticked set is what will be generated. It
  survives filtering, the views toggle, collapsing a node and a refresh; only a
  new connection clears it. A schema's own box is three-state and acts on the
  rows **currently on screen**, so it composes with the filter instead of
  fighting it.
- **Right-click** offers *select all shown*, *clear*, *invert*, *open in
  inspector* and *refresh*.

Ticking a row is never the same gesture as selecting it: the checkbox swallows
its own press.

## The inspector

The panel on the right shows the table under the cursor — fetched in the
background, cached per table, so walking the tree costs one round trip each.
Four tabs:

| Tab | What it shows |
|:---|:---|
| Columns | Ordinal, name, type, nullability, default, `PK`*n* and comment |
| Keys | The primary key, **in key order** rather than column order |
| Foreign keys | Both directions — what this table points at, and what points at it |
| Indexes | Name, uniqueness, columns in index order |

This is the same model the templates see, so it doubles as a way to check what a
template will get. Foreign keys and indexes are new in rudbgen; jdbgen showed
neither.

With a template tab active the inspector becomes the **variable palette**
instead: every field the current model offers — table fields, column fields,
foreign keys, indexes, the statements, the decorators, the conditions and your
own custom variables — each with a one-line description, filterable, and
clickable to insert `${…}` at the caret. See [The template
tab](#the-template-tab).

## The Generate tab

![The Generate tab](screenshots/generate.png)

The Generate tab is permanent and is the **only** place a connection's
generation profile is edited. Everything on it is saved to that connection as
you change it — debounced, and only when it actually differs from what was last
written.

- **Template set** — the saved sets, plus `Custom` as soon as the list matches
  none of them. Two sets ship: *Java + MyBatis* and *PHP CodeIgniter*.
  **Save as set…** stores the current list under a name.
- **The template list** — one row per template: a tick, its name, its file, the
  **output name template** that names the file it writes, and buttons to edit or
  remove it. *Add template…* opens a file picker starting in your templates
  directory.

  The output name is itself a template: `${name.suffix.pascal}Model.java`, or
  `${package.replace('.','/')}/${name.suffix.pascal}.java` to write into a
  package tree — the directories are created for you. An absolute path, or one
  that climbs out of the output directory with `..`, is refused.
- **Output directory**, with a chooser.
- **Author** — what `${author}` renders as.
- **Custom variables** — a key/value table with a trailing empty row that
  becomes a real row as soon as you type in it.
- **Apply abbreviations** — the switch that inserts `.abbr` into every `${name}`
  reference automatically. **Rules…** beside it opens the [rules
  editor](#abbreviation-rules); the switch there and the switch here are the
  same value.

The **Edit** glyph on a template row — or a double click on its file — opens
that template in [a tab of its own](#the-template-tab).

## Abbreviation rules

![The abbreviation rules editor](screenshots/abbreviations.png)

An abbreviation rule rewrites a piece of an identifier on its way into a
generated name: `EMP` → `Employee`, `NO` → `Number`. The list is **global**, not
per connection — a shop's vocabulary is a fact about the shop — and lives in
`abbreviations.json`. Open the editor with **Rules…** on the Generate tab, or
from the menu.

Each row is a rule:

| Column | What it means |
|:---|:---|
| **Apply** | Whether the rule takes part in a run. A tick, not a deletion: switching a rule off for one project does not cost you the rule |
| **Whole name** | Whether it replaces the entire identifier rather than one word inside it |
| **Abbreviation** | What to look for |
| **Replacement** | What to put in its place. May be empty, which is how a `TB_` prefix is stripped |

**How a name is rewritten.** A whole-name rule replaces the entire identifier
and ends there. Otherwise the name is split at `_` and `-`, and each word is
looked up on its own, separators kept where they were. Both kinds match
**whatever the case** — this is the one deliberate behavioural difference from
jdbgen, whose word rules only ever matched lower-case segments and therefore
never fired on `TB_USR`. So `album` → `Disc` turns `T_SAMPLE_ALBUM` into
`T_SAMPLE_Disc`.

The last row is always empty and becomes a real one the moment you type in it;
an emptied row is dropped when you save. Beside the **Abbreviation** field of a
whole-name row is a picker offering the tables the explorer has loaded, so a
rule about a table is written against a name that exists rather than one typed
from memory. It appears only while something is connected.

Two rules that are on, are of the same kind, and look for the same thing (case
ignored) would end as one entry in the dictionary, and which of the two survived
would be invisible. **Save** is disabled while that stands, and the offending
rows are outlined. **Cancel** throws the whole draft away — nothing is written
until **Save**.

## The template tab

![The template editor](screenshots/template-editor.png)

A template opens in a tab of its own from the **Edit** glyph on the Generate
tab, from *Open a template…* on the welcome screen, or from the menu. It is the
editor with template-language highlighting on the left, a **live preview** on
the right, and the [variable palette](#the-inspector) in place of the inspector.

- **The preview follows the buffer, not the file.** It renders the text as it
  now stands against the table named in its header — pick another from the
  dropdown — so a change shows before it is saved.
- **Diagnostics** are gutter marks with a message: a parse error stops the
  render and names the line; an **unknown field** — `${nmae}` — is a warning,
  because the engine renders it to nothing and jdbgen's way of finding it was to
  read the generated file. Clicking a diagnostic jumps the caret to it.
- <kbd>Ctrl</kbd>+<kbd>S</kbd> writes the file; the tab carries a dirty marker
  until it does, and closing a dirty tab asks first.
- <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> shows and hides the preview
  half.

![Completion inside a statement](screenshots/completion.png)

<kbd>Ctrl</kbd>+<kbd>Space</kbd> — and typing `${` — offers what may be written
where the caret is: the statement names at the start of one, the option names
inside it, and the fields of the current model. Each entry carries the same
one-line description the palette shows.

## Preview and dry run

Both live on the status bar, beside the arithmetic of the run: *n* tables · *m*
templates → *n*×*m* files into the output directory.

- **Preview** opens a Preview tab and renders **one** pair — pick the table and
  the template from the two dropdowns in its header.
- **Dry run** renders **all** of them without writing anything, and lists what
  would happen: every path, whether it replaces an existing file, and how big it
  would be. Selecting a row shows the text underneath.

A dry run is the cheap way to find out that an output name template collides, or
that a template fails on one particular table.

## Running the generation

**Generate ▶** (<kbd>Ctrl</kbd>+<kbd>G</kbd>) is disabled with a reason in its
tooltip — *no connection*, *no tables ticked*, *no templates ticked*, *no output
directory* — rather than an error box after the click. The first missing thing is
the one it names.

Every template is parsed **before** any file is written, so a template with a
syntax error costs you nothing: the run stops, names the template, which half of
it (body or output name) and the line.

![The overwrite question](screenshots/overwrite.png)

While it runs, a progress dialog shows a bar, a log and **Cancel** — cancellation
takes effect at the next file boundary, so a half-written file is never left
behind. When a file already exists, the default policy asks, with **Overwrite**,
**Skip**, **Overwrite all**, **Skip all** and **Cancel**. The default policy
itself is a setting (`ask`, `overwrite` or `skip`).

![The result summary](screenshots/summary.png)

At the end comes the result summary: every file written, skipped and failed —
with the failing template's line for each failure — the count of unknown-field
warnings, and **Open output directory**.

Files are written atomically, so a crash cannot leave a truncated source file in
your tree.

## Importing from jdbgen

![The import wizard](screenshots/import.png)

**Import from jdbgen…** on the welcome screen — also in the menu — moves a
[jdbgen](https://github.com/xcomart/jdbgen) configuration across in one pass. It
is offered whenever rudbgen can find `config.json` in jdbgen's own configuration
directory, and **Other file…** points it at a copy from another machine.

jdbgen keeps the connection URL, the user name and the password encrypted behind
a master password. rudbgen has none: it asks for jdbgen's **once**, here
(<kbd>Enter</kbd> is *Next*), reads both of jdbgen's encryption formats, and then
puts the passwords where they belong — the OS keychain. The password is not kept
a moment longer than the step it is typed in, and **your jdbgen configuration is
never touched**: the file is opened read-only, so changing your mind leaves you
with a working jdbgen.

The second step is a checklist. Connections, drivers, template sets and
abbreviation rules each have a tick, and under them:

- **Settings** — jdbgen's language and its light/dark choice, off by default.
- **Names that are already taken** — keep both, and what comes from jdbgen is
  added with `(imported)` after its name; or leave the jdbgen entry behind. A
  driver rudbgen already ships a definition for is the common case, and *keep
  both* repoints the imported connections at the imported definition so nothing
  ends up naming a driver that is not there.
- **Worth knowing** — everything the mapping decided that you would otherwise
  have to discover: a stock driver matched onto a built-in, a connection naming
  a driver the file does not define, a keep-alive interval that is not a number,
  a template file found in neither of jdbgen's directories. The note about
  abbreviation rules matching whatever the case is always among them, because it
  is the one behaviour that changes underneath you.

The last step reports what was written. A machine with no usable keychain is not
a failed import: those connections are saved without their password and named
here, and rudbgen asks for the password when you open one.

## Settings

<kbd>Ctrl</kbd>+<kbd>,</kbd> opens Settings: the UI theme and the editor theme
(both with live preview, both importable from a file and extensible from a user
theme directory), the interface language, the fonts, the Java heap the embedded
JVM starts with, the window chrome, and the default overwrite policy. The
language changes live — there is nothing to restart.

Eight languages ship: English, Korean, Japanese, Simplified Chinese, German,
Spanish, French and Russian.

## Keyboard shortcuts

On macOS, read <kbd>Cmd</kbd> for <kbd>Ctrl</kbd>.

| Shortcut | Action |
|:---|:---|
| <kbd>Ctrl</kbd>+<kbd>N</kbd> | New connection |
| <kbd>Ctrl</kbd>+<kbd>G</kbd> | Generate |
| <kbd>Ctrl</kbd>+<kbd>B</kbd> | Show/hide the explorer |
| <kbd>Ctrl</kbd>+<kbd>I</kbd> | Show/hide the inspector |
| <kbd>Ctrl</kbd>+<kbd>,</kbd> | Settings |
| <kbd>Ctrl</kbd>+<kbd>S</kbd> | Save the template tab on top |
| <kbd>Ctrl</kbd>+<kbd>Shift</kbd>+<kbd>P</kbd> | Show/hide the live preview |
| <kbd>Ctrl</kbd>+<kbd>Space</kbd> | Suggest what may be written here |
| <kbd>Esc</kbd> | Dismiss the dialog on top |
| <kbd>Ctrl</kbd>+<kbd>Q</kbd> | Quit |

Preview and Dry run are deliberately unbound: they are one button press away,
and the chords are better spent on the template editor.

---

## Related documentation

- [Template reference](template-reference.md) — the language of the files the
  template list points at.
- [Custom queries](custom-queries.md) — the driver editor's four SQL overrides.
- [Installation](installation.md) — where the settings and templates are kept.
