//! Kerf's own compositor: a sequence drawn from Kerf's model directly, in
//! linear light, with the drawing carried from FilmCraft's renderer
//! (`kerf-draw`), and media decoded by libav.
//!
//! It draws what the EffectCraft compositor (`scene`) draws, the same way:
//!
//! - Picture tracks stack in sequence order, and captions play over them
//!   all. A nested sequence is drawn at its own canvas and frame rate, at
//!   the time its clip maps to.
//! - A clip's picture is fitted to the canvas as the graph fits it, its
//!   masks and crop cut it, its effects run on it, and it is scaled,
//!   flipped, rotated about its anchor, and moved onto the canvas with its
//!   opacity and blend mode. Opaque media left where fitting puts it is
//!   letterboxed with black, as the graph pads it.
//! - A dissolve fades the incoming clip in over the outgoing one, a wipe
//!   reveals it along its front, and a dip fades a solid through.
//! - Titles are text centered on a transparent canvas, solids a color, and
//!   an adjustment layer applies its effects to the picture beneath it.
//!
//! Raster effects and transitions (EffectCraft's) are not drawn here yet:
//! a program with them is reported as unsupported.

use crate::footage::{frame_count, ClipSource, Pictures};
use crate::{ComposeError, Frame};
use kerf_core::program::{canvas, program_sequence};
use kerf_draw::blend::{composite, composite_placed};
use kerf_draw::correct::Correction;
use kerf_draw::text::{self, Justify, Text};
use kerf_draw::{blur, mask, wipe, Affine, Blend, Image, Rect, Vec2};
use kerf_model::{
    Catalog, Clip, Content, Domain, Effect, Engine, Id, Param, Project, Rate, Ratio, Rgba,
    Sequence, Time, Track, TrackKind, Value,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Nested sequences deeper than this are refused, as the graph refuses
/// them.
const MAX_NESTING: usize = 16;

/// The clip properties that place a picture.
const GEOMETRY: [&str; 12] = [
    "crop_l", "crop_r", "crop_t", "crop_b", "scale", "scale_x", "scale_y", "rotation", "anchor_x",
    "anchor_y", "x", "y",
];

type Result<T> = std::result::Result<T, ComposeError>;

/// A program ready to draw.
pub(crate) struct Drawing {
    /// The program's sequence first, then the sequences nested in it.
    sequences: Vec<SequencePlan>,
    pictures: Pictures,
    captions: bool,
}

struct SequencePlan {
    width: u32,
    height: u32,
    rate: Rate,
    duration: Time,
    tracks: Vec<TrackPlan>,
    captions: Vec<CaptionPlan>,
}

struct CaptionPlan {
    text: String,
    from: Time,
    to: Time,
}

struct TrackPlan {
    shown: Vec<Shown>,
    dips: Vec<Dip>,
}

/// A clip as its track shows it: its span with transition handles, and
/// how it enters.
struct Shown {
    plan: ClipPlan,
    from: Time,
    to: Time,
    reveal: Option<(Reveal, Time, Time)>,
}

#[derive(Clone, Copy)]
enum Reveal {
    Dissolve,
    Wipe { angle: f64, softness: f64 },
}

/// A dip's color between the two clips of its cut.
struct Dip {
    color: Rgba,
    from: Time,
    to: Time,
}

struct ClipPlan {
    clip: Clip,
    source: Source,
    /// The source's size in its own pixels.
    size: (u32, u32),
    /// Fitted to the canvas, keeping its aspect, as media and nested
    /// sequences are.
    fit: bool,
    /// Padded to the canvas with black, as the graph pads it.
    letterboxed: bool,
    /// A frame showing it alone is its media's picture as decoded.
    plain: bool,
    /// Any of the properties that place it animate.
    animated: bool,
    /// The most of its size it is ever shown at, when under 1: its frames
    /// are decoded at that size.
    shown: f64,
    blend: Blend,
    effects: Vec<Effect>,
}

enum Source {
    Media {
        picture: usize,
    },
    Nested {
        sequence: usize,
        source_in: Time,
        speed: Ratio,
        duration: Time,
    },
    Solid(Rgba),
    Title(TitlePlan),
    Adjustment,
}

struct TitlePlan {
    text: String,
    font: String,
    /// In frame heights.
    size: f64,
    color: Rgba,
    justify: Justify,
}

impl Drawing {
    pub fn new(
        project: &Project,
        catalog: &Catalog,
        sequence: Option<&str>,
        captions: bool,
    ) -> Result<Self> {
        let top = program_sequence(project, sequence)
            .map_err(|error| ComposeError::Invalid(error.to_string()))?;
        let mut builder = Builder {
            project,
            catalog,
            sequences: Vec::new(),
            built: HashMap::new(),
            clips: Vec::new(),
            nesting: vec![top.id.clone()],
        };
        // The program's sequence is planned first, so it is the first.
        builder.sequences.push(None);
        let plan = builder.sequence(top)?;
        builder.sequences[0] = Some(plan);
        Ok(Self {
            sequences: builder
                .sequences
                .into_iter()
                .map(|plan| plan.expect("every sequence planned"))
                .collect(),
            pictures: Pictures::new(builder.clips),
            captions,
        })
    }

    pub fn size(&self) -> (u32, u32) {
        (self.sequences[0].width, self.sequences[0].height)
    }

    pub fn rate(&self) -> Rate {
        self.sequences[0].rate
    }

    pub fn duration(&self) -> Time {
        self.sequences[0].duration
    }

    pub fn problems(&self) -> Vec<String> {
        self.pictures
            .problems()
            .into_iter()
            .map(|(_, problem)| problem)
            .collect()
    }

    /// The picture at sequence time `at` over black, at `scale` of the
    /// canvas (1 is full size).
    pub fn frame(&self, at: Time, scale: f64) -> Frame {
        if scale >= 1.0 {
            if let Some(frame) = self.plain_frame(at) {
                return frame;
            }
        }
        let image = self.draw(0, at, scale.clamp(1.0 / 16.0, 1.0));
        Frame {
            width: image.w as u32,
            height: image.h as u32,
            rgba: image.to_rgba8_over_black(),
        }
    }

    /// A frame that is one media clip alone, untouched, at full size: its
    /// picture as decoded, which drawing it would give again.
    fn plain_frame(&self, at: Time) -> Option<Frame> {
        let plan = &self.sequences[0];
        if self.captions && plan.captions.iter().any(|caption| caption.shows(at)) {
            return None;
        }
        let mut showing = plan.tracks.iter().flat_map(|track| {
            let shown = track.shown.iter().filter(|shown| shown.shows(at)).map(Some);
            let dips = track.dips.iter().filter(|dip| dip.shows(at)).map(|_| None);
            shown.chain(dips)
        });
        let shown = showing.next()??;
        if showing.next().is_some() || !shown.plan.plain {
            return None;
        }
        let Source::Media { picture } = shown.plan.source else {
            return None;
        };
        let rgba = self.pictures.rgba(picture, at - shown.plan.clip.start)?;
        Some(Frame {
            width: plan.width,
            height: plan.height,
            rgba,
        })
    }

    /// Sequence `index` at its time `at`, at `scale` of its canvas,
    /// transparent where nothing is.
    fn draw(&self, index: usize, at: Time, scale: f64) -> Image {
        let plan = &self.sequences[index];
        let (w, h) = output_size(plan.width, plan.height, scale);
        let mut canvas = Image::new(w, h);
        for track in &plan.tracks {
            for shown in track.shown.iter().filter(|shown| shown.shows(at)) {
                self.layer(&mut canvas, plan, shown, at, scale);
            }
            for dip in track.dips.iter().filter(|dip| dip.shows(at)) {
                let opacity = f64::from(dip.color.a) / 255.0
                    * (1.0 - (2.0 * progress(at, dip.from, dip.to) - 1.0).abs());
                let solid = Image::filled(
                    w,
                    h,
                    premultiplied(Rgba {
                        a: 255,
                        ..dip.color
                    }),
                );
                composite(&mut canvas, &solid, opacity as f32, Blend::Normal);
            }
        }
        if self.captions {
            for caption in plan.captions.iter().filter(|caption| caption.shows(at)) {
                let size = (f64::from(plan.height) * 0.045).max(12.0) * scale;
                let text = Text {
                    text: &caption.text,
                    font: "sans-serif",
                    size: size as f32,
                    color: Rgba::WHITE,
                    justify: Justify::Center,
                    outline: Some((
                        Rgba {
                            a: 217,
                            ..Rgba::BLACK
                        },
                        (size * 0.06) as f32,
                    )),
                };
                text::draw(&mut canvas, &text, [w as f32 / 2.0, h as f32 * 0.95], 1.0);
            }
        }
        canvas
    }

    /// Draw a shown clip over `canvas` at sequence time `at`.
    fn layer(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
    ) {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        if matches!(plan.source, Source::Adjustment) {
            self.adjust(canvas, sequence, plan, time, scale);
            return;
        }
        // Shared with the frame cache until something changes it.
        let Some((mut picture, per_source)) = self.picture(sequence, plan, time, scale) else {
            return;
        };
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let (w, h) = (canvas.w, canvas.h);
        let place = Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
        let mut crop = crop_rect(plan, time, size, per_source, (picture.w, picture.h));
        let to_output = if plan.letterboxed {
            // The fitted picture on black, the size of the canvas.
            let mut boxed = Image::filled(w, h, [0.0, 0.0, 0.0, 1.0]);
            let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
            composite_placed(
                &mut boxed,
                &picture,
                &place.then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source)),
                crop.take().unwrap_or(whole),
                1.0,
                Blend::Normal,
            );
            picture = Arc::new(boxed);
            Affine::IDENTITY
        } else {
            let masks = mask::prepare(&clip.masks, size, per_source, time);
            if let Some(covered) = mask::coverage(&masks, picture.w, picture.h) {
                Arc::make_mut(&mut picture).mul_coverage(&covered);
            }
            place.then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source))
        };
        if !plan.effects.is_empty() {
            let spread = to_output.determinant().abs().sqrt().max(1e-6);
            self.effects(Arc::make_mut(&mut picture), plan, time, scale / spread);
        }
        let reveal = shown
            .reveal
            .map(|(reveal, start, end)| (reveal, progress(at, start, end)));
        if let Some((Reveal::Wipe { angle, softness }, progress)) = reveal {
            let covered = wipe::coverage(picture.w, picture.h, angle, softness, progress);
            Arc::make_mut(&mut picture).mul_coverage(&covered);
        }
        let fade = match reveal {
            Some((Reveal::Dissolve, progress)) => progress,
            _ => 1.0,
        };
        let opacity = (number(&param(clip, "opacity").value_at(time)) * fade) as f32;
        match crop {
            None if (picture.w, picture.h) == (w, h) && identity(&to_output) => {
                composite(canvas, &picture, opacity, plan.blend);
            }
            crop => {
                let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
                let crop = crop.unwrap_or(whole);
                composite_placed(canvas, &picture, &to_output, crop, opacity, plan.blend);
            }
        }
    }

    /// A clip's picture at clip time `time` before its masks and effects,
    /// with how many of its pixels a source pixel is.
    fn picture(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
    ) -> Option<(Arc<Image>, f64)> {
        let (cw, ch) = output_size(sequence.width, sequence.height, scale);
        Some(match &plan.source {
            Source::Media { picture } => {
                let factor = plan.shown * scale;
                let size = (
                    ((f64::from(plan.size.0) * factor).round() as u32).max(1),
                    ((f64::from(plan.size.1) * factor).round() as u32).max(1),
                );
                let decoded = self.pictures.picture(*picture, time, size)?;
                let per_source = decoded.w as f64 / f64::from(plan.size.0);
                (decoded, per_source)
            }
            Source::Nested {
                sequence: nested,
                source_in,
                speed,
                duration,
            } => {
                let inner = &self.sequences[*nested];
                let shown = nested_time(time, *source_in, *speed, *duration, inner.rate);
                let image = self.draw(*nested, shown, scale);
                let per_source = image.w as f64 / f64::from(inner.width);
                (Arc::new(image), per_source)
            }
            Source::Solid(color) => (
                Arc::new(Image::filled(cw, ch, premultiplied(*color))),
                scale,
            ),
            Source::Title(title) => {
                let mut image = Image::new(cw, ch);
                let text = Text {
                    text: &title.text,
                    font: &title.font,
                    size: (title.size * ch as f64).max(1.0) as f32,
                    color: title.color,
                    justify: title.justify,
                    outline: None,
                };
                text::draw(&mut image, &text, [cw as f32 / 2.0, ch as f32 / 2.0], 0.5);
                (Arc::new(image), scale)
            }
            Source::Adjustment => return None,
        })
    }

    /// Where a clip's source pixels land on the canvas at clip time `time`,
    /// in canvas pixels: cropped, fitted, scaled, flipped, rotated about
    /// its anchor, and moved, as the graph places it.
    fn geometry(&self, sequence: &SequencePlan, plan: &ClipPlan, time: Time) -> Affine {
        place(
            plan,
            (f64::from(sequence.width), f64::from(sequence.height)),
            time,
        )
        .0
    }

    /// Run a clip's effects on its picture; `density` is how many of the
    /// picture's pixels a canvas pixel is.
    fn effects(&self, picture: &mut Image, plan: &ClipPlan, time: Time, density: f64) {
        for effect in &plan.effects {
            let value = |name: &str| effect_number(effect, name, time);
            match effect.fx.as_str() {
                "color.basic" => {
                    let correction = Correction::new(&value);
                    if !correction.is_identity() {
                        picture.map_encoded(|rgb| correction.apply(rgb));
                    }
                }
                "blur.gaussian" => {
                    // The graph blurs by half the radius, in canvas pixels.
                    let sigma = (value("radius") / 2.0 * density) as f32;
                    blur::gaussian(picture, sigma, sigma, true);
                }
                _ => {}
            }
        }
    }

    /// An adjustment layer: its effects on the picture beneath it, weighed
    /// against that picture by its opacity, masks, and frame.
    fn adjust(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
    ) {
        let (w, h) = (canvas.w, canvas.h);
        let mut adjusted = canvas.clone();
        self.effects(&mut adjusted, plan, time, scale);
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
        let weight: Vec<f32> = frame
            .px
            .chunks_exact(4)
            .map(|pixel| pixel[3] * opacity)
            .collect();
        if plan.blend == Blend::Normal {
            for ((pixel, adjusted), weight) in canvas
                .px
                .chunks_exact_mut(4)
                .zip(adjusted.px.chunks_exact(4))
                .zip(&weight)
            {
                for k in 0..4 {
                    pixel[k] += (adjusted[k] - pixel[k]) * weight;
                }
            }
        } else {
            adjusted.mul_coverage(&weight);
            composite(canvas, &adjusted, 1.0, plan.blend);
        }
    }
}

