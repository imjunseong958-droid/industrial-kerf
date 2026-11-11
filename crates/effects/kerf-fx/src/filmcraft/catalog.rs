//! The FilmCraft effects Kerf carries: their Kerf ids, versions, and what each is for, and the
//! catalog table (`kerf-model/src/effects.tsv`) written from their definitions.
//!
//! Each FilmCraft parameter becomes a Kerf parameter of the same name, as numbers, switches,
//! choices (words), colors, and text. A point becomes an `_x` and a `_y` number across the
//! picture (0 to 1), or, for an offset, in pixels; a color wheel an `_x` and a `_y` from -1 to 1;
//! a curve text of `x,y` points; a LUT a `.cube` or `.3dl` file; another track a track id.

use super::project::effect::TRACK_CHOICES;
use super::project::{EffectDef, EffectKind, ParamDef, ParamKind, ParamValue};
use std::fmt::Write as _;

/// FilmCraft's id, Kerf's id, the version Kerf carries it at, and when an editor reaches for
/// it. Version 2 replaces the EffectCraft effect Kerf listed under the same id, which this Kerf
/// no longer reads.
pub(crate) const CARRIED: &[(&str, &str, u32, &str)] = &[
    // Blur & Sharpen
    ("bokeh_blur", "blur.bokeh", 1, "Lens-shaped defocus whose bright spots bloom into shaped bokeh; dreamy backgrounds and night lights."),
    ("camera_blur", "blur.camera", 1, "Out-of-focus blur as a camera lens makes it; soft backgrounds and focus pulls."),
    ("channel_blur", "blur.channel", 2, "Blurs red, green, blue, or alpha separately; color fringing and glitchy softness."),
    ("compound_blur", "blur.compound", 2, "Blurs more where another track is brighter (layer=V2); depth-of-field fakes and selective softness."),
    ("directional_blur", "blur.directional", 2, "Motion blur in one direction; fake fast motion and whip pans."),
    ("focus_blur", "blur.focus", 1, "Keeps a band or circle sharp and blurs the rest; tilt-shift miniatures and spotlighting a subject."),
    ("reduce_interlace_flicker", "blur.reduceflicker", 1, "Softens vertically to stop thin lines flickering; fine stripes, small text, interlaced footage."),
    ("sharpen", "blur.sharpen", 2, "Sharpens edges; crisp up soft footage, kept subtle."),
    ("unsharp_mask", "blur.unsharp", 2, "Sharpens by contrast at edges; restores detail after scaling."),
    // Color Correction
    ("asc_cdl", "color.asccdl", 1, "Slope, offset, and power per channel with saturation, the ASC CDL; matching a colorist's grade numbers."),
    ("black_white", "color.blackwhite", 2, "Turns the picture black and white; monochrome and retro looks."),
    ("brightness_contrast", "color.brightnesscontrast", 2, "Plain brightness and contrast; basic correction."),
    ("change_to_color", "color.changetocolor", 2, "Replaces one hue with another; recolor clothing or props."),
    ("channel_mix", "color.channelmixer", 2, "Mixes color channels into each other; creative grades, custom black and white."),
    ("color_balance", "color.colorbalance", 2, "Shifts red, green, and blue in shadows, midtones, and highlights; warm or cool grading."),
    ("color_pass", "color.colorpass", 1, "Keeps one color and grays everything else, or the reverse; color-pop highlights."),
    ("color_replace", "color.colorreplace", 1, "Paints one picked color with another; quick recolors of flat areas."),
    ("gamma_correction", "color.gamma", 1, "Brightens or darkens the midtones alone; lifting murky footage."),
    ("leave_color", "color.leavecolor", 2, "Keeps one color and desaturates the rest; color-pop highlights."),
    ("levels", "color.levels", 2, "Input and output levels with gamma; contrast and exposure fixes."),
    ("lumetri", "color.lumetri", 2, "A full grade in one effect: white balance and tone, a creative look or LUT, curves, color wheels, an HSL secondary, and a vignette; everyday color correction and film looks."),
    ("proc_amp", "color.procamp", 1, "Brightness, contrast, hue, and saturation together; quick video-style correction."),
    ("tint", "color.tint", 2, "Maps black and white to two colors; duotones and monochrome tints."),
    ("video_limiter", "color.videolimiter", 2, "Limits luma and chroma to the legal range; delivery compliance."),
    ("vignette", "stylize.vignette", 2, "Darkens or lightens the edges; draws the eye to the center, cinematic mood."),
    ("cineon_converter", "utility.cineon", 2, "Converts log Cineon footage to video and back; log to video."),
    // Channel
    ("invert", "channel.invert", 2, "Inverts colors or one channel; negative-film looks, flash frames, glitches."),
    // Distort
    ("corner_pin", "distort.cornerpin", 2, "Moves the four corners; fit a picture into a screen or frame."),
    ("lens_distortion", "distort.lens", 2, "Barrel or pincushion lens curvature; fisheye looks, or undoing a wide lens."),
    ("magnify", "distort.magnify", 2, "A magnifying circle or square; zoom in on a detail."),
    ("mirror", "distort.mirror", 2, "Mirrors the picture across a line; symmetric, surreal reflections."),
    ("offset", "distort.offset", 2, "Shifts the picture with wraparound; seamless scrolls and glitch slides."),
    ("transform", "distort.transform", 2, "Position, scale, rotation, and skew with shutter motion blur; precise moves inside the picture."),
    ("warp_stabilizer", "distort.warpstabilizer", 2, "Smooths or removes camera shake from an analysis of the clip's frames; handheld and shaky footage."),
    ("wave_warp", "distort.wavewarp", 2, "Moving waves across the picture; flags, water, wobbling text."),
    // Generate
    ("checkerboard", "generate.checkerboard", 2, "A checkerboard; backgrounds and patterns."),
    ("four_color_gradient", "generate.fourcolor", 2, "A gradient from four colored points; backgrounds and color washes."),
    ("gradient", "generate.gradient", 1, "A smooth linear or radial gradient between two colors, dithered against banding; skies, backgrounds, shading."),
    ("ramp", "generate.gradientramp", 2, "A linear or radial ramp between two colors; skies, backgrounds, shading."),
    ("grid", "generate.grid", 2, "Draws a grid of lines; layouts, tech overlays, and backgrounds."),
    ("paint_bucket", "generate.paintbucket", 2, "Fills a region from a point; recolor flat areas."),
    ("write_on", "generate.writeon", 2, "Paints a stroke along the path its brush point takes over the clip; underlines and doodles."),
    // Immersive Video
    ("vr_blur", "vr.blur", 2, "Blurs a 360° frame evenly on the sphere, seams and poles included; soft 360 backgrounds."),
    ("vr_denoise", "vr.denoise", 2, "Reduces noise on a 360° frame without seams; cleaning VR footage."),
    ("vr_plane_to_sphere", "vr.planetosphere", 2, "Places a flat picture onto a 360° frame; titles and graphics inside VR."),
    ("vr_projection", "vr.projection", 1, "Changes the view of a 360° frame: field of view, pan, tilt, and roll; reframing VR shots."),
    ("vr_rotate_sphere", "vr.rotatesphere", 2, "Turns a 360° frame around its axes; leveling the horizon or choosing the front view."),
    ("vr_sharpen", "vr.sharpen", 2, "Sharpens a 360° frame evenly without seams; crisper VR footage."),
    // Keying
    ("alpha_adjust", "key.alphaadjust", 1, "Scales, ignores, or inverts the picture's alpha, or shows it as a mask; fixing cutouts."),
    ("color_key", "key.color", 1, "Keys out one color with a tolerance and feathered edge; simple flat backgrounds."),
    ("extract", "key.extract", 2, "Keys by brightness into black and white; removes black or white backgrounds."),
    ("logo_cutout", "key.logocutout", 1, "Cuts a logo out of its plain background by color or brightness; logos and line art over footage."),
    ("luma_key", "key.luma", 1, "Keys out dark areas by brightness; footage shot against black, fire, sparks."),
    ("track_matte", "key.trackmatte", 1, "Shows the picture through another track's alpha or brightness (matte=V2); text filled with video, shaped reveals."),
    ("ultra_key", "key.ultra", 1, "A chroma key for green and blue screen with matte, spill, and color controls; studio footage."),
    // Lights & Glows
    ("glint", "light.glint", 1, "Star-shaped glints on the brightest spots; sparkle, jewelry, sunlight on water."),
    ("light_leaks", "light.leaks", 1, "Animated warm light leaks washing over the picture; film, vlog, and nostalgic looks."),
    ("rgb_split", "light.rgbsplit", 1, "Splits the red, green, and blue channels apart; glitch hits, chromatic aberration, music videos."),
    ("wonder_glow", "light.wonderglow", 1, "A soft, rich glow from the highlights; dreamy, cinematic bloom."),
    // Noise & Grain
    ("median", "noise.median", 2, "A median filter; removes noise, painterly smoothing."),
    ("noise", "noise.noise", 2, "Adds random noise, in color or not; film texture and analog grit."),
    // Perspective
    ("drop_shadow", "perspective.dropshadow", 2, "A drop shadow; separates text and cutouts from the background."),
    // Stylize
    ("mosaic", "stylize.mosaic", 2, "Pixelates into blocks; censoring and retro 8-bit."),
    ("posterize", "stylize.posterize", 2, "Reduces the color levels; poster and pop-art looks."),
    // Text
    ("clip_name", "text.clipname", 1, "Burns the clip's name into the picture; review copies and versions."),
    ("metadata_burnin", "text.burnin", 1, "Burns timecode, the frame number, or the file name into the picture; review copies and slates."),
    ("simple_text", "text.simple", 1, "A line of text drawn on the clip itself, with a backing box; labels, quick captions, and lower thirds."),
    ("timecode", "text.timecode", 2, "Burns the timecode into the picture; review copies and slates."),
    // Time
    ("echo", "time.echo", 2, "Blends earlier or later frames over the current one; trails, ghosting, and motion echoes."),
    ("posterize_time", "time.posterizetime", 2, "Holds frames to play at a lower frame rate; choppy, stop-motion, or stylized movement."),
    // Transform
    ("auto_reframe", "transform.autoreframe", 1, "Keeps the moving subject in frame when the sequence's aspect differs from the clip's; vertical and square versions."),
    ("horizontal_flip", "transform.hflip", 1, "Mirrors the picture left to right; flipping a shot's direction."),
    ("spacer", "transform.spacer", 1, "Pads the picture with a border of color, rounded if you like; framed cards and picture-in-picture."),
    ("vertical_flip", "transform.vflip", 1, "Turns the picture upside down; reflections and surreal flips."),
    // Utility
    ("auto_align", "utility.autoalign", 1, "Moves the picture's visible content to an edge or the center with a margin; aligning cutouts and logos."),
    ("edge_feather", "utility.edgefeather", 1, "Softens the picture's outer edges into transparency; blending cutouts and picture-in-picture."),
    ("rounded_crop", "utility.roundedcrop", 1, "Crops with rounded corners, a feather, and a border; rounded picture-in-picture and cards."),
    ("stroke", "utility.stroke", 1, "An outline around the picture's alpha, inside, outside, or centered; stickers and outlined cutouts."),
];

