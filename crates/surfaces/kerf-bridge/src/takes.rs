//! A shot's takes held to its guide, for the editor's graph to show beside
//! each and to say which holds best ([`kerf_shots::takes`], the reading an
//! agent's `look` makes with `takes`).
//!
//! Reading one draws the guide's sequence and follows the subject through
//! the take, which is seconds a take, so it is done off the worker that
//! answers the editor: `take_read` as each is read, then `takes_read`.

use crate::Shared;
use serde_json::{json, Value};
use std::sync::mpsc::SyncSender;
use std::sync::Arc;

fn failed(tx: &SyncSender<Value>, id: &str, why: String) {
    crate::emit(
        tx,
        json!({"type": "error", "message": why, "command": "takes_read", "media": id}),
    );
}

/// Answer `takes_read` for media `id`.
pub(crate) fn read(
    shared: &Arc<Shared>,
    client: &kerf_runtime::RuntimeClient,
    id: String,
    tx: &SyncSender<Value>,
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
            let mut reading = kerf_shots::takes::read(&project, &directory, &id, take, &guide);
            readings.push(reading.clone());
            reading["type"] = json!("take_read");
            reading["media"] = json!(id);
            reading["revision"] = json!(revision);
            crate::emit(&tx, reading);
        }
        let mut done = kerf_shots::takes::summary(&guide, &readings);
        done["type"] = json!("takes_read");
        done["media"] = json!(id);
        done["revision"] = json!(revision);
        done["takes"] = json!(readings.len());
        crate::emit(&tx, done);
    });
}
