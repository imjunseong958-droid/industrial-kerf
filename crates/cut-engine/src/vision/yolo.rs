//! Native YOLO inference + ByteTrack pipeline with JSONL output.

use super::bytetrack::{ByteTracker, TrackConfig, TrackedDetection};
use super::nms::{nms_boxes, NmsBox};
use super::VisionEngineError;
use crate::ffmpeg::{resolve_ffmpeg_bin, tracking_decode_prefix};
use crate::probe::info;
use crate::resources::tracking_sample_fps;
use serde::Serialize;
use std::io::{BufReader, Read};
use std::path::Path;
use std::process::{Child, ChildStdout, Command, Stdio};

pub const DEFAULT_IMGSZ: u32 = 640;
pub const DEFAULT_STRIDE: u32 = 32;

#[derive(Debug, Clone, PartialEq)]
pub struct YoloRawOutput {
    pub data: Vec<f32>,
    pub layout: YoloOutputLayout,
    pub num_predictions: usize,
    pub num_classes: usize,
}

impl Default for YoloRawOutput {
    fn default() -> Self {
        Self {
            data: Vec::new(),
            layout: YoloOutputLayout::RowsFirst,
            num_predictions: 0,
            num_classes: 0,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum YoloOutputLayout {
    /// `[1, 4 + nc, n]`. Channels-first (transposed).
    ChannelsFirst,
    /// `[1, n, 4 + nc]`. One row per prediction.
    RowsFirst,
}

#[derive(Debug, Clone, PartialEq)]
pub struct LetterboxInfo {
    pub scale: f32,
    pub pad_x: f32,
    pub pad_y: f32,
    pub new_w: u32,
    pub new_h: u32,
    pub imgsz: u32,
}

#[derive(Debug, Clone)]
pub struct VideoFrame {
    pub index: usize,
    pub width: u32,
    pub height: u32,
    pub rgb: Vec<u8>,
    /// Original source dimensions; `0` means the decoded `width`/`height`.
    pub orig_width: u32,
    pub orig_height: u32,
}

impl VideoFrame {
    fn source_width(&self) -> u32 {
        if self.orig_width == 0 {
            self.width
        } else {
            self.orig_width
        }
    }

    fn source_height(&self) -> u32 {
        if self.orig_height == 0 {
            self.height
        } else {
            self.orig_height
        }
    }
}

pub trait FrameSource {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, VisionEngineError>;

    /// Fill `dest` with the next frame, reusing `dest.rgb` capacity when possible.
    fn next_frame_into(&mut self, dest: &mut VideoFrame) -> Result<bool, VisionEngineError> {
        match self.next_frame()? {
            None => Ok(false),
            Some(frame) => {
                *dest = frame;
                Ok(true)
            }
        }
    }
}

pub trait InferenceBackend {
    fn infer(
        &mut self,
        nchw: &[f32],
        n: usize,
        c: usize,
        h: usize,
        w: usize,
        dest: &mut YoloRawOutput,
    ) -> Result<(), VisionEngineError>;
}

#[cfg(feature = "onnx")]
pub use super::onnx::OnnxRuntimeBackend as OnnxBackend;

#[cfg(not(feature = "onnx"))]
pub struct OnnxBackend;

#[cfg(not(feature = "onnx"))]
impl InferenceBackend for OnnxBackend {
    fn infer(
        &mut self,
        _nchw: &[f32],
        _n: usize,
        _c: usize,
        _h: usize,
        _w: usize,
        _dest: &mut YoloRawOutput,
    ) -> Result<(), VisionEngineError> {
        Err(VisionEngineError::new("ONNX runtime not enabled"))
    }
}

/// Deterministic backend for unit tests.
pub struct SyntheticBackend {
    outputs: Vec<YoloRawOutput>,
    index: usize,
}

impl SyntheticBackend {
    pub fn new(outputs: Vec<YoloRawOutput>) -> Self {
        Self { outputs, index: 0 }
    }
}

impl InferenceBackend for SyntheticBackend {
    fn infer(
        &mut self,
        _nchw: &[f32],
        _n: usize,
        _c: usize,
        _h: usize,
        _w: usize,
        dest: &mut YoloRawOutput,
    ) -> Result<(), VisionEngineError> {
        if self.index >= self.outputs.len() {
            dest.data.clear();
            dest.layout = YoloOutputLayout::RowsFirst;
            dest.num_predictions = 0;
            dest.num_classes = 0;
            return Ok(());
        }
        let output = &self.outputs[self.index];
        dest.data.clear();
        dest.data.extend_from_slice(&output.data);
        dest.layout = output.layout;
        dest.num_predictions = output.num_predictions;
        dest.num_classes = output.num_classes;
        self.index += 1;
        Ok(())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TrackOptions {
    pub fps: Option<f64>,
    pub max_frames: Option<usize>,
    pub conf: Option<f32>,
    pub iou: Option<f32>,
    pub imgsz: Option<u32>,
    pub class_names: Option<Vec<String>>,
}

/// Decode options for the native tracker: sample fps + downscale before RGB.
#[derive(Debug, Clone)]
pub struct TrackDecodeOptions {
    pub max_frames: Option<usize>,
    pub fps: Option<f64>,
    pub imgsz: u32,
}

impl Default for TrackDecodeOptions {
    fn default() -> Self {
        Self {
            max_frames: None,
            fps: None,
            imgsz: DEFAULT_IMGSZ,
        }
    }
}

/// ffmpeg `-vf` that drops frames early and fits the tensor before RGB conversion.
pub fn tracking_decode_filter(width: u32, height: u32, fps: f64) -> String {
    let fps = if fps.is_finite() && fps > 0.0 {
        fps
    } else {
        crate::resources::DEFAULT_TRACKING_FPS
    };
    let fps_arg = if (fps - fps.round()).abs() < 1e-9 {
        format!("{}", fps.round() as i64)
    } else {
        format!("{fps}")
    };
    format!("fps={fps_arg},scale={}:{}", width.max(1), height.max(1))
}

/// Decode size that never upscales and fits inside `imgsz` on the long edge.
pub fn scaled_decode_size(orig_w: u32, orig_h: u32, imgsz: u32) -> (u32, u32) {
    if orig_w == 0 || orig_h == 0 {
        return (imgsz.max(1), imgsz.max(1));
    }
    if orig_w <= imgsz && orig_h <= imgsz {
        (orig_w, orig_h)
    } else {
        let info = letterbox_info(orig_w, orig_h, imgsz);
        (info.new_w.max(1), info.new_h.max(1))
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct JsonlDetection {
    pub track_id: u64,
    pub class_id: usize,
    pub class_name: String,
    pub confidence: f32,
    pub bbox: [f32; 4],
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DetectionFrame {
    pub frame_index: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_seconds: Option<f64>,
    pub width: u32,
    pub height: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fps: Option<f64>,
    pub detections: Vec<JsonlDetection>,
}

#[derive(Default)]
struct InferenceScratch {
    letterbox_buffer: Vec<f32>,
    src_xs: Vec<usize>,
    output: YoloRawOutput,
    letterbox_layout: (u32, u32, u32),
}

pub struct YoloEngine {
    weights_path: String,
    imgsz: u32,
    #[allow(dead_code)]
    stride: u32,
    conf: f32,
    iou: f32,
    backend: Option<Box<dyn InferenceBackend>>,
    scratch: InferenceScratch,
}

impl YoloEngine {
    pub fn from_weights(path: impl AsRef<Path>) -> Result<Self, VisionEngineError> {
        let path_ref = path.as_ref();
        let weights_path = path_ref.display().to_string();
        #[cfg_attr(not(feature = "onnx"), allow(unused_mut))]
        let mut engine = Self {
            weights_path,
            imgsz: DEFAULT_IMGSZ,
            stride: DEFAULT_STRIDE,
            conf: 0.25,
            iou: 0.7,
            backend: None,
            scratch: InferenceScratch::default(),
        };
        #[cfg(feature = "onnx")]
        {
            let candidate = if path_ref.extension().and_then(|ext| ext.to_str()) == Some("pt") {
                path_ref.with_extension("onnx")
            } else {
                path_ref.to_path_buf()
            };
            if candidate.extension().and_then(|ext| ext.to_str()) == Some("onnx")
                && candidate.is_file()
            {
                engine.backend = Some(Box::new(OnnxBackend::from_file(&candidate)?));
            }
        }
        Ok(engine)
    }

    pub fn with_backend(mut self, backend: Box<dyn InferenceBackend>) -> Self {
        self.backend = Some(backend);
        self
    }

    pub fn set_confidence(&mut self, conf: f32) {
        self.conf = conf;
    }

    pub fn set_iou(&mut self, iou: f32) {
        self.iou = iou;
    }

    pub fn weights_path(&self) -> &str {
        &self.weights_path
    }

    /// Run tracking on precomputed per-frame detections and emit JSONL lines.
    pub fn track_detections(&self, frames: &[DetectionFrame]) -> Result<String, VisionEngineError> {
        let mut out = String::new();
        for (index, frame) in frames.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            serialize_frame_json_into(frame, &mut out)?;
        }
        Ok(out)
    }

    /// Process frames from an injected source, optionally running inference.
    pub fn track_video(
        &mut self,
        source: &mut dyn FrameSource,
        options: &TrackOptions,
        backend: Option<&mut dyn InferenceBackend>,
    ) -> Result<String, VisionEngineError> {
        let frames = self.track_video_frames(source, options, backend)?;
        let mut out = String::new();
        for (index, frame) in frames.iter().enumerate() {
            if index > 0 {
                out.push('\n');
            }
            serialize_frame_json_into(frame, &mut out)?;
        }
        Ok(out)
    }

    /// Same pipeline as `track_video`, but keep structured frames (no JSONL).
    pub fn track_video_frames(
        &mut self,
        source: &mut dyn FrameSource,
        options: &TrackOptions,
        mut backend: Option<&mut dyn InferenceBackend>,
    ) -> Result<Vec<DetectionFrame>, VisionEngineError> {
        let conf = options.conf.unwrap_or(self.conf);
        let iou = options.iou.unwrap_or(self.iou);
        let imgsz = options.imgsz.unwrap_or(self.imgsz);
        let class_names = options.class_names.as_deref().unwrap_or(&[]);
        if backend.is_none() && self.weights_path.ends_with(".onnx") && self.backend.is_none() {
            return Err(VisionEngineError::new(
                "ONNX weights require a loaded backend; rebuild cut-engine with --features onnx",
            ));
        }
        let mut tracker = ByteTracker::new(TrackConfig::default());
        let mut frames = Vec::new();
        let mut frame_count = 0usize;
        let mut frame = VideoFrame {
            index: 0,
            width: 0,
            height: 0,
            rgb: Vec::new(),
            orig_width: 0,
            orig_height: 0,
        };

        while source.next_frame_into(&mut frame)? {
            if let Some(max_frames) = options.max_frames {
                if frame_count >= max_frames {
                    break;
                }
            }
            let tracked = if let Some(backend) = backend.as_deref_mut() {
                run_frame_inference(
                    &mut self.scratch,
                    backend,
                    &frame,
                    conf,
                    iou,
                    imgsz,
                    &mut tracker,
                )?
            } else if let Some(backend) = self.backend.as_mut() {
                run_frame_inference(
                    &mut self.scratch,
                    backend.as_mut(),
                    &frame,
                    conf,
                    iou,
                    imgsz,
                    &mut tracker,
                )?
            } else {
                Vec::new()
            };

            frames.push(build_detection_frame(
                frame.index,
                frame.source_width(),
                frame.source_height(),
                options.fps,
                &tracked,
                class_names,
            ));
            frame_count += 1;
        }
        Ok(frames)
    }
}

fn run_frame_inference(
    scratch: &mut InferenceScratch,
    backend: &mut dyn InferenceBackend,
    frame: &VideoFrame,
    conf: f32,
    iou: f32,
    imgsz: u32,
    tracker: &mut ByteTracker,
) -> Result<Vec<TrackedDetection>, VisionEngineError> {
    let layout = (frame.width, frame.height, imgsz);
    let reset_pad = scratch.letterbox_layout != layout;
    scratch.letterbox_layout = layout;
    let (nchw, info) = letterbox_nchw_into(
        &frame.rgb,
        frame.width,
        frame.height,
        imgsz,
        &mut scratch.letterbox_buffer,
        &mut scratch.src_xs,
        reset_pad,
    );
    backend.infer(
        nchw,
        1,
        3,
        imgsz as usize,
        imgsz as usize,
        &mut scratch.output,
    )?;
    let mut boxes = decode_yolo_output(
        &scratch.output,
        conf,
        imgsz,
        frame.width,
        frame.height,
        &info,
    );
    let orig_w = frame.source_width();
    let orig_h = frame.source_height();
    if orig_w != frame.width || orig_h != frame.height {
        let scale_x = orig_w as f32 / frame.width.max(1) as f32;
        let scale_y = orig_h as f32 / frame.height.max(1) as f32;
        for boxed in &mut boxes {
            boxed.x1 *= scale_x;
            boxed.x2 *= scale_x;
            boxed.y1 *= scale_y;
            boxed.y2 *= scale_y;
        }
    }
    let nmsed = nms_boxes(boxes, iou);
    Ok(tracker.update(&nmsed))
}

pub fn letterbox_info(orig_w: u32, orig_h: u32, imgsz: u32) -> LetterboxInfo {
    let scale = (imgsz as f32 / orig_w as f32).min(imgsz as f32 / orig_h as f32);
    let new_w = (orig_w as f32 * scale).round() as u32;
    let new_h = (orig_h as f32 * scale).round() as u32;
    let pad_x = (imgsz as f32 - new_w as f32) / 2.0;
    let pad_y = (imgsz as f32 - new_h as f32) / 2.0;
    LetterboxInfo {
        scale,
        pad_x,
        pad_y,
        new_w,
        new_h,
        imgsz,
    }
}

const INV_255: f32 = 1.0 / 255.0;
const LETTERBOX_PAD: f32 = 114.0 / 255.0;

pub fn letterbox_nchw<'a>(
    rgb: &[u8],
    width: u32,
    height: u32,
    imgsz: u32,
    buffer: &'a mut Vec<f32>,
    src_xs: &mut Vec<usize>,
) -> (&'a [f32], LetterboxInfo) {
    letterbox_nchw_into(rgb, width, height, imgsz, buffer, src_xs, true)
}

pub fn letterbox_nchw_into<'a>(
    rgb: &[u8],
    width: u32,
    height: u32,
    imgsz: u32,
    buffer: &'a mut Vec<f32>,
    src_xs: &mut Vec<usize>,
    reset_pad: bool,
) -> (&'a [f32], LetterboxInfo) {
    let info = letterbox_info(width, height, imgsz);
    let plane = (imgsz as usize).saturating_mul(imgsz as usize);
    let needed = 3 * plane;
    if buffer.len() != needed {
        buffer.clear();
        buffer.resize(needed, LETTERBOX_PAD);
    } else if reset_pad {
        buffer.fill(LETTERBOX_PAD);
    }

    if width == imgsz && height == imgsz {
        let pixels = (width as usize)
            .saturating_mul(height as usize)
            .min(rgb.len() / 3);
        for i in 0..pixels {
            let src = i * 3;
            buffer[i] = rgb[src] as f32 * INV_255;
            buffer[plane + i] = rgb[src + 1] as f32 * INV_255;
            buffer[2 * plane + i] = rgb[src + 2] as f32 * INV_255;
        }
        return (buffer.as_slice(), info);
    }

    let inv_scale = if info.scale > 0.0 {
        1.0 / info.scale
    } else {
        0.0
    };
    let pad_x = info.pad_x.round() as usize;
    let pad_y = info.pad_y.round() as usize;
    let max_x = width.saturating_sub(1);
    let max_y = height.saturating_sub(1);
    let stride = width as usize;
    let imgsz_usize = imgsz as usize;
    let needed_xs = info.new_w as usize;
    if src_xs.len() != needed_xs {
        src_xs.resize(needed_xs, 0);
    }
    for (x, slot) in src_xs.iter_mut().enumerate() {
        *slot = (((x as f32) * inv_scale).floor() as u32).min(max_x) as usize;
    }
    let expected_rgb = stride.saturating_mul(height as usize).saturating_mul(3);
    let rgb_ok = expected_rgb > 0 && rgb.len() >= expected_rgb;
    for y in 0..info.new_h {
        let src_y = (((y as f32) * inv_scale).floor() as u32).min(max_y) as usize;
        let src_row = src_y * stride;
        let dst_row = (y as usize + pad_y) * imgsz_usize;
        for (x, &src_x) in src_xs.iter().enumerate() {
            let src_index = (src_row + src_x) * 3;
            let dst_index = dst_row + x + pad_x;
            if dst_index >= plane {
                continue;
            }
            if rgb_ok || src_index + 2 < rgb.len() {
                buffer[dst_index] = rgb[src_index] as f32 * INV_255;
                buffer[plane + dst_index] = rgb[src_index + 1] as f32 * INV_255;
                buffer[2 * plane + dst_index] = rgb[src_index + 2] as f32 * INV_255;
            }
        }
    }
    (buffer.as_slice(), info)
}

pub fn invert_letterbox_xyxy(
    cx: f32,
    cy: f32,
    w: f32,
    h: f32,
    info: &LetterboxInfo,
    orig_w: u32,
    orig_h: u32,
) -> (f32, f32, f32, f32) {
    let cx = (cx - info.pad_x) / info.scale;
    let cy = (cy - info.pad_y) / info.scale;
    let w = w / info.scale;
    let h = h / info.scale;
    let x1 = (cx - w / 2.0).clamp(0.0, orig_w as f32);
    let y1 = (cy - h / 2.0).clamp(0.0, orig_h as f32);
    let x2 = (cx + w / 2.0).clamp(0.0, orig_w as f32);
    let y2 = (cy + h / 2.0).clamp(0.0, orig_h as f32);
    (x1, y1, x2, y2)
}

pub fn decode_yolo_output(
    output: &YoloRawOutput,
    conf_threshold: f32,
    imgsz: u32,
    orig_w: u32,
    orig_h: u32,
    info: &LetterboxInfo,
) -> Vec<NmsBox> {
    if output.num_predictions == 0 || output.num_classes == 0 {
        return Vec::new();
    }
    let channels = 4 + output.num_classes;
    let mut boxes = Vec::with_capacity(output.num_predictions);
    match output.layout {
        YoloOutputLayout::ChannelsFirst => {
            for index in 0..output.num_predictions {
                let cx = output.data[index];
                let cy = output.data[output.num_predictions + index];
                let w = output.data[2 * output.num_predictions + index];
                let h = output.data[3 * output.num_predictions + index];
                let (class_id, score) = best_class_channels_first(
                    &output.data,
                    output.num_predictions,
                    output.num_classes,
                    index,
                );
                if score < conf_threshold {
                    continue;
                }
                let (x1, y1, x2, y2) = invert_letterbox_xyxy(cx, cy, w, h, info, orig_w, orig_h);
                boxes.push(NmsBox {
                    x1,
                    y1,
                    x2,
                    y2,
                    confidence: score,
                    class_id,
                });
            }
        }
        YoloOutputLayout::RowsFirst => {
            for index in 0..output.num_predictions {
                let base = index * channels;
                if base + 3 >= output.data.len() {
                    break;
                }
                let cx = output.data[base];
                let cy = output.data[base + 1];
                let w = output.data[base + 2];
                let h = output.data[base + 3];
                let (class_id, score) =
                    best_class_rows_first(&output.data[base + 4..base + channels]);
                if score < conf_threshold {
                    continue;
                }
                let (x1, y1, x2, y2) = invert_letterbox_xyxy(cx, cy, w, h, info, orig_w, orig_h);
                boxes.push(NmsBox {
                    x1,
                    y1,
                    x2,
                    y2,
                    confidence: score,
                    class_id,
                });
            }
        }
    }
    let _ = imgsz;
    boxes
}

fn best_class_channels_first(data: &[f32], n: usize, nc: usize, index: usize) -> (usize, f32) {
    let mut best_id = 0usize;
    let mut best_score = f32::NEG_INFINITY;
    for class_id in 0..nc {
        let score = data[(4 + class_id) * n + index];
        if score > best_score {
            best_score = score;
            best_id = class_id;
        }
    }
    (best_id, best_score)
}

fn best_class_rows_first(scores: &[f32]) -> (usize, f32) {
    let mut best_id = 0usize;
    let mut best_score = f32::NEG_INFINITY;
    for (class_id, score) in scores.iter().enumerate() {
        if *score > best_score {
            best_score = *score;
            best_id = class_id;
        }
    }
    (best_id, best_score)
}

fn build_detection_frame(
    frame_index: usize,
    width: u32,
    height: u32,
    fps: Option<f64>,
    tracked: &[TrackedDetection],
    class_names: &[String],
) -> DetectionFrame {
    let detections = tracked
        .iter()
        .map(|det| JsonlDetection {
            track_id: det.track_id,
            class_id: det.class_id,
            class_name: class_names
                .get(det.class_id)
                .cloned()
                .unwrap_or_else(|| det.class_id.to_string()),
            confidence: det.confidence,
            bbox: [det.x1, det.y1, det.x2, det.y2],
        })
        .collect();
    DetectionFrame {
        frame_index,
        time_seconds: fps.map(|fps| frame_index as f64 / fps),
        width,
        height,
        fps,
        detections,
    }
}

/// Decode RGB24 frames from ffmpeg for the native tracker.
pub struct FfmpegRgbFrameSource {
    child: Child,
    stdout: BufReader<ChildStdout>,
    width: u32,
    height: u32,
    orig_width: u32,
    orig_height: u32,
    index: usize,
}

impl FfmpegRgbFrameSource {
    pub fn open(source: &str, max_frames: Option<usize>) -> Result<Self, VisionEngineError> {
        Self::open_with(
            source,
            TrackDecodeOptions {
                max_frames,
                ..TrackDecodeOptions::default()
            },
        )
    }

    pub fn open_with(source: &str, options: TrackDecodeOptions) -> Result<Self, VisionEngineError> {
        let probed = info(source).map_err(|error| VisionEngineError::new(error.to_string()))?;
        let orig_width = probed.width.unwrap_or(0);
        let orig_height = probed.height.unwrap_or(0);
        if orig_width == 0 || orig_height == 0 {
            return Err(VisionEngineError::new(
                "source has no video stream dimensions",
            ));
        }
        let imgsz = options.imgsz.max(32);
        let fps = tracking_sample_fps(options.fps);
        let (width, height) = scaled_decode_size(orig_width, orig_height, imgsz);
        let vf = tracking_decode_filter(width, height, fps);
        let ffmpeg = resolve_ffmpeg_bin();
        let mut args = tracking_decode_prefix();
        args.extend([
            "-hide_banner".into(),
            "-loglevel".into(),
            "error".into(),
            "-i".into(),
            source.to_string(),
            "-an".into(),
            "-vf".into(),
            vf,
            "-f".into(),
            "rawvideo".into(),
            "-pix_fmt".into(),
            "rgb24".into(),
        ]);
        if let Some(limit) = options.max_frames {
            args.push("-frames:v".into());
            args.push(limit.to_string());
        }
        args.push("pipe:1".into());
        let mut child = Command::new(ffmpeg)
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::inherit())
            .spawn()
            .map_err(|error| VisionEngineError::new(error.to_string()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| VisionEngineError::new("ffmpeg stdout missing"))?;
        Ok(Self {
            child,
            stdout: BufReader::with_capacity(1 << 20, stdout),
            width,
            height,
            orig_width,
            orig_height,
            index: 0,
        })
    }
}

impl Drop for FfmpegRgbFrameSource {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl FrameSource for FfmpegRgbFrameSource {
    fn next_frame(&mut self) -> Result<Option<VideoFrame>, VisionEngineError> {
        let mut frame = VideoFrame {
            index: 0,
            width: self.width,
            height: self.height,
            rgb: Vec::new(),
            orig_width: self.orig_width,
            orig_height: self.orig_height,
        };
        if self.next_frame_into(&mut frame)? {
            Ok(Some(frame))
        } else {
            Ok(None)
        }
    }

    fn next_frame_into(&mut self, dest: &mut VideoFrame) -> Result<bool, VisionEngineError> {
        let frame_size = (self.width as usize)
            .saturating_mul(self.height as usize)
            .saturating_mul(3);
        dest.width = self.width;
        dest.height = self.height;
        dest.orig_width = self.orig_width;
        dest.orig_height = self.orig_height;
        dest.index = self.index;
        if dest.rgb.len() != frame_size {
            dest.rgb.clear();
            dest.rgb.resize(frame_size, 0);
        }
        match self.stdout.read_exact(&mut dest.rgb) {
            Ok(()) => {
                self.index += 1;
                Ok(true)
            }
            Err(error) if error.kind() == std::io::ErrorKind::UnexpectedEof => Ok(false),
            Err(error) => Err(VisionEngineError::new(error.to_string())),
        }
    }
}

/// Compact JSON with sorted keys matching `ultralytics_track.py`.
pub fn serialize_frame_json(frame: &DetectionFrame) -> Result<String, VisionEngineError> {
    let mut out = String::with_capacity(128 + frame.detections.len() * 160);
    serialize_frame_json_into(frame, &mut out)?;
    Ok(out)
}

fn push_json_f64(out: &mut String, value: f64) {
    match serde_json::Number::from_f64(value) {
        Some(number) => out.push_str(&number.to_string()),
        None => out.push_str("null"),
    }
}

fn serialize_frame_json_into(
    frame: &DetectionFrame,
    out: &mut String,
) -> Result<(), VisionEngineError> {
    out.push('{');
    out.push_str("\"detections\":[");
    for (index, det) in frame.detections.iter().enumerate() {
        if index > 0 {
            out.push(',');
        }
        out.push_str("{\"bbox\":[");
        push_json_f64(out, det.bbox[0] as f64);
        out.push(',');
        push_json_f64(out, det.bbox[1] as f64);
        out.push(',');
        push_json_f64(out, det.bbox[2] as f64);
        out.push(',');
        push_json_f64(out, det.bbox[3] as f64);
        out.push_str("],\"class_id\":");
        out.push_str(&det.class_id.to_string());
        out.push_str(",\"class_name\":");
        out.push_str(
            &serde_json::to_string(&det.class_name)
                .map_err(|error| VisionEngineError::new(error.to_string()))?,
        );
        out.push_str(",\"confidence\":");
        push_json_f64(out, det.confidence as f64);
        out.push_str(",\"track_id\":");
        out.push_str(&det.track_id.to_string());
        out.push('}');
    }
    out.push(']');
    if let Some(fps) = frame.fps {
        out.push_str(",\"fps\":");
        push_json_f64(out, fps);
    }
    out.push_str(",\"frame_index\":");
    out.push_str(&frame.frame_index.to_string());
    out.push_str(",\"height\":");
    out.push_str(&frame.height.to_string());
    if let Some(time_seconds) = frame.time_seconds {
        out.push_str(",\"time_seconds\":");
        push_json_f64(out, time_seconds);
    }
    out.push_str(",\"width\":");
    out.push_str(&frame.width.to_string());
    out.push('}');
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tracking_decode_fits_without_upscaling() {
        assert_eq!(scaled_decode_size(1920, 1080, 640), (640, 360));
        assert_eq!(scaled_decode_size(320, 180, 640), (320, 180));
        assert_eq!(tracking_decode_filter(640, 360, 8.0), "fps=8,scale=640:360");
    }
}