/// Point parameters that are offsets in pixels rather than places across the picture.
const OFFSETS: &[(&str, &str)] = &[
    ("auto_reframe", "offset"),
];

/// Field names an effect line takes itself, which a parameter cannot be.
const FIELDS: &[&str] = &["at", "enabled", "reset"];

/// What an effect written for 0 to 1 needs to keep what an HDR picture holds above 1, and the colors
/// past BT.709 it holds as values below 0.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Headroom {
    /// Nothing: it keeps those values, or it means to limit them.
    Keeps,
    /// Each pixel brought under 1 and inside BT.709, and put back: its math is on the pixel alone.
    PerPixel,
    /// The whole picture brought between 0 and 1, and put back: it reads its neighbors or moves pixels.
    Whole,
    /// Each pixel brought inside BT.709 and put back: it keeps what is above 1 but not what is below 0.
    Gamut,
}

/// How effect `id` handles an HDR picture. Measured by running every effect, at its defaults and with
/// each number moved a tenth of its range, on footage above SDR white and past BT.709.
pub fn headroom(id: &str) -> Headroom {
    match id {
        "blur.bilateral" | "blur.radialzoom" | "distort.opticscompensation"
        | "distort.rollingshutterrepair" | "distort.upscale" | "image.fill"
        | "matte.simplechoker" | "noise.addgrain" | "noise.dustscratches" | "noise.removegrain" => {
            Headroom::Whole
        }
        "blur.sharpen" | "blur.unsharp" | "color.lumetri" | "light.glint" | "light.wonderglow"
        | "vr.sharpen" => Headroom::Gamut,
        "channel.invert" | "color.asccdl" | "color.autocolor" | "color.autocontrast" | "color.basic"
        | "color.autolevels" | "color.brightnesscontrast" | "color.changetocolor"
        | "color.channelmixer" | "color.colorbalance" | "color.curves" | "color.gamma"
        | "color.huesaturation" | "color.levels" | "color.photofilter" | "color.procamp"
        | "color.selectivecolor" | "color.shadowhighlight" | "color.tint" | "key.advancedspill"
        | "key.color" | "key.extract" | "key.keycleaner" | "key.logocutout" | "key.ultra"
        | "key.unmult" | "light.leaks" | "look.film" | "look.flash" | "look.noir" | "look.vhs"
        | "noise.noise" | "stylize.posterize" | "stylize.vignette" | "utility.cineon" => Headroom::PerPixel,
        _ => Headroom::Keeps,
    }
}

