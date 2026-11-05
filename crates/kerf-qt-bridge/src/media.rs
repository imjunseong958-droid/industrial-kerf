//! A shared project's media files in the editor: copies into `media/` made
//! off the runtime worker (named by what they hold; see
//! [`kerf_collab::media`]), and which media are offline, not downloaded, or
//! not carried by the project's repository.

use crate::{emit, Command, Shared};
use kerf_collab::git::is_lfs_pointer;
use kerf_collab::media::{absolute, external, relative_to};
pub(crate) use kerf_collab::media::{copy_into_media, needs_copy};
use kerf_model::{Catalog, Id, Op, Project};
use serde::Serialize;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;
use std::thread;

/// Copy the file at `path` into the project's `media/` folder on its own
/// thread, then hand the runtime worker `then(copy)`, which names the copy
/// by its project-relative path. A file `probe` refuses is reported and
/// never copied, so what is not media never lands in `media/`, where the
/// next sync would share it; so is a copy that fails. Live sync waits while
/// copies run.
pub(crate) fn copy_then(
    shared: &Arc<Shared>,
    path: &str,
    commands: &SyncSender<Command>,
    tx: &SyncSender<Value>,
    probe: impl FnOnce(&Path) -> Result<(), String> + Send + 'static,
    then: impl FnOnce(String) -> Command + Send + 'static,
) {
    let source = absolute(&shared.directory, Path::new(path));
    let copying = Copying::start(shared);
    let (commands, tx) = (commands.clone(), tx.clone());
    thread::spawn(move || {
        if let Err(error) = probe(&source) {
            drop(copying);
            crate::error(&tx, format!("cannot import: {error}"));
            return;
        }
        let copied = copy_into_media(&copying.0.directory, &source);
        drop(copying);
        match copied {
            Ok(copy) => {
                let _ = commands.send(then(copy));
            }
            Err(error) => crate::error(
                &tx,
                format!("cannot copy {} into media/: {error}", source.display()),
            ),
        }
    });
}

/// Whether `file` reads as media, as an import reads it.
pub(crate) fn probe(file: &Path) -> Result<(), String> {
    crate::tasks::probe(file).map(drop)
}

/// A copy into `media/` that is running, counted in `Shared::copies` until
/// it is dropped.
struct Copying(Arc<Shared>);

impl Copying {
    fn start(shared: &Arc<Shared>) -> Self {
        shared.copies.fetch_add(1, Ordering::SeqCst);
        Self(shared.clone())
    }
}

impl Drop for Copying {
    fn drop(&mut self) {
        self.0.copies.fetch_sub(1, Ordering::SeqCst);
    }
}

/// Media that cannot play as the project names them, or that collaborators
/// do not get: offline (no file), Git LFS pointers (the file was not
/// downloaded), and external files (outside the project folder, or ignored
/// by Git). Each media id is in at most one list, in that order.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub(crate) struct MediaStatus {
    pub missing: Vec<String>,
    pub pointers: Vec<String>,
    pub external: Vec<String>,
}

impl MediaStatus {
    pub fn event(&self) -> Value {
        json!({"type": "media_status", "missing": self.missing,
            "pointers": self.pointers, "external": self.external})
    }
}

/// Where each of `project`'s media stands; relative paths are read against
/// `directory`.
pub(crate) fn status(project: &Project, directory: &Path) -> MediaStatus {
    let mut status = MediaStatus::default();
    let mut present = Vec::new();
    for media in &project.media {
        let file = absolute(directory, Path::new(&media.path));
        let id = media.id.to_string();
        if !file.is_file() {
            status.missing.push(id);
        } else if is_lfs_pointer(&file) {
            status.pointers.push(id);
        } else {
            present.push((id, file));
        }
    }
    let files: Vec<PathBuf> = present.iter().map(|(_, file)| file.clone()).collect();
    for ((id, _), external) in present.into_iter().zip(external(directory, &files)) {
        if external {
            status.external.push(id);
        }
    }
    status
}

/// Emit `media_status` for `project` when it differs from the last one the
/// bridge emitted.
pub(crate) fn publish_status(shared: &Shared, tx: &SyncSender<Value>, project: &Project) {
    let status = status(project, &shared.directory);
    let Ok(mut last) = shared.media_status.lock() else {
        return;
    };
    if *last != status {
        emit(tx, status.event());
        *last = status;
    }
}

/// What consolidating a project's media did: the relinks to apply, how
/// many files it copied, and the media it had to leave.
#[derive(Debug, Default)]
pub(crate) struct Consolidation {
    pub relinks: Vec<(Id, String)>,
    pub copied: usize,
    pub skipped: Vec<String>,
}

