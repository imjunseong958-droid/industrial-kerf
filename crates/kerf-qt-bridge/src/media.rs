//! A shared project's media files: copies into `media/` named by what they
//! hold, so every collaborator's copy of one file has one name, and which
//! media are offline, not downloaded, or outside the project.

use crate::{emit, Command, Shared};
use kerf_collab::git::is_lfs_pointer;
use kerf_collab::scaffold::MEDIA_DIR;
use kerf_model::{Catalog, Id, Op, Project};
use serde::Serialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::thread;

/// Hex digits of the SHA-256 a copy's name carries. Longer only when a
/// different file already holds the shorter name.
const NAME_DIGITS: [usize; 5] = [8, 12, 16, 32, 64];

/// Whether importing `path` into the project in `directory` copies the file
/// into `media/` first: the project keeps its media there (it has the
/// folder, or it is a repository whose clone has not made it yet) and the
/// file is outside the project.
pub(crate) fn needs_copy(directory: &Path, path: &Path) -> bool {
    let shares_media = directory.join(MEDIA_DIR).is_dir() || directory.join(".git").exists();
    shares_media && !inside(directory, &absolute(directory, path))
}

fn absolute(directory: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        directory.join(path)
    }
}

/// Whether `file` is inside `directory`, comparing folders as they resolve
/// (macOS's `/var` is `/private/var`); the file's own name is kept even
/// when it is a link.
pub(crate) fn inside(directory: &Path, file: &Path) -> bool {
    match (directory.canonicalize(), resolved_parent(file)) {
        (Ok(directory), Some(parent)) => parent.starts_with(directory),
        _ => false,
    }
}

fn resolved_parent(file: &Path) -> Option<PathBuf> {
    file.parent()
        .map(|parent| {
            if parent.as_os_str().is_empty() {
                Path::new(".")
            } else {
                parent
            }
        })
        .and_then(|parent| parent.canonicalize().ok())
}

/// `file` relative to `directory` with `/` separators, when it is inside.
fn relative_to(directory: &Path, file: &Path) -> Option<String> {
    let directory = directory.canonicalize().ok()?;
    let path = resolved_parent(file)?.join(file.file_name()?);
    let relative = path.strip_prefix(directory).ok()?;
    let parts: Option<Vec<&str>> = relative
        .components()
        .map(|component| match component {
            Component::Normal(part) => part.to_str(),
            _ => None,
        })
        .collect();
    parts
        .filter(|parts| !parts.is_empty())
        .map(|parts| parts.join("/"))
}

/// Copy the file at `path` into the project's `media/` folder on its own
/// thread, then hand the runtime worker `then(copy)`, which names the copy
/// by its project-relative path. A copy that fails is reported.
pub(crate) fn copy_then(
    directory: &Path,
    path: &str,
    commands: &SyncSender<Command>,
    tx: &SyncSender<Value>,
    then: impl FnOnce(String) -> Command + Send + 'static,
) {
    let source = absolute(directory, Path::new(path));
    let directory = directory.to_owned();
    let (commands, tx) = (commands.clone(), tx.clone());
    thread::spawn(move || match copy_into_media(&directory, &source) {
        Ok(copy) => {
            let _ = commands.send(then(copy));
        }
        Err(error) => crate::error(
            &tx,
            format!("cannot copy {} into media/: {error}", source.display()),
        ),
    });
}

