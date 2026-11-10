//! How an effect runs on a clip's picture. A crate implements
//! [`PictureEffect`] for each effect it has and lists them; the compositor
//! finds them by id, so a new effect changes that crate and the catalog, not
//! the compositor. Every effect the compositor runs comes through here,
//! whichever crate it is in and wherever its code came from.

use crate::{Affine, Image};
use kerf_color::WorkingSpace;
use kerf_model::{Catalog, Effect, Mask, Time, Value};
use std::collections::BTreeMap;
use std::sync::Arc;

/// The files a program's effects name, read once before anything is drawn,
/// by the path a parameter gives.
#[derive(Clone, Debug, Default)]
pub struct Files(BTreeMap<String, Arc<str>>);

impl Files {
    pub const fn new() -> Self {
        Self(BTreeMap::new())
    }

    /// The text of the file at `path`, when an effect had it read.
    pub fn text(&self, path: &str) -> Option<&Arc<str>> {
        self.0.get(path)
    }

    pub fn insert(&mut self, path: impl Into<String>, text: Arc<str>) {
        self.0.insert(path.into(), text);
    }
}

/// What an effect may read beyond its picture and its parameters: the
/// clip's other frames, and the rest of the sequence. What a host cannot
/// give reads as nothing.
pub trait EffectHost: Sync {
    /// The clip's picture at clip time `at` as the effect at `before` among
    /// its effects gets it: masked, with the effects before it run, the size
    /// of the picture the effect runs on.
    fn picture_at(&self, at: Time, before: usize) -> Option<Image>;

    /// The clip's picture alone at clip time `at`, at `scale` of its source
    /// size, for an effect that measures its footage.
    fn source_at(&self, _at: Time, _scale: f32) -> Option<Image> {
        None
    }

    /// Picture track `id` drawn alone at this moment, the size of the
    /// output.
    fn track(&self, _id: &str) -> Option<Image> {
        None
    }

    /// `frames` stereo frames (interleaved) of sound track `track` from
    /// sequence time `start` seconds, at `rate` Hz.
    fn sound(&self, _track: &str, _start: f64, _frames: usize, _rate: u32) -> Option<Vec<f32>> {
        None
    }

    /// The picture's pixels to the output's, the clip's motion included.
    fn to_output(&self) -> Affine {
        Affine::IDENTITY
    }

    /// The timecode of the clip's source now.
    fn source_timecode(&self) -> String {
        String::new()
    }

    fn sequence_name(&self) -> String {
        String::new()
    }

    /// The sequence's frame size, at full size.
    fn sequence_size(&self) -> (u32, u32) {
        (0, 0)
    }

    /// The clip's source size, at full size.
    fn source_size(&self) -> (u32, u32) {
        (0, 0)
    }

    /// What says which clip this is and which of its source it shows, for
    /// an effect that keeps what it measured.
    fn clip_key(&self) -> u64 {
        0
    }
}

/// Where and when an effect runs.
#[derive(Clone, Copy)]
pub struct EffectRun<'a> {
    /// Clip time.
    pub at: Time,
    /// Sequence time.
    pub sequence_at: Time,
    /// How many of the picture's pixels a source pixel is.
    pub per_source: f64,
    /// How many of the picture's pixels a pixel of the sequence's canvas is.
    pub per_canvas: f64,
    /// The effect's place among the clip's effects.
    pub index: usize,
    /// Whether the picture is drawn for a later effect of the clip, which
    /// reads the clip's other frames.
    pub nested: bool,
    /// Makes random effects differ between clips and stay the same between
    /// renders.
    pub seed: u32,
    /// The clip's masks, which effects read as paths.
    pub masks: &'a [Mask],
    pub frame_rate: f64,
    /// How long the clip is.
    pub duration: Time,
    /// The sequence's working color space, which a grade works in.
    pub working: WorkingSpace,
    /// The sequence's timecode now, for burn-ins.
    pub timecode: &'a str,
    pub clip_name: &'a str,
    /// The sequence's picture tracks, by id, from the lowest.
    pub tracks: &'a [String],
    /// The sequence's sound tracks, by id.
    pub sounds: &'a [String],
    /// The files the program's effects had read ([`PictureEffect::prepare`]).
    pub files: &'a Files,
    /// None where the effect runs without a clip of its own, as on an
    /// adjustment layer.
    pub host: Option<&'a dyn EffectHost>,
}

impl EffectRun<'_> {
    /// A parameter's value at the run's time, or its default.
    fn value(&self, effect: &Effect, name: &str) -> Option<Value> {
        match effect.params.get(name) {
            Some(param) => Some(param.value_at(self.at)),
            None => Catalog::builtin()
                .fx(effect.fx.as_str())
                .and_then(|def| def.find(name))
                .map(|def| def.default.clone()),
        }
    }

    /// A number parameter, or 0 when it has none.
    pub fn number(&self, effect: &Effect, name: &str) -> f64 {
        self.value(effect, name)
            .and_then(|value| value.as_number())
            .unwrap_or(0.0)
    }

    /// A text, choice, or mask parameter, or `None` when it is empty.
    pub fn text(&self, effect: &Effect, name: &str) -> Option<String> {
        match self.value(effect, name)? {
            Value::Text(text) | Value::Enum(text) if !text.is_empty() => Some(text),
            _ => None,
        }
    }

    /// The clip's other frames, as this effect gets them, for an effect
    /// that does without them in a picture drawn for another effect.
    pub fn frames(&self) -> Option<Frames<'_>> {
        let host = self.host.filter(|_| !self.nested)?;
        Some(Frames {
            host,
            before: self.index,
        })
    }
}

/// The clip's pictures at other times, as one of its effects gets them.
#[derive(Clone, Copy)]
pub struct Frames<'a> {
    host: &'a dyn EffectHost,
    before: usize,
}

impl Frames<'_> {
    /// The clip's picture at clip time `at`.
    pub fn at(&self, at: Time) -> Option<Image> {
        self.host.picture_at(at, self.before)
    }
}

/// An effect drawn on a clip's picture, premultiplied linear light, and
/// masked by the clip's cutting masks.
pub trait PictureEffect: Sync {
    /// Its id in the catalog, as a document names it.
    fn id(&self) -> &'static str;

    /// Read the files `effect` names into `files`, before anything is
    /// drawn. A file that cannot be read is an error naming it.
    fn prepare(&self, _effect: &Effect, _files: &mut Files) -> Result<(), String> {
        Ok(())
    }

    fn apply(&self, picture: &mut Image, effect: &Effect, run: &EffectRun);
}
