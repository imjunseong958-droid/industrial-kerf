//! A Kerf sequence as an EffectCraft project, built in memory for a
//! render and never saved.
//!
//! - A sequence is a comp at its canvas size and rate. Picture tracks
//!   stack in sequence order, so a later track plays over an earlier one,
//!   and captions play over them all.
//! - A media clip is a footage layer of its own footage item. Its layer
//!   time is clip time; the footage source works out the media frame from
//!   the clip's source in and speed (see `footage`). A nested sequence is
//!   a precomp layer that time remapping maps onto the nested comp's time.
//! - A clip's picture is fitted to the canvas as the graph fits it, then
//!   cropped (a mask), scaled, flipped, rotated about its anchor, and
//!   moved. Animated properties are sampled at every frame the layer shows
//!   into linear keys, so each frame is exactly what Kerf evaluates. Media
//!   left where fitting puts it is letterboxed with black, as the graph
//!   pads it. Media never shown at its full size is decoded at the size it
//!   is shown, as the graph scales it before overlaying it, unless a
//!   raster effect works on its pixels.
//! - Effects run in the clip's order in layer space: raster effects as
//!   EffectCraft built them, `color.basic` as a plug-in with the graph's
//!   arithmetic, `blur.gaussian` as Gaussian Blur at the same size on
//!   screen.
//! - A dissolve fades the incoming clip in over the outgoing one, a wipe
//!   reveals it with Linear Wipe, and a dip fades a solid through. A raster
//!   transition plays the outgoing clip over the incoming one while its
//!   effect completes, as EffectCraft's transitions work.
//! - Titles are text layers in a comp the size of the canvas, solids are
//!   solid layers, and an adjustment layer is an adjustment solid carrying
//!   the clip's effects over everything beneath it.
//! - A clip's masks are the layer's masks, in the clip's order, ahead of a
//!   crop's, which intersects them; an effect's mask parameter is the
//!   mask's place among them.
//! - A track an effect reads (a track parameter, as a displacement map or a
//!   matte) is drawn on its own in a comp of its own, whose layer sits out
//!   of sight at the bottom of the comp for the effect to read, even when
//!   the track itself is hidden.

use crate::color;
use crate::footage::{frame_count, ClipSource, TICKS_PER_FLICK};
use crate::ComposeError;
use effectcraft_color::{BlendMode, Label};
use effectcraft_effects::EffectSpec;
use effectcraft_keyframe::{Interp as FxInterp, Justify, ShapePath, TextDoc};
use effectcraft_project::build::{self, Ids};
use effectcraft_project::{
    AlphaMode, BitDepth, Comp, Footage, FootageKind, GroupKind, ItemId, ItemKind,
    Keyframe as FxKeyframe, Layer, LayerId, LayerSource, MaskMode, ParamUi, Project as FxProject,
    Property, Solid, Value as FxValue,
};
use effectcraft_time::{FrameRate, Tick, TICKS_PER_SECOND};
use kerf_core::program::{canvas, program_sequence};
use kerf_model::{
    Catalog, Clip, Content, Domain, Engine, FxKind, Id, Mask, MaskShape, Param, ParamType, Project,
    Rate, Rgba, Sequence, Time, Track, TrackKind, Transition, Value,
};
use std::collections::HashMap;
use std::sync::Arc;

/// Nested sequences deeper than this are refused, as the graph refuses
/// them.
const MAX_NESTING: usize = 16;

/// Nested comps are shown a hair after the source time they map to, so a
/// frame that starts exactly there is not lost to rounding in seconds.
const NESTED_LEAD: i64 = 1_000;

/// The family a title's generic font names stand for.
const FONTS: &[(&str, &str)] = &[
    ("sans-serif", "Inter"),
    ("serif", "Noto Serif"),
    ("monospace", "JetBrains Mono"),
];

/// A sequence ready to render.
pub(crate) struct Scene {
    pub project: FxProject,
    pub comp: ItemId,
    pub width: u32,
    pub height: u32,
    pub rate: Rate,
    pub duration: Time,
    pub clips: HashMap<ItemId, ClipSource>,
    /// The top comp's layers that are their media's picture as decoded,
    /// with the footage each shows (see `Builder::plain`).
    pub plain: HashMap<LayerId, ItemId>,
}

type Result<T> = std::result::Result<T, ComposeError>;

/// `sequence` of `project` (the first when absent) as a scene.
pub(crate) fn build(
    project: &Project,
    catalog: &Catalog,
    sequence: Option<&str>,
    captions: bool,
) -> Result<Scene> {
    let sequence = program_sequence(project, sequence)
        .map_err(|error| ComposeError::Invalid(error.to_string()))?;
    let mut out = FxProject::default();
    // Pictures are worked on in floating point: no banding, and no rounding
    // between steps.
    out.settings.bit_depth = BitDepth::Bpc32;
    let mut builder = Builder {
        project,
        catalog,
        out,
        clips: HashMap::new(),
        nesting: vec![sequence.id.clone()],
        captions,
        effects: 0,
        plain: HashMap::new(),
        track_layers: HashMap::new(),
    };
    let comp = builder.comp(sequence)?;
    let (width, height) = canvas(sequence);
    Ok(Scene {
        project: builder.out,
        comp,
        width,
        height,
        rate: sequence.rate,
        duration: sequence.duration(),
        clips: builder.clips,
        plain: builder.plain,
    })
}

pub(crate) fn ticks(time: Time) -> Tick {
    Tick(time.flicks() * TICKS_PER_FLICK)
}

fn frame_rate(rate: Rate) -> FrameRate {
    FrameRate {
        num: i64::from(rate.num()),
        den: i64::from(rate.den()),
    }
}

/// How a clip enters over the clip before it.
#[derive(Clone, Copy)]
enum Reveal {
    Dissolve,
    Wipe { angle: f64, softness: f64 },
}

/// A clip as a track shows it: its span with transition handles, and the
/// transitions it plays through.
struct Shown<'a> {
    clip: &'a Clip,
    from: Time,
    to: Time,
    reveal: Option<(Reveal, Time, Time)>,
    /// It plays under the clip after it through a dissolve or wipe.
    fades_out: bool,
    /// The raster transition it leaves by, over the clip after it.
    leave: Option<(&'a Transition, Time, Time)>,
    /// It leaves by a raster transition, which the graph bakes into it.
    raster_out: bool,
}

/// A dip's color between the two clips of its cut.
struct Dip {
    color: Rgba,
    from: Time,
    to: Time,
}

/// What a picture layer shows and how it fits the canvas.
struct Picture {
    source: LayerSource,
    size: (u32, u32),
    /// Fitted to the canvas, keeping its aspect, as media and nested
    /// sequences are.
    fit: bool,
    /// Kept transparent where the source is (a solid's color alpha).
    opacity: f64,
    /// Maps the layer onto a nested comp's time.
    remap: Option<Remap>,
}

/// Source positions of a nested sequence at the start of its clip.
#[derive(Clone, Copy)]
enum Remap {
    /// A nested sequence played from `source_in` at `speed`.
    Nested {
        source_in: Time,
        speed: kerf_model::Ratio,
        duration: Time,
    },
    /// A comp shown at its first frame throughout, as a title is.
    Still,
    /// A comp whose time starts where the layer does.
    Shift,
}

struct Builder<'a> {
    project: &'a Project,
    catalog: &'a Catalog,
    out: FxProject,
    clips: HashMap<ItemId, ClipSource>,
    nesting: Vec<Id>,
    captions: bool,
    /// Effect groups made so far, for their uids (see `effect_group`).
    effects: u64,
    plain: HashMap<LayerId, ItemId>,
    /// The layers of the tracks effects read in the comp being built, by
    /// track id (see `comp_of`).
    track_layers: HashMap<String, LayerId>,
}