/// Copy `source` into the project's `media/` folder as
/// `<stem>-<8 hex of its SHA-256>.<ext>`, hashing while copying, and return
/// the copy's project-relative path. A file already there with the same
/// content is reused, so two people adding one file make one path; a
/// different file never takes its name.
pub(crate) fn copy_into_media(directory: &Path, source: &Path) -> io::Result<String> {
    let mut input = File::open(source)?;
    if !input.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a file", source.display()),
        ));
    }
    if is_lfs_pointer(source) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "{} is a Git LFS pointer, not the media itself; download it (git lfs pull) first",
                source.display()
            ),
        ));
    }
    let folder = directory.join(MEDIA_DIR);
    fs::create_dir_all(&folder)?;
    let temporary = folder.join(format!(".kerf-copy-{}.part", crate::key()));
    let copied = File::create(&temporary).and_then(|output| copy_hashing(&mut input, output));
    let (digest, length) = match copied {
        Ok(copied) => copied,
        Err(error) => {
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    let result = place(&folder, &temporary, source, &digest, length);
    let _ = fs::remove_file(&temporary);
    result.map(|name| format!("{MEDIA_DIR}/{name}"))
}

/// Give the finished copy its content name, or reuse the identical file
/// that has it. Returns the file name.
fn place(
    folder: &Path,
    temporary: &Path,
    source: &Path,
    digest: &str,
    length: u64,
) -> io::Result<String> {
    for digits in NAME_DIGITS {
        let name = content_name(source, &digest[..digits]);
        let target = folder.join(&name);
        if target.exists() {
            if same_content(&target, digest, length)? {
                return Ok(name);
            }
            continue;
        }
        // A hard link never replaces a file another process just made;
        // file systems without links take a rename instead.
        match fs::hard_link(temporary, &target) {
            Ok(()) => return Ok(name),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if same_content(&target, digest, length)? {
                    return Ok(name);
                }
            }
            Err(_) => {
                fs::rename(temporary, &target)?;
                return Ok(name);
            }
        }
    }
    Err(io::Error::other(format!(
        "no free name for {} in media/",
        source.display()
    )))
}

/// The name a copy of `source` gets: its stem, the hash, its extension. A
/// stem that already ends in this hash (a file from another project's
/// `media/`) keeps one.
pub(crate) fn content_name(source: &Path, hash: &str) -> String {
    let stem = source
        .file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stem: String = stem
        .trim()
        .chars()
        .map(|c| {
            if c.is_control() || matches!(c, '/' | '\\' | ':') {
                '-'
            } else {
                c
            }
        })
        .collect();
    let stem = stem.trim_start_matches('.');
    let stem = stem
        .strip_suffix(&format!("-{hash}"))
        .unwrap_or(stem)
        .trim();
    let stem = if stem.is_empty() { "media" } else { stem };
    match source.extension() {
        Some(extension) => format!("{stem}-{hash}.{}", extension.to_string_lossy()),
        None => format!("{stem}-{hash}"),
    }
}

fn copy_hashing(input: &mut File, mut output: File) -> io::Result<(String, u64)> {
    let mut hasher = Sha256::new();
    let mut buffer = vec![0; 1 << 20];
    let mut length = 0u64;
    loop {
        let read = match input.read(&mut buffer) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        };
        hasher.update(&buffer[..read]);
        output.write_all(&buffer[..read])?;
        length += read as u64;
    }
    output.sync_all()?;
    Ok((hex(&hasher.finalize()), length))
}

fn hash_file(path: &Path) -> io::Result<String> {
    let mut input = File::open(path)?;
    let mut hasher = Sha256::new();
    io::copy(&mut input, &mut hasher)?;
    Ok(hex(&hasher.finalize()))
}

