#!/usr/bin/env bash
# Fetch the real footage the quality measurements run on (docs/architecture.md,
# Real footage) into a folder outside the repository, and say where it went.
#
#   scripts/fetch-real-footage.sh [folder]     # default: target/real-footage
#   KERF_REAL_FOOTAGE=target/real-footage cargo test --release -p kerf-image \
#       --test real_footage -- --ignored --nocapture
#
# The frames are from Netflix's Chimera (2014), Creative Commons Attribution 4.0
# International (licenses/README.md): uncompressed DCI 4K TIFFs, P3 primaries and
# PQ, as the Open Content bucket keeps them. About 1.4 GB is downloaded; what is
# kept is 16-bit PNG, at 4K and at 1080p, about 0.9 GB. Frames already fetched
# are not fetched again.
#
#   traffic/  frames 2874-2897 of 3_TRAFFIC, a locked-off shot of a freeway at
#             23.976 fps: a white box truck crosses the frame in about 30 frames
#             under the camera's own shutter, in front of a stack of tires and a
#             background that do not move.
#   fountain/ frames 3836-3837 of 4_TODDLER_FOUNTAIN: water catching the sun.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
out="${1:-$root/target/real-footage}"
bucket="https://s3.amazonaws.com/download.opencontent.netflix.com/Chimera/tif_DCI4k2398p"
mkdir -p "$out"
raw="$(mktemp -d "${TMPDIR:-/tmp}/kerf-footage.XXXXXX")"
trap 'rm -rf "$raw"' EXIT

command -v ffmpeg >/dev/null || { echo "ffmpeg is needed" >&2; exit 1; }

# fetch <group> <first> <last>: each frame as <group>/4k/NNNN.png and
# <group>/1080/NNNN.png.
fetch() {
    local group="$1" first="$2" last="$3" frame name tif
    mkdir -p "$out/$group/4k" "$out/$group/1080"
    for frame in $(seq "$first" "$last"); do
        name="$(printf %04d "$frame")"
        [ -s "$out/$group/1080/$name.png" ] && continue
        tif="$raw/$name.tif"
        curl -fsS --retry 3 -o "$tif" "$bucket/Chimera_DCI4k2398p_HDR_P3PQ_$(printf %05d "$frame").tif"
        ffmpeg -v error -y -i "$tif" -pix_fmt rgb48be "$out/$group/4k/$name.png"
        ffmpeg -v error -y -i "$tif" -vf scale=2048:-2:flags=area -pix_fmt rgb48be "$out/$group/1080/$name.png"
        rm -f "$tif"
    done
}

fetch traffic 2874 2897
fetch fountain 3836 3837
curl -fsS -o "$out/LICENSE-Chimera.txt" \
    "https://s3.amazonaws.com/download.opencontent.netflix.com/Chimera/creative-commons-attribution-4-intl-public-license.txt"
du -sh "$out"
echo "KERF_REAL_FOOTAGE=$out"
