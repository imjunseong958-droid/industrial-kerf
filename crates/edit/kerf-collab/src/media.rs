//! A shared project's media files: copies into `media/` named by what they
//! hold, so every collaborator's copy of one file has one name, and which
//! files the repository would not carry to collaborators.

use crate::git::is_lfs_pointer;
use crate::scaffold::MEDIA_DIR;
use sha2::{Digest, Sha256};
use std::collections::BTreeSet;
use std::fs::{self, File};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Where files are written before they take their place in the project.
/// Git ignores it (`.kerf/cache/` is in the shared `.gitignore` and in every
/// repository's `info/exclude`), so a half-written file is never committed,
/// and it is on the project's volume, so a finished file moves in by name.
pub const INCOMING_DIR: &str = ".kerf/cache/incoming";

/// Hex digits of the SHA-256 a copy's name carries. Longer only when a
/// different file already holds the shorter name.
const NAME_DIGITS: [usize; 5] = [8, 12, 16, 32, 64];

/// Whether the project in `directory` keeps its media in `media/`: it has
/// the folder, or it is a repository whose clone has not made it yet.
pub fn shares_media(directory: &Path) -> bool {
    directory.join(MEDIA_DIR).is_dir() || directory.join(".git").exists()
}

/// Whether importing `path` into the project in `directory` copies the file
/// into `media/` first: the project keeps its media there, and the file is
/// [`external`] where it is.
pub fn needs_copy(directory: &Path, path: &Path) -> bool {
    shares_media(directory) && external(directory, &[absolute(directory, path)])[0]
}

/// `path` read against the project folder when it is relative.
pub fn absolute(directory: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_owned()
    } else {
        directory.join(path)
    }
}

/// Which of `files` collaborators would not get from the project in
/// `directory`: files outside the project folder, and files in it that Git
/// ignores, such as a render beside `project.kerf` (but not in `media/`,
/// where a copy would be ignored too).
pub fn external(directory: &Path, files: &[PathBuf]) -> Vec<bool> {
    let media = format!("{MEDIA_DIR}/");
    let relative: Vec<Option<String>> = files
        .iter()
        .map(|file| relative_to(directory, file))
        .collect();
    let candidates: Vec<&str> = relative
        .iter()
        .flatten()
        .map(String::as_str)
        .filter(|path| !path.starts_with(&media))
        .collect();
    let ignored = ignored(directory, &candidates);
    files
        .iter()
        .zip(&relative)
        .map(|(file, relative)| match relative {
            Some(relative) => ignored.contains(relative),
            None => !inside(directory, file),
        })
        .collect()
}

/// Which of `paths` (relative to `directory`, with `/` separators) Git
/// ignores in the repository at `directory`, as given. Tracked files are
/// not ignored; nothing is when `directory` is not the top of a repository.
fn ignored(directory: &Path, paths: &[&str]) -> BTreeSet<String> {
    if paths.is_empty() || !directory.join(".git").exists() {
        return BTreeSet::new();
    }
    let child = crate::git::program("git")
        .arg("-C")
        .arg(directory)
        .args(["check-ignore", "-z", "--stdin"])
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return BTreeSet::new();
    };
    let mut input = Vec::new();
    for path in paths {
        input.extend_from_slice(path.as_bytes());
        input.push(0);
    }
    // Written beside the read, so a long answer never blocks the question.
    let writer = child.stdin.take().map(|mut stdin| {
        thread::spawn(move || {
            // Git that stopped reading has failed, which its status says.
            let _ = stdin.write_all(&input);
        })
    });
    let output = child.wait_with_output();
    if let Some(writer) = writer {
        // Its only work is the write above, which does not panic.
        let _ = writer.join();
    }
    // 0 when something is ignored, 1 when nothing is.
    match output {
        Ok(output) if matches!(output.status.code(), Some(0 | 1)) => output
            .stdout
            .split(|byte| *byte == 0)
            .filter(|path| !path.is_empty())
            .map(|path| String::from_utf8_lossy(path).into_owned())
            .collect(),
        _ => BTreeSet::new(),
    }
}

