# Shared projects

A Kerf project is a folder: the edit is `project.kerf`, a text document in
[the Kerf language](kerf-language.md), beside the media it plays and the
presets and looks it uses. People share it the way code is shared, with Git:
commits, pulls, pushes, branches, and pull requests, on GitHub or any other
Git host. Kerf runs those for you when you sync, and merges `project.kerf`
by what it holds, clip by clip, so two people editing at the same time both
keep their work.

Kerf uses your own `git`, and `gh` when it is installed, with your
configuration and credentials. The editor never prompts: set up access to
the remote first (`gh auth login` then `gh auth setup-git`, or an SSH key in
your agent or Keychain, such as `ssh-add --apple-use-keychain`, with the
host known from one connection in a terminal, `ssh -T git@github.com`).
Without that, a sync reports an error instead of waiting: ssh runs in batch
mode with a 20-second connection timeout, and a transfer slower than
1000 bytes a second for a minute gives up. Your own `GIT_SSH_COMMAND`,
`core.sshCommand`, and `http.lowSpeedLimit`/`http.lowSpeedTime` are used as
they are, and `kerf sync` in a terminal can still prompt.

| Tool | Needed for |
|---|---|
| `git` | Every shared project |
| Git LFS (`brew install git-lfs`) | Media files of 100 MB or more on GitHub |
| GitHub CLI (`brew install gh`, then `gh auth login`) | Creating GitHub repositories and opening pull requests from Kerf |

## Starting a project

`kerf` without a project opens the editor's project launcher, as does
**File → New Project…**. Its sidebar has **New Project**, **Recent**,
**From Preset**, and **Clone from GitHub**; **Open an Existing Project…**
opens any project folder.

New Project offers two templates: **Empty 16:9** (1920×1080) and
**Vertical 9:16** (1080×1920). Under
**Details** you set the name, the location, and the frame rate (23.976, 24,
25, 29.97, 30, or 60; 30 by default), and whether to share it:

- **Share with Git** makes the project a repository.
- **Create a private GitHub repository** creates one with `gh` and
  publishes the project to it. It is offered when `gh` is signed in, and
  shows the account.
- A remote URL publishes the project to an existing empty repository.

**Choose** makes the folder and opens it. **Recent** lists the last twelve
projects opened; **File → Open Project…** and **File → Open Recent** open
another project in place of the current one.

The same from a terminal:

```bash
kerf new "My Film" --location ~/Movies --rate 24 --git
kerf new "My Film" --location ~/Movies --vertical --github          # private
kerf new "My Film" --location ~/Movies --github --public
kerf new "My Film" --location ~/Movies --remote git@github.com:team/my-film.git
```

`--square` is the 1:1 template. `--remote` and `--github` imply `--git`.
`--location` defaults to the current directory.

A new project is a folder named after the project (characters file systems
refuse become `-`), holding `project.kerf` with one empty sequence of the
template's size and rate, and empty `media/` and `presets/` folders. A
shared project also gets `.gitignore` and `.gitattributes` and a repository
on `main` with a first commit, "Create My Film", pushed to its remote when
it has one. A location that already holds a non-empty folder of that name
is refused. When Git LFS is not installed, Kerf says so: such a project
cannot share media files of 100 MB or more on GitHub.

## Project layout

```text
My Film/
  project.kerf     the edit, merged structurally
  media/           media files, named media/<stem>-<hash>.<ext>
  presets/         the team's preset library (*.kerfpreset)
  .kerf/looks/     the project's looks (*.kerflook)
  .gitignore
  .gitattributes
```

