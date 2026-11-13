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
- A media line can say what its file is made from:
  `recipe="shots/open.kerfrecipe"` names a recipe, a text file with the steps
  that make it: a program to run, a sequence of the project for Kerf to
  render, a model to ask. `kerf make <media>` shows the steps, asks, and runs
  them. A 3D render or a video model's shot can be made again from the
  project. In the editor, such a media has Make from Recipe… in the bin's
  menu, which shows every step and asks before any runs.
- Each making is a take, kept beside the ones before it, and the media is the
  newest: `kerf takes <media>` lists them and `kerf takes <media> --use 1`
  goes back to one. Kerf says when a shot wants making again and for what (its
  recipe, a file the recipe reads, a sequence it renders), and trimming,
  moving, grading or titling a made shot never asks for it again.
- A recipe's `generate` line asks a video or image model for the shot: a
  prompt, a guide to follow, first and last frames, references, a seed. Kerf
  comes with no model. One is a program you name in `~/.kerf/models.json`,
  which is handed what was asked as a file; see
  [docs/shots.md](docs/shots.md#models). The repository has such a program
  for a model on a ComfyUI server, `kerf-comfy`, which the packages do not
  carry yet: it fits the guide to the model's size and rate, asks the server,
  and writes the take
  ([docs/shots.md](docs/shots.md#a-model-on-a-comfyui-server)).
- A recipe can tell a model when something happens, and not only where and
  what: a `gen=signal@1` clip draws nothing and holds a number from 0 to 1
  that you keyframe, and `generate … signal=<clip>` hands the model what it
  comes to on each frame. `kerf-comfy` gives it to a workflow at a `{signal}`
  blank, a number for each frame the model makes; the workflow it comes with
  reads none yet. Changing a signal asks for the shot again; nothing else in
  its sequence does ([docs/shots.md](docs/shots.md#signals)). An agent's
  `look` with `signal` writes a signal's keys from the onsets of a sound on
  the timeline, so that what a model is told happens on its beats.
- A recipe's `render` line can draw a pass in place of the picture:
  `pass=ids` gives every clip one flat color of its own, and `pass=depth`
  how near each pixel is, for a model that follows one
  ([docs/shots.md](docs/shots.md#passes)).
- An agent's `look` with `shows` reads what shows of each clip on a frame:
  where it is, what hides it, and what has left the frame. A floor drawn
  over what stands on it is said before a making is asked for.
- A recipe's `render` line takes a size and a rate
  (`render s2 {guide.mp4} size=832x480 rate=16`), so a guide is drawn as the
  model reads it and not at the sequence's own size and rate: a guide that
  took 54 s to draw took 8. A picture of another shape fills the size, with
  what is over cut off evenly. What `render` draws no longer has a sound
  track.
- A model's entry in `~/.kerf/models.json` can say what the model takes,
  under `takes`: the picture it makes, its rate, its longest shot, and the
  files it reads. An agent's `look` lists it with the model, with the
  `render` line that draws a guide at that size and rate, so a recipe is
  written for the model it asks. `kerf-comfy --workflow wan-vace --describe`
  prints the entry for a workflow
  ([docs/shots.md](docs/shots.md#what-a-model-takes)).
- A project can be worked on from a terminal with no editor open. `kerf load`
  takes a `project.kerf` you changed by hand as the project, showing the
  lines that change first; `kerf proposals` lists what an agent proposed,
  and `kerf accept` and `kerf reject` answer it. `kerf make` and `kerf
  takes` now say so when the file is not what Kerf has, before anything
  runs.
- While a shot is made from its recipe, the editor shows what its steps say
  as they say it, and **Stop Making** on the media in the bin stops it. A
  step that is stopped is asked to end before it is made to, on macOS and
  Linux, so a model's program can take back the job it gave a server:
  `kerf-comfy` does, in the editor and on Ctrl-C in a terminal, where before
  the job ran on with nobody to fetch it.
- Before a shot is made, `kerf make` and the editor note what its recipe asks
  that its model has said it does not take: a guide drawn larger than the
  model reads, a shot longer than it makes, a file it has no use for. An
  agent is told the same, and what would refuse the making, as soon as it
  proposes the media or looks, and a new skill, `model-shot`, carries what
  it cannot see for itself of blocking a shot for a model.
- An agent can go round the whole loop of a shot a model makes: it writes the
  recipe as part of its proposal, where you read it before you accept, asks
  for the making with its new `make` tool, and looks at the take. A recipe it
  writes again is kept beside the one before (`open@2.kerfrecipe`), so undo
  goes back, and choosing an earlier take brings back the recipe it was made
  from. By default you are asked: `kerf proposals` lists what it wants made,
  and `kerf make` makes it. In `~/.kerf/agent.json` you can let it do more on
  its own, as a coding agent's permission modes do: `accept-edits` accepts its
  edits as it makes them, and `auto` also runs the makings it asks for, up to
  `makes` a session ([docs/shots.md](docs/shots.md#an-agents-part)).
- A shot that does not fill the frame says the `scale` that does: a picture is
  fit inside the frame first, so it is seldom what the sizes alone say.
- Kerf no longer has 38 effects, none of which a look, a grade or a preset
  used. Gone are those that make a look a video model makes better: Brush
  Strokes, Emboss and Color Emboss, Find Edges, Roughen Edges, Motion Tile,
  Replicate, Strobe Light, Alpha, Echo and Edge Glow, Lighting Effects,
  Volumetric Rays, Lens Flare, Lightning, Cell Pattern, Bulge, Spherize, Twirl,
  Turbulent Displace, Long Shadow, and five of the 360° effects (Chromatic
  Aberrations, Color Gradients, Digital Glitch, Fractal Noise, Glow). And
  those a clip does by itself: Move, Grow, Shrink, Spin, Wiggle and Camera
  Shake (keyframes, an expression such as `x=(noise(t*3)*0.02)`, or a look),
  3D Rotate and Basic 3D (`three_d=true` with `rotate_x`, `rotate_y`, `z`),
  Bevel Alpha (`depth=` and `bevel=`), Circle and Ellipse (`gen=shape@1`), and
  Clone. A project that uses one does not open until its line is changed: the
  error names the line and what to use instead.
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
  timeline. Gone so far: Customise Keyboard, the Graph Editor, the Finish and
  Deliver trays (scopes, the loudness meter, the QC pane, the render queue and
  the paperwork), the Inspector with its Source Monitor, the Effects, Titles and
  Presets browser, Sequence Settings, and "Try 3 versions at once".
- The toolbar is cut down to the sidebar's button, the project's name, Export
  (once there is a cut) and one button for the picture and timeline. The ≡ menu
  is gone: your account sits at the foot of the sidebar and opens Settings,
  Media, Import and View. The format picker, Follow Kerf and Media's button moved
  to the timeline's strip. "Edited 2 min ago" is the title's tooltip; the bar
  speaks only when something is going on.
- Settings is a page in the window, not a dialog: Account, AI connection,
  General and Advanced, with Esc or Back to leave.
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
  - Media goes into the timeline from the Media sheet: double-click it, or
    choose it and use Clip ▸ Insert or Overwrite, and the whole of it lands at
    the playhead. A clip's position, crop, sound, effects and titles are Kerf's
    to change: ask in the conversation. The mask tools still draw on the
    picture.
  - File ▸ Save Current as Preset… and Import Editing Preset… still save and
    apply a preset; the list of a project's presets is gone.
- A clip with a Corner Pin on it, which is what tracking a surface writes, now
  draws on the GPU with the rest of its frame, where one had sent every frame
  it was in to the CPU. The picture is the same. How much faster such a shot
  plays and renders has not been measured yet.
- So do clips with a Drop Shadow, a Rounded Crop, a Spacer or a Mosaic: a
  picture in a picture with rounded corners and a shadow, or a face hidden
  behind blocks, no longer takes its frames off the GPU. These are not timed
  yet either.
- Curves, Hue/Saturation, Selective Color, Photo Filter, Unmult and the Cineon
  Converter draw on the GPU too, in SDR and in HDR sequences, so a clip graded
  with one of them keeps its frames there. Curves with a point outside 0 to 1
  is still drawn on the CPU. Not timed yet.
- Lens Distortion and Magnify draw on the GPU as well: straightening a wide
  lens, or a loupe over a detail, no longer sends the frame to the CPU. Not
  timed yet.
- Gradient, Ramp, 4-Color Gradient, Checkerboard and Grid draw on the GPU: a
  gradient behind a title or a grid over a frame keeps the frame there. Not
  timed yet. With these, 45 of Kerf's effects and transitions still send a
  frame to the CPU, where 63 did.

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
