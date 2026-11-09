# Changelog

What changed in each release of Industrial Kerf. A release is a `v<version>` tag
on `main`; its section here becomes the release's notes on GitHub
(`.github/workflows/package.yml`), so write it for someone using Kerf.

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
