//! A sequence drawn from Kerf's model, in linear light, with the drawing
//! carried from FilmCraft's renderer (`kerf-draw`):
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
//!   reveals it along its front, and a dip fades a solid through. The
//!   transitions carried from FilmCraft (`kerf_fx::transitions`) mix the
//!   two clips' pictures, each drawn as it shows there, across the cut.
//! - Titles are text centered on a transparent canvas, solids a color, and
//!   an adjustment layer applies its effects to the picture beneath it.
//! - A clip's effects run on its picture in order, after its masks, each
//!   found by its id and run the same way (`crate::domains`), and read the
//!   clip's other frames, the sequence's other tracks, and its sound tracks
//!   as they need.

use crate::footage::{frame_count, ClipSource, Pictures};
use crate::{domains, frames};
use crate::{Accel, ComposeError, ComposeOptions, Frame, Pass};
use kerf_color::{HdrOutput, WorkingSpace};
use kerf_core::program::{canvas, program_sequence};
use kerf_draw::blend::{composite, composite_placed};
use kerf_draw::correct::Correction;
use kerf_draw::effect::{EffectHost, EffectRun, Files};
use kerf_draw::source::{premultiplied, PictureSource, SourceRun};
use kerf_draw::text::{self, Justify, Text};
use kerf_draw::{blur, mask, wipe, Affine, Blend, Image, Rect, Vec2};
use kerf_fx::filmcraft::{self, catalog::Headroom};
use kerf_fx::transitions::{apply_scaled, Fx};
use kerf_model::time::format_timecode;
use kerf_model::{
    Catalog, Clip, Content, Domain, Effect, FxDef, Id, Media, MotionPath, Param, ParamType, Params,
    Project, Rate, Rgba, Sequence, Speed, Time, Track, TrackKind, Value,
};
use rayon::prelude::*;
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

mod focus;
mod gpu;
mod bevel;
mod three_d;

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
/// What an effect's picture was brought between 0 and 1 by.
enum Held {
    Nothing,
    Unit(Vec<[f32; 2]>),
    Gamut(Vec<f32>),
    /// What the whole picture had taken from it, and what was left divided by.
    Scale(f32, f32),
}

impl Held {
    fn put_back(self, picture: &mut Image) {
        match self {
            Held::Nothing => {}
            Held::Unit(held) if held.len() * 4 == picture.px.len() => picture.from_unit(&held),
            Held::Gamut(held) if held.len() * 4 == picture.px.len() => picture.from_gamut(&held),
            Held::Unit(_) | Held::Gamut(_) => {}
            Held::Scale(lift, by) => picture.unlift_rgb(lift, by),
        }
    }
}

pub(crate) struct Drawing {
    /// How the program leaves, when it is an HDR sequence.
    hdr: Option<HdrOutput>,
    /// The program's sequence first, then the sequences nested in it.
    sequences: Vec<SequencePlan>,
    pictures: Pictures,
    /// The sound of media files, decoded for the effects that draw it.
    sounds: crate::sound::Sounds,
    captions: bool,
    accel: Accel,
    keep_frames: bool,
    disk: Option<std::sync::Arc<crate::disk::DiskFrames>>,
    /// The files the program's effects name, read once.
    files: Files,
    /// What is drawn of the program.
    pass: Pass,
    /// The color each clip that draws has in an ids pass, in the order they are told in.
    ids: Vec<(Id, Rgba)>,
}

/// The color the clip told `index`th has in an ids pass: no two near each other for as many clips
/// as a scene has, and none near black, which is where there is no clip.
fn id_color(index: usize) -> Rgba {
    // Hues a golden angle apart never bunch, and three kinds of color in turn part neighbors.
    let hue = (index as f64 * 137.507_764) % 360.0;
    let (saturation, value) = [(0.85, 1.0), (0.55, 0.8), (1.0, 0.62)][index % 3];
    let chroma = value * saturation;
    let within = chroma * (1.0 - ((hue / 60.0) % 2.0 - 1.0).abs());
    let (r, g, b) = match (hue / 60.0) as u32 {
        0 => (chroma, within, 0.0),
        1 => (within, chroma, 0.0),
        2 => (0.0, chroma, within),
        3 => (0.0, within, chroma),
        4 => (within, 0.0, chroma),
        _ => (chroma, 0.0, within),
    };
    let byte = |channel: f64| ((channel + value - chroma) * 255.0).round() as u8;
    Rgba {
        r: byte(r),
        g: byte(g),
        b: byte(b),
        a: 255,
    }
}

/// `picture` in the one color `color`, where and as much as it was there.
fn flat(picture: &mut Image, color: [f32; 3]) {
    picture.px.par_chunks_mut(4).for_each(|pixel| {
        let alpha = pixel[3];
        pixel[0] = color[0] * alpha;
        pixel[1] = color[1] * alpha;
        pixel[2] = color[2] * alpha;
    });
}

struct SequencePlan {
    name: String,
    width: u32,
    height: u32,
    rate: Rate,
    /// Timecode of the first frame.
    start: Time,
    drop_frame: bool,
    duration: Time,
    tracks: Vec<TrackPlan>,
    /// Its picture tracks' ids, from the lowest, for effects that read one.
    track_ids: Vec<String>,
    /// Its sound tracks, for effects that draw their sound, and their ids
    /// in order.
    sounds: Vec<SoundPlan>,
    sound_ids: Vec<String>,
    captions: Vec<CaptionPlan>,
    /// The clips other clips move with.
    rigs: HashMap<Id, Rig>,
}

struct SoundPlan {
    id: String,
    clips: Vec<SoundClip>,
}

/// A media clip's sound on a sound track.
struct SoundClip {
    path: String,
    start: Time,
    duration: Time,
    source_in: Time,
    speed: Speed,
}

struct CaptionPlan {
    text: String,
    from: Time,
    to: Time,
}

struct TrackPlan {
    id: Id,
    /// Whether it shows; a hidden track is drawn only for effects that read
    /// it.
    shows: bool,
    shown: Vec<Shown>,
    dips: Vec<Dip>,
    pairs: Vec<Pair>,
}

/// A transition that mixes two shown clips' pictures across their cut.
struct Pair {
    /// The clips it leaves and enters, by their place in the track's shown;
    /// the same clip when only one of them shows.
    from: usize,
    to: usize,
    /// Only the clip it enters shows: it enters from nothing.
    from_nothing: bool,
    def: FxDef,
    params: Params,
    start: Time,
    end: Time,
}

/// A clip as its track shows it: its span with transition handles, and
/// how it enters.
struct Shown {
    plan: ClipPlan,
    from: Time,
    to: Time,
    reveal: Option<Entrance>,
}

/// How a clip enters over the one before it, and from when to when.
type Entrance = (Reveal, Time, Time);

#[derive(Clone, Copy, Debug)]
enum Reveal {
    Dissolve,
    Wipe {
        angle: f64,
        softness: f64,
    },
    /// Over the outgoing clip as it is: a look's transition draws it.
    Over,
}

/// A dip's color between the two clips of its cut.
struct Dip {
    color: Rgba,
    from: Time,
    to: Time,
}

/// A clip other clips move with: what it takes to work out how it has moved them.
struct Rig {
    clip: Clip,
    /// The source's size in its own pixels.
    size: (u32, u32),
    fit: bool,
    path: Option<MotionPath>,
    digest: Option<u64>,
}

struct ClipPlan {
    clip: Clip,
    /// Its name, or its media's file or nested sequence's name.
    name: String,
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
    /// Placed in depth, seen by the sequence's camera.
    three_d: bool,
    /// The most of its size it is ever shown at, when under 1: its frames
    /// are decoded at that size.
    shown: f64,
    blend: Blend,
    effects: Vec<Effect>,
    path: Option<MotionPath>,
    /// A number for everything about the clip that does not change over time, wherever it starts.
    /// `None` when it draws on something the document does not hold: another track, a sound, a file.
    digest: Option<u64>,
    /// Its effects read the sequence's time or name, as a burn-in does.
    reads_sequence_time: bool,
}

enum Source {
    Media {
        picture: usize,
    },
    Nested {
        sequence: usize,
        source_in: Time,
        speed: Speed,
        duration: Time,
        /// The one track of it to show.
        angle: Option<Id>,
    },
    Solid(Rgba),
    Title(TitlePlan),
    /// A generator a crate draws (`crate::domains`).
    Drawn(&'static dyn PictureSource),
    Adjustment,
    /// A null: it draws nothing, and other clips move with it.
    Nothing,
    /// A mesh: it has no picture of its own, and is drawn in depth from its triangles
    /// (`three_d::mesh`).
    Mesh,
}

struct TitlePlan {
    text: String,
    font: String,
    /// In frame heights.
    size: f64,
    color: Rgba,
    justify: Justify,
    /// Where a variable font's axes are set, by tag, as they are given.
    axes: Vec<([u8; 4], Param)>,
    /// The font's OpenType features it turns on or off, as `features` names them.
    features: Vec<([u8; 4], u32)>,
    /// How it comes in, when it does.
    reveal: Option<Box<Reveals>>,
}

/// A title's reveal, as its parameters say.
struct Reveals {
    reveal: Param,
    by_word: bool,
    soft: Param,
    /// Across and down, in frame heights.
    from: [Param; 2],
    scale: Param,
    /// In degrees.
    rotation: Param,
    opacity: Param,
}

impl Drawing {
    pub fn new(project: &Project, catalog: &Catalog, options: &ComposeOptions) -> Result<Self> {
        let (sequence, pass) = (options.sequence.as_deref(), options.pass);
        // Looks draw as the built-ins they stand for.
        let expanded = kerf_model::look::expand(project, catalog).map_err(ComposeError::Invalid)?;
        let project = &expanded;
        let top = program_sequence(project, sequence)
            .map_err(|error| ComposeError::Invalid(error.to_string()))?;
        let mut builder = Builder {
            project,
            catalog,
            sequences: Vec::new(),
            built: HashMap::new(),
            clips: Vec::new(),
            nesting: vec![top.id.clone()],
            effects: Vec::new(),
            hdr: HdrOutput::new(top.color).is_some(),
        };
        // The program's sequence is planned first, so it is the first.
        builder.sequences.push(None);
        let plan = builder.sequence(top)?;
        builder.sequences[0] = Some(plan);
        let mut files = Files::new();
        for effect in &builder.effects {
            if let Some(code) = domains::effect(effect.fx.as_str()) {
                code.prepare(effect, &mut files)
                    .map_err(ComposeError::Invalid)?;
            }
        }
        let sequences: Vec<SequencePlan> = builder
            .sequences
            .into_iter()
            .map(|plan| plan.expect("every sequence planned"))
            .collect();
        let mut ids: Vec<(Id, Rgba)> = Vec::new();
        for shown in sequences
            .iter()
            .flat_map(|sequence| &sequence.tracks)
            .flat_map(|track| &track.shown)
        {
            let id = &shown.plan.clip.id;
            let draws = !matches!(shown.plan.source, Source::Nothing | Source::Adjustment);
            if draws && !ids.iter().any(|(told, _)| told == id) {
                ids.push((id.clone(), id_color(ids.len())));
            }
        }
        Ok(Self {
            hdr: HdrOutput::new(top.color),
            sequences,
            pictures: Pictures::new(builder.clips),
            sounds: crate::sound::Sounds::default(),
            captions: options.captions,
            // A pass has no kernels: the CPU draws it.
            accel: if pass == Pass::Picture {
                options.accel
            } else {
                Accel::Never
            },
            keep_frames: options.keep_frames,
            disk: options.frame_folder.as_deref().map(crate::disk::open),
            files,
            pass,
            ids,
        })
    }

