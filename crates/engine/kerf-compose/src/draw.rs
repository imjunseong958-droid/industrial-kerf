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
use kerf_draw::{mask, wipe, Affine, Blend, Image, Rect, Vec2};
use kerf_fx::transitions::{apply_scaled, Fx};
use kerf_fx::video::{self, catalog::Headroom};
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

mod bevel;
mod builder;
mod effects;
mod focus;
mod geometry;
mod gpu;
mod ids;
mod output;
mod plan;
mod ready;
mod stack;
mod three_d;
mod titles;

use builder::*;
use effects::*;
use geometry::*;
use ids::*;
use plan::*;
use ready::*;
use stack::*;
use titles::*;

pub use ids::ID_COLORS;
pub use three_d::world::{Form, InDepth, World};

type Result<T> = std::result::Result<T, ComposeError>;

/// A program ready to draw.
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
    /// The color of the one clip a matte is of, as an ids pass writes it.
    matte: Option<[u8; 3]>,
    /// An ids pass, or a matte, with each clip laid down whole or not at all at each pixel
    /// ([`ComposeOptions::hard_edges`]).
    hard: bool,
    /// The color each clip that draws has in an ids pass, in the order they are told in.
    ids: Vec<(Id, Rgba)>,
    /// Clear pictures the size of a frame, to draw a clip alone on and lay it down hard, kept to
    /// be drawn on again: a picture that size made anew for each clip took more than drawing it.
    clear: std::sync::Mutex<Vec<Image>>,
}

