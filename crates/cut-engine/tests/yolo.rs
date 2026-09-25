use cut_engine::vision::bytetrack::{ByteTracker, TrackConfig};
use cut_engine::vision::nms::{nms_boxes, NmsBox};
use cut_engine::vision::yolo::{
    decode_yolo_output, invert_letterbox_xyxy, letterbox_info, letterbox_nchw,
    serialize_frame_json, DetectionFrame, FrameSource, SyntheticBackend, TrackOptions, VideoFrame,
    YoloEngine, YoloOutputLayout, YoloRawOutput,
};
use cut_engine::vision::VisionEngineError;

struct VecFrameSource {
    frames: Vec<VideoFrame>,
    index: usize,
}

impl FrameSource for VecFrameSource {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, VisionEngineError> {
        if self.index >= self.frames.len() {
            return Ok(None);
        }
        let frame = self.frames[self.index].clone();
        self.index += 1;
        Ok(Some(frame))
    }
}

#[test]
fn letterbox_invert_roundtrip() {
    let width = 640u32;
    let height = 480u32;
    let imgsz = 640u32;
    let info = letterbox_info(width, height, imgsz);
    let cx = info.pad_x + 100.0;
    let cy = info.pad_y + 80.0;
    let w = 120.0;
    let h = 90.0;
    let (x1, y1, x2, y2) = invert_letterbox_xyxy(cx, cy, w, h, &info, width, height);
    assert!(x1 >= 0.0 && y1 >= 0.0);
    assert!(x2 <= width as f32);
    assert!(y2 <= height as f32);
    assert!(x2 > x1);
    assert!(y2 > y1);

    let rgb = vec![128u8; (width * height * 3) as usize];
    let mut buffer = Vec::new();
    let (nchw, _) = letterbox_nchw(&rgb, width, height, imgsz, &mut buffer, &mut Vec::new());
    assert_eq!(nchw.len(), 3 * imgsz as usize * imgsz as usize);
}

#[test]
fn nms_suppresses_overlap() {
    let boxes = vec![
        NmsBox {
            x1: 10.0,
            y1: 10.0,
            x2: 50.0,
            y2: 50.0,
            confidence: 0.9,
            class_id: 0,
        },
        NmsBox {
            x1: 12.0,
            y1: 12.0,
            x2: 48.0,
            y2: 48.0,
            confidence: 0.8,
            class_id: 0,
        },
        NmsBox {
            x1: 100.0,
            y1: 100.0,
            x2: 140.0,
            y2: 140.0,
            confidence: 0.7,
            class_id: 0,
        },
    ];
    let kept = nms_boxes(boxes, 0.5);
    assert_eq!(kept.len(), 2);
    assert!(kept
        .iter()
        .any(|b| (b.confidence - 0.9).abs() < f32::EPSILON));
    assert!(kept
        .iter()
        .any(|b| (b.confidence - 0.7).abs() < f32::EPSILON));
}

#[test]
fn bytetrack_keeps_id_across_three_frames() {
    let mut tracker = ByteTracker::new(TrackConfig {
        track_high_thresh: 0.25,
        track_low_thresh: 0.1,
        match_thresh: 0.3,
        track_buffer: 30,
    });
    let mut first_id = None;
    for offset in 0..3 {
        let x1 = 50.0 + offset as f32 * 10.0;
        let tracked = tracker.update(&[NmsBox {
            x1,
            y1: 50.0,
            x2: x1 + 40.0,
            y2: 90.0,
            confidence: 0.9,
            class_id: 0,
        }]);
        assert_eq!(tracked.len(), 1);
        if offset == 0 {
            first_id = Some(tracked[0].track_id);
            assert_eq!(first_id, Some(1));
        } else {
            assert_eq!(tracked[0].track_id, first_id.unwrap());
        }
    }
}

#[test]
fn jsonl_schema_matches_ultralytics_contract() {
    let frame = DetectionFrame {
        frame_index: 0,
        time_seconds: Some(0.0),
        width: 1920,
        height: 1080,
        fps: Some(30.0),
        detections: vec![cut_engine::vision::yolo::JsonlDetection {
            track_id: 1,
            class_id: 0,
            class_name: "cam".to_string(),
            confidence: 0.95,
            bbox: [100.0, 50.0, 300.0, 250.0],
        }],
    };
    let json = serialize_frame_json(&frame).unwrap();
    assert!(json.contains("\"detections\""));
    assert!(json.contains("\"frame_index\":0"));
    assert!(json.contains("\"class_name\":\"cam\""));
    assert!(json.contains("\"track_id\":1"));
    assert!(!json.contains(' '));
    let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
    let keys: Vec<_> = parsed.as_object().unwrap().keys().cloned().collect();
    assert_eq!(
        keys,
        vec![
            "detections",
            "fps",
            "frame_index",
            "height",
            "time_seconds",
            "width"
        ]
    );
}

#[test]
fn synthetic_yolo_tensor_decode_rows_first() {
    let info = letterbox_info(640, 480, 640);
    let output = YoloRawOutput {
        layout: YoloOutputLayout::RowsFirst,
        num_predictions: 1,
        num_classes: 2,
        data: vec![
            320.0, 240.0, 80.0, 60.0, 0.1, 0.9, // cx, cy, w, h, cls0, cls1
        ],
    };
    let boxes = decode_yolo_output(&output, 0.25, 640, 640, 480, &info);
    assert_eq!(boxes.len(), 1);
    assert_eq!(boxes[0].class_id, 1);
    assert!(boxes[0].confidence > 0.8);
}

#[test]
fn scaled_frame_boxes_map_back_to_source_pixels() {
    let frame = VideoFrame {
        index: 0,
        width: 640,
        height: 360,
        rgb: vec![0u8; 640 * 360 * 3],
        orig_width: 1920,
        orig_height: 1080,
    };
    let mut source = VecFrameSource {
        frames: vec![frame],
        index: 0,
    };
    let backend_output = YoloRawOutput {
        layout: YoloOutputLayout::RowsFirst,
        num_predictions: 1,
        num_classes: 1,
        data: vec![320.0, 180.0, 64.0, 36.0, 0.99],
    };
    let mut backend = SyntheticBackend::new(vec![backend_output]);
    let mut engine = YoloEngine::from_weights("/tmp/custom.onnx").unwrap();
    let frames = engine
        .track_video_frames(
            &mut source,
            &TrackOptions {
                fps: Some(8.0),
                max_frames: Some(1),
                conf: Some(0.25),
                iou: Some(0.7),
                imgsz: Some(640),
                class_names: Some(vec!["cam".to_string()]),
            },
            Some(&mut backend),
        )
        .unwrap();
    assert_eq!(frames[0].width, 1920);
    assert_eq!(frames[0].height, 1080);
    let bbox = frames[0].detections[0].bbox;
    assert!(bbox[2] > bbox[0]);
    assert!(bbox[0] < 1920.0);
    assert!(bbox[2] <= 1920.0 + 1.0);
}

#[test]
fn yolo_engine_track_detections_emits_jsonl() {
    let engine = YoloEngine::from_weights("/tmp/custom.pt").unwrap();
    let jsonl = engine
        .track_detections(&[DetectionFrame {
            frame_index: 0,
            time_seconds: None,
            width: 100,
            height: 100,
            fps: None,
            detections: vec![],
        }])
        .unwrap();
    assert_eq!(
        jsonl,
        "{\"detections\":[],\"frame_index\":0,\"height\":100,\"width\":100}"
    );
}
