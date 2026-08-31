use super::{
    clamp01, is_record, normalize_label_key, round_coordinate, track_order, NormalizedBoundingBox,
    NormalizedDetection, ParsedTrackingOutput, TrackId, UnknownLabelPolicy, VisionOutputError,
    VisionTarget,
};
use serde_json::Value;
use std::collections::HashMap;

struct FrameContext {
    index: u64,
    time: f64,
    width: Option<f64>,
    height: Option<f64>,
    fps: Option<f64>,
    detections: Vec<Value>,
}

fn number_value(value: &Value, name: &str, line: Option<usize>) -> Result<f64, VisionOutputError> {
    match value.as_f64() {
        Some(number) if number.is_finite() => Ok(number),
        _ => Err(VisionOutputError::new(
            format!("{name} must be a finite number."),
            line,
        )),
    }
}

fn non_negative_integer(
    value: &Value,
    name: &str,
    line: Option<usize>,
) -> Result<u64, VisionOutputError> {
    let number = number_value(value, name, line)?;
    if number < 0.0 || number.fract() != 0.0 {
        return Err(VisionOutputError::new(
            format!("{name} must be a non-negative integer."),
            line,
        ));
    }
    Ok(number as u64)
}

fn optional_number(
    value: &Value,
    name: &str,
    line: Option<usize>,
) -> Result<Option<f64>, VisionOutputError> {
    if value.is_null() {
        return Ok(None);
    }
    Ok(Some(number_value(value, name, line)?))
}

fn parse_track_id(value: &Value, line: Option<usize>) -> Result<TrackId, VisionOutputError> {
    if let Some(number) = value.as_i64() {
        if number < 0 {
            return Err(VisionOutputError::new(
                "track_id must be a non-negative integer or non-empty string.",
                line,
            ));
        }
        return Ok(TrackId::Number(number));
    }
    if let Some(text) = value.as_str() {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return Err(VisionOutputError::new("track_id must not be empty.", line));
        }
        if trimmed.chars().all(|c| c.is_ascii_digit()) {
            if let Ok(number) = trimmed.parse::<i64>() {
                return Ok(TrackId::Number(number));
            }
        }
        return Ok(TrackId::String(trimmed.to_string()));
    }
    Err(VisionOutputError::new(
        "Every detection must include a track_id.",
        line,
    ))
}

fn parse_dimensions(
    frame: &Value,
    fallback_width: Option<f64>,
    fallback_height: Option<f64>,
    line: Option<usize>,
) -> Result<(Option<f64>, Option<f64>), VisionOutputError> {
    let mut width = optional_number(
        frame
            .get("width")
            .or_else(|| frame.get("frame_width"))
            .unwrap_or(&Value::Null),
        "width",
        line,
    )?;
    let mut height = optional_number(
        frame
            .get("height")
            .or_else(|| frame.get("frame_height"))
            .unwrap_or(&Value::Null),
        "height",
        line,
    )?;
    let shape = frame.get("orig_shape").or_else(|| frame.get("shape"));
    if width.is_none() || height.is_none() {
        if let Some(shape) = shape.and_then(|s| s.as_array()) {
            if shape.len() >= 2 {
                let shape_height = optional_number(&shape[0], "shape[0]", line)?;
                let shape_width = optional_number(&shape[1], "shape[1]", line)?;
                if height.is_none() {
                    height = shape_height;
                }
                if width.is_none() {
                    width = shape_width;
                }
            }
        }
    }
    if width.is_none() {
        width = fallback_width;
    }
    if height.is_none() {
        height = fallback_height;
    }
    if let Some(w) = width {
        if w <= 0.0 {
            return Err(VisionOutputError::new(
                "width must be greater than zero.",
                line,
            ));
        }
    }
    if let Some(h) = height {
        if h <= 0.0 {
            return Err(VisionOutputError::new(
                "height must be greater than zero.",
                line,
            ));
        }
    }
    Ok((width, height))
}

