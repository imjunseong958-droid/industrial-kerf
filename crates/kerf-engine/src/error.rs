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
    #[error("{0}")]
    Program(#[from] kerf_core::program::ProgramError),
    #[error("{0}")]
    Compose(#[from] kerf_compose::ComposeError),
    #[error("ffmpeg failed ({code}): {stderr}")]
    Ffmpeg { code: i32, stderr: String },
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
