//! Native-preview decisions: whether the viewer can play a project natively
//! or needs a rendered preview.
//!
//! The plan is a presentation decision only. It never changes a project and
//! it does not replace the canonical runtime or the offline compiler.

use serde::{Deserialize, Serialize};

/// How serious a preview reason is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FindingSeverity {
    Error,
    Warning,
}

/// Maximum number of reasons retained in one preview plan.
pub const MAX_PREVIEW_REASONS: usize = 32;

/// The preview path safe for a project or selected time range.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewMode {
    /// Native playback is complete for the relevant project state.
    NativeExact,
    /// Native playback is incomplete, so a rendered preview is required.
    RenderedFallback,
    /// The project cannot currently be rendered or played until an error is
    /// fixed, such as a missing source or a compiler rejection.
    Unavailable,
}

impl PreviewMode {
    /// Whether this mode can be presented as an exact native preview.
    pub const fn is_native_exact(self) -> bool {
        matches!(self, Self::NativeExact)
    }

    /// Whether this mode requires the rendered fallback path.
    pub const fn requires_render(self) -> bool {
        matches!(self, Self::RenderedFallback)
    }
}

/// A bounded project-time interval selected for a rendered preview.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewRange {
    pub start: f64,
    pub end: f64,
}

impl PreviewRange {
    /// Construct a finite, positive project-time range.
    pub fn new(start: f64, end: f64) -> Option<Self> {
        (start.is_finite() && end.is_finite() && start >= 0.0 && end > start)
            .then_some(Self { start, end })
    }

    /// Whether this range intersects another range.
    pub fn intersects(self, other: Self) -> bool {
        self.start < other.end && other.start < self.end
    }

    /// Return the range duration.
    pub fn duration(self) -> f64 {
        self.end - self.start
    }
}

/// One bounded reason why native playback is not exact.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewReason {
    pub code: String,
    pub entity_path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_range: Option<PreviewRange>,
    pub message: String,
    pub severity: FindingSeverity,
}

/// Typed decision consumed by the native viewer before it displays frames.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PreviewPlan {
    pub mode: PreviewMode,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub range: Option<PreviewRange>,
    pub reasons: Vec<PreviewReason>,
    pub reasons_truncated: bool,
}

impl PreviewPlan {
    /// Whether the viewer may present native playback as exact.
    pub const fn native_exact(&self) -> bool {
        self.mode.is_native_exact()
    }

    /// Whether the viewer should request a rendered fallback.
    pub const fn needs_rendered_fallback(&self) -> bool {
        self.mode.requires_render()
    }

    /// Whether errors prevent either preview path from being trusted.
    pub const fn unavailable(&self) -> bool {
        matches!(self.mode, PreviewMode::Unavailable)
    }
}
