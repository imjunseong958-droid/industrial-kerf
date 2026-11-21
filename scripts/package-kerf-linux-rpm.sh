#!/usr/bin/env bash
# The Linux package as an .rpm, for Fedora and what installs rpms as it does, made from
# the folder scripts/package-kerf-linux.sh packs: the same files as the tar.xz and the
# .deb, in /opt/industrial-kerf, which dnf installs for every user and removes again.
#
# It adds what the .deb adds (scripts/package-kerf-linux-deb.sh): the desktop entry in
# /usr/share/applications, the icon in the hicolor theme, and kerf-editor, kerf, kerfd and
# kerf-mcp on PATH as links into /opt/industrial-kerf/bin.
#
# Its Requires are what the folder takes from the system, read out of the binaries: each
# library one names that lib/ does not carry, as rpm names a library (libX11.so.6()(64bit)),
# which is the name every rpm distribution's packages provide it under, and the newest
# version of the C library's symbols any of them asks for (libc.so.6(GLIBC_2.39)(64bit)), so
# that dnf brings what a bare system lacks and refuses one whose C library is too old.
# rpm's own reading of the binaries is turned off: it would name what lib/ carries as
# needed from the system, and offer the package's FFmpeg to everything else installed. What
# the editor opens as it starts rather than names is added: the Vulkan loader and Mesa's
# Vulkan drivers, which it draws with, and the Wayland client library.
#
# Runs on the Ubuntu machine that made the folder, with rpm (apt install rpm) and binutils.
# It has no scriptlets: the desktop's own triggers read the entry and the icons.
# scripts/check-kerf-linux-rpm.sh installs it.
#
#   scripts/package-kerf-linux-rpm.sh path/to/Industrial-Kerf-<version>-linux-x64 [out.rpm]
set -euo pipefail

