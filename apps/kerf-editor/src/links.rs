//! kerf:// project links: a project, a revision and a moment of it.
//!
//! ```text
//! kerf://<project>/rev<N>[@HH:MM:SS:FF][?branch=…&head=…&remote=…]
//! ```
//!
//! `<project>` is the slug of the project's folder name ([`slug`]): lower
//! case, every run of anything but letters and digits one `-`. Following a
//! link opens the project and seeks to the timecode. A project this computer
//! does not know (none of the recent ones) is an error shown to the person,
//! never a guess. macOS hands a link to the running editor; elsewhere (and
//! from a shell) it comes as the editor's argument.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Link {
    /// The project's slug as written.
    pub project: String,
    pub revision: Option<u64>,
    /// "HH:MM:SS:FF" as written.
    pub timecode: Option<String>,
    pub branch: Option<String>,
}

/// Whether `text` is a kerf:// link at all. Any text may be asked about,
/// a folder named in any script among them.
pub fn is_link(text: &str) -> bool {
    text.as_bytes()
        .get(..7)
        .is_some_and(|head| head.eq_ignore_ascii_case(b"kerf://"))
}

/// Reads a link; the error says why it is not one, in words for a person.
pub fn parse(text: &str) -> Result<Link, String> {
    if !is_link(text) {
        return Err("Not a kerf:// link".into());
    }
    // A fragment (`#…`, as a browser may add one) says nothing here.
    let rest = text[7..].split('#').next().unwrap_or_default();
    let (rest, query) = match rest.split_once('?') {
        Some((rest, query)) => (rest, Some(query)),
        None => (rest, None),
    };
    let (project, place) = rest.split_once('/').unwrap_or((rest, ""));
    let project = percent_decode(project).to_lowercase();
    if project.is_empty() {
        return Err("The link names no project".into());
    }
    let mut link = Link {
        project,
        ..Link::default()
    };
    let place = place.trim_end_matches('/');
    if !place.is_empty() {
        let (revision, timecode) = match place.split_once('@') {
            Some((revision, timecode)) => (revision, Some(timecode)),
            None => (place, None),
        };
        let number = revision
            .strip_prefix("rev")
            .and_then(|number| number.parse::<u64>().ok())
            .ok_or_else(|| format!("“{revision}” is not a revision (rev and a number)"))?;
        link.revision = Some(number);
        if let Some(timecode) = timecode {
            let parts: Vec<&str> = timecode.split([':', ';']).collect();
            if parts.len() != 4
                || parts
                    .iter()
                    .any(|part| part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()))
            {
                return Err(format!("“{timecode}” is not a timecode (HH:MM:SS:FF)"));
            }
            link.timecode = Some(timecode.to_owned());
        }
    }
    if let Some(query) = query {
        for pair in query.split('&') {
            if let Some(("branch", value)) = pair.split_once('=') {
                link.branch = Some(percent_decode(value));
            }
        }
    }
    Ok(link)
}

/// The slug a project name or folder makes: "Atlas Cut" -> "atlas-cut".
/// Empty when the name has no letter or digit.
pub fn slug(name: &str) -> String {
    let mut slug = String::new();
    let mut gap = false;
    for character in name.chars() {
        if character.is_alphanumeric() {
            if gap && !slug.is_empty() {
                slug.push('-');
            }
            gap = false;
            slug.extend(character.to_lowercase());
        } else {
            gap = true;
        }
    }
    slug
}

/// The link to `revision` of the project in `directory`, at `seconds` at `fps`.
pub fn make(directory: &Path, revision: u64, seconds: Option<f64>, fps: f64) -> Option<String> {
    let name = directory.file_name()?.to_string_lossy();
    let slug = slug(&name);
    if slug.is_empty() {
        return None;
    }
    let mut link = format!("kerf://{slug}/rev{revision}");
    if let Some(seconds) = seconds {
        link.push('@');
        link.push_str(&crate::model::timecode(seconds, fps));
    }
    Some(link)
}

/// The folder among `candidates` whose slug is the link's; an error when
/// none is, or when two are.
pub fn resolve(link: &Link, candidates: &[PathBuf]) -> Result<PathBuf, String> {
    let wanted = slug(&link.project);
    let mut found: Vec<&PathBuf> = candidates
        .iter()
        .filter(|candidate| {
            candidate
                .file_name()
                .is_some_and(|name| slug(&name.to_string_lossy()) == wanted)
        })
        .collect();
    found.dedup();
    match found.as_slice() {
        [one] => Ok((*one).clone()),
        [] => Err(format!(
            "No project here is called “{}”. Open it once, and its links will work.",
            link.project
        )),
        _ => Err(format!(
            "More than one project here is called “{}”.",
            link.project
        )),
    }
}

/// Whether the project `resolve` found can be opened: its folder is still
/// there and holds its project.kerf. The error says so to the person.
pub fn openable(project: &Path) -> Result<(), String> {
    let name = project
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| project.display().to_string());
    if project.join("project.kerf").is_file() {
        Ok(())
    } else {
        Err(format!(
            "The project “{name}” is no longer at {}.",
            project.display()
        ))
    }
}

/// The seconds of the link's timecode at `fps`.
pub fn seconds(link: &Link, fps: f64) -> Option<f64> {
    let timecode = link.timecode.as_deref()?;
    let parts: Vec<f64> = timecode
        .split([':', ';'])
        .filter_map(|part| part.parse().ok())
        .collect();
    let [hours, minutes, secs, frames] = parts.as_slice() else {
        return None;
    };
    let fps = if fps > 0.0 { fps } else { 25.0 };
    Some(hours * 3600.0 + minutes * 60.0 + secs + frames / fps.round().max(1.0))
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && bytes[index + 1].is_ascii_hexdigit()
            && bytes[index + 2].is_ascii_hexdigit()
        {
            let hex = |byte: u8| (byte as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(bytes[index + 1]) * 16 + hex(bytes[index + 2]));
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}
