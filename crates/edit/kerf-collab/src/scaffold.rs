//! New project folders: the document, the media and preset folders, and,
//! for a shared project, the repository and its ignore and attribute files.

use crate::git::{self, CollabError, Result};
use kerf_lang::{print_document, PrintOptions};
use kerf_model::{Catalog, Id, Project, Rate, Sequence, Time};
use serde::Deserialize;
use std::fs;
use std::path::{Path, PathBuf};

/// Where a shared project keeps its media, so every collaborator has the
/// same file behind each media id.
pub const MEDIA_DIR: &str = "media";
/// The project's preset library, shared with the project.
pub const PRESETS_DIR: &str = "presets";

/// Settings that belong to one machine and may hold credentials or name
/// programs to run, relative to the project. They are never staged, and a
/// branch that tracks them is never checked out.
pub const LOCAL_SETTINGS: [&str; 3] = [".kerf/config.json", ".kerf/mcp.json", ".kerf/vision.json"];

/// Runtime state, caches, renders, and machine-local settings stay out of
/// the repository.
pub const GITIGNORE: &str = "\
# Kerf runtime state and caches, per machine
.kerf-runtime.*
.kerfd.sock
.kerf-render-*
project.kerf.tmp
.kerf/cache/
.kerf/runtime/
.kerf/docs/generated/
# Machine-local settings, which may hold credentials
.kerf/config.json
.kerf/mcp.json
.kerf/vision.json
# Renders land beside project.kerf
/*.mp4
.DS_Store
";

/// `project.kerf` merges structurally, and the review log by keeping both
/// sides' lines (it is only appended to); media goes through Git LFS when
/// it was available when the project was made.
pub fn gitattributes(lfs: bool) -> String {
    let mut text = String::from(
        "*.kerf merge=kerf\n*.kerflook text eol=lf\n*.kerfpreset text eol=lf\n\
         review/*.jsonl merge=union\n",
    );
    if lfs {
        text.push_str("media/** filter=lfs diff=lfs merge=lfs -text\n");
    }
    text
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Template {
    /// 1920×1080, 16:9.
    #[default]
    Standard,
    /// 1080×1920, 9:16.
    Vertical,
    /// 1080×1080.
    Square,
}

impl Template {
    pub fn size(self) -> (u32, u32) {
        match self {
            Self::Standard => (1920, 1080),
            Self::Vertical => (1080, 1920),
            Self::Square => (1080, 1080),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GitHubRepository {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default = "yes")]
    pub private: bool,
}

fn yes() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewProject {
    pub name: String,
    pub location: PathBuf,
    #[serde(default)]
    pub template: Template,
    /// Frames per second as written in documents: `23.976`, `29.97`, `30`.
    #[serde(default)]
    pub rate: Option<String>,
    /// Make the project a Git repository.
    #[serde(default)]
    pub git: bool,
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub github: Option<GitHubRepository>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Created {
    pub path: PathBuf,
    pub remote: Option<String>,
    pub warnings: Vec<String>,
}

/// The folder a project named `name` gets: the name, without characters
/// file systems refuse.
pub fn folder_name(name: &str) -> Result<String> {
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
    let cleaned = cleaned.trim_matches('.').trim().to_owned();
    if cleaned.is_empty() {
        return Err(CollabError::new("INVALID_NAME", "the project needs a name"));
    }
    Ok(cleaned)
}

/// The first document of a project made from `template` at `rate`.
pub fn initial_project(name: &str, template: Template, rate: Rate) -> Project {
    let (width, height) = template.size();
    let mut project = Project::new(name);
    project.sequences.push(Sequence {
        id: Id::new("s1").expect("valid id"),
        name: name.to_owned(),
        rate,
        drop_frame: false,
        width,
        height,
        sample_rate: 48_000,
        start: Time::ZERO,
        tracks: Vec::new(),
        markers: Vec::new(),
    });
    project
}

/// Make the project folder `new` describes. A shared project becomes a
/// repository with a first commit, and is published when it names a remote
/// or a GitHub repository to create.
pub fn create(new: &NewProject, catalog: &Catalog) -> Result<Created> {
    let rate = match new.rate.as_deref().map(str::trim).filter(|r| !r.is_empty()) {
        Some(text) => Rate::parse(text)
            .map_err(|error| CollabError::new("INVALID_RATE", format!("{text}: {error}")))?,
        None => Rate::FPS_30,
    };
    let directory = new.location.join(folder_name(&new.name)?);
    if directory.exists()
        && fs::read_dir(&directory)
            .map(|mut entries| entries.next().is_some())
            .unwrap_or(true)
    {
        return Err(CollabError::new(
            "ALREADY_EXISTS",
            format!("{} already exists", directory.display()),
        ));
    }
    let io = |error: std::io::Error| CollabError::new("IO_ERROR", error.to_string());
    fs::create_dir_all(directory.join(MEDIA_DIR)).map_err(io)?;
    fs::create_dir_all(directory.join(PRESETS_DIR)).map_err(io)?;
    let project = initial_project(new.name.trim(), new.template, rate);
    fs::write(
        directory.join(git::PROJECT_FILE),
        print_document(&project, catalog, PrintOptions::default()),
    )
    .map_err(io)?;
    let mut created = Created {
        path: directory.clone(),
        ..Created::default()
    };
    if !(new.git || new.remote.is_some() || new.github.is_some()) {
        return Ok(created);
    }
    let tools = git::local_tools();
    if !tools.git {
        return Err(CollabError::new("GIT_UNAVAILABLE", "git is not installed"));
    }
    prepare_repository_files(&directory, tools.git_lfs).map_err(io)?;
    git::init(&directory)?;
    if !tools.git_lfs {
        created.warnings.push(
            "Git LFS is not installed, so media files of 100 MB or more cannot be shared. Install it with `brew install git-lfs`."
                .into(),
        );
    }
    git::commit(&directory, &format!("Create {}", new.name.trim()))?;
    if let Some(github) = &new.github {
        if !tools.gh {
            return Err(CollabError::new(
                "GH_UNAVAILABLE",
                "creating a GitHub repository needs the GitHub CLI (brew install gh, then gh auth login)",
            ));
        }
        let name = match &github.name {
            Some(name) if !name.trim().is_empty() => name.trim().to_owned(),
            _ => repository_slug(&new.name),
        };
        created.remote = Some(git::create_github_repository(
            &directory,
            &name,
            github.private,
        )?);
    } else if let Some(remote) = new.remote.as_deref().filter(|r| !r.trim().is_empty()) {
        git::set_remote(&directory, remote.trim())?;
        created.remote = Some(remote.trim().to_owned());
        if let Err(error) = git::push(&directory) {
            created.warnings.push(format!(
                "The project was not pushed to {}: {}",
                remote.trim(),
                error.message
            ));
        }
    }
    Ok(created)
}

/// Write the ignore and attribute files a shared project needs, keeping
/// any that exist.
pub fn prepare_repository_files(directory: &Path, lfs: bool) -> std::io::Result<()> {
    let ignore = directory.join(".gitignore");
    if !ignore.exists() {
        fs::write(&ignore, GITIGNORE)?;
    }
    let attributes = directory.join(".gitattributes");
    if !attributes.exists() {
        fs::write(&attributes, gitattributes(lfs))?;
    }
    Ok(())
}

/// A GitHub repository name for a project name: letters, digits, `-`, `_`,
/// and `.`.
pub fn repository_slug(name: &str) -> String {
    let slug: String = name
        .trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let slug = slug.trim_matches('-').to_owned();
    if slug.is_empty() {
        "kerf-project".into()
    } else {
        slug
    }
}
