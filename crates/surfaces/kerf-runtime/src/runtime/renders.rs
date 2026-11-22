//! The render queue: starting a render job, the status and progress its
//! renderer reports, cancelling it, and which finished jobs are kept.

use super::{JournalEvent, Runtime, MAX_FINISHED_JOBS};
use crate::kernel;
use crate::render::{render_cancel_path, render_name, render_output, Encoding};
use crate::{io_error, unique_id, RenderJob, RenderRange, RuntimeError, StartRenderOptions};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

impl Runtime {
    /// Queue a render of the first sequence of the project, or of what a
    /// pending `proposal` would make of it, and return the document to
    /// render, with media paths resolved. Requests go through
    /// [`Runtime::start_render_with`]; tests use this shorter form.
    #[cfg(test)]
    pub fn start_render_range(
        &mut self,
        quality: &str,
        range: Option<RenderRange>,
        proposal: Option<&str>,
        sequence: Option<&str>,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        self.start_render_with(StartRenderOptions {
            quality: quality.to_owned(),
            range,
            proposal: proposal.map(str::to_owned),
            sequence: sequence.map(str::to_owned),
            ..StartRenderOptions::default()
        })
    }

    /// Queue a render as `options` describe it: what it renders, its quality
    /// and codec, where its file goes, and its name. Returns the job, the
    /// document to render with media paths resolved, and the file to write.
    pub fn start_render_with(
        &mut self,
        options: StartRenderOptions,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        let StartRenderOptions {
            quality,
            range,
            proposal,
            sequence,
            codec,
            output,
            name,
        } = options;
        let (proposal, sequence) = (proposal.as_deref(), sequence.as_deref());
        let encoding = Encoding::new(&quality, codec.as_deref())?;
        let name = render_name(name.as_deref())?;
        let proposed = proposal.map(|id| self.proposal_result(id)).transpose()?;
        let project = proposed.as_ref().unwrap_or(&self.state.project);
        let document = self.renderable(project);
        let duration = kernel::render_sequence(project, sequence)?
            .duration()
            .seconds();
        if let Some(range) = range.as_ref() {
            RenderRange::new(range.start, range.duration)?;
            if range.end() > duration + 1e-9 {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_RANGE",
                    format!(
                        "render range ends at {:.3}s but project duration is {duration:.3}s",
                        range.end(),
                    ),
                ));
            }
        }
        let id = format!("render-{}", unique_id());
        let target = render_output(
            self.directory(),
            output.as_deref(),
            name.as_deref().unwrap_or(&id),
            encoding.extension(),
        )?;
        let writes = |file: &Path, status: fn(&str) -> bool| {
            self.state.jobs.values().any(|job| {
                status(&job.status) && job.output_path.as_deref().map(Path::new) == Some(file)
            })
        };
        if writes(&target.file, |status| !job_finished(status)) {
            return Err(RuntimeError::new(
                "RENDER_OUTPUT_BUSY",
                format!("another render is writing {}", target.file.display()),
            ));
        }
        // A file on disk, or one a kept job made or is making, is never
        // written over: the queue would misdescribe it.
        let target = target.free(|file| {
            file.symlink_metadata().is_ok()
                || writes(file, |status| {
                    status == "completed" || !job_finished(status)
                })
        })?;
        target.make(self.directory())?;
        let output = target.file;
        let cancel_path =
            render_cancel_path(self.project_path.parent().unwrap_or(Path::new(".")), &id);
        match fs::remove_file(&cancel_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(io_error(error)),
        }
        let job = RenderJob {
            job_id: id.clone(),
            status: "queued".into(),
            output_url: None,
            output_path: Some(output.to_string_lossy().into_owned()),
            error: None,
            range,
            sequence: sequence.map(str::to_owned),
            progress: 0.0,
            name,
            quality: Some(encoding.quality.to_owned()),
            codec: Some(encoding.codec.to_owned()),
        };
        let mut candidate = self.state.clone();
        candidate.jobs.insert(id, job.clone());
        bound_jobs(&mut candidate.jobs);
        self.commit_candidate(candidate, JournalEvent::Job { job: job.clone() })?;
        Ok((job, document, output))
    }

    pub fn update_job(
        &mut self,
        id: &str,
        status: &str,
        error: Option<String>,
        output_url: Option<String>,
        progress: Option<f64>,
    ) -> Result<(), RuntimeError> {
        if !matches!(
            status,
            "queued" | "running" | "completed" | "failed" | "cancelled"
        ) {
            return Err(RuntimeError::new(
                "INVALID_RENDER_STATUS",
                format!("unsupported render status: {status}"),
            ));
        }
        let current = self.state.jobs.get(id).cloned().ok_or_else(|| {
            RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
        })?;
        if matches!(
            current.status.as_str(),
            "completed" | "failed" | "cancelled"
        ) && current.status != status
        {
            return Err(RuntimeError::new(
                "RENDER_STATE_CONFLICT",
                format!("render job {id} is already {}", current.status),
            ));
        }
        let mut job = current.clone();
        job.status = status.into();
        job.error = error;
        job.output_url = output_url;
        if let Some(progress) = progress {
            if !progress.is_finite() || !(0.0..=1.0).contains(&progress) {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_PROGRESS",
                    "render progress must be finite and between 0 and 1",
                ));
            }
            job.progress = progress;
        }
        if status == "completed" {
            job.progress = 1.0;
        }
        if job.status == current.status
            && job.error == current.error
            && job.output_url == current.output_url
        {
            // Only progress moved. A render reports it often, and a kerfd
            // that restarts fails the job anyway, so it is kept in memory
            // rather than journaled; the next checkpoint carries it.
            self.state.jobs.insert(id.to_owned(), job);
            return Ok(());
        }
        let mut candidate = self.state.clone();
        candidate.jobs.insert(id.to_owned(), job.clone());
        // A job that finishes may take the finished history past its bound,
        // as journal replay of this record would.
        if job_finished(&job.status) {
            bound_jobs(&mut candidate.jobs);
        }
        self.commit_candidate(candidate, JournalEvent::Job { job })?;
        Ok(())
    }

    /// Every render job kept, oldest first: queued and running ones, and
    /// the newest finished.
    pub fn render_jobs(&self) -> Vec<RenderJob> {
        let mut jobs: Vec<RenderJob> = self.state.jobs.values().cloned().collect();
        jobs.sort_by(|a, b| job_order(&a.job_id).cmp(&job_order(&b.job_id)));
        jobs
    }

    pub fn render_job(&self, id: &str) -> Result<RenderJob, RuntimeError> {
        self.state.jobs.get(id).cloned().ok_or_else(|| {
            RuntimeError::new("RENDER_NOT_FOUND", format!("render job not found: {id}"))
        })
    }

    pub fn cancel_render(&mut self, id: &str) -> Result<RenderJob, RuntimeError> {
        let current = self.render_job(id)?;
        if matches!(current.status.as_str(), "queued" | "running") {
            let cancel_path =
                render_cancel_path(self.project_path.parent().unwrap_or(Path::new(".")), id);
            fs::write(&cancel_path, b"cancelled\n").map_err(io_error)?;
            self.update_job(
                id,
                "cancelled",
                Some("render job was cancelled".into()),
                None,
                None,
            )?;
        }
        self.render_job(id)
    }

    pub(super) fn reconcile_abandoned_jobs(&mut self) -> bool {
        let mut changed = false;
        for job in self.state.jobs.values_mut() {
            if matches!(job.status.as_str(), "queued" | "running") {
                job.status = "failed".into();
                job.error = Some("render job was abandoned when kerfd restarted".into());
                changed = true;
            }
        }
        changed
    }
}