impl<'a> Builder<'a> {
    fn comp(&mut self, sequence: &'a Sequence) -> Result<ItemId> {
        self.comp_of(sequence, None)
    }

    /// `sequence` as a comp: its enabled picture tracks, or `only` that
    /// track, shown or not, for the effects that read it.
    fn comp_of(&mut self, sequence: &'a Sequence, only: Option<&'a Track>) -> Result<ItemId> {
        let (width, height) = canvas(sequence);
        let rate = sequence.rate;
        let duration = sequence.duration().max(rate.frame_duration());
        let mut comp = Comp::new(width, height, frame_rate(rate), ticks(duration));
        // Nested comps show their own frames, as the graph composites a
        // nested sequence at its rate.
        comp.preserve_frame_rate = true;
        let built = |track: &Track| {
            track.kind == TrackKind::Video
                && only.map_or(track.enabled, |only| std::ptr::eq(only, track))
        };
        // The tracks effects here read come first, at the bottom and out of
        // sight; effects find them by track id.
        let outer = std::mem::take(&mut self.track_layers);
        let mut layers = Vec::new();
        for read in self.read_tracks(sequence, &built) {
            if self.nesting.len() >= MAX_NESTING {
                return Err(unsupported(
                    &read.id,
                    "tracks read through this many others",
                ));
            }
            self.nesting.push(sequence.id.clone());
            let item = self.comp_of(sequence, Some(read));
            self.nesting.pop();
            let source = LayerSource::Comp { item: item? };
            let mut layer = build::layer(
                &mut self.out,
                &comp,
                read.id.as_str(),
                source,
                (width, height),
                None,
            );
            layer.switches.video = false;
            layer.switches.audio = false;
            layer.switches.motion_blur = false;
            self.track_layers.insert(read.id.to_string(), layer.id);
            layers.push(layer);
        }
        for track in &sequence.tracks {
            if built(track) {
                self.track(sequence, &comp, track, &mut layers)?;
            }
        }
        self.track_layers = outer;
        if self.captions && only.is_none() {
            for track in &sequence.tracks {
                if track.kind == TrackKind::Subtitle && track.enabled {
                    for clip in &track.clips {
                        if let Some(layer) = self.caption(&comp, clip) {
                            layers.push(layer);
                        }
                    }
                }
            }
        }
        // EffectCraft lists the top layer first.
        layers.reverse();
        comp.layers = layers;
        let name = match only {
            Some(track) => format!("{} {}", sequence.name, track.id),
            None => sequence.name.clone(),
        };
        Ok(self
            .out
            .add_item(&name, Label::None, None, ItemKind::Comp(Arc::new(comp))))
    }

