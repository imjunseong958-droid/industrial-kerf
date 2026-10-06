//! Render workers. A worker renders a job's document in-process; the
//! runtime keeps the job's durable state.

use crate::{kernel, RenderRange, RuntimeError};
use std::path::{Path, PathBuf};

const RENDER_CANCEL_FILE_PREFIX: &str = ".cut-render-";

/// Return the project-local cancellation marker for one durable render job.
///
/// The marker is an execution signal, not project data. It lets a worker in a
/// separate client process deliver cancellation to the exact engine process
/// that owns the job's FFmpeg child.
pub fn render_cancel_path(directory: &Path, job_id: &str) -> PathBuf {
    directory.join(format!("{RENDER_CANCEL_FILE_PREFIX}{job_id}.cancel"))
}

fn engine_error(error: cut_engine::EngineError) -> RuntimeError {
    match error {
        cut_engine::EngineError::Cancelled => {
            RuntimeError::new("CANCELLED", "render job was cancelled")
        }
        other => RuntimeError::new("ENGINE_ERROR", other.to_string()),
    }
}

/// Run one render worker in-process. `document` is the `.cut` text a render
/// job carries; its first sequence is rendered, optionally only `range`.
/// Creating `cancel_file` cancels the render. The runtime remains the durable
/// owner of job state. `on_progress` runs on this worker thread and must not
/// block on UI state.
pub fn run_render<F>(
    id: String,
    document: String,
    output: PathBuf,
    draft: bool,
    range: Option<RenderRange>,
    cancel_file: Option<PathBuf>,
    mut on_progress: F,
) -> Result<(), RuntimeError>
where
    F: FnMut(f64),
{
    let project = kernel::parse(&document)?;
    let window = match range.as_ref() {
        Some(range) => {
            RenderRange::new(range.start, range.duration)?;
            Some(kernel::time_range(&project, range.start, range.end())?)
        }
        None => None,
    };
    let opts = cut_engine::render::RenderOptions {
        output: output.to_string_lossy().into_owned(),
        draft,
        ..cut_engine::render::RenderOptions::default()
    };
    let cancellation = cut_engine::CancellationToken::new();
    let watcher = cancel_file.as_ref().map(|path| {
        cut_engine::render::spawn_cancel_file_watcher(path.clone(), cancellation.clone())
    });
    let result = cut_engine::render::render_program_with_cancellation(
        &project,
        None,
        window,
        &opts,
        &cancellation,
        Some(&mut on_progress),
    );
    cancellation.cancel();
    if let Some(watcher) = watcher {
        let _ = watcher.join();
    }
    match result {
        Ok(()) => {
            if output.is_file() {
                Ok(())
            } else {
                Err(RuntimeError::new(
                    "ENGINE_ERROR",
                    format!(
                        "render job {id} completed without producing {}",
                        output.display()
                    ),
                ))
            }
        }
        Err(cut_engine::EngineError::Cancelled) => Err(RuntimeError::new(
            "CANCELLED",
            format!("render job {id} was cancelled"),
        )),
        Err(error) => Err(engine_error(error)),
    }
}