/// The FilmCraft definition Kerf carries as `id`.
pub fn carried(id: &str) -> Option<&'static EffectDef> {
    let (filmcraft, ..) = CARRIED.iter().find(|(_, kerf, ..)| *kerf == id)?;
    super::project::find_effect(filmcraft)
}

/// How a FilmCraft parameter is written in Kerf.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Written {
    Number,
    Bool,
    Text,
    Color,
    /// A choice, as these words in FilmCraft's order.
    Choice(Vec<String>),
    /// Another picture track, by id; FilmCraft's choice of `Video 1`… (0 for none).
    Track,
    /// A point: `_x` and `_y` across the picture, or, for an offset, in pixels.
    Point {
        offset: bool,
    },
    /// A color wheel's offset: `_x` and `_y` from -1 to 1.
    Wheel,
    /// A tone curve as text, `x,y` points from 0 to 1.
    Curve,
    /// A LUT file (`.cube` or `.3dl`).
    Lut,
}

pub(crate) fn written(effect: &str, param: &ParamDef) -> Written {
    match &param.kind {
        ParamKind::Float { .. } | ParamKind::Angle => Written::Number,
        ParamKind::Bool => Written::Bool,
        ParamKind::Color => Written::Color,
        ParamKind::Choice(options) if *options == TRACK_CHOICES => Written::Track,
        ParamKind::Choice(options) => Written::Choice(words(options)),
        ParamKind::Point => Written::Point {
            offset: OFFSETS.contains(&(effect, param.id)),
        },
        ParamKind::Wheel => Written::Wheel,
        ParamKind::Curve { .. } => Written::Curve,
        ParamKind::Text if param.id.ends_with("_lut") => Written::Lut,
        ParamKind::Text | ParamKind::Path => Written::Text,
    }
}

