//! Who staged a proposal.

use crate::{ErrorCode, RuntimeError};
use serde::{Deserialize, Serialize};

/// Who may stage a proposal.
pub const AUTHOR_KINDS: [&str; 4] = ["kerf", "mcp", "person", "qc"];

const MAX_NAME_BYTES: usize = 256;

/// Who wrote a proposal: `kind` is `kerf`, `mcp`, `person` or `qc`; `id` is a
/// person's email or an agent's client id; `role` is shown beside the name
/// (`MCP · via Kerf`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Author {
    pub kind: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
}

impl Default for Author {
    /// Kerf's own agent ([`Author::kerf`]).
    fn default() -> Self {
        Self::kerf()
    }
}

impl Author {
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            id: None,
            role: None,
        }
    }

    /// Kerf's own agent.
    pub fn kerf() -> Self {
        Self::new("kerf", "Kerf")
    }

    /// Whether kerfd takes it: a kind it knows, and a name, id and role of a length it keeps.
    pub fn validate(&self) -> Result<(), RuntimeError> {
        let invalid = |message: &str| RuntimeError::new(ErrorCode::InvalidAuthor, message);
        if !AUTHOR_KINDS.contains(&self.kind.as_str()) {
            return Err(invalid("author kind must be kerf, mcp, person, or qc"));
        }
        if self.name.trim().is_empty() || self.name.len() > MAX_NAME_BYTES {
            return Err(invalid("author name must be 1 to 256 bytes"));
        }
        for text in [&self.id, &self.role].into_iter().flatten() {
            if text.len() > MAX_NAME_BYTES {
                return Err(invalid("author id and role must be at most 256 bytes"));
            }
        }
        Ok(())
    }
}