fn same_content(path: &Path, digest: &str, length: u64) -> io::Result<bool> {
    if fs::metadata(path)?.len() != length {
        return Ok(false);
    }
    Ok(hash_file(path)? == digest)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Media that cannot play as the project names them: offline (no file),
/// Git LFS pointers (the file was not downloaded), and files outside the
/// project folder, which collaborators do not have. Each media id is in at
/// most one list, in that order.
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
    for media in &project.media {
        let file = absolute(directory, Path::new(&media.path));
        let id = media.id.to_string();
        if !file.is_file() {
            status.missing.push(id);
        } else if is_lfs_pointer(&file) {
            status.pointers.push(id);
        } else if !inside(directory, &file) {
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

/// Copy every media file of `project` (as the document names it) that is
/// outside `directory` into `media/`, and store paths of files already
/// inside it relative to it. Offline files are left and reported.
pub(crate) fn consolidate(project: &Project, directory: &Path) -> Consolidation {
    let mut result = Consolidation::default();
    let mut copies: BTreeMap<PathBuf, String> = BTreeMap::new();
    for media in &project.media {
        let path = Path::new(&media.path);
        if path.is_relative() {
            continue;
        }
        if !path.is_file() {
            result
                .skipped
                .push(format!("Skipped {}: {} is offline", media.id, media.path));
            continue;
        }
        if let Some(relative) = relative_to(directory, path) {
            result.relinks.push((media.id.clone(), relative));
            continue;
        }
        let copy = match copies.get(path) {
            Some(copy) => Ok(copy.clone()),
            None => copy_into_media(directory, path).inspect(|copy| {
                result.copied += 1;
                copies.insert(path.to_owned(), copy.clone());
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
    fn copies_are_named_by_content_and_reused() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Film");
        fs::create_dir_all(project.join("media")).unwrap();
        let outside = root.path().join("Downloads");
        fs::create_dir_all(&outside).unwrap();
        let source = outside.join("Beach Day.mov");
        fs::write(&source, b"the same picture").unwrap();

        assert!(needs_copy(&project, &source));
        let first = copy_into_media(&project, &source).unwrap();
        let hash = hex(&Sha256::digest(b"the same picture"));
        assert_eq!(first, format!("media/Beach Day-{}.mov", &hash[..8]));
        assert_eq!(fs::read(project.join(&first)).unwrap(), b"the same picture");
        assert!(!needs_copy(&project, &project.join(&first)));
        assert!(!needs_copy(&project, Path::new(&first)));

        // The same file from somewhere else lands on the same name.
        let again = root.path().join("Beach Day.mov");
        fs::write(&again, b"the same picture").unwrap();
        assert_eq!(copy_into_media(&project, &again).unwrap(), first);
        let names: Vec<_> = fs::read_dir(project.join("media"))
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(names.len(), 1, "no temporary files are left: {names:?}");

        // A different file with the same name gets its own.
        fs::write(&again, b"another picture").unwrap();
        let other = copy_into_media(&project, &again).unwrap();
        assert_ne!(other, first);
        assert!(other.starts_with("media/Beach Day-") && other.ends_with(".mov"));
    }

    #[test]
    fn a_different_file_never_takes_a_name_that_is_held() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("clip.mp4");
        fs::write(&source, b"mine").unwrap();
        let hash = hex(&Sha256::digest(b"mine"));
        // Someone else's file already holds the short name.
        fs::create_dir_all(root.path().join("p/media")).unwrap();
        let held = root.path().join(format!("p/media/clip-{}.mp4", &hash[..8]));
        fs::write(&held, b"theirs").unwrap();
        let copy = copy_into_media(&root.path().join("p"), &source).unwrap();
        assert_eq!(copy, format!("media/clip-{}.mp4", &hash[..12]));
        assert_eq!(fs::read(&held).unwrap(), b"theirs");
        assert_eq!(fs::read(root.path().join("p").join(copy)).unwrap(), b"mine");
    }

    #[test]
    fn content_names_keep_one_hash() {
        assert_eq!(
            content_name(Path::new("/a/clip-0123abcd.mov"), "0123abcd"),
            "clip-0123abcd.mov"
        );
        assert_eq!(
            content_name(Path::new("/a/notes"), "0123abcd"),
            "notes-0123abcd"
        );
        assert_eq!(
            content_name(Path::new("/a/ .mov"), "0123abcd"),
            "media-0123abcd.mov"
        );
        assert_eq!(
            content_name(Path::new("/a/.hidden"), "0123abcd"),
            "hidden-0123abcd"
        );
    }

    #[test]
    fn only_shared_projects_copy_files_from_outside() {
        let root = tempfile::tempdir().unwrap();
        let source = root.path().join("a.mov");
        fs::write(&source, b"x").unwrap();
        let plain = root.path().join("plain");
        fs::create_dir_all(&plain).unwrap();
        assert!(!needs_copy(&plain, &source));
        fs::create_dir_all(plain.join(".git")).unwrap();
        assert!(needs_copy(&plain, &source));
    }

    #[test]
    fn copying_a_git_lfs_pointer_is_refused() {
        let root = tempfile::tempdir().unwrap();
        let pointer = root.path().join("big.mov");
        fs::write(
            &pointer,
            "version https://git-lfs.github.com/spec/v1\noid sha256:00\nsize 9\n",
        )
        .unwrap();
        let error = copy_into_media(&root.path().join("p"), &pointer).unwrap_err();
        assert!(error.to_string().contains("Git LFS pointer"), "{error}");
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
        let result = consolidate(&project, &project_dir);
        assert_eq!(result.copied, 1);
        let hash = hex(&Sha256::digest(b"out"));
        let copy = format!("media/outside-{}.mov", &hash[..8]);
        let relinks: Vec<(String, String)> = result
            .relinks
            .iter()
            .map(|(id, path)| (id.to_string(), path.clone()))
            .collect();
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
}
