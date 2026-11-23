//! A render: the part of the program it covers, how it is asked for, and the job kerfd keeps.

use crate::{ErrorCode, RenderCodec, RenderQuality, RenderStatus, RuntimeError};
use serde::{Deserialize, Serialize};

/// Maximum duration of one bounded rendered-preview request.
pub const MAX_RENDER_RANGE_SECONDS: f64 = 120.0;

/// A bounded project-time range rendered for a preview job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderRange {
    pub start: f64,
    pub duration: f64,
}

impl RenderRange {
    /// Validate a positive, bounded project-time range.
    pub fn new(start: f64, duration: f64) -> Result<Self, RuntimeError> {
        if !start.is_finite()
            || start < 0.0
            || !duration.is_finite()
            || duration <= 0.0
            || duration > MAX_RENDER_RANGE_SECONDS
            || !(start + duration).is_finite()
        {
            return Err(RuntimeError::new(
                ErrorCode::InvalidRenderRange,
                format!(
                    "render range requires start >= 0, 0 < duration <= {MAX_RENDER_RANGE_SECONDS}s"
                ),
            ));
        }
        Ok(Self { start, duration })
    }

    pub fn end(&self) -> f64 {
        self.start + self.duration
    }
}

fn default_render_progress() -> f64 {
    0.0
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RenderJob {
    pub job_id: String,
    pub status: RenderStatus,
    pub output_url: Option<String>,
    pub output_path: Option<String>,
    pub error: Option<String>,
    #[serde(default)]
    pub range: Option<RenderRange>,
    /// The sequence rendered; the first when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(default = "default_render_progress")]
    pub progress: f64,
    /// What the render was called when it was started, for a queue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Absent on jobs recorded before renders kept it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub quality: Option<RenderQuality>,
    /// Absent on jobs recorded before renders kept it, which were H.264.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<RenderCodec>,
}

/// What a render started with options (kerf-runtime's `RuntimeClient::start_render_with`)
/// renders, how, and where.
///
/// - `quality`: `preview` (a draft from proxies), `full`, `review` (a
///   full-size H.264 review copy at 12 Mb/s), or `master` (ProRes 422 HQ
///   unless `codec` says otherwise).
/// - `codec`: `h264`, `hevc` or `prores_hq` (QuickTime with 24-bit PCM). A
///   preview is H.264 or HEVC, and a review copy is H.264.
/// - `output`: a folder (ending in `/`, or one that exists) or a file whose
///   extension the codec's container takes (`.mp4`, or `.mov` for ProRes).
///   A relative one is read against the project folder and must stay inside
///   it, out of hidden folders; missing folders inside it are made. An
///   absolute one is a folder the user chose: it must exist. Absent, the
///   file goes in the project folder. A render never writes over a file
///   on disk or one a kept job made or is making: into a folder it takes
///   the next free name (`Master 2.mov`); a file named outright is
///   refused with `RENDER_OUTPUT_EXISTS`, and one an unfinished job is
///   writing with `RENDER_OUTPUT_BUSY`. Nothing is made on disk for a
///   refused render.
/// - `name`: the job's name in a queue, and the file's name in a folder;
///   the job id names the file when absent.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StartRenderOptions {
    pub quality: RenderQuality,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub range: Option<RenderRange>,
    /// Render what this pending proposal would make of the project.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposal: Option<String>,
    /// The sequence to render; the first when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sequence: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub codec: Option<RenderCodec>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl StartRenderOptions {
    /// A render of the whole first sequence at `quality`.
    pub fn new(quality: RenderQuality) -> Self {
        Self {
            quality,
            ..Self::default()
        }
    }
}
