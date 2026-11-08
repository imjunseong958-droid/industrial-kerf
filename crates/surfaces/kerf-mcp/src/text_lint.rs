//! Text a viewer cannot read: titles and captions outside title safe, text
//! boxes drawn over each other, captions that go by too fast, flash
//! up too briefly, or follow each other too closely, and, from rendered
//! frames, text too faint against what is behind it.
//!
//! Boxes are laid out by `kerf-text` exactly as the compositor lays them
//! out (`kerf-compose` `draw.rs`, through `kerf-draw` `text::draw`): a
//! caption is Inter at 4.5% of the frame height (at least 12 px), centred,
//! its bottom at 95% of the frame; a title is its font at `size` frame
//! heights at its axes, centred on the frame, then cropped, scaled, rotated, and moved
//! like any picture. A box is the layout's line boxes, not its ink.

use crate::qc::{Finding, Rect};
use kerf_lang::Clock;
use kerf_model::{
    Catalog, Clip, Content, Domain, Param, Project, Rgba, Sequence, Time, TrackKind, Value,
};
use kerf_text::fonts::Axis;
use kerf_text::{layout, Align, ParagraphStyle, TextStyle};

/// The most characters a viewer reads in a second (Netflix's limit for
/// adult programmes).
pub const READING_SPEED: f64 = 17.0;
/// The least time a caption may stay up, in sixths of a second (5/6 s).
pub const SHORT_CAPTION_SIXTHS: i64 = 5;
/// The least gap between two captions, in frames; back to back is fine.
pub const CAPTION_GAP_FRAMES: i64 = 2;
/// The part of the frame titles and captions stay inside.
pub const TITLE_SAFE: f64 = 0.90;

/// How much lighter than what is behind it text has to be, or darker, as a ratio of relative
/// luminance: WCAG's least for text this large.
pub const LEAST_CONTRAST: f64 = 3.0;
/// How much of its ink may be faint before a text is: a little of it over a highlight still reads.
const FAINT_SHARE: f64 = 0.2;
/// A channel apart, in code values of 255, for a pixel to be said to be changed by the text.
const INK: u8 = 8;
/// How much light a pixel of a text drawn white on nothing has to give to be one of its glyphs.
const GLYPH: f64 = 0.25;
/// How many moments of a clip its contrast is read at, a little in from each end and in its middle.
const CONTRAST_MOMENTS: usize = 3;
/// How far past its box a text's outline or shadow is looked for, as a share of the frame's height.
const CONTRAST_MARGIN: f64 = 0.01;
/// The height frames are drawn at to read text against what is behind it. Half of a 1080p frame draws a
/// caption's strokes a few pixels wide.
const CONTRAST_HEIGHT: f64 = 540.0;
/// The most frames a contrast check draws at a time; three for each moment of each clip it reads.
const MOST_CONTRAST_FRAMES: usize = 360;

/// Pixels a box may reach past title safe, or into another box, without a
/// finding: what rounding to whole pixels moves.
const TOLERANCE: f64 = 0.5;
/// The most frames sampled when a title's placement animates.
const MAX_SAMPLES: i64 = 240;
/// What generic font names stand for, as the compositor resolves them.
const GENERIC: &[(&str, &str)] = &[
    ("sans-serif", "Inter"),
    ("serif", "Noto Serif"),
    ("monospace", "JetBrains Mono"),
];
/// The placement properties of a picture, as the compositor reads them.
const GEOMETRY: [&str; 12] = [
    "crop_l", "crop_r", "crop_t", "crop_b", "scale", "scale_x", "scale_y", "rotation", "anchor_x",
    "anchor_y", "x", "y",
];

#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    Caption,
    Title,
}

/// A clip that puts text on the picture.
struct Shown<'a> {
    clip: &'a Clip,
    kind: Kind,
    text: &'a str,
}

impl Shown<'_> {
    fn describe(&self) -> String {
        let kind = match self.kind {
            Kind::Caption => "caption",
            Kind::Title => "title",
        };
        match &self.clip.name {
            Some(name) if !name.trim().is_empty() => {
                format!("{kind} \"{}\" (`{}`)", name.trim(), self.clip.id)
            }
            _ => format!("{kind} `{}`", self.clip.id),
        }
    }

    /// Whether the box moves or changes size while the clip plays.
    fn animated(&self) -> bool {
        self.kind == Kind::Title
            && (GEOMETRY
                .iter()
                .any(|name| matches!(self.clip.props.get(*name), Some(Param::Animated(_))))
                || axes_keyed(self.clip))
    }

    /// The box at sequence time `at`, in pixels of the frame:
    /// `[left, top, right, bottom]`.
    fn bounds(&self, sequence: &Sequence, at: Time) -> Option<[f64; 4]> {
        match self.kind {
            Kind::Caption => caption_box(sequence, self.text),
            Kind::Title => title_box(sequence, self.clip, at - self.clip.start),
        }
    }
}

/// The clips of `sequence` whose text shows, in track order: captions on
/// enabled subtitle tracks and titles on enabled picture tracks.
fn shown(sequence: &Sequence) -> Vec<Shown<'_>> {
    let mut shown = Vec::new();
    for track in sequence.tracks.iter().filter(|track| track.enabled) {
        for clip in track.clips.iter().filter(|clip| clip.enabled) {
            let item = match (&track.kind, &clip.content) {
                (TrackKind::Subtitle, Content::Caption { text }) => Shown {
                    clip,
                    kind: Kind::Caption,
                    text,
                },
                (TrackKind::Video, Content::Generator { fx, params, .. })
                    if fx.as_str() == "title" =>
                {
                    let Some(Param::Static(Value::Text(text))) = params.get("text") else {
                        continue;
                    };
                    if matches!(clip.props.get("opacity"), Some(Param::Static(Value::Number(o))) if *o <= 0.0)
                    {
                        continue;
                    }
                    Shown {
                        clip,
                        kind: Kind::Title,
                        text,
                    }
                }
                _ => continue,
            };
            if !item.text.trim().is_empty() {
                shown.push(item);
            }
        }
    }
    shown
}