[ $# -ge 1 ] && [ $# -le 2 ] || { echo "usage: $0 path/to/the/packed/folder [out.rpm]" >&2; exit 2; }
root="$(cd "$(dirname "$0")/.." && pwd)"
stage="$(cd "$1" && pwd -P)"
rpm="${2:-$stage.rpm}"
packaging="$root/apps/kerf-editor/packaging/linux"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")"
prefix=/opt/industrial-kerf
. "$root/scripts/kerf-linux-desktop.sh"

for tool in rpmbuild objdump; do
    command -v "$tool" > /dev/null || { echo "$tool is needed (apt install rpm binutils)" >&2; exit 1; }
done
[ -x "$stage/bin/kerf-editor" ] || { echo "No bin/kerf-editor in $stage" >&2; exit 1; }
[ -d "$stage/icons" ] || { echo "No icons in $stage" >&2; exit 1; }

work="$(mktemp -d)"
trap 'rm -rf -- "$work"' EXIT
tree="$work/tree"
mkdir -p "$tree/opt" "$tree/usr/bin" "$tree/usr/share/applications"

# The folder as it is, but for what is a folder's alone, as in the .deb.
cp -a "$stage" "$tree$prefix"
rm -rf "$tree$prefix/install-desktop-entry.sh" "$tree$prefix/icons" "$tree$prefix/README.txt"
cat > "$tree$prefix/README.txt" <<README
Industrial Kerf $version for Linux, as its .rpm installs it.

It is among your applications, opens kerf:// project links, and has kerf-editor, kerf,
kerfd and kerf-mcp on PATH. This folder carries its own FFmpeg; what it takes from the
system (system-libraries.txt) the package's Requires name.

  sudo dnf remove industrial-kerf

takes all of it away. Your projects and settings are yours and stay.
README

for tool in kerf-editor kerf kerfd kerf-mcp; do
    [ -x "$tree$prefix/bin/$tool" ] || { echo "No bin/$tool in $stage" >&2; exit 1; }
    ln -s "$prefix/bin/$tool" "$tree/usr/bin/$tool"
done
install -m 644 "$packaging/industrial-kerf.desktop" "$tree/usr/share/applications/industrial-kerf.desktop"
kerf_check_entry "$tree/usr/share/applications/industrial-kerf.desktop" "$prefix/bin/kerf-editor"
for size in $kerf_icon_sizes; do
    install -D -m 644 "$stage/icons/industrial-kerf-$size.png" \
        "$tree/usr/share/icons/hicolor/${size}x${size}/apps/industrial-kerf.png"
done
chmod -R u+rwX,go+rX,go-w "$tree"

# What each binary names, what lib/ carries of that, and the newest C library symbols any
# asks for. A library carried is the package's own, by the RPATH each binary has.
carried="$work/carried"
find "$tree$prefix" \( -type f -o -type l \) -name '*.so*' -printf '%f\n' | sort -u > "$carried"
needed="$work/needed"
: > "$needed"
: > "$work/glibc"
while IFS= read -r -d '' file; do
    head -c 4 "$file" | grep -q 'ELF' || continue
    objdump -p "$file" 2> /dev/null | awk '$1 == "NEEDED" {print $2}' >> "$needed"
    # Versions of the C library's own symbols; its private ones are no promise of any version.
    objdump -T "$file" 2> /dev/null | grep -o 'GLIBC_[0-9][0-9.]*' >> "$work/glibc" || true
done < <(find "$tree$prefix" -type f -print0)
requires="$(sort -u "$needed" | grep -vxF -f "$carried" | grep -v '^ld-linux' | sed 's/$/()(64bit)/' || true)"
grep -q '^libc\.so\.6()(64bit)$' <<< "$requires" || { echo "The binaries name no C library: $requires" >&2; exit 1; }
glibc="$(sort -u -V "$work/glibc" | tail -n 1)"
[ -n "$glibc" ] || { echo "The binaries ask for no version of the C library's symbols" >&2; exit 1; }
requires+=$'\n'"libc.so.6($glibc)(64bit)"
# Opened as the editor starts, not named, so not among what the binaries name.
requires+=$'\n'"libvulkan.so.1()(64bit)"$'\n'"mesa-vulkan-drivers"$'\n'"libwayland-client.so.0()(64bit)"

spec="$work/industrial-kerf.spec"
{
    cat <<SPEC
Name: industrial-kerf
Version: ${version//-/\~}
Release: 1
Summary: Video editor, with an agent editing beside you
License: GPL-3.0-only AND MIT AND Apache-2.0
URL: https://github.com/imjunseong958-droid/industrial-kerf
BuildArch: x86_64
AutoReqProv: no
Requires: hicolor-icon-theme
SPEC
    sed 's/^/Requires: /' <<< "$requires"
    cat <<SPEC

# The files are as they were built: nothing is stripped, compiled or given build ids here.
%global __os_install_post %{nil}
%global debug_package %{nil}
%define _build_id_links none

%description
Industrial Kerf is a native video editor whose project is a text document. An
agent edits it beside you, through proposals you take or refuse, and people
share it through Git.

This package carries a GPL build of FFmpeg, in $prefix, and
takes the C library, X11 or Wayland, xkbcommon, a Vulkan driver and ALSA from
the system. It adds Kerf to the applications, opens kerf:// project links in
it, and puts kerf-editor, kerf, kerfd and kerf-mcp on PATH.

%install
# What was laid out above, whole: rpmbuild clears its build root before this.
cp -a "$tree/." "%{buildroot}/"

%files
%defattr(-,root,root,-)
$prefix
/usr/bin/kerf-editor
/usr/bin/kerf
/usr/bin/kerfd
/usr/bin/kerf-mcp
/usr/share/applications/industrial-kerf.desktop
SPEC
    for size in $kerf_icon_sizes; do
        echo "/usr/share/icons/hicolor/${size}x${size}/apps/industrial-kerf.png"
    done
} > "$spec"

if ! rpmbuild -bb --quiet --target x86_64 --buildroot "$work/buildroot" \
    --define "_topdir $work/rpmbuild" --define "_rpmdir $work/out" \
    --define "_binary_payload w6.xzdio" "$spec" > "$work/rpmbuild.log" 2>&1; then
    cat "$work/rpmbuild.log" >&2
    echo "rpmbuild could not make the package" >&2
    exit 1
fi
built="$(find "$work/out" -name '*.rpm' | head -n 1)"
[ -n "$built" ] || { cat "$work/rpmbuild.log" >&2; echo "rpmbuild made no package" >&2; exit 1; }
rm -f "$rpm"
mv "$built" "$rpm"
(cd "$(dirname "$rpm")" && sha256sum "$(basename "$rpm")" > "$(basename "$rpm").sha256")
echo "Requires: $(paste -sd ' ' <<< "$requires")" >&2
echo "$rpm"
