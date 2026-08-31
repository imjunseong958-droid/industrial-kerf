use cut_engine::vision::config::{validate_yolo26_config, Yolo26TrackingConfig};
use cut_engine::vision::planning::{
    build_crop_plan, build_tracking_plan, plan_crop_for_aspect_ratio, CropPlanRequest, CropTarget,
    TrackingPlanInput, TrackingPlanOptions,
};
use cut_engine::vision::{
    parse_and_normalize_tracking_output, NormalizedBoundingBox, NormalizedDetection,
    NormalizeOptions, TrackId, UnknownLabelPolicy, VisionConfigError, VisionOutputError,
    VisionTarget,
};
use std::collections::HashMap;

fn config_label_map() -> HashMap<String, VisionTarget> {
    HashMap::from([
        ("cam".to_string(), VisionTarget::Cam),
        ("chat".to_string(), VisionTarget::Chat),
        ("content".to_string(), VisionTarget::Content),
    ])
}

fn base_config() -> Yolo26TrackingConfig {
    Yolo26TrackingConfig {
        weights: "/tmp/custom-cut-yolo26s.pt".to_string(),
        label_map: config_label_map(),
        targets: None,
        cwd: None,
        tracker: None,
        confidence: None,
        conf: None,
        iou: None,
        fps: None,
        device: None,
        timeout_ms: None,
        unknown_label_policy: None,
    }
}

fn detection(
    track_id: TrackId,
    class_: VisionTarget,
    frame: u64,
    time: f64,
    bbox: NormalizedBoundingBox,
    confidence: f64,
) -> NormalizedDetection {
    NormalizedDetection {
        track_id,
        class_,
        confidence,
        frame,
        time,
        frame_index: frame,
        time_seconds: time,
        bbox,
    }
}

#[test]
fn rejects_stock_checkpoint_and_incomplete_mappings() {
    let err = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "yolo26s.pt".to_string(),
        label_map: config_label_map(),
        ..base_config()
    })
    .unwrap_err();
    assert!(err.message.contains("stock"));
    assert_eq!(err.field.as_deref(), Some("weights"));

    let err = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "yolo26s.onnx".to_string(),
        label_map: config_label_map(),
        ..base_config()
    })
    .unwrap_err();
    assert!(err.message.contains("stock"));

    let err = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "/tmp/custom.pt".to_string(),
        label_map: HashMap::from([("cam".to_string(), VisionTarget::Cam)]),
        ..base_config()
    })
    .unwrap_err();
    assert!(err.message.contains("all three targets (cam, chat, and content)"));

    let err = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "/tmp/custom.pt".to_string(),
        label_map: HashMap::from([
            ("webcam".to_string(), VisionTarget::Cam),
            ("chat".to_string(), VisionTarget::Chat),
            ("content".to_string(), VisionTarget::Content),
        ]),
        ..base_config()
    })
    .unwrap_err();
    assert!(err.message.contains("labelMap key \"webcam\" is not supported"));
}

#[test]
fn accepts_custom_weights_and_the_identity_label_map() {
    let omitted = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "./custom.pt".to_string(),
        label_map: HashMap::new(),
        ..base_config()
    })
    .unwrap();
    assert_eq!(omitted.label_map.get("cam"), Some(&VisionTarget::Cam));
    assert_eq!(omitted.label_map.get("0"), Some(&VisionTarget::Cam));
    assert_eq!(omitted.label_map.get("2"), Some(&VisionTarget::Content));

    let identity = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "./custom.pt".to_string(),
        label_map: config_label_map(),
        ..base_config()
    })
    .unwrap();
    assert_eq!(identity.label_map.get("cam"), Some(&VisionTarget::Cam));
    assert_eq!(identity.label_map.get("chat"), Some(&VisionTarget::Chat));
    assert_eq!(identity.label_map.get("content"), Some(&VisionTarget::Content));

    let config = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: " ./custom.pt ".to_string(),
        label_map: HashMap::from([
            (" Cam ".to_string(), VisionTarget::Cam),
            ("chat".to_string(), VisionTarget::Chat),
            ("content".to_string(), VisionTarget::Content),
        ]),
        ..base_config()
    })
    .unwrap();
    assert_eq!(config.weights, "./custom.pt");
    assert_eq!(config.label_map.get("cam"), Some(&VisionTarget::Cam));
    assert_eq!(config.label_map.get("chat"), Some(&VisionTarget::Chat));
    assert_eq!(config.label_map.get("content"), Some(&VisionTarget::Content));
    assert_eq!(config.tracker, "bytetrack.yaml");
    assert_eq!(config.confidence, 0.25);
    assert_eq!(config.iou, 0.7);
    assert_eq!(config.timeout_ms, 120_000);
    assert_eq!(config.unknown_label_policy, UnknownLabelPolicy::Ignore);
}

