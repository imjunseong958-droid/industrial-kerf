//! What the clips of a guide say they are (`prompt=` on a clip), for a model that reads it.
//!
//! A step's prompt says what the whole shot is, and a guide of plain solids does not say which
//! solid is which: handed two like boxes and "a red car and a white van", a model made both
//! red and white. A clip of the guide's sequence that says `prompt="a red car"` is handed over
//! with its own words and a mask of where the guide shows it, frame by frame, as the guide is
//! drawn, so the model is told what is where. Clips that say the same words are one region.

use kerf_model::making::{Ask, Source, Step};
use kerf_model::{Project, TrackKind};

/// Words clips of a guide say they are, and the clips that say them, by their ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Said {
    pub prompt: String,
    pub clips: Vec<String>,
}

/// The `render` step of `steps` that draws the guide `ask` hands its model, where a step of
/// them draws it.
pub(crate) fn guide_drawn<'a>(steps: &'a [Step], ask: &Ask) -> Option<&'a Step> {
    let Some(Source::Made(name)) = &ask.guide else {
        return None;
    };
    steps
        .iter()
        .find(|step| matches!(step, Step::Render { to, .. } if to == name))
}

/// What the clips of a guide of `sequence` say they are, as the guide draws them: of only what
/// a model films where `only_filmed` says so. Each prompt once, with the clips that say it, in
/// the order the first of them is told. A clip says nothing to a model where the guide does not
/// draw it: one that is off, on a track that is off, that draws no picture (a camera, a light),
/// or that a model does not film in a guide of only what one films. A nested sequence is drawn
/// as one clip, so what its own clips say is not heard.
pub fn said(project: &Project, sequence: &str, only_filmed: bool) -> Vec<Said> {
    let Some(sequence) = project.sequence(sequence) else {
        return Vec::new();
    };
    let mut said: Vec<Said> = Vec::new();
    let tracks = sequence
        .tracks
        .iter()
        .filter(|track| track.enabled && track.kind == TrackKind::Video);
    for track in tracks {
        for clip in &track.clips {
            let drawn = clip.enabled && clip.draws() && (!only_filmed || clip.filmed());
            let Some(prompt) = clip.prompt().filter(|_| drawn) else {
                continue;
            };
            let id = clip.id.to_string();
            match said.iter_mut().find(|region| region.prompt == prompt) {
                Some(region) => region.clips.push(id),
                None => said.push(Said {
                    prompt: prompt.to_owned(),
                    clips: vec![id],
                }),
            }
        }
    }
    said
}
