use super::{
    normalize_label_key, UnknownLabelPolicy, VisionConfigError, VisionTarget, DEFAULT_CONFIDENCE,
    DEFAULT_IOU, DEFAULT_TIMEOUT_MS, DEFAULT_TRACKER, VISION_TARGETS, YOLO26_SMALL_MODEL,
};
use regex::Regex;
use std::collections::{HashMap, HashSet};
use std::sync::LazyLock;

static STOCK_SMALL_MODEL_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)^yolo26s(?:-(?:seg|sem|depth|cls|pose|obb))?\.(?:pt|onnx)$").unwrap()
});
static REMOTE_WEIGHTS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)^(?:https?|ftp)://").unwrap());

const ALLOWED_LABEL_KEYS: &[&str] = &["cam", "chat", "content", "0", "1", "2"];

#[derive(Debug, Clone, PartialEq)]
pub struct Yolo26TrackingConfig {
    pub weights: String,
    pub label_map: HashMap<String, VisionTarget>,
    pub targets: Option<Vec<VisionTarget>>,
    pub cwd: Option<String>,
    pub tracker: Option<String>,
    pub confidence: Option<f64>,
    pub conf: Option<f64>,
    pub iou: Option<f64>,
    pub fps: Option<f64>,
    pub device: Option<String>,
    pub timeout_ms: Option<u64>,
    pub unknown_label_policy: Option<UnknownLabelPolicy>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedYolo26Config {
    pub weights: String,
    pub label_map: HashMap<String, VisionTarget>,
    pub cwd: Option<String>,
    pub tracker: String,
    pub confidence: f64,
    pub iou: f64,
    pub fps: Option<f64>,
    pub device: Option<String>,
    pub timeout_ms: u64,
    pub unknown_label_policy: UnknownLabelPolicy,
}

fn finite_number(value: f64, field: &str) -> Result<f64, VisionConfigError> {
    if !value.is_finite() {
        return Err(VisionConfigError::new(
            format!("{field} must be a finite number."),
            Some(field),
        ));
    }
    Ok(value)
}

fn threshold(value: Option<f64>, field: &str, fallback: f64) -> Result<f64, VisionConfigError> {
    match value {
        None => Ok(fallback),
        Some(number) => {
            let number = finite_number(number, field)?;
            if !(0.0..=1.0).contains(&number) {
                return Err(VisionConfigError::new(
                    format!("{field} must be between 0 and 1."),
                    Some(field),
                ));
            }
            Ok(number)
        }
    }
}

fn positive_number(value: f64, field: &str) -> Result<f64, VisionConfigError> {
    let number = finite_number(value, field)?;
    if number <= 0.0 {
        return Err(VisionConfigError::new(
            format!("{field} must be greater than 0."),
            Some(field),
        ));
    }
    Ok(number)
}

fn positive_integer(value: u64, field: &str) -> Result<u64, VisionConfigError> {
    if value == 0 {
        return Err(VisionConfigError::new(
            format!("{field} must be greater than 0."),
            Some(field),
        ));
    }
    Ok(value)
}

fn non_empty_text(value: &str, field: &str) -> Result<String, VisionConfigError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(VisionConfigError::new(
            format!("{field} must be a non-empty string."),
            Some(field),
        ));
    }
    if value.contains('\0') {
        return Err(VisionConfigError::new(
            format!("{field} must not contain a NUL byte."),
            Some(field),
        ));
    }
    Ok(trimmed.to_string())
}

fn assert_targets(value: Option<&[VisionTarget]>) -> Result<(), VisionConfigError> {
    let Some(value) = value else {
        return Ok(());
    };
    if value.len() != VISION_TARGETS.len() {
        return Err(VisionConfigError::new(
            format!(
                "targets must contain exactly {} and {}.",
                VISION_TARGETS[0], VISION_TARGETS[2]
            ),
            Some("targets"),
        ));
    }
    let mut values = std::collections::HashSet::new();
    for target in value {
        values.insert(target.as_str());
    }
    if values.len() != VISION_TARGETS.len() || !VISION_TARGETS.iter().all(|t| values.contains(t)) {
        return Err(VisionConfigError::new(
            format!(
                "targets must contain exactly {} and {}.",
                VISION_TARGETS[0], VISION_TARGETS[2]
            ),
            Some("targets"),
        ));
    }
    Ok(())
}

fn assert_custom_weights(weights: &str) -> Result<(), VisionConfigError> {
    if REMOTE_WEIGHTS_RE.is_match(weights) {
        return Err(VisionConfigError::new(
            "weights must be a local custom-trained checkpoint; remote URLs are rejected to prevent implicit downloads.",
            Some("weights"),
        ));
    }
    if STOCK_SMALL_MODEL_RE.is_match(weights) || weights == YOLO26_SMALL_MODEL {
        return Err(VisionConfigError::new(
            format!(
                "The stock {YOLO26_SMALL_MODEL} checkpoint is COCO-trained and does not define cam/chat/content. \
                 Pass a local custom-trained weights path instead."
            ),
            Some("weights"),
        ));
    }
    Ok(())
}

