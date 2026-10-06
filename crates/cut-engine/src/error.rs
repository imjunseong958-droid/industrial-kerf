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
    Program(#[from] cut_core::program::ProgramError),
    #[error("ffmpeg failed ({code}): {stderr}")]
    Ffmpeg { code: i32, stderr: String },
}

impl EngineError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
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