/// A title's font axes as they stand, by tag and name.
fn title_axes(clip: &Clip) -> impl Iterator<Item = ([u8; 4], &Param)> {
    let params = match &clip.content {
        Content::Generator { params, .. } => Some(params),
        _ => None,
    };
    kerf_model::TITLE_AXES.iter().filter_map(move |tag| {
        let name = std::str::from_utf8(tag).ok()?;
        Some((*tag, params?.get(name)?))
    })
}

fn axes_keyed(clip: &Clip) -> bool {
    title_axes(clip).any(|(_, param)| matches!(param, Param::Animated(_)))
}

/// The family `font` stands for, as the compositor resolves it.
pub(crate) fn family_of(font: &str) -> &str {
    GENERIC
        .iter()
        .find(|(generic, _)| generic.eq_ignore_ascii_case(font.trim()))
        .map_or(font.trim(), |(_, family)| family)
}

/// The frame as the compositor draws it: even sizes.
fn canvas(sequence: &Sequence) -> (f64, f64) {
    let even = |value: u32| f64::from((value.max(2) / 2) * 2);
    (even(sequence.width), even(sequence.height))
}

/// The line boxes of `text` laid out like the compositor lays it out, with
/// `place` of their height above `at`'s y and its x where `align` puts it.
fn laid_box(
    text: &str,
    (font, axes): (&str, Vec<Axis>),
    size: f64,
    align: Align,
    at: [f64; 2],
    place: f64,
) -> Option<[f64; 4]> {
    if text.trim().is_empty() || size <= 0.0 {
        return None;
    }
    let family = family_of(font);
    let style = TextStyle {
        family: if family.is_empty() { "Inter" } else { family }.to_owned(),
        size: size as f32,
        axes,
        ..TextStyle::default()
    };
    let paragraph = ParagraphStyle {
        align,
        ..ParagraphStyle::default()
    };
    let laid = layout(text, &style, &paragraph);
    let [left, top, right, bottom] = laid.bounds.map(f64::from);
    if right <= left || bottom <= top {
        return None;
    }
    let baseline = at[1] - (top + (bottom - top) * place);
    Some([
        at[0] + left,
        baseline + top,
        at[0] + right,
        baseline + bottom,
    ])
}

/// Where the compositor draws a caption: Inter at 4.5% of the frame
/// height, centred, its bottom at 95% of the frame.
fn caption_box(sequence: &Sequence, text: &str) -> Option<[f64; 4]> {
    let (w, h) = canvas(sequence);
    let size = (f64::from(sequence.height) * 0.045).max(12.0);
    laid_box(
        text,
        ("sans-serif", Vec::new()),
        size,
        Align::Center,
        [w / 2.0, h * 0.95],
        1.0,
    )
}

/// A property of `clip` at clip time `at`, or its default.
fn number(clip: &Clip, name: &str, at: Time) -> f64 {
    let value = match clip.props.get(name) {
        Some(param) => param.value_at(at),
        None => Catalog::builtin()
            .prop(Domain::Video, name)
            .map_or(Value::Number(0.0), |def| def.default.clone()),
    };
    value.as_number().unwrap_or(0.0)
}

/// Where the compositor draws a title at clip time `at`: laid out on a
/// frame-sized picture, centred, then placed like any picture.
fn title_box(sequence: &Sequence, clip: &Clip, at: Time) -> Option<[f64; 4]> {
    let Content::Generator { params, .. } = &clip.content else {
        return None;
    };
    let def = Catalog::builtin().fx("title")?;
    let get = |name: &str| -> Option<Value> {
        match params.get(name) {
            None => def.find(name).map(|param| param.default.clone()),
            Some(Param::Static(value)) => Some(value.clone()),
            // The compositor refuses animated titles; validation says so.
            Some(Param::Animated(_)) => None,
        }
    };
    let text = match get("text")? {
        Value::Text(text) => text,
        _ => return None,
    };
    let font = match get("font")? {
        Value::Text(font) if !font.trim().is_empty() => font,
        _ => "sans-serif".to_owned(),
    };
    let size = get("size")?.as_number().unwrap_or(0.08);
    let align = match get("align")? {
        Value::Enum(align) if align == "left" => Align::Left,
        Value::Enum(align) if align == "right" => Align::Right,
        _ => Align::Center,
    };
    // As the compositor sets them: at 0, an axis is left as the font has it. Each moment is measured
    // at what it is then, so a keyed axis is measured at the widest it reaches.
    let axes = title_axes(clip)
        .map(|(tag, param)| {
            let value = param.value_at(at).as_number().unwrap_or(0.0);
            (tag, value as f32)
        })
        .filter(|(_, value)| *value != 0.0)
        .collect();
    let (cw, ch) = canvas(sequence);
    let laid = laid_box(
        &text,
        (&font, axes),
        (size * ch).max(1.0),
        align,
        [cw / 2.0, ch / 2.0],
        0.5,
    )?;
    let [l, r, t, b, s, sx, sy, rotation, ax, ay, x, y] =
        GEOMETRY.map(|name| number(clip, name, at));
    // What the crop leaves of the title's picture.
    let (width, height) = (cw * (1.0 - l - r), ch * (1.0 - t - b));
    if width <= 0.0 || height <= 0.0 {
        return None;
    }
    let kept = [
        laid[0].max(cw * l),
        laid[1].max(ch * t),
        laid[2].min(cw * (1.0 - r)),
        laid[3].min(ch * (1.0 - b)),
    ];
    if kept[2] <= kept[0] || kept[3] <= kept[1] {
        return None;
    }
    // position + R(rotation) · scale · (p − anchor), as `place` in the
    // compositor; the title's picture is not fitted, so it keeps its size.
    let position = (
        cw / 2.0 + x * cw + ax * width,
        ch / 2.0 + y * ch + ay * height,
    );
    let anchor = (cw * l + width * (0.5 + ax), ch * t + height * (0.5 + ay));
    let (sin, cos) = rotation.to_radians().sin_cos();
    let (kx, ky) = (s * sx, s * sy);
    let mut placed = [f64::MAX, f64::MAX, f64::MIN, f64::MIN];
    for (px, py) in [
        (kept[0], kept[1]),
        (kept[2], kept[1]),
        (kept[0], kept[3]),
        (kept[2], kept[3]),
    ] {
        let (dx, dy) = ((px - anchor.0) * kx, (py - anchor.1) * ky);
        let (qx, qy) = (
            position.0 + cos * dx - sin * dy,
            position.1 + sin * dx + cos * dy,
        );
        placed = [
            placed[0].min(qx),
            placed[1].min(qy),
            placed[2].max(qx),
            placed[3].max(qy),
        ];
    }
    (placed[2] > placed[0] && placed[3] > placed[1]).then_some(placed)
}

