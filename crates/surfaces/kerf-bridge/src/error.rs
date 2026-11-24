//! Why the bridge could not do what a command asked, kept as the kind it is until an
//! `error` event says it: in the words it was said in, and by its code where it has one.

use crate::event::ErrorCode;
use kerf_engine::EngineError;
use kerf_runtime::RuntimeError;
use kerf_shots::ShotError;

/// Why the bridge could not do what a command asked. kerfd's refusal keeps its code on the
/// way to the editor, and a shot's or the engine's keeps its kind; the rest is only said.
///
/// Nothing turns this back into a `String`: whatever says it says it as an `error` event,
/// by [`BridgeError::code`] as well.
#[derive(Debug, thiserror::Error)]
pub(crate) enum BridgeError {
    /// What kerfd answered, or that it could not be reached: said in kerfd's words, which
    /// are its message alone.
    #[error("{}", .0.message)]
    Runtime(RuntimeError),
    /// A shot that could not be read or changed.
    #[error(transparent)]
    Shot(#[from] ShotError),
    /// What the engine refused.
    #[error(transparent)]
    Engine(#[from] EngineError),
    /// Anything else, as it is said.
    #[error("{0}")]
    Said(String),
}

impl BridgeError {
    /// The code an `error` says this by: kerfd's own, or for steps still in a recipe file
    /// the bridge's, which the editor offers to carry them by. Anything else has none.
    pub(crate) fn code(&self) -> Option<ErrorCode> {
        match self {
            Self::Runtime(error) => Some(error.code.clone()),
            Self::Shot(ShotError::NotCarried { .. })
            | Self::Engine(EngineError::NotCarried { .. }) => Some(ErrorCode::StepsNotCarried),
            _ => None,
        }
    }
}

impl From<RuntimeError> for BridgeError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}

impl From<String> for BridgeError {
    fn from(said: String) -> Self {
        Self::Said(said)
    }
}

impl From<&str> for BridgeError {
    fn from(said: &str) -> Self {
        Self::Said(said.to_owned())
    }
}