impl Shown {
    fn shows(&self, at: Time) -> bool {
        self.from <= at && at < self.to
    }
}

impl Dip {
    fn shows(&self, at: Time) -> bool {
        self.from <= at && at < self.to
    }
}

impl CaptionPlan {
    fn shows(&self, at: Time) -> bool {
        self.from <= at && at < self.to
    }
}

struct Builder<'a> {
    project: &'a Project,
    catalog: &'a Catalog,
    /// Planned sequences by index; `None` while one is being planned.
    sequences: Vec<Option<SequencePlan>>,
    /// The index of each nested sequence planned.
    built: HashMap<Id, usize>,
    clips: Vec<ClipSource>,
    nesting: Vec<Id>,
}

impl<'a> Builder<'a> {
    fn sequence(&mut self, sequence: &'a Sequence) -> Result<SequencePlan> {
        let (width, height) = canvas(sequence);
        let mut tracks = Vec::new();
        let mut captions = Vec::new();
        for track in &sequence.tracks {
            match track.kind {
                TrackKind::Video if track.enabled => tracks.push(self.track(sequence, track)?),
                TrackKind::Subtitle if track.enabled => {
                    for clip in track.clips.iter().filter(|clip| clip.enabled) {
                        if let Content::Caption { text } = &clip.content {
                            if !text.trim().is_empty() {
                                captions.push(CaptionPlan {
                                    text: text.clone(),
                                    from: clip.start,
                                    to: clip.end(),
                                });
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(SequencePlan {
            width,
            height,
            rate: sequence.rate,
            duration: sequence.duration(),
            tracks,
            captions,
        })
    }

    fn track(&mut self, sequence: &'a Sequence, track: &'a Track) -> Result<TrackPlan> {
        let rate = sequence.rate;
        let mut spans: Vec<(Time, Time, Option<(Reveal, Time, Time)>, bool)> = track
            .clips
            .iter()
            .map(|clip| (clip.start, clip.end(), None, false))
            .collect();
        let mut dips = Vec::new();
        for transition in &track.transitions {
            let (Some(from), Some(to)) = (
                track.clip_index(transition.from.as_str()),
                track.clip_index(transition.to.as_str()),
            ) else {
                continue;
            };
            let cut = track.clips[from].end();
            let (before, after) = transition.split_at_cut(rate);
            let (start, end) = (cut - before, cut + after);
            let reveal = match transition.fx.as_str() {
                "dip" => {
                    let middle = start + Time::from_flicks((end - start).flicks() / 2);
                    spans[from].1 = middle;
                    spans[to].0 = middle;
                    dips.push(Dip {
                        color: color(
                            self.catalog,
                            &transition.id,
                            &transition.params,
                            "dip",
                            "color",
                        )?,
                        from: start,
                        to: end,
                    });
                    continue;
                }
                "dissolve" | "crossfade" => Reveal::Dissolve,
                "wipe" => Reveal::Wipe {
                    angle: static_number(
                        self.catalog,
                        &transition.id,
                        &transition.params,
                        "wipe",
                        "angle",
                    )?,
                    softness: static_number(
                        self.catalog,
                        &transition.id,
                        &transition.params,
                        "wipe",
                        "softness",
                    )?,
                },
                other => {
                    return Err(unsupported(&transition.id, format!("transition `{other}`")));
                }
            };
            spans[from].1 = end;
            spans[from].3 = true;
            spans[to].0 = start;
            spans[to].2 = Some((reveal, start, end));
        }
        let mut shown = Vec::new();
        for (clip, (from, to, reveal, fades_out)) in track.clips.iter().zip(spans) {
            if !clip.enabled || matches!(clip.content, Content::Caption { .. }) {
                continue;
            }
            let adjustment =
                matches!(&clip.content, Content::Generator { fx, .. } if fx == "adjustment");
            if adjustment && (reveal.is_some() || fades_out) {
                return Err(unsupported(&clip.id, "a transition on an adjustment layer"));
            }
            let (from, to) = if adjustment {
                (clip.start, clip.end())
            } else {
                match self.span(clip, from, to) {
                    Some(span) => span,
                    None => continue,
                }
            };
            let Some(plan) = self.clip(sequence, clip, from, to, reveal.is_none())? else {
                continue;
            };
            shown.push(Shown {
                plan,
                from,
                to,
                reveal,
            });
        }
        Ok(TrackPlan { shown, dips })
    }

    /// The part of a shown clip its source covers, as the graph clamps it.
    fn span(&self, clip: &Clip, mut lo: Time, mut hi: Time) -> Option<(Time, Time)> {
        let length = match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()).map(|m| m.duration),
            Content::Sequence { sequence, .. } => self
                .project
                .sequence(sequence.as_str())
                .map(Sequence::duration),
            Content::Generator { .. } | Content::Caption { .. } => None,
        };
        if let (Some(length), Some((source_in, _))) = (length, clip.source_range()) {
            let speed = clip.speed();
            let (earliest, latest) = if speed.is_reverse() {
                let out = source_in + speed.source_span(clip.duration);
                (
                    clip.start - speed.timeline_span(length - out),
                    clip.start + speed.timeline_span(out),
                )
            } else {
                (
                    clip.start - speed.timeline_span(source_in),
                    clip.start + speed.timeline_span(length - source_in),
                )
            };
            lo = lo.max(earliest);
            hi = hi.min(latest);
        }
        (lo < hi).then_some((lo, hi))
    }

    /// A clip's plan, or nothing for a clip without a picture. `entering`
    /// is false for a clip a transition reveals.
    fn clip(
        &mut self,
        sequence: &'a Sequence,
        clip: &'a Clip,
        lo: Time,
        hi: Time,
        entering: bool,
    ) -> Result<Option<ClipPlan>> {
        let canvas = canvas(sequence);
        let (source, size, fit, alpha) = match &clip.content {
            Content::Media {
                media,
                source_in,
                speed,
            } => {
                let media = self.project.media(media.as_str()).ok_or_else(|| {
                    ComposeError::Invalid(format!("clip `{}` uses missing media", clip.id))
                })?;
                let Some(video) = media.video else {
                    return Ok(None);
                };
                self.clips.push(ClipSource {
                    path: media.path.clone(),
                    rate: media.rate,
                    frames: frame_count(media.rate, media.duration),
                    width: video.width,
                    height: video.height,
                    alpha: video.alpha,
                    source_in: *source_in,
                    speed: *speed,
                    duration: clip.duration,
                    shown: None,
                });
                (
                    Source::Media {
                        picture: self.clips.len() - 1,
                    },
                    (video.width, video.height),
                    true,
                    video.alpha,
                )
            }
            Content::Sequence {
                sequence: nested,
                source_in,
                speed,
            } => {
                if self.nesting.len() >= MAX_NESTING || self.nesting.contains(nested) {
                    return Err(unsupported(&clip.id, "sequences nested this deep"));
                }
                let inner = self.project.sequence(nested.as_str()).ok_or_else(|| {
                    ComposeError::Invalid(format!(
                        "clip `{}` uses missing sequence `{nested}`",
                        clip.id
                    ))
                })?;
                let index = match self.built.get(nested) {
                    Some(index) => *index,
                    None => {
                        self.nesting.push(nested.clone());
                        let index = self.sequences.len();
                        self.sequences.push(None);
                        let plan = self.sequence(inner);
                        self.nesting.pop();
                        self.sequences[index] = Some(plan?);
                        self.built.insert(nested.clone(), index);
                        index
                    }
                };
                (
                    Source::Nested {
                        sequence: index,
                        source_in: *source_in,
                        speed: *speed,
                        duration: clip.duration,
                    },
                    kerf_core::program::canvas(inner),
                    true,
                    true,
                )
            }
            Content::Generator { fx, params, .. } => match fx.as_str() {
                "solid" => (
                    Source::Solid(color(self.catalog, &clip.id, params, "solid", "color")?),
                    canvas,
                    false,
                    true,
                ),
                "title" => (
                    Source::Title(self.title(clip, params)?),
                    canvas,
                    false,
                    true,
                ),
                "adjustment" => (Source::Adjustment, canvas, false, true),
                other => return Err(unsupported(&clip.id, format!("generator `{other}`"))),
            },
            Content::Caption { .. } => return Ok(None),
        };
        let animated = GEOMETRY
            .iter()
            .any(|name| matches!(param(clip, name), Param::Animated(_)));
        let mut effects = Vec::new();
        for effect in clip.effects.iter().filter(|effect| effect.enabled) {
            let def = self.catalog.fx(effect.fx.as_str()).ok_or_else(|| {
                ComposeError::Invalid(format!("clip `{}` uses unknown effect", clip.id))
            })?;
            if def.domain != Domain::Video {
                continue;
            }
            match (def.engine, effect.fx.as_str()) {
                (Engine::Graph, "color.basic" | "blur.gaussian") => effects.push(effect.clone()),
                (Engine::Raster, id) => {
                    return Err(unsupported(&clip.id, format!("raster effect `{id}`")))
                }
                (_, other) => {
                    return Err(unsupported(
                        &clip.id,
                        format!("effect `{other}` on a picture"),
                    ))
                }
            }
        }
        let blend = match param(clip, "blend") {
            Param::Static(Value::Enum(blend) | Value::Text(blend)) => blend
                .parse::<Blend>()
                .map_err(|_| unsupported(&clip.id, format!("blend `{blend}`")))?,
            Param::Static(_) => Blend::Normal,
            Param::Animated(_) => return Err(unsupported(&clip.id, "animated `blend`")),
        };
        let mut plan = ClipPlan {
            clip: clip.clone(),
            source,
            size,
            fit,
            letterboxed: false,
            plain: false,
            animated,
            shown: 1.0,
            blend,
            effects,
        };
        let frames = Frames::new(sequence.rate, clip, lo, hi);
        let cw = f64::from(canvas.0);
        let ch = f64::from(canvas.1);
        let mut largest: f64 = 0.0;
        for time in frames.times() {
            let [l, r, t, b] = ["crop_l", "crop_r", "crop_t", "crop_b"]
                .map(|name| number(&param(clip, name).value_at(time)));
            if l + r >= 1.0 || t + b >= 1.0 {
                return Err(ComposeError::Invalid(format!(
                    "clip `{}` is cropped away entirely",
                    clip.id
                )));
            }
            let (_, scale) = place(&plan, (cw, ch), time);
            largest = largest.max(scale.x.abs()).max(scale.y.abs());
            if !animated {
                break;
            }
        }
        if matches!(plan.source, Source::Media { .. }) {
            plan.shown = largest.clamp(1e-6, 1.0);
            plan.letterboxed = !alpha && letterboxed(&plan, canvas);
            plan.plain =
                !alpha && self.nesting.len() == 1 && entering && size == canvas && untouched(&plan);
        }
        Ok(Some(plan))
    }

    /// A title's text and look, which do not animate.
    fn title(&self, clip: &Clip, params: &kerf_model::Params) -> Result<TitlePlan> {
        let def = self.catalog.fx("title").expect("the catalog defines title");
        let get = |name: &str| -> Result<Value> {
            match params.get(name) {
                None => Ok(def.find(name).expect("title parameter").default.clone()),
                Some(Param::Static(value)) => Ok(value.clone()),
                Some(Param::Animated(_)) => {
                    Err(unsupported(&clip.id, format!("animated title `{name}`")))
                }
            }
        };
        Ok(TitlePlan {
            text: match get("text")? {
                Value::Text(text) => text,
                _ => String::new(),
            },
            font: match get("font")? {
                Value::Text(font) if !font.trim().is_empty() => font,
                _ => "sans-serif".to_owned(),
            },
            size: get("size")?.as_number().unwrap_or(0.08),
            color: match get("color")? {
                Value::Color(color) => color,
                _ => Rgba::WHITE,
            },
            justify: match get("align")? {
                Value::Enum(align) if align == "left" => Justify::Left,
                Value::Enum(align) if align == "right" => Justify::Right,
                _ => Justify::Center,
            },
        })
    }
}

/// Where a clip's source pixels land on a `canvas` at clip time `time`,
/// and how many canvas pixels a source pixel is across and down.
fn place(plan: &ClipPlan, (cw, ch): (f64, f64), time: Time) -> (Affine, Vec2) {
    let clip = &plan.clip;
    let [l, r, t, b, s, sx, sy, rotation, ax, ay, x, y] =
        GEOMETRY.map(|name| number(&param(clip, name).value_at(time)));
    let (sw, sh) = (f64::from(plan.size.0), f64::from(plan.size.1));
    let (width, height) = (sw * (1.0 - l - r), sh * (1.0 - t - b));
    let (fw, fh) = if plan.fit {
        fitted(width, height, cw, ch)
    } else {
        (width, height)
    };
    let (kx, ky) = (s * sx, s * sy);
    let mut position = Vec2::new(cw / 2.0 + x * cw + ax * fw, ch / 2.0 + y * ch + ay * fh);
    let mut scale = Vec2::new(fw / width * kx, fh / height * ky);
    if !plan.animated && rotation == 0.0 && ax == 0.0 && ay == 0.0 {
        // A still picture lands on whole pixels.
        let (ow, oh) = ((fw * kx.abs()).round(), (fh * ky.abs()).round());
        position = Vec2::new(
            (position.x - ow / 2.0 + 1e-9).floor() + ow / 2.0,
            (position.y - oh / 2.0 + 1e-9).floor() + oh / 2.0,
        );
        scale = Vec2::new(ow / width * kx.signum(), oh / height * ky.signum());
    }
    let anchor = Vec2::new(sw * l + width * (0.5 + ax), sh * t + height * (0.5 + ay));
    (Affine::motion(position, scale, rotation, anchor), scale)
}

/// What a clip's crop at clip time `time` leaves of its `picture`, in the
/// picture's pixels (`per_source` of the source's); none when nothing is
/// cropped.
fn crop_rect(
    plan: &ClipPlan,
    time: Time,
    (sw, sh): (f64, f64),
    per_source: f64,
    picture: (usize, usize),
) -> Option<Rect> {
    let [l, r, t, b] = ["crop_l", "crop_r", "crop_t", "crop_b"]
        .map(|name| number(&param(&plan.clip, name).value_at(time)));
    if l == 0.0 && r == 0.0 && t == 0.0 && b == 0.0 {
        return None;
    }
    let (w, h) = (sw * per_source, sh * per_source);
    let (w, h) = (w.min(picture.0 as f64), h.min(picture.1 as f64));
    Some(Rect::new(
        l * w,
        t * h,
        w * (1.0 - l - r),
        h * (1.0 - t - b),
    ))
}

/// Whether the graph pads a clip's picture to the canvas with black:
/// media left where fitting puts it (flipped and cropped included), without
/// masks, that does not fill the canvas.
fn letterboxed(plan: &ClipPlan, canvas: (u32, u32)) -> bool {
    let clip = &plan.clip;
    let static_number = |name: &str| match param(clip, name) {
        Param::Static(Value::Number(n)) => Some(n),
        _ => None,
    };
    if let [Some(l), Some(r), Some(t), Some(b)] =
        ["crop_l", "crop_r", "crop_t", "crop_b"].map(static_number)
    {
        let (sw, sh) = (f64::from(plan.size.0), f64::from(plan.size.1));
        let (fw, fh) = fitted(
            sw * (1.0 - l - r),
            sh * (1.0 - t - b),
            f64::from(canvas.0),
            f64::from(canvas.1),
        );
        if fw >= f64::from(canvas.0) && fh >= f64::from(canvas.1) {
            return false;
        }
    }
    let unit = |axis: &str| match (static_number("scale"), static_number(axis)) {
        (Some(scale), Some(factor)) => ((scale * factor).abs() - 1.0).abs() < 5e-7,
        _ => false,
    };
    clip.masks.is_empty()
        && ["x", "y", "rotation", "anchor_x", "anchor_y"]
            .iter()
            .all(|name| static_number(name) == Some(0.0))
        && unit("scale_x")
        && unit("scale_y")
}

/// Whether a clip is its picture unmoved and untouched.
fn untouched(plan: &ClipPlan) -> bool {
    let clip = &plan.clip;
    let is = |name: &str, value: f64| matches!(param(clip, name), Param::Static(Value::Number(n)) if n == value);
    plan.blend == Blend::Normal
        && plan.effects.is_empty()
        && clip.masks.is_empty()
        && [
            "x", "y", "rotation", "anchor_x", "anchor_y", "crop_l", "crop_r", "crop_t", "crop_b",
        ]
        .iter()
        .all(|name| is(name, 0.0))
        && ["scale", "scale_x", "scale_y", "opacity"]
            .iter()
            .all(|name| is(name, 1.0))
}

/// The time a nested sequence shows at its clip's time `time`: on its own
/// frames, the frame showing at the source position, which a reversed clip
/// runs back from its source out.
fn nested_time(time: Time, source_in: Time, speed: Ratio, duration: Time, rate: Rate) -> Time {
    let span = speed.source_span(time.max(Time::ZERO)).flicks();
    let frame = rate.frame_duration().flicks().max(1);
    let index = if speed.is_reverse() {
        // The frame showing just before the source position.
        let position = (source_in + speed.source_span(duration)).flicks() - span;
        -(-position).div_euclid(frame) - 1
    } else {
        (source_in.flicks() + span).div_euclid(frame)
    };
    rate.time_of(index.max(0))
}

/// A clip property, or its catalog default.
fn param(clip: &Clip, name: &str) -> Param {
    clip.props.get(name).cloned().unwrap_or_else(|| {
        Param::Static(
            Catalog::builtin()
                .prop(Domain::Video, name)
                .map(|def| def.default.clone())
                .unwrap_or(Value::Number(0.0)),
        )
    })
}

/// An effect parameter's number at clip time `time`, or its default.
fn effect_number(effect: &Effect, name: &str, time: Time) -> f64 {
    match effect.params.get(name) {
        Some(param) => number(&param.value_at(time)),
        None => Catalog::builtin()
            .fx(effect.fx.as_str())
            .and_then(|def| def.find(name))
            .and_then(|def| def.default.as_number())
            .unwrap_or(0.0),
    }
}

fn static_number(
    catalog: &Catalog,
    entity: &Id,
    params: &kerf_model::Params,
    fx: &str,
    name: &str,
) -> Result<f64> {
    match params.get(name) {
        None => Ok(catalog
            .fx(fx)
            .and_then(|def| def.find(name))
            .and_then(|def| def.default.as_number())
            .unwrap_or(0.0)),
        Some(Param::Static(Value::Number(n))) => Ok(*n),
        Some(_) => Err(unsupported(entity, format!("animated `{name}`"))),
    }
}

fn color(
    catalog: &Catalog,
    entity: &Id,
    params: &kerf_model::Params,
    fx: &str,
    name: &str,
) -> Result<Rgba> {
    match params.get(name) {
        None => Ok(
            match catalog
                .fx(fx)
                .and_then(|def| def.find(name))
                .map(|def| &def.default)
            {
                Some(Value::Color(color)) => *color,
                _ => Rgba::BLACK,
            },
        ),
        Some(Param::Static(Value::Color(color))) => Ok(*color),
        Some(_) => Err(unsupported(entity, format!("animated `{name}`"))),
    }
}

fn unsupported(entity: &Id, what: impl Into<String>) -> ComposeError {
    ComposeError::Unsupported {
        entity: entity.to_string(),
        what: what.into(),
    }
}

fn number(value: &Value) -> f64 {
    value.as_number().unwrap_or(0.0)
}

/// A color as premultiplied linear light.
fn premultiplied(color: Rgba) -> [f32; 4] {
    let decode = kerf_draw::color::decode_table();
    let alpha = f32::from(color.a) / 255.0;
    [
        decode[usize::from(color.r)] * alpha,
        decode[usize::from(color.g)] * alpha,
        decode[usize::from(color.b)] * alpha,
        alpha,
    ]
}

/// How far a transition has got at `time`, from 0 to 1.
fn progress(time: Time, start: Time, end: Time) -> f64 {
    let span = (end - start).flicks().max(1) as f64;
    ((time - start).flicks() as f64 / span).clamp(0.0, 1.0)
}

/// The size FFmpeg's `scale=W:H:force_original_aspect_ratio=decrease`
/// gives a `width` × `height` picture on a `cw` × `ch` canvas.
fn fitted(width: f64, height: f64, cw: f64, ch: f64) -> (f64, f64) {
    let tall = (ch * width / height).round();
    let wide = (cw * height / width).round();
    (tall.min(cw).max(1.0), wide.min(ch).max(1.0))
}

fn output_size(width: u32, height: u32, scale: f64) -> (usize, usize) {
    (
        ((f64::from(width) * scale).round() as usize).max(1),
        ((f64::from(height) * scale).round() as usize).max(1),
    )
}

fn identity(m: &Affine) -> bool {
    (m.a - 1.0).abs() < 1e-9
        && (m.d - 1.0).abs() < 1e-9
        && m.b.abs() < 1e-12
        && m.c.abs() < 1e-12
        && m.e.abs() < 1e-9
        && m.f.abs() < 1e-9
}

/// The frames a clip shows between `lo` and `hi`, as clip times.
struct Frames {
    start: Time,
    rate: Rate,
    first: i64,
    last: i64,
}

impl Frames {
    fn new(rate: Rate, clip: &Clip, lo: Time, hi: Time) -> Self {
        let frame = rate.frame_duration().flicks();
        let ceil = |time: Time| -(-time.flicks()).div_euclid(frame);
        Self {
            start: clip.start,
            rate,
            first: ceil(lo),
            last: (ceil(hi) - 1).max(ceil(lo)),
        }
    }

    fn times(&self) -> impl Iterator<Item = Time> + '_ {
        (self.first..=self.last).map(|frame| self.rate.time_of(frame) - self.start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_sequences_show_their_own_frames() {
        let rate = Rate::FPS_24;
        let at = |frames: i64| rate.time_of(frames);
        // Forward from frame 10: clip frame 5 shows nested frame 15.
        assert_eq!(nested_time(at(5), at(10), Ratio::ONE, at(48), rate), at(15));
        // Half speed: clip frame 5 is nested frame 12.5, so 12 shows.
        let half = Ratio::parse("1/2").unwrap();
        assert_eq!(nested_time(at(5), at(10), half, at(48), rate), at(12));
        // Reversed over [10, 58): the first frame is 57.
        let back = Ratio::parse("-1").unwrap();
        assert_eq!(nested_time(Time::ZERO, at(10), back, at(48), rate), at(57));
        assert_eq!(nested_time(at(47), at(10), back, at(48), rate), at(10));
    }
}