/// Whether a render job with `status` has ended.
fn job_finished(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled")
}

/// The order render jobs were made in, from their ids: `render-<nanos>-<seq>`
/// in hex, where the time comes first and the sequence, which starts again
/// at 0 each time kerfd starts, breaks ties within one run. Each part is
/// compared as a number, so neither a longer sequence nor a longer time
/// sorts wrongly; an id of another shape sorts first, by its text.
fn job_order(id: &str) -> (Option<(u128, u64)>, &str) {
    let parsed = id
        .strip_prefix("render-")
        .and_then(|rest| rest.split_once('-'))
        .and_then(|(nanos, seq)| {
            Some((
                u128::from_str_radix(nanos, 16).ok()?,
                u64::from_str_radix(seq, 16).ok()?,
            ))
        });
    (parsed, id)
}

/// Drop the oldest finished render jobs until at most `MAX_FINISHED_JOBS`
/// remain, in the order [`job_order`] gives. Their outputs stay where they
/// are.
pub(super) fn bound_jobs(jobs: &mut HashMap<String, RenderJob>) {
    let mut finished: Vec<String> = jobs
        .iter()
        .filter(|(_, job)| job_finished(&job.status))
        .map(|(id, _)| id.clone())
        .collect();
    if finished.len() <= MAX_FINISHED_JOBS {
        return;
    }
    finished.sort_unstable_by(|a, b| job_order(a).cmp(&job_order(b)));
    for id in &finished[..finished.len() - MAX_FINISHED_JOBS] {
        jobs.remove(id);
    }
}
