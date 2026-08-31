#!/bin/sh
# Installs rudbgen for the current user: the program tree, a desktop entry and
# icons. Run from the unpacked release directory. No root required.
set -eu

prefix="${XDG_DATA_HOME:-$HOME/.local/share}"
bindir="$HOME/.local/bin"
appdir="$prefix/rudbgen"

here="$(cd "$(dirname "$0")" && pwd)"

# rudbgen is not a single file: it looks for the bridge JAR at lib/ and the
# bundled Java runtime at runtime/, both relative to the executable, so the
# whole tree has to move together. An earlier install is removed rather than
# copied over, or files left behind by its runtime would still be found by the
# new one.
rm -rf "$appdir"
mkdir -p "$appdir"
cp -R "$here/rudbgen" "$here/lib" "$here/runtime" "$appdir/"
chmod 755 "$appdir/rudbgen"

# templates/ is a reference copy: the three built-in templates are compiled
# into the binary and written into the configuration directory on first run
# (architecture.md 5). sample/ is not — the program reads it. A first run with
# no connections.json copies sample/sample_h2.db.mv.db next to the
# configuration, where H2 may open it read-write, and seeds the connection that
# opens the copy; without this directory beside the binary that first run has
# nothing to seed. drivers/ is read by the same first run: it holds the bundled
# H2 driver JAR, which is copied into the configuration and put on the
# h2-embedded class path, and without it the seeded connection cannot open until
# the driver manager has fetched the JAR from Maven Central. All three are
# installed because a tarball the user deletes after running this script should
# not take them with it.
for extra in templates sample drivers; do
    [ -d "$here/$extra" ] && cp -R "$here/$extra" "$appdir/" || true
done

# A link, not a copy: current_exe() resolves symlinks, so the binary still sees
# $appdir as its own directory and finds lib/ and runtime/ beside it.
mkdir -p "$bindir"
ln -sf "$appdir/rudbgen" "$bindir/rudbgen"

install -Dm644 "$here/com.aihouse.rudbgen.desktop" "$prefix/applications/com.aihouse.rudbgen.desktop"
install -Dm644 "$here/icons/rudbgen-128.png" "$prefix/icons/hicolor/128x128/apps/rudbgen.png"
install -Dm644 "$here/icons/rudbgen-256.png" "$prefix/icons/hicolor/256x256/apps/rudbgen.png"
install -Dm644 "$here/icons/rudbgen.svg" "$prefix/icons/hicolor/scalable/apps/rudbgen.svg"

# Refresh caches when the tools are around; harmless to skip.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/applications" || true
command -v gtk-update-icon-cache  >/dev/null 2>&1 && gtk-update-icon-cache -q "$prefix/icons/hicolor" || true

echo "installed rudbgen to $appdir, linked from $bindir (make sure it is on your PATH)"
