//! Vision adapter: config validation, tracking output normalization, planning,
//! and native YOLO + ByteTrack inference.

pub mod bytetrack;
pub mod config;
pub mod nms;
pub mod normalize;
#[cfg(feature = "onnx")]
pub mod onnx;
pub mod planning;
pub mod weights;
pub mod yolo;

pub use bytetrack::ByteTracker;
pub use config::{
    identity_label_map, resolve_label_map, validate_vision_config, validate_yolo26_config,
    ValidatedYolo26Config, Yolo26TrackingConfig,
};
pub use normalize::{normalize_detection_frames, parse_and_normalize_tracking_output};
pub use planning::{
    build_crop_plan, build_tracking_plan, crop_plan_from_json, plan_crop_for_aspect_ratio,
    plan_tracking, CropPlan, CropPlanMode, CropPlanRequest, PlanningInputError, RangeError,
    RoiKeyframe, TrackingPlan, TrackingPlanInput, TrackingPlanOptions, TrackingSegment,
};
pub use weights::resolve_inference_weights;
pub use yolo::{
    scaled_decode_size, tracking_decode_filter, DetectionFrame, FfmpegRgbFrameSource, FrameSource,
    InferenceBackend, OnnxBackend, SyntheticBackend, TrackDecodeOptions, TrackOptions, YoloEngine,
    YoloRawOutput, DEFAULT_IMGSZ,
};

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The three semantic classes exposed by the vision adapter.
pub const VISION_TARGETS: &[&str] = &["cam", "chat", "content"];

/// The official Ultralytics YOLO26 small detection checkpoint identifier.
pub const YOLO26_SMALL_MODEL: &str = "yolo26s.pt";

/// Official Ultralytics tracker config chosen for predictable IDs.
pub const DEFAULT_TRACKER: &str = "bytetrack.yaml";
pub const DEFAULT_CONFIDENCE: f64 = 0.25;
pub const DEFAULT_IOU: f64 = 0.7;
pub const DEFAULT_TIMEOUT_MS: u64 = 120_000;

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

    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "cam" => Some(Self::Cam),
            "chat" => Some(Self::Chat),
            "content" => Some(Self::Content),
            _ => None,
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

impl std::fmt::Display for VisionTarget {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum UnknownLabelPolicy {
    Ignore,
    Error,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParsedTrackingOutput {
    pub detections: Vec<NormalizedDetection>,
    #[serde(rename = "frameCount")]
    pub frame_count: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
}

#[derive(Debug, Clone)]
pub struct NormalizeOptions {
    pub label_map: HashMap<String, VisionTarget>,
    pub unknown_label_policy: UnknownLabelPolicy,
    pub fps: Option<f64>,
    pub frame_width: Option<f64>,
    pub frame_height: Option<f64>,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct VisionError {
    pub code: VisionErrorCode,
    pub message: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisionErrorCode {
    ConfigInvalid,
    ProcessFailed,
    Timeout,
    Aborted,
    OutputInvalid,
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct VisionConfigError {
    pub message: String,
    pub field: Option<String>,
}

impl VisionConfigError {
    pub fn new(message: impl Into<String>, field: Option<&str>) -> Self {
        Self {
            message: message.into(),
            field: field.map(str::to_string),
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct VisionOutputError {
    pub message: String,
    pub line: Option<usize>,
}

impl VisionOutputError {
    pub fn new(message: impl Into<String>, line: Option<usize>) -> Self {
        Self {
            message: message.into(),
            line,
        }
    }
}

#[derive(Debug, thiserror::Error)]
#[error("{message}")]
pub struct VisionEngineError {
    pub message: String,
}

impl VisionEngineError {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

pub(crate) fn round_coordinate(value: f64) -> f64 {
    format!("{:.12}", value).parse().unwrap_or(value)
}

pub(crate) fn clamp01(value: f64) -> f64 {
    value.clamp(0.0, 1.0)
}

pub(crate) fn normalize_label_key(value: &str) -> String {
    value.trim().to_lowercase()
}

pub(crate) fn is_record(value: &serde_json::Value) -> bool {
    value.is_object()
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