impl Drawing {
    pub fn new(project: &Project, catalog: &Catalog, options: &ComposeOptions) -> Result<Self> {
        // A matte is the ids pass with one clip's color made white.
        let (sequence, pass) = (
            options.sequence.as_deref(),
            match options.matte {
                Some(_) => Pass::Ids,
                None => options.pass,
            },
        );
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
        let files = files_named(&builder.effects)?;
        let sequences: Vec<SequencePlan> = builder
            .sequences
            .into_iter()
            .map(|plan| {
                plan.expect("a sequence that could not be planned fails the build, hidden or not")
            })
            .collect();
        let ids = told_ids(&sequences);
        let matte = matte_color(options.matte.as_deref(), &ids)?;
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
            matte,
            hard: options.hard_edges && pass == Pass::Ids,
            ids,
            clear: std::sync::Mutex::default(),
        })
    }

    /// A clear picture of `w` by `h`: one kept, or a new one.
    fn clear_picture(&self, w: usize, h: usize) -> Image {
        let mut kept = self
            .clear
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match kept
            .iter()
            .position(|picture| (picture.w, picture.h) == (w, h))
        {
            Some(at) => kept.swap_remove(at),
            None => Image::new(w, h),
        }
    }

    /// Keep `picture`, clear again, for [`Self::clear_picture`]: as many as are drawn on at once.
    fn keep_clear(&self, picture: Image) {
        let mut kept = self
            .clear
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if kept.len() < 16 {
            kept.push(picture);
        }
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
                let [r, g, b, _] = self.id_color(&plan.clip.id);
                flat(picture, [r, g, b]);
            }
            Pass::Depth | Pass::Relative if plan.three_d => {
                let scene = three_d::Scene::at(sequence, at, size);
                three_d::shade_by_depth(&scene, plan, picture, to_output, at - plan.clip.start);
            }
            Pass::Depth | Pass::Relative => flat(picture, [0.0; 3]),
            Pass::Normals if plan.three_d => {
                let scene = three_d::Scene::at(sequence, at, size);
                three_d::shade_by_facing(&scene, plan, picture, to_output, at - plan.clip.start);
            }
            Pass::Normals => flat(picture, [0.0; 3]),
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

    /// See [`Compositor::in_depth`](crate::Compositor::in_depth).
    pub fn in_depth(&self, at: Time) -> World {
        let mut world = World::default();
        let Some(sequence) = self.sequences.first() else {
            return world;
        };
        let size = (sequence.width as usize, sequence.height as usize);
        let scene = three_d::Scene::at(sequence, at, size);
        let canvas = (f64::from(sequence.width), f64::from(sequence.height));
        for track in sequence.tracks.iter().filter(|track| track.shows) {
            for shown in track.shown.iter().filter(|shown| shown.shows(at)) {
                let plan = &shown.plan;
                let time = at - plan.clip.start;
                let opacity = number(&param(&plan.clip, "opacity").value_at(time));
                if matches!(plan.source, Source::Nothing | Source::Adjustment) || opacity <= 0.0 {
                    continue;
                }
                let id = plan.clip.id.to_string();
                if !plan.three_d {
                    world.flat.push(id);
                    continue;
                }
                let Some(held) = held_in_depth(plan, time) else {
                    world.unread.push(id);
                    continue;
                };
                // As the document says it and not as it is drawn: a clip that holds still is
                // drawn on whole pixels of its sequence, under a pixel from here, and what is
                // read here is to be the same at any size.
                let numbers = geometry_numbers(&plan.clip, plan.path.as_ref(), time);
                let own = placed(plan.size, plan.fit, false, canvas, numbers).0;
                let moved = sequence.moved_by_parents(&plan.clip, at);
                // One more of its own `y` is a frame's height down, before its parents move it
                // across the frame and then in depth.
                let fall = (moved.c.abs() <= 1e-9 * moved.d.abs() && moved.d.abs() > 1e-9)
                    .then_some(moved.d)
                    .and_then(|fall| scene.falls(plan, fall));
                let to_output = moved.then_apply(&own);
                world.clips.push(three_d::world::of(
                    &scene, plan, &to_output, time, held, fall,
                ));
            }
        }
        world
    }

    /// See [`Compositor::lands`](crate::Compositor::lands).
    pub fn lands(&self, id: &str, at: Time) -> std::result::Result<crate::Landing, String> {
        let sequence = self.sequences.first().ok_or("there is no sequence")?;
        let plan = sequence
            .tracks
            .iter()
            .flat_map(|track| &track.shown)
            .map(|shown| &shown.plan)
            .find(|plan| plan.clip.id.as_str() == id)
            .ok_or("it is no clip of this sequence that draws")?;
        let time = at - plan.clip.start;
        if time.is_negative() || time >= plan.clip.duration {
            return Err("it does not show then".to_owned());
        }
        let Source::Media { picture } = plan.source else {
            return Err("it plays no footage".to_owned());
        };
        if plan.three_d {
            return Err("it is in depth, where a camera puts it on the frame".to_owned());
        }
        let frame = self
            .pictures
            .frame_shown(picture, time)
            .ok_or("it plays no footage")?;
        // The placing a frame is drawn by, from shares of the picture to shares of the canvas.
        let (wide, tall) = (f64::from(plan.size.0), f64::from(plan.size.1));
        let canvas = (f64::from(sequence.width), f64::from(sequence.height));
        let placed = Affine::scale(1.0 / canvas.0.max(1.0), 1.0 / canvas.1.max(1.0))
            .then_apply(&self.geometry(sequence, plan, time))
            .then_apply(&Affine::scale(wide, tall));
        let [left, right, top, bottom] = ["crop_l", "crop_r", "crop_t", "crop_b"]
            .map(|name| number(&param(&plan.clip, name).value_at(time)));
        Ok(crate::Landing {
            frame,
            kept: [left, top, 1.0 - right, 1.0 - bottom],
            opacity: number(&param(&plan.clip, "opacity").value_at(time)),
            masked: !plan.clip.masks.is_empty(),
            effects: plan
                .effects
                .iter()
                .map(|effect| effect.fx.to_string())
                .collect(),
            to_frame: [placed.a, placed.b, placed.c, placed.d, placed.e, placed.f],
        })
    }
}