fn array_coordinates(value: &[Value], line: Option<usize>) -> Result<[f64; 4], VisionOutputError> {
    if value.len() != 4 {
        return Err(VisionOutputError::new(
            "bbox arrays must contain four coordinates.",
            line,
        ));
    }
    Ok([
        number_value(&value[0], "bbox[0]", line)?,
        number_value(&value[1], "bbox[1]", line)?,
        number_value(&value[2], "bbox[2]", line)?,
        number_value(&value[3], "bbox[3]", line)?,
    ])
}

fn detection_flag(detection: &Value, key: &str) -> bool {
    detection
        .get(key)
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

fn parse_bbox(
    value: &Value,
    frame: &FrameContext,
    detection: &Value,
    line: Option<usize>,
) -> Result<NormalizedBoundingBox, VisionOutputError> {
    let object = if is_record(value) { Some(value) } else { None };
    let normalized_flag = detection_flag(detection, "normalized")
        || object.is_some_and(|o| detection_flag(o, "normalized"));
    let format = detection
        .get("bbox_format")
        .or_else(|| object.and_then(|o| o.get("bbox_format")))
        .and_then(|v| v.as_str());
    if let Some(fmt) = format {
        if fmt != "xyxy" && fmt != "xywh" {
            return Err(VisionOutputError::new(
                "bbox_format must be xyxy or xywh.",
                line,
            ));
        }
    }

    let (x1, y1, x2, y2) = if let Some(arr) = value.as_array() {
        let [first, second, third, fourth] = array_coordinates(arr, line)?;
        if format == Some("xywh") {
            if third < 0.0 || fourth < 0.0 {
                return Err(VisionOutputError::new(
                    "bbox width/height must not be negative.",
                    line,
                ));
            }
            (first, second, first + third, second + fourth)
        } else {
            (first, second, third, fourth)
        }
    } else if let Some(object) = object {
        let has_xyxy = ["x1", "y1", "x2", "y2"]
            .iter()
            .all(|key| object.get(*key).is_some());
        let has_ltrb = ["left", "top", "right", "bottom"]
            .iter()
            .all(|key| object.get(*key).is_some());
        let has_xywh = ["x", "y", "width", "height"]
            .iter()
            .all(|key| object.get(*key).is_some());

        if has_xyxy {
            if format == Some("xywh") {
                return Err(VisionOutputError::new(
                    "bbox_format xywh conflicts with x1/y1/x2/y2 object fields.",
                    line,
                ));
            }
            (
                number_value(object.get("x1").unwrap(), "bbox.x1", line)?,
                number_value(object.get("y1").unwrap(), "bbox.y1", line)?,
                number_value(object.get("x2").unwrap(), "bbox.x2", line)?,
                number_value(object.get("y2").unwrap(), "bbox.y2", line)?,
            )
        } else if has_ltrb {
            if format == Some("xywh") {
                return Err(VisionOutputError::new(
                    "bbox_format xywh conflicts with left/top/right/bottom object fields.",
                    line,
                ));
            }
            (
                number_value(object.get("left").unwrap(), "bbox.left", line)?,
                number_value(object.get("top").unwrap(), "bbox.top", line)?,
                number_value(object.get("right").unwrap(), "bbox.right", line)?,
                number_value(object.get("bottom").unwrap(), "bbox.bottom", line)?,
            )
        } else if has_xywh {
            if format == Some("xyxy") {
                return Err(VisionOutputError::new(
                    "bbox_format xyxy conflicts with x/y/width/height object fields.",
                    line,
                ));
            }
            let x = number_value(object.get("x").unwrap(), "bbox.x", line)?;
            let y = number_value(object.get("y").unwrap(), "bbox.y", line)?;
            let width = number_value(object.get("width").unwrap(), "bbox.width", line)?;
            let height = number_value(object.get("height").unwrap(), "bbox.height", line)?;
            if width < 0.0 || height < 0.0 {
                return Err(VisionOutputError::new(
                    "bbox width/height must not be negative.",
                    line,
                ));
            }
            (x, y, x + width, y + height)
        } else {
            return Err(VisionOutputError::new(
                "bbox must be an [x1,y1,x2,y2] array or supported object.",
                line,
            ));
        }
    } else {
        return Err(VisionOutputError::new(
            "bbox must be an [x1,y1,x2,y2] array or supported object.",
            line,
        ));
    };

    if x2 < x1 {
        return Err(VisionOutputError::new(
            "bbox x2/right must be >= x1/left.",
            line,
        ));
    }
    if y2 < y1 {
        return Err(VisionOutputError::new(
            "bbox y2/bottom must be >= y1/top.",
            line,
        ));
    }

    let (nx1, ny1, nx2, ny2) = if normalized_flag {
        (x1, y1, x2, y2)
    } else if frame.width.is_none() || frame.height.is_none() {
        if [x1, y1, x2, y2]
            .iter()
            .all(|coordinate| (0.0..=1.0).contains(coordinate))
        {
            (x1, y1, x2, y2)
        } else {
            return Err(VisionOutputError::new(
                "frame width/height are required for absolute bbox coordinates.",
                line,
            ));
        }
    } else {
        let width = frame.width.unwrap();
        let height = frame.height.unwrap();
        (x1 / width, y1 / height, x2 / width, y2 / height)
    };

    let left = clamp01(nx1);
    let top = clamp01(ny1);
    let right = clamp01(nx2);
    let bottom = clamp01(ny2);
    if right < left || bottom < top {
        return Err(VisionOutputError::new(
            "bbox must have non-negative normalized width and height.",
            line,
        ));
    }
    Ok(NormalizedBoundingBox {
        x: round_coordinate(left),
        y: round_coordinate(top),
        width: round_coordinate(right - left),
        height: round_coordinate(bottom - top),
    })
}

fn parse_frame(
    raw: &Value,
    line: usize,
    fallback: &NormalizeFallback,
) -> Result<FrameContext, VisionOutputError> {
    if !is_record(raw) {
        return Err(VisionOutputError::new(
            "Each output line must be a JSON frame object.",
            Some(line),
        ));
    }
    let index_value = raw
        .get("frame_index")
        .or_else(|| raw.get("frameIndex"))
        .or_else(|| raw.get("frame"))
        .cloned()
        .unwrap_or(Value::from(line.saturating_sub(1) as i64));
    let index = non_negative_integer(&index_value, "frame_index", Some(line))?;
    let fps = optional_number(raw.get("fps").unwrap_or(&Value::Null), "fps", Some(line))?
        .or(fallback.fps);
    if let Some(fps) = fps {
        if fps <= 0.0 {
            return Err(VisionOutputError::new(
                "fps must be greater than zero.",
                Some(line),
            ));
        }
    }
    let explicit_time = raw
        .get("time_seconds")
        .or_else(|| raw.get("timeSeconds"))
        .or_else(|| raw.get("time"));
    let time = match explicit_time {
        None | Some(Value::Null) => index as f64 / fps.unwrap_or(30.0),
        Some(value) => number_value(value, "time_seconds", Some(line))?,
    };
    if time < 0.0 {
        return Err(VisionOutputError::new(
            "time_seconds must not be negative.",
            Some(line),
        ));
    }
    let (width, height) =
        parse_dimensions(raw, fallback.frame_width, fallback.frame_height, Some(line))?;
    let raw_detections = raw
        .get("detections")
        .or_else(|| raw.get("boxes"))
        .or_else(|| raw.get("tracks"));
    let detections = match raw_detections {
        Some(Value::Array(items)) => {
            for item in items {
                if !is_record(item) {
                    return Err(VisionOutputError::new(
                        "Each detection must be an object.",
                        Some(line),
                    ));
                }
            }
            items.clone()
        }
        _ => {
            return Err(VisionOutputError::new(
                "Each frame must contain a detections array.",
                Some(line),
            ));
        }
    };
    Ok(FrameContext {
        index,
        time,
        width,
        height,
        fps,
        detections,
    })
}

fn parse_payload(value: &Value, line: usize) -> Result<Vec<Value>, VisionOutputError> {
    if let Some(array) = value.as_array() {
        return Ok(array.clone());
    }
    if !is_record(value) {
        return Err(VisionOutputError::new(
            "Runner output must contain frame objects.",
            Some(line),
        ));
    }
    if let Some(frames) = value.get("frames").and_then(|v| v.as_array()) {
        return Ok(frames.clone());
    }
    Ok(vec![value.clone()])
}

fn parse_json_lines(stdout: &str) -> Result<Vec<Value>, VisionOutputError> {
    let lines: Vec<&str> = stdout
        .split(['\r', '\n'])
        .filter(|line| !line.trim().is_empty())
        .collect();
    if lines.is_empty() {
        return Ok(Vec::new());
    }
    if lines.len() == 1 {
        let parsed: Value = serde_json::from_str(lines[0]).map_err(|error| {
            VisionOutputError::new("Runner output is not valid JSON.", Some(1))
                .with_cause(error.to_string())
        })?;
        return parse_payload(&parsed, 1);
    }
    let mut payloads = Vec::with_capacity(lines.len());
    for (index, line) in lines.iter().enumerate() {
        let parsed: Value = serde_json::from_str(line).map_err(|error| {
            VisionOutputError::new(
                "Each non-empty runner output line must be valid JSON.",
                Some(index + 1),
            )
            .with_cause(error.to_string())
        })?;
        payloads.push(parsed);
    }
    Ok(payloads)
}

fn label_candidates(detection: &Value) -> Vec<String> {
    let mut candidates = Vec::new();
    for key in [
        "class_name",
        "className",
        "label",
        "class",
        "class_id",
        "classId",
    ] {
        match detection.get(key) {
            Some(Value::String(text)) if !text.trim().is_empty() => {
                candidates.push(normalize_label_key(text));
            }
            Some(Value::Number(number)) if number.as_f64().is_some() => {
                candidates.push(number.to_string());
            }
            _ => {}
        }
    }
    candidates
}

fn resolve_target(detection: &Value, map: &HashMap<String, VisionTarget>) -> Option<VisionTarget> {
    for candidate in label_candidates(detection) {
        if let Some(target) = map.get(&candidate) {
            return Some(*target);
        }
    }
    None
}

fn confidence_of(detection: &Value, line: Option<usize>) -> Result<f64, VisionOutputError> {
    let value = detection
        .get("confidence")
        .or_else(|| detection.get("conf"))
        .ok_or_else(|| VisionOutputError::new("confidence must be a finite number.", line))?;
    let confidence = number_value(value, "confidence", line)?;
    if !(0.0..=1.0).contains(&confidence) {
        return Err(VisionOutputError::new(
            "confidence must be between 0 and 1.",
            line,
        ));
    }
    Ok(confidence)
}

fn stable_track_sort(a: &NormalizedDetection, b: &NormalizedDetection) -> std::cmp::Ordering {
    let class_order = a.class_.order().cmp(&b.class_.order());
    if class_order != std::cmp::Ordering::Equal {
        return class_order;
    }
    let id_order = track_order(&a.track_id, &b.track_id);
    if id_order != std::cmp::Ordering::Equal {
        return id_order;
    }
    a.bbox
        .x
        .partial_cmp(&b.bbox.x)
        .unwrap_or(std::cmp::Ordering::Equal)
        .then_with(|| {
            a.bbox
                .y
                .partial_cmp(&b.bbox.y)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
        .then_with(|| {
            b.confidence
                .partial_cmp(&a.confidence)
                .unwrap_or(std::cmp::Ordering::Equal)
        })
}

struct NormalizeFallback {
    fps: Option<f64>,
    frame_width: Option<f64>,
    frame_height: Option<f64>,
}

impl VisionOutputError {
    fn with_cause(mut self, cause: String) -> Self {
        self.message = format!("{} ({cause})", self.message);
        self
    }
}

/// Parse JSONL runner output and normalize detections to semantic targets.
pub fn parse_and_normalize_tracking_output(
    stdout: &str,
    options: &super::NormalizeOptions,
) -> Result<ParsedTrackingOutput, VisionOutputError> {
    let payloads = parse_json_lines(stdout)?;
    let fallback = NormalizeFallback {
        fps: options.fps,
        frame_width: options.frame_width,
        frame_height: options.frame_height,
    };
    let frames: Vec<FrameContext> = payloads
        .iter()
        .enumerate()
        .map(|(index, payload)| parse_frame(payload, index + 1, &fallback))
        .collect::<Result<_, _>>()?;

    let mut detections = Vec::new();
    let mut discovered_fps = options.fps;

    for (frame_position, frame) in frames.iter().enumerate() {
        if let Some(frame_fps) = frame.fps {
            match discovered_fps {
                None => discovered_fps = Some(frame_fps),
                Some(existing) if (existing - frame_fps).abs() > f64::EPSILON => {
                    return Err(VisionOutputError::new(
                        "Runner output contains inconsistent fps metadata.",
                        Some(frame_position + 1),
                    ));
                }
                _ => {}
            }
        }
        for (detection_position, detection) in frame.detections.iter().enumerate() {
            let line = Some(frame_position + 1);
            let target = resolve_target(detection, &options.label_map);
            if target.is_none() {
                if options.unknown_label_policy == UnknownLabelPolicy::Error {
                    return Err(VisionOutputError::new(
                        format!(
                            "Detection {detection_position} in frame {} has no labelMap entry.",
                            frame.index
                        ),
                        line,
                    ));
                }
                continue;
            }
            let track_value = detection
                .get("track_id")
                .or_else(|| detection.get("trackId"))
                .ok_or_else(|| {
                    VisionOutputError::new("Every detection must include a track_id.", line)
                })?;
            let track_id = parse_track_id(track_value, line)?;
            let bbox_value = detection
                .get("bbox")
                .or_else(|| detection.get("box"))
                .ok_or_else(|| {
                    VisionOutputError::new(
                        "bbox must be an [x1,y1,x2,y2] array or supported object.",
                        line,
                    )
                })?;
            let bbox = parse_bbox(bbox_value, frame, detection, line)?;
            let confidence = confidence_of(detection, line)?;
            let class_ = target.unwrap();
            detections.push(NormalizedDetection {
                track_id,
                class_,
                confidence,
                frame: frame.index,
                time: frame.time,
                frame_index: frame.index,
                time_seconds: frame.time,
                bbox,
            });
        }
    }

    detections.sort_by(|a, b| {
        a.frame
            .cmp(&b.frame)
            .then_with(|| {
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| stable_track_sort(a, b))
    });

    let max_frame = frames.iter().map(|frame| frame.index).max().unwrap_or(0);
    Ok(ParsedTrackingOutput {
        detections,
        frame_count: if frames.is_empty() { 0 } else { max_frame + 1 },
        fps: discovered_fps,
    })
}

/// Normalize in-memory tracker frames without a JSONL round-trip.
pub fn normalize_detection_frames(
    frames: &[crate::vision::yolo::DetectionFrame],
    options: &super::NormalizeOptions,
) -> Result<ParsedTrackingOutput, VisionOutputError> {
    let mut detections = Vec::new();
    let mut discovered_fps = options.fps;
    for (frame_position, frame) in frames.iter().enumerate() {
        let line = Some(frame_position + 1);
        if let Some(frame_fps) = frame.fps {
            if frame_fps <= 0.0 {
                return Err(VisionOutputError::new(
                    "fps must be greater than zero.",
                    line,
                ));
            }
            match discovered_fps {
                None => discovered_fps = Some(frame_fps),
                Some(existing) if (existing - frame_fps).abs() > f64::EPSILON => {
                    return Err(VisionOutputError::new(
                        "Runner output contains inconsistent fps metadata.",
                        line,
                    ));
                }
                _ => {}
            }
        }
        let time = frame
            .time_seconds
            .unwrap_or(frame.frame_index as f64 / frame.fps.or(options.fps).unwrap_or(30.0));
        if time < 0.0 {
            return Err(VisionOutputError::new(
                "time_seconds must not be negative.",
                line,
            ));
        }
        let width = frame.width as f64;
        let height = frame.height as f64;
        if width <= 0.0 || height <= 0.0 {
            return Err(VisionOutputError::new(
                "frame width/height are required for absolute bbox coordinates.",
                line,
            ));
        }
        for (detection_position, detection) in frame.detections.iter().enumerate() {
            let target = resolve_label(
                &detection.class_name,
                detection.class_id,
                &options.label_map,
            );
            if target.is_none() {
                if options.unknown_label_policy == UnknownLabelPolicy::Error {
                    return Err(VisionOutputError::new(
                        format!(
                            "Detection {detection_position} in frame {} has no labelMap entry.",
                            frame.frame_index
                        ),
                        line,
                    ));
                }
                continue;
            }
            if !(0.0..=1.0).contains(&(detection.confidence as f64)) {
                return Err(VisionOutputError::new(
                    "confidence must be between 0 and 1.",
                    line,
                ));
            }
            let bbox = xyxy_to_normalized(
                detection.bbox[0] as f64,
                detection.bbox[1] as f64,
                detection.bbox[2] as f64,
                detection.bbox[3] as f64,
                width,
                height,
                line,
            )?;
            let index = frame.frame_index as u64;
            detections.push(NormalizedDetection {
                track_id: TrackId::Number(detection.track_id as i64),
                class_: target.unwrap(),
                confidence: detection.confidence as f64,
                frame: index,
                time,
                frame_index: index,
                time_seconds: time,
                bbox,
            });
        }
    }
    detections.sort_by(|a, b| {
        a.frame
            .cmp(&b.frame)
            .then_with(|| {
                a.time
                    .partial_cmp(&b.time)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .then_with(|| stable_track_sort(a, b))
    });
    let max_frame = frames
        .iter()
        .map(|frame| frame.frame_index as u64)
        .max()
        .unwrap_or(0);
    Ok(ParsedTrackingOutput {
        detections,
        frame_count: if frames.is_empty() { 0 } else { max_frame + 1 },
        fps: discovered_fps,
    })
}

fn resolve_label(
    class_name: &str,
    class_id: usize,
    map: &HashMap<String, VisionTarget>,
) -> Option<VisionTarget> {
    map.get(&normalize_label_key(class_name))
        .copied()
        .or_else(|| map.get(&class_id.to_string()).copied())
}

fn xyxy_to_normalized(
    x1: f64,
    y1: f64,
    x2: f64,
    y2: f64,
    width: f64,
    height: f64,
    line: Option<usize>,
) -> Result<NormalizedBoundingBox, VisionOutputError> {
    if x2 < x1 {
        return Err(VisionOutputError::new(
            "bbox x2/right must be >= x1/left.",
            line,
        ));
    }
    if y2 < y1 {
        return Err(VisionOutputError::new(
            "bbox y2/bottom must be >= y1/top.",
            line,
        ));
    }
    let left = clamp01(x1 / width);
    let top = clamp01(y1 / height);
    let right = clamp01(x2 / width);
    let bottom = clamp01(y2 / height);
    if right < left || bottom < top {
        return Err(VisionOutputError::new(
            "bbox must have non-negative normalized width and height.",
            line,
        ));
    }
    Ok(NormalizedBoundingBox {
        x: round_coordinate(left),
        y: round_coordinate(top),
        width: round_coordinate(right - left),
        height: round_coordinate(bottom - top),
    })
}

#[cfg(test)]
mod tests {
    use crate::vision::VISION_TARGETS;

    #[test]
    fn vision_targets_constant() {
        assert_eq!(VISION_TARGETS, &["cam", "chat", "content"]);
    }
}