/// Sequence times to look at the text of `items` in `from`–`to`: the first
/// frame, or every frame (up to [`MAX_SAMPLES`], evenly spread) when a box
/// moves.
fn samples(sequence: &Sequence, items: &[&Shown], from: Time, to: Time) -> Vec<Time> {
    if !items.iter().any(|item| item.animated()) {
        return vec![from];
    }
    let frame = sequence.rate.frame_duration();
    let frames = ((to - from).flicks() / frame.flicks().max(1)).max(1);
    let step = (frames + MAX_SAMPLES - 1) / MAX_SAMPLES;
    (0..frames)
        .step_by(step.max(1) as usize)
        .map(|n| from + Time::from_flicks(frame.flicks() * n))
        .collect()
}

/// `box` in fractions of the frame, clipped to it.
fn normalised(sequence: &Sequence, [left, top, right, bottom]: [f64; 4]) -> Rect {
    let (w, h) = canvas(sequence);
    let (x0, y0) = ((left / w).clamp(0.0, 1.0), (top / h).clamp(0.0, 1.0));
    let (x1, y1) = ((right / w).clamp(0.0, 1.0), (bottom / h).clamp(0.0, 1.0));
    Rect {
        x: x0,
        y: y0,
        w: (x1 - x0).max(0.0),
        h: (y1 - y0).max(0.0),
    }
}

/// A frame as it draws: width, height, and RGBA bytes.
pub(crate) type Drawn = (u32, u32, Vec<u8>);

/// How much light a pixel gives, as WCAG reckons it, from 0 to 1.
fn luminance(pixel: &[u8]) -> f64 {
    fn light(code: u8) -> f64 {
        let code = f64::from(code) / 255.0;
        if code <= 0.04045 {
            code / 12.92
        } else {
            ((code + 0.055) / 1.055).powf(2.4)
        }
    }
    let alpha = f64::from(pixel[3]) / 255.0;
    (0.2126 * light(pixel[0]) + 0.7152 * light(pixel[1]) + 0.0722 * light(pixel[2])) * alpha
}

/// What a viewer gets of a text at a moment, from the frame `with` it and `without` it, and its `glyphs`
/// alone, in `area`.
#[derive(Debug, PartialEq)]
struct Reading {
    /// The share of its ink that is faint, and how strong it is where it is faint.
    faint: f64,
    ratio: f64,
    /// Whether anything of it changes the picture.
    shows: bool,
}

/// Reads a text by the picture, as a viewer does. Its ink is where its glyphs are and whatever else it
/// changes, each pixel set against what lies behind it there. That is read a patch at a time, about half
/// a line across, by the strongest of it there, so that fill the colour of the background still reads
/// if it has an outline to set it off, and a text going from a dark part of the picture to a light
/// one is seen to. `None` when none of it is in the area.
fn reading(
    with: &Drawn,
    without: &Drawn,
    glyphs: &Drawn,
    [left, top, right, bottom]: [usize; 4],
) -> Option<Reading> {
    let width = with.0 as usize;
    let patch = ((bottom - top) / 2).max(6);
    let mut patches: std::collections::BTreeMap<(usize, usize), Vec<f64>> = Default::default();
    let (mut shows, mut any_glyph) = (false, false);
    for y in top..bottom {
        for x in left..right {
            let at = (y * width + x) * 4;
            let (here, behind) = (&with.2[at..at + 4], &without.2[at..at + 4]);
            let changed = here.iter().zip(behind).any(|(a, b)| a.abs_diff(*b) >= INK);
            let glyph = luminance(&glyphs.2[at..at + 4]) > GLYPH;
            shows |= changed;
            any_glyph |= glyph;
            if changed || glyph {
                let (a, b) = (luminance(here), luminance(behind));
                let ratio = (a.max(b) + 0.05) / (a.min(b) + 0.05);
                patches
                    .entry((x / patch, y / patch))
                    .or_default()
                    .push(ratio);
            }
        }
    }
    if !any_glyph {
        return None;
    }
    let total: usize = patches.values().map(Vec::len).sum();
    let (mut faint, mut strengths) = (0, Vec::new());
    for ratios in patches.values_mut() {
        ratios.sort_by(f64::total_cmp);
        let strongest = ratios[(ratios.len() * 9 / 10).min(ratios.len() - 1)];
        if strongest < LEAST_CONTRAST {
            faint += ratios.len();
            strengths.push(strongest);
        }
    }
    strengths.sort_by(f64::total_cmp);
    Some(Reading {
        faint: faint as f64 / total.max(1) as f64,
        ratio: strengths
            .get(strengths.len() / 2)
            .copied()
            .unwrap_or(f64::INFINITY),
        shows,
    })
}

