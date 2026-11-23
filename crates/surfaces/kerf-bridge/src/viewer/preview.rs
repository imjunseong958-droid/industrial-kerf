//! Viewer previews: the program, a range of it, or what a proposal would make of it,
//! rendered on a thread of its own and played in the viewer in place of the program;
//! stopped, or left for the program again, when asked; and their files removed once
//! nothing shows them.

use crate::event::{Preview, PreviewStatus};
use crate::wake::Outbox;
use crate::{emit, error, Event, MediaProject, Shared};
use kerf_model::{
    AudioInfo, Clip, Content, Id, Media, Project, Ratio, Sequence, Time, Track, TrackKind,
    VideoInfo,
};
use kerf_runtime::{ErrorCode, RuntimeClient};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::thread;

/// One render the runtime worker hands to an export thread.
struct RenderRequest {
    quality: String,
    /// Played in place of the program once rendered.
    preview: bool,
    proposal: Option<String>,
    range: Option<kerf_runtime::RenderRange>,
    sequence: Option<String>,
}

impl RenderRequest {
    /// A `preview` event about this render. Every render the bridge runs here is a viewer
    /// preview (`preview` is always true); the render queue's exports are `queue.rs`'s.
    fn event(&self, status: PreviewStatus) -> Preview {
        Preview {
            status,
            proposal_id: self.proposal.clone(),
            ..Preview::default()
        }
    }
}

/// `preview`, as the runtime worker takes it with kerfd reached: its range checked, then
/// rendered as the preview of `turn` (see [`spawn_render`]).
pub(crate) fn start(
    turn: u64,
    proposal_id: Option<String>,
    start: Option<f64>,
    duration: Option<f64>,
    client: &RuntimeClient,
    tx: &Outbox,
    shared: &Arc<Shared>,
) {
    let range = match (start, duration) {
        (None, None) => Ok(None),
        (Some(start), Some(duration)) => kerf_runtime::RenderRange::new(start, duration)
            .map(Some)
            .map_err(|err| err.message),
        _ => Err("A preview range needs both start and duration".to_owned()),
    };
    let range = match range {
        Ok(range) => range,
        Err(message) => {
            error(tx, message);
            preview_unstarted(shared, tx, turn, proposal_id);
            return;
        }
    };
    spawn_render(
        RenderRequest {
            quality: "preview".into(),
            preview: true,
            proposal: proposal_id,
            range,
            sequence: None,
        },
        turn,
        client,
        tx,
        shared,
    );
}

/// A preview that ends before any render of it began: refused, or stopped
/// on its way here. One asked to stop says so, which answers the stop.
pub(crate) fn preview_unstarted(shared: &Shared, tx: &Outbox, turn: u64, proposal: Option<String>) {
    if shared.preview_turns.over(turn) {
        emit(
            tx,
            Event::Preview(Preview {
                status: PreviewStatus::Stopped,
                proposal_id: proposal,
                ..Preview::default()
            }),
        );
    }
}