/// A point's or wheel's two Kerf names.
pub(crate) fn axes(param: &str) -> [String; 2] {
    [format!("{param}_x"), format!("{param}_y")]
}

/// Choices as script words: lowercase letters and digits joined by hyphens, unique.
pub(crate) fn words(options: &[&str]) -> Vec<String> {
    let mut words: Vec<String> = Vec::new();
    for option in options {
        let mut word = String::new();
        for c in option.chars().map(|c| c.to_ascii_lowercase()) {
            if c.is_ascii_alphanumeric() {
                word.push(c);
            } else if c == '+' {
                word.push_str("-plus");
            } else if !word.is_empty() && !word.ends_with('-') {
                word.push('-');
            }
        }
        let mut word = word.trim_end_matches('-').to_owned();
        if word.is_empty() {
            word = "none".to_owned();
        }
        if words.contains(&word) {
            word = format!("{word}-{}", words.len() + 1);
        }
        words.push(word);
    }
    words
}

/// Lumetri's camera conversions, by the word Kerf names them with.
pub(crate) fn camera_luts() -> Vec<String> {
    super::luts::input_builtins()
        .map(|b| b.id.clone())
        .collect()
}

/// A curve's points as Kerf writes them.
pub(crate) fn curve_text(points: &[[f32; 2]]) -> String {
    points
        .iter()
        .map(|[x, y]| format!("{x},{y}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A FilmCraft (sRGB, straight) color as `#rrggbb[aa]`.
fn hex(c: [f32; 4]) -> String {
    let [r, g, b, a] = c.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
    if a == 255 {
        format!("#{r:02x}{g:02x}{b:02x}")
    } else {
        format!("#{r:02x}{g:02x}{b:02x}{a:02x}")
    }
}

fn category(id: &str) -> &'static str {
    match id.split('.').next().unwrap_or_default() {
        "blur" => "Blur & Sharpen",
        "channel" => "Channel",
        "color" => "Color Correction",
        "distort" => "Distort",
        "generate" => "Generate",
        "key" => "Keying",
        "light" => "Lights & Glows",
        "noise" => "Noise & Grain",
        "perspective" => "Perspective",
        "stylize" => "Stylize",
        "text" => "Text",
        "time" => "Time",
        "transform" => "Transform",
        "utility" => "Utility",
        "vr" => "Immersive Video",
        other => panic!("no category for `{other}`"),
    }
}

/// The catalog table of the carried effects (`kerf-model/src/effects.tsv`).
pub fn catalog_table() -> String {
    let mut out = String::from(
        "# Video effects Kerf draws with its own renderer, carried from FilmCraft\n\
         # (crates/render/src/{effects,gpufx,vfx}), generated by kerf-fx.\n\
         # Regenerate with `UPDATE_CATALOG=1 cargo test -p kerf-fx`.\n",
    );
    let mut carried: Vec<_> = CARRIED.iter().collect();
    carried.sort_by_key(|(_, id, ..)| (category(id), *id));
    for (filmcraft, id, version, guidance) in carried {
        let def = super::project::find_effect(filmcraft)
            .unwrap_or_else(|| panic!("FilmCraft has no `{filmcraft}`"));
        assert!(
            def.kind == EffectKind::Video && !def.intrinsic,
            "{filmcraft}"
        );
        write!(
            out,
            "effect\t{id}\t{}\t{}\t{guidance}",
            category(id),
            def.name
        )
        .unwrap();
        if *version != 1 {
            write!(out, "\t{version}").unwrap();
        }
        out.push('\n');
        let mut names: Vec<String> = Vec::new();
        for param in &def.params {
            let label = match param.group {
                Some(group) if group != param.label => format!("{group}: {}", param.label),
                _ => param.label.to_owned(),
            };
            let lines = lines(def, param, &label);
            for line in lines {
                let name = line
                    .split('\t')
                    .nth(2)
                    .expect("a parameter line names it")
                    .to_owned();
                assert!(
                    !FIELDS.contains(&name.as_str()),
                    "{id}: parameter `{name}` is a field name"
                );
                assert!(
                    !name.contains(['\t', ' ', '=', '"', '{', '}', '@']),
                    "{id}: `{name}` is not a script word"
                );
                assert!(!names.contains(&name), "{id}: `{name}` twice");
                names.push(name);
                out.push_str(&line);
                out.push('\n');
            }
        }
    }
    out
}

/// A FilmCraft parameter's lines in the table: tab, type, name, default, constraint, unit, label.
fn lines(def: &EffectDef, param: &ParamDef, label: &str) -> Vec<String> {
    let id = param.id;
    let line = |ty: &str, name: &str, default: &str, constraint: &str, unit: &str, label: &str| {
        format!("\t{ty}\t{name}\t{default}\t{constraint}\t{unit}\t{label}")
    };
    match (written(def.id, param), &param.kind, &param.default) {
        (Written::Number, ParamKind::Float { min, max, unit, .. }, ParamValue::Float(v)) => {
            let constraint = if min.is_finite() && max.is_finite() {
                format!("{min}..{max}")
            } else {
                String::new()
            };
            assert!(
                !(min.is_finite() && max.is_finite()) || (*min..=*max).contains(v),
                "{}: {id} default {v} out of range",
                def.id
            );
            let unit = match *unit {
                "°" => "degrees",
                other => other,
            };
            vec![line("number", id, &v.to_string(), &constraint, unit, label)]
        }
        (Written::Number, ParamKind::Angle, ParamValue::Float(v)) => {
            vec![line("number", id, &v.to_string(), "", "degrees", label)]
        }
        (Written::Bool, _, ParamValue::Bool(b)) => {
            vec![line("bool", id, &b.to_string(), "", "", label)]
        }
        (Written::Color, _, ParamValue::Color(c)) => {
            vec![line("color", id, &hex(*c), "", "", label)]
        }
        (Written::Choice(words), _, ParamValue::Choice(index)) => {
            let default = &words[(*index as usize).min(words.len() - 1)];
            vec![line("enum", id, default, &words.join("|"), "", label)]
        }
        (Written::Track, _, _) => vec![line("track", id, "", "", "", label)],
        (Written::Point { offset }, _, ParamValue::Vec2(v)) => {
            let [x, y] = axes(id);
            if offset {
                let at = |v: f64| if v.is_nan() { 0.0 } else { v };
                vec![
                    line(
                        "number",
                        &x,
                        &at(v.x).to_string(),
                        "-100000..100000",
                        "pixels",
                        &format!("{label} (across)"),
                    ),
                    line(
                        "number",
                        &y,
                        &at(v.y).to_string(),
                        "-100000..100000",
                        "pixels",
                        &format!("{label} (down)"),
                    ),
                ]
            } else {
                let (fx, fy) = place(def.id, id, *v);
                vec![
                    line(
                        "number",
                        &x,
                        &fx.to_string(),
                        "-10..10",
                        "picture widths",
                        &format!("{label} (0 left, 1 right)"),
                    ),
                    line(
                        "number",
                        &y,
                        &fy.to_string(),
                        "-10..10",
                        "picture heights",
                        &format!("{label} (0 top, 1 bottom)"),
                    ),
                ]
            }
        }
        (Written::Wheel, _, ParamValue::Vec2(v)) => {
            let [x, y] = axes(id);
            vec![
                line(
                    "number",
                    &x,
                    &v.x.to_string(),
                    "-1..1",
                    "",
                    &format!("{label} (across)"),
                ),
                line(
                    "number",
                    &y,
                    &v.y.to_string(),
                    "-1..1",
                    "",
                    &format!("{label} (down)"),
                ),
            ]
        }
        (Written::Curve, ParamKind::Curve { hue }, ParamValue::Curve(points)) => {
            let what = if *hue {
                "x,y points from 0 to 1, 0.5 neutral; empty for none"
            } else {
                "x,y points from 0 to 1"
            };
            vec![line(
                "text",
                id,
                &curve_text(points),
                "",
                "",
                &format!("{label} ({what})"),
            )]
        }
        (Written::Lut, _, _) => {
            let mut lines = vec![line("file", id, "", "cube|3dl", "", label)];
            if id == "input_lut" {
                // FilmCraft's built-in camera conversions, which its Input LUT also names.
                let mut words = vec!["none".to_owned()];
                words.extend(camera_luts());
                lines.push(line(
                    "enum",
                    "camera_lut",
                    "none",
                    &words.join("|"),
                    "",
                    &format!("{label}: built-in camera log conversion"),
                ));
            }
            lines
        }
        (Written::Text, _, ParamValue::Text(text)) => vec![line("text", id, text, "", "", label)],
        (written, kind, default) => {
            panic!("{}: {id} is {written:?} with {kind:?} {default:?}", def.id)
        }
    }
}

/// Where a point starts, across the picture: where FilmCraft puts an "auto" point (`end`, the
/// bottom centre; otherwise the effect's auto position or the centre), or the edge its default
/// (0 pixels) names.
fn place(effect: &str, param: &str, v: kerf_geom::Vec2) -> (f64, f64) {
    let (ax, ay) = super::project::effect::auto_point(effect, param)
        .unwrap_or((0.5, if param == "end" { 1.0 } else { 0.5 }));
    let at = |v: f64, auto: f64| {
        assert!(
            v.is_nan() || v == 0.0,
            "{effect}: {param} starts at {v} pixels"
        );
        if v.is_nan() {
            auto
        } else {
            0.0
        }
    };
    (at(v.x, ax), at(v.y, ay))
}

/// Every carried definition, for tests.
#[cfg(test)]
pub(crate) fn definitions() -> impl Iterator<Item = (&'static str, &'static EffectDef)> {
    CARRIED
        .iter()
        .map(|(filmcraft, id, ..)| (*id, super::project::find_effect(filmcraft).unwrap()))
}

#[cfg(test)]
mod tests {
    
    use super::*;

    #[test]
    fn effects_catalog_is_current() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../foundation/kerf-model/src/effects.tsv"
        );
        let table = catalog_table();
        if std::env::var_os("UPDATE_CATALOG").is_some() {
            std::fs::write(path, &table).unwrap();
            return;
        }
        assert!(
            std::fs::read_to_string(path).unwrap() == table,
            "the effects catalog is out of date; run `UPDATE_CATALOG=1 cargo test -p kerf-fx`"
        );
    }

    #[test]
    fn choices_become_words() {
        assert_eq!(
            words(&[
                "Normal",
                "Linear Dodge (Add)",
                "Stereoscopic - Over/Under",
                "None",
                "None"
            ]),
            [
                "normal",
                "linear-dodge-add",
                "stereoscopic-over-under",
                "none",
                "none-5"
            ]
        );
    }
}
