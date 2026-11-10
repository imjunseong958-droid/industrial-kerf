//! The render queue: `export {quality, codec?, sequence?, output?, name?}`,
//! `export_cancel {jobId}` and `render_queue {}`.
//!
//! An export is queued with kerfd at once, so it has its job id, its name
//! and the file it will write before anything renders (`export {status:
//! queued}`); a queued job holds its file, so a second export of the same
//! name takes the next free one. The queue's own thread then renders the
//! jobs one at a time, oldest first, from the document as it stood when
//! each was queued. Cancelling a queued job takes it out of the queue;
//! cancelling the running one stops its FFmpeg.
//!
//! The `render_queue` event is kerfd's list of render jobs (the queued, the
//! running and the newest finished, oldest first) with the running job's
//! progress as this bridge sees it. The viewer's own preview renders are
//! not outputs and are left out. It is sent again on every change of
//! status; progress in between comes on `export {status: running}`.

use crate::{emit, Shared};
use kerf_engine::render::RenderPreset;
use kerf_runtime::{RenderJob, RuntimeClient, RuntimeError, StartRenderOptions};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::{HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::SyncSender;
use std::sync::{Arc, Condvar, Mutex};

/// `export {quality: preview|full|review|master, codec?: h264|hevc|
/// prores_hq, sequence?, output?, name?}`.
#[derive(Clone, Debug, Default, Deserialize, PartialEq)]
pub(crate) struct ExportRequest {
    pub quality: String,
    #[serde(default)]
    pub codec: Option<String>,
    /// The sequence to render; the first when absent.
    #[serde(default)]
    pub sequence: Option<String>,
    /// A folder or a file; see [`StartRenderOptions`].
    #[serde(default)]
    pub output: Option<String>,
    /// The job's name in the queue, and the file's in a folder.
    #[serde(default)]
    pub name: Option<String>,
}

impl ExportRequest {
    fn options(&self) -> StartRenderOptions {
        let text = |value: &Option<String>| {
            value
                .as_deref()
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        };
        StartRenderOptions {
            quality: self.quality.trim().to_owned(),
            codec: text(&self.codec),
            sequence: text(&self.sequence),
            output: text(&self.output),
            name: text(&self.name),
            ..StartRenderOptions::default()
        }
    }
}

/// What queues, lists, cancels and renders jobs: kerfd, or a fake in the
/// tests.
pub(crate) trait Renders: Send + Sync {
    fn start(
        &self,
        options: &StartRenderOptions,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError>;
    fn list(&self) -> Result<Vec<RenderJob>, RuntimeError>;
    fn cancel(&self, job_id: &str) -> Result<RenderJob, RuntimeError>;
    /// Render `job` to `output`, recording with kerfd how it ended.
    fn render(
        &self,
        job: &RenderJob,
        document: &str,
        output: &Path,
        progress: &mut dyn FnMut(f64),
    ) -> Result<(), RuntimeError>;
}

impl Renders for RuntimeClient {
    fn start(
        &self,
        options: &StartRenderOptions,
    ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
        self.start_render_with(options)
    }

    fn list(&self) -> Result<Vec<RenderJob>, RuntimeError> {
        self.list_renders()
    }

    fn cancel(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
        self.cancel_render(job_id)
    }

    fn render(
        &self,
        job: &RenderJob,
        document: &str,
        output: &Path,
        progress: &mut dyn FnMut(f64),
    ) -> Result<(), RuntimeError> {
        // A preview-quality export is a draft from proxies, as it always was.
        let draft = job.quality.as_deref() == Some("preview");
        kerf_runtime::run_render_job(self, job, document, output, draft, progress)
    }
}

/// A job queued with kerfd, waiting for its turn.
struct Queued {
    job: RenderJob,
    document: String,
    output: PathBuf,
    renders: Arc<dyn Renders>,
}

#[derive(Default)]
struct State {
    waiting: VecDeque<Queued>,
    /// The job rendering and how far it is.
    running: Option<(String, f64)>,
    /// Jobs the viewer's preview renders made: not outputs.
    viewer: HashSet<String>,
    /// Jobs cancelled here, whose `cancelled` event was already sent.
    cancelled: HashSet<String>,
    closed: bool,
}

/// The queue the bridge's render thread works through.
#[derive(Default)]
pub(crate) struct Queue {
    state: Mutex<State>,
    wake: Condvar,
}

impl Queue {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// Leave the job a viewer preview render made out of the queue.
    pub(crate) fn mark_viewer(&self, job_id: &str) {
        self.lock().viewer.insert(job_id.to_owned());
    }

    /// The bridge closed: nothing more renders. The job rendering stops,
    /// and the render thread cancels what waits.
    pub(crate) fn close(&self, directory: &Path) {
        let running = {
            let mut state = self.lock();
            state.closed = true;
            state.running.as_ref().map(|(id, _)| id.clone())
        };
        self.wake.notify_all();
        if let Some(id) = running {
            // The render watches for this marker; nobody is left to see it.
            let _ = std::fs::write(kerf_runtime::render_cancel_path(directory, &id), b"");
        }
    }

    /// How many jobs wait, not counting the one rendering.
    #[cfg(test)]
    pub(crate) fn waiting(&self) -> usize {
        self.lock().waiting.len()
    }
}

/// An `error` event naming the refused command.
pub(crate) fn refused(tx: &SyncSender<Value>, command: &str, error: &RuntimeError) -> Value {
    let event = json!({"type": "error", "message": error.message, "code": error.code,
        "command": command});
    emit(tx, event.clone());
    event
}

/// What a job is called in the queue: its name, else what it renders.
pub(crate) fn job_name(job: &RenderJob) -> String {
    if let Some(name) = job.name.as_deref().filter(|name| !name.trim().is_empty()) {
        return name.to_owned();
    }
    let quality = match job.quality.as_deref() {
        Some("master") => "Master",
        Some("review") => "Review copy",
        Some("preview") => "Preview",
        _ => "Full render",
    };
    match job.codec.as_deref().and_then(RenderPreset::parse) {
        Some(RenderPreset::ProRes422Hq) if quality != "Master" => {
            format!("{quality} · ProRes 422 HQ")
        }
        Some(RenderPreset::HevcHdr) => format!("{quality} · HEVC 10-bit HDR"),
        _ => quality.to_owned(),
    }
}

/// An `export` event about `job`.
fn export_event(job: &RenderJob, status: &str, progress: f64) -> Value {
    let mut event = json!({
        "type": "export", "status": status, "jobId": job.job_id, "name": job_name(job),
        "path": job.output_path, "progress": progress,
        "quality": job.quality, "codec": job.codec,
    });
    if let Some(sequence) = &job.sequence {
        event["sequence"] = json!(sequence);
    }
    event
}

/// `export`: queue the render with kerfd and in the queue. A refusal
/// (`INVALID_RENDER_OUTPUT`, `RENDER_OUTPUT_EXISTS`, …) comes back as the
/// error; nothing is queued then.
pub(crate) fn submit(
    shared: &Shared,
    renders: Arc<dyn Renders>,
    request: &ExportRequest,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let (job, document, output) = renders.start(&request.options())?;
    emit(tx, export_event(&job, "queued", 0.0));
    {
        let mut state = shared.renders.lock();
        state.waiting.push_back(Queued {
            job,
            document,
            output,
            renders: renders.clone(),
        });
    }
    shared.renders.wake.notify_all();
    publish(shared, renders.as_ref(), tx)
}

/// `export_cancel {jobId}`: a queued job leaves the queue, the running one
/// stops. Either way kerfd records it cancelled, and `export {status:
/// cancelled}` and the queue follow. A job already finished stays as it is.
pub(crate) fn cancel(
    shared: &Shared,
    renders: &dyn Renders,
    job_id: &str,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let job = renders.cancel(job_id)?;
    if job.status == "cancelled" {
        let mut state = shared.renders.lock();
        state.waiting.retain(|queued| queued.job.job_id != job_id);
        let first = state.cancelled.insert(job_id.to_owned());
        drop(state);
        if first {
            emit(tx, export_event(&job, "cancelled", job.progress));
        }
    }
    publish(shared, renders, tx)
}

/// Send `render_queue` with kerfd's jobs as they stand.
pub(crate) fn publish(
    shared: &Shared,
    renders: &dyn Renders,
    tx: &SyncSender<Value>,
) -> Result<(), RuntimeError> {
    let jobs = renders.list()?;
    emit(tx, queue_event(shared, &jobs));
    Ok(())
}

/// The `render_queue` event for kerfd's `jobs`: the viewer's previews left
/// out (and earlier sessions' finished previews whose file is gone), the
/// running job's progress as the bridge has it.
pub(crate) fn queue_event(shared: &Shared, jobs: &[RenderJob]) -> Value {
    let state = shared.renders.lock();
    let jobs: Vec<Value> = jobs
        .iter()
        .filter(|job| !state.viewer.contains(&job.job_id))
        .filter(|job| {
            let finished = matches!(job.status.as_str(), "completed" | "failed" | "cancelled");
            let gone = job
                .output_path
                .as_deref()
                .is_none_or(|path| !Path::new(path).is_file());
            !(job.quality.as_deref() == Some("preview") && finished && gone)
        })
        .map(|job| {
            let rendering = state.running.as_ref().filter(|(id, _)| {
                *id == job.job_id && matches!(job.status.as_str(), "queued" | "running")
            });
            // The job this thread renders reads as running even before
            // kerfd has heard so.
            let (status, progress) = match (rendering, job.status.as_str()) {
                (Some((_, progress)), _) => ("running", *progress),
                (None, "completed") => ("completed", 1.0),
                (None, status) => (status, job.progress),
            };
            let mut entry = json!({
                "jobId": job.job_id, "name": job_name(job), "quality": job.quality,
                "codec": job.codec, "sequence": job.sequence, "status": status,
                "progress": progress, "outputPath": job.output_path,
            });
            if let Some(error) = &job.error {
                entry["error"] = json!(error);
            }
            entry
        })
        .collect();
    json!({"type": "render_queue", "jobs": jobs})
}

/// The render thread: take the oldest queued job, render it, report it,
/// and wait for the next. Once the bridge closes, what still waits is
/// cancelled with kerfd, so no job stays queued with nobody to render it.
pub(crate) fn worker(shared: Arc<Shared>, tx: SyncSender<Value>) {
    loop {
        let next = {
            let mut state = shared.renders.lock();
            while state.waiting.is_empty() && !state.closed {
                state = shared
                    .renders
                    .wake
                    .wait(state)
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
            }
            if state.closed {
                let left: Vec<Queued> = state.waiting.drain(..).collect();
                drop(state);
                for queued in left {
                    let _ = queued.renders.cancel(&queued.job.job_id);
                }
                return;
            }
            let next = state.waiting.pop_front();
            if let Some(queued) = &next {
                state.running = Some((queued.job.job_id.clone(), 0.0));
            }
            next
        };
        if let Some(queued) = next {
            render(&shared, &queued, &tx);
            shared.renders.lock().running = None;
            let _ = publish(&shared, queued.renders.as_ref(), &tx);
        }
    }
}

/// Render one job: `export {status: running}` with its progress, then
/// `completed`, `failed` or `cancelled`.
fn render(shared: &Shared, queued: &Queued, tx: &SyncSender<Value>) {
    let job = &queued.job;
    emit(tx, export_event(job, "running", 0.0));
    let _ = publish(shared, queued.renders.as_ref(), tx);
    // Half-percent steps: enough for a bar, and never a flood.
    let mut step = 0_i64;
    let result = queued
        .renders
        .render(job, &queued.document, &queued.output, &mut |progress| {
            let progress = progress.clamp(0.0, 1.0);
            if let Some((_, running)) = shared.renders.lock().running.as_mut() {
                *running = progress;
            }
            let now = (progress * 200.0).floor() as i64;
            if now > step {
                step = now;
                // Replaceable; never stall the render behind a busy GUI.
                let _ = tx.try_send(export_event(job, "running", progress));
            }
        });
    let mut done = job.clone();
    done.output_path = Some(queued.output.to_string_lossy().into_owned());
    match result {
        Ok(()) => emit(tx, export_event(&done, "completed", 1.0)),
        Err(error) if error.code == "CANCELLED" => {
            if shared.renders.lock().cancelled.insert(job.job_id.clone()) {
                emit(tx, export_event(&done, "cancelled", 0.0));
            }
        }
        Err(error) => {
            let mut event = export_event(&done, "failed", 0.0);
            event["message"] = json!(error.message);
            emit(tx, event);
            emit(
                tx,
                json!({"type": "error", "message": error.message, "code": error.code,
                    "command": "export", "jobId": job.job_id}),
            );
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver};
    use std::time::Duration;

    /// kerfd's render jobs in memory: renders finish when the test lets
    /// them, or end cancelled once cancelled.
    #[derive(Default)]
    pub(crate) struct Fake {
        jobs: Mutex<Vec<RenderJob>>,
        /// Jobs in the order they started rendering.
        pub rendered: Mutex<Vec<String>>,
        /// One message lets one render finish.
        gate: Mutex<Option<Receiver<()>>>,
        next: Mutex<u32>,
    }

    impl Fake {
        pub(crate) fn gated() -> (Arc<Self>, SyncSender<()>) {
            let (open, gate) = mpsc::sync_channel(16);
            let fake = Self::default();
            *fake.gate.lock().unwrap() = Some(gate);
            (Arc::new(fake), open)
        }

        fn status(&self, id: &str) -> String {
            let jobs = self.jobs.lock().unwrap();
            jobs.iter()
                .find(|job| job.job_id == id)
                .map(|job| job.status.clone())
                .unwrap_or_default()
        }

        fn set(&self, id: &str, status: &str) {
            for job in self.jobs.lock().unwrap().iter_mut() {
                if job.job_id == id
                    && !matches!(job.status.as_str(), "completed" | "failed" | "cancelled")
                {
                    job.status = status.into();
                }
            }
        }
    }

    impl Renders for Fake {
        fn start(
            &self,
            options: &StartRenderOptions,
        ) -> Result<(RenderJob, String, PathBuf), RuntimeError> {
            if options.quality == "nope" {
                return Err(RuntimeError::new(
                    "INVALID_RENDER_QUALITY",
                    "quality must be preview, full, review, or master",
                ));
            }
            let mut next = self.next.lock().unwrap();
            *next += 1;
            let id = format!("render-{next}");
            let extension = if options.codec.as_deref() == Some("prores_hq")
                || (options.quality == "master" && options.codec.is_none())
            {
                "mov"
            } else {
                "mp4"
            };
            let output = PathBuf::from(format!(
                "/project/{}.{extension}",
                options.name.clone().unwrap_or_else(|| id.clone())
            ));
            let job = RenderJob {
                job_id: id,
                status: "queued".into(),
                output_url: None,
                output_path: Some(output.to_string_lossy().into_owned()),
                error: None,
                range: None,
                sequence: options.sequence.clone(),
                progress: 0.0,
                name: options.name.clone(),
                quality: Some(options.quality.clone()),
                codec: Some(options.codec.clone().unwrap_or_else(|| {
                    if options.quality == "master" {
                        "prores_hq"
                    } else {
                        "h264"
                    }
                    .into()
                })),
            };
            self.jobs.lock().unwrap().push(job.clone());
            Ok((job, "kerf 1\n".into(), output))
        }

        fn list(&self) -> Result<Vec<RenderJob>, RuntimeError> {
            Ok(self.jobs.lock().unwrap().clone())
        }

        fn cancel(&self, job_id: &str) -> Result<RenderJob, RuntimeError> {
            self.set(job_id, "cancelled");
            let jobs = self.jobs.lock().unwrap();
            jobs.iter()
                .find(|job| job.job_id == job_id)
                .cloned()
                .ok_or_else(|| RuntimeError::new("RENDER_NOT_FOUND", "render job not found"))
        }

        fn render(
            &self,
            job: &RenderJob,
            _document: &str,
            _output: &Path,
            progress: &mut dyn FnMut(f64),
        ) -> Result<(), RuntimeError> {
            self.rendered.lock().unwrap().push(job.job_id.clone());
            self.set(&job.job_id, "running");
            progress(0.5);
            loop {
                if self.status(&job.job_id) == "cancelled" {
                    return Err(RuntimeError::new("CANCELLED", "render job was cancelled"));
                }
                let gate = self.gate.lock().unwrap();
                let opened = gate
                    .as_ref()
                    .map_or(Ok(()), |gate| gate.recv_timeout(Duration::from_millis(10)));
                drop(gate);
                if opened.is_ok() {
                    break;
                }
            }
            self.set(&job.job_id, "completed");
            Ok(())
        }
    }

    fn shared() -> Arc<Shared> {
        Arc::new(crate::tests_support::shared(&std::env::temp_dir()))
    }

    fn request(quality: &str, name: Option<&str>) -> ExportRequest {
        ExportRequest {
            quality: quality.into(),
            name: name.map(str::to_owned),
            ..ExportRequest::default()
        }
    }

    fn wait_for(
        rx: &Receiver<Value>,
        seen: &mut Vec<Value>,
        check: impl Fn(&Value) -> bool,
    ) -> Value {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(found) = seen.iter().find(|event| check(event)) {
                return found.clone();
            }
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match rx.recv_timeout(left) {
                Ok(event) => seen.push(event),
                Err(_) => panic!("no such event in {seen:#?}"),
            }
        }
    }

    #[test]
    fn export_commands_read_as_the_contract_says() {
        let Ok(crate::Command::Export(export)) = serde_json::from_value(json!({
            "type": "export", "quality": "master", "codec": "prores_hq", "sequence": "vertical",
            "output": "deliveries/", "name": "Atlas master",
        })) else {
            panic!("expected export");
        };
        assert_eq!(
            export.options(),
            StartRenderOptions {
                quality: "master".into(),
                codec: Some("prores_hq".into()),
                sequence: Some("vertical".into()),
                output: Some("deliveries/".into()),
                name: Some("Atlas master".into()),
                ..StartRenderOptions::default()
            }
        );
        // What the registry and the old editor send still reads.
        let Ok(crate::Command::Export(plain)) =
            serde_json::from_value(json!({"type": "export", "quality": "full"}))
        else {
            panic!("expected export");
        };
        assert_eq!(plain.options(), StartRenderOptions::new("full"));
        // Blank fields are absent.
        let Ok(crate::Command::Export(blank)) = serde_json::from_value(
            json!({"type": "export", "quality": "review", "name": " ", "output": ""}),
        ) else {
            panic!("expected export");
        };
        assert_eq!(blank.options(), StartRenderOptions::new("review"));
        assert!(matches!(
            serde_json::from_value(json!({"type": "export_cancel", "jobId": "render-1"})).unwrap(),
            crate::Command::ExportCancel { job_id } if job_id == "render-1"
        ));
        assert!(matches!(
            serde_json::from_value(json!({"type": "render_queue"})).unwrap(),
            crate::Command::RenderQueue
        ));
    }

    #[test]
    fn jobs_without_a_name_are_called_by_what_they_render() {
        let job = |quality: &str, codec: &str, name: Option<&str>| RenderJob {
            job_id: "render-1".into(),
            status: "queued".into(),
            output_url: None,
            output_path: None,
            error: None,
            range: None,
            sequence: None,
            progress: 0.0,
            name: name.map(str::to_owned),
            quality: Some(quality.into()),
            codec: Some(codec.into()),
        };
        assert_eq!(job_name(&job("master", "prores_hq", None)), "Master");
        assert_eq!(job_name(&job("master", "h264", Some("Atlas"))), "Atlas");
        assert_eq!(job_name(&job("review", "h264", None)), "Review copy");
        assert_eq!(
            job_name(&job("full", "prores_hq", None)),
            "Full render · ProRes 422 HQ"
        );
        assert_eq!(job_name(&job("preview", "h264", Some("  "))), "Preview");
    }

    #[test]
    fn cancelling_takes_a_queued_job_out_and_stops_the_running_one() {
        let shared = shared();
        let (tx, rx) = mpsc::sync_channel(256);
        let (fake, open) = Fake::gated();
        let worker = {
            let (shared, tx) = (shared.clone(), tx.clone());
            std::thread::spawn(move || worker(shared, tx))
        };
        let renders: Arc<dyn Renders> = fake.clone();
        for name in ["One", "Two", "Three"] {
            submit(&shared, renders.clone(), &request("full", Some(name)), &tx).unwrap();
        }
        let mut seen = Vec::new();
        wait_for(&rx, &mut seen, |e| {
            e["status"] == "running" && e["jobId"] == "render-1"
        });
        // The second never renders.
        cancel(&shared, fake.as_ref(), "render-2", &tx).unwrap();
        let cancelled = wait_for(&rx, &mut seen, |e| e["status"] == "cancelled");
        assert_eq!(cancelled["jobId"], "render-2");
        assert_eq!(cancelled["name"], "Two");
        // The running one stops.
        cancel(&shared, fake.as_ref(), "render-1", &tx).unwrap();
        wait_for(&rx, &mut seen, |e| {
            e["status"] == "cancelled" && e["jobId"] == "render-1"
        });
        wait_for(&rx, &mut seen, |e| {
            e["status"] == "running" && e["jobId"] == "render-3"
        });
        open.send(()).unwrap();
        wait_for(&rx, &mut seen, |e| {
            e["status"] == "completed" && e["jobId"] == "render-3"
        });
        assert_eq!(*fake.rendered.lock().unwrap(), ["render-1", "render-3"]);
        // Each cancel was told once.
        let told = seen
            .iter()
            .filter(|e| e["type"] == "export" && e["status"] == "cancelled")
            .count();
        assert_eq!(told, 2, "{seen:#?}");
        let queue = wait_for(&rx, &mut seen, |e| {
            e["type"] == "render_queue" && e["jobs"][2]["status"] == "completed"
        });
        let statuses: Vec<&Value> = queue["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|j| &j["status"])
            .collect();
        assert_eq!(
            statuses,
            [
                &json!("cancelled"),
                &json!("cancelled"),
                &json!("completed")
            ]
        );
        // A finished job stays as it is.
        cancel(&shared, fake.as_ref(), "render-3", &tx).unwrap();
        assert_eq!(fake.status("render-3"), "completed");
        shared.renders.close(&std::env::temp_dir());
        worker.join().unwrap();
    }

    #[test]
    fn refused_exports_queue_nothing() {
        let shared = shared();
        let (tx, _rx) = mpsc::sync_channel(16);
        let fake = Arc::new(Fake::default());
        let error = submit(&shared, fake.clone(), &request("nope", None), &tx).unwrap_err();
        assert_eq!(error.code, "INVALID_RENDER_QUALITY");
        assert_eq!(shared.renders.waiting(), 0);
        let event = refused(&tx, "export", &error);
        assert_eq!(event["command"], "export");
        assert_eq!(event["code"], "INVALID_RENDER_QUALITY");
    }

    #[test]
    fn the_queue_leaves_out_the_viewers_previews() {
        let shared = shared();
        let job = |id: &str, quality: &str, status: &str, path: Option<&str>| RenderJob {
            job_id: id.into(),
            status: status.into(),
            output_url: None,
            output_path: path.map(str::to_owned),
            error: (status == "failed").then(|| "ffmpeg exited".to_owned()),
            range: None,
            sequence: None,
            progress: 0.25,
            name: None,
            quality: Some(quality.into()),
            codec: Some("h264".into()),
        };
        let kept = std::env::temp_dir().join("kerf-queue-kept.mp4");
        std::fs::write(&kept, b"x").unwrap();
        shared.renders.mark_viewer("render-viewer");
        shared.renders.lock().running = Some(("render-running".into(), 0.75));
        let event = queue_event(
            &shared,
            &[
                job("render-viewer", "preview", "running", None),
                job(
                    "render-old-preview",
                    "preview",
                    "completed",
                    Some("/gone/preview.mp4"),
                ),
                job("render-kept-preview", "preview", "completed", kept.to_str()),
                job("render-running", "full", "running", Some("/p/a.mp4")),
                job("render-failed", "review", "failed", None),
            ],
        );
        let ids: Vec<&str> = event["jobs"]
            .as_array()
            .unwrap()
            .iter()
            .map(|job| job["jobId"].as_str().unwrap())
            .collect();
        assert_eq!(
            ids,
            ["render-kept-preview", "render-running", "render-failed"]
        );
        assert_eq!(event["jobs"][0]["progress"], 1.0);
        assert_eq!(event["jobs"][1]["progress"], 0.75);
        assert_eq!(event["jobs"][1]["outputPath"], "/p/a.mp4");
        assert_eq!(event["jobs"][2]["error"], "ffmpeg exited");
        assert_eq!(event["jobs"][2]["name"], "Review copy");
        let _ = std::fs::remove_file(kept);
    }
}
