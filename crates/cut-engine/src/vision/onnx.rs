//! ONNX Runtime inference backend for YOLO models.

use super::yolo::{InferenceBackend, YoloOutputLayout, YoloRawOutput};
use super::VisionEngineError;
use ort::session::builder::GraphOptimizationLevel;
use ort::session::Session;
use ort::value::TensorRef;
use std::path::Path;

pub struct OnnxRuntimeBackend {
    session: Session,
    input_name: String,
}

impl OnnxRuntimeBackend {
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, VisionEngineError> {
        // Sequential frame inference: a few intra-op threads beat oversubscribing
        // every logical core while ffmpeg is already decoding on the other side.
        let threads = crate::resources::onnx_thread_count();
        let mut builder = Session::builder().map_err(map_ort_error)?;
        builder = builder
            .with_optimization_level(GraphOptimizationLevel::Level3)
            .map_err(map_ort_error)?;
        builder = builder.with_intra_threads(threads).map_err(map_ort_error)?;
        builder = builder.with_inter_threads(1).map_err(map_ort_error)?;
        let session = builder
            .commit_from_file(path.as_ref())
            .map_err(map_ort_error)?;
        let input_name = session
            .inputs
            .first()
            .map(|input| input.name.clone())
            .unwrap_or_else(|| "images".to_string());
        Ok(Self {
            session,
            input_name,
        })
    }
}

impl InferenceBackend for OnnxRuntimeBackend {
    fn infer(
        &mut self,
        nchw: &[f32],
        n: usize,
        c: usize,
        h: usize,
        w: usize,
        dest: &mut YoloRawOutput,
    ) -> Result<(), VisionEngineError> {
        let expected = n * c * h * w;
        if nchw.len() != expected {
            return Err(VisionEngineError::new(format!(
                "input tensor size mismatch: expected {expected} elements, got {}",
                nchw.len()
            )));
        }

        let input = TensorRef::from_array_view(([n, c, h, w], nchw)).map_err(map_ort_error)?;
        let outputs = self
            .session
            .run(ort::inputs![self.input_name.as_str() => input])
            .map_err(map_ort_error)?;
        let output = &outputs[0];
        let view = output.try_extract_array::<f32>().map_err(map_ort_error)?;
        fill_yolo_output(view.shape(), view.as_slice().unwrap_or(&[]), dest)
    }
}

fn fill_yolo_output(
    shape: &[usize],
    data: &[f32],
    dest: &mut YoloRawOutput,
) -> Result<(), VisionEngineError> {
    if shape.len() != 3 || shape[0] != 1 {
        return Err(VisionEngineError::new(format!(
            "expected YOLO output shape [1, A, B], got {shape:?}"
        )));
    }
    let a = shape[1];
    let b = shape[2];
    let (layout, num_classes, num_predictions) = if a < b {
        if a < 4 {
            return Err(VisionEngineError::new(format!(
                "invalid YOLO output shape [1, {a}, {b}]: channels-first axis too small"
            )));
        }
        (YoloOutputLayout::ChannelsFirst, a - 4, b)
    } else {
        if b < 4 {
            return Err(VisionEngineError::new(format!(
                "invalid YOLO output shape [1, {a}, {b}]: rows-first axis too small"
            )));
        }
        (YoloOutputLayout::RowsFirst, b - 4, a)
    };
    dest.data.clear();
    dest.data.extend_from_slice(data);
    dest.layout = layout;
    dest.num_predictions = num_predictions;
    dest.num_classes = num_classes;
    Ok(())
}

fn map_ort_error(error: ort::Error) -> VisionEngineError {
    VisionEngineError::new(error.to_string())
}
