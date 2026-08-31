use thiserror::Error;

#[derive(Debug, Error)]
pub enum EngineError {
    #[error("{0}")]
    Message(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Compile(#[from] cut_core::compile::CompileError),
    #[error("{0}")]
    CompileV1(#[from] cut_core::compile_v1::CompileV1Error),
    #[error("{0}")]
    Vision(#[from] crate::vision::VisionError),
    #[error("ffmpeg failed ({code}): {stderr}")]
    Ffmpeg { code: i32, stderr: String },
    #[error("cut-engine binary not found")]
    BinaryNotFound,
}

impl EngineError {
    pub fn msg(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }
}

pub type Result<T> = std::result::Result<T, EngineError>;