/// `project` with the clip `id` of `sequence` left out.
fn without_clip(project: &Project, sequence: &str, id: &kerf_model::Id) -> Project {
    let mut project = project.clone();
    project
        .sequences
        .iter_mut()
        .filter(|found| found.id.as_str() == sequence)
        .flat_map(|found| &mut found.tracks)
        .flat_map(|track| &mut track.clips)
        .filter(|clip| clip.id == *id)
        .for_each(|clip| clip.enabled = false);
    project
}

/// `project` with only the clip `id` of `sequence`, white and whole on nothing: where its glyphs are,
/// whatever colour they are.
fn only_glyphs(project: &Project, sequence: &str, id: &kerf_model::Id) -> Project {
    let mut project = project.clone();
    for clip in project
        .sequences
        .iter_mut()
        .filter(|found| found.id.as_str() == sequence)
        .flat_map(|found| &mut found.tracks)
        .flat_map(|track| &mut track.clips)
    {
        if clip.id != *id {
            clip.enabled = false;
            continue;
        }
        clip.props.remove("opacity");
        if let Content::Generator { params, .. } = &mut clip.content {
            params.insert("color".into(), Param::Static(Value::Color(Rgba::WHITE)));
        }
    }
    project
}

/// The moments of `clip`'s `CONTRAST_MOMENTS` to read it at, where it has come in as far as it does: not
/// while it fades in or reveals itself, which is meant to be faint.
fn steady_moments(clip: &Clip) -> Vec<Time> {
    let at = |step: usize, of: usize| {
        clip.start + Time::from_flicks(clip.duration.flicks() * step as i64 / of as i64)
    };
    let level = |moment: Time| {
        let opacity = clip
            .props
            .get("opacity")
            .and_then(|param| param.value_at(moment - clip.start).as_number())
            .unwrap_or(1.0);
        let reveal = match &clip.content {
            Content::Generator { params, .. } => params
                .get("reveal")
                .and_then(|param| param.value_at(moment - clip.start).as_number())
                .unwrap_or(1.0),
            _ => 1.0,
        };
        opacity * reveal
    };
    let candidates: Vec<Time> = (1..=11).map(|step| at(step, 12)).collect();
    let most = candidates
        .iter()
        .map(|moment| level(*moment))
        .fold(0.0, f64::max);
    let steady: Vec<Time> = candidates
        .into_iter()
        .filter(|moment| level(*moment) >= most * 0.98)
        .collect();
    if steady.len() <= CONTRAST_MOMENTS {
        return steady;
    }
    (0..CONTRAST_MOMENTS)
        .map(|place| steady[place * (steady.len() - 1) / (CONTRAST_MOMENTS - 1)])
        .collect()
}

/// [`contrast`] by the compositor, adding to `findings`: how many clips it read and how many there were.
/// `going_on` says how far along it is, from 0 to 1, and whether to keep on, and it reads no more
/// once told not to.
pub(crate) fn contrast_of(
    project: &Project,
    sequence: &Sequence,
    range: (Time, Time),
    mut going_on: impl FnMut(f64) -> bool,
    findings: &mut Vec<Finding>,
) -> (usize, usize) {
    let scale = (CONTRAST_HEIGHT / f64::from(sequence.height.max(1))).min(1.0);
    let id = sequence.id.as_str();
    let mut drawn = 0usize;
    let room = contrast_frames(sequence, range).max(1);
    let draw = |shown: &Project, at: Time| {
        drawn += 1;
        if !going_on((drawn as f64 / room as f64).min(1.0)) {
            return None;
        }
        kerf_engine::still::draw_frame(shown, Some(id), at, scale)
            .ok()
            .map(|frame| (frame.width, frame.height, frame.rgba))
    };
    contrast(
        project,
        sequence,
        range,
        MOST_CONTRAST_FRAMES,
        draw,
        findings,
    )
}

/// A clip's crops, which take what they take off a text on purpose if they move.
const CROPS: [&str; 4] = ["crop_l", "crop_r", "crop_t", "crop_b"];

/// How much of a text's ink a crop that cuts none of it leaves, going by pixels.
const CROP_LEAVES: f64 = 0.99;

/// About how many frames [`contrast_of`] draws, for saying how far along it is.
fn contrast_frames(sequence: &Sequence, (from, to): (Time, Time)) -> usize {
    let of = shown(sequence)
        .iter()
        .filter(|item| item.clip.start < to && item.clip.end() > from)
        .count();
    (of * 3 * CONTRAST_MOMENTS).min(MOST_CONTRAST_FRAMES)
}

/// Whether the frame cuts a text off: its box goes past an edge, in pixels of the frame `glyphs` is,
/// and its glyphs reach that edge. A box goes past an edge for what a line could hold with no glyph
/// there, which title safe says something of and this does not.
fn cut_by_frame(glyphs: &Drawn, bounds: [f64; 4], scale: f64) -> bool {
    let (w, h) = (glyphs.0 as usize, glyphs.1 as usize);
    let ink = |x: usize, y: usize| {
        let at = (y * w + x) * 4;
        f64::from(glyphs.2[at]) / 255.0 >= GLYPH
    };
    let past = [
        bounds[0] * scale < 0.0,
        bounds[1] * scale < 0.0,
        bounds[2] * scale > w as f64,
        bounds[3] * scale > h as f64,
    ];
    (past[0] && (0..h).any(|y| ink(0, y)))
        || (past[1] && (0..w).any(|x| ink(x, 0)))
        || (past[2] && (0..h).any(|y| ink(w - 1, y)))
        || (past[3] && (0..w).any(|x| ink(x, h - 1)))
}

/// `project` with the clip `id` of `sequence` uncropped, for what a crop takes off it.
fn uncropped(mut project: Project, sequence: &str, id: &kerf_model::Id) -> Project {
    project
        .sequences
        .iter_mut()
        .filter(|found| found.id.as_str() == sequence)
        .flat_map(|found| &mut found.tracks)
        .flat_map(|track| &mut track.clips)
        .filter(|clip| clip.id == *id)
        .for_each(|clip| {
            for name in CROPS {
                clip.props.remove(name);
            }
        });
    project
}

