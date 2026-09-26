//! Region planning for observed video frames.

pub mod planning;

pub use planning::{
    build_crop_plan, build_tracking_plan, crop_plan_from_json, plan_crop_for_aspect_ratio,
    plan_tracking, CropPlan, CropPlanMode, CropPlanRequest, PlanningInputError, RangeError,
    RoiKeyframe, TrackingPlan, TrackingPlanInput, TrackingPlanOptions, TrackingSegment,
};
use serde::{Deserialize, Serialize};

/// The three semantic classes exposed by the vision adapter.
pub const VISION_TARGETS: &[&str] = &["cam", "chat", "content"];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VisionTarget {
    Cam,
    Chat,
    Content,
}

impl VisionTarget {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Cam => "cam",
            Self::Chat => "chat",
            Self::Content => "content",
        }
    }

    fn parse_label(s: &str) -> Result<Self, &'static str> {
        match s {
            "cam" => Ok(Self::Cam),
            "chat" => Ok(Self::Chat),
            "content" => Ok(Self::Content),
            _ => Err("expected cam, chat, or content"),
        }
    }

    pub fn order(self) -> usize {
        match self {
            Self::Cam => 0,
            Self::Chat => 1,
            Self::Content => 2,
        }
    }
}

impl std::str::FromStr for VisionTarget {
    type Err = &'static str;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse_label(s)
    }
}

impl std::fmt::Display for VisionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedBoundingBox {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum TrackId {
    Number(i64),
    String(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedDetection {
    #[serde(rename = "trackId")]
    pub track_id: TrackId,
    #[serde(rename = "class")]
    pub class_: VisionTarget,
    pub confidence: f64,
    pub frame: u64,
    pub time: f64,
    #[serde(rename = "frameIndex")]
    pub frame_index: u64,
    #[serde(rename = "timeSeconds")]
    pub time_seconds: f64,
    pub bbox: NormalizedBoundingBox,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NormalizedTrackingResult {
    pub source: String,
    pub model: String,
    pub targets: Vec<VisionTarget>,
    pub detections: Vec<NormalizedDetection>,
    pub frame_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
}

pub(crate) fn round_coordinate(value: f64) -> f64 {
    format!("{:.12}", value).parse().unwrap_or(value)
}

pub(crate) fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

pub(crate) fn track_order(a: &TrackId, b: &TrackId) -> std::cmp::Ordering {
    match (a, b) {
        (TrackId::Number(left), TrackId::Number(right)) => left.cmp(right),
        (TrackId::String(left), TrackId::String(right)) => numeric_str_cmp(left, right),
        (TrackId::Number(left), TrackId::String(right)) => {
            numeric_str_cmp(&left.to_string(), right)
        }
        (TrackId::String(left), TrackId::Number(right)) => {
            numeric_str_cmp(left, &right.to_string())
        }
    }
}

fn numeric_str_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    match (a.parse::<i64>(), b.parse::<i64>()) {
        (Ok(a_num), Ok(b_num)) => a_num.cmp(&b_num),
        _ => a.cmp(b),
    }
}