/// Run the preview `request`, the one of `turn`, on its own thread, one at
/// a time. It plays once rendered, unless the program was edited, the
/// preview stopped, or the bridge closed meanwhile. A stop or an exit sent
/// after it and here before it keeps it from starting.
fn spawn_render(
    request: RenderRequest,
    turn: u64,
    client: &RuntimeClient,
    tx: &Outbox,
    shared: &Arc<Shared>,
) {
    if shared.preview_turns.stopped(turn) {
        preview_unstarted(shared, tx, turn, request.proposal);
        return;
    }
    if shared.preview_busy.swap(true, Ordering::SeqCst) {
        error(tx, "A preview is already rendering");
        preview_unstarted(shared, tx, turn, request.proposal);
        return;
    }
    let client = client.clone();
    let tx = tx.clone();
    let shared = shared.clone();
    thread::spawn(move || {
        let revision = shared.revision.load(Ordering::SeqCst);
        let result = export(&client, &request, turn, &tx, &shared);
        let rendered = result.as_ref().ok().and_then(|(output, document)| {
            let program = kerf_runtime::parse_document(document).ok()?;
            let project = rendered_preview(&program, output, request.range.as_ref())?;
            Some((project, PathBuf::from(output)))
        });
        // Whether it stopped is settled with the viewer's slot held, which
        // `preview_exit` takes after asking for the stop: an exit sent before
        // this keeps the preview out, and one sent after finds it waiting
        // there and puts the program back.
        let stopped = {
            let slot = shared.project.lock();
            let stopped = shared.preview_turns.over(turn);
            // A preview of a program edited since, one stopped, or one for a
            // closed bridge, is never shown.
            let shown = match (&rendered, slot) {
                (Some((project, file)), Ok(mut slot))
                    if !stopped
                        && !shared.closed.load(Ordering::SeqCst)
                        && shared.revision.load(Ordering::SeqCst) == revision =>
                {
                    let project = MediaProject::Rendered(
                        project.clone(),
                        file.clone(),
                        request.proposal.clone(),
                    );
                    install_project(&shared, &mut slot, project);
                    true
                }
                _ => false,
            };
            if !shown && let Ok((output, _)) = &result {
                discard_preview(&shared, PathBuf::from(output));
            }
            stopped
        };
        match result {
            Ok(_) => {}
            Err(err) if stopped && err.code == ErrorCode::Cancelled => {}
            Err(err) => {
                error(&tx, err.message);
                emit(
                    &tx,
                    Event::Preview(Preview {
                        message: Some(
                            "Preview render failed. Retry after fixing the reported error."
                                .to_owned(),
                        ),
                        ..request.event(PreviewStatus::Required)
                    }),
                );
            }
        }
        // The answer to the stop, however the render ended.
        if stopped {
            emit(&tx, Event::Preview(request.event(PreviewStatus::Stopped)));
        }
        shared.preview_busy.store(false, Ordering::SeqCst);
    });
}

/// Render the program. Returns the output path and the document rendered.
/// Closing the bridge or stopping the preview cancels a preview, and a
/// preview that fails leaves no file behind.
fn export(
    client: &RuntimeClient,
    request: &RenderRequest,
    turn: u64,
    tx: &Outbox,
    shared: &Shared,
) -> Result<(String, String), kerf_runtime::RuntimeError> {
    let preview = request.preview;
    let (job, document, output) = client.start_render_range(
        &request.quality,
        request.range.clone(),
        request.proposal.as_deref(),
        request.sequence.as_deref(),
    )?;
    let cancel = kerf_runtime::render_cancel_path(client.directory(), &job.job_id);
    if preview {
        // The viewer's render, not an output.
        shared.renders.mark_viewer(&job.job_id);
        if let Ok(mut running) = shared.preview_cancel.lock() {
            *running = Some(cancel.clone());
        }
        // The bridge may have closed, or the preview stopped, before it
        // could see this render.
        if shared.closed.load(Ordering::SeqCst) || shared.preview_turns.stopped(turn) {
            cancel_preview(shared);
        }
    }
    let mut running = request.event(PreviewStatus::Running);
    running.progress = Some(0.0);
    emit(tx, Event::Preview(running.clone()));
    // Its frames are drawn in this process, so it begins on the bridge's drawers and holds
    // them until it is over: the bridge cancels it as it closes, and whoever is about to
    // exit waits for it to stop. For a bridge let go already it is not begun.
    let result = match shared.drawers.begin() {
        Some(_drawing) => kerf_runtime::run_render_job(
            client,
            &job,
            &document,
            &output,
            request.quality == "preview",
            |progress| {
                // Progress is replaceable; never stall rendering behind a busy GUI.
                running.progress = Some(progress);
                let _ = tx.try_send(Event::Preview(running.clone()));
            },
        ),
        None => Err(kerf_runtime::RuntimeError::new(
            ErrorCode::Cancelled,
            "render job was cancelled",
        )),
    };
    if preview && let Ok(mut running) = shared.preview_cancel.lock() {
        *running = None;
        // A cancel that came as the render finished outlives the job.
        let _ = std::fs::remove_file(&cancel);
    }
    match result {
        Ok(()) => {
            let path = output.to_string_lossy().into_owned();
            if !preview {
                emit(
                    tx,
                    Event::Preview(Preview {
                        path: Some(path.clone()),
                        progress: Some(1.0),
                        ..request.event(PreviewStatus::Completed)
                    }),
                );
            }
            Ok((path, document))
        }
        Err(err) => {
            if preview {
                discard_preview(shared, output);
            }
            if !(err.code == ErrorCode::Cancelled && shared.preview_turns.stopped(turn)) {
                emit(tx, Event::Preview(request.event(PreviewStatus::Failed)));
            }
            Err(err)
        }
    }
}

