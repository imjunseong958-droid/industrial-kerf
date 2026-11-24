//! A shot's takes held to its guide, for the editor's graph to show beside
//! each and to say which holds best ([`kerf_shots::takes`], the reading an
//! agent's `look` makes with `takes`).
//!
//! Reading one draws the guide's sequence and follows the subject through
//! the take, which is seconds a take, so it is done off the worker that
//! answers the editor: `take_read` as each is read, then `takes_read`.
//! Each take begins on the bridge's drawers (`Shared::drawers`): once the
//! bridge is let go no other is read.

use crate::event::{ErrorEvent, Event, TakeRead, TakesRead};
use crate::wake::Outbox;
use crate::Shared;
use kerf_engine::recipe::takes::Take;
use kerf_shots::takes::Guide;
use std::sync::Arc;

fn failed(tx: &Outbox, id: &str, why: String) {
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
    tx: &Outbox,
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
        read_each(&shared, &project, &id, &takes, &guide, revision, &tx);
    });
}

/// Read each of `takes` of media `id` against `guide` and say it, then say which holds best.
/// `project`, as it was at `revision`, has its media paths resolved.
fn read_each(
    shared: &Shared,
    project: &kerf_model::Project,
    id: &str,
    takes: &[Take],
    guide: &Guide,
    revision: u64,
    tx: &Outbox,
) {
    let mut readings = Vec::new();
    for take in takes {
        let reading = {
            // A take at a time: the takes not begun when the bridge is let go are left
            // unread, and what is about to exit waits for the one being read.
            let Some(_drawing) = shared.drawers.begin() else {
                return;
            };
            kerf_shots::takes::read(project, &shared.directory, id, take, guide)
        };
        crate::emit(
            tx,
            Event::TakeRead(TakeRead {
                take: take.number,
                path: take.stored.clone(),
                chosen: take.chosen,
                // A mean that is not a number was written as null; it stays so.
                overlap: reading["overlap"].as_f64().unwrap_or(f64::NAN),
                clips: reading["clips"].as_array().cloned().unwrap_or_default(),
                media: id.to_owned(),
                revision,
            }),
        );
        readings.push(reading);
    }
    let done = kerf_shots::takes::summary(guide, &readings);
    crate::emit(
        tx,
        Event::TakesRead(TakesRead {
            sequence: guide.sequence.clone(),
            clips: guide.clips.clone(),
            best: done["best"]
                .as_u64()
                .and_then(|best| u32::try_from(best).ok()),
            says: done["says"].as_str().unwrap_or_default().to_owned(),
            media: id.to_owned(),
            revision,
            takes: readings.len(),
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    #[test]
    fn a_closed_bridge_reads_no_take() {
        // A guide that needs no media to draw; the take itself is not there to follow, which
        // a reading says of it.
        let project = kerf_runtime::parse_document(
            "kerf 1\nproject name=\"Takes\"\n\
             media m1 path=\"shots/open.mp4\" rate=24 dur=12f video=64x64\n\
             sequence s1 rate=24 size=64x64 audio=48000\n\
             \x20 track V1 video\n\
             \x20   clip g1 gen=solid@1 at=0f dur=12f\n",
        )
        .expect("the document");
        let guide = Guide {
            sequence: "s1".to_owned(),
            clips: vec!["g1".to_owned()],
        };
        let takes: Vec<Take> = (1..=2)
            .map(|number| Take {
                number,
                stored: format!("shots/open.takes/{number:03}.mp4"),
                made: None,
                chosen: number == 2,
                changed: Vec::new(),
                current: true,
                rolled: 0,
                kept: false,
            })
            .collect();
        let shared = crate::tests_support::shared(&std::env::temp_dir().join("kerf-no-takes"));
        let (tx, rx) = crate::wake::outbox(8);
        let said = || -> Vec<Value> {
            read_each(&shared, &project, "m1", &takes, &guide, 7, &tx);
            rx.try_iter()
                .map(|event| serde_json::to_value(event).expect("an event writes as JSON"))
                .collect()
        };
        // Open, each take is read and said, and then which holds best.
        let read = said();
        let types: Vec<&str> = read
            .iter()
            .filter_map(|event| event["type"].as_str())
            .collect();
        assert_eq!(types, ["take_read", "take_read", "takes_read"], "{read:#?}");
        assert_eq!(read[1]["take"], 2);
        // Its drawers closed, as the bridge closes them when it is let go, none is.
        shared.drawers.close();
        assert_eq!(said(), Vec::<Value>::new());
    }
}
