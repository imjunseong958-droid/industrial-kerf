//! A clip's effects run on its picture, each found by its id and run the same way whichever crate
//! it is in: the files they name read once, an HDR picture brought under 1 for one written for 0
//! to 1, an adjustment layer's run on the picture beneath it, and what an effect reads beyond its
//! picture ([`ClipEnv`]): the clip's other frames and source, the sequence's other tracks, and
//! its sound tracks.

use super::*;

/// What an effect's picture was brought between 0 and 1 by.
pub(super) enum Held {
    Nothing,
    Unit(Vec<[f32; 2]>),
    Gamut(Vec<f32>),
    /// What the whole picture had taken from it, and what was left divided by.
    Scale(f32, f32),
}

impl Held {
    pub(super) fn put_back(self, picture: &mut Image) {
        match self {
            Held::Nothing => {}
            Held::Unit(held) if held.len() * 4 == picture.px.len() => picture.from_unit(&held),
            Held::Gamut(held) if held.len() * 4 == picture.px.len() => picture.from_gamut(&held),
            Held::Unit(_) | Held::Gamut(_) => {}
            Held::Scale(lift, by) => picture.unlift_rgb(lift, by),
        }
    }
}

impl Drawing {
    /// Run a clip's effects before the one at `upto` on its picture at clip
    /// time `time`, placed as `placed` says. Each is found by its id and run
    /// the same way, whichever crate it is in.
    pub(super) fn effects(
        &self,
        picture: &mut Image,
        plan: &ClipPlan,
        time: Time,
        placed: &Placed,
        upto: usize,
    ) {
        let sequence = placed.sequence;
        // How many of the picture's pixels a canvas pixel is.
        let per_canvas = placed.scale / placed.to_output.determinant().abs().sqrt().max(1e-6);
        let timecode = format_timecode(
            sequence.start + placed.at,
            sequence.rate,
            sequence.drop_frame,
        );
        for (index, effect) in plan.effects[..upto].iter().enumerate() {
            if video::instance::leaves_alone(effect, time, self.working()) {
                continue;
            }
            let Some(code) = domains::effect(effect.fx.as_str()) else {
                continue;
            };
            let held = self.hold_headroom(picture, effect);
            let env = ClipEnv {
                drawing: self,
                plan,
                placed,
                upto: index,
                working: (picture.w, picture.h),
                lift: match held {
                    Held::Scale(lift, by) => Some((lift, by)),
                    _ => None,
                },
            };
            let run = EffectRun {
                at: time,
                sequence_at: placed.at,
                per_source: placed.per_source,
                per_canvas,
                index,
                nested: upto < plan.effects.len(),
                seed: kerf_core::raster::seed(plan.clip.id.as_str()).wrapping_add(index as u32),
                masks: &plan.clip.masks,
                frame_rate: sequence.rate.fps(),
                duration: plan.clip.duration,
                working: self.working(),
                timecode: &timecode,
                clip_name: &plan.name,
                tracks: &sequence.track_ids,
                sounds: &sequence.sound_ids,
                files: &self.files,
                host: placed.env.then_some(&env as &dyn EffectHost),
            };
            code.apply(picture, effect, &run);
            held.put_back(picture);
        }
    }

    /// Bring an HDR picture under 1 for an effect written for 0 to 1. SDR footage in an HDR
    /// sequence is never above 1, so it meets the effect as it would in an SDR sequence.
    pub(super) fn hold_headroom(&self, picture: &mut Image, effect: &Effect) -> Held {
        if self.hdr.is_none() {
            return Held::Nothing;
        }
        let way = video::catalog::headroom(effect.fx.as_str());
        if way == Headroom::Keeps {
            return Held::Nothing;
        }
        let (peak, floor) = (picture.peak(), picture.floor());
        if way == Headroom::Gamut {
            return if floor < 0.0 {
                Held::Gamut(picture.to_gamut())
            } else {
                Held::Nothing
            };
        }
        if peak <= 1.0 && floor >= 0.0 {
            return Held::Nothing;
        }
        if way == Headroom::PerPixel {
            return Held::Unit(picture.to_unit());
        }
        // One scale for all footage up to the HDR peak, so it does not change between frames.
        let scale = peak.max((1000.0 / kerf_color::REFERENCE_WHITE_NITS) as f32);
        let lift = floor.min(0.0);
        let by = scale - lift;
        picture.lift_rgb(lift, by);
        Held::Scale(lift, by)
    }

