//! What Create and Clone are asked for, and why they cannot be yet: the
//! plan of a new project, the folder it gets and where, as a person writes
//! them, and the Git addresses a clone is made from.

use crate::prefs::{self, Prefs};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// What Create asks for, as the form says it.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plan {
    pub name: String,
    pub location: PathBuf,
    /// `standard`, `vertical` or `square`.
    pub template: String,
    pub rate: String,
    pub git: bool,
    /// A private GitHub repository, made with the GitHub CLI.
    pub github: bool,
    /// A Git address to push to; empty for none.
    pub remote: String,
    /// The kerfd the editor was started with.
    pub kerfd: Option<PathBuf>,
    /// `{path, bindings, texts?}` when it starts from a preset.
    pub preset: Option<Value>,
}

impl Plan {
    /// The bridge's `create` task for it: GitHub or a remote turn Git on,
    /// GitHub wins over a remote, and a kerfd named by its path is passed
    /// on (a bare name is looked up as the bridge would anyway).
    pub fn request(&self) -> Value {
        let remote = self.remote.trim();
        let mut request = json!({
            "type": "create",
            "name": self.name.trim(),
            "location": self.location.to_string_lossy(),
            "template": self.template,
            "rate": self.rate,
            "git": self.git || self.github || !remote.is_empty(),
        });
        if self.github {
            request["github"] = json!({"private": true});
        } else if !remote.is_empty() {
            request["remote"] = json!(remote);
        }
        if let Some(kerfd) = self
            .kerfd
            .as_ref()
            .filter(|kerfd| kerfd.components().count() > 1)
        {
            let kerfd = std::path::absolute(kerfd).unwrap_or_else(|_| kerfd.clone());
            request["kerfd"] = json!(kerfd.to_string_lossy());
        }
        if let Some(preset) = &self.preset {
            request["preset"] = preset.clone();
        }
        request
    }
}

/// Why a new project named `name` cannot be made in `location`, pushed to
/// `remote` (empty for none); None when it can.
pub fn details_problem(name: &str, location: &Path, remote: &str) -> Option<String> {
    let folder = folder_name(name);
    if folder.is_empty() {
        return Some("Name the project".into());
    }
    if location.as_os_str().is_empty() {
        return Some("Choose where to save the project".into());
    }
    if !location.is_absolute() {
        return Some(format!(
            "Choose a full location, such as {}",
            home_relative(&default_videos())
        ));
    }
    if occupied(&location.join(&folder)) {
        return Some(format!(
            "“{folder}” is already in that location; choose another name"
        ));
    }
    let remote = remote.trim();
    if !remote.is_empty() && !git_address(remote) {
        return Some("The remote must be a Git address".into());
    }
    None
}

/// Why the repository at `url` cannot be cloned into `location`; None
/// when it can.
pub fn clone_problem(url: &str, location: &Path) -> Option<String> {
    let url = url.trim();
    if url.is_empty() {
        return Some("Enter the repository's address".into());
    }
    let Some(folder) = repository_name(url).filter(|_| git_address(url)) else {
        return Some("Enter a Git address such as https://github.com/you/film.git".into());
    };
    if location.as_os_str().is_empty() || !location.is_absolute() {
        return Some("Choose where to put the copy".into());
    }
    if occupied(&location.join(&folder)) {
        return Some(format!("“{folder}” is already in that location"));
    }
    None
}

/// Whether `url` reads as an address Git clones from.
pub fn git_address(url: &str) -> bool {
    let url = url.trim();
    if ["https://", "http://", "ssh://", "git://", "file://"]
        .iter()
        .any(|scheme| url.starts_with(scheme))
    {
        return url
            .split("://")
            .nth(1)
            .is_some_and(|rest| rest.contains('/'));
    }
    // user@host:path, as SSH addresses are written.
    match (url.find('@'), url.find(':')) {
        (Some(at), Some(colon)) => {
            at > 0 && colon > at + 1 && colon + 1 < url.len() && !url.contains(' ')
        }
        _ => false,
    }
}

/// The folder a clone of `url` gets: its last part without `.git`, as Git
/// names it.
pub(super) fn repository_name(url: &str) -> Option<String> {
    let trimmed = url.trim().trim_end_matches('/');
    let last = trimmed.rsplit(['/', ':']).next()?;
    let name = last.strip_suffix(".git").unwrap_or(last);
    (!name.is_empty() && name != trimmed).then(|| name.to_owned())
}

/// The folder a project named `name` gets, as kerf-collab names it: the
/// name without the characters file systems refuse.
pub(super) fn folder_name(name: &str) -> String {
    let cleaned: String = name
        .trim()
        .chars()
        .map(|c| {
            if matches!(c, '/' | '\\' | ':' | '\0') {
                '-'
            } else {
                c
            }
        })
        .collect();
    cleaned.trim_matches('.').trim().to_owned()
}

/// The last part of `path`, else all of it.
pub(super) fn folder_of(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| path.display().to_string())
}

/// `path` as a person writes it: `~/Videos/Kerf` under the home folder.
pub(crate) fn home_relative(path: &Path) -> String {
    if let Some(rest) =
        prefs::home().and_then(|home| path.strip_prefix(home).ok().map(Path::to_path_buf))
    {
        if rest.as_os_str().is_empty() {
            return "~".into();
        }
        return format!("~{}{}", std::path::MAIN_SEPARATOR, rest.display());
    }
    path.display().to_string()
}

/// A path as typed, with `~` for the home folder.
pub(super) fn expanded(text: &str) -> PathBuf {
    let home = prefs::home();
    match (text.strip_prefix('~'), home) {
        (Some(""), Some(home)) => home,
        (Some(rest), Some(home)) if rest.starts_with(['/', '\\']) => home.join(&rest[1..]),
        _ => PathBuf::from(text),
    }
}

/// The Kerf folder among the person's videos.
fn default_videos() -> PathBuf {
    let home = prefs::home().unwrap_or_else(|| PathBuf::from("."));
    let videos = if cfg!(target_os = "macos") {
        "Movies"
    } else {
        "Videos"
    };
    home.join(videos).join("Kerf")
}

/// Where new projects go: beside the last one opened, else a Kerf folder
/// among the person's videos.
pub(super) fn default_location(prefs: &Prefs) -> PathBuf {
    if let Some(folder) = prefs
        .recent
        .iter()
        .filter_map(|project| project.parent())
        .find(|folder| folder.is_dir())
    {
        return folder.to_path_buf();
    }
    default_videos()
}

/// The first of Untitled, Untitled 2, … free in `location`.
pub(super) fn untitled(location: &Path) -> String {
    let mut name = "Untitled".to_owned();
    for n in 2..1000 {
        if !occupied(&location.join(&name)) {
            break;
        }
        name = format!("Untitled {n}");
    }
    name
}

/// A folder a new project or a clone cannot go into: a file, or a folder
/// with anything in it.
fn occupied(path: &Path) -> bool {
    if !path.exists() {
        return false;
    }
    !path.is_dir()
        || std::fs::read_dir(path)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
}
