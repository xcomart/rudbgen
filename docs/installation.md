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
├── drivers/h2-2.5.252.jar        the H2 JDBC driver, so that sample opens offline
└── README.md
                                  Linux also: install.sh, the .desktop file, icons/

rudbgen.app/                      macOS
└── Contents/
    ├── MacOS/rudbgen
    ├── lib/rudbgen-bridge.jar
    ├── runtime/
    ├── Resources/rudbgen.icns, templates/, sample/, drivers/
    └── Info.plist
```

`templates/` is a **reference copy**. The three shipped templates are also
compiled into the binary and written into your configuration directory the first
time rudbgen starts; editing the copy in the download changes nothing.

`sample/sample_h2.db.mv.db` is jdbgen's sample database, carried over so there is
something to generate from before there is a server to connect to. It holds
`T_SAMPLE_ALBUM` and `T_SAMPLE_MUSIC` in schema `PUBLIC`, both commented, and it
is the table every example in the [template
reference](template-reference.md) is written against.

You do not have to set it up. The first time rudbgen starts — precisely, the
first start that finds no `connections.json` in your configuration directory —
it copies this file next to that configuration and adds a connection called
**Sample H2 Embedded** that opens the copy. The copy is what the connection
names because H2 opens an embedded database read-write and takes a lock on it,
which an installed tree under `C:\Program Files`, `/opt` or a signed `.app`
bundle will not allow. Neither the copy nor the connection is ever made twice: a
`sample_h2.db.mv.db` already beside your configuration is left exactly as it is,
and a `connections.json` that exists — even one listing nothing, because you
deleted the sample — is never seeded again.

`drivers/h2-2.5.252.jar` is installed by that same first run: the JAR is copied
into `drivers/` in your configuration directory and put on the **H2 Embedded**
driver's class path, so the sample connection opens straight away with no
network. It is the only driver rudbgen ships. Every other one is downloaded from
Maven Central through **Drivers…** in the connection dialog, because their
licenses do not allow anyone else to redistribute them; H2 is dual licensed
under MPL 2.0 and EPL 1.0, which do. A JAR of the same name already in your configuration — one
you downloaded yourself — is left alone, and the file name carries the version,
so it is the same artifact either way.

To open a second copy by hand, or to open one after deleting the connection:
driver **H2 Embedded**, URL `jdbc:h2:<path>/sample_h2.db` — the path without the
`.mv.db` suffix, and with `/` separators on Windows too — and no user name or
password. Copy the file somewhere writable first, for the reason above.

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

Both come as `x86_64-pc-windows-msvc` for x64 PCs and `aarch64-pc-windows-msvc`
for ARM64 ones; the x64 build runs on ARM64 too, emulated. winget picks the
native one.

Either way the executable is self-signed at best, so SmartScreen may say
"Windows protected your PC"; choose **More info → Run anyway**. A signature from
an untrusted root is a tamper seal, not a reputation — winget does not change
that either.

## macOS

On a Mac, Apple Silicon or Intel, the project's Homebrew tap installs rudbgen
with one command:

```sh
brew install --cask xcomart/tap/rudbgen
```

Homebrew checks the download against the SHA256 recorded in the cask, moves
`rudbgen.app` into `/Applications` and clears the quarantine flag described
below, so the first launch opens normally. The release workflow updates the
cask as soon as a release is published. rudbgen [updates itself](#updating),
so `brew upgrade` skips it unless run with `--greedy`. The cask picks the
`aarch64-apple-darwin` or the `x86_64-apple-darwin` build to match the Mac.

To install by hand instead, unpack the `.tar.gz` and drag `rudbgen.app` to
Applications.

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

Unpack the `.tar.gz` and run `./install.sh`. It needs no root. Take the
`x86_64` or the `aarch64` archive, whichever `uname -m` names:

```sh
tar xzf rudbgen-vX.Y.Z-x86_64-unknown-linux-gnu.tar.gz
cd rudbgen-vX.Y.Z-x86_64-unknown-linux-gnu
./install.sh
```

It copies the tree to `~/.local/share/rudbgen`, links it from
`~/.local/bin/rudbgen`, and installs the desktop entry and the hicolor icons
under `~/.local/share`. The desktop entry is written with the link's absolute
path, so the launcher in the application menu works whether or not
`~/.local/bin` is on your `PATH`; add it to start rudbgen from a shell too.

The binary needs glibc 2.35 or newer: Ubuntu 22.04, Debian 12 and later, or a
distribution built on them, MX Linux 23 among them.

The link is a symlink rather than a copy on purpose: the executable resolves
symlinks when it looks for itself, so it still finds `lib/` and `runtime/` in
`~/.local/share/rudbgen`.

Running rudbgen from an unpacked archive without installing works too — the tree
is self-contained.

### AppImage

Each release also carries an AppImage,
`rudbgen-vX.Y.Z-<arch>-unknown-linux-gnu.AppImage`: the same tree in one file
that runs where it is, with nothing to install:

```sh
chmod +x rudbgen-vX.Y.Z-x86_64-unknown-linux-gnu.AppImage
./rudbgen-vX.Y.Z-x86_64-unknown-linux-gnu.AppImage
```

It needs the same glibc and system libraries as the archive, plus FUSE to
mount itself: the `fuse3` package, which desktop installs of those
distributions include. Without FUSE, start it with
`--appimage-extract-and-run`. The image adds no launcher to the application
menu; a tool such as Gear Lever or AppImageLauncher does that.

### Runtime dependencies

The Rust half draws through gpui. xkbcommon, which handles the keyboard, is
built into the binary; what it still needs from the system is libxcb, an EGL or
Vulkan loader and, in a Wayland session, `wayland-client`. Every mainstream
desktop install already has them; a minimal container does not. On Debian and
Ubuntu:

```sh
sudo apt-get install -y libxcb1 libegl1 libegl-mesa0 libwayland-client0
```

`install.sh` checks for them when it runs, together with glibc, and prints the
command that installs whatever is missing with apt, dnf, pacman or zypper.

## Using your own JDK

To use a JDK of your own where there is no bundled runtime, point
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
the order: a bundled runtime **wins over** both `RUDBGEN_JAVA_HOME` and
`JAVA_HOME`, which is what makes a download behave the same on a machine with a
JDK installed and one without. Neither variable overrules it.

Settings has a **Java heap** field (the `-Xmx` the embedded JVM starts with) and
an **Extra arguments** field, but no path field: the JVM is started once per
process, at the first connection, and cannot be restarted, so the runtime comes
from the environment rudbgen was launched with.

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
| `settings.json` | Theme, editor theme, language, fonts, JVM heap and extra arguments, window state, the default overwrite policy, the release an update dialog was told to ignore |
| `connections.json` | Saved connections, each with its generation profile — template ticks, output directory, author, custom variables |
| `drivers.json` | Driver definitions: class, JARs, URL shape, properties, the four custom queries |
| `template-sets.json` | Named template sets (jdbgen's presets) |
| `abbreviations.json` | The abbreviation rules |
| `templates/` | Your template files. The three shipped ones are written here on first run and **never overwritten afterwards**, so an edit survives an update |
| `drivers/` | JDBC driver JARs: the bundled H2 one, copied here on first run, and whatever you download from Maven Central |
| `dialects/` | Dialect profiles — the shipped ones and your own — and their images; `shipped.json` records what rudbgen wrote there (see [the UI guide](ui-guide.md#dialect-profiles)) |
| `themes/`, `editor-themes/` | User theme files |
| `known_hosts` | SSH host keys you have accepted |
| `sample_h2.db.mv.db` | The copy of the sample database the first run made, which **Sample H2 Embedded** opens |

Every `.json` file is read leniently and written atomically: an unknown field is
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
the tunnel's — so that the two can be changed independently. Untick **Remember
password** in the connection dialog and a connection keeps neither: nothing is
written to the store for it, any entry it had is removed, and the window asks
for the password every time it opens.

A store that refuses to save a password — locked, or denying access — does not
keep you from connecting: the connection is saved, opens with the password you
typed, and the status bar says the password was not kept.

A Secret Service with no default keyring yet — Xfce on MX Linux, for one, never
creates the login keyring — gets one the first time a password is saved: the
service asks for the new keyring's password in a prompt of its own, and the
password is saved into it once you answer.

On a machine with no such store — a headless Linux box with no Secret Service
running — rudbgen starts anyway and behaves as though no password had ever been
saved: when the database refuses a connection without one, you are asked for it.
It never falls back to writing the password to disk.

Unlike jdbgen, there is **no master password**. Nothing rudbgen writes is
encrypted with a key of its own, because nothing rudbgen writes is a secret; the
URL and the user name are stored in the clear, and credentials embedded in a URL
are masked out of logs and error messages.

Coming from jdbgen, the one place its master password is still needed is the
import wizard, which asks for it once and then moves the passwords it opened
into the keychain — see [Importing from
jdbgen](ui-guide.md#importing-from-jdbgen).

## Updating

rudbgen checks for a new release once per launch, in the background, and says
nothing at all when it cannot reach GitHub — the check is the least important
thing happening at startup. When there is one, a dialog shows the release notes
and offers **Update**, **Ignore this version** and **Cancel**; an ignored
version is not announced at startup again, though a later one is. **Check for
updates** in the menu asks the same question on demand, regardless of an
ignored version, and answers even when the answer is "I could not reach
GitHub".

Installing an update downloads the archive built for your exact platform,
verifies it against the checksum the release API reported, unpacks it beside the
installed copy and moves the new tree into the old one's place — the executable,
`lib/` and `runtime/` together, because a new binary beside an old bridge JAR is
a mismatch that only shows up at the first connection. rudbgen then restarts
into the build it just wrote. A copy started from an AppImage downloads the
release's AppImage instead and renames it over the image file, so keep that
file somewhere you can write to.

Two things it deliberately does not do: it never asks for administrator rights,
and it never touches your configuration directory. A copy you cannot overwrite —
a system package, a read-only mount, a `.app` opened from a disk image — fails
the swap and the dialog offers the release page instead.

On Windows, if you have already opened a database connection (or probed a
driver) the JVM is loaded and holds `lib/rudbgen-bridge.jar` and the runtime
open, so the swap is deferred: the update is parked beside the installation and
applied at the very start of the next launch, before anything Java has been
touched. You see the same flow either way.

An installed (rather than unzipped) Windows copy also has its recorded version
in *Apps & features* corrected after an update, so `winget list` and
`winget upgrade` stay in step with what is actually on disk. A portable copy has
no such entry and none is created.

Your templates, connections, drivers and themes are untouched by an update.

## Uninstalling

- **Windows, installed** — *Apps & features → rudbgen → Uninstall*, or
  `winget uninstall Xcomart.Rudbgen`.
- **Windows, portable** — delete the folder.
- **macOS** — drag `rudbgen.app` to the Trash, or
  `brew uninstall --cask rudbgen` for a copy Homebrew installed. Adding
  `--zap` also removes the configuration directory, though not the keychain
  entries.
- **Linux** — `rm -rf ~/.local/share/rudbgen ~/.local/bin/rudbgen` plus the
  desktop entry at `~/.local/share/applications/com.aihouse.rudbgen.desktop`
  and the icons named `rudbgen.*` under `~/.local/share/icons/hicolor`.

Apart from `--zap`, none of these removes your configuration directory, and
none removes your keychain entries. That is deliberate — some upgrade paths uninstall and reinstall — so to remove
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
