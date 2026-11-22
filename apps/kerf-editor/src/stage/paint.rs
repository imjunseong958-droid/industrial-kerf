//! Painting over the picture what the chosen clip has and what a mask tool
//! draws: each mask's outline, its name on a tag and the chosen one's
//! handles; the motion path's curve, its square points and a round one
//! where the clip is now; the outline being dragged out or the pen's
//! corners. Focus Region draws its box with the same stroke ([`stroke_over`]).

use super::overlay::{MotionPath, Projective, Pt};
use super::picture::to_point;
use super::shown::{share_to_window, Drawn, Shown, REACH};
use crate::theme::{self, over_picture};
use gpui::{
    point, px, quad, size, App, BorderStyle, Bounds, Corners, FontWeight, PathBuilder, Pixels,
    Point, SharedString, TextRun, Window,
};

/// Points of a handle's side.
const HANDLE: f32 = 7.0;

/// `points` mapped by `map` into the window, in the runs that are in
/// sight: one run, `closed`, when all of a closed outline is.
fn runs_in_sight(points: &[Pt], closed: bool, map: Projective) -> (Vec<Vec<Point<Pixels>>>, bool) {
    let mut runs: Vec<Vec<Point<Pixels>>> = Vec::new();
    let mut going = false;
    for at in points {
        if !map.in_sight(*at) {
            going = false;
            continue;
        }
        let shown = to_point(map.apply(*at));
        match runs.last_mut() {
            Some(run) if going => run.push(shown),
            _ => runs.push(vec![shown]),
        }
        going = true;
    }
    let whole = runs.len() == 1 && runs[0].len() == points.len();
    (runs, closed && whole)
}

/// The chosen clip's masks (outlines, a name on each, the chosen one's
/// handles) and its motion path (the curve, square points, and a round one
/// where the clip is now).
pub(super) fn paint_overlays(
    window: &mut Window,
    cx: &mut App,
    picture: Bounds<Pixels>,
    shown: &Shown,
    chosen_mask: Option<&str>,
    chosen_point: Option<usize>,
) {
    // In depth, what is on or behind the camera breaks an outline.
    if let Some(map) = shown.to_window(picture) {
        for mask in &shown.masks {
            let chosen = chosen_mask == Some(mask.name.as_str());
            let (line, closed) = mask.outline.line();
            let (runs, closed) = runs_in_sight(&line, closed, map);
            for run in &runs {
                stroke_over(window, run, closed, chosen, mask.path_only());
            }
            if let Some(anchor) = top_left(&runs.concat()) {
                paint_tag(window, cx, anchor, &mask.name, chosen, picture);
            }
            if chosen {
                for handle in mask.outline.handles() {
                    if map.in_sight(handle) {
                        paint_handle(window, to_point(map.apply(handle)), false);
                    }
                }
            }
        }
    }
    let Some(points) = &shown.path else {
        return;
    };
    let marker = |window: &mut Window, center: Point<Pixels>| {
        let radius = px(HANDLE / 2.0);
        window.paint_quad(quad(
            Bounds::new(
                point(center.x - radius, center.y - radius),
                size(radius * 2.0, radius * 2.0),
            ),
            Corners::all(radius),
            over_picture::line(),
            px(1.0),
            theme::accent(),
            BorderStyle::Solid,
        ));
    };
    // A path no one map shows: the curve where the clip's anchor goes, its
    // points where the anchor is at each.
    if let Some(sampled) = &shown.sampled {
        let mut runs: Vec<Vec<Point<Pixels>>> = Vec::new();
        let mut going = false;
        for place in &sampled.places {
            match place {
                Some(share) => {
                    let at = to_point(share_to_window(picture, *share));
                    match runs.last_mut() {
                        Some(run) if going => run.push(at),
                        _ => runs.push(vec![at]),
                    }
                    going = true;
                }
                None => going = false,
            }
        }
        for run in &runs {
            stroke_over(window, run, false, true, false);
        }
        if let Some(now) = sampled.now {
            marker(window, to_point(share_to_window(picture, now)));
        }
        for (index, handle) in sampled.handles.iter().enumerate() {
            if let Some(share) = handle {
                paint_handle(
                    window,
                    to_point(share_to_window(picture, *share)),
                    chosen_point == Some(index),
                );
            }
        }
        return;
    }
    let Some(map) = shown.path_to_window(picture) else {
        return;
    };
    let path = MotionPath::new(points.clone(), shown.aspect());
    let samples = path.samples();
    // All of it on or behind the camera is no curve to show.
    if !samples.iter().any(|at| map.in_sight(*at)) {
        return;
    }
    let (runs, _) = runs_in_sight(&samples, false, map);
    for run in &runs {
        stroke_over(window, run, false, true, false);
    }
    // The points are square, as a mask's handles are; the chosen one white.
    for (index, at) in points.iter().enumerate() {
        if map.in_sight(*at) {
            paint_handle(
                window,
                to_point(map.apply(*at)),
                chosen_point == Some(index),
            );
        }
    }
    let on = path.at(shown.along);
    if map.in_sight((on.x, on.y)) {
        marker(window, to_point(map.apply((on.x, on.y))));
    }
}