/// `preview_stop`: stop every preview sent before it, the one rendering
/// and one still on its way to its render alike. Each says `stopped` when
/// it is over; with none to stop the answer comes at once.
///
/// Which previews it stops is settled where the stop is sent, as that is
/// what orders it among them; the rest runs on a thread of its own, since
/// the events queue may be full while the editor is sending.
pub(crate) fn stop_preview(shared: &Arc<Shared>, tx: &Outbox) {
    let answered = shared.preview_turns.stop();
    let shared = shared.clone();
    let tx = tx.clone();
    thread::spawn(move || stopped_preview(&shared, answered, &tx));
}

/// What follows a stop: the render running is told, and a stop that no
/// preview answers for (`answered`) answers itself.
fn stopped_preview(shared: &Shared, answered: bool, tx: &Outbox) {
    cancel_preview(shared);
    if !answered {
        emit(
            tx,
            Event::Preview(Preview {
                status: PreviewStatus::Stopped,
                ..Preview::default()
            }),
        );
    }
}

/// `preview_exit`: stop every preview sent before it, as `preview_stop`
/// does, and play the program again in place of a rendered preview, which
/// the media worker announces with `viewer_reset`.
pub(crate) fn exit_preview(shared: &Arc<Shared>, tx: &Outbox) {
    shared.preview_turns.stop();
    let shared = shared.clone();
    let tx = tx.clone();
    thread::spawn(move || exited_preview(&shared, &tx));
}

/// What follows an exit, once its previews are asked to stop.
fn exited_preview(shared: &Shared, tx: &Outbox) {
    cancel_preview(shared);
    let model = shared.model.lock().ok().and_then(|model| model.clone());
    if let (Some(model), Ok(mut slot)) = (model, shared.project.lock()) {
        // A preview that was not stopped in time is waiting in the slot or
        // showing: both are looked at with the slot held, as a preview moves
        // from one to the other and into the slot with it held.
        let showing = shared
            .shown_preview
            .lock()
            .is_ok_and(|shown| shown.is_some());
        let waiting = matches!(*slot, Some(MediaProject::Rendered(..)));
        if showing || waiting {
            // A preview finishing now is of the program as it was.
            shared.revision.fetch_add(1, Ordering::SeqCst);
            install_project(shared, &mut slot, MediaProject::Native(model));
        }
    }
    emit(
        tx,
        Event::Preview(Preview {
            status: PreviewStatus::Exited,
            ..Preview::default()
        }),
    );
}

/// Stop the preview render running, if there is one.
pub(crate) fn cancel_preview(shared: &Shared) {
    if let Ok(running) = shared.preview_cancel.lock()
        && let Some(marker) = &*running
        && let Err(error) = std::fs::write(marker, b"")
    {
        log::warn!(
            "the preview render goes on: {} was not written: {error}",
            marker.display()
        );
    }
}

/// Put `project` in the media slot. A rendered preview it replaces there was
/// never opened, so its file goes now.
pub(crate) fn install_project(
    shared: &Shared,
    slot: &mut Option<MediaProject>,
    project: MediaProject,
) {
    if let Some(MediaProject::Rendered(_, file, _)) = slot.replace(project) {
        discard_preview(shared, file);
    }
}

/// Remove the preview the media worker plays and one still waiting for it:
/// the bridge has closed, so nothing will show either.
pub(crate) fn discard_previews(shared: &Shared) {
    let shown = shared
        .shown_preview
        .lock()
        .ok()
        .and_then(|mut shown| shown.take());
    if let Some(file) = shown {
        discard_preview(shared, file);
    }
    let waiting = shared.project.lock().ok().and_then(|mut slot| {
        if matches!(*slot, Some(MediaProject::Rendered(..))) {
            slot.take()
        } else {
            None
        }
    });
    if let Some(MediaProject::Rendered(_, file, _)) = waiting {
        discard_preview(shared, file);
    }
}

