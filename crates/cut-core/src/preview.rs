//! Native-preview decisions derived from the live/offline capability contract.
//!
//! The plan is a presentation decision only. It never changes a project and
//! it does not replace the canonical runtime or the offline compiler.

use crate::check::{check_project, FindingSeverity, TimelineCheckFinding};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::Path;

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

/// Analyze a v2 project using the same checker and capability matrix exposed
/// to agents. Findings outside `range` do not affect a selected preview.
pub fn analyze_project(
    project: &Value,
    media_root: Option<&Path>,
    range: Option<PreviewRange>,
) -> PreviewPlan {
    let report = check_project(project, media_root);
    let mut reasons = Vec::new();
    let mut reasons_truncated = false;
    let mut has_error = false;
    let mut needs_render = false;

    for finding in report
        .findings
        .iter()
        .filter(|finding| applies_to_range(finding, range))
    {
        let is_error = finding.severity == FindingSeverity::Error;
        let is_native_incomplete = finding.code == "live_offline_capability_mismatch";
        if !is_error && !is_native_incomplete {
            continue;
        }
        has_error |= is_error;
        needs_render |= is_native_incomplete;
        if reasons.len() >= MAX_PREVIEW_REASONS {
            reasons_truncated = true;
            continue;
        }
        reasons.push(PreviewReason {
            code: finding.code.clone(),
            entity_path: finding.entity_path.clone(),
            entity_id: finding.entity_id.clone(),
            time_range: finding
                .time_range
                .as_ref()
                .and_then(|range| PreviewRange::new(range.start, range.end)),
            message: finding.message.clone(),
            severity: finding.severity,
        });
    }

    if report.truncated {
        reasons_truncated = true;
    }
    let mode = if has_error {
        PreviewMode::Unavailable
    } else if needs_render {
        PreviewMode::RenderedFallback
    } else {
        PreviewMode::NativeExact
    };
    PreviewPlan {
        mode,
        range,
        reasons,
        reasons_truncated,
    }
}

fn applies_to_range(finding: &TimelineCheckFinding, range: Option<PreviewRange>) -> bool {
    let Some(range) = range else {
        return true;
    };
    finding
        .time_range
        .as_ref()
        .and_then(|time_range| PreviewRange::new(time_range.start, time_range.end))
        .is_none_or(|finding_range| finding_range.intersects(range))
}

#[cfg(test)]
mod tests {
    use super::{analyze_project, PreviewMode, PreviewRange};
    use serde_json::json;
    use tempfile::tempdir;

    fn project_with_source(root: &std::path::Path, extra: serde_json::Value) -> serde_json::Value {
        let source = root.join("source.mp4");
        std::fs::write(&source, b"placeholder").expect("source");
        let mut project = json!({
            "version": 2,
            "fps": 30,
            "sources": [{
                "id": "source-1",
                "path": "source.mp4",
                "duration": 10,
                "width": 320,
                "height": 180,
                "hasAudio": false
            }],
            "tracks": [{
                "id": "video-1",
                "type": "video",
                "clips": [{
                    "id": "clip-1",
                    "sourceId": "source-1",
                    "in": 0,
                    "out": 4,
                    "timelineStart": 0
                }]
            }]
        });
        if let (Some(target), Some(additions)) = (project.as_object_mut(), extra.as_object()) {
            target.extend(additions.clone());
        }
        project
    }

    #[test]
    fn simple_project_can_use_native_exact_preview() {
        let root = tempdir().expect("tempdir");
        let plan = analyze_project(
            &project_with_source(root.path(), json!({})),
            Some(root.path()),
            None,
        );
        assert_eq!(plan.mode, PreviewMode::NativeExact);
        assert!(plan.reasons.is_empty());
    }

    #[test]
    fn unsupported_feature_requires_rendered_fallback() {
        let root = tempdir().expect("tempdir");
        let mut project = project_with_source(root.path(), json!({}));
        project["tracks"][0]["clips"][0]["crop"] = json!({
            "x": 0,
            "y": 0,
            "width": 0.5,
            "height": 1
        });
        let plan = analyze_project(&project, Some(root.path()), None);
        assert_eq!(plan.mode, PreviewMode::RenderedFallback);
        assert!(plan.needs_rendered_fallback());
    }

    #[test]
    fn out_of_range_feature_does_not_force_selected_preview_fallback() {
        let root = tempdir().expect("tempdir");
        let mut project = project_with_source(root.path(), json!({}));
        project["tracks"][0]["clips"][0]["timelineStart"] = json!(5);
        project["tracks"][0]["clips"][0]["crop"] = json!({
            "x": 0,
            "y": 0,
            "width": 0.5,
            "height": 1
        });
        let range = PreviewRange::new(0.0, 4.0).expect("range");
        let plan = analyze_project(&project, Some(root.path()), Some(range));
        assert_eq!(plan.mode, PreviewMode::NativeExact);
    }
}
