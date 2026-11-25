//! What a shot hands a model, read from footage, for the editor's graph
//! ([`kerf_shots::tracks`], which an agent's `look` follows with `track`): a
//! clip that follows a point of the picture, for `moves=`, and a mask that
//! holds to what a clip shows, for `matte=`.
//!
//! The person points on a picture of the clip's first frame, so that is
//! drawn first (`shot_picture`). Following is a frame at a time through the
//! clip, seconds of it, so it is done off the worker that answers the
//! editor, and its edit is one of the project like any other: `shot_followed`
//! once it is in, or `error`.
//!
//! Each frame drawn for either begins on the bridge's drawers (`Shared::drawers`). Once the
//! bridge is let go no more are, and a clip half followed makes no edit and no answer.

use crate::event::{ErrorEvent, Event};
use crate::wake::Outbox;
use crate::CommandName;
use crate::Shared;
use kerf_shots::tracks::For;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// What to read a clip from footage for, as the editor names it.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Of {
    Moves,
    Matte,
}

impl Of {
    fn word(self) -> &'static str {
        match self {
            Self::Moves => "moves",
            Self::Matte => "matte",
        }
    }

    fn what(self) -> For {
        match self {
            Self::Moves => For::Moves,
            Self::Matte => For::Matte,
        }
    }
}

/// What the person pointed at: for `moves` the one point the clip follows, and for `matte`
/// points on what to cut out, with points off it.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Follow {
    pub clip: String,
    pub of: Of,
    pub points: Vec<[f64; 2]>,
    #[serde(default)]
    pub outside: Vec<[f64; 2]>,
}

fn failed(tx: &Outbox, command: CommandName, clip: &str, of: Of, why: String) {
    crate::emit(
        tx,
        Event::Error(ErrorEvent {
            message: why,
            command: Some(command),
            clip: Some(clip.to_owned()),
            of: Some(of),
            ..ErrorEvent::default()
        }),
    );
}

/// The project as it reads now, its media saying where their files are, and its revision.
fn opened(
    shared: &Shared,
    client: &kerf_runtime::RuntimeClient,
) -> Result<(u64, kerf_model::Project), String> {
    let (revision, text) = client.get_document(false).map_err(|error| error.message)?;
    let mut project = kerf_runtime::parse_document(&text).map_err(|error| error.message)?;
    kerf_runtime::resolve_media_paths(&mut project, &shared.directory);
    Ok((revision, project))
}

/// Answer `shot_picture`: the picture to point on, as a file the editor reads.
pub(crate) fn picture(
    shared: &Arc<Shared>,
    client: &kerf_runtime::RuntimeClient,
    clip: String,
    of: Of,
    tx: &Outbox,
) {
    let (shared, client, tx) = (shared.clone(), client.clone(), tx.clone());
    shared.workers.clone().spawn("bridge:picture", move || {
        let drawn = opened(&shared, &client).and_then(|(revision, project)| {
            let frame = {
                // The bridge was let go: nobody is left to point on a picture.
                let Some(_drawing) = shared.drawers.begin() else {
                    return Ok(None);
                };
                kerf_shots::tracks::picture(&project, &clip, of.what())?
            };
            let path = std::env::temp_dir().join(format!(
                "kerf-shot-picture-{}-{}.png",
                std::process::id(),
                of.word()
            ));
            kerf_engine::render::still::write_png(&frame, &path)
                .map_err(|error| error.to_string())?;
            Ok(Some(Event::ShotPicture {
                clip: clip.clone(),
                of,
                path,
                width: frame.width,
                height: frame.height,
                revision,
            }))
        });
        match drawn {
            Ok(Some(event)) => crate::emit(&tx, event),
            Ok(None) => {}
            Err(why) => failed(&tx, CommandName::ShotPicture, &clip, of, why),
        }
    });
}

/// Answer `shot_follow`: follow what was pointed at, and set the clip by it.
pub(crate) fn follow(
    shared: &Arc<Shared>,
    client: &kerf_runtime::RuntimeClient,
    asked: Follow,
    tx: &Outbox,
) {
    let (shared, client, tx) = (shared.clone(), client.clone(), tx.clone());
    shared.workers.clone().spawn("bridge:follow", move || {
        let Follow {
            clip,
            of,
            points,
            outside,
        } = asked;
        let done = opened(&shared, &client).and_then(|(revision, project)| {
            // A frame at a time: the frames not begun when the bridge is let go are left.
            let drawing = || shared.drawers.begin();
            let read = match (of, points.as_slice()) {
                (Of::Moves, [point]) => kerf_shots::tracks::moves(&project, &clip, *point, drawing),
                (Of::Moves, _) => Err("a clip follows one point".into()),
                (Of::Matte, inside) => {
                    kerf_shots::tracks::matte(&project, &clip, inside, &outside, drawing)
                }
            }?;
            // Followed part of its way for a bridge let go meanwhile: no edit is made of that.
            let Some(read) = read else {
                return Ok(None);
            };
            client
                .apply_script_request("ui", &crate::key(), &read.script, revision)
                .map_err(|error| error.message)?;
            Ok(Some(read))
        });
        match done {
            Ok(Some(read)) => crate::emit(
                &tx,
                Event::ShotFollowed {
                    clip,
                    of,
                    frames: read.frames,
                    keyframes: read.keyframes,
                    lost_at: read.lost_at,
                    says: read.says(),
                },
            ),
            Ok(None) => {}
            Err(why) => failed(&tx, CommandName::ShotFollow, &clip, of, why),
        }
    });
}
