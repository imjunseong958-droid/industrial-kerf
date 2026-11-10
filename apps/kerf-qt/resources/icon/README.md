# Kerf's icon

`industrial-kerf.svg` is the icon, and the only place it is drawn: a red-crowned
crane whose beak cuts a clip in two, under a branch of plum blossom, after an ink
painting of a crane. Everything else here is made from it by `tools/kerf-icon`:

```sh
cargo run -p kerf-icon            # writes the files below again
cargo run -p kerf-icon -- --check # fails if any no longer matches the SVG
```

| File | For |
|---|---|
| `industrial-kerf-<side>.png`, 16 to 1024 | The editor's windows and taskbar entry (16 to 256, compiled in at `:/icon/`, `ui/app_icon.cpp`) and the Linux package's `icons/` (16 to 512), which `install-desktop-entry.sh` installs into the user's hicolor theme |
| `industrial-kerf.ico` | `kerf-qt.exe`, through `kerf-qt.rc`, and the Windows installer (`SetupIconFile`): 16, 24, 32, 48, 64 and 256 pixels |
| `industrial-kerf.icns` | The macOS app's `Resources`, named by `CFBundleIconFile`: 16 to 512 points at 1x and 2x, the tile at macOS's 824 of 1024 where the others' is 896 |

The files are in the repository, not made by the build, so that building the editor
needs no SVG renderer on any of the three systems, CMake and the installer can name
them as plain files, and a change to the icon shows in review as the pictures it
changes. `cargo test -p kerf-icon` renders the SVG again and fails when a file is
missing or differs from it by more than rounding, so the two cannot drift apart.

## The photographs in the clip

The clip's frames hold five photographs of Hong Kong, Tokyo and New York at night, all
dedicated to the public domain (CC0 1.0) by their authors on Wikimedia Commons. Each is
cropped, scaled to 216 by 236 pixels, eased in saturation and tinted violet, and
embedded in the SVG as a JPEG; the originals are not in the repository.

| Frame | Photograph | Author | Source |
|---|---|---|---|
| 1, and 3 under the cut | Hong Kong in the night | Wilfredor | https://commons.wikimedia.org/wiki/File:Hong_Kong_in_the_night.jpg |
| 2 | Hong Kong in night (noise version) | Wilfredor | https://commons.wikimedia.org/wiki/File:Hong_Kong_in_night_(noise_version).jpg |
| 4 | Buildings at night around Shibuya | Syced | https://commons.wikimedia.org/wiki/File:Buildings_at_night_around_Shibuya.jpg |
| 5 | Times Square (New York City) | ISO Legacy | https://commons.wikimedia.org/wiki/File:Times_Square_(New_York_City).jpg |
| 6 | Buildings at night around Shibuya 2 | Syced | https://commons.wikimedia.org/wiki/File:Buildings_at_night_around_Shibuya_2.jpg |

Each was taken from Commons' 960 pixel wide rendition, then, with ImageMagick:

```sh
convert <photograph> -gravity <g> -crop <c> +repage -resize '216x236^' -gravity center -extent 216x236 \
    -modulate 104,62,100 \( +clone -fill '#6a45d8' -colorize 100 \) -compose SoftLight -composite \
    -sigmoidal-contrast 3x45% -strip -sampling-factor 4:2:0 -quality 86 <frame>.jpg
```

with `<g>` and `<c>`, in frame order: `west 520x555+60+0`, `center 420x460+60+20`,
`east 420x460+0+90`, `east 420x460+0+60`, `center 760x830+60+40`.
