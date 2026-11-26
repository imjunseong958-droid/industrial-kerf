//! This crate's generators as the compositor draws them, each a
//! [`PictureSource`]: a shape and particles, read from their clip's
//! parameters at the moment drawn. Sizes are in frame heights.

use crate::{Combine, Emitter, Figure, Outline, Paint, Shape};
use kerf_draw::source::{Extent, PictureSource, Reading, SourceRun};
use kerf_draw::{Image, Rect};
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
        let (w, h) = run.picture();
        let mut image = Image::new(w, h);
        let (shape, across) = shape_of(clip, run);
        let [x, y] = run.middle();
        shape.draw(&mut image, [(x + across) as f32, y as f32]);
        image
    }

    fn extent(&self, clip: &Clip, run: &SourceRun) -> Option<Extent> {
        let [middle_x, middle_y] = run.middle();
        let (shape, across) = shape_of(clip, run);
        let (contours, closed) = shape.contours();
        // What `Shape::draw` puts down: a fill inside a closed outline, and a stroke along it.
        let fills =
            shape.fill.shows() && closed && contours.iter().any(|contour| contour.len() >= 3);
        let strokes = shape.stroke[3] > 0.0 && shape.stroke_width > 0.0;
        if !fills && !strokes {
            return None;
        }
        let (mut least, mut most) = ([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]);
        for point in contours.iter().flatten() {
            for axis in 0..2 {
                least[axis] = least[axis].min(f64::from(point[axis]));
                most[axis] = most[axis].max(f64::from(point[axis]));
            }
        }
        if least[0] > most[0] {
            return None;
        }
        // A stroke lies half to either side of the outline, whatever its trim leaves of it.
        let grown = if strokes {
            f64::from(shape.stroke_width) / 2.0
        } else {
            0.0
        };
        let (left, top) = (
            middle_x + across + least[0] - grown,
            middle_y + least[1] - grown,
        );
        let plain = matches!(shape.figure, Figure::Rect { round, .. } if round <= 0.0)
            && shape.outlines.is_empty()
            && shape.offset == 0.0;
        Some(Extent {
            rect: Rect::new(
                left,
                top,
                most[0] - least[0] + 2.0 * grown,
                most[1] - least[1] + 2.0 * grown,
            ),
            filled: fills && shape.fill.covers() && !strokes && plain,
        })
    }
}

/// The shape of a `gen=shape` clip as it is drawn on a picture of the run's size, and how far
/// across from the picture's middle it is drawn ([`aligned`]).
fn shape_of(clip: &Clip, run: &SourceRun) -> (Shape, f64) {
    let height = run.size.1 as f64;
    let read = Reading::generator(clip, "shape", run.at);
    let size = |name: &str| (read.number(name) * height) as f32;
    let shape = Shape {
        figure: figure_of(&read, height),
        outlines: outlines_of(clip, height, run.at),
        offset: size("offset"),
        fill: paint_of(&read, height),
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
    };
    let across = aligned(&shape, &read.word("align"));
    (shape, across)
}

/// How far across its picture a shape is drawn from the middle, in pixels: not at all where it
/// is centered, and with `align=left` or `right` as far as puts that side of it, its stroke
/// included, on the middle. The middle is where `x` places the clip and what it turns and scales
/// about, so the shape's side is at `x` and grows from there, as a title's text does.
fn aligned(shape: &Shape, align: &str) -> f64 {
    let left = match align {
        "left" => true,
        "right" => false,
        _ => return 0.0,
    };
    let (contours, _) = shape.contours();
    let (mut least, mut most) = (f64::INFINITY, f64::NEG_INFINITY);
    for point in contours.iter().flatten() {
        least = least.min(f64::from(point[0]));
        most = most.max(f64::from(point[0]));
    }
    if least > most {
        return 0.0;
    }
    let grown = if shape.stroke[3] > 0.0 && shape.stroke_width > 0.0 {
        f64::from(shape.stroke_width) / 2.0
    } else {
        0.0
    };
    if left {
        grown - least
    } else {
        -(most + grown)
    }
}

/// `particles`: an emitter's particles, each where its birth puts it now.
struct Particles;

impl PictureSource for Particles {
    fn id(&self) -> &'static str {
        "particles"
    }

    fn draw(&self, clip: &Clip, run: &SourceRun) -> Image {
        let (w, h) = run.picture();
        let mut image = Image::new(w, h);
        let height = run.size.1 as f64;
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
            size: (
                read.number("size") * height,
                read.number("size_end") * height,
            ),
            color: (read.color("color"), read.color("color_end")),
            soft: read.number("soft"),
            origin: run.middle(),
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

/// What fills a shape, as it draws it, in pixels of a picture `height` high.
fn paint_of(read: &Reading, height: f64) -> Paint {
    let size = |name: &str| (read.number(name) * height) as f32;
    let from = [size("fill_start_x"), size("fill_start_y")];
    let to = [size("fill_end_x"), size("fill_end_y")];
    let colors = [read.color("fill"), read.color("fill_2")];
    match read.word("fill_kind").as_str() {
        "linear" => Paint::Linear { from, to, colors },
        "radial" => Paint::Radial { from, to, colors },
        "corners" => Paint::Corners([
            colors[0],
            colors[1],
            read.color("fill_3"),
            read.color("fill_4"),
        ]),
        "checker" => Paint::Checker {
            from,
            cell: size("fill_cell"),
            colors,
        },
        "grid" => Paint::Grid {
            from,
            cell: size("fill_cell"),
            line: size("fill_line"),
            colors,
        },
        _ => Paint::Solid(colors[0]),
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