    /// See [`Compositor::ids`](crate::Compositor::ids).
    pub fn ids(&self) -> Vec<(String, [u8; 3])> {
        self.ids
            .iter()
            .map(|(id, color)| (id.to_string(), [color.r, color.g, color.b]))
            .collect()
    }

    /// A clip's blend, which a pass does not have: it is laid over what is behind it.
    fn blend(&self, blend: Blend) -> Blend {
        if self.pass == Pass::Picture {
            blend
        } else {
            Blend::Normal
        }
    }

    /// `picture`, a clip's own with all done to it, as the pass drawn has it: where it was, in the
    /// clip's color, or by how near each of its pixels is.
    fn as_pass(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        picture: &mut Image,
        to_output: &Affine,
        at: Time,
        size: (usize, usize),
    ) {
        match self.pass {
            Pass::Picture => {}
            Pass::Ids => {
                let color = self
                    .ids
                    .iter()
                    .find(|(id, _)| *id == plan.clip.id)
                    .map_or(Rgba::WHITE, |(_, color)| *color);
                let [r, g, b, _] = premultiplied(color);
                flat(picture, [r, g, b]);
            }
            Pass::Depth if plan.three_d => {
                let scene = three_d::Scene::at(sequence, at, size);
                three_d::shade_by_depth(&scene, plan, picture, to_output, at - plan.clip.start);
            }
            Pass::Depth => flat(picture, [0.0; 3]),
        }
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
        let key = (self.keep_frames || self.disk.is_some())
            .then(|| self.frame_key(at, scale))
            .flatten();
        let kept = key.filter(|_| self.keep_frames);
        if let Some(frame) = kept.and_then(frames::get) {
            return frame;
        }
        let on_disk = self.disk_key(key, at, scale);
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk) {
            if let Some(frame) = disk.get(key, false) {
                if let Some(kept) = kept {
                    frames::put(kept, frame.clone());
                }
                return frame;
            }
        }
        let frame = self.draw_frame(at, scale);
        if let Some(kept) = kept {
            frames::put(kept, frame.clone());
        }
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk) {
            disk.put(key, false, frame.clone());
        }
        frame
    }

    /// See [`Compositor::flush_frames`](crate::Compositor::flush_frames).
    pub fn flush_frames(&self) {
        if let Some(disk) = &self.disk {
            disk.flush();
        }
    }

    /// A frame's key on disk: longer than in memory, since it stays longer and there are more.
    fn disk_key(&self, key: Option<u64>, at: Time, scale: f64) -> Option<u128> {
        self.disk.as_ref()?;
        let first = key?;
        let mut hash = DefaultHasher::new();
        first.hash(&mut hash);
        scale.clamp(1.0 / 16.0, 1.0).to_bits().hash(&mut hash);
        self.hdr.as_ref().map(|output| output.pq).hash(&mut hash);
        self.hash_sequence(&mut hash, 0, at)?;
        Some(u128::from(first) << 64 | u128::from(hash.finish()))
    }

    /// See [`Compositor::frame16`](crate::Compositor::frame16).
    pub fn frame16(&self, at: Time, scale: f64) -> Frame {
        let key = self
            .disk
            .is_some()
            .then(|| self.frame_key(at, scale))
            .flatten();
        let on_disk = self.disk_key(key, at, scale);
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk) {
            if let Some(frame) = disk.get(key, true) {
                return frame;
            }
        }
        let frame = self.draw_frame16(at, scale);
        if let Some((disk, key)) = self.disk.as_ref().zip(on_disk) {
            disk.put(key, true, frame.clone());
        }
        frame
    }

    /// The frame at `at` as the GPU draws it, or `None` where [`Drawing::frame`] would give it to the
    /// CPU, even partway through.
    pub(crate) fn frame_on_gpu(&self, at: Time, scale: f64) -> Option<Frame> {
        self.gpu_frame(at, scale.clamp(1.0 / 16.0, 1.0))
    }

    fn draw_frame16(&self, at: Time, scale: f64) -> Frame {
        let scale = scale.clamp(1.0 / 16.0, 1.0);
        if let Some(frame) = self.gpu_frame16(at, scale) {
            return frame;
        }
        let image = self.draw(0, at, scale);
        let rgba = match &self.hdr {
            Some(output) if self.pass == Pass::Picture => {
                image.to_signal16_over_black(|c| output.signal(c))
            }
            _ => image.to_rgba16_over_black(),
        };
        Frame {
            width: image.w as u32,
            height: image.h as u32,
            rgba,
        }
    }

    /// What effects are told the picture is.
    pub(super) fn working(&self) -> WorkingSpace {
        match &self.hdr {
            Some(output) if output.pq => WorkingSpace::Rec2100Pq,
            Some(_) => WorkingSpace::Rec2100Hlg,
            None => WorkingSpace::Rec709,
        }
    }

    fn draw_frame(&self, at: Time, scale: f64) -> Frame {
        if scale >= 1.0 && self.hdr.is_none() && self.pass == Pass::Picture {
            if let Some(frame) = self.plain_frame(at) {
                return frame;
            }
        }
        let scale = scale.clamp(1.0 / 16.0, 1.0);
        if let Some(frame) = self.gpu_frame(at, scale) {
            return frame;
        }
        let mut image = self.draw(0, at, scale);
        // A pass is numbers, which no display is to be fitted to.
        if let Some(output) = self.hdr.as_ref().filter(|_| self.pass == Pass::Picture) {
            image.px.par_chunks_mut(4).for_each(|pixel| {
                let sdr = output.sdr([pixel[0], pixel[1], pixel[2]]);
                pixel[..3].copy_from_slice(&sdr);
            });
        }
        Frame {
            width: image.w as u32,
            height: image.h as u32,
            rgba: image.to_rgba8_over_black(),
        }
    }

    /// The plan of the program's mesh clip `id`, and its sequence's.
    fn mesh_plan(&self, id: &str) -> Option<(&SequencePlan, &ClipPlan)> {
        let sequence = self.sequences.first()?;
        sequence
            .tracks
            .iter()
            .flat_map(|track| &track.shown)
            .map(|shown| &shown.plan)
            .find(|plan| plan.clip.id.as_str() == id && matches!(plan.source, Source::Mesh))
            .map(|plan| (sequence, plan))
    }

    /// See [`Compositor::mesh_file`](crate::Compositor::mesh_file).
    pub fn mesh_file(&self, id: &str) -> std::result::Result<crate::MeshFile, String> {
        let (_, plan) = self.mesh_plan(id).ok_or("it is no mesh of this sequence")?;
        three_d::mesh::holds(&plan.clip)
    }

    /// See [`Compositor::mesh_camera`](crate::Compositor::mesh_camera).
    pub fn mesh_camera(&self, id: &str, camera: usize, at: Time) -> Option<crate::CameraPlace> {
        let (sequence, plan) = self.mesh_plan(id)?;
        let time = at - plan.clip.start;
        if time.is_negative() || time >= plan.clip.duration {
            return None;
        }
        let size = (sequence.width as usize, sequence.height as usize);
        let scene = three_d::Scene::at(sequence, at, size);
        let to_output = self.geometry(sequence, plan, time);
        three_d::mesh::camera(&scene, plan, &to_output, time, camera)
    }

    /// See [`Compositor::mesh_view`](crate::Compositor::mesh_view).
    pub fn mesh_view(
        &self,
        id: &str,
        view: &crate::FileView,
        at: Time,
    ) -> Option<crate::CameraPlace> {
        let (sequence, plan) = self.mesh_plan(id)?;
        let time = at - plan.clip.start;
        if time.is_negative() || time >= plan.clip.duration {
            return None;
        }
        let size = (sequence.width as usize, sequence.height as usize);
        let scene = three_d::Scene::at(sequence, at, size);
        let to_output = self.geometry(sequence, plan, time);
        three_d::mesh::view(&scene, plan, &to_output, time, view)
    }

    /// See [`Compositor::frame_key`](crate::Compositor::frame_key).
    pub fn frame_key(&self, at: Time, scale: f64) -> Option<u64> {
        let mut hash = DefaultHasher::new();
        scale.clamp(1.0 / 16.0, 1.0).to_bits().hash(&mut hash);
        self.hdr.as_ref().map(|output| output.pq).hash(&mut hash);
        self.captions.hash(&mut hash);
        // Only where it is not the picture, so that a picture's number is what it was.
        if self.pass != Pass::Picture {
            self.pass.hash(&mut hash);
        }
        self.hash_sequence(&mut hash, 0, at)?;
        Some(hash.finish())
    }

    /// Hash everything sequence `index` at its time `at` draws from.
    fn hash_sequence(&self, hash: &mut DefaultHasher, index: usize, at: Time) -> Option<()> {
        self.hash_sequence_of(hash, index, at, None)
    }

    /// [`Drawing::hash_sequence`], of the one track `angle` when it names one.
    fn hash_sequence_of(
        &self,
        hash: &mut DefaultHasher,
        index: usize,
        at: Time,
        angle: Option<&Id>,
    ) -> Option<()> {
        let plan = &self.sequences[index];
        (
            plan.width,
            plan.height,
            plan.rate.fps().to_bits(),
            plan.drop_frame,
        )
            .hash(hash);
        for track in plan.tracks.iter().filter(|track| track.drawn(angle)) {
            for (place, shown) in track.shown.iter().enumerate() {
                if !shown.shows(at) {
                    continue;
                }
                let clip = &shown.plan;
                let time = at - clip.clip.start;
                clip.digest?.hash(hash);
                time.flicks().hash(hash);
                clip.shown.to_bits().hash(hash);
                clip.letterboxed.hash(hash);
                let mut parent = clip.clip.parent.as_ref();
                while let Some(rig) = parent.and_then(|id| plan.rigs.get(id)) {
                    rig.digest?.hash(hash);
                    (at - rig.clip.start).flicks().hash(hash);
                    parent = rig.clip.parent.as_ref();
                }
                if clip.reads_sequence_time {
                    at.flicks().hash(hash);
                }
                if let Some((reveal, start, end)) = shown.reveal {
                    format!("{reveal:?}").hash(hash);
                    progress(at, start, end).to_bits().hash(hash);
                }
                if let Source::Nested {
                    sequence: nested,
                    source_in,
                    speed,
                    duration,
                    angle,
                } = &clip.source
                {
                    let inner = &self.sequences[*nested];
                    let shown = nested_time(time, *source_in, speed, *duration, inner.rate);
                    self.hash_sequence_of(hash, *nested, shown, angle.as_ref())?;
                }
                for pair in track.pairs.iter().filter(|pair| {
                    pair.start <= at && at < pair.end && (pair.from == place || pair.to == place)
                }) {
                    format!("{}{:?}", pair.def.id, pair.params).hash(hash);
                    (pair.from == place, pair.from_nothing).hash(hash);
                    progress(at, pair.start, pair.end).to_bits().hash(hash);
                }
            }
            for dip in track.dips.iter().filter(|dip| dip.shows(at)) {
                (dip.color.r, dip.color.g, dip.color.b, dip.color.a).hash(hash);
                progress(at, dip.from, dip.to).to_bits().hash(hash);
            }
        }
        if self.captions {
            for caption in plan.captions.iter().filter(|caption| caption.shows(at)) {
                caption.text.hash(hash);
            }
        }
        Some(())
    }

    /// A frame that is one media clip alone, untouched, at full size: its
    /// picture as decoded, which drawing it would give again.
    fn plain_frame(&self, at: Time) -> Option<Frame> {
        let plan = &self.sequences[0];
        if self.captions && plan.captions.iter().any(|caption| caption.shows(at)) {
            return None;
        }
        let mut showing = plan
            .tracks
            .iter()
            .filter(|track| track.shows)
            .flat_map(|track| {
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
        self.draw_of(index, at, scale, None)
    }

    /// [`Drawing::draw`], of the one track `angle` when it names one.
    fn draw_of(&self, index: usize, at: Time, scale: f64, angle: Option<&Id>) -> Image {
        let plan = &self.sequences[index];
        let (w, h) = output_size(plan.width, plan.height, scale);
        let mut canvas = Image::new(w, h);
        let tracks = self.in_order(plan, at, scale, angle);
        let in_depth = tracks.iter().any(|track| {
            track
                .shown
                .iter()
                .any(|shown| shown.shows(at) && shown.plan.three_d)
        });
        let scene = in_depth.then(|| self.scene(plan, &tracks, at, scale, (w, h)));
        let moment = Moment {
            at,
            scale,
            scene: scene.as_ref(),
        };
        let mut next = 0;
        while next < tracks.len() {
            let crossing = self.crossing_run(plan, &tracks[next..], &moment);
            if crossing.is_empty() {
                self.track(&mut canvas, plan, tracks[next], &moment);
                next += 1;
            } else {
                next += crossing.len();
                self.crossing(&mut canvas, plan, &crossing, &moment);
            }
        }
        if self.captions && self.pass == Pass::Picture {
            for caption in plan.captions.iter().filter(|caption| caption.shows(at)) {
                let text_size = caption_size(plan, scale);
                let text = caption_text(caption, text_size);
                text::draw(&mut canvas, &text, [w as f32 / 2.0, h as f32 * 0.95], 1.0);
            }
        }
        canvas
    }

    /// The tracks to draw, with adjacent ones showing clips in depth put far to near.
    fn in_order<'p>(
        &self,
        plan: &'p SequencePlan,
        at: Time,
        scale: f64,
        angle: Option<&Id>,
    ) -> Vec<&'p TrackPlan> {
        let mut tracks: Vec<&TrackPlan> = plan
            .tracks
            .iter()
            .filter(|track| track.drawn(angle))
            .collect();
        fn in_depth(track: &TrackPlan, at: Time) -> Option<&ClipPlan> {
            track
                .shown
                .iter()
                .find(|shown| shown.shows(at) && shown.plan.three_d)
                .map(|shown| &shown.plan)
        }
        if !tracks.iter().any(|track| in_depth(track, at).is_some()) {
            return tracks;
        }
        let size = output_size(plan.width, plan.height, scale);
        let scene = three_d::Scene::at(plan, at, size);
        let depth = |track: &TrackPlan| {
            in_depth(track, at).map(|clip| {
                let place = Affine::scale(scale, scale).then_apply(&self.geometry(
                    plan,
                    clip,
                    at - clip.clip.start,
                ));
                scene.depth(clip, &place, at - clip.clip.start)
            })
        };
        let mut start = 0;
        while start < tracks.len() {
            if depth(tracks[start]).is_none() {
                start += 1;
                continue;
            }
            let mut end = start + 1;
            while end < tracks.len() && depth(tracks[end]).is_some() {
                end += 1;
            }
            // Stable, so that clips at one depth stack as their tracks do.
            tracks[start..end].sort_by(|a, b| {
                let (a, b) = (depth(a).unwrap_or(0.0), depth(b).unwrap_or(0.0));
                b.total_cmp(&a)
            });
            start = end;
        }
        tracks
    }

    /// How the clips in depth of `tracks` are seen and lit at `at`, with their pictures when a light
    /// casts shadows, and only then, since it takes every one of them at once.
    fn scene(
        &self,
        plan: &SequencePlan,
        tracks: &[&TrackPlan],
        at: Time,
        scale: f64,
        size: (usize, usize),
    ) -> three_d::Scene {
        let mut scene = three_d::Scene::at(plan, at, size);
        if self.pass != Pass::Picture {
            return scene.bare();
        }
        if scene.casts_shadows() {
            let casters = tracks
                .iter()
                .flat_map(|track| &track.shown)
                .filter(|shown| {
                    shown.shows(at)
                        && shown.plan.three_d
                        && !matches!(
                            shown.plan.source,
                            Source::Nothing | Source::Adjustment | Source::Mesh
                        )
                })
                .filter_map(|shown| {
                    let ready = self.ready(plan, shown, at, scale, size)?;
                    let time = at - shown.plan.clip.start;
                    let caster = three_d::Caster::new(
                        &scene,
                        &shown.plan,
                        ready.picture,
                        (&ready.to_output, ready.crop),
                        ready.opacity,
                        time,
                    );
                    let height = size.1 as f64;
                    let depth = three_d::depth_of(&shown.plan, time, height);
                    let bevel = three_d::bevel_of(&shown.plan, time, height);
                    let round = three_d::bevel_is_round(&shown.plan);
                    Some(caster.through_its_depth(depth, (bevel, round), scene.is_wide()))
                })
                .flatten()
                .collect();
            scene.casters = casters;
        }
        for wanted in scene.environments() {
            let picture = plan
                .tracks
                .iter()
                .find(|track| track.id.as_str() == wanted)
                .and_then(|track| track.shown.iter().find(|shown| shown.shows(at)))
                .and_then(|shown| self.ready(plan, shown, at, scale, size));
            if let Some(ready) = picture {
                scene.light_from(&wanted, &ready.picture);
            }
        }
        scene
    }

    /// The clips of the run of tracks in depth at the start of `tracks`, far to near as they stand, if
    /// any two of them cross; else none, and they draw one by one as ever.
    fn crossing_run<'p>(
        &self,
        plan: &'p SequencePlan,
        tracks: &[&'p TrackPlan],
        moment: &Moment,
    ) -> Vec<&'p Shown> {
        let Moment { at, scale, scene } = *moment;
        let Some(scene) = scene else {
            return Vec::new();
        };
        let run: Vec<&Shown> = tracks
            .iter()
            .map_while(|track| track.lone_in_depth(at))
            .collect();
        if run.len() < 2 {
            return Vec::new();
        }
        let planes: Vec<(three_d::Plane, Rect)> = run
            .iter()
            .map(|shown| {
                let clip = &shown.plan;
                let time = at - clip.clip.start;
                let place =
                    Affine::scale(scale, scale).then_apply(&self.geometry(plan, clip, time));
                let (w, h) = (f64::from(clip.size.0), f64::from(clip.size.1));
                let crop = crop_rect(
                    clip,
                    time,
                    (w, h),
                    1.0,
                    (clip.size.0 as usize, clip.size.1 as usize),
                )
                .unwrap_or_else(|| Rect::new(0.0, 0.0, w, h));
                (three_d::Plane::of(scene, clip, (w, h), &place, time), crop)
            })
            .collect();
        // Two that cross are cut where they meet. Two that do not are one way round all over, but
        // not always the way their middles have them: then they too are drawn by the pixel, so
        // that a floor is not drawn over what stands on it for having its middle the nearer.
        // A mesh has no one plane to be before or behind, so what stands with one is drawn by
        // the pixel too.
        let meshes = run
            .iter()
            .any(|shown| matches!(shown.plan.source, Source::Mesh));
        let crosses = meshes
            || planes.iter().enumerate().any(|(index, (plane, crop))| {
                planes[index + 1..].iter().any(|(other, other_crop)| {
                    plane.crosses(*crop, other, *other_crop)
                        || !plane.surely_behind(*crop, other, *other_crop, scene)
                })
            });
        if crosses {
            run
        } else {
            Vec::new()
        }
    }

    /// Clips in depth that cross, each cut where it meets another, over `canvas`.
    fn crossing(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        run: &[&Shown],
        moment: &Moment,
    ) {
        let Moment { at, scale, scene } = *moment;
        let Some(scene) = scene else {
            return;
        };
        let size = (canvas.w, canvas.h);
        let layers: Vec<three_d::Layer> = run
            .iter()
            .filter_map(|shown| {
                if matches!(shown.plan.source, Source::Mesh) {
                    return self.mesh(sequence, shown, moment, size);
                }
                let ready = self.ready(sequence, shown, at, scale, size)?;
                let (picture, depth) = three_d::project_with_depth(
                    scene,
                    &shown.plan,
                    &ready.picture,
                    (&ready.to_output, ready.crop),
                    at - shown.plan.clip.start,
                    size,
                );
                Some(three_d::Layer {
                    picture,
                    depth,
                    opacity: ready.opacity,
                    blend: self.blend(shown.plan.blend),
                })
            })
            .collect();
        three_d::composite_by_depth(canvas, &layers);
    }

    /// A track of `sequence` at `moment` over `canvas`.
    fn track(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        track: &TrackPlan,
        moment: &Moment,
    ) {
        let at = moment.at;
        for (index, shown) in track.shown.iter().enumerate() {
            if !shown.shows(at) {
                continue;
            }
            match track.pair_at(index, at) {
                // Drawn once, at the first of its clips that shows.
                Some(pair) if index == pair.from || !track.shown[pair.from].shows(at) => {
                    self.pair(canvas, sequence, track, pair, moment);
                }
                Some(_) => {}
                None => self.layer(canvas, sequence, shown, moment, shown.plan.blend),
            }
        }
        if self.pass != Pass::Picture {
            return;
        }
        for dip in track.dips.iter().filter(|dip| dip.shows(at)) {
            let opacity = f64::from(dip.color.a) / 255.0
                * (1.0 - (2.0 * progress(at, dip.from, dip.to) - 1.0).abs());
            let solid = Image::filled(
                canvas.w,
                canvas.h,
                premultiplied(Rgba {
                    a: 255,
                    ..dip.color
                }),
            );
            composite(canvas, &solid, opacity as f32, Blend::Normal);
        }
    }

    /// Two clips mixed by a FilmCraft transition at sequence time `at`,
    /// over `canvas`.
    fn pair(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        track: &TrackPlan,
        pair: &Pair,
        moment: &Moment,
    ) {
        let Moment { at, scale, .. } = *moment;
        let (w, h) = (canvas.w, canvas.h);
        let (from, to) = (&track.shown[pair.from], &track.shown[pair.to]);
        let side = |shown: &Shown| {
            let mut picture = Image::new(w, h);
            if shown.shows(at) {
                self.layer(&mut picture, sequence, shown, moment, Blend::Normal);
            }
            picture
        };
        let (a, b) = match (pair.from == pair.to, pair.from_nothing) {
            (false, _) => (side(from), side(to)),
            // One clip alone: it enters from nothing, or leaves into it.
            (true, true) => (Image::new(w, h), side(to)),
            (true, false) => (side(from), Image::new(w, h)),
        };
        let Some(fx) = Fx::new(&pair.def, &pair.params) else {
            return;
        };
        let p = progress(at, pair.start, pair.end) as f32;
        let mixed = apply_scaled(&fx, &a, &b, p, scale as f32);
        // A fade from or to nothing keeps the one clip's blend mode;
        // between two they mix normally.
        let blend = match (from.shows(at), to.shows(at)) {
            (true, false) => from.plan.blend,
            (false, true) => to.plan.blend,
            _ if pair.from == pair.to => from.plan.blend,
            _ => Blend::Normal,
        };
        composite(canvas, &mixed, 1.0, self.blend(blend));
    }

    /// Draw a shown clip over `canvas` at `moment` with `blend`.
    fn layer(
        &self,
        canvas: &mut Image,
        sequence: &SequencePlan,
        shown: &Shown,
        moment: &Moment,
        blend: Blend,
    ) {
        let Moment { at, scale, scene } = *moment;
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        if matches!(plan.source, Source::Nothing) {
            return;
        }
        if matches!(plan.source, Source::Adjustment) {
            if self.pass == Pass::Picture {
                self.adjust(canvas, sequence, plan, time, scale);
            }
            return;
        }
        let blend = self.blend(blend);
        let (w, h) = (canvas.w, canvas.h);
        if matches!(plan.source, Source::Mesh) {
            if let Some(mesh) = self.mesh(sequence, shown, moment, (w, h)) {
                composite(canvas, &mesh.picture, mesh.opacity, blend);
            }
            return;
        }
        let Some(Ready {
            picture,
            to_output,
            crop,
            opacity,
        }) = self.ready(sequence, shown, at, scale, (w, h))
        else {
            return;
        };
        if let (true, Some(scene)) = (plan.three_d, scene) {
            let projected =
                three_d::project(scene, plan, &picture, (&to_output, crop), time, (w, h));
            composite(canvas, &projected, opacity, blend);
            return;
        }
        match crop {
            None if (picture.w, picture.h) == (w, h) && identity(&to_output) => {
                composite(canvas, &picture, opacity, blend);
            }
            crop => {
                let whole = Rect::new(0.0, 0.0, picture.w as f64, picture.h as f64);
                let crop = crop.unwrap_or(whole);
                composite_placed(canvas, &picture, &to_output, crop, opacity, blend);
            }
        }
    }

    /// A shown clip that is a mesh, as the scene of `moment` sees it on a canvas of `size`: its
    /// picture, lit, or as the pass drawn has it, and how far ahead it is at each pixel.
    fn mesh(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        moment: &Moment,
        size: (usize, usize),
    ) -> Option<three_d::Layer> {
        let Moment { at, scale, scene } = *moment;
        let scene = scene?;
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        // It has no picture for a wipe to cross: an entrance fades it in.
        let fade = shown
            .reveal
            .map_or(1.0, |(_, start, end)| progress(at, start, end));
        let opacity = (number(&param(clip, "opacity").value_at(time)) * fade) as f32;
        if opacity <= 0.0 {
            return None;
        }
        let shade = match self.pass {
            Pass::Picture => three_d::mesh::Shade::Lit(&clip.id),
            Pass::Ids => {
                let color = self
                    .ids
                    .iter()
                    .find(|(id, _)| *id == clip.id)
                    .map_or(Rgba::WHITE, |(_, color)| *color);
                let [r, g, b, _] = premultiplied(color);
                three_d::mesh::Shade::Flat([r, g, b])
            }
            Pass::Depth => three_d::mesh::Shade::Near,
        };
        let to_output =
            Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
        let (picture, depth) = three_d::mesh::draw(scene, plan, &to_output, time, size, shade);
        Some(three_d::Layer {
            picture,
            depth,
            opacity,
            blend: self.blend(plan.blend),
        })
    }

    /// A shown clip's picture at sequence time `at` with its effects and reveal done, and how it goes
    /// down, for a canvas of `(w, h)`.
    fn ready(
        &self,
        sequence: &SequencePlan,
        shown: &Shown,
        at: Time,
        scale: f64,
        (w, h): (usize, usize),
    ) -> Option<Ready> {
        let plan = &shown.plan;
        let clip = &plan.clip;
        let time = at - clip.start;
        let Input {
            mut picture,
            to_output,
            crop,
            per_source,
        } = self.input(sequence, plan, time, scale, (w, h))?;
        if !plan.effects.is_empty() {
            let placed = Placed {
                sequence,
                at,
                scale,
                to_output,
                per_source,
                env: true,
            };
            self.effects(
                Arc::make_mut(&mut picture),
                plan,
                time,
                &placed,
                plan.effects.len(),
            );
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
        if self.pass != Pass::Picture {
            let picture = Arc::make_mut(&mut picture);
            self.as_pass(sequence, plan, picture, &to_output, at, (w, h));
        }
        Some(Ready {
            picture,
            to_output,
            crop,
            opacity,
        })
    }

    /// A clip's picture at clip time `time` as its effects get it, on a
    /// `canvas` of output pixels: masked and cropped, or, letterboxed, on
    /// black the size of the canvas.
    fn input(
        &self,
        sequence: &SequencePlan,
        plan: &ClipPlan,
        time: Time,
        scale: f64,
        (w, h): (usize, usize),
    ) -> Option<Input> {
        // Shared with the frame cache until something changes it.
        let (mut picture, per_source) = self.picture(sequence, plan, time, scale)?;
        let size = (f64::from(plan.size.0), f64::from(plan.size.1));
        let place = Affine::scale(scale, scale).then_apply(&self.geometry(sequence, plan, time));
        let mut crop = crop_rect(plan, time, size, per_source, (picture.w, picture.h));
        Some(if plan.letterboxed {
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
            Input {
                picture: Arc::new(boxed),
                to_output: Affine::IDENTITY,
                crop,
                per_source: place.determinant().abs().sqrt(),
            }
        } else {
            let masks = mask::prepare(&plan.clip.masks, size, per_source, time);
            if let Some(covered) = mask::coverage(&masks, picture.w, picture.h) {
                Arc::make_mut(&mut picture).mul_coverage(&covered);
            }
            Input {
                picture,
                to_output: place.then_apply(&Affine::scale(1.0 / per_source, 1.0 / per_source)),
                crop,
                per_source,
            }
        })
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
                angle,
            } => {
                let inner = &self.sequences[*nested];
                let shown = nested_time(time, *source_in, speed, *duration, inner.rate);
                let image = self.draw_of(*nested, shown, scale, angle.as_ref());
                let per_source = image.w as f64 / f64::from(inner.width);
                (Arc::new(image), per_source)
            }
            Source::Solid(color) => (
                Arc::new(Image::filled(cw, ch, premultiplied(*color))),
                scale,
            ),
            Source::Title(title) => (Arc::new(title_on(title, (cw, ch), time)), scale),
            Source::Drawn(source) => {
                let run = SourceRun {
                    at: time,
                    size: (cw, ch),
                };
                (Arc::new(source.draw(&plan.clip, &run)), scale)
            }
            Source::Adjustment | Source::Nothing | Source::Mesh => return None,
        })
    }

    /// Where a clip's source pixels land on the canvas at clip time `time`,
    /// in canvas pixels: cropped, fitted, scaled, flipped, rotated about
    /// its anchor, and moved, as the graph places it, and then moved with each of its parents.
    fn geometry(&self, sequence: &SequencePlan, plan: &ClipPlan, time: Time) -> Affine {
        let canvas = (f64::from(sequence.width), f64::from(sequence.height));
        let mut geometry = place(plan, canvas, time).0;
        let at = plan.clip.start + time;
        let mut parent = plan.clip.parent.as_ref();
        while let Some(rig) = parent.and_then(|id| sequence.rigs.get(id)) {
            geometry = parent_motion(rig, canvas, at - rig.clip.start).then_apply(&geometry);
            parent = rig.clip.parent.as_ref();
        }
        geometry
    }

    /// Run a clip's effects before the one at `upto` on its picture at clip
    /// time `time`, placed as `placed` says. Each is found by its id and run
    /// the same way, whichever crate it is in.
    fn effects(
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
            if filmcraft::instance::leaves_alone(effect, time, self.working()) {
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
    fn hold_headroom(&self, picture: &mut Image, effect: &Effect) -> Held {
        if self.hdr.is_none() {
            return Held::Nothing;
        }
        let way = filmcraft::catalog::headroom(effect.fx.as_str());
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
        // FilmCraft's effects see no clip of their own here.
        let placed = Placed {
            sequence,
            at: plan.clip.start + time,
            scale,
            to_output: Affine::IDENTITY,
            per_source: scale,
            env: false,
        };
        self.effects(&mut adjusted, plan, time, &placed, plan.effects.len());
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
            .as_chunks::<4>().0.iter()
            .map(|pixel| pixel[3] * opacity)
            .collect();
        if plan.blend == Blend::Normal {
            for ((pixel, adjusted), weight) in canvas
                .px
                .as_chunks_mut::<4>().0.iter_mut()
                .zip(adjusted.px.as_chunks::<4>().0.iter())
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

/// A number for everything about `clip` that does not change over time, wherever it starts, and
/// the file it shows. `None` when it draws on something the document does not hold.
fn clip_digest(clip: &Clip, catalog: &Catalog, media: Option<&Media>) -> Option<u64> {
    // A light wrapped from another track depends on what that track draws, hidden or not.
    if let Content::Generator { fx, params, .. } = &clip.content {
        let wraps = |param: &Param| !matches!(param, Param::Static(Value::Text(track)) if track.is_empty());
        if fx.as_str() == "light" && params.get("image").is_some_and(wraps) {
            return None;
        }
    }
    for effect in &clip.effects {
        let def = catalog.fx(effect.fx.as_str())?;
        let outside = def.params.iter().any(|param| {
            matches!(
                param.ty,
                ParamType::Track | ParamType::Sound | ParamType::File(_)
            ) && !effect_text_is_empty(effect, &param.name)
        });
        if outside {
            return None;
        }
    }
    let mut hash = DefaultHasher::new();
    let mut unplaced = clip.clone();
    unplaced.start = Time::ZERO;
    format!("{unplaced:?}").hash(&mut hash);
    // A file a generator reads, a mesh's, replaced under the same name is another picture.
    if let Content::Generator { fx, params, .. } = &clip.content {
        let def = catalog.fx(fx.as_str())?;
        for param in &def.params {
            if !matches!(param.ty, ParamType::File(_)) {
                continue;
            }
            if let Some(Param::Static(Value::Text(path))) = params.get(&param.name) {
                if !path.is_empty() {
                    let file = std::fs::metadata(path).ok()?;
                    file.len().hash(&mut hash);
                    file.modified().ok()?.hash(&mut hash);
                }
            }
        }
    }
    if let Some(media) = media {
        format!("{media:?}").hash(&mut hash);
        // A file replaced under the same name is another picture.
        let file = std::fs::metadata(&media.path).ok()?;
        file.len().hash(&mut hash);
        file.modified().ok()?.hash(&mut hash);
    }
    Some(hash.finish())
}

/// Whether effect parameter `name` holds no text, as a track or file not chosen does.
fn effect_text_is_empty(effect: &Effect, name: &str) -> bool {
    match effect.params.get(name) {
        Some(Param::Static(Value::Text(text))) => text.is_empty(),
        Some(_) => false,
        None => true,
    }
}

/// All that goes into what [`title_on`] draws at `time`, to keep that picture by. Most titles have no
/// keys and come to the same each frame, as a reveal does once it is over; one whose numbers move has
/// a key of its own at each frame, which is never kept.
fn title_key(title: &TitlePlan, (w, h): (usize, usize), time: Time) -> String {
    use std::fmt::Write;
    let mut key = format!(
        "title|{w}x{h}|{:?}|{:?}|{:x}|{:?}|{:?}",
        title.text,
        title.font,
        title.size.to_bits(),
        title.color,
        title.justify
    );
    let mut value = |param: &Param| {
        let _ = write!(key, "|{:x}", number(&param.value_at(time)).to_bits());
    };
    for (_, param) in &title.axes {
        value(param);
    }
    if let Some(reveal) = &title.reveal {
        value(&reveal.reveal);
        value(&reveal.soft);
        value(&reveal.from[0]);
        value(&reveal.from[1]);
        value(&reveal.scale);
        value(&reveal.rotation);
        value(&reveal.opacity);
    }
    for (tag, _) in &title.axes {
        let _ = write!(key, "|{tag:?}");
    }
    let _ = write!(key, "|{:?}", title.features);
    if let Some(reveal) = &title.reveal {
        let _ = write!(key, "|{}", reveal.by_word);
    }
    key
}

/// A title drawn on a transparent `w` × `h` picture.
fn title_on(title: &TitlePlan, (w, h): (usize, usize), time: Time) -> Image {
    let mut image = Image::new(w, h);
    // At 0, an axis is left as the font has it.
    let axes: Vec<([u8; 4], f32)> = title
        .axes
        .iter()
        .map(|(tag, param)| (*tag, number(&param.value_at(time)) as f32))
        .filter(|(_, value)| *value != 0.0)
        .collect();
    let text = Text {
        text: &title.text,
        font: &title.font,
        size: (title.size * h as f64).max(1.0) as f32,
        color: title.color,
        justify: title.justify,
        outline: None,
        axes: &axes,
        features: &title.features,
    };
    let at = [w as f32 / 2.0, h as f32 / 2.0];
    let Some(reveal) = &title.reveal else {
        text::draw(&mut image, &text, at, 0.5);
        return image;
    };
    let read = |param: &Param| number(&param.value_at(time));
    let (progress, soft) = (read(&reveal.reveal), read(&reveal.soft).max(0.01));
    let down = h as f64;
    let (from_x, from_y, from_scale, from_rotation, from_opacity) = (
        read(&reveal.from[0]) * down,
        read(&reveal.from[1]) * down,
        read(&reveal.scale),
        read(&reveal.rotation).to_radians(),
        read(&reveal.opacity),
    );
    let look = |piece: usize, pieces: usize| {
        let coming = ((progress * (pieces as f64 + soft) - piece as f64) / soft).clamp(0.0, 1.0);
        let along = coming * coming * (3.0 - 2.0 * coming);
        let away = 1.0 - along;
        text::Piece {
            offset: [(from_x * away) as f32, (from_y * away) as f32],
            scale: (from_scale + (1.0 - from_scale) * along) as f32,
            rotation: (from_rotation * away) as f32,
            opacity: (from_opacity + (1.0 - from_opacity) * along) as f32,
        }
    };
    text::draw_pieces(&mut image, &text, at, 0.5, reveal.by_word, &look);
    image
}

/// The size a caption's text is drawn at.
fn caption_size(sequence: &SequencePlan, scale: f64) -> f64 {
    (f64::from(sequence.height) * 0.045).max(12.0) * scale
}

/// A caption's text, as it is burned in.
fn caption_text(caption: &CaptionPlan, size: f64) -> Text<'_> {
    Text {
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
        axes: &[],
        features: &[],
    }
}

/// A caption drawn on a transparent `w` × `h` picture.
/// All that goes into what [`caption_on`] draws, to keep that picture by.
fn caption_key(
    caption: &CaptionPlan,
    sequence: &SequencePlan,
    scale: f64,
    (w, h): (usize, usize),
) -> String {
    let size = caption_size(sequence, scale);
    let text = caption_text(caption, size);
    format!(
        "caption|{w}x{h}|{:?}|{:?}|{:x}|{:?}|{:?}|{:?}",
        text.text,
        text.font,
        text.size.to_bits(),
        text.color,
        text.justify,
        text.outline
    )
}

fn caption_on(
    caption: &CaptionPlan,
    sequence: &SequencePlan,
    scale: f64,
    (w, h): (usize, usize),
) -> Image {
    let mut image = Image::new(w, h);
    let text = caption_text(caption, caption_size(sequence, scale));
    text::draw(&mut image, &text, [w as f32 / 2.0, h as f32 * 0.95], 1.0);
    image
}

/// What a sequence is being drawn at: when, how large, and, with clips in depth, how they are seen.
#[derive(Clone, Copy)]
struct Moment<'a> {
    at: Time,
    scale: f64,
    scene: Option<&'a three_d::Scene>,
}

/// A clip's picture with all done to it that is done to the picture itself, and how it goes down.
struct Ready {
    picture: Arc<Image>,
    /// Its pixels to output pixels.
    to_output: Affine,
    /// What its crop leaves of it, in its pixels.
    crop: Option<Rect>,
    opacity: f32,
}

/// A clip's picture as its effects get it.
struct Input {
    picture: Arc<Image>,
    /// Its pixels to output pixels.
    to_output: Affine,
    /// What its crop leaves of it, in its pixels.
    crop: Option<Rect>,
    /// How many of its pixels a source pixel is.
    per_source: f64,
}

/// Where a clip's effects run: on its picture at sequence time `at`, drawn
/// at `scale` of `sequence`'s canvas.
struct Placed<'a> {
    sequence: &'a SequencePlan,
    at: Time,
    scale: f64,
    to_output: Affine,
    per_source: f64,
    /// Whether FilmCraft's effects may read the clip's other frames and the
    /// sequence's other tracks.
    env: bool,
}

/// What the effect at `upto` among a clip's effects reads beyond its
/// picture.
#[derive(Clone, Copy)]
struct ClipEnv<'a> {
    drawing: &'a Drawing,
    plan: &'a ClipPlan,
    placed: &'a Placed<'a>,
    upto: usize,
    /// The size of the picture the effect runs on.
    working: (usize, usize),
    /// What the effect's picture had taken from it, and what was left divided by, when an HDR
    /// picture was brought under 1 for it whole: the clip's other frames meet it the same way.
    lift: Option<(f32, f32)>,
}