    /// An adjustment layer: its effects on the picture beneath it, weighed
    /// against that picture by its opacity, masks, and frame.
    pub(super) fn adjust(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
    ) {
        let (w, h) = (canvas.w, canvas.h);
        let mut adjusted = canvas.clone();
        // FilmCraft's effects see no clip of their own here.
        let placed = Placed {
            sequence,
            at: plan.clip.start + time,
            scale,
            to_output: Affine::IDENTITY,
            per_source: scale,
            env: false,
            reach: None,
        };
        self.effects(&mut adjusted, plan, time, &placed, plan.effects.len());
        let weight = self.adjustment_weight(sequence, plan, time, scale, (w, h));
        weigh_in(canvas, adjusted, &weight, plan.blend);
    }

    /// How much an adjustment layer at clip time `time` weighs at each pixel of a `w` × `h` canvas
    /// drawn at `scale`: as much as its opacity, masks, and frame cover there.
    fn adjustment_weight(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        (w, h): (usize, usize),
    ) -> Vec<f32> {
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let mut frame = Image::filled(w, h, [1.0; 4]);
        let masks = mask::prepare(&plan.clip.masks, size, scale, time);
        if let Some(covered) = mask::coverage(&masks, w, h) {
            frame.mul_coverage(&covered);
        }
        let place = Affine::scale(scale, scale)
            .then_apply(&self.geometry(sequence, plan, time))
            .then_apply(&Affine::scale(1.0 / scale, 1.0 / scale));
        frame = match crop_rect(plan, time, size, scale, (w, h)) {
            Some(crop) => frame.transformed_within(w, h, &place, crop),
            None if identity(&place) => frame,
            None => frame.transformed(w, h, &place),
        };
        let opacity = number(&param(&plan.clip, "opacity").value_at(time)) as f32;
        frame
            .px
            .as_chunks::<4>()
            .0
            .iter()
            .map(|pixel| pixel[3] * opacity)
            .collect()
    }
}

/// The files `effects` name, read once for all the clips that name them.
pub(super) fn files_named(effects: &[Effect]) -> Result<Files> {
    let mut files = Files::new();
    for effect in effects {
        if let Some(code) = domains::effect(effect.fx.as_str()) {
            code.prepare(effect, &mut files)
                .map_err(ComposeError::Invalid)?;
        }
    }
    Ok(files)
}

/// `adjusted`, the picture beneath an adjustment layer with its effects done to it, weighed against
/// that picture on `canvas` by `weight` at each pixel and laid over it with `blend`.
fn weigh_in(canvas: &mut Image, mut adjusted: Image, weight: &[f32], blend: Blend) {
    if blend == Blend::Normal {
        for ((pixel, adjusted), weight) in canvas
            .px
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(adjusted.px.as_chunks::<4>().0.iter())
            .zip(weight)
        {
            for k in 0..4 {
                pixel[k] += (adjusted[k] - pixel[k]) * weight;
            }
        }
    } else {
        adjusted.mul_coverage(weight);
        composite(canvas, &adjusted, 1.0, blend);
    }
}

/// Where a clip's effects run: on its picture at sequence time `at`, drawn
/// at `scale` of `sequence`'s canvas.
pub(super) struct Placed<'a> {
    pub(super) sequence: &'a SequencePlan,
    pub(super) at: Time,
    pub(super) scale: f64,
    pub(super) to_output: Affine,
    pub(super) per_source: f64,
    /// Whether FilmCraft's effects may read the clip's other frames and the
    /// sequence's other tracks.
    pub(super) env: bool,
    /// How far past its frame the clip's picture reaches, which its other frames reach as well.
    pub(super) reach: Option<Reach>,
}

/// What the effect at `upto` among a clip's effects reads beyond its
/// picture.
#[derive(Clone, Copy)]
pub(super) struct ClipEnv<'a> {
    pub(super) drawing: &'a Drawing,
    pub(super) plan: &'a ClipPlan,
    pub(super) placed: &'a Placed<'a>,
    pub(super) upto: usize,
    /// The size of the picture the effect runs on.
    pub(super) working: (usize, usize),
    /// What the effect's picture had taken from it, and what was left divided by, when an HDR
    /// picture was brought under 1 for it whole: the clip's other frames meet it the same way.
    pub(super) lift: Option<(f32, f32)>,
}

