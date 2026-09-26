use cut_engine::vision::planning::{
    build_crop_plan, build_tracking_plan, plan_crop_for_aspect_ratio, CropPlanRequest, CropTarget,
    TrackingPlanInput, TrackingPlanOptions,
};
use cut_engine::vision::{NormalizedBoundingBox, NormalizedDetection, TrackId, VisionTarget};

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
fn keeps_cam_chat_content_segments_separate() {
    let plan = build_tracking_plan(
        TrackingPlanInput::Detections(&[
            detection(
                TrackId::Number(4),
                VisionTarget::Chat,
                0,
                0.0,
                NormalizedBoundingBox {
                    x: 0.55,
                    y: 0.1,
                    width: 0.3,
                    height: 0.3,
                },
                0.9,
            ),
            detection(
                TrackId::Number(7),
                VisionTarget::Content,
                0,
                0.0,
                NormalizedBoundingBox {
                    x: 0.2,
                    y: 0.25,
                    width: 0.45,
                    height: 0.5,
                },
                0.9,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                0,
                0.0,
                NormalizedBoundingBox {
                    x: 0.1,
                    y: 0.2,
                    width: 0.15,
                    height: 0.2,
                },
                0.9,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                1,
                0.1,
                NormalizedBoundingBox {
                    x: 0.11,
                    y: 0.2,
                    width: 0.15,
                    height: 0.2,
                },
                0.9,
            ),
            detection(
                TrackId::Number(4),
                VisionTarget::Chat,
                1,
                0.1,
                NormalizedBoundingBox {
                    x: 0.55,
                    y: 0.1,
                    width: 0.3,
                    height: 0.3,
                },
                0.9,
            ),
            detection(
                TrackId::Number(7),
                VisionTarget::Content,
                1,
                0.1,
                NormalizedBoundingBox {
                    x: 0.2,
                    y: 0.25,
                    width: 0.45,
                    height: 0.5,
                },
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
        NormalizedBoundingBox {
            x: 0.1,
            y: 0.2,
            width: 0.15,
            height: 0.2
        }
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
                NormalizedBoundingBox {
                    x: 0.0,
                    y: 0.0,
                    width: 0.1,
                    height: 0.1,
                },
                0.9,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                1,
                0.1,
                NormalizedBoundingBox {
                    x: 0.1,
                    y: 0.0,
                    width: 0.1,
                    height: 0.1,
                },
                0.1,
            ),
            detection(
                TrackId::Number(1),
                VisionTarget::Cam,
                8,
                0.8,
                NormalizedBoundingBox {
                    x: 0.2,
                    y: 0.0,
                    width: 0.1,
                    height: 0.1,
                },
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
            NormalizedBoundingBox {
                x: 0.1,
                y: 0.2,
                width: 0.2,
                height: 0.2,
            },
            0.9,
        ),
        detection(
            TrackId::Number(1),
            VisionTarget::Cam,
            1,
            0.1,
            NormalizedBoundingBox {
                x: 0.3,
                y: 0.2,
                width: 0.2,
                height: 0.2,
            },
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
        NormalizedBoundingBox {
            x: 0.0,
            y: 0.0,
            width: 1.0,
            height: 1.0
        }
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
