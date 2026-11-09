#!/usr/bin/env bash
# The complete source of what scripts/package-kerf-macos.sh packs, as one tar to
# publish beside the app: the app carries Homebrew's GPL build of FFmpeg, so it has
# to be offered with the source of all of it. Kerf at this commit with its Rust
# crates, and the source of each Homebrew formula the app has files of
# (target/package-macos/bundled-formulae.txt), as Homebrew fetches it to build that
# formula, with the formula itself, which says how it was built.
#
# Runs on the Mac that made the app, after it.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
version="$(sed -n '/^\[workspace.package\]/,/^\[/s/^version = "\(.*\)"/\1/p' "$root/Cargo.toml")"
arch="$(uname -m)"
name="Industrial-Kerf-$version-macos-$arch-source"
formulae="$root/target/package-macos/bundled-formulae.txt"
out="$root/target/package-macos-source"
stage="$out/$name"
[ -s "$formulae" ] || { echo "no $formulae: run scripts/package-kerf-macos.sh first" >&2; exit 1; }
rm -rf "$out"
mkdir -p "$stage/homebrew"

# Kerf, and the crates its Cargo.lock names.
git -C "$root" archive --format=tar --prefix=kerf/ HEAD | tar -x -C "$stage"
(cd "$stage/kerf" && cargo vendor --locked --versioned-dirs vendor > cargo-vendor-config.toml)

# Each formula's source as Homebrew fetches it, and the formula, which is how it was
# built. A formula's installed version is the one the app carries.
while read -r formula formula_version; do
    installed="$(brew list --versions "$formula" | awk '{print $2}')"
    if [ "${installed%_*}" != "${formula_version%_*}" ]; then
        echo "$formula is installed at $installed, but the app carries $formula_version" >&2
        exit 1
    fi
    brew fetch --build-from-source --force --quiet "$formula"
    cached="$(brew --cache --build-from-source "$formula")"
    mkdir -p "$stage/homebrew/$formula-$formula_version"
    if [ -d "$cached" ]; then
        # A formula built from a Git checkout (x264) is cached as that checkout.
        tar -C "$(dirname "$cached")" -cf "$stage/homebrew/$formula-$formula_version/$(basename "$cached").tar" "$(basename "$cached")"
    else
        cp "$cached" "$stage/homebrew/$formula-$formula_version/"
    fi
    rb="$stage/homebrew/$formula-$formula_version/$formula.rb"
    brew cat "$formula" > "$rb" 2>/dev/null ||
        curl -fsSL --retry 5 -o "$rb" "https://raw.githubusercontent.com/Homebrew/homebrew-core/HEAD/Formula/${formula:0:1}/$formula.rb"
done < "$formulae"
cp "$formulae" "$stage/homebrew/bundled-formulae.txt"

cat > "$stage/README.txt" <<NOTE
The complete source of Industrial Kerf $version for macOS ($arch).

kerf/      Kerf at commit $(git -C "$root" rev-parse HEAD), with its Rust crates in
           kerf/vendor (cargo build --offline after merging cargo-vendor-config.toml
           into .cargo/config.toml).
homebrew/  For each Homebrew formula the app has files of: its source archive, as
           Homebrew fetches it to build it, and the formula (.rb) that builds it.
NOTE

tar -C "$out" -cf "$out/$name.tar" "$name"
(cd "$out" && shasum -a 256 "$name.tar" > "$name.tar.sha256")
rm -rf "$stage"
echo "$out/$name.tar"