/// Where in its own pixels the picture of a clip in depth is at clip time `time`, for
/// [`Drawing::in_depth`], and whether it fills that: none for a mesh, which has no picture of its
/// own, and nothing where its numbers cannot say.
fn held_in_depth(plan: &ClipPlan, time: Time) -> Option<Option<(Rect, bool)>> {
    // Where in its own pixels its picture is, where its numbers say: all of what
    // its crop leaves of a picture that is opaque, and a shape's own outline.
    let whole = (f64::from(plan.size.0), f64::from(plan.size.1));
    let crop = crop_rect(
        plan,
        time,
        whole,
        1.0,
        (plan.size.0 as usize, plan.size.1 as usize),
    )
    .unwrap_or_else(|| Rect::new(0.0, 0.0, whole.0, whole.1));
    match &plan.source {
        _ if !plan.clip.masks.is_empty() => None,
        Source::Mesh => Some(None),
        Source::Solid(_) => Some(Some((crop, true))),
        Source::Media { .. } if !plan.alpha => Some(Some((crop, true))),
        // Its picture reaches past its frame as far as its outline goes, which its
        // crop alone cuts ([`Drawing::reach`]).
        Source::Drawn(source) => {
            let size = (plan.size.0 as usize, plan.size.1 as usize);
            let frame = Rect::new(0.0, 0.0, whole.0, whole.1);
            source
                .extent(&plan.clip, &SourceRun::new(time, size))
                .and_then(|extent| {
                    let drawn = extent.rect;
                    let within = crop_past(plan, time, frame, drawn).unwrap_or(drawn);
                    let some = within.w > 0.0 && within.h > 0.0;
                    some.then_some(Some((within, extent.filled)))
                })
        }
        _ => None,
    }
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

fn number(value: &Value) -> f64 {
    value.as_number().unwrap_or(0.0)
}

/// How far a transition has got at `time`, from 0 to 1.
fn progress(time: Time, start: Time, end: Time) -> f64 {
    let span = (end - start).flicks().max(1) as f64;
    ((time - start).flicks() as f64 / span).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The table the editor's overlay of a parent's motion is to be held to, so what it draws
    /// over the picture moves as the picture does.
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
                in_depth: true,
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

    /// The table the editor's overlays are to be held to, so they land on what they are drawn
    /// over. Each row is a camera, a parent, a clip, its picture's size and whether it is fitted,
    /// and the canvas; then a point of the picture, in fractions of it, and where on the canvas it
    /// lands, or that no pixel shows it. A row with a path names the clip's anchor, which the path
    /// says the place of. The parent is a null, so its `z`, `rotate_x`, and `rotate_y` are the
    /// clip's with its own: the last rows, the first two of them landing where a clip with those
    /// numbers itself does, further up.
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
        let table: [Row; 30] = [
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
            ("", "z=1", "three_d=true", (1920, 1080), false, (1920, 1080), (0.0, 0.0), Some((404.425304296, 227.489233666))),
            ("", "rotate_y=60", "three_d=true", (1920, 1080), false, (1920, 1080), (1.0, 1.0), Some((2051.824322445, 1768.30236275))),
            ("", "x=0.1 y=-0.05 z=0.3 rotate_x=-20 rotate_y=35", "three_d=true z=0.2 rotate_y=15 x=0.2 scale=0.5", (1920, 1080), false, (1920, 1080), (0.25, 0.75), Some((1304.251220744, 671.403611257))),
            ("pan=10 tilt=-5", "x=-0.1 rotation=20 scale=0.8 z=0.5 rotate_y=-40", "three_d=true z=-0.1 rotate_x=25 y=0.15 scale=0.6", (1920, 1080), false, (1920, 1080), (0.9, 0.2), Some((806.73633729, 481.710745385))),
            ("", "anchor_x=0.25 rotation=15 rotate_y=50 z=0.2", "three_d=true x=0.3", (1920, 1080), false, (1920, 1080), (0.5, 0.5), Some((1395.719634219, 568.948088124))),
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
}
