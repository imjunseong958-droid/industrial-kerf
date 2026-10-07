//! Render workers. A worker renders a job's document in-process; the
//! runtime keeps the job's durable state.

use crate::{kernel, RenderJob, RenderRange, RuntimeClient, RuntimeError};
use std::fs;
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

/// Run a queued render job in-process and record with cutd how it ended:
/// running, then completed with its output, or failed unless it was
/// cancelled. `document` is the `.cut` text the job carries; the job's
/// sequence, or the first, is rendered. `on_progress` runs on this thread and must not
/// block on UI state.
pub fn run_render_job(
    client: &RuntimeClient,
    job: &RenderJob,
    document: &str,
    output: &Path,
    draft: bool,
    on_progress: impl FnMut(f64),
) -> Result<(), RuntimeError> {
    let cancel_file = render_cancel_path(client.directory(), &job.job_id);
    let _ = client.update_job(&job.job_id, "running", None, None);
    let result = render(
        document,
        output,
        draft,
        job.sequence.as_deref(),
        job.range.as_ref(),
        &cancel_file,
        on_progress,
    );
    let recorded = match &result {
        Ok(()) => client.update_job(
            &job.job_id,
            "completed",
            None,
            Some(output.to_string_lossy().into_owned()),
        ),
        Err(error) if error.code == "CANCELLED" => Ok(()),
        Err(error) => client.update_job(&job.job_id, "failed", Some(error.message.clone()), None),
    };
    let _ = fs::remove_file(&cancel_file);
    result.and(recorded)
}

/// Render `document`, or `range` of it, to `output`. Creating `cancel_file`
/// cancels the render.
fn render(
    document: &str,
    output: &Path,
    draft: bool,
    sequence: Option<&str>,
    range: Option<&RenderRange>,
    cancel_file: &Path,
    mut on_progress: impl FnMut(f64),
) -> Result<(), RuntimeError> {
    let project = kernel::parse(document)?;
    let target = kernel::render_sequence(&project, sequence)?;
    let window = match range {
        Some(range) => {
            RenderRange::new(range.start, range.duration)?;
            Some(kernel::time_range(target, range.start, range.end())?)
        }
        None => None,
    };
    let opts = cut_engine::render::RenderOptions {
        output: output.to_string_lossy().into_owned(),
        draft,
        ..cut_engine::render::RenderOptions::default()
    };
    let cancellation = cut_engine::CancellationToken::new();
    let watcher = cut_engine::render::spawn_cancel_file_watcher(
        cancel_file.to_path_buf(),
        cancellation.clone(),
    );
    let result = cut_engine::render::render_program_with_cancellation(
        &project,
        sequence,
        window,
        &opts,
        &cancellation,
        Some(&mut on_progress),
    );
    cancellation.cancel();
    let _ = watcher.join();
    match result {
        Ok(()) if output.is_file() => Ok(()),
        Ok(()) => Err(RuntimeError::new(
            "ENGINE_ERROR",
            format!("render completed without producing {}", output.display()),
        )),
        Err(cut_engine::EngineError::Cancelled) => {
            Err(RuntimeError::new("CANCELLED", "render job was cancelled"))
        }
        Err(error) => Err(RuntimeError::new("ENGINE_ERROR", error.to_string())),
    }
}
