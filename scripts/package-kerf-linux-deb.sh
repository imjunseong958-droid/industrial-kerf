#!/usr/bin/env bash
# The Linux package as a .deb, for Debian 13, Ubuntu 24.04 and what comes from them, made
# from the folder scripts/package-kerf-linux.sh packs: the same files as the tar.xz, in
# /opt/industrial-kerf, which apt installs for every user and removes again.
#
# A folder registers nothing of itself; a .deb does. It puts the desktop entry
# (apps/kerf-qt/packaging/linux/industrial-kerf.desktop) in /usr/share/applications, so Kerf
# is among the applications and opens kerf:// review links, its icon in the hicolor theme,
# and kerf-qt, kerf, kerfd and kerf-mcp on PATH, as links into /opt/industrial-kerf/bin:
# each finds the rest beside the file it really is. ffmpeg and ffprobe stay off PATH, where
# the distribution's own may be.
#
# Its Depends are what the folder takes from the system (system-libraries.txt), as the
# packages of this machine that provide each library a binary names, with the versions
# their symbols ask for: dpkg-shlibdeps reads them out of the binaries. So apt brings what
# a bare system lacks, and refuses a system whose C library is too old rather than leave
# the editor not to start. What it loads only when first used is in Recommends: OpenSSL for
# Qt's TLS and the Vulkan loader for the GPU.
#
# Runs on the Ubuntu machine that made the folder, with dpkg-dev. It has no maintainer
# scripts: the desktop's own triggers read the entry and the icons.
# scripts/check-kerf-linux-deb.sh installs it.
#
#   scripts/package-kerf-linux-deb.sh path/to/Industrial-Kerf-<version>-linux-x64 [out.deb]
set -euo pipefail

