# Installation

rudbgen is one download per platform. **No JDK is needed**: every release
archive carries a Java runtime built with `jlink`, and rudbgen uses the one next
to its executable.

[← Documentation index](README.md)

- [What is in a download](#what-is-in-a-download)
- [Windows](#windows)
- [macOS](#macos)
- [Linux](#linux)
- [Using your own JDK](#using-your-own-jdk)
- [Where rudbgen keeps your data](#where-rudbgen-keeps-your-data)
- [Passwords and the OS keychain](#passwords-and-the-os-keychain)
- [Updating](#updating)
- [Uninstalling](#uninstalling)

## What is in a download

Every archive is a **tree**, not a single binary, and the tree has to stay
together: the executable finds its Java runtime and its bridge JAR by walking
relative paths from itself.

```
rudbgen-vX.Y.Z-<target>/          Windows (zip and installer) and Linux
├── rudbgen(.exe)
├── lib/rudbgen-bridge.jar        the Java half, loaded into the embedded JVM
├── runtime/                      the jlink Java runtime
├── templates/                    the three shipped templates, for reference
├── sample/sample_h2.db.mv.db     a sample H2 database to try a connection on
└── README.md
                                  Linux also: install.sh, the .desktop file, icons/

rudbgen.app/                      macOS
└── Contents/
    ├── MacOS/rudbgen
    ├── lib/rudbgen-bridge.jar
    ├── runtime/
    ├── Resources/rudbgen.icns, templates/, sample/
    └── Info.plist
```

`templates/` is a **reference copy**. The three shipped templates are also
compiled into the binary and written into your configuration directory the first
time rudbgen starts; editing the copy in the download changes nothing.

`sample/sample_h2.db.mv.db` is jdbgen's sample database, carried over so there is
something to generate from before there is a server to connect to. It holds
`T_SAMPLE_ALBUM` and `T_SAMPLE_MUSIC` in schema `PUBLIC`, both commented, and it
is the table every example in the [template
reference](template-reference.md) is written against. To open it: driver **H2
Embedded**, URL `jdbc:h2:<path>/sample_h2.db` — the path without the `.mv.db`
suffix — and no user name or password. Copy it somewhere writable first; H2
opens an embedded database read-write and the copy in an installed tree may not
be.

Moving `rudbgen.exe` out of its folder produces a program that opens its window
and then fails on the first connection, because `lib/` and `runtime/` are no
longer beside it.

## Windows

Two downloads, the same program inside.

- **`…-setup.exe`** — an installer. It installs into your user profile, so it
  needs no administrator rights and raises no UAC prompt, and it adds a
  Start-menu entry plus an **Apps & features** entry you can uninstall from
  later. This is also what `winget` installs.
- **`….zip`** — the same tree, portable. Unzip it wherever you like and run
  `rudbgen.exe` from inside the folder. Keep the folder together.

Either way the executable is self-signed at best, so SmartScreen may say
"Windows protected your PC"; choose **More info → Run anyway**. A signature from
an untrusted root is a tamper seal, not a reputation — winget does not change
that either.

## macOS

Unpack the `.tar.gz` and drag `rudbgen.app` to Applications.

The bundle is ad-hoc signed rather than notarized, so Gatekeeper quarantines it
on arrival: the first launch needs **right-click → Open** instead of a
double-click. If macOS still refuses (newer versions offer no way through), drop
the quarantine flag and launch normally:

```sh
xattr -r -d com.apple.quarantine /Applications/rudbgen.app
```

On macOS 15 and later there is a second hurdle. The system asks each app
separately for permission to reach the local network, and because the bundle is
only ad-hoc signed it usually never gets the prompt and never appears under
**System Settings → Privacy & Security → Local Network**, which offers no way to
add an app by hand. The permission is then denied silently: connections to a
database on your LAN — a `192.168.x.x` or `10.x.x.x` address, or a `.local` name
— fail with "No route to host", while `localhost` connections work as usual. The
dependable way through is to launch the binary from Terminal, whose execution
context is always allowed on the local network:

```sh
/Applications/rudbgen.app/Contents/MacOS/rudbgen
```

It has to be the executable itself; `open rudbgen.app` hands the launch to
launchd and does not count. If you would rather try to get the prompt back, run
`tccutil reset All com.aihouse.rudbgen`, delete every copy of `rudbgen.app`
(empty the Trash too), reboot, then reinstall and launch — with an ad-hoc
signature it may or may not ask.

## Linux

Unpack the `.tar.gz` and run `./install.sh`. It needs no root:

```sh
tar xzf rudbgen-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
cd rudbgen-vX.Y.Z-x86_64-unknown-linux-gnu
./install.sh
```

It copies the tree to `~/.local/share/rudbgen`, links it from
`~/.local/bin/rudbgen`, and installs the desktop entry and the hicolor icons
under `~/.local/share`. Make sure `~/.local/bin` is on your `PATH`.

The link is a symlink rather than a copy on purpose: the executable resolves
symlinks when it looks for itself, so it still finds `lib/` and `runtime/` in
`~/.local/share/rudbgen`.

Running rudbgen from an unpacked archive without installing works too — the tree
is self-contained.

### Runtime dependencies

The Rust half draws through gpui, which links against the system's
`libxkbcommon` (with its X11 half), `wayland-client` and `fontconfig`. Every
mainstream desktop install already has them; a minimal container does not. On
Debian and Ubuntu:

```sh
sudo apt-get install -y libxkbcommon0 libxkbcommon-x11-0 libwayland-client0 libfontconfig1
```

## Using your own JDK

To use a JDK of your own instead of the bundled runtime, point
`RUDBGEN_JAVA_HOME` at it (Java 17 or newer):

```sh
RUDBGEN_JAVA_HOME=/usr/lib/jvm/java-21-openjdk rudbgen
```

The Java runtime is looked for in this order, and the first candidate that
actually holds a JVM library wins:

1. `runtime/` beside the executable, then `../runtime/` — which is where the
   macOS bundle keeps it,
2. `RUDBGEN_JAVA_HOME`,
3. `JAVA_HOME`,
4. failing all of those, whatever `java` on `PATH` resolves to.

A build from source has no bundled runtime, so it falls through to step 2. Note
the order: a bundled runtime **wins over** `JAVA_HOME`, which is what makes a
download behave the same on a machine with a JDK installed and one without. Set
`RUDBGEN_JAVA_HOME` to overrule it.

Settings has a **JVM heap** field (the `-Xmx` the embedded JVM starts with) but
no path field: the JVM is started once per process and cannot be restarted, so
the runtime is chosen before the window is up.

`RUDBGEN_BRIDGE_JAR` overrides where the bridge JAR is looked for, in the same
spirit; it is a development convenience and a packaged install never needs it.

## Where rudbgen keeps your data

One directory, and nothing else — no registry keys beyond the installer's own
uninstall entry, and no files beside the program.

| Platform | Directory |
|:---|:---|
| Windows | `%APPDATA%\rudbgen\config` |
| macOS | `~/Library/Application Support/rudbgen` |
| Linux | `~/.config/rudbgen` (or `$XDG_CONFIG_HOME/rudbgen`) |

Inside it:

| Entry | What it holds |
|:---|:---|
| `settings.json` | Theme, editor theme, language, fonts, JVM heap, window state, the default overwrite policy |
| `connections.json` | Saved connections, each with its generation profile — template ticks, output directory, author, custom variables |
| `drivers.json` | Driver definitions: class, JARs, URL shape, properties, the four custom queries |
| `template-sets.json` | Named template sets (jdbgen's presets) |
| `abbreviations.json` | The abbreviation rules |
| `templates/` | Your template files. The three shipped ones are written here on first run and **never overwritten afterwards**, so an edit survives an update |
| `drivers/` | JDBC driver JARs downloaded from Maven Central |
| `themes/`, `editor-themes/` | User theme files |
| `known_hosts` | SSH host keys you have accepted |

Every file is JSON, read leniently and written atomically: an unknown field is
ignored rather than fatal, and a crash mid-write cannot leave a half-written
file. They are meant to be editable by hand, and a UTF-8 BOM — which several
Windows editors add on save — is tolerated.

Paths inside `connections.json` are stored **relative to this directory** when
they point below it and absolute otherwise, so a configuration directory can be
copied to another machine with its templates intact.

**Passwords are not in any of these files.** See below.

## Passwords and the OS keychain

Database passwords and SSH key passphrases go into the operating system's own
credential store, under the service name `rudbgen`:

| Platform | Store |
|:---|:---|
| Windows | Credential Manager |
| macOS | Keychain |
| Linux | the freedesktop Secret Service (GNOME Keyring, KWallet, …) |

Each connection has up to two entries — one for the database password, one for
the tunnel's — so that the two can be changed independently.

On a machine with no such store — a headless Linux box with no Secret Service
running — rudbgen starts anyway and behaves as though no password had ever been
saved: you are asked for it each time. It never falls back to writing the
password to disk.

Unlike jdbgen, there is **no master password**. Nothing rudbgen writes is
encrypted with a key of its own, because nothing rudbgen writes is a secret; the
URL and the user name are stored in the clear, and credentials embedded in a URL
are masked out of logs and error messages.

## Updating

rudbgen checks for a new release once per launch, in the background, and says
nothing at all when it cannot reach GitHub — the check is the least important
thing happening at startup. When there is one, a dialog offers to install it.
**Check for updates** in the menu asks the same question on demand and answers
even when the answer is "I could not reach GitHub".

Installing an update downloads the archive built for your exact platform,
verifies it against the checksum the release API reported, unpacks it beside the
installed copy and moves the new tree into the old one's place — the executable,
`lib/` and `runtime/` together, because a new binary beside an old bridge JAR is
a mismatch that only shows up at the first connection. rudbgen then restarts
into the build it just wrote.

Two things it deliberately does not do: it never asks for administrator rights,
and it never touches your configuration directory. A copy you cannot overwrite —
a system package, a read-only mount, a `.app` opened from a disk image — fails
the swap and the dialog offers the release page instead.

On Windows, if you have already opened a database connection the JVM is loaded
and holds `lib/rudbgen-bridge.jar` and the runtime open, so the swap is deferred:
the update is parked beside the installation and applied at the very start of
the next launch, before anything Java has been touched. You see the same flow
either way.

An installed (rather than unzipped) Windows copy also has its recorded version
in *Apps & features* corrected after an update, so `winget list` and
`winget upgrade` stay in step with what is actually on disk. A portable copy has
no such entry and none is created.

Your templates, connections, drivers and themes are untouched by an update.

## Uninstalling

- **Windows, installed** — *Apps & features → rudbgen → Uninstall*, or
  `winget uninstall Xcomart.Rudbgen`.
- **Windows, portable** — delete the folder.
- **macOS** — drag `rudbgen.app` to the Trash.
- **Linux** — `rm -rf ~/.local/share/rudbgen ~/.local/bin/rudbgen` plus the
  desktop entry at `~/.local/share/applications/com.aihouse.rudbgen.desktop`
  and the icons named `rudbgen.*` under `~/.local/share/icons/hicolor`.

None of these removes your configuration directory or your keychain entries.
That is deliberate — some upgrade paths uninstall and reinstall — so to remove
every trace, delete the [configuration
directory](#where-rudbgen-keeps-your-data) yourself and remove the `rudbgen`
entries from your credential store.

---

## Related documentation

- [User interface guide](ui-guide.md) — the first connection, and what the
  window is made of.
- [Template reference](template-reference.md) — the language of the files in
  `templates/`.
- [Custom queries](custom-queries.md) — for a driver whose metadata is wrong.
