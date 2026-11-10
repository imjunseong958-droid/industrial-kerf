//! This crate's generators as the compositor draws them, each a
//! [`PictureSource`]: a shape and particles, read from their clip's
//! parameters at the moment drawn. Sizes are in frame heights.

use crate::{Combine, Emitter, Figure, Outline, Shape};
use kerf_draw::source::{PictureSource, Reading, SourceRun};
use kerf_draw::Image;
use kerf_model::{Catalog, Clip, Time, SHAPE_OUTLINE};

/// The generators of this crate.
pub const SOURCES: &[&dyn PictureSource] = &[&ShapeSource, &Particles];

/// `shape`: an outline filled and stroked, with the outlines its
/// `fx shape.outline` lines combine with it.
struct ShapeSource;

impl PictureSource for ShapeSource {
    fn id(&self) -> &'static str {
        "shape"
    }

    fn draw(&self, clip: &Clip, run: &SourceRun) -> Image {
        let (w, h) = run.size;
        let mut image = Image::new(w, h);
        let height = h as f64;
        let read = Reading::generator(clip, "shape", run.at);
        let size = |name: &str| (read.number(name) * height) as f32;
        Shape {
            figure: figure_of(&read, height),
            outlines: outlines_of(clip, height, run.at),
            offset: size("offset"),
            fill: read.color("fill"),
            stroke: read.color("stroke"),
            stroke_width: size("stroke_width"),
            trim: (
                read.number("trim_start") as f32,
                read.number("trim_end") as f32,
                read.number("trim_offset") as f32,
            ),
            taper: (
                read.number("taper_start") as f32,
                read.number("taper_end") as f32,
            ),
        }
        .draw(&mut image, [(w as f64 / 2.0) as f32, (height / 2.0) as f32]);
        image
    }
}

/// `particles`: an emitter's particles, each where its birth puts it now.
struct Particles;

impl PictureSource for Particles {
    fn id(&self) -> &'static str {
        "particles"
    }

    fn draw(&self, clip: &Clip, run: &SourceRun) -> Image {
        let (w, h) = run.size;
        let mut image = Image::new(w, h);
        let height = h as f64;
        let read = Reading::generator(clip, "particles", run.at);
        // Who is alive and where they are comes from the rate and the rest at the start, since
        // a particle's way is worked out from its birth.
        let start = Reading::generator(clip, "particles", Time::ZERO);
        Emitter {
            rate: start.number("rate"),
            life: start.number("life"),
            velocity: start.number("velocity") * height,
            vary: start.number("vary"),
            direction: start.number("direction"),
            spread: start.number("spread"),
            gravity: start.number("gravity") * height,
            size: (read.number("size") * height, read.number("size_end") * height),
            color: (read.color("color"), read.color("color_end")),
            soft: read.number("soft"),
            origin: [w as f64 / 2.0, height / 2.0],
            area: [
                read.number("area_width") * height,
                read.number("area_height") * height,
            ],
            seed: read.number("seed").round() as u64,
        }
        .draw(&mut image, run.at.seconds());
        image
    }
}

/// An outline's numbers as the shape draws them, in pixels of a picture `height` high.
fn figure_of(read: &Reading, height: f64) -> Figure {
    let size = |name: &str| (read.number(name) * height) as f32;
    let (width, tall) = (size("width"), size("height"));
    match read.word("kind").as_str() {
        "ellipse" => Figure::Ellipse {
            width,
            height: tall,
        },
        "polygon" => Figure::Polygon {
            width,
            height: tall,
            sides: read.number("sides").round().max(3.0) as u32,
        },
        "star" => Figure::Star {
            width,
            height: tall,
            points: read.number("sides").round().max(2.0) as u32,
            inner: read.number("inner") as f32,
        },
        "path" => Figure::Path {
            points: kerf_model::parse_points(&read.word("points"))
                .unwrap_or_default()
                .into_iter()
                .map(|point| [(point[0] * height) as f32, (point[1] * height) as f32])
                .collect(),
            closed: read.on("closed"),
            smooth: read.on("smooth"),
        },
        _ => Figure::Rect {
            width,
            height: tall,
            round: size("round"),
        },
    }
}

/// A shape clip's further outlines, in the order its `fx shape.outline` lines give them.
fn outlines_of(clip: &Clip, height: f64, time: Time) -> Vec<Outline> {
    clip.effects
        .iter()
        .filter(|effect| effect.enabled && effect.fx == SHAPE_OUTLINE)
        .map(|effect| {
            let read = Reading {
                params: Some(&effect.params),
                def: Catalog::builtin().fx(SHAPE_OUTLINE),
                time,
            };
            Outline {
                combine: match read.word("mode").as_str() {
                    "subtract" => Combine::Subtract,
                    "intersect" => Combine::Intersect,
                    "difference" => Combine::Difference,
                    _ => Combine::Add,
                },
                figure: figure_of(&read, height),
                at: [
                    (read.number("x") * height) as f32,
                    (read.number("y") * height) as f32,
                ],
                rotation: read.number("rotation") as f32,
            }
        })
        .collect()
}