/// Whether `file` is inside `directory`, comparing folders as they resolve
/// (macOS's `/var` is `/private/var`); the file's own name is kept even
/// when it is a link.
pub fn inside(directory: &Path, file: &Path) -> bool {
    match (kerf_paths::canonicalize(directory), resolved_parent(file)) {
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
        .and_then(|parent| kerf_paths::canonicalize(parent).ok())
}

/// `file` relative to `directory` with `/` separators, when it is inside.
pub fn relative_to(directory: &Path, file: &Path) -> Option<String> {
    let directory = kerf_paths::canonicalize(directory).ok()?;
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

/// A new path in the project's [`INCOMING_DIR`], which is made when
/// missing, ending in `suffix`. What interrupted copies left there is
/// cleared first (see [`sweep`]).
pub fn incoming_path(directory: &Path, suffix: &str) -> io::Result<PathBuf> {
    let folder = directory.join(INCOMING_DIR);
    fs::create_dir_all(&folder)?;
    sweep(&folder, SystemTime::now());
    Ok(folder.join(format!("{}{suffix}", unique())))
}

/// How long an entry in [`INCOMING_DIR`] goes unwritten before it is taken
/// for one an interrupted copy left.
const STALE: Duration = Duration::from_secs(24 * 60 * 60);

/// Remove the entries in `folder` that another process's [`incoming_path`]
/// made and nothing has written for [`STALE`] by `now`: an app quit mid-copy
/// leaves them, as a download an older `kerf <URL>` stopped part way left a
/// folder, and no one else would. A copy in progress keeps writing, so it is
/// never stale.
fn sweep(folder: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(folder) else {
        return;
    };
    let ours = format!("{}-", std::process::id());
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        if !is_unique(name) || name.starts_with(&ours) {
            continue;
        }
        let path = entry.path();
        let Ok(metadata) = fs::symlink_metadata(&path) else {
            continue;
        };
        let stale = last_written(&path, &metadata)
            .and_then(|at| now.duration_since(at).ok())
            .is_some_and(|age| age > STALE);
        if stale {
            // One that stays is tried again next time.
            let _ = if metadata.is_dir() {
                fs::remove_dir_all(&path)
            } else {
                fs::remove_file(&path)
            };
        }
    }
}

/// When `path` was last written: for a folder (an older `kerf <URL>`
/// downloaded into one), whatever directly in it was written last.
fn last_written(path: &Path, metadata: &fs::Metadata) -> Option<SystemTime> {
    let mut latest = metadata.modified().ok()?;
    if metadata.is_dir() {
        for entry in fs::read_dir(path).ok()?.flatten() {
            // not the listing's own metadata, which Windows may not have brought up to date
            // for a file still open for writing
            if let Ok(modified) = fs::symlink_metadata(entry.path()).and_then(|m| m.modified()) {
                latest = latest.max(modified);
            }
        }
    }
    Some(latest)
}

/// Whether `name` begins as [`unique`] makes it: `<pid>-<nanos>-<sequence>`.
fn is_unique(name: &str) -> bool {
    let number = |part: Option<&str>| {
        part.is_some_and(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
    };
    let mut parts = name.splitn(3, '-');
    number(parts.next())
        && number(parts.next())
        && parts
            .next()
            .is_some_and(|rest| rest.starts_with(|c: char| c.is_ascii_digit()))
}

fn unique() -> String {
    static SEQUENCE: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| since.as_nanos());
    format!(
        "{}-{nanos}-{}",
        std::process::id(),
        SEQUENCE.fetch_add(1, Ordering::Relaxed)
    )
}

/// Copy `source` into the project's `media/` folder as
/// `<stem>-<8 hex of its SHA-256>.<ext>`, hashing while copying, and return
/// the copy's project-relative path. The copy is written in
/// [`INCOMING_DIR`] until it is whole. A file already there with the same
/// content is reused, so two people adding one file make one path; a
/// different file never takes its name.
pub fn copy_into_media(directory: &Path, source: &Path) -> io::Result<String> {
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
    let temporary = incoming_path(directory, ".part")?;
    let copied = File::create(&temporary).and_then(|output| copy_hashing(&mut input, output));
    let (digest, length) = match copied {
        Ok(copied) => copied,
        Err(error) => {
            // One that stays in the incoming folder is swept as stale.
            let _ = fs::remove_file(&temporary);
            return Err(error);
        }
    };
    let result = place(&folder, &temporary, source, &digest, length);
    // Gone already where it was moved into place; as above, one that stays
    // is swept.
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
        // A hard link never replaces a file another process just made.
        match fs::hard_link(temporary, &target) {
            Ok(()) => return Ok(name),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {
                if same_content(&target, digest, length)? {
                    return Ok(name);
                }
            }
            Err(_) => {
                move_into_place(temporary, &target)?;
                return Ok(name);
            }
        }
    }
    Err(io::Error::other(format!(
        "no free name for {} in media/",
        source.display()
    )))
}

