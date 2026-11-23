//! `render`: the sequence, a range of it, or what a pending proposal would make, rendered to a
//! file by a job; or written whole as a timeline file another editor opens.

use super::tools::{input, RenderInput};
use super::{program, program_position};
use crate::{KerfMcpRouter, McpCode, McpError};
use kerf_model::Time;
use kerf_runtime::{ErrorCode, RenderJob, RenderQuality, RenderRange, RenderStatus};
use serde_json::{json, Value};
use std::path::PathBuf;

impl KerfMcpRouter {
    /// Start a render job of the project, or of a pending proposal, in the
    /// background and return it queued. It runs in this process, which
    /// waits for it or cancels it before exiting (see `finish_renders`).
    fn render_start(
        &self,
        draft: bool,
        range: Option<RenderRange>,
        proposal: Option<&str>,
        sequence: Option<&str>,
    ) -> Result<(RenderJob, PathBuf), McpError> {
        let (job, document, output) = self
            .runtime
            .start_render_range(
                if draft {
                    RenderQuality::Preview
                } else {
                    RenderQuality::Full
                },
                range,
                proposal,
                sequence,
            )
            .map_err(McpError::from)?;
        let runtime = self.runtime.clone();
        let worker_job = job.clone();
        let worker_output = output.clone();
        self.spawn_render(job.job_id.clone(), move || {
            let mut last_progress = -1.0_f64;
            let rendered = kerf_runtime::run_render_job(
                &runtime,
                &worker_job,
                &document,
                &worker_output,
                draft,
                |progress| {
                    if progress < 1.0 && progress - last_progress < 0.01 {
                        return;
                    }
                    last_progress = progress;
                    // A progress kerfd did not take is made up for by the next one, and how
                    // the job ended is recorded by run_render_job.
                    let _ = runtime.update_job_with_progress(
                        &worker_job.job_id,
                        RenderStatus::Running,
                        None,
                        None,
                        Some(progress),
                    );
                },
            );
            // kerfd has the job's end, which the agent asks it for; a render that failed, or
            // whose end kerfd could not be told, is said here too. A cancel is not a failure.
            if let Err(error) = rendered
                && error.code != ErrorCode::Cancelled
            {
                log::warn!(
                    "render {} failed, or kerfd was not told how it ended: {}",
                    worker_job.job_id,
                    error.message
                );
            }
        });
        Ok((job, output))
    }

    /// Write a sequence of the project, or of what a pending proposal would
    /// make, as a timeline file another editor opens, and say what the file
    /// does not carry.
    fn handoff(
        &self,
        format: &str,
        sequence: Option<&str>,
        proposal: Option<&str>,
    ) -> Result<Value, McpError> {
        let format = kerf_interchange::Format::ALL
            .into_iter()
            .find(|known| known.name() == format)
            .ok_or_else(|| {
                McpError::new(
                    McpCode::InvalidArguments,
                    format!("handoff `{format}` is not otio or fcpxml"),
                )
            })?;
        let (revision, mut project) = self.project_at(proposal)?;
        let sequence = program(&project, sequence)?;
        let (id, name) = (sequence.id.to_string(), sequence.name.clone());
        // Another editor finds a file by where it is, not by where the project is.
        let directory = self.runtime.directory();
        kerf_runtime::resolve_media_paths(&mut project, directory);
        let output = directory.join("deliveries").join(format!(
            "{}.{}",
            file_stem(&name, &id),
            format.name()
        ));
        let written = kerf_interchange::export(&project, Some(&id), &output, format)
            .map_err(|error| McpError::new(McpCode::HandlerFailed, error.to_string()))?;
        let listed = |issues: &[kerf_interchange::InterchangeIssue]| -> Vec<Value> {
            issues
                .iter()
                .map(|issue| json!({"code": issue.code, "at": issue.path, "what": issue.message}))
                .collect()
        };
        let mut result = json!({
            "output": written.output_path,
            "handoff": format.name(),
            "sequence": id,
            "revision": revision,
            "notCarried": listed(&written.report.losses),
            "check": listed(&written.report.warnings),
        });
        if let Some(proposal) = proposal {
            result["proposal"] = json!(proposal);
        }
        Ok(result)
    }

    pub(crate) fn render(&self, arguments: Value) -> Result<Value, McpError> {
        let input: RenderInput = input(arguments)?;
        if let Some(format) = input.handoff.as_deref() {
            if input.job.is_some()
                || input.cancel
                || input.quality.is_some()
                || input.from.is_some()
                || input.to.is_some()
            {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`handoff` writes the whole sequence as a timeline file; it takes no job, quality, or range",
                ));
            }
            return self.handoff(format, input.sequence.as_deref(), input.proposal.as_deref());
        }
        if let Some(job) = input.job.as_deref() {
            if input.quality.is_some()
                || input.from.is_some()
                || input.to.is_some()
                || input.sequence.is_some()
                || input.proposal.is_some()
            {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    "`job` reads a render; it takes no quality, range, sequence, or proposal",
                ));
            }
            let job = if input.cancel {
                self.runtime.cancel_render(job)?
            } else {
                self.runtime.render_job(job)?
            };
            return Ok(json!({
                "job": job.job_id,
                "status": job.status,
                "progress": job.progress,
                "output": job.output_path,
                "error": job.error,
            }));
        }
        if input.cancel {
            return Err(McpError::new(
                McpCode::InvalidArguments,
                "cancel needs `job`",
            ));
        }
        let draft = match input.quality.as_deref() {
            None | Some("draft") => true,
            Some("full") => false,
            Some(other) => {
                return Err(McpError::new(
                    McpCode::InvalidArguments,
                    format!("quality `{other}` is not draft or full"),
                ))
            }
        };
        let (_, project) = self.project_at(input.proposal.as_deref())?;
        let sequence = program(&project, input.sequence.as_deref())?;
        if !sequence.duration().is_positive() {
            return Err(McpError::new(
                McpCode::EmptySequence,
                format!(
                    "sequence `{}` is empty{}",
                    sequence.id,
                    self.any_pending("proposal")
                ),
            ));
        }
        let mut range = None;
        if input.from.is_some() || input.to.is_some() {
            let from = match &input.from {
                Some(text) => program_position(&project, sequence, text)?,
                None => Time::ZERO,
            };
            let to = match &input.to {
                Some(text) => program_position(&project, sequence, text)?,
                None => sequence.duration(),
            };
            range = Some(kerf_runtime::RenderRange::new(
                from.seconds(),
                (to - from).seconds(),
            )?);
        }
        // The cut itself, whole and at full quality: what the person will
        // keep, not a check of a range, a draft, or a proposal.
        let deliverable = !draft && range.is_none() && input.proposal.is_none();
        let (job, output) = self.render_start(
            draft,
            range,
            input.proposal.as_deref(),
            input.sequence.as_deref(),
        )?;
        let mut result = json!({
            "job": job.job_id,
            "status": job.status,
            "output": output,
            "sequence": sequence.id.to_string(),
            "revision": self.runtime.get_state()?.revision,
            "deliverable": deliverable,
        });
        if let Some(id) = input.proposal {
            result["proposal"] = json!(id);
        }
        Ok(result)
    }
}

/// A file name for a sequence's hand-off: its name without what a path or a
/// file system keeps for itself, or its id where that leaves nothing.
fn file_stem(name: &str, id: &str) -> String {
    let stem: String = name
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect();
    let stem = stem.trim().trim_matches('.').trim();
    if stem.is_empty() {
        id.to_owned()
    } else {
        stem.to_owned()
    }
}
