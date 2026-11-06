//! Text a viewer cannot read: titles and captions outside title safe, text
//! boxes drawn over each other, and captions that go by too fast, flash
//! up too briefly, or follow each other too closely.
//!
//! Boxes are laid out by `kerf-text` exactly as the compositor lays them
//! out (`kerf-compose` `draw.rs`, through `kerf-draw` `text::draw`): a
//! caption is Inter at 4.5% of the frame height (at least 12 px), centred,
//! its bottom at 95% of the frame; a title is its font at `size` frame
//! heights at its axes, centred on the frame, then cropped, scaled, rotated, and moved
//! like any picture. A box is the layout's line boxes, not its ink.

use crate::qc::{Finding, Rect};
use kerf_lang::Clock;
use kerf_model::{Catalog, Clip, Content, Domain, Param, Sequence, Time, TrackKind, Value};
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