#[test]
fn normalizes_deterministic_runner_output() {
    let stdout = serde_json::json!({
        "frame_index": 1,
        "time_seconds": 0.04,
        "width": 1000,
        "height": 500,
        "detections": [
            { "track_id": 11, "class_name": "chat", "confidence": 0.7, "bbox": [500, 100, 900, 400] },
            { "track_id": 2, "class_name": "cam", "confidence": 0.9, "bbox": [100, 50, 300, 250] },
            { "track_id": 99, "class_name": "car", "confidence": 0.99, "bbox": [0, 0, 100, 100] },
        ]
    })
    .to_string();

    let result = parse_and_normalize_tracking_output(
        &stdout,
        &NormalizeOptions {
            label_map: config_label_map(),
            unknown_label_policy: UnknownLabelPolicy::Ignore,
            fps: Some(25.0),
            frame_width: None,
            frame_height: None,
        },
    )
    .unwrap();

    assert_eq!(result.frame_count, 2);
    assert_eq!(result.detections.len(), 2);
    assert_eq!(result.detections[0].track_id, TrackId::Number(2));
    assert_eq!(result.detections[0].class_, VisionTarget::Cam);
    assert_eq!(
        result.detections[0].bbox,
        NormalizedBoundingBox {
            x: 0.1,
            y: 0.1,
            width: 0.2,
            height: 0.4,
        }
    );
    assert_eq!(result.detections[1].class_, VisionTarget::Chat);
}

#[test]
fn fails_on_unknown_labels_when_configured() {
    let stdout = serde_json::json!({
        "frame": 0,
        "detections": [
            { "track_id": "7", "class_name": "cam", "confidence": 0.5, "bbox": { "x": 0.2, "y": 0.3, "width": 0.4, "height": 0.2 }, "normalized": true },
            { "track_id": 8, "class_name": "other", "confidence": 0.4, "bbox": [0, 0, 1, 1], "normalized": true },
        ]
    })
    .to_string();

    let err = parse_and_normalize_tracking_output(
        &stdout,
        &NormalizeOptions {
            label_map: config_label_map(),
            unknown_label_policy: UnknownLabelPolicy::Error,
            fps: None,
            frame_width: None,
            frame_height: None,
        },
    )
    .unwrap_err();
    assert!(matches!(err, VisionOutputError { .. }));
}

#[test]
fn normalizes_xyxy_xywh_and_object_boxes() {
    let stdout = serde_json::json!({
        "frame": 0,
        "width": 1000,
        "height": 500,
        "detections": [
            {
                "track_id": 1,
                "class_name": "cam",
                "confidence": 0.9,
                "bbox": { "x": 100, "y": 50, "width": 200, "height": 100 },
                "bbox_format": "xywh"
            },
            {
                "track_id": 2,
                "class_name": "cam",
                "confidence": 0.9,
                "bbox": { "x": 0.2, "y": 0.3, "width": 0.4, "height": 0.2 },
                "bbox_format": "xywh",
                "normalized": true
            },
            {
                "track_id": 3,
                "class_name": "cam",
                "confidence": 0.9,
                "bbox": { "x1": 100, "y1": 50, "x2": 300, "y2": 250 },
                "bbox_format": "xyxy"
            },
        ]
    })
    .to_string();

    let result = parse_and_normalize_tracking_output(
        &stdout,
        &NormalizeOptions {
            label_map: config_label_map(),
            unknown_label_policy: UnknownLabelPolicy::Ignore,
            fps: None,
            frame_width: None,
            frame_height: None,
        },
    )
    .unwrap();

    let boxes: Vec<_> = result.detections.iter().map(|item| item.bbox.clone()).collect();
    assert_eq!(
        boxes,
        vec![
            NormalizedBoundingBox { x: 0.1, y: 0.1, width: 0.2, height: 0.2 },
            NormalizedBoundingBox { x: 0.2, y: 0.3, width: 0.4, height: 0.2 },
            NormalizedBoundingBox { x: 0.1, y: 0.1, width: 0.2, height: 0.4 },
        ]
    );
}

