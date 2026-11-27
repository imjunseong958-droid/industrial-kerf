use std::path::PathBuf;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{0}")]
    Message(String),
    #[error("capability unavailable: {capability}: {message}")]
    CapabilityUnavailable { capability: String, message: String },
    #[error("operation cancelled")]
    Cancelled,
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Program(#[from] kerf_core::program::ProgramError),
    #[error(transparent)]
    Compose(#[from] kerf_compose::ComposeError),
    #[error("ffmpeg failed ({code}): {stderr}")]
    Ffmpeg { code: i32, stderr: String },
    /// The project has no media by this id.
    #[error("no media `{0}` in this project")]
    NoMedia(String),
    /// A media's steps are still in the recipe file it names, where a making does not read
    /// them: said with how they come into the document.
    #[error(
        "the steps of media `{media}` are in the recipe file `{recipe}`, and a media is made from the steps under its line: `kerf carry {media}`, or an agent's `edit` with `carry: [\"{media}\"]`, writes them there"
    )]
    NotCarried { media: String, recipe: String },
    /// No model by this name is set up in the file the person sets models up in, `file`, where
    /// the models of `have` are: said with how one is, and with `note` where there is one.
    #[error(
        "no model `{name}` is set up: {} {}. A model is reached by a provider named there under \"providers\": a server, `{{\"<name>\": {{\"url\": \"https://…\"}}}}`, or a program, `{{\"{}\": {{\"run\": [\"<program>\", \"{{request}}\"]}}}}`{}",
        .file.display(),
        names_said(.have),
        // A program is one model by its provider's name, which holds no `/`.
        .name.rsplit('/').next().unwrap_or_default(),
        .note.as_ref().map(|note| format!(". {note}")).unwrap_or_default()
    )]
    NoModel {
        name: String,
        file: PathBuf,
        have: Vec<String>,
        note: Option<String>,
    },
}

/// The models a person has set up, as [`EngineError::NoModel`] says them.
fn names_said(have: &[String]) -> String {
    if have.is_empty() {
        "sets up none".to_owned()
    } else {
        format!("sets up {}", have.join(", "))
    }
}

impl EngineError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    /// The clip, transition, or sequence a render cannot draw, if the error
    /// names one.
    pub fn entity(&self) -> Option<&str> {
        match self {
            Self::Program(error) => error.entity(),
            Self::Compose(error) => error.entity(),
            _ => None,
        }
    }

    pub fn capability_unavailable(
        capability: impl Into<String>,
        message: impl Into<String>,
    ) -> Self {
        Self::CapabilityUnavailable {
            capability: capability.into(),
            message: message.into(),
        }
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;
