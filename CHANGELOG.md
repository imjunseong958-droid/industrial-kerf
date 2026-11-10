# Changelog

What changed in each release of Industrial Kerf. A release is a `v<version>` tag
on `main`; its section here becomes the release's notes on GitHub
(`.github/workflows/package.yml`), so write it for someone using Kerf.

## 0.1.2

- On Windows, a `kerf://` review link opens in the Kerf you already have open,
  and brings it to the front, instead of starting another. Linux does the same.
- On Linux, `./install-desktop-entry.sh` in the unpacked folder adds Kerf to your
  applications and has `kerf://` review links open in it, for you alone;
  `--remove` takes it away.
- A `.deb` for Debian 13, Ubuntu 24.04 and what comes from them:
  `sudo apt install ./Industrial-Kerf-…-linux-x64.deb` installs Kerf with what it
  needs from the system, adds it to your applications, opens `kerf://` review
  links in it, and puts `kerf`, `kerfd` and `kerf-mcp` on your `PATH`;
  `sudo apt remove industrial-kerf` takes it away. The `.tar.xz` is still there
  for other distributions, and for those who want a folder.
- A title can turn its font's OpenType features on and off:
  `features="tnum zero"` gives tabular figures and a slashed zero, `smcp` small
  capitals where the font has them, `-liga` turns ligatures off, and `ss01` or
  `salt=2` take a stylistic set or an alternate. An agent's `look` with `font`
  names the features a font has.
- Kerf has an icon of its own, a crane cutting a clip: on its windows and in the
  taskbar, in the Dock and Finder on a Mac, on `kerf-qt.exe`, the installer, the
  Start menu and `kerf://` links on Windows, and among your applications on Linux
  with the `.deb`, or once `./install-desktop-entry.sh` has run.
- Kerf for Mac needs macOS 26 or later, and on an older macOS it now says so
  instead of failing to open.
- A number that takes keyframes can be an expression of the clip's time:
  `rotation=(t*90)`, `x=(sin(t*pi)*0.1)`, `opacity=(0.5+0.5*noise(t*3))`. The
  Inspector shows what it gives at the clip's first frame; setting the value
  there replaces it.
- `KERF_GRAPH_RENDER=1` and `kerf-engine render --graph`, which had FFmpeg draw
  the picture as renders did before Kerf's own renderer, are gone. Renders,
  playback, and previews already drew with that renderer; nothing about them
  changes.
- A media line can say what its file was made from: `recipe="shots/open.json"`
  names a recipe, a text file with the command that makes it, and
  `kerf make <media>` runs it and puts the new file in the media's place. A
  3D render or a video model's shot can be made again from the project. In
  the editor, such a media has Make from Recipe… in the bin's menu, which
  shows the command and asks before it runs.
- An effect's preview in the browser shows the clip's effects in the order they
  are in, where it had run some kinds before others, and a title or a nested
  sequence now has a preview of every effect.
- Moving the playhead just as an edit lands no longer leaves the viewer on the
  frame it showed before: the picture follows the playhead to where it was put.
- A shape whose offset shrinks it past its middle is gone at once, where the
  editor had stopped for several seconds to find that nothing was left.
- Rolling Shutter Repair now repairs HDR footage in an HDR sequence, where it had
  left the picture as it was.
- The editor is being cut down to the conversation, the picture and the
  timeline. Gone so far: Customise Keyboard, the Graph Editor, and the Finish and
  Deliver trays (scopes, the loudness meter, the QC pane, the render queue and
  the paperwork).
  - Edit ▸ Keyboard Shortcuts still chooses between the Kerf, Premiere Pro and
    Final Cut Pro layouts; keys of your own no longer apply.
  - Keyframes and their curves are still the project's, for Kerf and `kerf` to
    write; a motion path is still drawn and dragged on the picture.
  - Sequence ▸ Run QC still runs the checks, and the Export menu's Preflight
    lists what they found. Export still renders a Draft or a Master, the master
    to `deliveries/` as before. The editor no longer writes a delivery's
    paperwork (comments CSV, markers, EDL, captions).
  - A proposal that changes one clip's colour says so in a line, without the
    small vectorscope.

## 0.1.1

On a Mac without Homebrew's FFmpeg, 0.1.0 did not find the FFmpeg inside the app:
media came in at a made-up size, rate and length, and what needs FFmpeg to read or
write a file did not work. Kerf now uses the FFmpeg it comes with, on Windows as
before and on macOS too. Windows was not affected.

- Kerf for Linux (x86-64): a folder that runs on its own, from Ubuntu 24.04 on,
  with its complete source beside it.
- A Windows installer: Kerf installs for you alone, with no administrator, adds
  itself to the Start menu, opens `kerf://` review links, and uninstalls from
  Settings ▸ Apps. The zip is still there for those who want a folder.
- Help ▸ Kerf User Guide opens the guide that comes with Kerf, so it works with
  no connection.
- The Kerf Cloud sign-in is kept in the system keychain, as API keys are; one
  kept in a file by 0.1.0 moves there by itself.
- An API key changed while the system keychain could not take it (locked, or
  access denied) is no longer replaced by the older key the next time Kerf
  opens.
- A crash in the editor's own code, in Qt or in FFmpeg now leaves a crash report
  too, not only one in Kerf's Rust, and Kerf tells of it the next time it opens.

## 0.1.0

The first release, for Windows and for Macs with Apple silicon: a zip that runs on
its own for each, and the complete source of each, since their FFmpeg makes them GPL
version 3. Neither is signed: Windows SmartScreen asks before the first run (More
info ▸ Run anyway), and so does macOS (right-click the app ▸ Open, or System
Settings ▸ Privacy & Security ▸ Open Anyway).

- Edit by asking: Kerf proposes each edit on the timeline, and nothing changes
  until you accept it.
- A timeline, source monitor, Inspector, effects and titles, masks, and
  hundreds of looks, grades and transitions, drawn on the GPU.
- Review notes kept with the project, blocking notes that hold delivery back, and
  `kerf://` links to a version and a timecode.
- Projects shared through Git and GitHub, merged clip by clip.
- Finish checks (loudness, gamut, title safe, captions, flash and black) and
  Deliver: ProRes masters, an EDL, markers, and captions.
- Crash reports kept on your computer, settings that carry over from one version
  to the next, and API keys in the system keychain.
- Help ▸ Kerf User Guide, and Help ▸ About Kerf.
- Kerf tells you when a newer release is out, once a day; Settings ▸ Updates
  turns it off.
