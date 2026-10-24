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

# Exec= in a desktop entry takes no shell variables, so $HOME cannot be
# written there; put the absolute path of the link in at install time instead.
# The launcher then works whether or not ~/.local/bin is on PATH. Only the
# binary name is replaced, never the whole line.
install -d "$prefix/applications"
sed "s|^Exec=rudbgen|Exec=$bindir/rudbgen|" "$here/com.aihouse.rudbgen.desktop" \
    > "$prefix/applications/com.aihouse.rudbgen.desktop"
chmod 644 "$prefix/applications/com.aihouse.rudbgen.desktop"
install -Dm644 "$here/icons/rudbgen-128.png" "$prefix/icons/hicolor/128x128/apps/rudbgen.png"
install -Dm644 "$here/icons/rudbgen-256.png" "$prefix/icons/hicolor/256x256/apps/rudbgen.png"
install -Dm644 "$here/icons/rudbgen.svg" "$prefix/icons/hicolor/scalable/apps/rudbgen.svg"

# Refresh caches when the tools are around; harmless to skip.
command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database "$prefix/applications" || true
command -v gtk-update-icon-cache  >/dev/null 2>&1 && gtk-update-icon-cache -q "$prefix/icons/hicolor" || true

echo "installed rudbgen to $appdir, linked from $bindir"
echo "  the desktop launcher works as is; add $bindir to PATH to run it from a shell"

# What rudbgen needs from the system, checked here rather than left to fail at
# the first launch. It is linked against libxcb and both halves of xkbcommon,
# so the loader will not start it while any of them is missing; it draws
# through Vulkan or, without it, EGL, loaded at run time; and in a Wayland
# session it loads the Wayland client library. Nothing is installed from here,
# since that would need root: what is missing is listed together with the
# command that installs it. The glibc floor is the one the release build is
# checked against in .github/workflows/release.yml.
glibc_floor=2.35

ldconfig_bin="$(command -v ldconfig 2>/dev/null || true)"
for candidate in /sbin/ldconfig /usr/sbin/ldconfig; do
    if [ -z "$ldconfig_bin" ] && [ -x "$candidate" ]; then
        ldconfig_bin="$candidate"
    fi
done

has_lib() {
    if [ -n "$ldconfig_bin" ] &&
        "$ldconfig_bin" -p 2>/dev/null | grep -q "^[[:space:]]*$1 (libc6,x86-64"; then
        return 0
    fi
    for dir in /usr/lib/x86_64-linux-gnu /lib/x86_64-linux-gnu /usr/lib64 /lib64 /usr/lib /lib; do
        if [ -e "$dir/$1" ]; then
            return 0
        fi
    done
    return 1
}

if command -v apt-get >/dev/null 2>&1; then
    manager=apt
elif command -v dnf >/dev/null 2>&1; then
    manager=dnf
elif command -v pacman >/dev/null 2>&1; then
    manager=pacman
elif command -v zypper >/dev/null 2>&1; then
    manager=zypper
else
    manager=
fi

# The package that provides each library under each package manager. EGL is
# asked for together with Mesa's implementation of it: the loader alone draws
# nothing.
package_for() {
    case "$manager:$1" in
        apt:libxcb.so.1) echo libxcb1 ;;
        apt:libxkbcommon.so.0) echo libxkbcommon0 ;;
        apt:libxkbcommon-x11.so.0) echo libxkbcommon-x11-0 ;;
        apt:libEGL.so.1) echo libegl1 libegl-mesa0 ;;
        apt:libwayland-client.so.0) echo libwayland-client0 ;;
        dnf:libxcb.so.1) echo libxcb ;;
        dnf:libxkbcommon.so.0) echo libxkbcommon ;;
        dnf:libxkbcommon-x11.so.0) echo libxkbcommon-x11 ;;
        dnf:libEGL.so.1) echo mesa-libEGL ;;
        dnf:libwayland-client.so.0) echo libwayland-client ;;
        pacman:libxcb.so.1) echo libxcb ;;
        pacman:libxkbcommon.so.0) echo libxkbcommon ;;
        pacman:libxkbcommon-x11.so.0) echo libxkbcommon-x11 ;;
        pacman:libEGL.so.1) echo mesa ;;
        pacman:libwayland-client.so.0) echo wayland ;;
        zypper:libxcb.so.1) echo libxcb1 ;;
        zypper:libxkbcommon.so.0) echo libxkbcommon0 ;;
        zypper:libxkbcommon-x11.so.0) echo libxkbcommon-x11-0 ;;
        zypper:libEGL.so.1) echo Mesa-libEGL1 ;;
        zypper:libwayland-client.so.0) echo libwayland-client0 ;;
    esac
}

missing=
for lib in libxcb.so.1 libxkbcommon.so.0 libxkbcommon-x11.so.0; do
    has_lib "$lib" || missing="$missing $lib"
done
if ! has_lib libvulkan.so.1 && ! has_lib libEGL.so.1; then
    missing="$missing libEGL.so.1"
fi
if [ -n "${WAYLAND_DISPLAY:-}" ] && ! has_lib libwayland-client.so.0; then
    missing="$missing libwayland-client.so.0"
fi

glibc="$(getconf GNU_LIBC_VERSION 2>/dev/null | sed 's/^glibc //' || true)"
old_glibc=
if [ -z "$glibc" ]; then
    old_glibc="no glibc was found"
elif [ "$(printf '%s\n' "$glibc_floor" "$glibc" | sort -V | head -n 1)" != "$glibc_floor" ]; then
    old_glibc="this system has glibc $glibc"
fi

if [ -z "$missing" ] && [ -z "$old_glibc" ]; then
    echo "  runtime check: glibc $glibc and every library rudbgen needs are present"
fi

if [ -n "$old_glibc" ]; then
    echo
    echo "warning: rudbgen needs glibc $glibc_floor or newer, and $old_glibc."
    echo "  It will not start here; it runs on Ubuntu 22.04, Debian 12 and later, or a"
    echo "  distribution built on them."
fi

if [ -n "$missing" ]; then
    echo
    echo "warning: rudbgen needs libraries this system does not have:"
    for lib in $missing; do
        echo "  $lib"
    done
    packages=
    for lib in $missing; do
        packages="$packages $(package_for "$lib")"
    done
    case "$manager" in
        apt) echo "install them with:"; echo "  sudo apt install$packages" ;;
        dnf) echo "install them with:"; echo "  sudo dnf install$packages" ;;
        pacman) echo "install them with:"; echo "  sudo pacman -S$packages" ;;
        zypper) echo "install them with:"; echo "  sudo zypper install$packages" ;;
        *) echo "install the packages that provide them with your distribution's package manager." ;;
    esac
fi
