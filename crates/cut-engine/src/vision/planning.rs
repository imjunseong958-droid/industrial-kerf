use super::{
    clamp01, round_coordinate, track_order, NormalizedBoundingBox, NormalizedDetection,
    NormalizedTrackingResult, VisionTarget, VISION_TARGETS,
};
use serde::{Deserialize, Serialize, Serializer};
use serde_json::Value;

const DEFAULT_PLAN_CONFIDENCE: f64 = 0.25;
const DEFAULT_MAX_GAP_SECONDS: f64 = 0.5;
const DEFAULT_SMOOTHING_FACTOR: f64 = 0.5;
const EPSILON: f64 = 1e-12;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RoiKeyframe {
    pub segment_id: String,
    pub track_id: super::TrackId,
    #[serde(rename = "class")]
    pub class_: VisionTarget,
    pub frame: u64,
    pub time: f64,
    pub confidence: f64,
    pub bbox: NormalizedBoundingBox,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingSegment {
    pub id: String,
    pub track_id: super::TrackId,
    #[serde(rename = "class")]
    pub class_: VisionTarget,
    pub start_time: f64,
    pub end_time: f64,
    pub keyframes: Vec<RoiKeyframe>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TrackingPlan {
    pub segments: Vec<TrackingSegment>,
    pub by_class: ClassSegments,
    pub keyframes: Vec<RoiKeyframe>,
    pub keyframes_by_class: ClassKeyframes,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClassSegments {
    pub cam: Vec<TrackingSegment>,
    pub chat: Vec<TrackingSegment>,
    pub content: Vec<TrackingSegment>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClassKeyframes {
    pub cam: Vec<RoiKeyframe>,
    pub chat: Vec<RoiKeyframe>,
    pub content: Vec<RoiKeyframe>,
}

#[derive(Debug, Clone, Default)]
pub struct TrackingPlanOptions {
    pub min_confidence: Option<f64>,
    pub confidence_threshold: Option<f64>,
    pub max_gap_seconds: Option<f64>,
    pub max_gap: Option<f64>,
    pub smoothing_factor: Option<f64>,
    pub smoothing_alpha: Option<f64>,
}

struct ResolvedPlanOptions {
    min_confidence: f64,
    max_gap_seconds: f64,
    smoothing_factor: f64,
}

struct PlannedDetection {
    source: NormalizedDetection,
    bbox: NormalizedBoundingBox,
}

struct SegmentDraft {
    class_: VisionTarget,
    track_id: super::TrackId,
    detections: Vec<PlannedDetection>,
}

fn finite(value: f64, field: &str) -> Result<f64, RangeError> {
    if !value.is_finite() {
        return Err(RangeError::new(format!("{field} must be a finite number.")));
    }
    Ok(value)
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct RangeError {
    pub message: String,
}

impl RangeError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

fn range(value: f64, field: &str, minimum: f64, maximum: f64) -> Result<f64, RangeError> {
    let number = finite(value, field)?;
    if number < minimum || number > maximum {
        return Err(RangeError::new(format!(
            "{field} must be between {minimum} and {maximum}."
        )));
    }
    Ok(number)
}

fn positive_or_zero(value: f64, field: &str) -> Result<f64, RangeError> {
    let number = finite(value, field)?;
    if number < 0.0 {
        return Err(RangeError::new(format!(
            "{field} must be greater than or equal to zero."
        )));
    }
    Ok(number)
}

fn resolve_alias(
    primary: Option<f64>,
    alias: Option<f64>,
    field: &str,
    default_value: f64,
) -> Result<f64, RangeError> {
    if let (Some(primary), Some(alias)) = (primary, alias) {
        if (primary - alias).abs() > f64::EPSILON {
            return Err(RangeError::new(format!("{field} aliases cannot disagree.")));
        }
    }
    Ok(primary.or(alias).unwrap_or(default_value))
}

fn resolve_plan_options(options: &TrackingPlanOptions) -> Result<ResolvedPlanOptions, RangeError> {
    let min_confidence = range(
        resolve_alias(
            options.min_confidence,
            options.confidence_threshold,
            "confidence",
            DEFAULT_PLAN_CONFIDENCE,
        )?,
        "minConfidence",
        0.0,
        1.0,
    )?;
    let max_gap_seconds = positive_or_zero(
        resolve_alias(
            options.max_gap_seconds,
            options.max_gap,
            "maxGapSeconds",
            DEFAULT_MAX_GAP_SECONDS,
        )?,
        "maxGapSeconds",
    )?;
    let smoothing_factor = range(
        resolve_alias(
            options.smoothing_factor,
            options.smoothing_alpha,
            "smoothingFactor",
            DEFAULT_SMOOTHING_FACTOR,
        )?,
        "smoothingFactor",
        0.0,
        1.0,
    )?;
    Ok(ResolvedPlanOptions {
        min_confidence,
        max_gap_seconds,
        smoothing_factor,
    })
}

fn normalize_box(bbox: &NormalizedBoundingBox) -> Result<NormalizedBoundingBox, RangeError> {
    let x = finite(bbox.x, "bbox.x")?;
    let y = finite(bbox.y, "bbox.y")?;
    let width = finite(bbox.width, "bbox.width")?;
    let height = finite(bbox.height, "bbox.height")?;
    if width < 0.0 || height < 0.0 {
        return Err(RangeError::new(
            "bbox width and height must not be negative.",
        ));
    }
    let left = clamp01(x);
    let top = clamp01(y);
    let right = clamp01(x + width);
    let bottom = clamp01(y + height);
    Ok(NormalizedBoundingBox {
        x: round_coordinate(left),
        y: round_coordinate(top),
        width: round_coordinate((right - left).max(0.0)),
        height: round_coordinate((bottom - top).max(0.0)),
    })
}

fn normalize_detection(
    detection: &NormalizedDetection,
) -> Result<PlannedDetection, PlanningInputError> {
    if !VISION_TARGETS
        .iter()
        .any(|target| VisionTarget::from_str(target) == Some(detection.class_))
    {
        return Err(PlanningInputError::new(format!(
            "Planning detections must use {} classes.",
            VISION_TARGETS.join(", ")
        )));
    }
    let frame = finite(detection.frame as f64, "frame").map_err(PlanningInputError::from_range)?;
    let time = finite(detection.time, "time").map_err(PlanningInputError::from_range)?;
    range(detection.confidence, "confidence", 0.0, 1.0).map_err(PlanningInputError::from_range)?;
    if detection.frame as f64 != frame || frame < 0.0 || frame.fract() != 0.0 {
        return Err(PlanningInputError::new(
            "frame must be a non-negative integer.",
        ));
    }
    if time < 0.0 {
        return Err(PlanningInputError::new("time must be non-negative."));
    }
    match &detection.track_id {
        super::TrackId::String(s) if s.trim().is_empty() => {
            return Err(PlanningInputError::new(
                "trackId must be a non-empty string or number.",
            ));
        }
        _ => {}
    }
    Ok(PlannedDetection {
        source: detection.clone(),
        bbox: normalize_box(&detection.bbox).map_err(PlanningInputError::from_range)?,
    })
}

#[derive(Debug, Clone, PartialEq, thiserror::Error)]
#[error("{message}")]
pub struct PlanningInputError {
    pub message: String,
}

impl PlanningInputError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    fn from_range(error: RangeError) -> Self {
        Self::new(error.message)
    }
}

fn source_detections<'a>(
    input: &'a TrackingPlanInput<'a>,
) -> Result<&'a [NormalizedDetection], PlanningInputError> {
    match input {
        TrackingPlanInput::Detections(detections) => Ok(detections),
        TrackingPlanInput::Result(result) => Ok(&result.detections),
    }
}

pub enum TrackingPlanInput<'a> {
    Detections(&'a [NormalizedDetection]),
    Result(&'a NormalizedTrackingResult),
}

fn class_order(value: VisionTarget) -> usize {
    value.order()
}

fn detection_order(a: &PlannedDetection, b: &PlannedDetection) -> std::cmp::Ordering {
    let a_source = &a.source;
    let b_source = &b.source;
    a_source
        .time
        .partial_cmp(&b_source.time)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| a_source.frame.cmp(&b_source.frame))
        .then_with(|| class_order(a_source.class_).cmp(&class_order(b_source.class_)))
        .then_with(|| track_order(&a_source.track_id, &b_source.track_id))
        .then_with(|| {
            a.bbox
                .x
                .partial_cmp(&b.bbox.x)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            a.bbox
                .y
                .partial_cmp(&b.bbox.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            a.bbox
                .width
                .partial_cmp(&b.bbox.width)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            a.bbox
                .height
                .partial_cmp(&b.bbox.height)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            b_source
                .confidence
                .partial_cmp(&a_source.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

fn group_key(value: &PlannedDetection) -> String {
    let track_kind = match &value.source.track_id {
        super::TrackId::Number(_) => "number",
        super::TrackId::String(_) => "string",
    };
    format!(
        "{}\0{}\0{}",
        value.source.class_.as_str(),
        track_kind,
        match &value.source.track_id {
            super::TrackId::Number(n) => n.to_string(),
            super::TrackId::String(s) => s.clone(),
        }
    )
}

fn blend_box(
    previous: &NormalizedBoundingBox,
    current: &NormalizedBoundingBox,
    factor: f64,
) -> NormalizedBoundingBox {
    let blend = |before: f64, now: f64| round_coordinate(before * (1.0 - factor) + now * factor);
    NormalizedBoundingBox {
        x: blend(previous.x, current.x),
        y: blend(previous.y, current.y),
        width: blend(previous.width, current.width),
        height: blend(previous.height, current.height),
    }
}

fn smooth_segment(detections: &[PlannedDetection], factor: f64) -> Vec<NormalizedBoundingBox> {
    let mut boxes = Vec::with_capacity(detections.len());
    for (index, detection) in detections.iter().enumerate() {
        let box_ = if index == 0 {
            detection.bbox.clone()
        } else {
            blend_box(&boxes[index - 1], &detection.bbox, factor)
        };
        boxes.push(box_);
    }
    boxes
}

fn make_segment_id(segment: &SegmentDraft, ordinal: usize) -> String {
    let track = match &segment.track_id {
        super::TrackId::Number(n) => n.to_string(),
        super::TrackId::String(s) => s
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect(),
    };
    format!("{}:{track}:{ordinal}", segment.class_.as_str())
}

fn by_class_segments(segments: &[TrackingSegment]) -> ClassSegments {
    ClassSegments {
        cam: segments
            .iter()
            .filter(|segment| segment.class_ == VisionTarget::Cam)
            .cloned()
            .collect(),
        chat: segments
            .iter()
            .filter(|segment| segment.class_ == VisionTarget::Chat)
            .cloned()
            .collect(),
        content: segments
            .iter()
            .filter(|segment| segment.class_ == VisionTarget::Content)
            .cloned()
            .collect(),
    }
}

fn by_class_keyframes(keyframes: &[RoiKeyframe]) -> ClassKeyframes {
    ClassKeyframes {
        cam: keyframes
            .iter()
            .filter(|keyframe| keyframe.class_ == VisionTarget::Cam)
            .cloned()
            .collect(),
        chat: keyframes
            .iter()
            .filter(|keyframe| keyframe.class_ == VisionTarget::Chat)
            .cloned()
            .collect(),
        content: keyframes
            .iter()
            .filter(|keyframe| keyframe.class_ == VisionTarget::Content)
            .cloned()
            .collect(),
    }
}

/// Build stable, class-separated tracking segments and smoothed ROI keyframes.
pub fn build_tracking_plan(
    input: TrackingPlanInput<'_>,
    options: &TrackingPlanOptions,
) -> Result<TrackingPlan, PlanningInputError> {
    let resolved = resolve_plan_options(options).map_err(PlanningInputError::from_range)?;
    let mut filtered = Vec::new();
    for detection in source_detections(&input)? {
        let normalized = normalize_detection(detection)?;
        if normalized.source.confidence >= resolved.min_confidence {
            filtered.push(normalized);
        }
    }
    filtered.sort_by(detection_order);

    let mut groups: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    let mut drafts: Vec<SegmentDraft> = Vec::new();
    for detection in filtered {
        let key = group_key(&detection);
        if let Some(&index) = groups.get(&key) {
            let Some(previous) = drafts[index].detections.last() else {
                continue;
            };
            if detection.source.time - previous.source.time > resolved.max_gap_seconds {
                let next = SegmentDraft {
                    class_: detection.source.class_,
                    track_id: detection.source.track_id.clone(),
                    detections: vec![detection],
                };
                groups.insert(key, drafts.len());
                drafts.push(next);
            } else {
                drafts[index].detections.push(detection);
            }
        } else {
            let next = SegmentDraft {
                class_: detection.source.class_,
                track_id: detection.source.track_id.clone(),
                detections: vec![detection],
            };
            groups.insert(key, drafts.len());
            drafts.push(next);
        }
    }

    drafts.sort_by(|a, b| {
        let a_first = a.detections.first().map(|d| &d.source);
        let b_first = b.detections.first().map(|d| &d.source);
        match (a_first, b_first) {
            (Some(a_first), Some(b_first)) => a_first
                .time
                .partial_cmp(&b_first.time)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| class_order(a.class_).cmp(&class_order(b.class_)))
                .then_with(|| track_order(&a.track_id, &b.track_id)),
            _ => std::cmp::Ordering::Equal,
        }
    });

    let mut segments = Vec::new();
    let mut all_keyframes = Vec::new();
    for (ordinal, draft) in drafts.iter().enumerate() {
        let id = make_segment_id(draft, ordinal);
        let boxes = smooth_segment(&draft.detections, resolved.smoothing_factor);
        let Some(first) = draft.detections.first() else {
            continue;
        };
        let Some(last) = draft.detections.last() else {
            continue;
        };
        let keyframes = draft
            .detections
            .iter()
            .enumerate()
            .map(|(index, detection)| RoiKeyframe {
                segment_id: id.clone(),
                track_id: draft.track_id.clone(),
                class_: draft.class_,
                frame: detection.source.frame,
                time: detection.source.time,
                confidence: detection.source.confidence,
                bbox: boxes[index].clone(),
            })
            .collect::<Vec<_>>();
        let segment = TrackingSegment {
            id,
            track_id: draft.track_id.clone(),
            class_: draft.class_,
            start_time: first.source.time,
            end_time: last.source.time,
            keyframes: keyframes.clone(),
        };
        segments.push(segment);
        all_keyframes.extend(keyframes);
    }

    segments.sort_by(|a, b| {
        a.start_time
            .partial_cmp(&b.start_time)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| class_order(a.class_).cmp(&class_order(b.class_)))
            .then_with(|| track_order(&a.track_id, &b.track_id))
            .then_with(|| a.id.cmp(&b.id))
    });
    all_keyframes.sort_by(|a, b| {
        a.time
            .partial_cmp(&b.time)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| class_order(a.class_).cmp(&class_order(b.class_)))
            .then_with(|| track_order(&a.track_id, &b.track_id))
            .then_with(|| a.frame.cmp(&b.frame))
            .then_with(|| a.segment_id.cmp(&b.segment_id))
    });

    let by_class = by_class_segments(&segments);
    let keyframes_by_class = by_class_keyframes(&all_keyframes);
    Ok(TrackingPlan {
        segments,
        by_class,
        keyframes: all_keyframes,
        keyframes_by_class,
    })
}

pub fn plan_tracking(
    input: TrackingPlanInput<'_>,
    options: &TrackingPlanOptions,
) -> Result<TrackingPlan, PlanningInputError> {
    build_tracking_plan(input, options)
}

#[derive(Debug, Clone)]
pub struct CropPlanRequest {
    pub aspect_ratio: f64,
    pub source_aspect_ratio: Option<f64>,
    pub source_width: Option<f64>,
    pub source_height: Option<f64>,
    pub cam: Option<CropTarget>,
    pub chat: Option<CropTarget>,
    pub content: Option<CropTarget>,
    pub padding: Option<f64>,
}

#[derive(Debug, Clone)]
pub enum CropTarget {
    Box(NormalizedBoundingBox),
    Detection(NormalizedDetection),
    Keyframe(RoiKeyframe),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CropPlanMode {
    Center,
    Cam,
    Chat,
    Content,
    CamChat,
    CamContent,
    ChatContent,
    All,
}

impl Serialize for CropPlanMode {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.as_str())
    }
}

impl CropPlanMode {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Center => "center",
            Self::Cam => "cam",
            Self::Chat => "chat",
            Self::Content => "content",
            Self::CamChat => "cam_chat",
            Self::CamContent => "cam_content",
            Self::ChatContent => "chat_content",
            Self::All => "all",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CropPlan {
    pub aspect_ratio: f64,
    pub source_aspect_ratio: f64,
    pub crop: NormalizedBoundingBox,
    pub bbox: NormalizedBoundingBox,
    pub included: CropIncluded,
    pub mode: CropPlanMode,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CropIncluded {
    pub cam: bool,
    pub chat: bool,
    pub content: bool,
}

fn target_box(target: Option<&CropTarget>) -> Result<Option<NormalizedBoundingBox>, RangeError> {
    match target {
        None => Ok(None),
        Some(CropTarget::Box(box_)) => Ok(Some(normalize_box(box_)?)),
        Some(CropTarget::Detection(detection)) => Ok(Some(normalize_box(&detection.bbox)?)),
        Some(CropTarget::Keyframe(keyframe)) => Ok(Some(normalize_box(&keyframe.bbox)?)),
    }
}

fn union_box(boxes: &[NormalizedBoundingBox]) -> Option<NormalizedBoundingBox> {
    if boxes.is_empty() {
        return None;
    }
    let left = boxes
        .iter()
        .map(|box_| box_.x)
        .fold(f64::INFINITY, f64::min);
    let top = boxes
        .iter()
        .map(|box_| box_.y)
        .fold(f64::INFINITY, f64::min);
    let right = boxes
        .iter()
        .map(|box_| box_.x + box_.width)
        .fold(f64::NEG_INFINITY, f64::max);
    let bottom = boxes
        .iter()
        .map(|box_| box_.y + box_.height)
        .fold(f64::NEG_INFINITY, f64::max);
    normalize_box(&NormalizedBoundingBox {
        x: left,
        y: top,
        width: right - left,
        height: bottom - top,
    })
    .ok()
}

fn expand_box(
    bbox: &NormalizedBoundingBox,
    padding: f64,
) -> Result<NormalizedBoundingBox, RangeError> {
    normalize_box(&NormalizedBoundingBox {
        x: bbox.x - padding,
        y: bbox.y - padding,
        width: bbox.width + padding * 2.0,
        height: bbox.height + padding * 2.0,
    })
}

fn contains(outer: &NormalizedBoundingBox, inner: &NormalizedBoundingBox) -> bool {
    outer.x <= inner.x + EPSILON
        && outer.y <= inner.y + EPSILON
        && outer.x + outer.width + EPSILON >= inner.x + inner.width
        && outer.y + outer.height + EPSILON >= inner.y + inner.height
}

fn crop_mode(
    cam: Option<&NormalizedBoundingBox>,
    chat: Option<&NormalizedBoundingBox>,
    content: Option<&NormalizedBoundingBox>,
) -> CropPlanMode {
    let mut selected = Vec::new();
    if cam.is_some() {
        selected.push("cam");
    }
    if chat.is_some() {
        selected.push("chat");
    }
    if content.is_some() {
        selected.push("content");
    }
    match selected.len() {
        0 => CropPlanMode::Center,
        1 => match selected[0] {
            "cam" => CropPlanMode::Cam,
            "chat" => CropPlanMode::Chat,
            _ => CropPlanMode::Content,
        },
        2 => match (cam.is_some(), chat.is_some(), content.is_some()) {
            (true, true, false) => CropPlanMode::CamChat,
            (true, false, true) => CropPlanMode::CamContent,
            _ => CropPlanMode::ChatContent,
        },
        _ => CropPlanMode::All,
    }
}

/// Fit a target-aspect crop around selected cam, chat, and/or content regions.
pub fn plan_crop_for_aspect_ratio(request: &CropPlanRequest) -> Result<CropPlan, RangeError> {
    let aspect_ratio = finite(request.aspect_ratio, "aspectRatio")?;
    if aspect_ratio <= 0.0 {
        return Err(RangeError::new("aspectRatio must be greater than zero."));
    }
    let has_source_width = request.source_width.is_some();
    let has_source_height = request.source_height.is_some();
    if has_source_width != has_source_height {
        return Err(RangeError::new(
            "sourceWidth and sourceHeight must be provided together.",
        ));
    }
    let mut source_aspect_ratio = request.source_aspect_ratio;
    if has_source_width && has_source_height {
        let source_width = positive_or_zero(request.source_width.unwrap(), "sourceWidth")?;
        let source_height = positive_or_zero(request.source_height.unwrap(), "sourceHeight")?;
        if source_width <= 0.0 || source_height <= 0.0 {
            return Err(RangeError::new(
                "sourceWidth and sourceHeight must be greater than zero.",
            ));
        }
        let derived = source_width / source_height;
        if let Some(existing) = source_aspect_ratio {
            if (existing - derived).abs() > EPSILON {
                return Err(RangeError::new(
                    "sourceAspectRatio disagrees with sourceWidth/sourceHeight.",
                ));
            }
        }
        source_aspect_ratio = Some(derived);
    }
    let source_aspect_ratio = match source_aspect_ratio {
        None => 1.0,
        Some(value) => finite(value, "sourceAspectRatio")?,
    };
    if source_aspect_ratio <= 0.0 {
        return Err(RangeError::new(
            "sourceAspectRatio must be greater than zero.",
        ));
    }
    let padding = match request.padding {
        None => 0.0,
        Some(value) => positive_or_zero(value, "padding")?,
    };
    let cam = target_box(request.cam.as_ref())?;
    let chat = target_box(request.chat.as_ref())?;
    let content = target_box(request.content.as_ref())?;
    let selected: Vec<NormalizedBoundingBox> = [cam.clone(), chat.clone(), content.clone()]
        .into_iter()
        .flatten()
        .collect();
    let union = union_box(&selected);
    let padded = match union.as_ref() {
        None => None,
        Some(bbox) => Some(expand_box(bbox, padding)?),
    };

    let base = padded.unwrap_or(NormalizedBoundingBox {
        x: 0.0,
        y: 0.0,
        width: 1.0,
        height: 1.0,
    });
    let base_width = base.width.max(EPSILON);
    let base_height = base.height.max(EPSILON);
    let normalized_aspect_ratio = aspect_ratio / source_aspect_ratio;
    let mut crop_width = base_width.max(base_height * normalized_aspect_ratio);
    let mut crop_height = base_height.max(base_width / normalized_aspect_ratio);
    if crop_width > 1.0 {
        crop_width = 1.0;
        crop_height = 1.0 / normalized_aspect_ratio;
    }
    if crop_height > 1.0 {
        crop_height = 1.0;
        crop_width = normalized_aspect_ratio;
    }
    crop_width = crop_width.clamp(EPSILON, 1.0);
    crop_height = crop_height.clamp(EPSILON, 1.0);

    let center_x = union
        .as_ref()
        .map(|box_| box_.x + box_.width / 2.0)
        .unwrap_or(0.5);
    let center_y = union
        .as_ref()
        .map(|box_| box_.y + box_.height / 2.0)
        .unwrap_or(0.5);
    let left = (center_x - crop_width / 2.0).clamp(0.0, 1.0 - crop_width);
    let top = (center_y - crop_height / 2.0).clamp(0.0, 1.0 - crop_height);
    let crop = normalize_box(&NormalizedBoundingBox {
        x: left,
        y: top,
        width: crop_width,
        height: crop_height,
    })?;
    let included = CropIncluded {
        cam: cam.as_ref().is_some_and(|box_| contains(&crop, box_)),
        chat: chat.as_ref().is_some_and(|box_| contains(&crop, box_)),
        content: content.as_ref().is_some_and(|box_| contains(&crop, box_)),
    };
    let mode = crop_mode(cam.as_ref(), chat.as_ref(), content.as_ref());
    Ok(CropPlan {
        aspect_ratio,
        source_aspect_ratio,
        bbox: crop.clone(),
        crop,
        included,
        mode,
    })
}

pub fn build_crop_plan(request: &CropPlanRequest) -> Result<CropPlan, RangeError> {
    plan_crop_for_aspect_ratio(request)
}

fn json_f64(object: &serde_json::Map<String, Value>, key: &str) -> Option<f64> {
    object.get(key).and_then(Value::as_f64)
}

fn crop_target_from_json(value: Option<&Value>) -> Result<Option<CropTarget>, RangeError> {
    let Some(value) = value else {
        return Ok(None);
    };
    if value.is_null() {
        return Ok(None);
    }
    if value.get("bbox").is_some() {
        if let Ok(detection) = serde_json::from_value::<NormalizedDetection>(value.clone()) {
            return Ok(Some(CropTarget::Detection(detection)));
        }
        if let Ok(keyframe) = serde_json::from_value::<RoiKeyframe>(value.clone()) {
            return Ok(Some(CropTarget::Keyframe(keyframe)));
        }
    }
    let bbox: NormalizedBoundingBox = serde_json::from_value(value.clone())
        .map_err(|_| RangeError::new("crop target must be a bounding box or detection"))?;
    Ok(Some(CropTarget::Box(bbox)))
}

/// Build a crop plan from the TypeScript `CropPlanRequest` JSON shape.
pub fn crop_plan_from_json(value: &Value) -> Result<CropPlan, RangeError> {
    let object = value
        .as_object()
        .ok_or_else(|| RangeError::new("crop request must be an object"))?;
    let aspect_ratio = json_f64(object, "aspectRatio")
        .ok_or_else(|| RangeError::new("aspectRatio must be a finite number."))?;
    plan_crop_for_aspect_ratio(&CropPlanRequest {
        aspect_ratio,
        source_aspect_ratio: json_f64(object, "sourceAspectRatio"),
        source_width: json_f64(object, "sourceWidth"),
        source_height: json_f64(object, "sourceHeight"),
        cam: crop_target_from_json(object.get("cam"))?,
        chat: crop_target_from_json(object.get("chat"))?,
        content: crop_target_from_json(object.get("content"))?,
        padding: json_f64(object, "padding"),
    })
}