/// Whether a crop that does not move takes anything off a clip, which a crop that moves is meant to, as
/// a reveal.
fn cropped_still(clip: &Clip) -> bool {
    let at_all = CROPS.iter().any(|name| {
        clip.props
            .get(*name)
            .and_then(|param| param.value_at(Time::ZERO).as_number())
            .is_some_and(|value| value > 0.0)
    });
    at_all
        && !CROPS
            .iter()
            .any(|name| matches!(clip.props.get(*name), Some(Param::Animated(_))))
}

/// Whether `cropped` has fewer of a text's glyphs than `whole`. By how much ink, not by where, since a crop
/// also moves what it leaves to the middle of the frame; a little either way is its edges falling on
/// other pixels.
fn crop_cuts(whole: &Drawn, cropped: &Drawn) -> bool {
    let ink = |drawn: &Drawn| {
        drawn
            .2
            .as_chunks::<4>().0.iter()
            .filter(|pixel| f64::from(pixel[0]) / 255.0 >= GLYPH)
            .count() as f64
    };
    ink(cropped) < ink(whole) * CROP_LEAVES
}

/// The moment within `(from, to)` where a text that moves goes furthest past an edge of a frame `(w, h)`
/// with some of it left in the frame, which is not at any of the moments already read: when none is,
/// nothing more need be drawn. Its boxes take no frames to work out.
fn furthest_past(
    item: &Shown,
    sequence: &Sequence,
    (from, to): (Time, Time),
    (w, h): (f64, f64),
) -> Option<Time> {
    if !item.animated() || to <= from {
        return None;
    }
    let past = |at: Time| {
        let [left, top, right, bottom] = item.bounds(sequence, at)?;
        if right <= 0.0 || bottom <= 0.0 || left >= w || top >= h {
            return None;
        }
        let by = (-left).max(-top).max(right - w).max(bottom - h);
        (by > 0.0).then_some(by)
    };
    // On the frames the clip has, which what keys it goes by.
    let frame = sequence.rate.frame_duration().flicks().max(1);
    let into = (from - item.clip.start).flicks() / frame + 1;
    let first = item.clip.start + Time::from_flicks(frame * into);
    samples(sequence, &[item], first, to)
        .into_iter()
        .filter_map(|at| Some((at, past(at)?)))
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(at, _)| at)
}

/// Titles and captions of `sequence` in `project`, whose media paths are resolved, within a range,
/// too faint against what is behind them or cut off by the frame, read from the picture: `draw` gives
/// the frame of a project at a moment, or `None` if it cannot be drawn. It draws at most `most` frames, spread over what
/// there is, and returns how many clips it read and how many there were. Frames take time, so this
/// is for `look` to be asked for, and for the full check, and is not part of the fast one.
pub(crate) fn contrast(
    project: &Project,
    sequence: &Sequence,
    (from, to): (Time, Time),
    most: usize,
    mut draw: impl FnMut(&Project, Time) -> Option<Drawn>,
    findings: &mut Vec<Finding>,
) -> (usize, usize) {
    let shown: Vec<Shown> = shown(sequence)
        .into_iter()
        .filter(|item| item.clip.start < to && item.clip.end() > from)
        .collect();
    let clock = Clock::of_sequence(sequence);
    let id = sequence.id.as_str();
    let of = shown.len();
    let room = (most / (3 * CONTRAST_MOMENTS)).max(1);
    let picked: Vec<&Shown> = if of <= room {
        shown.iter().collect()
    } else {
        (0..room).map(|place| &shown[place * of / room]).collect()
    };
    for item in &picked {
        let (behind, alone) = (
            without_clip(project, id, &item.clip.id),
            only_glyphs(project, id, &item.clip.id),
        );
        let mut worst: Option<(Reading, Time)> = None;
        let mut cut: Option<Time> = None;
        let mut cropped: Option<(Drawn, Time)> = None;
        let moments = steady_moments(item.clip);
        for moment in moments.iter().copied() {
            let frames = draw(project, moment)
                .and_then(|with| Some((with, draw(&behind, moment)?, draw(&alone, moment)?)));
            let Some((with, without, glyphs)) = frames else {
                continue;
            };
            let size = (with.0, with.1);
            if size != (without.0, without.1) || size != (glyphs.0, glyphs.1) || with.1 == 0 {
                continue;
            }
            let Some(bounds) = item.bounds(sequence, moment) else {
                continue;
            };
            let scale = f64::from(with.1) / f64::from(sequence.height.max(1));
            if cut.is_none() && cut_by_frame(&glyphs, bounds, scale) {
                cut = Some(moment);
            }
            if cropped.is_none() && cropped_still(item.clip) {
                cropped = Some((glyphs.clone(), moment));
            }
            let margin = CONTRAST_MARGIN * f64::from(sequence.height);
            let place = |value: f64, most: u32| {
                ((value * scale).round().max(0.0) as usize).min(most as usize)
            };
            let area = [
                place(bounds[0] - margin, with.0),
                place(bounds[1] - margin, with.1),
                place(bounds[2] + margin, with.0),
                place(bounds[3] + margin, with.1),
            ];
            if area[0] >= area[2] || area[1] >= area[3] {
                continue;
            }
            let Some(read) = reading(&with, &without, &glyphs, area) else {
                continue;
            };
            if worst
                .as_ref()
                .is_none_or(|(worst, _)| read.faint > worst.faint)
            {
                worst = Some((read, moment));
            }
        }
        // Between the moments read, where a box says so, one more frame of glyphs tells.
        if let (None, Some(first), Some(last)) = (cut, moments.first(), moments.last()) {
            let frame = (f64::from(sequence.width), f64::from(sequence.height));
            if let Some(moment) = furthest_past(item, sequence, (*first, *last), frame) {
                if let (Some(glyphs), Some(bounds)) =
                    (draw(&alone, moment), item.bounds(sequence, moment))
                {
                    let scale = f64::from(glyphs.1) / f64::from(sequence.height.max(1));
                    if cut_by_frame(&glyphs, bounds, scale) {
                        cut = Some(moment);
                    }
                }
            }
        }
        if let Some(when) = cut {
            let place = clock.print_position(when);
            findings.push(
                Finding::warning(
                    "text.clipped",
                    item.clip.id.as_str(),
                    format!(
                        "{} is cut off by the edge of the frame at {place}",
                        item.describe()
                    ),
                )
                .during(sequence, item.clip.start, item.clip.end()),
            );
        }
        // A crop that does not move: one frame of its glyphs whole says what it takes off.
        if let Some((glyphs, when)) = cropped {
            let whole = uncropped(alone.clone(), id, &item.clip.id);
            if draw(&whole, when).is_some_and(|whole| crop_cuts(&whole, &glyphs)) {
                let place = clock.print_position(when);
                findings.push(
                    Finding::warning(
                        "text.clipped",
                        item.clip.id.as_str(),
                        format!("{} is cut off by its crop at {place}", item.describe()),
                    )
                    .during(sequence, item.clip.start, item.clip.end()),
                );
            }
        }
        let Some((read, when)) = worst else {
            continue;
        };
        if read.faint < FAINT_SHARE {
            continue;
        }
        let place = clock.print_position(when);
        let message = if read.shows {
            format!(
                "{} is too faint against what is behind it at {place}: {:.1}:1 over {:.0}% of it, where {LEAST_CONTRAST}:1 is the least that reads; change its color or what is behind it",
                item.describe(),
                read.ratio,
                read.faint * 100.0,
            )
        } else {
            format!(
                "{} does not show at {place}: something covers it, or it is the color of what is behind it",
                item.describe(),
            )
        };
        let ratio = if read.ratio.is_finite() {
            read.ratio
        } else {
            1.0
        };
        findings.push(
            Finding::warning("text.contrast", item.clip.id.as_str(), message)
                .during(sequence, item.clip.start, item.clip.end())
                .measured((ratio * 100.0).round() / 100.0, LEAST_CONTRAST, ":1"),
        );
    }
    (picked.len(), of)
}

