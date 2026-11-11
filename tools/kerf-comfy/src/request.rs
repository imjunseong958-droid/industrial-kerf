//! What Kerf asks of a model: the request a recipe's `generate` writes
//! (`docs/shots.md`, `kerf-engine/src/recipe/model.rs`).

use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The version of request this program reads.
const REQUEST_VERSION: u32 = 1;

/// A request as its file says it. Every file in it is where it is on this
/// machine.
#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct Request {
    #[serde(rename = "kerf-request")]
    version: u32,
    /// What the file should show, in words.
    #[serde(default)]
    pub prompt: String,
    /// A picture in motion to follow.
    #[serde(default)]
    pub guide: Option<PathBuf>,
    /// The first and the last frame.
    #[serde(default)]
    pub first: Option<PathBuf>,
    #[serde(default)]
    pub last: Option<PathBuf>,
    /// Pictures of who and what is in it, and of how it should look.
    #[serde(default)]
    pub references: Vec<PathBuf>,
    #[serde(default)]
    pub seconds: Option<f64>,
    #[serde(default)]
    pub size: Option<(u32, u32)>,
    #[serde(default)]
    pub seed: Option<u64>,
    /// Anything else the `generate` line said, as it was written.
    #[serde(default)]
    pub more: BTreeMap<String, String>,
    /// The file to write.
    pub out: PathBuf,
}

impl Request {
    /// Read the request `file` holds.
    pub fn read(file: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(file)
            .map_err(|error| format!("cannot read the request {}: {error}", file.display()))?;
        Self::parse(&text).map_err(|error| format!("the request {}: {error}", file.display()))
    }

    /// Read a request's text.
    pub fn parse(text: &str) -> Result<Self, String> {
        let request: Self = serde_json::from_str(text).map_err(|error| error.to_string())?;
        if request.version != REQUEST_VERSION {
            return Err(format!(
                "it is `kerf-request` {}, and this program reads {REQUEST_VERSION}",
                request.version
            ));
        }
        Ok(request)
    }

    /// The file the request names as `name`: `guide`, `first`, `last`, `ref`
    /// for the first reference, or `ref2` and on for the ones after it.
    pub fn file(&self, name: &str) -> Option<&Path> {
        match name {
            "guide" => self.guide.as_deref(),
            "first" => self.first.as_deref(),
            "last" => self.last.as_deref(),
            other => {
                reference(other).and_then(|index| self.references.get(index).map(PathBuf::as_path))
            }
        }
    }

    /// The names of the files it gives, as [`Request::file`] takes them.
    pub fn files(&self) -> Vec<String> {
        let mut names: Vec<String> = ["guide", "first", "last"]
            .into_iter()
            .filter(|name| self.file(name).is_some())
            .map(str::to_owned)
            .collect();
        names.extend((0..self.references.len()).map(|index| match index {
            0 => "ref".to_owned(),
            index => format!("ref{}", index + 1),
        }));
        names
    }
}

/// Which reference `name` is, counting from 0: `ref` is the first, `ref2`
/// the second.
fn reference(name: &str) -> Option<usize> {
    match name.strip_prefix("ref")? {
        "" => Some(0),
        number => number
            .parse::<usize>()
            .ok()
            .filter(|number| *number >= 2)
            .map(|number| number - 1),
    }
}

/// Whether `name` is one a request gives a file under.
pub fn names_a_file(name: &str) -> bool {
    matches!(name, "guide" | "first" | "last") || reference(name).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The request `docs/shots.md` shows.
    const SHOWN: &str = r#"{
      "kerf-request": 1,
      "model": "dawn",
      "prompt": "a fishing boat leaves the harbour at dawn, fog",
      "guide": "/project/shots/open.takes/003.work/guide.mp4",
      "first": "/project/refs/boat.png",
      "last": null,
      "references": ["/project/refs/sea.png", "/project/refs/fog.png"],
      "seconds": 4.0,
      "size": null,
      "seed": 7,
      "more": { "strength": "0.6" },
      "out": "/project/shots/open.takes/.making-003.mp4"
    }"#;

    #[test]
    fn a_request_is_read_as_kerf_writes_it() {
        let request = Request::parse(SHOWN).unwrap();
        assert_eq!(
            request.prompt,
            "a fishing boat leaves the harbour at dawn, fog"
        );
        assert_eq!(
            (request.seconds, request.size, request.seed),
            (Some(4.0), None, Some(7))
        );
        assert_eq!(
            request.more.get("strength").map(String::as_str),
            Some("0.6")
        );
        assert_eq!(
            request.out,
            Path::new("/project/shots/open.takes/.making-003.mp4")
        );
        assert_eq!(request.files(), ["guide", "first", "ref", "ref2"]);
        assert_eq!(
            request.file("ref2"),
            Some(Path::new("/project/refs/fog.png"))
        );
        assert_eq!(request.file("last"), None);
        assert_eq!(request.file("ref3"), None);
        let sized =
            Request::parse(&SHOWN.replace("\"size\": null", "\"size\": [1280, 720]")).unwrap();
        assert_eq!(sized.size, Some((1280, 720)));
    }

    #[test]
    fn a_request_of_another_version_is_refused() {
        let newer = Request::parse(&SHOWN.replace("\"kerf-request\": 1", "\"kerf-request\": 2"))
            .unwrap_err();
        assert!(newer.contains("this program reads 1"), "{newer}");
        assert!(Request::parse("{}").is_err());
    }

    #[test]
    fn only_a_requests_own_names_are_files() {
        for name in ["guide", "first", "last", "ref", "ref2", "ref12"] {
            assert!(names_a_file(name), "{name}");
        }
        for name in ["prompt", "seed", "ref1", "ref0", "refs", "reference"] {
            assert!(!names_a_file(name), "{name}");
        }
    }
}
