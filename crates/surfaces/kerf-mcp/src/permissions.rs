//! What a person lets an agent do without asking: their own
//! `~/.kerf/agent.json` (`KERF_AGENT` names another file), outside any
//! project, so a project cannot grant itself an agent that runs its
//! recipes, as it cannot name the programs they run.
//!
//! ```json
//! { "permissions": "auto", "makes": 5 }
//! ```
//!
//! - `ask`, the default: an agent's edit is a proposal the person accepts,
//!   and a making it asks for is one the person runs.
//! - `accept-edits`: an agent's edit is accepted as it is made; a making is
//!   still the person's to run.
//! - `auto`: an edit is accepted, and a making an agent asks for runs, up to
//!   `makes` of them in one session (five unless it says), after which it
//!   asks again.
//!
//! Every edit is still one revision that undo takes back, and a making still
//! writes a new take beside the ones before it, so what was let through is
//! seen and undone as anything else.

use serde::Deserialize;
use std::path::{Path, PathBuf};

/// How many makings an agent runs on its own in one session when the file
/// does not say.
pub const DEFAULT_MAKES: u32 = 5;

/// What an agent may do without a person saying yes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mode {
    /// Every edit is proposed, and every making asked for.
    #[default]
    Ask,
    /// Edits are accepted as they are made; makings are asked for.
    AcceptEdits,
    /// Edits are accepted, and makings run, up to the limit.
    Auto,
}

impl Mode {
    /// The name the file gives it.
    pub fn name(self) -> &'static str {
        match self {
            Self::Ask => "ask",
            Self::AcceptEdits => "accept-edits",
            Self::Auto => "auto",
        }
    }

    fn named(name: &str) -> Option<Self> {
        match name {
            "ask" => Some(Self::Ask),
            "accept-edits" => Some(Self::AcceptEdits),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// A person's permissions for the agents on their projects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Permissions {
    pub mode: Mode,
    /// The most makings an agent runs on its own in one session.
    pub makes: u32,
    /// The file they were read from, there or not.
    pub file: PathBuf,
}

impl Default for Permissions {
    fn default() -> Self {
        Self {
            mode: Mode::Ask,
            makes: DEFAULT_MAKES,
            file: PathBuf::new(),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    permissions: Option<String>,
    makes: Option<u32>,
}

impl Permissions {
    /// The person's: the file `KERF_AGENT` names, or `~/.kerf/agent.json`.
    /// No file is `ask`.
    pub fn load() -> Result<Self, String> {
        let file = std::env::var_os("KERF_AGENT")
            .filter(|path| !path.is_empty())
            .map(PathBuf::from)
            .or_else(|| {
                // Windows sets no HOME.
                std::env::var_os("HOME")
                    .or_else(|| std::env::var_os("USERPROFILE"))
                    .filter(|home| !home.is_empty())
                    .map(|home| PathBuf::from(home).join(".kerf").join("agent.json"))
            })
            .unwrap_or_else(|| PathBuf::from("agent.json"));
        Self::read(&file)
    }

    /// The permissions `file` gives. A file that is there and does not read
    /// is an error, never taken as one that lets more through.
    pub fn read(file: &Path) -> Result<Self, String> {
        let text = match std::fs::read_to_string(file) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self {
                    file: file.to_path_buf(),
                    ..Self::default()
                })
            }
            Err(error) => return Err(format!("cannot read {}: {error}", file.display())),
        };
        let read: File = serde_json::from_str(&text)
            .map_err(|error| format!("{} does not read: {error}", file.display()))?;
        let mode = match read.permissions.as_deref() {
            None => Mode::Ask,
            Some(name) => Mode::named(name).ok_or_else(|| {
                format!(
                    "{}: `permissions` is ask, accept-edits, or auto, not {name:?}",
                    file.display()
                )
            })?,
        };
        Ok(Self {
            mode,
            makes: read.makes.unwrap_or(DEFAULT_MAKES),
            file: file.to_path_buf(),
        })
    }

    /// Whether an agent's edit is accepted as it is made.
    pub fn accepts_edits(&self) -> bool {
        matches!(self.mode, Mode::AcceptEdits | Mode::Auto)
    }

    /// Whether a making an agent asks for runs, when it has run `made` this
    /// session.
    pub fn runs_makings(&self, made: u32) -> bool {
        self.mode == Mode::Auto && made < self.makes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(text: &str) -> Result<Permissions, String> {
        let folder = tempfile::tempdir().unwrap();
        let file = folder.path().join("agent.json");
        std::fs::write(&file, text).unwrap();
        Permissions::read(&file)
    }

    #[test]
    fn no_file_asks() {
        let folder = tempfile::tempdir().unwrap();
        let permissions = Permissions::read(&folder.path().join("agent.json")).unwrap();
        assert_eq!(permissions.mode, Mode::Ask);
        assert!(!permissions.accepts_edits());
        assert!(!permissions.runs_makings(0));
    }

    #[test]
    fn each_mode_lets_through_what_it_says() {
        let edits = read(r#"{"permissions": "accept-edits"}"#).unwrap();
        assert!(edits.accepts_edits());
        assert!(!edits.runs_makings(0));
        let auto = read(r#"{"permissions": "auto", "makes": 2}"#).unwrap();
        assert!(auto.accepts_edits());
        assert!(auto.runs_makings(1));
        // Past its limit a making is asked for again.
        assert!(!auto.runs_makings(2));
        assert_eq!(
            read(r#"{"permissions": "auto"}"#).unwrap().makes,
            DEFAULT_MAKES
        );
    }

    #[test]
    fn a_file_that_does_not_read_lets_nothing_more_through() {
        let wrong = read(r#"{"permissions": "always"}"#).unwrap_err();
        assert!(
            wrong.contains("`permissions` is ask, accept-edits, or auto"),
            "{wrong}"
        );
        assert!(read(r#"{"permission": "auto"}"#).is_err());
        assert!(read("not json").is_err());
    }
}