    /// The picture tracks of `sequence` that the effects and transitions on
    /// the `built` tracks read, each once.
    fn read_tracks(
        &self,
        sequence: &'a Sequence,
        built: &dyn Fn(&Track) -> bool,
    ) -> Vec<&'a Track> {
        let mut read: Vec<&'a Track> = Vec::new();
        let mut uses: Vec<(&Id, &kerf_model::Params)> = Vec::new();
        for track in sequence.tracks.iter().filter(|track| built(track)) {
            let clips = track.clips.iter().filter(|clip| clip.enabled);
            let effects = clips
                .flat_map(|clip| &clip.effects)
                .filter(|effect| effect.enabled);
            uses.extend(effects.map(|effect| (&effect.fx, &effect.params)));
            uses.extend(
                track
                    .transitions
                    .iter()
                    .map(|transition| (&transition.fx, &transition.params)),
            );
        }
        for (fx, params) in uses {
            let Some(def) = self.catalog.fx(fx.as_str()) else {
                continue;
            };
            for param in def
                .params
                .iter()
                .filter(|param| param.ty == ParamType::Track)
            {
                let Some(Param::Static(Value::Text(id))) = params.get(&param.name) else {
                    continue;
                };
                let track = sequence
                    .tracks
                    .iter()
                    .find(|track| track.kind == TrackKind::Video && track.id == id.as_str());
                if let Some(track) =
                    track.filter(|track| !read.iter().any(|r| std::ptr::eq(*r, *track)))
                {
                    read.push(track);
                }
            }
        }
        read
    }

    fn track(
        &mut self,
        sequence: &'a Sequence,
        comp: &Comp,
        track: &'a Track,
        layers: &mut Vec<Layer>,
    ) -> Result<()> {
        let (items, dips) = self.shown(track, sequence.rate)?;
        for item in &items {
            if !item.clip.enabled {
                continue;
            }
            if matches!(&item.clip.content, Content::Generator { fx, .. } if fx == "adjustment") {
                if item.reveal.is_some() || item.fades_out || item.leave.is_some() {
                    return Err(unsupported(
                        &item.clip.id,
                        "a transition on an adjustment layer",
                    ));
                }
                layers.push(self.adjustment(comp, item.clip)?);
                continue;
            }
            let Some((lo, hi)) = self.span(item) else {
                continue;
            };
            if let Some(layer) = self.picture(comp, item, lo, hi)? {
                layers.push(layer);
            }
        }
        for dip in dips {
            layers.push(self.dip(comp, &dip));
        }
        Ok(())
    }

    /// The clips of a track with their transitions, as the graph shows
    /// them.
    fn shown(&self, track: &'a Track, rate: Rate) -> Result<(Vec<Shown<'a>>, Vec<Dip>)> {
        let mut shown: Vec<Shown<'a>> = track
            .clips
            .iter()
            .map(|clip| Shown {
                clip,
                from: clip.start,
                to: clip.end(),
                reveal: None,
                fades_out: false,
                leave: None,
                raster_out: false,
            })
            .collect();
        let mut dips = Vec::new();
        let mut over = Vec::new();
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
                    shown[from].to = middle;
                    shown[to].from = middle;
                    dips.push(Dip {
                        color: self.color(&transition.id, &transition.params, "dip", "color")?,
                        from: start,
                        to: end,
                    });
                    continue;
                }
                "dissolve" | "crossfade" => Reveal::Dissolve,
                "wipe" => Reveal::Wipe {
                    angle: self.static_number(
                        &transition.id,
                        &transition.params,
                        "wipe",
                        "angle",
                    )?,
                    softness: self.static_number(
                        &transition.id,
                        &transition.params,
                        "wipe",
                        "softness",
                    )?,
                },
                other if self.raster_transition(other) => {
                    // The outgoing clip plays over the incoming one until
                    // its effect has taken it away.
                    shown[from].to = start;
                    shown[from].raster_out = true;
                    shown[to].from = start;
                    over.push(Shown {
                        clip: &track.clips[from],
                        from: start,
                        to: end,
                        reveal: None,
                        fades_out: false,
                        leave: Some((transition, start, end)),
                        raster_out: true,
                    });
                    continue;
                }
                other => {
                    return Err(unsupported(&transition.id, format!("transition `{other}`")));
                }
            };
            shown[from].to = end;
            shown[from].fades_out = true;
            shown[to].from = start;
            shown[to].reveal = Some((reveal, start, end));
        }
        shown.extend(over);
        Ok((shown, dips))
    }

    fn raster_transition(&self, fx: &str) -> bool {
        self.catalog
            .fx(fx)
            .is_some_and(|def| def.engine == Engine::Raster && def.kind == FxKind::Transition)
    }

    /// The part of a shown clip its source covers, as the graph clamps it.
    fn span(&self, item: &Shown) -> Option<(Time, Time)> {
        let clip = item.clip;
        let (mut lo, mut hi) = (item.from, item.to);
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

    // ---- layers ----------------------------------------------------------------

    fn picture(&mut self, comp: &Comp, item: &Shown, lo: Time, hi: Time) -> Result<Option<Layer>> {
        let clip = item.clip;
        let canvas = (comp.width, comp.height);
        let Some(mut picture) = self.source(clip, canvas)? else {
            return Ok(None);
        };
        let frames = Frames::new(comp, clip, lo, hi);
        let letterboxed = self.letterboxed(item, &picture, canvas);
        if letterboxed {
            picture = self.letterbox(comp, clip, &frames, picture, lo, hi)?;
        }
        let mut layer = self.layer(comp, clip, picture.source.clone(), picture.size, lo, hi);
        let geometry = if letterboxed {
            Geometry {
                scale: Curve::Constant([1.0]),
                largest: 1.0,
            }
        } else {
            let geometry =
                self.geometry(&mut layer, clip, &frames, picture.size, picture.fit, canvas)?;
            self.masks(&mut layer, clip, &frames, picture.size);
            if !self.baked(item) {
                self.decode_as_shown(&picture.source, &geometry);
            }
            geometry
        };
        let reveal = item.reveal;
        let alpha = picture.opacity;
        self.opacity(&mut layer, clip, &frames, &|_, time| {
            let progress = match reveal {
                Some((Reveal::Dissolve, start, end)) => progress(time, start, end),
                _ => 1.0,
            };
            alpha * progress
        });
        layer.blend_mode = self.blend(clip)?;
        let mut seeds = Seeds::new(clip);
        self.effects(
            &mut layer,
            clip,
            &frames,
            picture.size,
            &geometry,
            &mut seeds,
        )?;
        if let Some((Reveal::Wipe { angle, softness }, start, end)) = reveal {
            self.wipe(&mut layer, clip, picture.size, angle, softness, start, end);
        }
        if let Some((transition, start, end)) = item.leave {
            self.leave(
                &mut layer,
                clip,
                &frames,
                picture.size,
                transition,
                start,
                end,
                &mut seeds,
            )?;
        }
        if let Some(remap) = picture.remap {
            self.remap(&mut layer, clip, remap, lo, hi);
        }
        if let (true, LayerSource::Footage { item: footage }) =
            (self.plain(item, &picture, canvas), &picture.source)
        {
            self.plain.insert(layer.id, *footage);
        }
        Ok(Some(layer))
    }

    /// Whether a clip's layer in the top comp is its media's picture as
    /// decoded: opaque, the size of the canvas, unmoved and untouched. A
    /// frame showing it alone is that picture, and is read as it is.
    fn plain(&self, item: &Shown, picture: &Picture, canvas: (u32, u32)) -> bool {
        let clip = item.clip;
        let LayerSource::Footage { item: footage } = picture.source else {
            return false;
        };
        let is = |name: &str, value: f64| matches!(self.param(clip, name), Param::Static(Value::Number(n)) if n == value);
        let normal = matches!(
            self.param(clip, "blend"),
            Param::Static(Value::Enum(blend) | Value::Text(blend)) if blend == "normal"
        );
        self.nesting.len() == 1
            && picture.size == canvas
            && normal
            && item.reveal.is_none()
            && item.leave.is_none()
            && self.clips.get(&footage).is_some_and(|source| !source.alpha)
            && clip.effects.iter().all(|effect| !effect.enabled)
            && clip.masks.is_empty()
            && [
                "x", "y", "rotation", "anchor_x", "anchor_y", "crop_l", "crop_r", "crop_t",
                "crop_b",
            ]
            .iter()
            .all(|name| is(name, 0.0))
            && ["scale", "scale_x", "scale_y", "opacity"]
                .iter()
                .all(|name| is(name, 1.0))
    }

    /// Whether the graph pads a clip's picture to the canvas with black:
    /// opaque media left where fitting puts it (flipped and cropped
    /// included), with no raster effect baked into it. Its bars then cover
    /// the tracks below, and its opacity, effects, and transitions take
    /// them along.
    fn letterboxed(&self, item: &Shown, picture: &Picture, canvas: (u32, u32)) -> bool {
        let clip = item.clip;
        let LayerSource::Footage { item: footage } = picture.source else {
            return false;
        };
        // A picture that fills the canvas leaves no bars to draw.
        let crop =
            ["crop_l", "crop_r", "crop_t", "crop_b"].map(|name| match self.param(clip, name) {
                Param::Static(Value::Number(edge)) => Some(edge),
                _ => None,
            });
        if let [Some(l), Some(r), Some(t), Some(b)] = crop {
            let (sw, sh) = (f64::from(picture.size.0), f64::from(picture.size.1));
            let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
            let (fw, fh) = fitted(sw * (1.0 - l - r), sh * (1.0 - t - b), cw, ch);
            if fw >= cw && fh >= ch {
                return false;
            }
        }
        let opaque = self.clips.get(&footage).is_some_and(|source| !source.alpha);
        let is = |name: &str, value: f64| matches!(self.param(clip, name), Param::Static(Value::Number(n)) if n == value);
        let unit = |axis: &str| match (self.param(clip, "scale"), self.param(clip, axis)) {
            (Param::Static(Value::Number(scale)), Param::Static(Value::Number(factor))) => {
                ((scale * factor).abs() - 1.0).abs() < 5e-7
            }
            _ => false,
        };
        // Masks cut the picture, not bars around it.
        opaque
            && clip.masks.is_empty()
            && !self.baked(item)
            && ["x", "y", "rotation", "anchor_x", "anchor_y"]
                .iter()
                .all(|name| is(name, 0.0))
            && unit("scale_x")
            && unit("scale_y")
    }

    /// Whether a raster effect works on a shown clip's pixels, which the
    /// graph bakes into its picture at the picture's own size.
    fn baked(&self, item: &Shown) -> bool {
        item.raster_out
            || item.clip.effects.iter().any(|effect| {
                effect.enabled
                    && self
                        .catalog
                        .fx(effect.fx.as_str())
                        .is_some_and(|fx| fx.engine == Engine::Raster)
            })
    }

    /// Decode a media layer's frames at the most it is shown at, when that
    /// is less than their size.
    fn decode_as_shown(&mut self, source: &LayerSource, geometry: &Geometry) {
        if let LayerSource::Footage { item } = source {
            if let Some(clip) = self.clips.get_mut(item) {
                clip.shown = (geometry.largest < 1.0).then_some(geometry.largest);
            }
        }
    }

    /// A clip's fitted picture on black the size of the canvas, in a comp
    /// whose time starts at `lo`.
    fn letterbox(
        &mut self,
        comp: &Comp,
        clip: &Clip,
        frames: &Frames,
        picture: Picture,
        lo: Time,
        hi: Time,
    ) -> Result<Picture> {
        let canvas = (comp.width, comp.height);
        let mut inner = Comp::new(
            canvas.0,
            canvas.1,
            comp.frame_rate,
            Tick(ticks(hi - lo).0 + 1),
        );
        let item = self.solid("letterbox", Rgba::BLACK, canvas);
        let bars = build::layer(
            &mut self.out,
            &inner,
            "letterbox",
            LayerSource::Solid { item },
            canvas,
            None,
        );
        let mut layer = build::layer(
            &mut self.out,
            &inner,
            clip.id.as_str(),
            picture.source.clone(),
            picture.size,
            None,
        );
        // The picture's layer time is still clip time.
        layer.start_time = ticks(clip.start - lo);
        layer.switches.motion_blur = false;
        let geometry = self.geometry(&mut layer, clip, frames, picture.size, true, canvas)?;
        // Its effects run on the letterboxed picture, not on the media.
        self.decode_as_shown(&picture.source, &geometry);
        inner.layers = vec![layer, bars];
        let item = self.out.add_item(
            clip.id.as_str(),
            Label::None,
            None,
            ItemKind::Comp(Arc::new(inner)),
        );
        Ok(Picture {
            source: LayerSource::Comp { item },
            size: canvas,
            fit: false,
            opacity: picture.opacity,
            remap: Some(Remap::Shift),
        })
    }

    /// What a clip's layer shows, or nothing for a clip without a picture.
    fn source(&mut self, clip: &Clip, canvas: (u32, u32)) -> Result<Option<Picture>> {
        Ok(Some(match &clip.content {
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
                let footage = Footage {
                    path: media.path.clone(),
                    kind: FootageKind::Video,
                    width: video.width,
                    height: video.height,
                    pixel_aspect: 1.0,
                    frame_rate: frame_rate(media.rate),
                    duration: ticks(media.duration),
                    has_video: true,
                    has_audio: false,
                    alpha: if video.alpha {
                        AlphaMode::Straight
                    } else {
                        AlphaMode::Ignore
                    },
                    ..Footage::default()
                };
                let item = self.out.add_item(
                    clip.id.as_str(),
                    Label::None,
                    None,
                    ItemKind::Footage(footage),
                );
                self.clips.insert(
                    item,
                    ClipSource {
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
                    },
                );
                Picture {
                    source: LayerSource::Footage { item },
                    size: (video.width, video.height),
                    fit: true,
                    opacity: 1.0,
                    remap: None,
                }
            }
            Content::Sequence {
                sequence,
                source_in,
                speed,
            } => {
                if self.nesting.len() >= MAX_NESTING || self.nesting.contains(sequence) {
                    return Err(unsupported(&clip.id, "sequences nested this deep"));
                }
                let nested = self.project.sequence(sequence.as_str()).ok_or_else(|| {
                    ComposeError::Invalid(format!(
                        "clip `{}` uses missing sequence `{sequence}`",
                        clip.id
                    ))
                })?;
                self.nesting.push(nested.id.clone());
                let item = self.comp(nested);
                self.nesting.pop();
                Picture {
                    source: LayerSource::Comp { item: item? },
                    size: kerf_core::program::canvas(nested),
                    fit: true,
                    opacity: 1.0,
                    remap: Some(Remap::Nested {
                        source_in: *source_in,
                        speed: *speed,
                        duration: clip.duration,
                    }),
                }
            }
            Content::Generator { fx, params } => match fx.as_str() {
                "solid" => {
                    let color = self.color(&clip.id, params, "solid", "color")?;
                    let item = self.solid(clip.id.as_str(), color, canvas);
                    Picture {
                        source: LayerSource::Solid { item },
                        size: canvas,
                        fit: false,
                        opacity: f64::from(color.a) / 255.0,
                        remap: None,
                    }
                }
                "title" => Picture {
                    source: LayerSource::Comp {
                        item: self.title(clip, params, canvas)?,
                    },
                    size: canvas,
                    fit: false,
                    opacity: 1.0,
                    remap: Some(Remap::Still),
                },
                other => return Err(unsupported(&clip.id, format!("generator `{other}`"))),
            },
            Content::Caption { .. } => return Ok(None),
        }))
    }

    fn layer(
        &mut self,
        comp: &Comp,
        clip: &Clip,
        source: LayerSource,
        size: (u32, u32),
        lo: Time,
        hi: Time,
    ) -> Layer {
        let mut layer = build::layer(&mut self.out, comp, clip.id.as_str(), source, size, None);
        // Layer time is clip time.
        layer.start_time = ticks(clip.start);
        layer.in_point = ticks(lo);
        layer.out_point = ticks(hi);
        layer.switches.motion_blur = false;
        layer
    }

    fn solid(&mut self, name: &str, color: Rgba, (width, height): (u32, u32)) -> ItemId {
        let channel = |value: u8| f32::from(value) / 255.0;
        self.out.add_item(
            name,
            Label::None,
            None,
            ItemKind::Solid(Solid {
                color: [channel(color.r), channel(color.g), channel(color.b)],
                width,
                height,
                pixel_aspect: 1.0,
            }),
        )
    }

    /// A title: its text on a transparent comp the size of the canvas, as
    /// the graph draws it. Text is centered on the frame, begins at the
    /// center when aligned left, and ends there when aligned right.
    fn title(
        &mut self,
        clip: &Clip,
        params: &kerf_model::Params,
        (width, height): (u32, u32),
    ) -> Result<ItemId> {
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
        let text = match get("text")? {
            Value::Text(text) => text,
            _ => String::new(),
        };
        let size = get("size")?.as_number().unwrap_or(0.08) * f64::from(height);
        let color = match get("color")? {
            Value::Color(color) => color,
            _ => Rgba::WHITE,
        };
        let justify = match get("align")? {
            Value::Enum(align) if align == "left" => Justify::Left,
            Value::Enum(align) if align == "right" => Justify::Right,
            _ => Justify::Center,
        };
        let family = match get("font")? {
            Value::Text(font) if !font.trim().is_empty() => font,
            _ => "sans-serif".to_owned(),
        };
        let family = FONTS
            .iter()
            .find(|(generic, _)| generic.eq_ignore_ascii_case(&family))
            .map_or(family.clone(), |(_, family)| (*family).to_owned());
        let doc = TextDoc {
            text,
            font: family,
            size: size.max(1.0),
            fill: rgba(color),
            justify,
            ..TextDoc::default()
        };
        let mut comp = Comp::new(
            width,
            height,
            FrameRate { num: 1, den: 1 },
            Tick(TICKS_PER_SECOND),
        );
        let center = f64::from(height) / 2.0;
        let layer = self.text_layer(
            &comp,
            clip.id.as_str(),
            doc,
            [f64::from(width) / 2.0, center],
            0.5,
        );
        comp.layers = vec![layer];
        Ok(self.out.add_item(
            clip.id.as_str(),
            Label::None,
            None,
            ItemKind::Comp(Arc::new(comp)),
        ))
    }

    /// A text layer whose lines sit with `place` of their height above
    /// `at`'s height: a half centers them on it, 1 puts their bottom there.
    fn text_layer(
        &mut self,
        comp: &Comp,
        name: &str,
        doc: TextDoc,
        at: [f64; 2],
        place: f64,
    ) -> Layer {
        let bounds = effectcraft_text::layout_doc(&doc).bounds;
        let baseline = at[1] - (bounds[1] + (bounds[3] - bounds[1]) * place);
        let mut layer = build::layer(
            &mut self.out,
            comp,
            name,
            LayerSource::Text,
            (comp.width, comp.height),
            None,
        );
        layer.in_point = Tick(i64::MIN / 4);
        layer.out_point = Tick(i64::MAX / 4);
        if let Some(text) = layer.props.prop_mut("text/sourceText") {
            text.value = FxValue::Text(Box::new(doc));
        }
        if let Some(position) = layer.props.prop_mut("transform/position") {
            position.value = FxValue::Vec3([at[0], baseline, 0.0]);
        }
        layer
    }

    /// A caption over the picture: white with a dark edge, centered near
    /// the bottom of the frame.
    fn caption(&mut self, comp: &Comp, clip: &Clip) -> Option<Layer> {
        let Content::Caption { text } = &clip.content else {
            return None;
        };
        if !clip.enabled || text.trim().is_empty() {
            return None;
        }
        let height = f64::from(comp.height);
        let size = (height * 0.045).max(12.0);
        let doc = TextDoc {
            text: text.clone(),
            size,
            fill: [1.0; 4],
            stroke: [0.0, 0.0, 0.0, 0.85],
            stroke_width: size * 0.12,
            apply_stroke: true,
            stroke_over_fill: false,
            justify: Justify::Center,
            ..TextDoc::default()
        };
        let at = [f64::from(comp.width) / 2.0, height * 0.95];
        let mut layer = self.text_layer(comp, clip.id.as_str(), doc, at, 1.0);
        layer.in_point = ticks(clip.start);
        layer.out_point = ticks(clip.end());
        Some(layer)
    }

    fn adjustment(&mut self, comp: &Comp, clip: &Clip) -> Result<Layer> {
        let canvas = (comp.width, comp.height);
        let item = self.solid(clip.id.as_str(), Rgba::BLACK, canvas);
        let mut layer = self.layer(
            comp,
            clip,
            LayerSource::Solid { item },
            canvas,
            clip.start,
            clip.end(),
        );
        layer.switches.adjustment = true;
        let frames = Frames::new(comp, clip, clip.start, clip.end());
        let geometry = self.geometry(&mut layer, clip, &frames, canvas, false, canvas)?;
        self.masks(&mut layer, clip, &frames, canvas);
        self.opacity(&mut layer, clip, &frames, &|_, _| 1.0);
        layer.blend_mode = self.blend(clip)?;
        let mut seeds = Seeds::new(clip);
        self.effects(&mut layer, clip, &frames, canvas, &geometry, &mut seeds)?;
        Ok(layer)
    }

    /// A dip's solid over the track, fading in to the cut and out after.
    fn dip(&mut self, comp: &Comp, dip: &Dip) -> Layer {
        let canvas = (comp.width, comp.height);
        let item = self.solid("dip", dip.color, canvas);
        let mut layer = build::layer(
            &mut self.out,
            comp,
            "dip",
            LayerSource::Solid { item },
            canvas,
            None,
        );
        layer.start_time = ticks(dip.from);
        layer.in_point = ticks(dip.from);
        layer.out_point = ticks(dip.to);
        let half = Time::from_flicks((dip.to - dip.from).flicks() / 2);
        let alpha = f64::from(dip.color.a) / 255.0 * 100.0;
        let key = |at: Time, value: f64| FxKeyframe::new(ticks(at), FxValue::Scalar(value));
        if let Some(opacity) = layer.props.prop_mut("transform/opacity") {
            opacity.value = FxValue::Scalar(0.0);
            opacity.keys = vec![
                key(Time::ZERO, 0.0),
                key(half, alpha),
                key(half + half, 0.0),
            ];
        }
        layer
    }

    // ---- properties --------------------------------------------------------------

    /// A clip property, or its catalog default.
    fn param(&self, clip: &Clip, name: &str) -> Param {
        clip.props.get(name).cloned().unwrap_or_else(|| {
            Param::Static(
                self.catalog
                    .prop(Domain::Video, name)
                    .map(|def| def.default.clone())
                    .unwrap_or(Value::Number(0.0)),
            )
        })
    }

    /// Crop, fit, scale, flip, rotation, anchor, and position. Returns how
    /// many canvas pixels a layer pixel covers.
    fn geometry(
        &mut self,
        layer: &mut Layer,
        clip: &Clip,
        frames: &Frames,
        size: (u32, u32),
        fit: bool,
        canvas: (u32, u32),
    ) -> Result<Geometry> {
        let names = [
            "crop_l", "crop_r", "crop_t", "crop_b", "scale", "scale_x", "scale_y", "rotation",
            "anchor_x", "anchor_y", "x", "y",
        ];
        let params = names.map(|name| self.param(clip, name));
        let animated = params
            .iter()
            .any(|param| matches!(param, Param::Animated(_)));
        let at = |time: Time| -> [f64; 12] {
            std::array::from_fn(|index| number(&params[index].value_at(time)))
        };
        for time in frames.times() {
            let [l, r, t, b, ..] = at(time);
            if l + r >= 1.0 || t + b >= 1.0 {
                return Err(ComposeError::Invalid(format!(
                    "clip `{}` is cropped away entirely",
                    clip.id
                )));
            }
            if !animated {
                break;
            }
        }
        let (sw, sh) = (f64::from(size.0), f64::from(size.1));
        let (cw, ch) = (f64::from(canvas.0), f64::from(canvas.1));
        let place = |time: Time| -> [f64; 11] {
            let [l, r, t, b, s, sx, sy, rotation, ax, ay, x, y] = at(time);
            let (width, height) = (sw * (1.0 - l - r), sh * (1.0 - t - b));
            let (fw, fh) = if fit {
                fitted(width, height, cw, ch)
            } else {
                (width, height)
            };
            let (kx, ky) = (s * sx, s * sy);
            let mut position = [cw / 2.0 + x * cw + ax * fw, ch / 2.0 + y * ch + ay * fh];
            let mut scale = [fw / width * kx, fh / height * ky];
            if !animated && rotation == 0.0 && ax == 0.0 && ay == 0.0 {
                // A still picture lands on whole pixels.
                let (ow, oh) = ((fw * kx.abs()).round(), (fh * ky.abs()).round());
                position = [
                    (position[0] - ow / 2.0 + 1e-9).floor() + ow / 2.0,
                    (position[1] - oh / 2.0 + 1e-9).floor() + oh / 2.0,
                ];
                scale = [ow / width * kx.signum(), oh / height * ky.signum()];
            }
            [
                sw * l + width * (0.5 + ax),
                sh * t + height * (0.5 + ay),
                position[0],
                position[1],
                scale[0] * 100.0,
                scale[1] * 100.0,
                rotation,
                l,
                r,
                t,
                b,
            ]
        };
        let curve = frames.curve(animated, place);
        set(layer, "transform/anchor", &curve, |v| {
            FxValue::Vec3([v[0], v[1], 0.0])
        });
        set(layer, "transform/position", &curve, |v| {
            FxValue::Vec3([v[2], v[3], 0.0])
        });
        set(layer, "transform/scale", &curve, |v| {
            FxValue::Vec3([v[4], v[5], 100.0])
        });
        set(layer, "transform/rotation", &curve, |v| {
            FxValue::Scalar(v[6])
        });
        if curve
            .values()
            .any(|v| v[7..11].iter().any(|edge| *edge != 0.0))
        {
            // A crop is a mask around what is left, in source pixels. It
            // intersects the clip's own masks; alone, it covers as Add does.
            let mask = build::mask(
                &mut Ids(&mut self.out.next_id),
                "crop",
                ShapePath::rect([0.0; 2], 0.0, 0.0),
                MaskMode::Intersect,
                [255; 3],
            );
            if let Some(masks) = layer.props.sub_mut("masks") {
                masks.children.push(mask.into());
            }
            set(layer, "masks/crop/path", &curve, |v| {
                let (left, right) = (sw * v[7], sw * (1.0 - v[8]));
                let (top, bottom) = (sh * v[9], sh * (1.0 - v[10]));
                FxValue::Path(ShapePath::rect(
                    [(left + right) / 2.0, (top + bottom) / 2.0],
                    right - left,
                    bottom - top,
                ))
            });
        }
        Ok(Geometry {
            scale: curve.map(|v| (v[4] * v[5]).abs().sqrt() / 100.0),
            // Between keys the scale runs straight, so its largest is at one.
            largest: curve
                .values()
                .map(|v| v[4].abs().max(v[5].abs()) / 100.0)
                .fold(0.0, f64::max),
        })
    }

    /// Opacity, with what a transition adds: `extra` at clip time and
    /// sequence time.
    fn opacity(
        &self,
        layer: &mut Layer,
        clip: &Clip,
        frames: &Frames,
        extra: &dyn Fn(Time, Time) -> f64,
    ) {
        let opacity = self.param(clip, "opacity");
        let fades = frames.times().any(|time| {
            extra(time, time + clip.start) != extra(frames.first(), frames.first() + clip.start)
        });
        let animated = matches!(opacity, Param::Animated(_)) || fades;
        let curve = frames.curve(animated, |time| {
            [number(&opacity.value_at(time)) * extra(time, time + clip.start) * 100.0]
        });
        set(layer, "transform/opacity", &curve, |v| {
            FxValue::Scalar(v[0])
        });
    }

    fn blend(&self, clip: &Clip) -> Result<BlendMode> {
        let blend = match self.param(clip, "blend") {
            Param::Static(Value::Enum(blend) | Value::Text(blend)) => blend,
            Param::Static(_) => "normal".to_owned(),
            Param::Animated(_) => return Err(unsupported(&clip.id, "animated `blend`")),
        };
        Ok(match blend.as_str() {
            "normal" => BlendMode::Normal,
            "add" => BlendMode::Add,
            "multiply" => BlendMode::Multiply,
            "screen" => BlendMode::Screen,
            "overlay" => BlendMode::Overlay,
            "darken" => BlendMode::Darken,
            "lighten" => BlendMode::Lighten,
            "difference" => BlendMode::Difference,
            other => return Err(unsupported(&clip.id, format!("blend `{other}`"))),
        })
    }

    // ---- effects -----------------------------------------------------------------

    /// The clip's masks, in fractions of its `size`-pixel picture, ahead of
    /// a crop's mask.
    fn masks(&mut self, layer: &mut Layer, clip: &Clip, frames: &Frames, size: (u32, u32)) {
        let (w, h) = (f64::from(size.0), f64::from(size.1));
        let at = |x: f64, y: f64| [x * w, y * h];
        let mut groups = Vec::new();
        for mask in &clip.masks {
            let path = match &mask.shape {
                MaskShape::Rect([l, t, r, b]) => {
                    ShapePath::rect(at((l + r) / 2.0, (t + b) / 2.0), (r - l) * w, (b - t) * h)
                }
                MaskShape::Ellipse([x, y, rx, ry]) => {
                    ShapePath::ellipse(at(*x, *y), 2.0 * rx * w, 2.0 * ry * h)
                }
                MaskShape::Polygon { points, closed } => {
                    let points: Vec<[f64; 2]> = points.iter().map(|[x, y]| at(*x, *y)).collect();
                    ShapePath::polygon(&points, *closed)
                }
            };
            let mode = match mask.mode {
                kerf_model::MaskMode::Add => MaskMode::Add,
                kerf_model::MaskMode::Subtract => MaskMode::Subtract,
                kerf_model::MaskMode::Intersect => MaskMode::Intersect,
                kerf_model::MaskMode::Lighten => MaskMode::Lighten,
                kerf_model::MaskMode::Darken => MaskMode::Darken,
                kerf_model::MaskMode::Difference => MaskMode::Difference,
                kerf_model::MaskMode::None => MaskMode::None,
            };
            let mut group = build::mask(
                &mut Ids(&mut self.out.next_id),
                mask.name.as_str(),
                path,
                mode,
                [255, 200, 0],
            );
            if let GroupKind::Mask { inverted, .. } = &mut group.kind {
                *inverted = mask.inverted;
            }
            // Set on the group itself: every mask shares a match id, so a
            // path through the layer could find another.
            for name in ["feather", "opacity", "expansion"] {
                let param = mask.params.get(name).cloned().unwrap_or_else(|| {
                    Param::Static(
                        self.catalog
                            .mask_prop(name)
                            .map_or(Value::Number(0.0), |def| def.default.clone()),
                    )
                });
                let curve = frames.curve(matches!(param, Param::Animated(_)), |time| {
                    [number(&param.value_at(time))]
                });
                match name {
                    "feather" => set_group(
                        &mut group,
                        name,
                        &curve,
                        |v| FxValue::Vec2([v[0]; 2]),
                        false,
                    ),
                    "opacity" => set_group(
                        &mut group,
                        name,
                        &curve,
                        |v| FxValue::Scalar(v[0] * 100.0),
                        false,
                    ),
                    _ => set_group(&mut group, name, &curve, |v| FxValue::Scalar(v[0]), false),
                }
            }
            groups.push(group);
        }
        if let Some(masks) = layer.props.sub_mut("masks") {
            for (index, group) in groups.into_iter().enumerate() {
                masks.children.insert(index, group.into());
            }
        }
    }

    fn effects(
        &mut self,
        layer: &mut Layer,
        clip: &Clip,
        frames: &Frames,
        size: (u32, u32),
        geometry: &Geometry,
        seeds: &mut Seeds,
    ) -> Result<()> {
        let layer_size = [f64::from(size.0), f64::from(size.1)];
        for effect in clip.effects.iter().filter(|effect| effect.enabled) {
            let def = self.catalog.fx(effect.fx.as_str()).ok_or_else(|| {
                ComposeError::Invalid(format!("clip `{}` uses unknown effect", clip.id))
            })?;
            if def.domain != Domain::Video {
                continue;
            }
            let group = match (def.engine, effect.fx.as_str()) {
                (Engine::Raster, id) => {
                    let spec = kerf_fx::spec(id).ok_or_else(|| {
                        ComposeError::Invalid(format!("`{id}` is not a raster effect"))
                    })?;
                    self.raster_group(
                        spec,
                        &effect.params,
                        &clip.masks,
                        frames,
                        layer_size,
                        seeds.next(),
                    )
                }
                (Engine::Graph, "color.basic") => {
                    let spec = color::color_basic();
                    let mut group = self.effect_group(spec, layer_size, 0);
                    for param in &def.params {
                        let set_param = effect
                            .params
                            .get(&param.name)
                            .cloned()
                            .unwrap_or_else(|| Param::Static(param.default.clone()));
                        let curve = frames.curve(matches!(set_param, Param::Animated(_)), |time| {
                            [number(&set_param.value_at(time))]
                        });
                        set_group(
                            &mut group,
                            &param.name,
                            &curve,
                            |v| FxValue::Scalar(v[0]),
                            false,
                        );
                    }
                    group
                }
                (Engine::Graph, "blur.gaussian") => {
                    let spec = effectcraft_effects::find("ec.blur.gaussian")
                        .expect("EffectCraft has Gaussian Blur");
                    let mut group = self.effect_group(spec, layer_size, 0);
                    let radius = effect.params.get("radius").cloned().unwrap_or_else(|| {
                        Param::Static(
                            def.find("radius")
                                .map_or(Value::Number(4.0), |param| param.default.clone()),
                        )
                    });
                    // The graph blurs the picture on the canvas; here the
                    // layer is blurred before it is scaled.
                    let animated =
                        matches!(radius, Param::Animated(_)) || geometry.scale.is_animated();
                    let curve = frames.curve(animated, |time| {
                        let scale = geometry.scale.at(frames.tick(time)).max(1e-6);
                        [number(&radius.value_at(time)) / scale]
                    });
                    set_group(
                        &mut group,
                        "blurriness",
                        &curve,
                        |v| FxValue::Scalar(v[0]),
                        false,
                    );
                    if let Some(repeat) = group.prop_mut("repeatEdge") {
                        repeat.value = FxValue::Bool(true);
                    }
                    group
                }
                (Engine::Graph, other) => {
                    return Err(unsupported(
                        &clip.id,
                        format!("effect `{other}` on a picture"),
                    ))
                }
            };
            push_effect(layer, group);
        }
        Ok(())
    }

    /// An EffectCraft effect's group for a clip with `seed`: EffectCraft
    /// seeds an effect's randomness with the low bits of its group's uid.
    fn effect_group(
        &mut self,
        spec: &'static EffectSpec,
        layer_size: [f64; 2],
        seed: u32,
    ) -> effectcraft_project::PropGroup {
        let mut ids = Ids(&mut self.out.next_id);
        let mut group = effectcraft_effects::instantiate(spec, &mut ids, spec.name, layer_size);
        self.effects += 1;
        group.uid = (self.effects << 32) | u64::from(seed);
        group
    }

    fn raster_group(
        &mut self,
        spec: &'static EffectSpec,
        params: &kerf_model::Params,
        masks: &[Mask],
        frames: &Frames,
        layer_size: [f64; 2],
        seed: u32,
    ) -> effectcraft_project::PropGroup {
        let mut group = self.effect_group(spec, layer_size, seed);
        for param in &spec.params {
            if param.ui == ParamUi::Mask {
                // The named mask's place among the layer's masks, from 1;
                // 0 for none.
                let place = match params.get(param.id) {
                    Some(Param::Static(Value::Text(name))) => {
                        masks.iter().position(|mask| mask.name == name.as_str())
                    }
                    _ => None,
                };
                if let Some(prop) = group.prop_mut(param.id) {
                    prop.value = FxValue::Scalar(place.map_or(0.0, |index| (index + 1) as f64));
                }
                continue;
            }
            if param.ui == ParamUi::Layer {
                // The layer of the track the parameter names, if any.
                let read = match params.get(param.id) {
                    Some(Param::Static(Value::Text(track))) => self.track_layers.get(track),
                    _ => None,
                };
                if let Some(prop) = group.prop_mut(param.id) {
                    prop.value = FxValue::Layer(read.map(|layer| layer.0));
                }
                continue;
            }
            let hold = matches!(param.ui, ParamUi::Checkbox | ParamUi::Popup { .. });
            let animated = kerf_fx::is_animated(param, params);
            let values: Vec<(Tick, FxValue)> = if animated {
                frames
                    .times()
                    .map(|time| {
                        (
                            frames.tick(time),
                            kerf_fx::value_at(param, params, time, layer_size),
                        )
                    })
                    .collect()
            } else {
                vec![(
                    Tick(0),
                    kerf_fx::value_at(param, params, frames.first(), layer_size),
                )]
            };
            if let Some(prop) = group.prop_mut(param.id) {
                set_values(prop, values, hold);
            }
        }
        group
    }

    /// A wipe revealing the clip over the one before it: Linear Wipe run
    /// backward across the transition, its edge and feather placed where
    /// the graph's wipe puts them.
    #[allow(clippy::too_many_arguments)]
    fn wipe(
        &mut self,
        layer: &mut Layer,
        clip: &Clip,
        size: (u32, u32),
        angle: f64,
        softness: f64,
        start: Time,
        end: Time,
    ) {
        let spec = effectcraft_effects::find("ec.transition.linearwipe")
            .expect("EffectCraft has Linear Wipe");
        let (width, height) = (f64::from(size.0), f64::from(size.1));
        let mut group = self.effect_group(spec, [width, height], 0);
        // The graph's wipe front is a line across the picture's width and
        // height; in pixels its normal is the direction over the size.
        let (dy, dx) = angle.to_radians().sin_cos();
        let normal = [dx / width, dy / height];
        let length = normal[0].hypot(normal[1]).max(1e-12);
        let normal = [normal[0] / length, normal[1] / length];
        let span = normal[0].abs() * width + normal[1].abs() * height;
        let soft = softness.max(0.001);
        // Linear Wipe takes away the side its angle points from, so it runs
        // opposite the reveal.
        let wipe_angle = (-normal[0]).atan2(normal[1]).to_degrees();
        let done = |progress: f64| {
            (1.0 + 1.5 * soft - progress * (1.0 + soft)) / (1.0 + 2.0 * soft) * 100.0
        };
        if let Some(prop) = group.prop_mut("angle") {
            prop.value = FxValue::Scalar(wipe_angle);
        }
        if let Some(prop) = group.prop_mut("feather") {
            prop.value = FxValue::Scalar(soft * span);
        }
        if let Some(prop) = group.prop_mut("completion") {
            set_values(
                prop,
                vec![
                    (ticks(start - clip.start), FxValue::Scalar(done(0.0))),
                    (ticks(end - clip.start), FxValue::Scalar(done(1.0))),
                ],
                false,
            );
        }
        push_effect(layer, group);
    }

    /// A raster transition the clip leaves by: its effect last in the
    /// clip's stack, completing across the transition.
    #[allow(clippy::too_many_arguments)]
    fn leave(
        &mut self,
        layer: &mut Layer,
        clip: &Clip,
        frames: &Frames,
        size: (u32, u32),
        transition: &Transition,
        start: Time,
        end: Time,
        seeds: &mut Seeds,
    ) -> Result<()> {
        let spec = kerf_fx::spec(transition.fx.as_str()).ok_or_else(|| {
            ComposeError::Invalid(format!("`{}` is not a raster transition", transition.fx))
        })?;
        let layer_size = [f64::from(size.0), f64::from(size.1)];
        let mut group = self.raster_group(
            spec,
            &transition.params,
            &clip.masks,
            frames,
            layer_size,
            seeds.next(),
        );
        if let Some(prop) = group.prop_mut(kerf_fx::COMPLETION) {
            set_values(
                prop,
                vec![
                    (ticks(start - clip.start), FxValue::Scalar(0.0)),
                    (ticks(end - clip.start), FxValue::Scalar(100.0)),
                ],
                false,
            );
        }
        push_effect(layer, group);
        Ok(())
    }

    /// Time remapping onto a nested comp: the nested sequence's position at
    /// each clip time, in seconds.
    fn remap(&mut self, layer: &mut Layer, clip: &Clip, remap: Remap, lo: Time, hi: Time) {
        let keys = match remap {
            Remap::Still => vec![(Tick(0), FxValue::Scalar(0.0))],
            Remap::Shift => vec![
                (ticks(lo - clip.start), FxValue::Scalar(0.0)),
                (ticks(hi - clip.start), FxValue::Scalar((hi - lo).seconds())),
            ],
            Remap::Nested {
                source_in,
                speed,
                duration,
            } => {
                let position = |at: Time| -> f64 {
                    let at = at - clip.start;
                    let exact = |time: Time| time.flicks() as f64 * TICKS_PER_FLICK as f64;
                    let span =
                        exact(at) * f64::from(speed.num().unsigned_abs()) / f64::from(speed.den());
                    let position = if speed.is_reverse() {
                        // The frame showing just before the source position.
                        exact(source_in + speed.source_span(duration)) - span - NESTED_LEAD as f64
                    } else {
                        exact(source_in) + span + NESTED_LEAD as f64
                    };
                    position / TICKS_PER_SECOND as f64
                };
                vec![
                    (ticks(lo - clip.start), FxValue::Scalar(position(lo))),
                    (ticks(hi - clip.start), FxValue::Scalar(position(hi))),
                ]
            }
        };
        let mut prop = Ids(&mut self.out.next_id)
            .prop("timeRemap", "Time Remap", FxValue::Scalar(0.0))
            .with_ui(ParamUi::Number);
        set_values(&mut prop, keys, false);
        layer.props.children.push(prop.into());
    }

    // ---- parameters ------------------------------------------------------------

    fn static_number(
        &self,
        entity: &Id,
        params: &kerf_model::Params,
        fx: &str,
        name: &str,
    ) -> Result<f64> {
        match params.get(name) {
            None => Ok(self
                .catalog
                .fx(fx)
                .and_then(|def| def.find(name))
                .and_then(|def| def.default.as_number())
                .unwrap_or(0.0)),
            Some(Param::Static(Value::Number(n))) => Ok(*n),
            Some(_) => Err(unsupported(entity, format!("animated `{name}`"))),
        }
    }

    fn color(
        &self,
        entity: &Id,
        params: &kerf_model::Params,
        fx: &str,
        name: &str,
    ) -> Result<Rgba> {
        match params.get(name) {
            None => Ok(
                match self
                    .catalog
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

fn rgba(color: Rgba) -> [f32; 4] {
    [color.r, color.g, color.b, color.a].map(|channel| f32::from(channel) / 255.0)
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

fn push_effect(layer: &mut Layer, group: effectcraft_project::PropGroup) {
    if let Some(effects) = layer.props.sub_mut("effects") {
        effects.children.push(group.into());
    }
}

/// The seeds a clip's raster effects take, as the bake gives them.
struct Seeds {
    base: u32,
    next: u32,
}

impl Seeds {
    fn new(clip: &Clip) -> Self {
        Self {
            base: kerf_core::raster::seed(clip.id.as_str()),
            next: 0,
        }
    }

    fn next(&mut self) -> u32 {
        let seed = self.base.wrapping_add(self.next);
        self.next += 1;
        seed
    }
}

/// The frames a layer shows, as clip times.
struct Frames {
    start: Time,
    rate: Rate,
    first: i64,
    last: i64,
}

impl Frames {
    fn new(comp: &Comp, clip: &Clip, lo: Time, hi: Time) -> Self {
        let rate = Rate::new(comp.frame_rate.num as u32, comp.frame_rate.den as u32)
            .unwrap_or(Rate::FPS_24);
        let frame = rate.frame_duration().flicks();
        let ceil = |time: Time| -(-time.flicks()).div_euclid(frame);
        Self {
            start: clip.start,
            rate,
            first: ceil(lo),
            last: (ceil(hi) - 1).max(ceil(lo)),
        }
    }

    fn first(&self) -> Time {
        self.rate.time_of(self.first) - self.start
    }

    fn times(&self) -> impl Iterator<Item = Time> + '_ {
        (self.first..=self.last).map(|frame| self.rate.time_of(frame) - self.start)
    }

    /// Layer time for clip time.
    fn tick(&self, time: Time) -> Tick {
        ticks(time)
    }

    /// Values at every frame shown, kept where they change course, or one
    /// value when nothing is `animated`.
    fn curve<const N: usize>(&self, animated: bool, at: impl Fn(Time) -> [f64; N]) -> Curve<N> {
        if !animated || self.first == self.last {
            return Curve::Constant(at(self.first()));
        }
        let mut keys: Vec<(Tick, [f64; N])> = Vec::new();
        for time in self.times() {
            let value = at(time);
            let tick = self.tick(time);
            if let [.., (t0, a), (t1, b)] = keys.as_slice() {
                let u = (t1.0 - t0.0) as f64 / (tick.0 - t0.0) as f64;
                let on_line = (0..N).all(|i| {
                    let expected = a[i] + (value[i] - a[i]) * u;
                    (expected - b[i]).abs() <= 1e-9 * (1.0 + b[i].abs())
                });
                if on_line {
                    keys.pop();
                }
            }
            keys.push((tick, value));
        }
        if keys.iter().all(|(_, value)| *value == keys[0].1) {
            Curve::Constant(keys[0].1)
        } else {
            Curve::Keys(keys)
        }
    }
}

/// A property's values over a layer's frames.
enum Curve<const N: usize> {
    Constant([f64; N]),
    Keys(Vec<(Tick, [f64; N])>),
}

impl<const N: usize> Curve<N> {
    fn values(&self) -> Box<dyn Iterator<Item = &[f64; N]> + '_> {
        match self {
            Self::Constant(value) => Box::new(std::iter::once(value)),
            Self::Keys(keys) => Box::new(keys.iter().map(|(_, value)| value)),
        }
    }

    fn map(&self, f: impl Fn(&[f64; N]) -> f64) -> Curve<1> {
        match self {
            Self::Constant(value) => Curve::Constant([f(value)]),
            Self::Keys(keys) => Curve::Keys(
                keys.iter()
                    .map(|(tick, value)| (*tick, [f(value)]))
                    .collect(),
            ),
        }
    }

    fn is_animated(&self) -> bool {
        matches!(self, Self::Keys(_))
    }
}

impl Curve<1> {
    /// The value at layer time `at`, between keys as EffectCraft
    /// interpolates them.
    fn at(&self, at: Tick) -> f64 {
        match self {
            Self::Constant([value]) => *value,
            Self::Keys(keys) => {
                let next = keys.partition_point(|(tick, _)| *tick <= at);
                match (next.checked_sub(1).map(|i| keys[i]), keys.get(next)) {
                    (Some((_, [value])), None) | (None, Some(&(_, [value]))) => value,
                    (Some((t0, [a])), Some(&(t1, [b]))) => {
                        a + (b - a) * (at.0 - t0.0) as f64 / (t1.0 - t0.0) as f64
                    }
                    (None, None) => 0.0,
                }
            }
        }
    }
}

/// How many canvas pixels a layer pixel covers.
struct Geometry {
    /// Across, at each clip time.
    scale: Curve<1>,
    /// Along either side, at the most.
    largest: f64,
}

fn set<const N: usize>(
    layer: &mut Layer,
    path: &str,
    curve: &Curve<N>,
    value: impl Fn(&[f64; N]) -> FxValue,
) {
    if let Some(prop) = layer.props.prop_mut(path) {
        set_curve(prop, curve, value, false);
    }
}

fn set_group<const N: usize>(
    group: &mut effectcraft_project::PropGroup,
    path: &str,
    curve: &Curve<N>,
    value: impl Fn(&[f64; N]) -> FxValue,
    hold: bool,
) {
    if let Some(prop) = group.prop_mut(path) {
        set_curve(prop, curve, value, hold);
    }
}

fn set_curve<const N: usize>(
    prop: &mut Property,
    curve: &Curve<N>,
    value: impl Fn(&[f64; N]) -> FxValue,
    hold: bool,
) {
    let values = match curve {
        Curve::Constant(constant) => vec![(Tick(0), value(constant))],
        Curve::Keys(keys) => keys.iter().map(|(tick, v)| (*tick, value(v))).collect(),
    };
    set_values(prop, values, hold);
}

/// One value, or keys between which EffectCraft interpolates linearly
/// (or holds).
fn set_values(prop: &mut Property, values: Vec<(Tick, FxValue)>, hold: bool) {
    let mut values = values;
    if values.len() == 1 {
        prop.value = values.remove(0).1;
        prop.keys.clear();
        return;
    }
    prop.value = values[0].1.clone();
    prop.keys = values
        .into_iter()
        .map(|(tick, value)| {
            let mut key = FxKeyframe::new(tick, value);
            if hold {
                key.in_interp = FxInterp::Hold;
                key.out_interp = FxInterp::Hold;
            }
            key
        })
        .collect();
}