/// Remove a preview render nothing shows any more, or keep it for the media
/// worker to try again while it is still open. Only previews come here;
/// exports are the person's.
pub(crate) fn discard_preview(shared: &Shared, file: PathBuf) {
    if !remove_preview(&file)
        && let Ok(mut stale) = shared.stale_previews.lock()
    {
        stale.push(file);
    }
}

/// Try the previews not removed yet again; `true` once none are left.
pub(crate) fn sweep_previews(shared: &Shared) -> bool {
    shared.stale_previews.lock().map_or(true, |mut stale| {
        stale.retain(|file| !remove_preview(file));
        stale.is_empty()
    })
}

/// Remove a preview file; `false` while something still holds it open.
fn remove_preview(file: &Path) -> bool {
    match std::fs::remove_file(file) {
        Ok(()) => true,
        Err(err) => err.kind() == std::io::ErrorKind::NotFound,
    }
}

/// A project that plays a rendered preview of `program` in its place: all
/// of it, or the `range` rendered, at its own place on the program.
fn rendered_preview(
    program: &Project,
    output: &str,
    range: Option<&kerf_runtime::RenderRange>,
) -> Option<Project> {
    let sequence = program.sequences.first()?;
    let (start, duration) = match range {
        Some(range) => (
            Time::from_seconds(range.start)?,
            Time::from_seconds(range.duration)?,
        ),
        None => (Time::ZERO, sequence.duration()),
    };
    let id = |text: &str| Id::new(text).ok();
    let media = id("preview")?;
    let clip = |name: &str| {
        Some(Clip::new(
            id(name)?,
            Content::Media {
                media: media.clone(),
                source_in: Time::ZERO,
                speed: Ratio::ONE.into(),
            },
            start,
            duration,
        ))
    };
    let mut picture = Track::new(id("V1")?, TrackKind::Video);
    picture.clips.push(clip("preview-picture")?);
    let mut sound = Track::new(id("A1")?, TrackKind::Audio);
    sound.clips.push(clip("preview-sound")?);
    Some(Project {
        name: program.name.clone(),
        media: vec![Media {
            id: media,
            path: output.to_owned(),
            rate: sequence.rate,
            drop_frame: false,
            start: Time::ZERO,
            duration,
            video: Some(VideoInfo {
                width: sequence.width,
                height: sequence.height,
                alpha: false,
                ..Default::default()
            }),
            audio: Some(AudioInfo {
                channels: 2,
                sample_rate: 48_000,
            }),
            reel: None,
            recipe: None,
            steps: Vec::new(),
            sync: None,
        }],
        sequences: vec![Sequence {
            id: id("program")?,
            name: String::new(),
            tracks: vec![picture, sound],
            markers: Vec::new(),
            start: Time::ZERO,
            ..sequence.clone()
        }],
        retired: Default::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::{discard_previews, rendered_preview, MediaProject, Shared};

    use serde_json::json;
    use std::path::Path;
    use std::sync::atomic::Ordering;
    use std::sync::mpsc;
    use std::sync::Arc;
    use std::time::Duration;

    fn shared() -> Shared {
        crate::tests_support::shared(&std::env::temp_dir())
    }

    /// A rendered preview of a one-cut program, playing `file`.
    fn rendered(file: &Path) -> MediaProject {
        let program = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Bridge\"\n\
             media m1 path=\"/media/a.mov\" rate=24 dur=10s video=1920x1080\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip c1 media=m1 src=0f at=0f dur=60f\n",
        )
        .unwrap();
        let project = rendered_preview(&program, file.to_str().unwrap(), None).unwrap();
        MediaProject::Rendered(project, file.to_owned(), None)
    }

    #[test]
    fn closing_removes_the_previews_without_waiting_for_the_media_worker() {
        let directory = tempfile::tempdir().unwrap();
        let shared = crate::tests_support::shared(directory.path());
        let playing = directory.path().join("render-1.mp4");
        let waiting = directory.path().join("render-2.mp4");
        std::fs::write(&playing, b"preview").unwrap();
        std::fs::write(&waiting, b"preview").unwrap();
        *shared.shown_preview.lock().unwrap() = Some(playing.clone());
        *shared.project.lock().unwrap() = Some(rendered(&waiting));

        discard_previews(&shared);
        assert!(!playing.exists() && !waiting.exists());
        assert!(shared.shown_preview.lock().unwrap().is_none());
        assert!(shared.project.lock().unwrap().is_none());
    }

    /// The next event as JSON, from a thread that may not have run yet.
    fn next(rx: &mpsc::Receiver<super::Event>) -> serde_json::Value {
        serde_json::to_value(rx.recv_timeout(Duration::from_secs(10)).unwrap()).unwrap()
    }

    fn preview_request(proposal: &str) -> super::RenderRequest {
        super::RenderRequest {
            quality: "preview".into(),
            preview: true,
            proposal: Some(proposal.into()),
            range: None,
            sequence: None,
        }
    }

    #[test]
    fn stopping_with_no_preview_running_answers_at_once() {
        let shared = Arc::new(shared());
        let (tx, rx) = crate::wake::outbox(4);
        super::stop_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));

        // With one running, the render answers once it has stopped.
        let directory = tempfile::tempdir().unwrap();
        let marker = kerf_runtime::render_cancel_path(directory.path(), "job-1");
        *shared.preview_cancel.lock().unwrap() = Some(marker.clone());
        assert!(shared.preview_turns.send(|| true));
        let turn = shared.preview_turns.take();
        super::stop_preview(&shared, &tx);
        crate::tests_support::wait(Duration::from_secs(10), || marker.exists().then_some(()))
            .expect("the render was told to stop");
        assert!(shared.preview_turns.stopped(turn));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
        // It is over, stopped, and the next stop has nothing to wait for.
        assert!(shared.preview_turns.over(turn));
        super::stop_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));
    }

    /// The editor sends `preview` and then `preview_stop`; the stop acts
    /// where it is sent and the preview waits for the runtime worker, so the
    /// stop is first. The preview must not start, and answers the stop.
    #[test]
    fn a_stop_that_gets_ahead_of_its_preview_keeps_it_from_starting() {
        let directory = tempfile::tempdir().unwrap();
        let shared = Arc::new(crate::tests_support::shared(directory.path()));
        let client = kerf_runtime::RuntimeClient::new(directory.path()).unwrap();
        let (tx, rx) = crate::wake::outbox(8);
        assert!(shared.preview_turns.send(|| true));
        super::stop_preview(&shared, &tx);
        let turn = shared.preview_turns.take();
        super::spawn_render(preview_request("proposal-7"), turn, &client, &tx, &shared);
        assert_eq!(
            next(&rx),
            json!({"type": "preview", "status": "stopped", "proposalId": "proposal-7"})
        );
        assert!(
            !shared.preview_busy.load(Ordering::SeqCst),
            "no render began"
        );
        assert!(
            rx.recv_timeout(Duration::from_millis(200)).is_err(),
            "one stop, one answer"
        );

        // The same with an exit, which also answers for itself.
        assert!(shared.preview_turns.send(|| true));
        super::exit_preview(&shared, &tx);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "exited"}));
        let turn = shared.preview_turns.take();
        super::spawn_render(preview_request("proposal-8"), turn, &client, &tx, &shared);
        assert_eq!(
            next(&rx),
            json!({"type": "preview", "status": "stopped", "proposalId": "proposal-8"})
        );
        assert!(
            !shared.preview_busy.load(Ordering::SeqCst),
            "no render began"
        );

        // A preview the runtime worker refuses answers the stop too.
        assert!(shared.preview_turns.send(|| true));
        super::stop_preview(&shared, &tx);
        let turn = shared.preview_turns.take();
        super::preview_unstarted(&shared, &tx, turn, None);
        assert_eq!(next(&rx), json!({"type": "preview", "status": "stopped"}));
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }
}