`.gitattributes` routes `project.kerf` through Kerf's merge (`*.kerf
merge=kerf`), keeps looks, presets, and recipes as text with Unix line
endings, and, when Git LFS was installed as the project was made, stores
`media/` with Git LFS (`media/** filter=lfs diff=lfs merge=lfs -text`), and
the takes a recipe makes the same way (`**/*.takes/**`), with each take's
record and request left as text ([shots.md](shots.md#takes)). A project made
before takes has neither line; add them to its `.gitattributes` by hand.

What is per machine stays out of the repository: kerfd's state and journal
(`.kerf-runtime.*`, `.kerfd.sock`, `.kerf/runtime/`), caches, proxies, and
generated docs (`.kerf/cache/`, `.kerf/docs/generated/`), settings that may
hold credentials or start programs (`.kerf/config.json`, `.kerf/mcp.json`,
`.kerf/vision.json`), kerfd's temporary `project.kerf.tmp`, and renders,
which land beside `project.kerf` as `*.mp4`; so an `.mp4` beside
`project.kerf` is never shared, and media belongs in `media/`. Kerf also
lists these in the repository's own `.git/info/exclude` before it stages
anything, so a repository whose `.gitignore` lacks them does not commit them
either.

The three settings files are never committed, even when a shared
`.gitignore` lets them in. A branch, upstream, or repository that tracks one
of them, or tracks `.kerf` itself as a file or link, is refused when you
sync, switch to it, or clone it (a refused clone is removed), because
checking it out would replace this machine's settings with a
collaborator's. Remove it from the repository in a terminal (`git rm
--cached .kerf/mcp.json`, commit, push). Kerf chat also ignores a project
`.kerf/mcp.json` that Git tracks, and says so.

A project is shared through the repository whose top folder it is. A
project inside some other repository, such as a folder in a source
checkout, is not shared through it; the Collaborate menu says so.

## Sharing a project

- **Collaborate → Publish to GitHub…** makes the project a repository if it
  is not one, then creates a GitHub repository with `gh` (private unless you
  say otherwise) or adds the remote URL you give, and pushes.
- **Clone from GitHub** in the launcher takes a repository URL and a
  location, clones into a folder named after the repository, and, with Git
  LFS installed, downloads its media. `git clone` works as well; open the
  folder with **Open an Existing Project…**.

Collaborators need write access to the repository (on GitHub, the
repository's Settings → Collaborators).

## Syncing

**Collaborate → Sync Now**, or a click on the sync chip in the status bar,
shares your edits and brings in everyone else's. The chip shows the branch
and how far it is from the remote: `⎇ main ↑2 ↓1` is two commits to push and
one to pull.

One sync:

1. Stages everything in the project folder, as `git add -A` does. Hashing
   new media is the slow part, so it runs beside the editor, which stays
   responsive. A file that Git would store at 100 MB or more is refused by
   name, and nothing is left staged; that includes media in a project made
   with Git LFS, opened on a machine without it.
2. Fetches the remote.
3. Commits your edits. The message describes them, such as "Added marker
   intro", or "Changed clip c3: start, duration (+2 more)" followed by each
   change on its own line; `kerf sync -m` gives your own, and
   **Collaborate → Commit…** commits with your message without syncing.
   Without a Git identity configured, your login name is used.
4. Merges the remote branch: a fast-forward when you had nothing new,
   otherwise a merge commit, with `project.kerf` merged structurally (see
   below). Media the merge brings in is not downloaded yet.
5. Loads the merged project into the editor as one revision.
6. Pushes. A branch the remote does not have yet is published and tracks
   it.
7. With Git LFS, downloads the media the merge brought in, and the viewer
   reloads it.
8. Reports what happened: the status bar lists each incoming commit by
   author and subject ("jun · Trimmed c3") and each conflict the merge
   settled.

kerfd, which owns the project, does steps 3 to 5, so nothing else writes
`project.kerf` while it changes. A sync that brings in nothing leaves the
project and its history alone. A sync that changes the project clears undo
history and discards a pending proposal: Undo does not step back across a
sync. The earlier versions stay in Git history.

### Live Sync

**Collaborate → Live Sync** syncs every 10 seconds, and a few seconds after
each edit, for as long as it is on. Each cycle fetches when the project has
a remote, merges only when you have edits or the remote has new commits,
and pushes when you are ahead; a cycle never starts while another runs. The
setting is remembered per project. A failure is reported once, and again
only when it changes.

With Live Sync on, collaborators see each other's edits within seconds.
They still arrive as commits: there is no shared cursor or playhead.

### When a sync stops

- **`project.kerf` changed on disk.** Something other than Kerf changed the
  file, such as a `git pull` in a terminal or a text editor. The editor
  shows a banner: **Load from Disk** loads the file as one revision (undo
  history clears), **Keep Editor Version** writes the editor's project over
  it. Kerf refuses edits and syncs until you choose. `kerf load` in a
  terminal is Load from Disk: it shows the lines that change and asks.
- **Both sides changed another file.** Git merges files other than
  `project.kerf` line by line. When two people changed the same lines of a
  look, a preset, or another text file, the merge is abandoned and the
  error names the files. Your edits stay committed locally; merge in a
  terminal (`git pull --no-rebase`, fix the files, commit), then choose
  **Load from Disk** and sync.
- **The merged project is not valid,** or does not read in this version of
  Kerf (it uses something a newer Kerf added). The merge is abandoned or
  undone and the branch is left as it was, with your edits committed.
- **A merge is in progress** in the repository, started in a terminal.
  Finish it or run `git merge --abort` first.
- **The push was rejected** because someone pushed in between. The next
  sync merges their commits and pushes.
- **A file is too large** for GitHub. Install Git LFS (see below), or keep
  the file out of the project.
- **A branch tracks machine-local settings,** such as `.kerf/mcp.json` (see
  Project layout). Nothing is merged until it is removed from the branch.

## How `project.kerf` merges

Git merges lines, and two people editing one project collide on lines
constantly: both append a clip to a track, both allocate `c7`. Kerf
instead reads the common ancestor, your version, and theirs, and merges
them element by element by id: the project's name, media, sequences, tracks,
clips, transitions, and markers. The result is printed as the canonical
document again.

"Yours" is the version of the person syncing; "theirs" is what came from
the remote.

- **Changed on one side:** that side's version, including a removal.
- **Changed on both sides, in different fields:** both changes. You trim
  `c3` and they add a blur to it; the merged clip is trimmed and blurred.
  The fields are a media's file (its path, rate, timecode, duration, and
  streams, taken together) and its reel, so a relink on one side and a
  replacement on the other is one field changed on both sides; a sequence's name, rate, size, sample rate, and start; a track's
  name, enabled, locked, and muted state; a clip's track, start, duration,
  what it plays from where, enabled state, name, link, each property (such
  as `scale` or `opacity`), each mask by name, and its effect stack as a
  whole. Transitions and markers merge as a whole.
- **The same field changed differently on both sides:** yours is kept, and
  the sync reports it, for example `s1/V1/c3: duration changed on both
  sides (kept yours)`. Their value is not lost: it is in their commit, and
  you can set it again.
- **Removed on one side, changed on the other:** your side wins. A clip you
  changed and they removed stays; a clip you removed and they changed stays
  removed. Both are reported.
- **Removed, but still used:** a track the other side put a clip on, media
  one of its clips plays, a marker one of its looks lands on, a sequence one
  of its clips nests, or a track one of its effects reads (a track matte, an
  audio spectrum) comes back from the side that kept it, reported as
  restored. A nested sequence or a track an effect reads comes back whole,
  with its clips, and what it uses comes back with it.
- **The same id added on both sides for different things:** theirs gets a
  fresh id, and everything of theirs that named it follows. The sync
  reports the rename, such as `c7` to `c9`. The same thing added on both
  sides under one id is kept once, and a link group both sides added under
  one id that shares a clip is one group, holding both sides' clips.
- **Clips that overlap:** when both sides put a clip in the same place on a
  track, theirs moves to the nearest free track of the same kind (above for
  picture, below for sound), and Kerf adds a track when none is free:
  `s1/V1/c9: overlapped c5 on V1; moved to V3`.
- **Transitions that no longer fit their cut** after the merge, because a
  clip moved or was trimmed, are removed and reported.

Your order of elements holds; their additions follow the element before
them in their version. The merged project must pass Kerf's validation; when
it cannot, the merge stops as described above.

Each reported conflict names where it is as a path of ids (`media m2`,
`sequence s1`, `s1/V1`, `s1/V1/c3`, `s1/marker beat1`), what collided, and
whose version was kept.

## Branches and pull requests

- **Collaborate → New Branch…** commits your edits, then makes a branch
  from here and switches to it. The next sync publishes it.
- **Collaborate → Switch Branch** commits your edits, switches, and loads
  that branch's project as one revision; undo history clears when the
  project differs.
- **Collaborate → Open Pull Request…** opens a pull request for the current
  branch with `gh`, with your title; an empty description becomes the list
  of the branch's commit subjects. Without `gh` signed in, or when `gh`
  cannot create it, it opens GitHub's page for starting one; the item is
  available whenever the project has a remote. The default branch is
  refused: make a branch for the change first. Sync first, so GitHub has the
  branch.

A pull request's diff reads as the edit, because `project.kerf` is
canonical text with one line per clip:

```diff
   track V1 video
-    clip c3 media=m2 src=00:00:04:00 at=00:00:06:00 dur=00:00:03:00
+    clip c3 media=m2 src=00:00:04:00 at=00:00:06:00 dur=00:00:02:12
+      fx blur.gaussian@1 radius=8
```

GitHub's merge button merges lines; it does not run Kerf's merge. When
GitHub reports a conflict in `project.kerf`, merge the default branch into
yours in a terminal, where Kerf's merge runs (see the merge driver below):

```bash
git fetch
git merge origin/main      # project.kerf merges structurally
```

Then **Load from Disk** in the editor, or `kerf load` in a terminal, and
sync.

## Shared presets and media slots

A [preset](presets.md) is Kerf editing code with media slots: "slot 1's
video plays here, cut at these frames, with this look". In a shared project
the presets, the media, and which file fills which slot are all part of the
repository.

### The preset library

`presets/` is the project's preset library, so presets travel with every
sync. **Window → Presets** lists the presets in `presets/`, then
`.kerf/presets` and the project folder, each with its slots and who changed
it last and when (from its last commit). **Save Current as Preset…** saves
the program sequence as `presets/<name>.kerfpreset`, with the name cleaned
up as project folders are (`Intro/Outro` becomes
`presets/Intro-Outro.kerfpreset`); when that file exists, Kerf asks before
replacing it. **Apply** opens the usual binding dialog,
where each slot is given project media, and stages the result as a
proposal.

### Slots are media ids

Applying a preset binds each slot to a media id of the project, such as
`m1`. In a shared project a media id names a file in `media/`, in the
repository, so every collaborator has the same bytes behind `m1`: slot 1's
video is the same for everyone.

In a project with a `media/` folder, as every new project has, importing
a file from outside the project copies it into `media/` first, named `<stem>-<the first 8 hex digits of its SHA-256>.<ext>`
(`interview-3f9a2c41.mov`). Two people importing the same file make the same
path, so the two additions do not conflict; different files never share a
name. Paths are stored relative to the project with `/`, so they resolve on
every machine. A file already inside the project folder is used where it
is, unless Git ignores it there (like a render beside `project.kerf`), and
then it is copied into `media/` too. A copy is written under
`.kerf/cache/incoming/` until it is whole, and Live Sync waits for it, so a
half-copied file is never shared. A file that does not read as media is
refused before anything is copied. `kerf <media>` from a terminal copies,
and downloads, into `media/` the same way.

The Project bin marks media that is not ready: **OFFLINE** when the file is
missing, **NOT DOWNLOADED** when it is a Git LFS file not yet downloaded,
and **EXTERNAL** when it is outside the project folder, or ignored by Git
there, and so is not shared. **Consolidate Media** (Collaborate menu, or the
bin's context menu) copies every external file into `media/` and relinks
it, as one edit that can be undone; missing files, and files that do not
read as media, are skipped and listed.

### Replace Media

**Replace Media…** in the Project bin's context menu swaps the file behind a
media id. Every clip that uses it keeps its timing, so a slot can be filled
with a different take; after the next sync, everyone sees the new file. A
file too short for the clips, or without the picture or sound they need, is
refused, and the error names the clips. The new file is copied into
`media/` as an import is, and the replacement is an ordinary edit that can
be undone.

To try a different file without changing it for everyone, make a branch
first.

### New Project from Preset

**From Preset** in the launcher starts a project of your own from a shared
pattern. Pick a `.kerfpreset` file, or a preset from a recent project's
library. Each slot shows what it needs (length, picture, sound); choose a
file for it, and where in the file to start. The launcher shows which files
fit which slots. **Choose** makes the project, copies the preset into its
`presets/` and the files into its `media/`, imports them, and applies the
preset as the project's first edit. A shared project then commits it,
"Start from preset <name>", and pushes when it has a remote.

## Large media and Git LFS

GitHub refuses files of 100 MB or more. Git LFS stores them separately and
keeps a small pointer in the repository. A project made while Git LFS is
installed stores `media/` with it. Kerf refuses to stage a file of 100 MB or
more that Git LFS does not store, rather than failing at the push.

To add Git LFS to a project made without it:

```bash
git lfs install
git lfs track "media/**"
git add .gitattributes
git commit -m "Store media with Git LFS"
```

Media committed earlier stays in plain Git history; `git lfs migrate import
--include="media/**"` rewrites that history, which changes every commit, so
agree on it with the team first.

Merges and branch switches leave Git LFS media as pointers so they never
wait on the network; the download after each sync replaces them. Until
then the bin shows **NOT DOWNLOADED**, and such a file cannot be imported
or used to replace media. GitHub meters Git LFS storage and bandwidth per
account, so check its plan before sharing hours of footage.

## Using plain git in a terminal

A shared project is an ordinary repository, and every Git tool works on it.

- **The merge driver.** When the editor opens a shared project, and
  before a sync merges incoming commits, Kerf configures `project.kerf`'s
  merge in the repository: `merge.kerf.driver` in `.git/config` runs
  `kerfd merge-driver %O %A %B %P`, and `.git/info/attributes` routes
  `project.kerf` to it. Neither file is committed; each clone on each
  machine has its own; Kerf finds kerfd beside itself, on `PATH`, or where
  it was told. Once a project has been opened in Kerf, `git merge` and
  `git pull` in a terminal merge `project.kerf` structurally too, and print
  the conflicts they settle. Without it, Git merges `project.kerf` line by
  line. To set it up by hand:

  ```bash
  git config merge.kerf.name "Kerf structural merge"
  git config merge.kerf.driver "/path/to/kerfd merge-driver %O %A %B %P"
  echo "project.kerf merge=kerf" >> .git/info/attributes
  ```

  The setting names the kerfd that wrote it. If that kerfd moves, opening
  the project in Kerf again rewrites it; until then Git reports a conflict
  in `project.kerf`. Run `git merge --abort` and sync from Kerf.
- **Merge, do not rebase.** Use `git pull --no-rebase`. A rebase swaps
  ours and theirs, so "kept yours" would keep the remote's version.
- **Load from Disk.** While Kerf has the project open, kerfd owns
  `project.kerf`. After a `git pull`, `git switch`, or anything else that
  changes it, the editor shows that `project.kerf` changed on disk; choose
  **Load from Disk**.
- **Commit at any time.** kerfd writes `project.kerf` after every change,
  so the file is always the current edit.

`kerf sync` runs the editor's sync from a terminal, in the current
directory or the project `--dir` names. It goes through the project's
running kerfd (the editor's, when the project is open), or starts one:

```text
$ kerf sync -m "Tighten the intro"
committed 1a2b3c4
merged 1 incoming commit(s):
  9f8e7d6 jun · Added clip t1 on V2 (+1 more)
conflict: s1/V1/c3: duration changed on both sides (kept yours)
changed in the project:
  Added clip t1 on V2
pushed to origin/main
```

## Limitations

- Edits travel as commits, at most every few seconds with Live Sync; there
  is no shared cursor or playhead.
- A sync that changes the project clears undo history.
- When both sides change the same field, yours is kept; theirs stays in
  their commit.
- Looks, presets, and other text files merge line by line; colliding edits
  to one of them stop the sync until they are merged in a terminal.
- GitHub's merge button does not run Kerf's merge.
- Media outside the project folder is not shared until it is consolidated.
  Fonts that titles use must be installed on each machine.
- A project inside another repository is not shared through it.
- The editor never prompts for credentials; Git and `gh` must already have
  access to the remote.