impl Consolidation {
    /// One edit script relinking every media this moved.
    pub fn script(&self, project: &Project) -> String {
        let ops: Vec<Op> = self
            .relinks
            .iter()
            .map(|(media, path)| Op::RelinkMedia {
                media: media.clone(),
                path: path.clone(),
            })
            .collect();
        kerf_lang::print_ops(project, Catalog::builtin(), &ops)
    }

    /// Lines for a sync report.
    pub fn summary(&self) -> Vec<String> {
        let mut lines = Vec::new();
        if self.copied > 0 {
            lines.push(format!(
                "Copied {} file{} into media/",
                self.copied,
                if self.copied == 1 { "" } else { "s" }
            ));
        }
        let moved = self.relinks.len();
        if moved > 0 {
            lines.push(format!("Relinked {moved} media to the project folder",));
        }
        if moved == 0 && self.skipped.is_empty() {
            lines.push("Every media file is already in the project".into());
        }
        lines.extend(self.skipped.iter().cloned());
        lines
    }
}

/// Copy every media file of `project` that collaborators would not get
/// (outside `directory`, or ignored by Git) into `media/`, and store paths
/// of other files inside it relative to it. Only files `probe` reads as
/// media are copied, so a path the document names cannot carry any other
/// file into the repository; those and offline files are left and
/// reported.
pub(crate) fn consolidate(
    project: &Project,
    directory: &Path,
    probe: impl Fn(&Path) -> Result<(), String>,
) -> Consolidation {
    let mut result = Consolidation::default();
    let mut copies: BTreeMap<PathBuf, String> = BTreeMap::new();
    let files: Vec<PathBuf> = project
        .media
        .iter()
        .map(|media| absolute(directory, Path::new(&media.path)))
        .collect();
    let external = external(directory, &files);
    for ((media, file), external) in project.media.iter().zip(&files).zip(external) {
        if !external && Path::new(&media.path).is_relative() {
            continue;
        }
        if !file.is_file() {
            result
                .skipped
                .push(format!("Skipped {}: {} is offline", media.id, media.path));
            continue;
        }
        if !external {
            if let Some(relative) = relative_to(directory, file) {
                result.relinks.push((media.id.clone(), relative));
            }
            continue;
        }
        let copy = match copies.get(file) {
            Some(copy) => Ok(copy.clone()),
            None => probe(file)
                .and_then(|()| copy_into_media(directory, file).map_err(|error| error.to_string()))
                .inspect(|copy| {
                    result.copied += 1;
                    copies.insert(file.clone(), copy.clone());
                }),
        };
        match copy {
            Ok(copy) => result.relinks.push((media.id.clone(), copy)),
            Err(error) => result
                .skipped
                .push(format!("Skipped {}: {error}", media.id)),
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tests_support::{hermetic, sh, shared};
    use std::fs;
    use std::sync::mpsc;
    use std::time::Duration;

    fn project(media: &[(&str, &str)]) -> Project {
        let mut text = String::from("kerf 1\nproject name=\"Media\"\n");
        for (id, path) in media {
            text.push_str(&format!(
                "media {id} path=\"{path}\" rate=24 dur=48f video=64x64\n"
            ));
        }
        text.push_str("sequence s1 rate=24 size=64x64 audio=48000\n");
        kerf_runtime::parse_document(&text).unwrap()
    }

    /// Every file reads as media.
    fn media(_: &Path) -> Result<(), String> {
        Ok(())
    }

    /// A shared project folder with Kerf's ignore rules.
    fn repository(directory: &Path) {
        hermetic();
        fs::create_dir_all(directory.join("media")).unwrap();
        kerf_collab::scaffold::prepare_repository_files(directory, false).unwrap();
        sh(directory, &["init", "-q", "-b", "main"]);
    }

    #[test]
    fn media_and_projection_commands_are_read() {
        let read = |value: serde_json::Value| serde_json::from_value::<Command>(value);
        let Command::MediaReplace { media, path } =
            read(json!({"type": "media_replace", "media": "m2", "path": "/takes/b.mov"})).unwrap()
        else {
            panic!("expected media_replace");
        };
        assert_eq!((media.as_str(), path.as_str()), ("m2", "/takes/b.mov"));
        assert!(read(json!({"type": "media_replace", "media": "m2"})).is_err());
        assert!(matches!(
            read(json!({"type": "projection_import"})).unwrap(),
            Command::ProjectionImport
        ));
        // It loads onto the current revision; one sent along is ignored.
        assert!(matches!(
            read(json!({"type": "projection_import", "revision": 3})).unwrap(),
            Command::ProjectionImport
        ));
        assert!(matches!(
            read(json!({"type": "projection_repair"})).unwrap(),
            Command::ProjectionRepair
        ));
    }

    #[test]
    fn copies_are_counted_until_they_finish() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Film");
        fs::create_dir_all(project.join("media")).unwrap();
        let shared = Arc::new(shared(&project));
        let first = Copying::start(&shared);
        let second = Copying::start(&shared);
        assert_eq!(shared.copies.load(Ordering::SeqCst), 2);
        drop((first, second));
        assert_eq!(shared.copies.load(Ordering::SeqCst), 0);

        let source = root.path().join("clip.mov");
        fs::write(&source, b"picture").unwrap();
        let (commands, copied) = mpsc::sync_channel(1);
        let (tx, _events) = mpsc::sync_channel(4);
        copy_then(
            &shared,
            &source.to_string_lossy(),
            &commands,
            &tx,
            |_| Ok(()),
            |path| Command::Import { path, media_only: false },
        );
        let Ok(Command::Import { path, .. }) = copied.recv_timeout(Duration::from_secs(10)) else {
            panic!("expected the copy's import");
        };
        assert!(path.starts_with("media/clip-"), "{path}");
        assert_eq!(shared.copies.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn a_file_that_is_not_media_is_never_copied_into_media() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Film");
        fs::create_dir_all(project.join("media")).unwrap();
        let shared = Arc::new(shared(&project));
        let source = root.path().join("notes.txt");
        fs::write(&source, b"my secret notes").unwrap();
        let (commands, imports) = mpsc::sync_channel(1);
        let (tx, events) = mpsc::sync_channel(4);
        copy_then(
            &shared,
            &source.to_string_lossy(),
            &commands,
            &tx,
            |file| Err(format!("{} is text, not media", file.display())),
            |path| Command::Import { path, media_only: false },
        );
        let event = events.recv_timeout(Duration::from_secs(10)).unwrap();
        assert_eq!(event["type"], "error", "{event}");
        assert!(imports.try_recv().is_err());
        assert_eq!(fs::read_dir(project.join("media")).unwrap().count(), 0);
        assert_eq!(shared.copies.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn media_status_names_offline_undownloaded_and_outside_files() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Film");
        fs::create_dir_all(project_dir.join("media")).unwrap();
        fs::write(project_dir.join("media/here.mov"), b"x").unwrap();
        fs::write(
            project_dir.join("media/pointer.mov"),
            "version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 9\n",
        )
        .unwrap();
        let outside = root.path().join("outside.mov");
        fs::write(&outside, b"x").unwrap();
        let inside_absolute = project_dir.join("media/here.mov");
        let project = project(&[
            ("m1", "media/here.mov"),
            ("m2", "media/gone.mov"),
            ("m3", "media/pointer.mov"),
            ("m4", &outside.to_string_lossy()),
            ("m5", &inside_absolute.to_string_lossy()),
            ("m6", &root.path().join("lost.mov").to_string_lossy()),
        ]);
        assert_eq!(
            status(&project, &project_dir),
            MediaStatus {
                missing: vec!["m2".into(), "m6".into()],
                pointers: vec!["m3".into()],
                external: vec!["m4".into()],
            }
        );
    }

    #[test]
    fn media_git_ignores_is_external_and_consolidated_into_media() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("My Film");
        repository(&project_dir);
        // A render beside project.kerf, which the shared .gitignore keeps
        // out of the repository, and a shot that is shared.
        fs::write(project_dir.join("interview.mp4"), b"render").unwrap();
        fs::write(project_dir.join("shot.mov"), b"shot").unwrap();
        let project = project(&[("m1", "interview.mp4"), ("m2", "shot.mov")]);
        assert_eq!(status(&project, &project_dir).external, ["m1"]);

        let result = consolidate(&project, &project_dir, media);
        assert_eq!(result.copied, 1, "{:?}", result.skipped);
        let [(id, copy)] = result.relinks.as_slice() else {
            panic!("expected one relink: {:?}", result.relinks);
        };
        assert_eq!(id.as_str(), "m1");
        assert!(copy.starts_with("media/interview-"), "{copy}");
        assert_eq!(fs::read(project_dir.join(copy)).unwrap(), b"render");
        let relinked =
            kerf_lang::apply_script(&project, Catalog::builtin(), &result.script(&project))
                .unwrap();
        assert!(status(&relinked.project, &project_dir).external.is_empty());

        // Added to the repository on purpose, it is shared after all.
        sh(&project_dir, &["add", "-f", "interview.mp4"]);
        assert!(status(&project, &project_dir).external.is_empty());
    }

    #[test]
    fn consolidating_copies_outside_files_once_and_relinks_them() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Film");
        fs::create_dir_all(project_dir.join("media")).unwrap();
        fs::write(project_dir.join("inside.mov"), b"in").unwrap();
        let outside = root.path().join("outside.mov");
        fs::write(&outside, b"out").unwrap();
        let outside_path = outside.to_string_lossy().into_owned();
        let inside_path = project_dir
            .join("inside.mov")
            .to_string_lossy()
            .into_owned();
        let lost_path = root.path().join("lost.mov").to_string_lossy().into_owned();
        let project = project(&[
            ("m1", "media/kept.mov"),
            ("m2", &outside_path),
            ("m3", &outside_path),
            ("m4", &inside_path),
            ("m5", &lost_path),
        ]);
        let result = consolidate(&project, &project_dir, media);
        assert_eq!(result.copied, 1);
        let relinks: Vec<(String, String)> = result
            .relinks
            .iter()
            .map(|(id, path)| (id.to_string(), path.clone()))
            .collect();
        let copy = relinks[0].1.clone();
        assert!(
            copy.starts_with("media/outside-") && copy.ends_with(".mov"),
            "{copy}"
        );
        assert_eq!(fs::read(project_dir.join(&copy)).unwrap(), b"out");
        assert_eq!(
            relinks,
            vec![
                ("m2".into(), copy.clone()),
                ("m3".into(), copy.clone()),
                ("m4".into(), "inside.mov".into()),
            ]
        );
        assert_eq!(result.skipped.len(), 1);
        assert!(result.skipped[0].contains("m5"), "{:?}", result.skipped);
        let script = result.script(&project);
        assert!(
            script.contains(&format!("media relink m2 path=\"{copy}\"")),
            "{script}"
        );
        let relinked = kerf_lang::apply_script(&project, Catalog::builtin(), &script).unwrap();
        assert_eq!(relinked.project.media("m4").unwrap().path, "inside.mov");
    }

    #[test]
    fn consolidating_never_copies_a_file_that_is_not_media() {
        let root = tempfile::tempdir().unwrap();
        let project_dir = root.path().join("Film");
        fs::create_dir_all(project_dir.join("media")).unwrap();
        // A collaborator names a secret where media belongs.
        let secret = root.path().join("config.json");
        fs::write(&secret, br#"{"token": "secret"}"#).unwrap();
        let clip = root.path().join("clip.mov");
        fs::write(&clip, b"picture").unwrap();
        let project = project(&[
            ("m1", &secret.to_string_lossy()),
            ("m2", &clip.to_string_lossy()),
        ]);
        let probe = |file: &Path| match file.extension().and_then(|e| e.to_str()) {
            Some("mov") => Ok(()),
            _ => Err(format!("{} is not media", file.display())),
        };
        let result = consolidate(&project, &project_dir, probe);
        assert_eq!(result.copied, 1);
        assert_eq!(result.relinks.len(), 1);
        assert_eq!(result.relinks[0].0.as_str(), "m2");
        assert_eq!(result.skipped.len(), 1, "{:?}", result.skipped);
        assert!(
            result.skipped[0].starts_with("Skipped m1: "),
            "{:?}",
            result.skipped
        );
        let copied: Vec<_> = fs::read_dir(project_dir.join("media"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(copied.len(), 1, "{copied:?}");
        assert!(copied[0].starts_with("clip-"), "{copied:?}");
        // What the editor passes in reads a file the way an import does.
        assert!(crate::tasks::probe(&secret).is_err());
    }

    #[test]
    fn text_that_ffprobe_draws_as_a_picture_is_not_media() {
        let root = tempfile::tempdir().unwrap();
        for (name, text) in [
            ("secret-key.asc", "-----BEGIN PGP PRIVATE KEY BLOCK-----\n"),
            ("passwords.txt", "my secret notes\n"),
        ] {
            let path = root.path().join(name);
            fs::write(&path, text.repeat(64)).unwrap();
            let Ok(info) = kerf_engine::info(&path.to_string_lossy()) else {
                // Without ffprobe nothing reads as media anyway.
                continue;
            };
            assert!(
                crate::tasks::probed_media(&path, &info).is_err(),
                "{name} read as media: {info:?}"
            );
        }
    }
}