/// Titles and captions that reach outside title safe, and text boxes that
/// cover each other while both show.
pub(crate) fn text(sequence: &Sequence, findings: &mut Vec<Finding>) {
    let shown = shown(sequence);
    let clock = Clock::of_sequence(sequence);
    let (w, h) = canvas(sequence);
    let inset = (1.0 - TITLE_SAFE) / 2.0;
    let safe = [w * inset, h * inset, w * (1.0 - inset), h * (1.0 - inset)];
    for item in &shown {
        let clip = item.clip;
        let mut worst: Option<(f64, [f64; 4])> = None;
        for at in samples(sequence, &[item], clip.start, clip.end()) {
            let Some(bounds) = item.bounds(sequence, at) else {
                continue;
            };
            let outside = [
                safe[0] - bounds[0],
                safe[1] - bounds[1],
                bounds[2] - safe[2],
                bounds[3] - safe[3],
            ]
            .into_iter()
            .fold(0.0, f64::max);
            if outside > TOLERANCE && worst.is_none_or(|(most, _)| outside > most) {
                worst = Some((outside, bounds));
            }
        }
        let Some((outside, bounds)) = worst else {
            continue;
        };
        let pixels = outside.round();
        findings.push(
            Finding::warning(
                "text.safe_area",
                clip.id.as_str(),
                format!(
                    "{} reaches {pixels} px outside title safe ({}% of the {}x{} frame) from {} to {}",
                    item.describe(),
                    (TITLE_SAFE * 100.0).round(),
                    sequence.width,
                    sequence.height,
                    clock.print_position(clip.start),
                    clock.print_position(clip.end()),
                ),
            )
            .during(sequence, clip.start, clip.end())
            .measured(pixels, 0.0, "px")
            .on(normalised(sequence, bounds)),
        );
    }
    for (index, first) in shown.iter().enumerate() {
        for second in &shown[index + 1..] {
            let from = first.clip.start.max(second.clip.start);
            let to = first.clip.end().min(second.clip.end());
            if from >= to {
                continue;
            }
            let mut worst: Option<(f64, [f64; 4], Time)> = None;
            for at in samples(sequence, &[first, second], from, to) {
                let (Some(a), Some(b)) = (first.bounds(sequence, at), second.bounds(sequence, at))
                else {
                    continue;
                };
                let both = [
                    a[0].max(b[0]),
                    a[1].max(b[1]),
                    a[2].min(b[2]),
                    a[3].min(b[3]),
                ];
                let depth = (both[2] - both[0]).min(both[3] - both[1]);
                if depth > TOLERANCE && worst.is_none_or(|(most, _, _)| depth > most) {
                    worst = Some((depth, both, at));
                }
            }
            let Some((depth, both, _)) = worst else {
                continue;
            };
            // A caption is what the note is about; else the later track's
            // title, which is drawn over the other.
            let (subject, other) = if second.kind == Kind::Caption && first.kind != Kind::Caption {
                (second, first)
            } else if first.kind == Kind::Caption {
                (first, second)
            } else {
                (second, first)
            };
            let pixels = depth.round();
            findings.push(
                Finding::warning(
                    "text.overlap",
                    subject.clip.id.as_str(),
                    format!(
                        "{} overlaps {} by {pixels} px from {} to {}",
                        subject.describe(),
                        other.describe(),
                        clock.print_position(from),
                        clock.print_position(to),
                    ),
                )
                .during(sequence, from, to)
                .measured(pixels, 0.0, "px")
                .on(normalised(sequence, both)),
            );
        }
    }
}