impl ClipEnv<'_> {
    /// The clip's picture at sequence time `at`, as the effect gets it.
    fn picture_at(&self, at: Time) -> Option<Image> {
        let placed = self.placed;
        let time = at - self.plan.clip.start;
        let (cw, ch) = output_size(placed.sequence.width, placed.sequence.height, placed.scale);
        let input = self
            .drawing
            .input(placed.sequence, self.plan, time, placed.scale, (cw, ch))?;
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
        let moment = Moment {
            at,
            scale,
            scene: scene.as_ref(),
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

impl TrackPlan {
    /// Whether it is drawn: as it says, unless a clip holding its sequence names one track, which
    /// is drawn even when hidden, as the angles of a multicam sequence may be.
    fn drawn(&self, angle: Option<&Id>) -> bool {
        match angle {
            Some(angle) => self.id == *angle,
            None => self.shows,
        }
    }

    /// The transition mixing its clip `index` at `at`.
    fn pair_at(&self, index: usize, at: Time) -> Option<&Pair> {
        self.pairs.iter().find(|pair| {
            pair.start <= at && at < pair.end && (pair.from == index || pair.to == index)
        })
    }

    /// The one clip in depth it shows at `at`, drawing a picture of its own: not in a transition or a
    /// dip, whose mixes are of whole pictures, which cannot be cut.
    fn lone_in_depth(&self, at: Time) -> Option<&Shown> {
        let mut showing = self
            .shown
            .iter()
            .enumerate()
            .filter(|(_, shown)| shown.shows(at));
        let (index, shown) = showing.next()?;
        let mixed = self.dips.iter().any(|dip| dip.shows(at)) || self.pair_at(index, at).is_some();
        let drawn = !matches!(shown.plan.source, Source::Nothing | Source::Adjustment);
        (showing.next().is_none() && shown.plan.three_d && drawn && !mixed).then_some(shown)
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
    /// The effects of every clip planned, for the LUT files they name.
    effects: Vec<Effect>,
    /// The program is an HDR sequence, so footage keeps what SDR could not show.
    hdr: bool,
}

impl<'a> Builder<'a> {
    fn sequence(&mut self, sequence: &'a Sequence) -> Result<SequencePlan> {
        let (width, height) = canvas(sequence);
        let mut tracks = Vec::new();
        let mut captions = Vec::new();
        let mut sounds = Vec::new();
        for track in &sequence.tracks {
            match track.kind {
                TrackKind::Audio => sounds.push(SoundPlan {
                    id: track.id.to_string(),
                    clips: track
                        .clips
                        .iter()
                        .filter(|clip| clip.enabled)
                        .filter_map(|clip| {
                            let Content::Media {
                                media,
                                source_in,
                                speed,
                            } = &clip.content
                            else {
                                return None;
                            };
                            let media = self.project.media(media.as_str())?;
                            media.audio.as_ref()?;
                            Some(SoundClip {
                                path: media.path.clone(),
                                start: clip.start,
                                duration: clip.duration,
                                source_in: *source_in,
                                speed: speed.clone(),
                            })
                        })
                        .collect(),
                }),
                TrackKind::Video if track.enabled => tracks.push(self.track(sequence, track)?),
                // Effects read hidden tracks too; one that cannot be drawn
                // reads as empty.
                TrackKind::Video => tracks.push(self.track(sequence, track).unwrap_or(TrackPlan {
                    id: track.id.clone(),
                    shows: false,
                    shown: Vec::new(),
                    dips: Vec::new(),
                    pairs: Vec::new(),
                })),
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
        let rigs = tracks
            .iter()
            .flat_map(|track: &TrackPlan| &track.shown)
            .map(|shown| &shown.plan)
            .filter(|plan| {
                sequence
                    .tracks
                    .iter()
                    .flat_map(|track| &track.clips)
                    .any(|clip| clip.parent.as_ref() == Some(&plan.clip.id))
            })
            .map(|plan| {
                (
                    plan.clip.id.clone(),
                    Rig {
                        clip: plan.clip.clone(),
                        size: plan.size,
                        fit: plan.fit,
                        path: plan.path.clone(),
                        digest: plan.digest,
                    },
                )
            })
            .collect();
        Ok(SequencePlan {
            rigs,
            name: sequence.name.clone(),
            width,
            height,
            rate: sequence.rate,
            start: sequence.start,
            drop_frame: sequence.drop_frame,
            duration: sequence.duration(),
            track_ids: tracks.iter().map(|track| track.id.to_string()).collect(),
            tracks,
            sound_ids: sounds.iter().map(|sound| sound.id.clone()).collect(),
            sounds,
            captions,
        })
    }

    fn track(&mut self, sequence: &'a Sequence, track: &'a Track) -> Result<TrackPlan> {
        let rate = sequence.rate;
        let mut spans: Vec<(Time, Time, Option<Entrance>, bool)> = track
            .clips
            .iter()
            .map(|clip| (clip.start, clip.end(), None, false))
            .collect();
        let mut dips = Vec::new();
        // FilmCraft's transitions, by the track's clips they join.
        let mut mixes = Vec::new();
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
                kerf_model::look::STACK => Reveal::Over,
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
                other if kerf_fx::transitions::filmcraft_id(other).is_some() => {
                    let def = self.catalog.fx(other).ok_or_else(|| {
                        ComposeError::Invalid(format!("unknown transition `{other}`"))
                    })?;
                    // Both clips show through the transition, which mixes them.
                    spans[from].1 = end;
                    spans[from].3 = true;
                    spans[to].0 = start;
                    mixes.push((from, to, def.clone(), transition.params.clone(), start, end));
                    continue;
                }
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
        // Each clip's place in `shown`, by its place on the track.
        let mut places = HashMap::new();
        for (index, (clip, (from, to, reveal, fades_out))) in
            track.clips.iter().zip(spans).enumerate()
        {
            if !clip.enabled || matches!(clip.content, Content::Caption { .. }) {
                continue;
            }
            let adjustment =
                matches!(&clip.content, Content::Generator { fx, .. } if fx == "adjustment");
            let mixed = mixes.iter().any(|mix| mix.0 == index || mix.1 == index);
            if adjustment && (reveal.is_some() || fades_out || mixed) {
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
            let Some(plan) = self.clip(sequence, clip, from, to, reveal.is_none() && !mixed)?
            else {
                continue;
            };
            places.insert(index, shown.len());
            shown.push(Shown {
                plan,
                from,
                to,
                reveal,
            });
        }
        let pairs = mixes
            .into_iter()
            .filter_map(|(from, to, def, params, start, end)| {
                let (from, to, from_nothing) = match (places.get(&from), places.get(&to)) {
                    (Some(from), Some(to)) => (*from, *to, false),
                    // One side alone mixes with nothing.
                    (Some(only), None) => (*only, *only, false),
                    (None, Some(only)) => (*only, *only, true),
                    // Neither shows: nothing to draw.
                    (None, None) => return None,
                };
                Some(Pair {
                    from,
                    to,
                    from_nothing,
                    def,
                    params,
                    start,
                    end,
                })
            })
            .collect();
        Ok(TrackPlan {
            id: track.id.clone(),
            shows: track.enabled,
            shown,
            dips,
            pairs,
        })
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
        if let Some(length) = length {
            let ends = (clip.played_at(Time::ZERO)?, clip.played_at(length)?);
            let (earliest, latest) = if clip.speed().is_reverse() {
                (ends.1, ends.0)
            } else {
                ends
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
        let mut name = clip.name.clone().unwrap_or_else(|| clip.id.to_string());
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
                if clip.name.is_none() {
                    if let Some(file) = std::path::Path::new(&media.path).file_name() {
                        name = file.to_string_lossy().into_owned();
                    }
                }
                self.clips.push(ClipSource {
                    path: media.path.clone(),
                    rate: media.rate,
                    frames: frame_count(media.rate, media.duration),
                    width: video.width,
                    height: video.height,
                    alpha: video.alpha,
                    deep: video
                        .is_deep()
                        .then(|| crate::footage::input_transform(video.color, self.hdr)),
                    retime: clip.retime,
                    source_in: *source_in,
                    speed: speed.clone(),
                    duration: clip.duration,
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
                if clip.name.is_none() {
                    name = inner.name.clone();
                }
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
                        speed: speed.clone(),
                        duration: clip.duration,
                        angle: clip.angle.clone(),
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
                "mesh" => (Source::Mesh, canvas, false, true),
                "null" | "camera" | "light" | kerf_model::SIGNAL => {
                    (Source::Nothing, canvas, false, true)
                }
                other => match domains::source(other) {
                    Some(source) => (Source::Drawn(source), canvas, false, true),
                    None => return Err(unsupported(&clip.id, format!("generator `{other}`"))),
                },
            },
            Content::Caption { .. } => return Ok(None),
        };
        let animated = clip.parent.is_some()
            || !matches!(param(clip, "path"), Param::Static(Value::Text(text)) if text.is_empty())
            || GEOMETRY
                .iter()
                .any(|name| param(clip, name).varies());
        // A mesh is in depth whether it says so or not.
        let three_d = matches!(param(clip, "three_d"), Param::Static(Value::Bool(true)))
            || matches!(source, Source::Mesh);
        if matches!(source, Source::Mesh) {
            // It has no picture for either to be done to.
            if clip.effects.iter().any(|effect| effect.enabled) {
                return Err(unsupported(&clip.id, "an effect on a mesh"));
            }
            if !clip.masks.is_empty() {
                return Err(unsupported(&clip.id, "a mask on a mesh"));
            }
            three_d::mesh::check(clip).map_err(|why| {
                ComposeError::Invalid(format!("clip `{}`: its mesh's file {why}", clip.id))
            })?;
        }
        let mut effects = Vec::new();
        for effect in clip.effects.iter().filter(|effect| effect.enabled) {
            let def = self.catalog.fx(effect.fx.as_str()).ok_or_else(|| {
                ComposeError::Invalid(format!("clip `{}` uses unknown effect", clip.id))
            })?;
            if def.domain != Domain::Video {
                continue;
            }
            match effect.fx.as_str() {
                // Part of the shape, which reads it where it draws.
                kerf_model::SHAPE_OUTLINE => {}
                id if domains::effect(id).is_some() => effects.push(effect.clone()),
                other => {
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
            Param::Animated(_) | Param::Expr(_) => {
                return Err(unsupported(&clip.id, "animated `blend`"))
            }
        };
        self.effects.extend(effects.iter().cloned());
        let media = match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()),
            _ => None,
        };
        let digest = clip_digest(clip, self.catalog, media);
        let reads_sequence_time = effects
            .iter()
            .any(|effect| effect.fx.as_str().starts_with("text."));
        let path = motion_path(clip, canvas);
        let mut plan = ClipPlan {
            clip: clip.clone(),
            name,
            source,
            size,
            fit,
            letterboxed: false,
            plain: false,
            animated,
            three_d,
            shown: 1.0,
            blend,
            effects,
            path,
            digest,
            reads_sequence_time,
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
            // A parent can scale it past anything its own properties say.
            let moved = clip.parent.is_some() || plan.path.is_some() || three_d;
            plan.shown = if moved { 1.0 } else { largest.clamp(1e-6, 1.0) };
            plan.letterboxed = !alpha && !moved && letterboxed(&plan, canvas);
            let deep = media
                .and_then(|media| media.video)
                .is_some_and(|video| video.is_deep());
            plan.plain = !alpha
                && !deep
                && clip.retime == kerf_model::Retime::Frames
                && !moved
                && self.nesting.len() == 1
                && entering
                && size == canvas
                && untouched(&plan);
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
                Some(Param::Animated(_) | Param::Expr(_)) => {
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
            axes: kerf_model::TITLE_AXES
                .iter()
                .filter_map(|tag| Some((*tag, params.get(std::str::from_utf8(tag).ok()?)?.clone())))
                .collect(),
            // The document is refused where they do not read; a title built past it draws without.
            features: match get("features")? {
                Value::Text(features) => kerf_model::parse_features(&features).unwrap_or_default(),
                _ => Vec::new(),
            },
            reveal: {
                let raw = |name: &str| {
                    params
                        .get(name)
                        .cloned()
                        .unwrap_or_else(|| Param::Static(def.find(name).expect("title parameter").default.clone()))
                };
                // Left at rest, it draws as plain text does.
                params.contains_key("reveal").then(|| Box::new(Reveals {
                    reveal: raw("reveal"),
                    by_word: matches!(params.get("reveal_by"), Some(Param::Static(Value::Enum(by))) if by == "word"),
                    soft: raw("reveal_soft"),
                    from: [raw("from_x"), raw("from_y")],
                    scale: raw("from_scale"),
                    rotation: raw("from_rotation"),
                    opacity: raw("from_opacity"),
                }))
            },
        })
    }
}

/// Where a clip's source pixels land on a `canvas` at clip time `time`,
/// and how many canvas pixels a source pixel is across and down.
fn place(plan: &ClipPlan, canvas: (f64, f64), time: Time) -> (Affine, Vec2) {
    let numbers = geometry_numbers(&plan.clip, plan.path.as_ref(), time);
    placed(plan.size, plan.fit, !plan.animated, canvas, numbers)
}

/// The values of a clip's [`GEOMETRY`] at clip time `time`, with its motion path added in.
fn geometry_numbers(clip: &Clip, path: Option<&MotionPath>, time: Time) -> [f64; 12] {
    let mut numbers = GEOMETRY.map(|name| number(&param(clip, name).value_at(time)));
    if let Some(path) = path {
        let on = path.at(number(&param(clip, "along").value_at(time)));
        numbers[10] += on.x;
        numbers[11] += on.y;
        if matches!(param(clip, "orient"), Param::Static(Value::Bool(true))) {
            numbers[7] += on.heading;
        }
    }
    numbers
}

/// A clip's motion path on a canvas of this shape, if it has one.
fn motion_path(clip: &Clip, canvas: (u32, u32)) -> Option<MotionPath> {
    let Param::Static(Value::Text(text)) = param(clip, "path") else {
        return None;
    };
    let points = kerf_model::parse_path_points(&text).ok()?;
    let aspect = f64::from(canvas.0) / f64::from(canvas.1.max(1));
    Some(MotionPath::new(points, aspect))
}

/// [`place`] from the geometry's values: pixels of a source of `size`, fitted to the canvas when
/// `fit`, and on whole pixels when `snap` and nothing rules that out.
fn placed(
    size: (u32, u32),
    fit: bool,
    snap: bool,
    (cw, ch): (f64, f64),
    [l, r, t, b, s, sx, sy, rotation, ax, ay, x, y]: [f64; 12],
) -> (Affine, Vec2) {
    let (sw, sh) = (f64::from(size.0), f64::from(size.1));
    let (width, height) = (sw * (1.0 - l - r), sh * (1.0 - t - b));
    let (fw, fh) = if fit {
        fitted(width, height, cw, ch)
    } else {
        (width, height)
    };
    let (kx, ky) = (s * sx, s * sy);
    let mut position = Vec2::new(cw / 2.0 + x * cw + ax * fw, ch / 2.0 + y * ch + ay * fh);
    let mut scale = Vec2::new(fw / width * kx, fh / height * ky);
    if snap && rotation == 0.0 && ax == 0.0 && ay == 0.0 {
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

/// How a parent has moved its children at its clip time `time`: from where it would sit with no
/// position, scale, or rotation of its own, to where it is.
fn parent_motion(rig: &Rig, canvas: (f64, f64), time: Time) -> Affine {
    let numbers = geometry_numbers(&rig.clip, rig.path.as_ref(), time);
    let [l, r, t, b, _, _, _, _, ax, ay, _, _] = numbers;
    let rest = [l, r, t, b, 1.0, 1.0, 1.0, 0.0, ax, ay, 0.0, 0.0];
    let (now, _) = placed(rig.size, rig.fit, false, canvas, numbers);
    let (before, _) = placed(rig.size, rig.fit, false, canvas, rest);
    before
        .inverse()
        .map_or(Affine::IDENTITY, |inverse| now.then_apply(&inverse))
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
fn nested_time(time: Time, source_in: Time, speed: &Speed, duration: Time, rate: Rate) -> Time {
    let span = speed.played(time.max(Time::ZERO)).flicks();
    let frame = rate.frame_duration().flicks().max(1);
    let index = if speed.is_reverse() {
        // The frame showing just before the source position.
        let position = (source_in + speed.played(duration)).flicks() - span;
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

    /// The same table holds the editor's overlay to this, in `graph_test.cpp`
    /// (`parentsMoveAsTheCompositorMovesThem`).
    #[test]
    fn a_parents_motion_is_where_the_table_says() {
        type Row = (
            &'static str,
            (u32, u32),
            bool,
            (f64, f64),
            (f64, f64),
            (f64, f64),
        );
        const HD: (u32, u32) = (1920, 1080);
        const HD_F: (f64, f64) = (1920.0, 1080.0);
        let table: [Row; 7] = [
            (
                "x=-0.25 y=0.1",
                HD,
                false,
                HD_F,
                (1200.0, 540.0),
                (720.0, 648.0),
            ),
            (
                "rotation=90",
                HD,
                false,
                HD_F,
                (1200.0, 540.0),
                (960.0, 780.0),
            ),
            (
                "scale=0.5",
                HD,
                false,
                HD_F,
                (1200.0, 300.0),
                (1080.0, 420.0),
            ),
            (
                "scale_x=2 rotation=30 anchor_x=0.25 anchor_y=-0.5 x=0.1",
                HD,
                false,
                HD_F,
                (100.0, 900.0),
                (-1138.948082142, -560.577136594),
            ),
            (
                "crop_l=0.2 crop_t=0.1 scale=1.5 y=-0.2",
                (1280, 720),
                true,
                HD_F,
                (640.0, 200.0),
                (480.0, -186.0),
            ),
            (
                "rotation=45 anchor_x=0.5 scale=0.75",
                (1000, 500),
                true,
                (1080.0, 1920.0),
                (540.0, 960.0),
                (793.621753619, 673.621753619),
            ),
            (
                "path=\"0,0 0.2,0.3 0.5,-0.1\" along=0.5 orient=true scale=0.8",
                HD,
                false,
                HD_F,
                (1000.0, 500.0),
                (1462.428694397, 793.07977502),
            ),
        ];
        for (set, size, fit, canvas, from, to) in table {
            let document = format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=1920x1080 audio=48000\n  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n      set {set}\n"
            );
            let project = kerf_lang::parse_document(&document, Catalog::builtin())
                .unwrap_or_else(|errors| panic!("{set}: {errors:?}"))
                .project;
            let clip = project.sequences[0].tracks[0].clips[0].clone();
            let shape = (canvas.0 as u32, canvas.1 as u32);
            let path = motion_path(&clip, shape);
            let rig = Rig {
                clip,
                size,
                fit,
                path,
                digest: None,
            };
            let moved = parent_motion(&rig, canvas, Time::ZERO);
            let at = moved.apply(Vec2::new(from.0, from.1));
            assert!(
                (at.x - to.0).abs() < 1e-6 && (at.y - to.1).abs() < 1e-6,
                "{set}: {at:?} against {to:?}"
            );
        }
    }

    /// The same table holds the editor's overlays to this, in `graph_test.cpp`
    /// (`clipsInDepthLandAsTheCompositorDrawsThem`). Each row is a camera, a parent, a clip, its
    /// picture's size and whether it is fitted, and the canvas; then a point of the picture, in
    /// fractions of it, and where on the canvas it lands, or that no pixel shows it. A row with a path
    /// names the clip's anchor, which the path says the place of.
    #[test]
    fn a_clip_in_depth_lands_where_the_table_says() {
        type Row = (
            &'static str,
            &'static str,
            &'static str,
            (u32, u32),
            bool,
            (u32, u32),
            (f64, f64),
            Option<(f64, f64)>,
        );
        #[rustfmt::skip]
        let table: [Row; 25] = [
            ("", "", "three_d=true", (1920, 1080), false, (1920, 1080), (0.25, 0.75), Some((480.0, 810.0))),
            ("", "", "three_d=true z=1", (1920, 1080), false, (1920, 1080), (0.0, 0.0), Some((404.425304296, 227.489233666))),
            ("", "", "three_d=true z=-0.5 x=0.25 y=-0.25 scale=0.5", (1920, 1080), false, (1920, 1080), (1.0, 1.0), Some((2469.363321845, 540.0))),
            ("", "", "three_d=true rotate_y=60", (1920, 1080), false, (1920, 1080), (0.0, 0.0), Some((652.380423425, 193.927976353))),
            ("", "", "three_d=true rotate_y=60", (1920, 1080), false, (1920, 1080), (1.0, 1.0), Some((2051.824322445, 1768.30236275))),
            ("", "", "three_d=true z=0.4 rotate_x=-35 rotate_y=20 scale=0.6 rotation=30 x=0.15", (1920, 1080), false, (1920, 1080), (0.1, 0.9), Some((813.293244888, 536.56976567))),
            ("x=0.1 y=-0.1 z=-0.2", "", "three_d=true z=0.5", (1920, 1080), false, (1920, 1080), (0.75, 0.25), Some((1150.784280848, 432.683842023))),
            ("pan=15 tilt=-10 roll=20 fov=60", "", "three_d=true rotate_y=-25", (1920, 1080), false, (1920, 1080), (0.9, 0.1), Some((925.135566674, 137.609390156))),
            ("", "", "three_d=true crop_l=0.2 crop_t=0.1 anchor_x=0.25 rotate_y=30", (1920, 1080), false, (1920, 1080), (0.6, 0.6), Some((932.497292288, 597.735845172))),
            ("tilt=5", "", "three_d=true rotate_x=25 scale=0.75", (1280, 720), true, (1080, 1920), (0.2, 0.8), Some((300.339390717, 1313.504377609))),
            ("", "x=-0.1 rotation=20", "three_d=true z=0.5 rotate_y=30 x=0.2", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((1083.787134162, 636.289297037))),
            ("pan=10", "", "three_d=true z=1 rotate_y=40 path=\"0,0 0.2,0.3 0.5,-0.1\" along=0", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((698.394625114, 540.0))),
            ("pan=10", "", "three_d=true z=1 rotate_y=40 path=\"0,0 0.2,0.3 0.5,-0.1\" along=0.5", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((980.39775465, 708.363349572))),
            ("pan=10", "", "three_d=true z=1 rotate_y=40 path=\"0,0 0.2,0.3 0.5,-0.1\" along=1", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((1235.761146107, 480.464694441))),
            ("", "", "three_d=true z=0.5 rotate_y=40 anchor_x=0.25 path=\"0,0 0.2,0.3 0.5,-0.1\" along=0.25", (1920, 1080), false, (1920, 1080), (0.75, 0.5), Some((1461.484615779, 736.839478992))),
            ("", "", "three_d=true z=0.5 rotate_y=40 anchor_x=0.25 path=\"0,0 0.2,0.3 0.5,-0.1\" along=1", (1920, 1080), false, (1920, 1080), (0.75, 0.5), Some((2108.520888367, 446.575181565))),
            ("", "x=-0.1 scale=0.8", "three_d=true z=0.5 path=\"0,0 0.2,0.3\" along=1", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((1044.45932111, 730.033472497))),
            ("", "", "three_d=true rotate_y=30 orient=true path=\"0,0 0.2,0.3 0.5,-0.1\" along=0.5", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((1448.468778868, 836.127664149))),
            // A path it faces along, from an anchor off the middle of a turned clip: `sampledPath` in the editor.
            ("", "", "three_d=true z=0.2 rotate_y=30 orient=true anchor_x=0.25 path=\"0,0 0.2,0.3 0.5,-0.1\" along=0.5", (1920, 1080), false, (1920, 1080), (0.75, 0.5), Some((1869.824956407, 835.685078047))),
            ("", "", "three_d=true z=0.2 rotate_y=30 orient=true anchor_x=0.25 path=\"0,0 0.2,0.3 0.5,-0.1\" along=1", (1920, 1080), false, (1920, 1080), (0.75, 0.5), Some((2326.408450295, 433.723742822))),
            ("", "", "three_d=true z=-5", (1920, 1080), false, (1920, 1080), (0.5, 0.5), None),
            ("", "", "three_d=true rotate_y=90", (1920, 1080), false, (1920, 1080), (0.75, 0.5), None),
            ("", "", "three_d=true x=0.1 rotate_y=90", (1920, 1080), false, (1920, 1080), (0.75, 0.5), Some((1243.825954944, 540.0))),
            ("pan=80", "", "three_d=true", (1920, 1080), false, (1920, 1080), (0.0, 0.5), None),
            ("pan=80", "", "three_d=true", (1920, 1080), false, (1920, 1080), (1.0, 0.5), Some((-636.292895473, 540.0))),
        ];
        for (camera, rig, set, size, fit, canvas, from, to) in table {
            let line = |set: &str| {
                if set.is_empty() {
                    String::new()
                } else {
                    format!("      set {set}\n")
                }
            };
            let parent = if rig.is_empty() { "" } else { " parent=n1" };
            let document = format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size={}x{} audio=48000\n  track V1 video\n    clip n1 gen=null@1 at=0f dur=48f\n{}  track V2 video\n    clip c1 gen=solid@1 at=0f dur=48f{parent}\n      set {set}\n  track V3 video\n    clip cam gen=camera@1 at=0f dur=48f\n{}",
                canvas.0,
                canvas.1,
                line(rig),
                line(camera),
            );
            let project = kerf_lang::parse_document(&document, Catalog::builtin())
                .unwrap_or_else(|errors| panic!("{set}: {errors:?}"))
                .project;
            let mut drawing =
                Drawing::new(&project, Catalog::builtin(), &ComposeOptions::default())
                    .unwrap_or_else(|error| panic!("{set}: {error}"));
            // What is planned for a solid, but for what a piece of media would be.
            let plan = &mut drawing.sequences[0].tracks[1].shown[0].plan;
            plan.size = size;
            plan.fit = fit;
            let sequence = &drawing.sequences[0];
            let plan = &sequence.tracks[1].shown[0].plan;
            let scene =
                three_d::Scene::at(sequence, Time::ZERO, (canvas.0 as usize, canvas.1 as usize));
            let (w, h) = (f64::from(size.0), f64::from(size.1));
            let place = drawing.geometry(sequence, plan, Time::ZERO);
            let plane = three_d::Plane::of(&scene, plan, (w, h), &place, Time::ZERO);
            let wanted = (from.0 * w, from.1 * h);
            match to {
                Some(to) => {
                    let (hit, _) = scene
                        .meets(&plane, to)
                        .unwrap_or_else(|| panic!("{camera} {set}: nothing at {to:?}"));
                    let (u, v) = plane.at(hit);
                    assert!(
                        (u - wanted.0).abs() < 1e-6 && (v - wanted.1).abs() < 1e-6,
                        "{camera} {set}: {to:?} shows {u},{v}, not {wanted:?}"
                    );
                }
                None => {
                    for (x, y) in (0..canvas.0)
                        .step_by(4)
                        .flat_map(|x| (0..canvas.1).step_by(4).map(move |y| (x, y)))
                    {
                        let Some((hit, _)) = scene.meets(&plane, (f64::from(x), f64::from(y)))
                        else {
                            continue;
                        };
                        let (u, v) = plane.at(hit);
                        assert!(
                            (u / w - from.0).hypot(v / h - from.1) > 0.02,
                            "{camera} {set}: {x},{y} shows {u},{v}"
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn nested_sequences_show_their_own_frames() {
        let rate = Rate::FPS_24;
        let at = |frames: i64| rate.time_of(frames);
        // Forward from frame 10: clip frame 5 shows nested frame 15.
        assert_eq!(
            nested_time(at(5), at(10), &Speed::ONE, at(48), rate),
            at(15)
        );
        // Half speed: clip frame 5 is nested frame 12.5, so 12 shows.
        let half = Speed::from(kerf_model::Ratio::parse("1/2").unwrap());
        assert_eq!(nested_time(at(5), at(10), &half, at(48), rate), at(12));
        // Reversed over [10, 58): the first frame is 57.
        let back = Speed::from(kerf_model::Ratio::parse("-1").unwrap());
        assert_eq!(nested_time(Time::ZERO, at(10), &back, at(48), rate), at(57));
        assert_eq!(nested_time(at(47), at(10), &back, at(48), rate), at(10));
    }
}
