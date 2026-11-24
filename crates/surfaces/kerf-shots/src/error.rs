//! Why a shot could not be read or changed.

use kerf_engine::EngineError;

/// Why a shot could not be read or changed. What a caller does something about has a kind
/// of its own, so that nobody has to read the words to tell; the rest is only said.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ShotError {
    /// The project has no media by this id. An agent is told where it may be instead: in a
    /// proposal not built on.
    #[error("no media `{0}` in this project")]
    NoMedia(String),
    /// The media's steps are still in the recipe file it names. The editor offers to carry
    /// them into the document.
    #[error(
        "the steps of media `{media}` are in the recipe file `{recipe}`, and a media is made from the steps under its line: `kerf carry {media}`, or an agent's `edit` with `carry: [\"{media}\"]`, writes them there"
    )]
    NotCarried { media: String, recipe: String },
    /// The project has no clip by the id asked for: what [`kerf_model::Project::no_clip`]
    /// says of it.
    #[error("{0}")]
    NoClip(String),
    /// Anything else, as it is said.
    #[error("{0}")]
    Other(String),
}

impl From<EngineError> for ShotError {
    fn from(error: EngineError) -> Self {
        match error {
            EngineError::NoMedia(id) => Self::NoMedia(id),
            EngineError::NotCarried { media, recipe } => Self::NotCarried { media, recipe },
            other => Self::Other(other.to_string()),
        }
    }
}

impl From<String> for ShotError {
    fn from(said: String) -> Self {
        Self::Other(said)
    }
}

impl From<&str> for ShotError {
    fn from(said: &str) -> Self {
        Self::Other(said.to_owned())
    }
}

/// For a caller that only says it.
impl From<ShotError> for String {
    fn from(error: ShotError) -> Self {
        error.to_string()
    }
}
