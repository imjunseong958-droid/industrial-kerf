//! Diagnostics shared by validation, operations, and the text layer.

use crate::id::Id;
use std::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Error,
    Warning,
}

/// A line and column in a text document, both 1-based.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Diagnostic {
    pub severity: Severity,
    /// Stable machine-readable code, such as `clip.overlap`.
    pub code: &'static str,
    pub message: String,
    pub entity: Option<Id>,
    pub span: Option<Span>,
    /// What the message says is not there, for a caller that does something about it rather
    /// than read the words. Said by the refusals of a clip, a media, or a position's anchor
    /// that a line or an operation names; other refusals of something missing do not say it.
    /// Boxed: a diagnostic is what a refused operation returns, and stays small for that.
    pub missing: Option<Box<Missing>>,
}

/// Something a [`Diagnostic`] says is not there: its kind, and the id it was named by.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Missing {
    pub kind: MissingKind,
    pub id: String,
}

/// The kind of thing a [`Missing`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MissingKind {
    Clip,
    Media,
    /// A position's anchor, `<id>.start` or `<id>.end`, which may name a clip or a marker.
    ClipOrMarker,
}

impl Diagnostic {
    pub fn error(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Error,
            code,
            message: message.into(),
            entity: None,
            span: None,
            missing: None,
        }
    }

    pub fn warning(code: &'static str, message: impl Into<String>) -> Self {
        Self {
            severity: Severity::Warning,
            ..Self::error(code, message)
        }
    }

    pub fn entity(mut self, id: &Id) -> Self {
        self.entity = Some(id.clone());
        self
    }

    pub fn at(mut self, line: usize, column: usize) -> Self {
        self.span = Some(Span { line, column });
        self
    }

    /// That `id`, a `kind` of thing, is what is not there.
    pub fn missing(mut self, kind: MissingKind, id: impl Into<String>) -> Self {
        self.missing = Some(Box::new(Missing {
            kind,
            id: id.into(),
        }));
        self
    }

    pub fn is_error(&self) -> bool {
        self.severity == Severity::Error
    }
}

impl fmt::Display for Diagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let severity = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(f, "{severity}[{}]", self.code)?;
        if let Some(span) = self.span {
            write!(f, " {}:{}", span.line, span.column)?;
        }
        write!(f, ": {}", self.message)
    }
}

impl std::error::Error for Diagnostic {}