/// Move a file to `target` where hard links fail: by name on file systems
/// without links, and by a copy beside the target when `media/` is on
/// another volume (a link to a drive, which Git does not look into).
fn move_into_place(temporary: &Path, target: &Path) -> io::Result<()> {
    if fs::rename(temporary, target).is_ok() {
        return Ok(());
    }
    let beside = target.with_file_name(format!(".kerf-copy-{}.part", unique()));
    let moved = fs::copy(temporary, &beside).and_then(|_| fs::rename(&beside, target));
    if moved.is_err() {
        // There may be none; the move's own error is what to tell.
        let _ = fs::remove_file(&beside);
    }
    moved
}

/// The name a copy of `source` gets: its stem, the hash, its extension. A
/// stem that already ends in this hash (a file from another project's
/// `media/`) keeps one.
pub fn content_name(source: &Path, hash: &str) -> String {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{git, scaffold};
    use std::process::Command;

    fn names(folder: &Path) -> Vec<std::ffi::OsString> {
        fs::read_dir(folder)
            .map(|entries| entries.map(|entry| entry.unwrap().file_name()).collect())
            .unwrap_or_default()
    }

    /// A repository with Kerf's ignore rules, nothing committed.
    fn repository(directory: &Path) {
        fs::create_dir_all(directory.join(MEDIA_DIR)).unwrap();
        scaffold::prepare_repository_files(directory, false).unwrap();
        git::init(directory).unwrap();
    }

    fn untracked(directory: &Path) -> String {
        let output = Command::new("git")
            .arg("-C")
            .arg(directory)
            .args(["status", "--porcelain", "--untracked-files=all"])
            .output()
            .unwrap();
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout).into_owned()
    }

    #[test]
    fn copies_in_progress_are_written_where_git_never_stages_them() {
        let root = tempfile::tempdir().unwrap();
        let project = root.path().join("Film");
        repository(&project);
        let before = untracked(&project);
        // A half-written copy sits in the incoming folder until it is
        // whole, never in media/, so a sync meanwhile does not commit it.
        let partial = incoming_path(&project, ".part").unwrap();
        assert!(
            partial.starts_with(project.join(".kerf/cache")),
            "{partial:?}"
        );
        fs::write(&partial, b"half a picture").unwrap();
        assert_eq!(untracked(&project), before);
        assert!(names(&project.join("media")).is_empty());

        let source = root.path().join("clip.mov");
        fs::write(&source, b"picture").unwrap();
        let copy = copy_into_media(&project, &source).unwrap();
        assert_eq!(names(&project.join(INCOMING_DIR)).len(), 1, "only ours");
        assert!(untracked(&project).contains(&copy));

        // Where the incoming folder cannot be made, no partial copy goes
        // into media/ instead.
        let blocked = root.path().join("Blocked");
        fs::create_dir_all(blocked.join("media")).unwrap();
        fs::create_dir_all(blocked.join(".kerf")).unwrap();
        fs::write(blocked.join(".kerf/cache"), "").unwrap();
        assert!(copy_into_media(&blocked, &source).is_err());
        assert!(names(&blocked.join("media")).is_empty());
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
}