fn expected_target_for_key(key: &str) -> Option<VisionTarget> {
    match key {
        "cam" | "0" => Some(VisionTarget::Cam),
        "chat" | "1" => Some(VisionTarget::Chat),
        "content" | "2" => Some(VisionTarget::Content),
        _ => None,
    }
}

fn is_allowed_label_key(key: &str) -> bool {
    ALLOWED_LABEL_KEYS.contains(&key)
}

/// Identity map for models that emit cam/chat/content and class IDs 0/1/2.
pub fn identity_label_map() -> HashMap<String, VisionTarget> {
    HashMap::from([
        ("cam".to_string(), VisionTarget::Cam),
        ("chat".to_string(), VisionTarget::Chat),
        ("content".to_string(), VisionTarget::Content),
        ("0".to_string(), VisionTarget::Cam),
        ("1".to_string(), VisionTarget::Chat),
        ("2".to_string(), VisionTarget::Content),
    ])
}

/// Resolve an optional or partial label map to the full identity-backed map.
pub fn resolve_label_map(
    input: &HashMap<String, VisionTarget>,
) -> Result<HashMap<String, VisionTarget>, VisionConfigError> {
    if input.is_empty() {
        return Ok(identity_label_map());
    }

    let mut user_map = HashMap::new();
    let mut covered_targets = HashSet::new();

    for (raw_key, raw_target) in input {
        let key = normalize_label_key(raw_key);
        if key.is_empty() {
            return Err(VisionConfigError::new(
                "labelMap keys must be non-empty strings.",
                Some("labelMap"),
            ));
        }
        if !is_allowed_label_key(&key) {
            return Err(VisionConfigError::new(
                format!(
                    "labelMap key \"{raw_key}\" is not supported. Keys must be cam, chat, content or class IDs 0, 1, 2."
                ),
                Some("labelMap"),
            ));
        }
        if user_map.contains_key(&key) {
            return Err(VisionConfigError::new(
                format!("labelMap contains duplicate key after normalization: {raw_key}."),
                Some("labelMap"),
            ));
        }
        let expected = expected_target_for_key(&key).expect("allowed key has expected target");
        if *raw_target != expected {
            return Err(VisionConfigError::new(
                format!("labelMap.{raw_key} must be {}.", expected.as_str()),
                Some("labelMap"),
            ));
        }
        user_map.insert(key, *raw_target);
        covered_targets.insert(*raw_target);
    }

    if !VISION_TARGETS.iter().all(|target| {
        covered_targets
            .iter()
            .any(|mapped| mapped.as_str() == *target)
    }) {
        return Err(VisionConfigError::new(
            "labelMap must include entries for all three targets (cam, chat, and content).",
            Some("labelMap"),
        ));
    }

    let mut merged = identity_label_map();
    for (key, target) in user_map {
        merged.insert(key, target);
    }
    Ok(merged)
}

/// Validate and normalize configuration without touching the filesystem.
pub fn validate_yolo26_config(
    input: &Yolo26TrackingConfig,
) -> Result<ValidatedYolo26Config, VisionConfigError> {
    let weights = non_empty_text(&input.weights, "weights")?;
    assert_custom_weights(&weights)?;
    assert_targets(input.targets.as_deref())?;

    let label_map = resolve_label_map(&input.label_map)?;

    if let (Some(confidence), Some(conf)) = (input.confidence, input.conf) {
        if (confidence - conf).abs() > f64::EPSILON {
            return Err(VisionConfigError::new(
                "confidence and conf cannot disagree.",
                Some("confidence"),
            ));
        }
    }

    let confidence_value = input.confidence.or(input.conf);
    let tracker = match &input.tracker {
        None => DEFAULT_TRACKER.to_string(),
        Some(value) => non_empty_text(value, "tracker")?,
    };
    let cwd = match &input.cwd {
        None => None,
        Some(value) => Some(non_empty_text(value, "cwd")?),
    };
    let device = match &input.device {
        None => None,
        Some(value) => Some(non_empty_text(value, "device")?),
    };
    let confidence = threshold(confidence_value, "confidence", DEFAULT_CONFIDENCE)?;
    let iou = threshold(input.iou, "iou", DEFAULT_IOU)?;
    let fps = match input.fps {
        None => None,
        Some(value) => Some(positive_number(value, "fps")?),
    };
    let timeout_ms = match input.timeout_ms {
        None => DEFAULT_TIMEOUT_MS,
        Some(value) => positive_integer(value, "timeoutMs")?,
    };
    let unknown_label_policy = input
        .unknown_label_policy
        .unwrap_or(UnknownLabelPolicy::Ignore);

    Ok(ValidatedYolo26Config {
        weights,
        label_map,
        cwd,
        tracker,
        confidence,
        iou,
        fps,
        device,
        timeout_ms,
        unknown_label_policy,
    })
}

pub fn validate_vision_config(
    input: &Yolo26TrackingConfig,
) -> Result<ValidatedYolo26Config, VisionConfigError> {
    validate_yolo26_config(input)
}