#[test]
fn keeps_cam_chat_content_segments_separate() {
    let plan = build_tracking_plan(
        TrackingPlanInput::Detections(&[
            detection(
                TrackId::Number(4),
                VisionTarget::Chat,
                0,
                0.0,
                NormalizedBoundingBox { x: 0.55, y: 0.1, width: 0.3, height: 0.3 },
                0.9,
            ),
            detection(
                TrackId::Number(7),
                VisionTarget::Content,
                0,
                0.0,
                NormalizedBoundingBox { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
                0.9,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                0,
                0.0,
                NormalizedBoundingBox { x: 0.1, y: 0.2, width: 0.15, height: 0.2 },
                0.9,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                1,
                0.1,
                NormalizedBoundingBox { x: 0.11, y: 0.2, width: 0.15, height: 0.2 },
                0.9,
            ),
            detection(
                TrackId::Number(4),
                VisionTarget::Chat,
                1,
                0.1,
                NormalizedBoundingBox { x: 0.55, y: 0.1, width: 0.3, height: 0.3 },
                0.9,
            ),
            detection(
                TrackId::Number(7),
                VisionTarget::Content,
                1,
                0.1,
                NormalizedBoundingBox { x: 0.2, y: 0.25, width: 0.45, height: 0.5 },
                0.9,
            ),
        ]),
        &TrackingPlanOptions::default(),
    )
    .unwrap();

    assert_eq!(plan.by_class.cam.len(), 1);
    assert_eq!(plan.by_class.chat.len(), 1);
    assert_eq!(plan.by_class.content.len(), 1);
    assert!(plan
        .keyframes_by_class
        .cam
        .iter()
        .all(|keyframe| keyframe.class_ == VisionTarget::Cam));
    assert!(plan
        .keyframes_by_class
        .content
        .iter()
        .all(|keyframe| keyframe.class_ == VisionTarget::Content));
    assert_eq!(
        plan.by_class.cam[0].keyframes[0].bbox,
        NormalizedBoundingBox { x: 0.1, y: 0.2, width: 0.15, height: 0.2 }
    );
}

#[test]
fn filters_low_confidence_and_splits_after_gap() {
    let plan = build_tracking_plan(
        TrackingPlanInput::Detections(&[
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                0,
                0.0,
                NormalizedBoundingBox { x: 0.0, y: 0.0, width: 0.1, height: 0.1 },
                0.9,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                1,
                0.1,
                NormalizedBoundingBox { x: 0.1, y: 0.0, width: 0.1, height: 0.1 },
                0.1,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                8,
                0.8,
                NormalizedBoundingBox { x: 0.2, y: 0.0, width: 0.1, height: 0.1 },
                0.9,
            ),
        ]),
        &TrackingPlanOptions {
            min_confidence: Some(0.5),
            max_gap_seconds: Some(0.2),
            ..TrackingPlanOptions::default()
        },
    )
    .unwrap();

    assert_eq!(plan.by_class.cam.len(), 2);
    assert_eq!(
        plan.by_class
            .cam
            .iter()
            .map(|segment| segment.keyframes.len())
            .collect::<Vec<_>>(),
        vec![1, 1]
    );
}

