//! A program's sound clip by clip, for playback that mixes it live as a
//! render does.

use super::{
    place, program_sequence, samples, sound_filters, Crossfade, Graph, ProgramError, Result, Shown,
};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{Catalog, Clip, Content, Id, Project, Sequence, Time, TrackKind};

/// How long a nested sound plays on past its end through effects on the
/// clips holding it, for their ringing to fade out as it does in a render.
const RING: Time = Time::from_flicks(FLICKS_PER_SECOND / 4);

/// A media clip's sound on a program, as a render mixes it: heard over
/// `[start, end)`, through every sequence clip it is nested in.
///
/// A render chains each clip's filters ([`sound_filters`]) and sums the
/// chains of a sequence. The filters of a clip holding a sequence are
/// linear, so they commute with that sum: a nested clip's chain followed by
/// its holders' is its share of the program's sound.
#[derive(Clone, Debug, PartialEq)]
pub struct ProgramSound {
    /// The media clip.
    pub clip: Id,
    /// Its media file.
    pub path: String,
    pub start: Time,
    pub end: Time,
    /// The media clip, then each sequence clip it is nested in, outward.
    layers: Vec<Layer>,
}

/// A clip as its track plays it.
#[derive(Clone, Debug, PartialEq)]
struct Layer {
    clip: Clip,
    /// Its span with crossfade handles, in its sequence's time.
    from: Time,
    to: Time,
    fade_in: Option<Crossfade>,
    fade_out: Option<Crossfade>,
    /// The length of its media or nested sequence.
    length: Option<Time>,
}

impl Layer {
    fn shown(&self) -> Shown<'_> {
        Shown {
            clip: &self.clip,
            from: self.from,
            to: self.to,
            fade_in: self.fade_in.map(Crossfade::reveal),
            fade_out: self.fade_out.map(Crossfade::reveal),
        }
    }
}

/// How to play part of a [`ProgramSound`].
#[derive(Clone, Debug, PartialEq)]
pub struct SoundCut {
    /// The media decoded, `[start, end)` in its own time. The filters read
    /// its samples timed from `source.0`.
    pub source: (Time, Time),
    /// The program time of the first sample out.
    pub start: Time,
    /// From the decoded samples to stereo at [`SAMPLE_RATE`](super::SAMPLE_RATE).
    pub filters: String,
}

impl ProgramSound {
    /// Whether the media plays backward, which a render decodes whole
    /// before it plays any of it.
    pub fn reverse(&self) -> bool {
        self.layers[0].clip.speed().is_reverse()
    }

    /// How to play `[from, to)` of this sound, as a render of that range
    /// mixes it; `None` when none of it is heard there.
    pub fn cut(&self, catalog: &Catalog, from: Time, to: Time) -> Result<Option<SoundCut>> {
        let (from, mut to) = (from.max(self.start), to.min(self.end));
        if from >= to {
            return Ok(None);
        }
        let rings = self.layers[1..]
            .iter()
            .any(|layer| layer.clip.effects.iter().any(|effect| effect.enabled));
        if to == self.end && rings {
            to += RING;
        }
        // Outside in: each clip plays in the window its holder decodes.
        let mut window = (from, to);
        let mut steps = Vec::with_capacity(self.layers.len());
        for layer in self.layers.iter().rev() {
            let Some(portion) = layer.shown().portion(layer.length, window.0, window.1) else {
                return Ok(None);
            };
            let filters = sound_filters(
                catalog,
                &layer.clip,
                layer.fade_in,
                layer.fade_out,
                &portion,
            )?;
            window = Graph::source_window(&layer.clip, portion.decode, portion.hi)
                .expect("sound clips have a source");
            steps.push((filters, portion.lo, window));
        }
        // Inside out: each holder reads its sequence's mix of the window it
        // decodes, where `mix` places the clip inside.
        let mut steps = steps.into_iter().rev();
        let (mut filters, mut start, source) = steps.next().expect("a sound has its media clip");
        for (outer, lo, (decoded, end)) in steps {
            let placed = place(samples(start - decoded), end - decoded);
            filters = format!("{filters},{placed},{outer}");
            start = lo;
        }
        Ok(Some(SoundCut {
            source,
            start,
            filters,
        }))
    }
}

/// The sound of `sequence` (the first when `None`) clip by clip: every
/// media clip a render of it mixes, in order of start.
///
/// A nested sequence playing at another speed is left out: a render
/// changes the speed of its mix as a whole.
pub fn program_sound(
    project: &Project,
    catalog: &Catalog,
    sequence: Option<&str>,
) -> Result<Vec<ProgramSound>> {
    let sequence = program_sequence(project, sequence)?;
    let mut graph = Graph::new(project, catalog, sequence.rate);
    let whole = (Time::ZERO, sequence.duration());
    let mut sounds = Vec::new();
    graph.sounds(
        sequence,
        whole,
        Time::ZERO,
        whole,
        &mut Vec::new(),
        &mut sounds,
    )?;
    sounds.sort_by_key(|sound| sound.start);
    Ok(sounds)
}

impl<'a> Graph<'a> {
    /// The media clips heard in `[from, to)` of `sequence`, as `mix` mixes
    /// them. Program time is the sequence's plus `offset`; the `holders`
    /// of the sequence are heard over `heard` of the program.
    fn sounds(
        &mut self,
        sequence: &'a Sequence,
        (from, to): (Time, Time),
        offset: Time,
        heard: (Time, Time),
        holders: &mut Vec<Layer>,
        sounds: &mut Vec<ProgramSound>,
    ) -> Result<()> {
        for track in &sequence.tracks {
            if track.kind != TrackKind::Audio || !track.enabled || track.muted {
                continue;
            }
            let (items, _) = self.shown(track, sequence.rate)?;
            for item in items.iter().filter(|item| item.clip.enabled) {
                let Some(portion) = self.portion(item, from, to) else {
                    continue;
                };
                let heard = (
                    heard.0.max(portion.lo + offset),
                    heard.1.min(portion.hi + offset),
                );
                if heard.0 >= heard.1 {
                    continue;
                }
                let clip = item.clip;
                let layer = Layer {
                    clip: clip.clone(),
                    from: item.from,
                    to: item.to,
                    fade_in: Crossfade::of(item.fade_in),
                    fade_out: Crossfade::of(item.fade_out),
                    length: self.source_length(clip),
                };
                match &clip.content {
                    Content::Media { media, .. } => {
                        let media = self.project.media(media.as_str()).ok_or_else(|| {
                            ProgramError::Invalid(format!("clip `{}` uses missing media", clip.id))
                        })?;
                        if media.audio.is_none() {
                            continue;
                        }
                        sounds.push(ProgramSound {
                            clip: clip.id.clone(),
                            path: media.path.clone(),
                            start: heard.0,
                            end: heard.1,
                            layers: std::iter::once(layer)
                                .chain(holders.iter().rev().cloned())
                                .collect(),
                        });
                    }
                    Content::Sequence {
                        sequence: nested, ..
                    } if clip.speed().value() == 1.0 => {
                        let nested = self.nested(clip, nested)?;
                        let (start, end) = Self::source_window(clip, portion.decode, portion.hi)
                            .expect("nested clips have a source");
                        // The window's start is where the clip decodes from.
                        let inner = offset + portion.decode - start;
                        self.nesting.push(nested.id.clone());
                        holders.push(layer);
                        self.sounds(nested, (start, end), inner, heard, holders, sounds)?;
                        holders.pop();
                        self.nesting.pop();
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }
}