/// What a mask tool has drawn so far: a rectangle or an ellipse dragged
/// out, or the pen's corners with a dashed side to the pointer, its first
/// corner white when a click there would close the outline.
pub(super) fn paint_drawn(
    window: &mut Window,
    picture: Bounds<Pixels>,
    shown: &Shown,
    drawn: &Drawn,
) {
    let Some(map) = shown.to_window(picture) else {
        return;
    };
    if let Some(outline) = &drawn.outline {
        let (line, closed) = outline.line();
        let (runs, closed) = runs_in_sight(&line, closed, map);
        for run in &runs {
            stroke_over(window, run, closed, true, false);
        }
    }
    let corners: Vec<Pt> = drawn
        .pen
        .iter()
        .filter(|at| map.in_sight(**at))
        .map(|at| map.apply(*at))
        .collect();
    let Some((&first, &last)) = corners.first().zip(corners.last()) else {
        return;
    };
    let line: Vec<Point<Pixels>> = corners.iter().map(|at| to_point(*at)).collect();
    stroke_over(window, &line, false, true, false);
    if let Some(pointer) = drawn.pointer {
        stroke_over(
            window,
            &[to_point(last), to_point(pointer)],
            false,
            false,
            true,
        );
    }
    let closes = corners.len() >= 3
        && drawn.pointer.is_some_and(|pointer| {
            (first.0 - pointer.0).abs() <= REACH && (first.1 - pointer.1).abs() <= REACH
        });
    for (index, corner) in corners.iter().enumerate() {
        paint_handle(window, to_point(*corner), index == 0 && closes);
    }
}

fn top_left(line: &[Point<Pixels>]) -> Option<Point<Pixels>> {
    let first = *line.first()?;
    Some(line.iter().fold(first, |least, at| {
        point(least.x.min(at.x), least.y.min(at.y))
    }))
}

/// An outline over the picture: a dark rule, then a white line on it, which
/// reads on any picture; dashed for a path only. Focus Region's box is
/// drawn with it too (`focus::FocusRegion`).
pub fn stroke_over(
    window: &mut Window,
    line: &[Point<Pixels>],
    closed: bool,
    chosen: bool,
    dashed: bool,
) {
    if line.len() < 2 {
        return;
    }
    let build = |width: f32, dash: bool| {
        let mut builder = PathBuilder::stroke(px(width));
        if dash {
            builder = builder.dash_array(&[px(4.0), px(3.0)]);
        }
        builder.move_to(line[0]);
        for at in &line[1..] {
            builder.line_to(*at);
        }
        if closed {
            builder.line_to(line[0]);
        }
        builder.build().ok()
    };
    if let Some(path) = build(3.0, false) {
        window.paint_path(path, over_picture::rule());
    }
    if let Some(path) = build(1.5, dashed) {
        let ink = if chosen {
            over_picture::line()
        } else {
            over_picture::line_idle()
        };
        window.paint_path(path, ink);
    }
}

/// A mask's name on a tag just above `anchor` (its outline's top left), kept
/// inside the picture: 8 points under its top edge and 4 in from its sides.
fn paint_tag(
    window: &mut Window,
    cx: &mut App,
    anchor: Point<Pixels>,
    name: &str,
    chosen: bool,
    picture: Bounds<Pixels>,
) {
    let words: SharedString = name.to_owned().into();
    let run = TextRun {
        len: words.len(),
        font: theme::ui_font(FontWeight::SEMIBOLD),
        color: over_picture::tag_ink().into(),
        background_color: None,
        underline: None,
        strikethrough: None,
    };
    let line = window
        .text_system()
        .shape_line(words, px(11.0), &[run], None);
    let (across, down) = (f32::from(line.width) + 10.0, 16.0);
    let (left, top) = (f32::from(picture.origin.x), f32::from(picture.origin.y));
    let (right, bottom) = (
        left + f32::from(picture.size.width),
        top + f32::from(picture.size.height),
    );
    // Clear of the top-left handle, by 4 points.
    let x = (f32::from(anchor.x) - HANDLE / 2.0)
        .clamp(left + 4.0, (left + 4.0).max(right - 4.0 - across));
    let y = (f32::from(anchor.y) - HANDLE / 2.0 - 4.0 - down)
        .clamp(top + 8.0, (top + 8.0).max(bottom - 4.0 - down));
    let tag = Bounds::new(point(px(x), px(y)), size(px(across), px(down)));
    window.paint_quad(quad(
        tag,
        Corners::all(px(4.0)),
        if chosen {
            theme::accent()
        } else {
            over_picture::tag()
        },
        px(0.0),
        gpui::transparent_black(),
        BorderStyle::Solid,
    ));
    if let Err(error) = line.paint(point(px(x + 5.0), px(y)), px(down), window, cx) {
        log::warn!("a mask's name did not draw: {error}");
    }
}

/// A handle: the accent with a white edge, or white with the accent's when
/// `inverted` (a chosen point).
fn paint_handle(window: &mut Window, center: Point<Pixels>, inverted: bool) {
    let half = px(HANDLE / 2.0);
    let (fill, edge) = if inverted {
        (over_picture::line(), theme::accent())
    } else {
        (theme::accent(), over_picture::line())
    };
    window.paint_quad(quad(
        Bounds::new(
            point(center.x - half, center.y - half),
            size(px(HANDLE), px(HANDLE)),
        ),
        Corners::default(),
        fill,
        px(1.0),
        edge,
        BorderStyle::Solid,
    ));
}