impl ClipEnv<'_> {
    /// The clip's picture at sequence time `at`, as the effect gets it.
    pub(super) fn picture_at(&self, at: Time) -> Option<Image> {
        let placed = self.placed;
        let time = at - self.plan.clip.start;
        let (cw, ch) = output_size(placed.sequence.width, placed.sequence.height, placed.scale);
        let input = self.drawing.input(
            placed.sequence,
            self.plan,
            time,
            placed.scale,
            (cw, ch),
            placed.reach,
        )?;
        let mut picture = Arc::unwrap_or_clone(input.picture);
        if (picture.w, picture.h) != self.working {
            let (w, h) = self.working;
            let fit = Affine::scale(
                w as f64 / picture.w.max(1) as f64,
                h as f64 / picture.h.max(1) as f64,
            );
            picture = picture.transformed(w, h, &fit);
        }
        let earlier = Placed { at, ..*placed };
        self.drawing
            .effects(&mut picture, self.plan, time, &earlier, self.upto);
        Some(picture)
    }
}

impl EffectHost for ClipEnv<'_> {
    fn picture_at(&self, at: Time, before: usize) -> Option<Image> {
        let env = ClipEnv {
            upto: before.min(self.upto),
            ..*self
        };
        let mut picture = env.picture_at(self.plan.clip.start + at)?;
        if let Some((lift, by)) = self.lift {
            picture.lift_rgb(lift, by);
        }
        Some(picture)
    }

    fn sound(&self, track: &str, start: f64, frames: usize, rate: u32) -> Option<Vec<f32>> {
        let track = self.placed.sequence.sounds.iter().find(|t| t.id == track)?;
        let mut mixed = vec![0.0; frames * 2];
        let kept = f64::from(crate::sound::RATE);
        for clip in &track.clips {
            let Some(sound) = self.drawing.sounds.of(&clip.path) else {
                continue;
            };
            let length = sound.len() / 2;
            let (from, duration) = (clip.start.seconds(), clip.duration.seconds());
            for (index, out) in mixed.as_chunks_mut::<2>().0.iter_mut().enumerate() {
                let offset = start + index as f64 / f64::from(rate.max(1)) - from;
                if !(0.0..duration).contains(&offset) {
                    continue;
                }
                let played = clip.speed.played_seconds(offset);
                let played = if clip.speed.is_reverse() {
                    clip.speed.played_seconds(duration) - played
                } else {
                    played
                };
                let at = (clip.source_in.seconds() + played) * kept;
                match at as usize {
                    _ if at < 0.0 => {}
                    sample if sample < length => {
                        out[0] += sound[2 * sample];
                        out[1] += sound[2 * sample + 1];
                    }
                    _ => {}
                }
            }
        }
        Some(mixed)
    }

    fn source_at(&self, at: Time, scale: f32) -> Option<Image> {
        let (picture, _) = self.drawing.picture(
            self.placed.sequence,
            self.plan,
            at,
            f64::from(scale.clamp(1.0 / 64.0, 1.0)),
            None,
        )?;
        Some(Arc::unwrap_or_clone(picture))
    }

    fn track(&self, id: &str) -> Option<Image> {
        let placed = self.placed;
        let track = placed.sequence.tracks.iter().find(|track| track.id == id)?;
        let (w, h) = output_size(placed.sequence.width, placed.sequence.height, placed.scale);
        let mut canvas = Image::new(w, h);
        let (at, scale) = (placed.at, placed.scale);
        let in_depth = track.shown.iter().any(|shown| shown.plan.three_d);
        let scene = in_depth.then(|| {
            self.drawing
                .scene(placed.sequence, &[track], at, scale, (w, h))
        });
        // A track an effect reads is read as its own picture.
        let moment = Moment {
            at,
            scale,
            scene: scene.as_ref(),
            clears: false,
        };
        self.drawing
            .track(&mut canvas, placed.sequence, track, &moment);
        Some(canvas)
    }

    fn to_output(&self) -> Affine {
        self.placed.to_output
    }

    fn source_timecode(&self) -> String {
        let time = self.placed.at - self.plan.clip.start;
        let source = match &self.plan.clip.content {
            Content::Media {
                source_in, speed, ..
            }
            | Content::Sequence {
                source_in, speed, ..
            } => *source_in + speed.played(time),
            _ => time,
        };
        let sequence = self.placed.sequence;
        format_timecode(source, sequence.rate, sequence.drop_frame)
    }

    fn sequence_name(&self) -> String {
        self.placed.sequence.name.clone()
    }

    fn sequence_size(&self) -> (u32, u32) {
        (self.placed.sequence.width, self.placed.sequence.height)
    }

    fn source_size(&self) -> (u32, u32) {
        self.plan.size
    }

    fn clip_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let clip = &self.plan.clip;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        clip.id.as_str().hash(&mut hasher);
        clip.duration.flicks().hash(&mut hasher);
        if let Some((source_in, _)) = clip.source_range() {
            source_in.flicks().hash(&mut hasher);
        }
        format!("{:?}", clip.speed()).hash(&mut hasher);
        hasher.finish()
    }
}
