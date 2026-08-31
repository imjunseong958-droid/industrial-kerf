//! Resolve YOLO checkpoints to ONNX files for in-process inference.
//!
//! `.pt` is a training/export artifact. Runtime tracking only loads `.onnx`
//! (the path itself, or a sibling `*.onnx` next to a `.pt` checkpoint).

use super::VisionEngineError;
use std::path::{Path, PathBuf};

/// Map a configured weights path to the ONNX file that native tracking will load.
pub fn resolve_inference_weights(path: impl AsRef<Path>) -> Result<PathBuf, VisionEngineError> {
    let path = path.as_ref();
    let display = path.display();
    let ext = path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| ext.to_ascii_lowercase());
    match ext.as_deref() {
        Some("onnx") => {
            if path.is_file() {
                Ok(path.to_path_buf())
            } else {
                Err(VisionEngineError::new(format!(
                    "ONNX weights not found: {display}"
                )))
            }
        }
        Some("pt") => {
            let onnx = path.with_extension("onnx");
            if onnx.is_file() {
                Ok(onnx)
            } else {
                Err(VisionEngineError::new(format!(
                    "PyTorch checkpoints are not loaded at inference time. Export ONNX next to the weights:\n  yolo export model={display} format=onnx\nExpected native weights at {}",
                    onnx.display()
                )))
            }
        }
        _ => {
            let onnx = path.with_extension("onnx");
            if onnx.is_file() {
                Ok(onnx)
            } else if path.is_file() {
                Err(VisionEngineError::new(format!(
                    "weights must be .onnx (or a .pt checkpoint with a sibling .onnx): {display}"
                )))
            } else {
                Err(VisionEngineError::new(format!(
                    "ONNX weights not found: {display}"
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_inference_weights;
    use std::fs;
    use std::io::Write;

    #[test]
    fn pt_resolves_to_sibling_onnx() {
        let dir = tempfile::tempdir().unwrap();
        let pt = dir.path().join("custom.pt");
        let onnx = dir.path().join("custom.onnx");
        fs::write(&pt, b"pt").unwrap();
        let mut file = fs::File::create(&onnx).unwrap();
        file.write_all(b"onnx").unwrap();
        assert_eq!(resolve_inference_weights(&pt).unwrap(), onnx);
        assert_eq!(resolve_inference_weights(&onnx).unwrap(), onnx);
    }

    #[test]
    fn pt_without_onnx_explains_export() {
        let dir = tempfile::tempdir().unwrap();
        let pt = dir.path().join("only.pt");
        fs::write(&pt, b"pt").unwrap();
        let err = resolve_inference_weights(&pt).unwrap_err();
        assert!(err.message.contains("yolo export"));
        assert!(err.message.contains("only.onnx"));
    }
}