/// What a viewer reads of a caption: its characters, with each run of
/// spaces and line breaks one space.
fn characters(text: &str) -> usize {
    text.split_whitespace()
        .map(|word| word.chars().count())
        .sum::<usize>()
        + text.split_whitespace().count().saturating_sub(1)
}

/// Captions that go by faster than a viewer reads, stay up less than 5/6
/// of a second, or leave less than two frames before the next one.
pub(crate) fn captions(sequence: &Sequence, findings: &mut Vec<Finding>) {
    let rate = sequence.rate;
    let clock = Clock::of_sequence(sequence);
    let second = Time::from_seconds(1.0).unwrap_or(Time::ZERO);
    let shortest = Time::from_flicks(second.flicks() * SHORT_CAPTION_SIXTHS / 6);
    for track in sequence
        .tracks
        .iter()
        .filter(|track| track.kind == TrackKind::Subtitle && track.enabled)
    {
        let captions: Vec<(&Clip, &str)> = track
            .clips
            .iter()
            .filter(|clip| clip.enabled)
            .filter_map(|clip| match &clip.content {
                Content::Caption { text } if !text.trim().is_empty() => Some((clip, text.as_str())),
                _ => None,
            })
            .collect();
        for (clip, text) in &captions {
            let seconds = clip.duration.seconds();
            let span = || {
                format!(
                    "from {} to {}",
                    clock.print_position(clip.start),
                    clock.print_position(clip.end())
                )
            };
            if seconds > 0.0 {
                let speed = characters(text) as f64 / seconds;
                if speed > READING_SPEED + 1e-9 {
                    let shown = (speed * 10.0).round() / 10.0;
                    findings.push(
                        Finding::warning(
                            "caption.reading_speed",
                            clip.id.as_str(),
                            format!(
                                "caption `{}` asks for {shown} characters a second, more than {READING_SPEED} ({} characters in {:.2} s, {})",
                                clip.id,
                                characters(text),
                                seconds,
                                span(),
                            ),
                        )
                        .during(sequence, clip.start, clip.end())
                        .measured(shown, READING_SPEED, "cps"),
                    );
                }
            }
            if clip.duration < shortest {
                let shown = (seconds * 1000.0).round() / 1000.0;
                findings.push(
                    Finding::warning(
                        "caption.short",
                        clip.id.as_str(),
                        format!(
                            "caption `{}` is up for {} ({seconds:.2} s), less than 5/6 of a second, {}",
                            clip.id,
                            clock.print_duration(clip.duration),
                            span(),
                        ),
                    )
                    .during(sequence, clip.start, clip.end())
                    .measured(shown, (shortest.seconds() * 1000.0).round() / 1000.0, "s"),
                );
            }
        }
        for pair in captions.windows(2) {
            let ((before, _), (after, _)) = (pair[0], pair[1]);
            let gap = after.start - before.end();
            let frames = rate.frames_round(gap);
            if gap > Time::ZERO && frames < CAPTION_GAP_FRAMES {
                findings.push(
                    Finding::warning(
                        "caption.gap",
                        after.id.as_str(),
                        format!(
                            "caption `{}` follows `{}` after {frames} frame{} at {}; leave {CAPTION_GAP_FRAMES} frames or none",
                            after.id,
                            before.id,
                            if frames == 1 { "" } else { "s" },
                            clock.print_position(rate.round(before.end())),
                        ),
                    )
                    .during(sequence, before.end(), after.start)
                    .measured(frames as f64, CAPTION_GAP_FRAMES as f64, "frames"),
                );
            }
        }
    }
}

#[cfg(test)]
mod contrast_tests {
    use super::*;
    use kerf_lang::parse_document;

    fn project(body: &str) -> Project {
        parse_document(
            &format!(
                "kerf 1\nproject name=\"t\"\nsequence s1 rate=24 size=640x360 audio=48000\n{body}"
            ),
            Catalog::builtin(),
        )
        .unwrap_or_else(|errors| panic!("{errors:?}"))
        .project
    }

    fn read(body: &str) -> Vec<Finding> {
        let project = project(body);
        let sequence = &project.sequences[0];
        let mut findings = Vec::new();
        let range = (Time::ZERO, sequence.duration());
        contrast(
            &project,
            sequence,
            range,
            240,
            |shown, at| {
                kerf_engine::still::draw_frame(shown, Some("s1"), at, 1.0)
                    .ok()
                    .map(|frame| (frame.width, frame.height, frame.rgba))
            },
            &mut findings,
        );
        findings
    }

    /// A title in `color` over a solid in `behind`.
    fn title_over(behind: &str, color: &str, more: &str) -> String {
        format!(
            "  track V1 video\n    clip back gen=solid@1 at=0f dur=48f\n      set color={behind}\n  track V2 video\n    clip words gen=title@1 at=0f dur=48f\n      set text=\"Plain words\" color={color} size=0.12{more}\n"
        )
    }

    fn ratio_of(finding: &Finding) -> f64 {
        finding.value.unwrap()
    }

    #[test]
    fn text_that_stands_out_is_fine() {
        for (behind, color) in [
            ("#000000", "#ffffff"),
            ("#ffffff", "#000000"),
            // Just past 3:1: white on a mid grey of about 4.5 of luminance 0.18.
            ("#808080", "#ffffff"),
        ] {
            let findings = read(&title_over(behind, color, ""));
            assert!(findings.is_empty(), "{behind} {color}: {findings:?}");
        }
    }

    #[test]
    fn text_too_faint_is_flagged_with_how_faint() {
        // A little apart: luminances 0.318 and 0.185 make (0.368) / (0.235) = 1.57:1.
        let findings = read(&title_over("#777777", "#999999", ""));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert_eq!(findings[0].code, "text.contrast");
        assert_eq!(findings[0].entity.as_deref(), Some("words"));
        let ratio = ratio_of(&findings[0]);
        assert!((ratio - 1.57).abs() < 0.1, "{ratio}");
        assert!(findings[0].at.is_some(), "{findings:?}");
    }