#[test]
fn smooths_bbox_jitter_deterministically() {
    let input = [
        detection(
            TrackId::Number(1),
            VisionTarget::Cam,
            0,
            0.0,
            NormalizedBoundingBox { x: 0.1, y: 0.2, width: 0.2, height: 0.2 },
            0.9,
        ),
        detection(
            TrackId::Number(1),
            VisionTarget::Cam,
            1,
            0.1,
            NormalizedBoundingBox { x: 0.3, y: 0.2, width: 0.2, height: 0.2 },
            0.9,
        ),
    ];
    let first = build_tracking_plan(
        TrackingPlanInput::Detections(&input),
        &TrackingPlanOptions {
            smoothing_factor: Some(0.5),
            ..TrackingPlanOptions::default()
        },
    )
    .unwrap();
    let second = build_tracking_plan(
        TrackingPlanInput::Detections(&input),
        &TrackingPlanOptions {
            smoothing_factor: Some(0.5),
            ..TrackingPlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(first.keyframes[1].bbox.x, 0.2);
    assert_eq!(first.segments, second.segments);
    let sharp = build_tracking_plan(
        TrackingPlanInput::Detections(&input),
        &TrackingPlanOptions {
            smoothing_factor: Some(1.0),
            ..TrackingPlanOptions::default()
        },
    )
    .unwrap();
    assert_eq!(sharp.keyframes[1].bbox.x, 0.3);
}

#[test]
fn crop_plan_includes_selected_regions() {
    let result = plan_crop_for_aspect_ratio(&CropPlanRequest {
        aspect_ratio: 16.0 / 9.0,
        source_aspect_ratio: Some(1.0),
        source_width: None,
        source_height: None,
        cam: Some(CropTarget::Box(NormalizedBoundingBox {
            x: 0.1,
            y: 0.35,
            width: 0.15,
            height: 0.2,
        })),
        chat: Some(CropTarget::Box(NormalizedBoundingBox {
            x: 0.55,
            y: 0.25,
            width: 0.3,
            height: 0.35,
        })),
        content: None,
        padding: Some(0.02),
    })
    .unwrap();

    assert_eq!(result.mode.as_str(), "cam_chat");
    assert!(result.included.cam);
    assert!(result.included.chat);
    assert!(!result.included.content);
    assert!(result.crop.x >= 0.0);
    assert!(result.crop.y >= 0.0);
    assert!(result.crop.x + result.crop.width <= 1.0);
    assert!(result.crop.y + result.crop.height <= 1.0);
    let ratio = (result.crop.width * result.source_aspect_ratio) / result.crop.height;
    assert!((ratio - 16.0 / 9.0).abs() < 1e-10);
}

#[test]
fn crop_plan_content_only() {
    let result = plan_crop_for_aspect_ratio(&CropPlanRequest {
        aspect_ratio: 16.0 / 9.0,
        source_aspect_ratio: Some(1.0),
        source_width: None,
        source_height: None,
        cam: None,
        chat: None,
        content: Some(CropTarget::Box(NormalizedBoundingBox {
            x: 0.2,
            y: 0.25,
            width: 0.45,
            height: 0.5,
        })),
        padding: Some(0.02),
    })
    .unwrap();

    assert_eq!(result.mode.as_str(), "content");
    assert!(!result.included.cam);
    assert!(!result.included.chat);
    assert!(result.included.content);
    assert!(result.crop.x >= 0.0);
    assert!(result.crop.y >= 0.0);
    assert!(result.crop.x + result.crop.width <= 1.0);
    assert!(result.crop.y + result.crop.height <= 1.0);
}

#[test]
fn crop_plan_all_three_regions() {
    let result = plan_crop_for_aspect_ratio(&CropPlanRequest {
        aspect_ratio: 16.0 / 9.0,
        source_aspect_ratio: Some(1.0),
        source_width: None,
        source_height: None,
        cam: Some(CropTarget::Box(NormalizedBoundingBox {
            x: 0.1,
            y: 0.35,
            width: 0.15,
            height: 0.2,
        })),
        chat: Some(CropTarget::Box(NormalizedBoundingBox {
            x: 0.55,
            y: 0.25,
            width: 0.3,
            height: 0.35,
        })),
        content: Some(CropTarget::Box(NormalizedBoundingBox {
            x: 0.2,
            y: 0.25,
            width: 0.45,
            height: 0.5,
        })),
        padding: Some(0.02),
    })
    .unwrap();

    assert_eq!(result.mode.as_str(), "all");
    assert!(result.included.cam);
    assert!(result.included.chat);
    assert!(result.included.content);
    assert!(result.crop.x >= 0.0);
    assert!(result.crop.y >= 0.0);
    assert!(result.crop.x + result.crop.width <= 1.0);
    assert!(result.crop.y + result.crop.height <= 1.0);
    let ratio = (result.crop.width * result.source_aspect_ratio) / result.crop.height;
    assert!((ratio - 16.0 / 9.0).abs() < 1e-10);
}

#[test]
fn crop_plan_center_fallback() {
    let result = build_crop_plan(&CropPlanRequest {
        aspect_ratio: 1.0,
        source_aspect_ratio: None,
        source_width: None,
        source_height: None,
        cam: None,
        chat: None,
        content: None,
        padding: None,
    })
    .unwrap();
    assert_eq!(result.mode.as_str(), "center");
    assert!(!result.included.cam);
    assert_eq!(
        result.crop,
        NormalizedBoundingBox { x: 0.0, y: 0.0, width: 1.0, height: 1.0 }
    );
}

#[test]
fn crop_plan_16_9_source_to_9_16() {
    let result = plan_crop_for_aspect_ratio(&CropPlanRequest {
        aspect_ratio: 9.0 / 16.0,
        source_aspect_ratio: Some(16.0 / 9.0),
        source_width: None,
        source_height: None,
        cam: None,
        chat: None,
        content: None,
        padding: None,
    })
    .unwrap();
    assert_eq!(result.crop.height, 1.0);
    assert!((result.crop.width - 0.31640625).abs() < 1e-10);
    let ratio = (result.crop.width * result.source_aspect_ratio) / result.crop.height;
    assert!((ratio - 9.0 / 16.0).abs() < 1e-10);
}

#[test]
fn rejects_remote_weights() {
    let err = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "https://example.com/model.pt".to_string(),
        label_map: config_label_map(),
        ..base_config()
    })
    .unwrap_err();
    assert!(err.message.contains("remote URLs"));
}

#[test]
fn config_error_is_typed() {
    let err = validate_yolo26_config(&Yolo26TrackingConfig {
        weights: "yolo26s.pt".to_string(),
        label_map: config_label_map(),
        ..base_config()
    })
    .unwrap_err();
    let _: VisionConfigError = err;
}