[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: $0 path/to/the/packed/folder [out.deb]" >&2; exit 2; }
root="$(cd "$(dirname "$0")/.." && pwd)"
stage="$(cd "$1" && pwd -P)"
deb="${2:-$stage.deb}"
packaging="$root/apps/kerf-qt/packaging/linux"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")"
prefix=/opt/industrial-kerf
. "$root/scripts/kerf-linux-desktop.sh"

for tool in dpkg-deb dpkg-shlibdeps; do
    command -v "$tool" > /dev/null || { echo "$tool is needed (apt install dpkg-dev)" >&2; exit 1; }
done
[ -x "$stage/bin/kerf-qt" ] || { echo "No bin/kerf-qt in $stage" >&2; exit 1; }
[ -x "$stage/install-desktop-entry.sh" ] || { echo "No install-desktop-entry.sh in $stage" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
tree="$work/tree"
mkdir -p "$tree/DEBIAN" "$tree/opt" "$tree/usr/bin" "$tree/usr/share/applications" "$tree/usr/share/doc/industrial-kerf"

# The folder as it is, but for what is a folder's alone: the script that registers it for
# one user, the icons that script installs, and a README that says to unpack it anywhere.
cp -a "$stage" "$tree$prefix"
rm -rf "$tree$prefix/install-desktop-entry.sh" "$tree$prefix/icons" "$tree$prefix/README.txt"
cat > "$tree$prefix/README.txt" <<EOF
Industrial Kerf $version for Linux, as its .deb installs it.

It is among your applications, opens kerf:// review links, and has kerf-qt, kerf, kerfd
and kerf-mcp on PATH. This folder carries its own Qt and FFmpeg; what it takes from the
system (system-libraries.txt) the package's Depends name.

  sudo apt remove industrial-kerf

takes all of it away. Your projects and settings are yours and stay.
EOF

for tool in kerf-qt kerf kerfd kerf-mcp; do
    [ -x "$tree$prefix/bin/$tool" ] || { echo "No bin/$tool in $stage" >&2; exit 1; }
    ln -s "$prefix/bin/$tool" "$tree/usr/bin/$tool"
done

# The entry is the one install-desktop-entry.sh writes for a user, but for where the editor
# is: the two may not drift apart.
install -m 644 "$packaging/industrial-kerf.desktop" "$tree/usr/share/applications/industrial-kerf.desktop"
HOME="$work/home" XDG_DATA_HOME="$work/home/data" XDG_CONFIG_HOME="$work/home/config" \
    "$stage/install-desktop-entry.sh" > /dev/null
if ! diff <(grep -v -e '^Exec=' -e '^TryExec=' "$work/home/data/applications/industrial-kerf.desktop") \
    <(grep -v -e '^Exec=' -e '^TryExec=' "$packaging/industrial-kerf.desktop") >&2; then
    echo "industrial-kerf.desktop and the entry install-desktop-entry.sh writes differ in more than where the editor is" >&2
    exit 1
fi
kerf_check_entry "$tree/usr/share/applications/industrial-kerf.desktop" "$prefix/bin/kerf-qt"
for size in $kerf_icon_sizes; do
    install -D -m 644 "$stage/icons/industrial-kerf-$size.png" \
        "$tree/usr/share/icons/hicolor/${size}x${size}/apps/industrial-kerf.png"
done

cat > "$tree/usr/share/doc/industrial-kerf/copyright" <<EOF
Industrial Kerf $version
Source: https://github.com/imjunseong958-droid/industrial-kerf

Industrial Kerf's own code is under the MIT licence ($prefix/LICENSE.txt). This package
as a whole is under the GNU General Public License version 3 ($prefix/COPYING-GPLv3.txt;
on Debian also /usr/share/common-licenses/GPL-3), as it joins that code with a GPL build
of FFmpeg. It also carries Qt 6 under the LGPL version 3, and libraries from Ubuntu under
their own terms.

$prefix/THIRD-PARTY.txt says which licence covers which part, and
$prefix/SOURCE.txt where the complete source is.
EOF

# Readable by everyone, writable by root alone, whatever the build machine's umask was.
chmod -R u+rwX,go+rX,go-w "$tree"

# What provides each library a binary names and the package does not carry. The folder has
# a DEBIAN beside it so that dpkg-shlibdeps knows where the package's root is, and with it
# what \$ORIGIN in each RPATH means: what lib/ carries is then the package's own, and asks
# for no other.
mkdir "$work/debian"
: > "$work/debian/control"
binaries=()
while IFS= read -r -d '' file; do
    head -c 4 "$file" | grep -q 'ELF' && binaries+=("${file#"$work"/}")
done < <(find "$tree$prefix" -type f -print0)
if ! shlibs="$(cd "$work" && dpkg-shlibdeps -O "${binaries[@]}" 2> "$work/shlibdeps.log")"; then
    grep -v 'warning:' "$work/shlibdeps.log" >&2
    echo "dpkg-shlibdeps could not say what the package depends on" >&2
    exit 1
fi
# FFmpeg's libavcodec, as BtbN builds it, has glibc's resolver built in, which names
# private symbols of glibc (__resolv_context_get, __res_iclose and others). For those
# dpkg-shlibdeps allows this machine's glibc alone (libc6 >> 2.39, << 2.40), which would
# refuse every other system. Only the oldest glibc it asks for is kept: the package is
# installed and run on a newer one, Debian 13's, before it is published (package.yml), and
# again as published (verify-release.yml).
depends="$(sed -e 's/^shlibs:Depends=//' -e 's/, /\n/g' <<< "$shlibs" | grep -Ev '^libc6 \((<<|>>) ' | paste -sd ',' | sed 's/,/, /g')"
grep -q 'libc6 (>= ' <<< "$depends" || { echo "dpkg-shlibdeps named no C library: $shlibs" >&2; exit 1; }

# hicolor-icon-theme has the theme's index, without which no desktop looks in its folders.
cat > "$tree/DEBIAN/control" <<EOF
Package: industrial-kerf
Version: ${version//-/\~}
Architecture: amd64
Maintainer: Industrial Kerf <imjunseong958-droid@users.noreply.github.com>
Installed-Size: $(du -sk --exclude=DEBIAN "$tree" | cut -f1)
Depends: $depends, hicolor-icon-theme
Recommends: libssl3t64, libvulkan1
Section: video
Priority: optional
Homepage: https://github.com/imjunseong958-droid/industrial-kerf
Description: Video editor, with an agent editing beside you
 Industrial Kerf is a native video editor whose project is a text document. An
 agent edits it beside you, through proposals you take or refuse, and people
 share it through Git.
 .
 This package carries its own Qt and a GPL build of FFmpeg, in
 $prefix, and takes the C library, X11, OpenGL, fontconfig,
 D-Bus, GLib and ALSA from the system. It adds Kerf to the applications, opens
 kerf:// review links in it, and puts kerf-qt, kerf, kerfd and kerf-mcp on
 PATH.
EOF
(cd "$tree" && find . -type f ! -path './DEBIAN/*' -printf '%P\0' | LC_ALL=C sort -z | xargs -0 md5sum > DEBIAN/md5sums)

rm -f "$deb"
dpkg-deb --root-owner-group -Zxz --build "$tree" "$deb" > /dev/null
(cd "$(dirname "$deb")" && sha256sum "$(basename "$deb")" > "$(basename "$deb").sha256")
echo "Depends: $depends, hicolor-icon-theme" >&2
echo "$deb"
