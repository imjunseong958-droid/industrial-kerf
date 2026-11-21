//! A shot's takes held to its guide, for the editor's graph to show beside
//! each and to say which holds best ([`kerf_shots::takes`], the reading an
//! agent's `look` makes with `takes`).
//!
//! Reading one draws the guide's sequence and follows the subject through
//! the take, which is seconds a take, so it is done off the worker that
//! answers the editor: `take_read` as each is read, then `takes_read`.

use crate::event::{ErrorEvent, Event, TakeRead, TakesRead};
use crate::Shared;
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

fn failed(tx: &SyncSender<Event>, id: &str, why: String) {
    crate::emit(
        tx,
        Event::Error(ErrorEvent {
            message: why,
            command: Some("takes_read".to_owned()),
            media: Some(id.to_owned()),
            ..ErrorEvent::default()
        }),
    );
}

/// Answer `takes_read` for media `id`.
pub(crate) fn read(
    shared: &Arc<Shared>,
    client: &kerf_runtime::RuntimeClient,
    id: String,
    tx: &SyncSender<Event>,
) {
    let directory = shared.directory.clone();
    let asked = client
        .get_document(false)
        .and_then(|(revision, text)| Ok((revision, kerf_runtime::parse_document(&text)?)))
        .map_err(|error| error.message)
        .and_then(|(revision, project)| {
            let guide = kerf_shots::takes::guide(&project, &id)?;
            let takes = kerf_shots::takes::takes(&project, &directory, &id)?;
            Ok((revision, project, guide, takes))
        });
    let (revision, mut project, guide, takes) = match asked {
        Ok(asked) => asked,
        Err(why) => return failed(tx, &id, why),
    };
    let tx = tx.clone();
    let shared = shared.clone();
    std::thread::spawn(move || {
        kerf_runtime::resolve_media_paths(&mut project, &directory);
        let mut readings = Vec::new();
        for take in &takes {
            if shared.closed.load(std::sync::atomic::Ordering::SeqCst) {
                return;
            }
            let reading = kerf_shots::takes::read(&project, &directory, &id, take, &guide);
            crate::emit(
                &tx,
                Event::TakeRead(TakeRead {
                    take: take.number,
                    path: take.stored.clone(),
                    chosen: take.chosen,
                    // A mean that is not a number was written as null; it stays so.
                    overlap: reading["overlap"].as_f64().unwrap_or(f64::NAN),
                    clips: reading["clips"].as_array().cloned().unwrap_or_default(),
                    media: id.clone(),
                    revision,
                }),
            );
            readings.push(reading);
        }
        let done = kerf_shots::takes::summary(&guide, &readings);
        crate::emit(
            &tx,
            Event::TakesRead(TakesRead {
                sequence: guide.sequence.clone(),
                clips: guide.clips.clone(),
                best: done["best"]
                    .as_u64()
                    .and_then(|best| u32::try_from(best).ok()),
                says: done["says"].as_str().unwrap_or_default().to_owned(),
                media: id.clone(),
                revision,
                takes: readings.len(),
            }),
        );
    });
}