    #[test]
    fn text_the_color_of_what_is_behind_it_does_not_show() {
        let findings = read(&title_over("#ffffff", "#ffffff", ""));
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].message.contains("does not show"),
            "{findings:?}"
        );
    }

    #[test]
    fn text_covered_by_a_picture_does_not_show() {
        let body = format!(
            "{}  track V3 video\n    clip over gen=solid@1 at=0f dur=48f\n      set color=#ff0000\n",
            title_over("#000000", "#ffffff", "")
        );
        let findings = read(&body);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(
            findings[0].message.contains("does not show"),
            "{findings:?}"
        );
    }

    #[test]
    fn text_over_a_bright_part_of_a_dark_picture_is_flagged_for_that_part() {
        // White words, half of them over white and half of them over black.
        let body = "  track V1 video\n    clip dark gen=solid@1 at=0f dur=48f\n      set color=#000000\n  track V2 video\n    clip light gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.5 x=-0.25\n  track V3 video\n    clip words gen=title@1 at=0f dur=48f\n      set text=\"Plain words\" color=#ffffff size=0.12\n";
        let findings = read(body);
        assert_eq!(findings.len(), 1, "{findings:?}");
        assert!(findings[0].message.contains('%'), "{findings:?}");
        // A little of it over a highlight is not worth saying.
        let body = "  track V1 video\n    clip dark gen=solid@1 at=0f dur=48f\n      set color=#000000\n  track V2 video\n    clip light gen=solid@1 at=0f dur=48f\n      set color=#ffffff scale=0.05 x=-0.15\n  track V3 video\n    clip words gen=title@1 at=0f dur=48f\n      set text=\"Plain words\" color=#ffffff size=0.12\n";
        assert!(read(body).is_empty());
    }

    #[test]
    fn a_title_that_fades_in_is_read_where_it_has_come_in() {
        let findings = read(&title_over("#000000", "#ffffff", " opacity={0f:0 24f:1}"));
        assert!(findings.is_empty(), "{findings:?}");
        // And one that stays faint throughout is faint.
        let findings = read(&title_over("#000000", "#ffffff", " opacity=0.1"));
        assert_eq!(findings.len(), 1, "{findings:?}");
    }

    #[test]
    fn a_captions_outline_keeps_it_readable_over_anything() {
        let captions =
            "  track S1 subtitle\n    clip cap caption=\"Words to read\" at=0f dur=48f\n";
        for color in ["#000000", "#808080", "#ffffff"] {
            let body = format!(
                "  track V1 video\n    clip back gen=solid@1 at=0f dur=48f\n      set color={color}\n{captions}"
            );
            let findings = read(&body);
            assert!(findings.is_empty(), "{color}: {findings:?}");
        }
    }

    /// Whether a title in `color` is flagged over a moving, busy picture of everything from black to
    /// white, with how faint it is.
    fn over_busy_footage(dir: &std::path::Path, color: &str) -> Option<Vec<f64>> {
        let path = dir.join("busy.mp4");
        if !path.exists() {
            let output = std::process::Command::new("ffmpeg")
                .args(["-hide_banner", "-loglevel", "error", "-y", "-f", "lavfi"])
                .args(["-i", "testsrc2=s=640x360:r=24:d=2"])
                .args([
                    "-c:v",
                    "libx264",
                    "-preset",
                    "ultrafast",
                    "-pix_fmt",
                    "yuv420p",
                ])
                .arg(&path)
                .output()
                .ok()?;
            if !output.status.success() {
                return None;
            }
        }
        let body = format!(
            "kerf 1\nproject name=\"t\"\nmedia m path=\"{}\" rate=24 dur=48f video=640x360\nsequence s1 rate=24 size=640x360 audio=48000\n  track V1 video\n    clip c media=m src=0f at=0f dur=48f\n  track V2 video\n    clip words gen=title@1 at=0f dur=48f\n      set text=\"Plain words\" color={color} size=0.12\n",
            path.display()
        );
        let project = parse_document(&body, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        let sequence = &project.sequences[0];
        let mut findings = Vec::new();
        contrast_of(
            &project,
            sequence,
            (Time::ZERO, sequence.duration()),
            |_| true,
            &mut findings,
        );
        Some(
            findings
                .iter()
                .filter_map(|finding| finding.value)
                .collect(),
        )
    }

    #[test]
    fn over_a_busy_picture_it_goes_by_the_parts_of_the_text() {
        let dir = tempfile::tempdir().expect("a folder");
        let Some(white) = over_busy_footage(dir.path(), "#ffffff") else {
            eprintln!("no ffmpeg");
            return;
        };
        let gray = over_busy_footage(dir.path(), "#808080").expect("as above");
        let black = over_busy_footage(dir.path(), "#000000").expect("as above");
        eprintln!("over testsrc2: white {white:?}, gray {gray:?}, black {black:?}");
        // Mid gray has little to set it off from a picture with as much either side of it.
        assert_eq!(gray.len(), 1, "{gray:?}");
    }

    #[test]
    fn it_reads_no_more_frames_than_it_is_given() {
        let body = (0..30)
            .map(|at| {
                format!(
                    "    clip t{at} gen=title@1 at={}f dur=10f\n      set text=\"Word\"\n",
                    at * 10
                )
            })
            .collect::<String>();
        let project = project(&format!("  track V1 video\n{body}"));
        let sequence = &project.sequences[0];
        let mut drawn = 0;
        let (read, of) = contrast(
            &project,
            sequence,
            (Time::ZERO, sequence.duration()),
            18,
            |_, _| {
                drawn += 1;
                None
            },
            &mut Vec::new(),
        );
        assert_eq!((read, of), (2, 30));
        assert!(drawn <= 18, "{drawn}");
    }
}
