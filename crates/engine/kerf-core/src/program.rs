//! Render compiler for `kerf-model` projects.
//!
//! Lowers one sequence of a project to an FFmpeg filter graph. Times stay
//! exact until they are printed: every position and duration is computed in
//! flicks and written with nine decimals, and audio is placed by sample.
//! Clip properties, effects, transitions, and generators are lowered from
//! their catalog definitions; anything without a lowering is reported as
//! [`ProgramError::Unsupported`] naming the item, never silently dropped.
//!
//! - Picture tracks stack in sequence order, so a later track plays over an
//!   earlier one. Sound comes only from audio tracks.
//! - A transition plays across its cut using the clips' handles: a dissolve
//!   or wipe reveals the incoming clip over the outgoing one, a dip fades
//!   through its color, and an audio crossfade fades both sides.
//! - A nested sequence is composited on its own canvas and placed like any
//!   other picture; its sound is mixed and placed like a clip's.
//! - An adjustment layer applies its effects to everything beneath it.

use crate::overlay::{atempo_filters, captions_filter, escape_drawtext_text, fmt, CaptionCue};
use kerf_model::time::FLICKS_PER_SECOND;
use kerf_model::{
    Bezier, Catalog, Clip, Content, Domain, Engine, Id, Interp, Param, Project, Rate, Rgba,
    Sequence, Time, Track, TrackKind, Transition, Value,
};
use std::collections::HashMap;

mod sound;

pub use sound::{program_sound, ProgramSound, SoundCut, LiveBus};

/// Output audio sample rate.
pub const SAMPLE_RATE: u32 = 48_000;

/// Nested sequences deeper than this are refused.
const MAX_NESTING: usize = 16;

#[derive(Clone, Debug, Default)]
pub struct ProgramOptions {
    /// The sequence to render; the first one when absent.
    pub sequence: Option<String>,
    /// Render only `[from, to)` of the sequence, as an output starting at 0.
    pub range: Option<(Time, Time)>,
    /// Filters applied to the finished picture before captions are drawn.
    pub video_post_filters: Vec<String>,
    /// Keep the picture's transparency: composite over nothing and finish
    /// in RGBA, as for a file other renders read.
    pub transparent: bool,
    /// Compile the sound alone, for a picture drawn elsewhere: the program
    /// maps `[audio]` and nothing else.
    pub audio_only: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CompiledProgram {
    /// `ffmpeg -y <inputs> -filter_complex <graph> -map [video] -map [audio]`;
    /// the caller appends codec arguments, the rate, and the output.
    pub argv: Vec<String>,
    pub filter_complex: String,
    pub duration: Time,
    pub rate: Rate,
    pub width: u32,
    pub height: u32,
}

impl CompiledProgram {
    /// The output rate as FFmpeg reads it, `num/den`.
    pub fn rate_arg(&self) -> String {
        rate_arg(self.rate)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum ProgramError {
    #[error("{0}")]
    Invalid(String),
    #[error("{entity}: {what} cannot be rendered yet")]
    Unsupported { entity: String, what: String },
}

impl ProgramError {
    fn unsupported(entity: &Id, what: impl Into<String>) -> Self {
        Self::Unsupported {
            entity: entity.to_string(),
            what: what.into(),
        }
    }

    /// The clip, transition, or sequence the error is about.
    pub fn entity(&self) -> Option<&str> {
        match self {
            Self::Unsupported { entity, .. } => Some(entity),
            Self::Invalid(_) => None,
        }
    }
}

type Result<T> = std::result::Result<T, ProgramError>;

/// The sequence a render reads: `id`, or the first.
pub fn program_sequence<'a>(project: &'a Project, id: Option<&str>) -> Result<&'a Sequence> {
    match id {
        Some(id) => project
            .sequence(id)
            .ok_or_else(|| ProgramError::Invalid(format!("no sequence `{id}`"))),
        None => project
            .sequences
            .first()
            .ok_or_else(|| ProgramError::Invalid("the project has no sequence".into())),
    }
}

/// Compile `project` to a filter graph.
pub fn compile_program(
    project: &Project,
    catalog: &Catalog,
    options: &ProgramOptions,
) -> Result<CompiledProgram> {
    // Looks draw as the built-ins they stand for.
    let expanded = kerf_model::look::expand(project, catalog).map_err(ProgramError::Invalid)?;
    let project = &expanded;
    let sequence = program_sequence(project, options.sequence.as_deref())?;
    let (from, to) = match options.range {
        Some((from, to)) => {
            if from.is_negative() || to <= from {
                return Err(ProgramError::Invalid(
                    "a render range needs 0 <= from < to".into(),
                ));
            }
            (from, to)
        }
        None => (Time::ZERO, sequence.duration()),
    };
    if !to.is_positive() {
        return Err(ProgramError::Invalid(format!(
            "sequence `{}` is empty",
            sequence.id
        )));
    }
    let mut graph = Graph::new(project, catalog, sequence.rate);
    let video = if options.audio_only {
        None
    } else {
        let video = if options.transparent {
            graph.composite(sequence, from, to, Background::Clear, false)?
        } else {
            match graph.concat_program(sequence, from, to)? {
                Some(video) => video,
                None => graph.composite(sequence, from, to, Background::Black, false)?,
            }
        };
        let finished = graph.label("program");
        graph.parts.push(format!(
            "[{video}]{}format={}[{finished}]",
            options
                .video_post_filters
                .iter()
                .map(|filter| format!("{filter},"))
                .collect::<String>(),
            if options.transparent {
                "rgba"
            } else {
                "yuv420p"
            }
        ));
        Some(graph.captions(sequence, from, to, finished))
    };
    let audio = graph.mix(sequence, from, to, None)?;
    let (width, height) = canvas(sequence);
    let filter_complex = graph.parts.join(";\n");
    let mut argv = vec!["ffmpeg".to_owned(), "-y".to_owned()];
    for input in &graph.inputs {
        if input.seek.is_positive() {
            argv.extend(["-ss".to_owned(), secs(input.seek)]);
        }
        argv.extend([
            "-t".to_owned(),
            secs(input.span),
            "-i".to_owned(),
            input.path.clone(),
        ]);
    }
    argv.extend([
        "-filter_complex".to_owned(),
        filter_complex.replace('\n', ""),
    ]);
    if let Some(video) = video {
        argv.extend(["-map".to_owned(), format!("[{video}]")]);
    }
    argv.extend(["-map".to_owned(), format!("[{audio}]")]);
    Ok(CompiledProgram {
        argv,
        filter_complex,
        duration: to - from,
        rate: sequence.rate,
        width,
        height,
    })
}

/// Exact seconds with nine decimals and no trailing zeros.
pub fn secs(time: Time) -> String {
    let flicks = i128::from(time.flicks());
    let per = i128::from(FLICKS_PER_SECOND);
    let nanos = (flicks.abs() * 1_000_000_000 + per / 2) / per;
    let sign = if flicks < 0 && nanos != 0 { "-" } else { "" };
    let text = format!(
        "{sign}{}.{:09}",
        nanos / 1_000_000_000,
        nanos % 1_000_000_000
    );
    text.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// Samples at [`SAMPLE_RATE`] in `time`, rounded.
pub fn samples(time: Time) -> i64 {
    let flicks = i128::from(time.flicks()) * i128::from(SAMPLE_RATE);
    let per = i128::from(FLICKS_PER_SECOND);
    ((flicks + per / 2).div_euclid(per)) as i64
}

pub fn rate_arg(rate: Rate) -> String {
    format!("{}/{}", rate.num(), rate.den())
}

/// The sequence frame size, made even for 4:2:0 output.
pub fn canvas(sequence: &Sequence) -> (u32, u32) {
    let even = |value: u32| (value.max(2) / 2) * 2;
    (even(sequence.width), even(sequence.height))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Background {
    Black,
    Clear,
}

struct Input {
    path: String,
    seek: Time,
    span: Time,
}

/// How a clip enters across a transition.
#[derive(Clone, Copy)]
enum Reveal {
    Dissolve,
    Wipe { angle: f64, softness: f64 },
    Crossfade { equal_power: bool },
}

/// A clip as a track shows it: its span with transition handles and the
/// transitions it fades through.
struct Shown<'a> {
    clip: &'a Clip,
    from: Time,
    to: Time,
    fade_in: Option<(Reveal, Time, Time)>,
    fade_out: Option<(Reveal, Time, Time)>,
}

/// A dip's color, between the two clips of its cut.
struct Dip {
    color: Rgba,
    from: Time,
    to: Time,
}

/// The decoded part of a shown clip inside a window, in its sequence's time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Portion {
    /// First decoded timeline time. Material before `lo` is decoded only so
    /// a transition that began before the window fades correctly.
    pub decode: Time,
    /// The part shown or heard, `[lo, hi)`.
    pub lo: Time,
    pub hi: Time,
}

impl Portion {
    /// Clip-relative offset of the first decoded frame.
    pub fn offset(&self, clip: &Clip) -> Time {
        self.decode - clip.start
    }
}

impl Shown<'_> {
    /// The part of the item inside `[from, to)`, kept inside the `length`
    /// of its media or nested sequence.
    fn portion(&self, length: Option<Time>, from: Time, to: Time) -> Option<Portion> {
        let item = self;
        let mut lo = item.from.max(from);
        let mut hi = item.to.min(to);
        // Keep the source inside the media or nested sequence.
        if let Some(ends) = length.and_then(|length| {
            item.clip
                .played_at(Time::ZERO)
                .zip(item.clip.played_at(length))
        }) {
            let (earliest, latest) = if item.clip.speed().is_reverse() {
                (ends.1, ends.0)
            } else {
                ends
            };
            lo = lo.max(earliest);
            hi = hi.min(latest);
        }
        if lo >= hi {
            return None;
        }
        let mut decode = lo;
        // An outgoing clip fades only in a crossfade.
        let fade_out = item
            .fade_out
            .filter(|(reveal, ..)| matches!(reveal, Reveal::Crossfade { .. }));
        for (_, start, end) in [item.fade_in, fade_out].into_iter().flatten() {
            if start < lo && lo < end {
                decode = decode.min(start.max(item.from));
            }
        }
        Some(Portion { decode, lo, hi })
    }
}

struct Graph<'a> {
    project: &'a Project,
    catalog: &'a Catalog,
    rate: Rate,
    next: u32,
    parts: Vec<String>,
    inputs: Vec<Input>,
    input_index: HashMap<(String, i64, i64), usize>,
    nesting: Vec<Id>,
}

impl<'a> Graph<'a> {
    fn new(project: &'a Project, catalog: &'a Catalog, rate: Rate) -> Self {
        Self {
            project,
            catalog,
            rate,
            next: 0,
            parts: Vec::new(),
            inputs: Vec::new(),
            input_index: HashMap::new(),
            nesting: Vec::new(),
        }
    }

    fn label(&mut self, prefix: &str) -> String {
        let label = format!("{prefix}{}", self.next);
        self.next += 1;
        label
    }

    fn input(&mut self, path: &str, seek: Time, span: Time) -> usize {
        let key = (path.to_owned(), seek.flicks(), span.flicks());
        if let Some(index) = self.input_index.get(&key) {
            return *index;
        }
        let index = self.inputs.len();
        self.inputs.push(Input {
            path: path.to_owned(),
            seek,
            span,
        });
        self.input_index.insert(key, index);
        index
    }

    // ---- tracks --------------------------------------------------------------

    fn shown(&self, track: &'a Track, rate: Rate) -> Result<(Vec<Shown<'a>>, Vec<Dip>)> {
        let mut shown: Vec<Shown<'a>> = track
            .clips
            .iter()
            .map(|clip| Shown {
                clip,
                from: clip.start,
                to: clip.end(),
                fade_in: None,
                fade_out: None,
            })
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
                    shown[from].to = middle;
                    shown[to].from = middle;
                    dips.push(Dip {
                        color: self.color(&transition.id, &transition.params, "dip", "color")?,
                        from: start,
                        to: end,
                    });
                    continue;
                }
                "dissolve" => Reveal::Dissolve,
                "wipe" => Reveal::Wipe {
                    angle: self.static_number(transition, "wipe", "angle")?,
                    softness: self.static_number(transition, "wipe", "softness")?,
                },
                "crossfade" => Reveal::Crossfade {
                    equal_power: self.choice(&transition.params, "crossfade", "curve")
                        == "equal-power",
                },
                other => {
                    return Err(ProgramError::unsupported(
                        &transition.id,
                        format!("transition `{other}`"),
                    ));
                }
            };
            shown[from].to = end;
            shown[from].fade_out = Some((reveal, start, end));
            shown[to].from = start;
            shown[to].fade_in = Some((reveal, start, end));
        }
        Ok((shown, dips))
    }

    /// The part of `item` inside `[from, to)`, with its source bounds.
    fn portion(&self, item: &Shown, from: Time, to: Time) -> Option<Portion> {
        item.portion(self.source_length(item.clip), from, to)
    }

    fn source_length(&self, clip: &Clip) -> Option<Time> {
        match &clip.content {
            Content::Media { media, .. } => self.project.media(media.as_str()).map(|m| m.duration),
            Content::Sequence { sequence, .. } => self
                .project
                .sequence(sequence.as_str())
                .map(Sequence::duration),
            Content::Generator { .. } | Content::Caption { .. } => None,
        }
    }

    /// The source interval a portion reads, in source order.
    fn source_window(clip: &Clip, start: Time, end: Time) -> Option<(Time, Time)> {
        let (source_in, _) = clip.source_range()?;
        let speed = clip.speed();
        let (a, b) = (start - clip.start, end - clip.start);
        Some(if speed.is_reverse() {
            let out = source_in + speed.played(clip.duration);
            (out - speed.played(b), out - speed.played(a))
        } else {
            (source_in + speed.played(a), source_in + speed.played(b))
        })
    }

    // ---- picture ---------------------------------------------------------------

    /// A whole sequence window as one picture stream starting at 0.
    fn composite(
        &mut self,
        sequence: &'a Sequence,
        from: Time,
        to: Time,
        background: Background,
        with_captions: bool,
    ) -> Result<String> {
        let (width, height) = canvas(sequence);
        let base = self.label("base");
        let (color, format) = match background {
            Background::Black => ("black", "yuv420p"),
            Background::Clear => ("black@0", "rgba"),
        };
        self.parts.push(format!(
            "color=c={color}:s={width}x{height}:r={}:d={},format={format}[{base}]",
            rate_arg(self.rate),
            secs(to - from)
        ));
        let mut current = base;
        for track in &sequence.tracks {
            if track.kind != TrackKind::Video || !track.enabled {
                continue;
            }
            let (items, dips) = self.shown(track, sequence.rate)?;
            for item in &items {
                if !item.clip.enabled {
                    continue;
                }
                if matches!(&item.clip.content, Content::Generator { fx, .. } if fx == "adjustment")
                {
                    if item.fade_in.is_some() || item.fade_out.is_some() {
                        return Err(ProgramError::unsupported(
                            &item.clip.id,
                            "a transition on an adjustment layer",
                        ));
                    }
                    current = self.adjustment(current, item.clip, from, to)?;
                    continue;
                }
                let Some(portion) = self.portion(item, from, to) else {
                    continue;
                };
                if let Some(layer) = self.picture(sequence, item, &portion, from)? {
                    let out = self.label("layer");
                    self.parts.push(format!(
                        "[{current}][{}]overlay=x='{}':y='{}':eof_action=pass:shortest=0:format=auto:enable='between(t,{},{})'[{out}]",
                        layer.label,
                        layer.x,
                        layer.y,
                        secs(portion.lo - from),
                        secs(portion.hi - from),
                    ));
                    current = out;
                }
            }
            for dip in dips {
                current = self.dip(sequence, current, &dip, from, to);
            }
        }
        if with_captions {
            current = self.captions(sequence, from, to, current);
        }
        Ok(current)
    }

    /// Draw the captions of every enabled subtitle track.
    fn captions(&mut self, sequence: &Sequence, from: Time, to: Time, input: String) -> String {
        let mut current = input;
        for track in &sequence.tracks {
            if track.kind != TrackKind::Subtitle || !track.enabled {
                continue;
            }
            for clip in &track.clips {
                let Content::Caption { text } = &clip.content else {
                    continue;
                };
                let (start, end) = (clip.start.max(from), clip.end().min(to));
                if !clip.enabled || start >= end || text.trim().is_empty() {
                    continue;
                }
                let cue = CaptionCue {
                    start: (start - from).seconds(),
                    end: (end - from).seconds(),
                    text: text.clone(),
                };
                let out = self.label("caption");
                self.parts
                    .push(format!("[{current}]{}[{out}]", captions_filter(&cue)));
                current = out;
            }
        }
        current
    }

    /// A clip's picture for `portion`, placed on the window, with the
    /// overlay position expressions.
    fn picture(
        &mut self,
        sequence: &'a Sequence,
        item: &Shown,
        portion: &Portion,
        window: Time,
    ) -> Result<Option<Layer>> {
        let clip = item.clip;
        if !clip.masks.is_empty() {
            return Err(ProgramError::unsupported(&clip.id, "masks"));
        }
        if clip.angle.is_some() {
            return Err(ProgramError::unsupported(&clip.id, "a multicam angle"));
        }
        if matches!(
            clip.props.get("three_d"),
            Some(Param::Static(Value::Bool(true)))
        ) {
            return Err(ProgramError::unsupported(&clip.id, "a clip in depth"));
        }
        let (width, height) = canvas(sequence);
        let length = portion.hi - portion.decode;
        let mut filters = Vec::new();
        let mut transparent = false;
        let source = match &clip.content {
            Content::Media { media, .. } => {
                let media = self.project.media(media.as_str()).ok_or_else(|| {
                    ProgramError::Invalid(format!("clip `{}` uses missing media", clip.id))
                })?;
                let Some(video) = media.video else {
                    return Ok(None);
                };
                transparent = video.alpha;
                let (start, end) = Self::source_window(clip, portion.decode, portion.hi)
                    .expect("media clips have a source");
                let index = self.input(&media.path, start, end - start);
                filters.push(format!("trim=start=0:end={}", secs(end - start)));
                filters.push("setpts=PTS-STARTPTS".into());
                Source::Stream(format!("{index}:v"))
            }
            Content::Sequence {
                sequence: nested, ..
            } => {
                let nested = self.nested(clip, nested)?;
                let (start, end) = Self::source_window(clip, portion.decode, portion.hi)
                    .expect("nested clips have a source");
                self.nesting.push(nested.id.clone());
                let label = self.composite(nested, start, end, Background::Clear, true)?;
                self.nesting.pop();
                Source::Stream(label)
            }
            Content::Generator { fx, params, .. } => {
                let color = rate_arg(self.rate);
                let span = secs(length);
                match fx.as_str() {
                    "solid" => {
                        let fill = self.color(&clip.id, params, "solid", "color")?;
                        Source::Generated(format!(
                            "color=c={}:s={width}x{height}:r={color}:d={span},format=rgba",
                            ffmpeg_color(fill)
                        ))
                    }
                    "title" if params.contains_key("reveal") => {
                        return Err(ProgramError::unsupported(&clip.id, "a title's reveal"))
                    }
                    "title" => Source::Generated(format!(
                        "color=c=black@0:s={width}x{height}:r={color}:d={span},format=rgba,{}",
                        self.title(clip, params, height)?
                    )),
                    other => {
                        return Err(ProgramError::unsupported(
                            &clip.id,
                            format!("generator `{other}`"),
                        ))
                    }
                }
            }
            Content::Caption { .. } => return Ok(None),
        };
        let speed = clip.speed();
        let Some(ratio) = speed.constant() else {
            return Err(ProgramError::unsupported(&clip.id, "a speed ramp"));
        };
        if speed.is_reverse() {
            filters.push("reverse".into());
        }
        if ratio.num().unsigned_abs() != ratio.den() {
            filters.push(format!(
                "setpts=PTS*{}/{}",
                ratio.den(),
                ratio.num().unsigned_abs()
            ));
        }
        let fit = matches!(source, Source::Stream(_));
        let geometry = self.geometry(clip, portion, window, (width, height), fit)?;
        filters.extend(geometry.filters);
        let mut alpha = geometry.alpha;
        let effects = self.video_effects(clip)?;
        let opacity = self.param(clip, Domain::Video, "opacity");
        let tau = clip_time("T", portion.offset(clip));
        let fade = item.fade_in.map(|(reveal, start, end)| {
            (
                reveal,
                (start - portion.decode).seconds(),
                (end - start).seconds(),
            )
        });
        alpha |= transparent || fade.is_some() || !is_number(&opacity, 1.0) || !fit;
        if geometry.identity && matches!(clip.content, Content::Media { .. }) {
            filters.push(format!(
                "pad={width}:{height}:(ow-iw)/2:(oh-ih)/2:color={},setsar=1",
                if transparent { "black@0" } else { "black" }
            ));
        }
        filters.push(
            if alpha {
                "format=rgba"
            } else {
                "format=yuv420p"
            }
            .into(),
        );
        filters.extend(effects);
        match &opacity {
            Param::Static(Value::Number(value)) if *value != 1.0 => {
                filters.push(format!("colorchannelmixer=aa={}", fmt(*value)));
            }
            Param::Animated(_) => filters.push(format!(
                "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({})'",
                expression(&opacity, &tau)
            )),
            _ => {}
        }
        if let Some((reveal, start, duration)) = fade {
            filters.push(reveal_filter(reveal, start, duration));
        }
        filters.push(format!(
            "trim=start={}:duration={}",
            secs(portion.lo - portion.decode),
            secs(portion.hi - portion.lo)
        ));
        filters.push(format!(
            "setpts=PTS-STARTPTS+{}/TB",
            secs(portion.lo - window)
        ));
        let label = self.label("clip");
        let chain = filters.join(",");
        self.parts.push(match source {
            Source::Stream(input) => format!("[{input}]{chain}[{label}]"),
            Source::Generated(head) => format!("{head},{chain}[{label}]"),
        });
        Ok(Some(Layer {
            label,
            x: geometry.x,
            y: geometry.y,
        }))
    }

    fn nested(&self, clip: &Clip, id: &Id) -> Result<&'a Sequence> {
        if self.nesting.len() >= MAX_NESTING || self.nesting.contains(id) {
            return Err(ProgramError::unsupported(
                &clip.id,
                "sequences nested this deep",
            ));
        }
        self.project.sequence(id.as_str()).ok_or_else(|| {
            ProgramError::Invalid(format!("clip `{}` uses missing sequence `{id}`", clip.id))
        })
    }

    fn title(&self, clip: &Clip, params: &kerf_model::Params, height: u32) -> Result<String> {
        let def = self.catalog.fx("title").expect("the catalog defines title");
        let get = |name: &str| -> Result<Value> {
            match params.get(name) {
                None => Ok(def.find(name).expect("title parameter").default.clone()),
                Some(Param::Static(value)) => Ok(value.clone()),
                Some(Param::Animated(_)) => Err(ProgramError::unsupported(
                    &clip.id,
                    format!("animated title `{name}`"),
                )),
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
        let x = match get("align")? {
            Value::Enum(align) if align == "left" => "w/2",
            Value::Enum(align) if align == "right" => "w/2-text_w",
            _ => "(w-text_w)/2",
        };
        let mut options = vec![
            format!("text={}", escape_drawtext_text(&text)),
            format!("fontcolor={}", ffmpeg_color(color)),
            format!("fontsize={}", fmt(size.max(1.0))),
            format!("x='{x}'"),
            "y='(h-text_h)/2'".into(),
            "expansion=none".into(),
        ];
        if let Value::Text(font) = get("font")? {
            if font != "sans-serif" && !font.is_empty() {
                options.push(format!("font={}", escape_drawtext_text(&font)));
            }
        }
        Ok(format!("drawtext={}", options.join(":")))
    }

    /// Crop, size, flip, rotation, and position of a clip's picture.
    fn geometry(
        &self,
        clip: &Clip,
        portion: &Portion,
        window: Time,
        (width, height): (u32, u32),
        fit: bool,
    ) -> Result<Geometry> {
        let p = |name: &str| self.param(clip, Domain::Video, name);
        for name in ["anchor_x", "anchor_y"] {
            if !is_number(&p(name), 0.0) {
                return Err(ProgramError::unsupported(&clip.id, format!("`{name}`")));
            }
        }
        if !matches!(p("path"), Param::Static(Value::Text(path)) if path.is_empty()) {
            return Err(ProgramError::unsupported(&clip.id, "a motion path"));
        }
        if let Param::Static(Value::Enum(blend)) | Param::Static(Value::Text(blend)) = p("blend") {
            if blend != "normal" {
                return Err(ProgramError::unsupported(
                    &clip.id,
                    format!("blend `{blend}`"),
                ));
            }
        }
        let mut filters = Vec::new();
        let crop = ["crop_l", "crop_r", "crop_t", "crop_b"].map(&p);
        if crop.iter().any(|param| matches!(param, Param::Animated(_))) {
            return Err(ProgramError::unsupported(&clip.id, "animated crop"));
        }
        let [left, right, top, bottom] = crop.map(|param| number(&param, 0.0));
        if left + right >= 1.0 || top + bottom >= 1.0 {
            return Err(ProgramError::Invalid(format!(
                "clip `{}` is cropped away entirely",
                clip.id
            )));
        }
        if [left, right, top, bottom].iter().any(|edge| *edge != 0.0) {
            filters.push(format!(
                "crop=w='max(2,trunc(iw*{}/2)*2)':h='max(2,trunc(ih*{}/2)*2)':x='trunc(iw*{}/2)*2':y='trunc(ih*{}/2)*2'",
                fmt(1.0 - left - right),
                fmt(1.0 - top - bottom),
                fmt(left),
                fmt(top)
            ));
        }
        if fit {
            filters.push(format!(
                "scale={width}:{height}:force_original_aspect_ratio=decrease"
            ));
        }
        // Expressions in the clip's own time: `t` before placement counts
        // from the first decoded frame.
        let local = clip_time("t", portion.offset(clip));
        let (scale, scale_x, scale_y) = (p("scale"), p("scale_x"), p("scale_y"));
        let factor = |axis: &Param| -> String {
            match (&scale, axis) {
                (Param::Static(Value::Number(s)), Param::Static(Value::Number(a))) => {
                    fmt((s * a).abs())
                }
                _ => format!(
                    "abs(({})*({}))",
                    expression(&scale, &local),
                    expression(axis, &local)
                ),
            }
        };
        let (fx, fy) = (factor(&scale_x), factor(&scale_y));
        if fx != "1" || fy != "1" {
            filters.push(format!(
                "scale=w='max(2,trunc(iw*({fx})/2)*2)':h='max(2,trunc(ih*({fy})/2)*2)':eval=frame"
            ));
        }
        if number(&scale, 1.0) * number(&scale_x, 1.0) < 0.0 {
            filters.push("hflip".into());
        }
        if number(&scale, 1.0) * number(&scale_y, 1.0) < 0.0 {
            filters.push("vflip".into());
        }
        let rotation = p("rotation");
        let rotates = !is_number(&rotation, 0.0);
        if rotates {
            filters.push(format!(
                "format=rgba,rotate='({})*PI/180':fillcolor=none",
                expression(&rotation, &local)
            ));
        }
        // Overlay expressions run on the window: `t` is window time.
        let placed = clip_time("t", window - clip.start);
        let (x, y) = (p("x"), p("y"));
        let identity =
            !rotates && fx == "1" && fy == "1" && is_number(&x, 0.0) && is_number(&y, 0.0);
        Ok(Geometry {
            filters,
            alpha: rotates,
            identity,
            x: format!("(main_w-overlay_w)/2+main_w*({})", expression(&x, &placed)),
            y: format!("(main_h-overlay_h)/2+main_h*({})", expression(&y, &placed)),
        })
    }

    fn video_effects(&self, clip: &Clip) -> Result<Vec<String>> {
        let mut filters = Vec::new();
        for effect in clip.effects.iter().filter(|effect| effect.enabled) {
            let def = self.catalog.fx(effect.fx.as_str()).ok_or_else(|| {
                ProgramError::Invalid(format!("clip `{}` uses unknown effect", clip.id))
            })?;
            if def.engine == Engine::Raster {
                return Err(ProgramError::unsupported(
                    &clip.id,
                    format!(
                        "raster effect `{}` here (raster effects run on media clips)",
                        effect.fx
                    ),
                ));
            }
            let value = |name: &str| -> Result<f64> {
                match effect.params.get(name) {
                    None => Ok(def
                        .find(name)
                        .and_then(|param| param.default.as_number())
                        .unwrap_or(0.0)),
                    Some(Param::Static(Value::Number(n))) => Ok(*n),
                    Some(_) => Err(ProgramError::unsupported(
                        &clip.id,
                        format!("animated `{}` {name}", effect.fx),
                    )),
                }
            };
            match effect.fx.as_str() {
                "color.basic" => filters.extend(color_filters(&ColorParams {
                    exposure: value("exposure")?,
                    contrast: value("contrast")?,
                    brightness: value("brightness")?,
                    saturation: value("saturation")?,
                    temperature: value("temperature")?,
                    tint: value("tint")?,
                    gamma: value("gamma")?,
                    highlights: value("highlights")?,
                    shadows: value("shadows")?,
                    vibrance: value("vibrance")?,
                    mix: value("mix")?,
                })),
                "blur.gaussian" => {
                    let radius = value("radius")?;
                    if radius > 0.0 {
                        filters.push(format!("gblur=sigma={}", fmt(radius / 2.0)));
                    }
                }
                other => {
                    return Err(ProgramError::unsupported(
                        &clip.id,
                        format!("effect `{other}` on a picture"),
                    ))
                }
            }
        }
        Ok(filters)
    }

    /// Apply an adjustment layer's effects to `input` for the clip's span.
    fn adjustment(&mut self, input: String, clip: &Clip, from: Time, to: Time) -> Result<String> {
        let (lo, hi) = (clip.start.max(from), clip.end().min(to));
        if lo >= hi {
            return Ok(input);
        }
        let mut filters = self.video_effects(clip)?;
        if filters.is_empty() {
            return Ok(input);
        }
        for name in [
            "x", "y", "scale", "scale_x", "scale_y", "rotation", "crop_l", "crop_r", "crop_t",
            "crop_b",
        ] {
            let default = if name.starts_with("scale") { 1.0 } else { 0.0 };
            if !is_number(&self.param(clip, Domain::Video, name), default) {
                return Err(ProgramError::unsupported(
                    &clip.id,
                    format!("`{name}` on an adjustment layer"),
                ));
            }
        }
        filters.push("format=rgba".into());
        let opacity = self.param(clip, Domain::Video, "opacity");
        match &opacity {
            Param::Static(Value::Number(value)) if *value != 1.0 => {
                filters.push(format!("colorchannelmixer=aa={}", fmt(*value)));
            }
            Param::Animated(_) => filters.push(format!(
                "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*({})'",
                expression(&opacity, &clip_time("T", lo - clip.start))
            )),
            _ => {}
        }
        let (keep, process, adjusted, out) = (
            self.label("keep"),
            self.label("adjust"),
            self.label("adjusted"),
            self.label("layer"),
        );
        let (start, end) = (secs(lo - from), secs(hi - from));
        self.parts
            .push(format!("[{input}]split[{keep}][{process}]"));
        self.parts.push(format!(
            "[{process}]trim=start={start}:end={end},setpts=PTS-STARTPTS,{},setpts=PTS+{start}/TB[{adjusted}]",
            filters.join(",")
        ));
        self.parts.push(format!(
            "[{keep}][{adjusted}]overlay=eof_action=pass:shortest=0:format=auto:enable='between(t,{start},{end})'[{out}]"
        ));
        Ok(out)
    }

    fn dip(
        &mut self,
        sequence: &Sequence,
        input: String,
        dip: &Dip,
        from: Time,
        to: Time,
    ) -> String {
        let (lo, hi) = (dip.from.max(from), dip.to.min(to));
        if lo >= hi {
            return input;
        }
        let (width, height) = canvas(sequence);
        let duration = dip.to - dip.from;
        let half = secs(Time::from_flicks(duration.flicks() / 2));
        let (layer, out) = (self.label("dip"), self.label("layer"));
        self.parts.push(format!(
            "color=c={}:s={width}x{height}:r={}:d={},format=rgba,fade=t=in:st=0:d={half}:alpha=1,fade=t=out:st={half}:d={half}:alpha=1,trim=start={}:duration={},setpts=PTS-STARTPTS+{}/TB[{layer}]",
            ffmpeg_color(dip.color),
            rate_arg(self.rate),
            secs(duration),
            secs(lo - dip.from),
            secs(hi - lo),
            secs(lo - from),
        ));
        self.parts.push(format!(
            "[{input}][{layer}]overlay=eof_action=pass:shortest=0:format=auto:enable='between(t,{},{})'[{out}]",
            secs(lo - from),
            secs(hi - from)
        ));
        out
    }

    /// Plain cuts of full-frame media on one track become one concatenation
    /// instead of a stack of overlays.
    fn concat_program(
        &mut self,
        sequence: &'a Sequence,
        from: Time,
        to: Time,
    ) -> Result<Option<String>> {
        let mut visible = sequence.tracks.iter().filter(|track| {
            track.kind == TrackKind::Video
                && track.enabled
                && track
                    .clips
                    .iter()
                    .any(|clip| clip.enabled && clip.start < to && clip.end() > from)
        });
        let (Some(track), None) = (visible.next(), visible.next()) else {
            return Ok(None);
        };
        if !track.transitions.is_empty() {
            return Ok(None);
        }
        let plain = |clip: &Clip| {
            let p = |name: &str| self.param(clip, Domain::Video, name);
            matches!(&clip.content, Content::Media { media, .. }
                if self.project.media(media.as_str()).is_some_and(|m| m.video.is_some()))
                && ["x", "y", "rotation", "anchor_x", "anchor_y"]
                    .iter()
                    .all(|name| is_number(&p(name), 0.0))
                && ["scale", "scale_x", "scale_y", "opacity"]
                    .iter()
                    .all(|name| is_number(&p(name), 1.0))
                && matches!(p("blend"), Param::Static(Value::Enum(ref b)) if b == "normal")
        };
        let clips: Vec<&Clip> = track
            .clips
            .iter()
            .filter(|clip| clip.enabled && clip.start < to && clip.end() > from)
            .collect();
        if clips.len() < 2 || !clips.iter().all(|clip| plain(clip)) {
            return Ok(None);
        }
        let (width, height) = canvas(sequence);
        let mut segments = Vec::new();
        let mut cursor = from;
        for clip in clips {
            let item = Shown {
                clip,
                from: clip.start,
                to: clip.end(),
                fade_in: None,
                fade_out: None,
            };
            let Some(portion) = self.portion(&item, from, to) else {
                continue;
            };
            if portion.lo > cursor {
                segments.push(self.black(width, height, portion.lo - cursor));
            }
            let layer = self
                .picture(sequence, &item, &portion, portion.lo)?
                .expect("plain clips have a picture");
            segments.push(layer.label);
            cursor = portion.hi;
        }
        if cursor < to {
            segments.push(self.black(width, height, to - cursor));
        }
        let out = self.label("concat");
        self.parts.push(format!(
            "{}concat=n={}:v=1:a=0[{out}]",
            segments
                .iter()
                .map(|segment| format!("[{segment}]"))
                .collect::<String>(),
            segments.len()
        ));
        Ok(Some(out))
    }

    fn black(&mut self, width: u32, height: u32, duration: Time) -> String {
        let label = self.label("gap");
        self.parts.push(format!(
            "color=c=black:s={width}x{height}:r={}:d={},format=yuv420p,setsar=1[{label}]",
            rate_arg(self.rate),
            secs(duration)
        ));
        label
    }

    // ---- sound -------------------------------------------------------------------

    /// The sound of a sequence window: stereo at [`SAMPLE_RATE`], starting at 0.
    fn mix(
        &mut self,
        sequence: &'a Sequence,
        from: Time,
        to: Time,
        angle: Option<&Id>,
    ) -> Result<String> {
        let duration = to - from;
        let bed = self.label("bed");
        self.parts.push(format!(
            "aevalsrc=0:c=stereo:s={SAMPLE_RATE}:d={}[{bed}]",
            secs(duration)
        ));
        let mut inputs = vec![bed];
        // What feeds each bus, by its id.
        let mut feeding: HashMap<Id, Vec<String>> = HashMap::new();
        for track in &sequence.tracks {
            if !heard(track, angle) {
                continue;
            }
            let (items, _) = self.shown(track, sequence.rate)?;
            for item in &items {
                if !item.clip.enabled {
                    continue;
                }
                let Some(portion) = self.portion(item, from, to) else {
                    continue;
                };
                if let Some(label) = self.sound(item, &portion, from, duration)? {
                    match &track.bus {
                        Some(bus) => feeding.entry(bus.clone()).or_default().push(label),
                        None => inputs.push(label),
                    }
                }
            }
        }
        for bus in sequence.buses_in_order() {
            let Some(fed) = feeding.remove(&bus.id) else {
                continue;
            };
            let bed = self.label("bed");
            self.parts.push(format!(
                "aevalsrc=0:c=stereo:s={SAMPLE_RATE}:d={}[{bed}]",
                secs(duration)
            ));
            let label = self.label("bus");
            let filters = bus_filters(self.catalog, bus, from)?;
            self.parts.push(format!(
                "[{bed}]{}amix=inputs={}:duration=first:dropout_transition=0:normalize=0,asetpts=N/SR/TB{}{}[{label}]",
                fed.iter().map(|input| format!("[{input}]")).collect::<String>(),
                fed.len() + 1,
                if filters.is_empty() { "" } else { "," },
                filters,
            ));
            match &bus.to {
                Some(to) => feeding.entry(to.clone()).or_default().push(label),
                None => inputs.push(label),
            }
        }
        let mixed = self.label("mix");
        self.parts.push(format!(
            "{}amix=inputs={}:duration=first:dropout_transition=0:normalize=0,atrim=duration={},asetpts=PTS-STARTPTS[{mixed}]",
            inputs.iter().map(|input| format!("[{input}]")).collect::<String>(),
            inputs.len(),
            secs(duration)
        ));
        Ok(mixed)
    }

    fn sound(
        &mut self,
        item: &Shown,
        portion: &Portion,
        window: Time,
        total: Time,
    ) -> Result<Option<String>> {
        let clip = item.clip;
        let input = match &clip.content {
            Content::Media { media, .. } => {
                let media = self.project.media(media.as_str()).ok_or_else(|| {
                    ProgramError::Invalid(format!("clip `{}` uses missing media", clip.id))
                })?;
                if media.audio.is_none() {
                    return Ok(None);
                }
                let (start, end) = Self::source_window(clip, portion.decode, portion.hi)
                    .expect("media clips have a source");
                let index = self.input(&media.path, start, end - start);
                format!("{index}:a")
            }
            Content::Sequence {
                sequence: nested, ..
            } => {
                let nested = self.nested(clip, nested)?;
                let (start, end) = Self::source_window(clip, portion.decode, portion.hi)
                    .expect("nested clips have a source");
                self.nesting.push(nested.id.clone());
                let label = self.mix(nested, start, end, clip.angle.as_ref())?;
                self.nesting.pop();
                label
            }
            Content::Generator { .. } | Content::Caption { .. } => return Ok(None),
        };
        let filters = sound_filters(
            self.catalog,
            clip,
            Crossfade::of(item.fade_in),
            Crossfade::of(item.fade_out),
            portion,
        )?;
        let label = self.label("sound");
        self.parts.push(format!(
            "[{input}]{filters},{}[{label}]",
            place(samples(portion.lo - window), total)
        ));
        Ok(Some(label))
    }

    // ---- parameters ------------------------------------------------------------

    /// A clip property, or its catalog default.
    fn param(&self, clip: &Clip, domain: Domain, name: &str) -> Param {
        param(self.catalog, clip, domain, name)
    }

    fn static_number(&self, transition: &Transition, fx: &str, name: &str) -> Result<f64> {
        match transition.params.get(name) {
            None => Ok(self
                .catalog
                .fx(fx)
                .and_then(|def| def.find(name))
                .and_then(|def| def.default.as_number())
                .unwrap_or(0.0)),
            Some(Param::Static(Value::Number(n))) => Ok(*n),
            Some(_) => Err(ProgramError::unsupported(
                &transition.id,
                format!("animated `{name}`"),
            )),
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
                    .map(|d| &d.default)
                {
                    Some(Value::Color(color)) => *color,
                    _ => Rgba::BLACK,
                },
            ),
            Some(Param::Static(Value::Color(color))) => Ok(*color),
            Some(_) => Err(ProgramError::unsupported(
                entity,
                format!("animated `{name}`"),
            )),
        }
    }

    fn choice(&self, params: &kerf_model::Params, fx: &str, name: &str) -> String {
        match params.get(name) {
            Some(Param::Static(Value::Enum(choice))) => choice.clone(),
            _ => match self
                .catalog
                .fx(fx)
                .and_then(|def| def.find(name))
                .map(|d| &d.default)
            {
                Some(Value::Enum(choice)) => choice.clone(),
                _ => String::new(),
            },
        }
    }
}

/// How a clip's sound fades across a crossfade, over `[start, end)` of its
/// sequence.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Crossfade {
    pub start: Time,
    pub end: Time,
    /// An equal-power curve rather than a linear one.
    pub equal_power: bool,
}

impl Crossfade {
    fn of(fade: Option<(Reveal, Time, Time)>) -> Option<Self> {
        match fade? {
            (Reveal::Crossfade { equal_power }, start, end) => Some(Self {
                start,
                end,
                equal_power,
            }),
            _ => None,
        }
    }

    fn reveal(self) -> (Reveal, Time, Time) {
        (
            Reveal::Crossfade {
                equal_power: self.equal_power,
            },
            self.start,
            self.end,
        )
    }
}

/// The filters that make one audio clip's sound, as a render mixes it,
/// before it is placed on its sequence.
///
/// The clip is decoded from `portion.decode`, and `[portion.lo,
/// portion.hi)` of it is heard: the chain outputs stereo at
/// [`SAMPLE_RATE`] starting at `lo`. A media clip's chain reads its
/// decoded samples timed from the first one; a nested sequence's reads the
/// sequence's mix from the time decoded. `fade_in` and `fade_out` are the
/// crossfades it plays through.
/// Whether `track` is heard in a mix: as it says, unless a clip holding its sequence names one track,
/// which is heard even when muted, as the sound of a camera not in use may be.
fn heard(track: &kerf_model::Track, angle: Option<&Id>) -> bool {
    if track.kind != TrackKind::Audio {
        return false;
    }
    match angle {
        Some(angle) => track.id == *angle,
        None => track.enabled && !track.muted,
    }
}

pub fn sound_filters(
    catalog: &Catalog,
    clip: &Clip,
    fade_in: Option<Crossfade>,
    fade_out: Option<Crossfade>,
    portion: &Portion,
) -> Result<String> {
    let mut filters = Vec::new();
    if let Content::Media { .. } = clip.content {
        let (start, end) = Graph::source_window(clip, portion.decode, portion.hi)
            .expect("media clips have a source");
        filters.push(format!("atrim=start=0:end={}", secs(end - start)));
        filters.push("asetpts=PTS-STARTPTS".into());
        filters.push(format!(
            "aformat=sample_fmts=fltp:sample_rates={SAMPLE_RATE}:channel_layouts=stereo"
        ));
    }
    let speed = clip.speed();
    if speed.is_reverse() {
        filters.push("areverse".into());
    }
    // The sound of a ramp is made beforehand (`kerf_compose::ramps`), which leaves it at speed 1. One
    // that could not be keeps the length it should have, at its speed as one number.
    filters.extend(atempo_filters(speed.average(clip.duration)));
    let local = clip_time("t", portion.offset(clip));
    match param(catalog, clip, Domain::Audio, "gain") {
        Param::Static(Value::Number(gain)) if gain != 0.0 => {
            filters.push(format!("volume={}dB", fmt(gain)));
        }
        gain @ Param::Animated(_) => filters.push(format!(
            "volume='pow(10,({})/20)':eval=frame",
            expression(&gain, &local)
        )),
        _ => {}
    }
    match param(catalog, clip, Domain::Audio, "pan") {
        Param::Static(Value::Number(pan)) if pan != 0.0 => filters.push(format!(
            "pan=stereo|c0={}*c0|c1={}*c1",
            fmt((1.0 - pan).min(1.0)),
            fmt((1.0 + pan).min(1.0))
        )),
        Param::Animated(_) => {
            return Err(ProgramError::unsupported(&clip.id, "animated pan"));
        }
        _ => {}
    }
    filters.extend(audio_effects(catalog, clip)?);
    for (direction, fade) in [("in", fade_in), ("out", fade_out)] {
        let Some(Crossfade {
            start,
            end,
            equal_power,
        }) = fade
        else {
            continue;
        };
        // A fade in over before the first decoded sample leaves it all
        // heard.
        if direction == "in" && end <= portion.decode {
            continue;
        }
        filters.push(format!(
            "afade=t={direction}:st={}:d={}:curve={}",
            secs(start - portion.decode),
            secs(end - start),
            if equal_power { "qsin" } else { "tri" }
        ));
    }
    filters.push(format!(
        "atrim=start={}:duration={}",
        secs(portion.lo - portion.decode),
        secs(portion.hi - portion.lo)
    ));
    filters.push("asetpts=PTS-STARTPTS".into());
    Ok(filters.join(","))
}

/// The filters that start a sound `delay` samples into a stream `total`
/// long.
fn place(delay: i64, total: Time) -> String {
    // `adelay` leaves the silence it adds before its first input frame
    // untimed, and times what follows from there, which `atrim` misreads
    // as the stream's end: count the samples instead.
    let delay = if delay > 0 {
        format!("adelay=delays={delay}S:all=1,asetpts=N/SR/TB,")
    } else {
        String::new()
    };
    format!(
        "{delay}apad,atrim=duration={},asetpts=PTS-STARTPTS",
        secs(total)
    )
}

/// The filters a clip's sound effects lower to, in order. A render and
/// playback both run these.
/// A bus's filters, for a stream timed from `from` on its sequence.
pub fn bus_filters(catalog: &Catalog, bus: &kerf_model::Bus, from: Time) -> Result<String> {
    let mut filters = audio_effects_of(catalog, &bus.id, &bus.effects)?;
    let local = clip_time("t", from);
    let number = |name: &str| -> Param {
        bus.params.get(name).cloned().unwrap_or_else(|| {
            Param::Static(
                catalog
                    .prop(Domain::Audio, name)
                    .map(|def| def.default.clone())
                    .unwrap_or(Value::Number(0.0)),
            )
        })
    };
    match number("gain") {
        Param::Static(Value::Number(gain)) if gain != 0.0 => {
            filters.push(format!("volume={}dB", fmt(gain)));
        }
        gain @ Param::Animated(_) => filters.push(format!(
            "volume='pow(10,({})/20)':eval=frame",
            expression(&gain, &local)
        )),
        _ => {}
    }
    match number("pan") {
        Param::Static(Value::Number(pan)) if pan != 0.0 => filters.push(format!(
            "pan=stereo|c0={}*c0|c1={}*c1",
            fmt((1.0 - pan).min(1.0)),
            fmt((1.0 + pan).min(1.0))
        )),
        Param::Animated(_) => {
            return Err(ProgramError::unsupported(&bus.id, "animated pan"));
        }
        _ => {}
    }
    Ok(filters.join(","))
}

fn audio_effects(catalog: &Catalog, clip: &Clip) -> Result<Vec<String>> {
    audio_effects_of(catalog, &clip.id, &clip.effects)
}

fn audio_effects_of(
    catalog: &Catalog,
    owner: &Id,
    effects: &[kerf_model::Effect],
) -> Result<Vec<String>> {
    let mut filters = Vec::new();
    for effect in effects.iter().filter(|effect| effect.enabled) {
        let value = |name: &str| -> Result<f64> {
            match effect.params.get(name) {
                None => Ok(catalog
                    .fx(effect.fx.as_str())
                    .and_then(|fx| fx.find(name))
                    .and_then(|def| match def.default {
                        Value::Number(n) => Some(n),
                        _ => None,
                    })
                    .unwrap_or(0.0)),
                Some(Param::Static(Value::Number(n))) => Ok(*n),
                Some(_) => Err(ProgramError::unsupported(
                    owner,
                    format!("animated `{}` {name}", effect.fx),
                )),
            }
        };
        match effect.fx.as_str() {
            "audio.eq3" => {
                let (low, mid, high) = (value("low")?, value("mid")?, value("high")?);
                if low != 0.0 {
                    filters.push(format!("bass=g={}:f=250", fmt(low)));
                }
                if mid != 0.0 {
                    filters.push(format!("equalizer=f=1000:t=o:w=1:g={}", fmt(mid)));
                }
                if high != 0.0 {
                    filters.push(format!("treble=g={}:f=4000", fmt(high)));
                }
            }
            "audio.eq" => {
                let (lowcut, presence) = (value("lowcut_hz")?, value("presence_db")?);
                if lowcut > 0.0 {
                    filters.push(format!("highpass=f={}", fmt(lowcut)));
                }
                if presence != 0.0 {
                    filters.push(format!("equalizer=f=3000:t=q:w=1:g={}", fmt(presence)));
                }
            }
            "audio.limiter" => {
                // `alimiter` takes the limit as a linear level. Its
                // auto-level, which would raise everything back up to the
                // limit, is off, and it makes up for its look-ahead so the
                // sound stays on the picture.
                let (limit, release) = (value("limit_db")?, value("release_ms")?);
                let level = 10f64.powf(limit.clamp(-24.0, 0.0) / 20.0);
                filters.push(format!(
                    "alimiter=limit={}:release={}:level=false:latency=true",
                    fmt(level),
                    fmt(release.clamp(1.0, 8000.0))
                ));
            }
            "audio.compressor" => {
                let level = |db: f64| fmt(10f64.powf(db / 20.0));
                filters.push(format!(
                    "acompressor=threshold={}:ratio={}:attack={}:release={}:makeup={}",
                    level(value("threshold_db")?.clamp(-60.0, 0.0)),
                    fmt(value("ratio")?.clamp(1.0, 20.0)),
                    fmt(value("attack_ms")?.clamp(0.5, 500.0)),
                    fmt(value("release_ms")?.clamp(10.0, 2000.0)),
                    level(value("makeup_db")?.clamp(0.0, 24.0)),
                ));
            }
            "audio.gate" => {
                filters.push(format!(
                    "agate=threshold={}:ratio={}:attack={}:release={}",
                    fmt(10f64.powf(value("threshold_db")?.clamp(-80.0, 0.0) / 20.0)),
                    fmt(value("ratio")?.clamp(1.0, 20.0)),
                    fmt(value("attack_ms")?.clamp(0.5, 500.0)),
                    fmt(value("release_ms")?.clamp(10.0, 2000.0)),
                ));
            }
            "audio.deesser" => {
                filters.push(format!("deesser=i={}", fmt(value("amount")?.clamp(0.0, 1.0))));
            }
            "audio.denoise" => {
                filters.push(format!(
                    "afftdn=nr={}:nf={}",
                    fmt(value("reduction_db")?.clamp(0.01, 60.0)),
                    fmt(value("noise_db")?.clamp(-80.0, -20.0))
                ));
            }
            "audio.delay" => {
                let time = value("time_ms")?.clamp(1.0, 5000.0);
                let feedback = value("feedback")?.clamp(0.0, 0.9);
                let mix = value("mix")?.clamp(0.0, 1.0);
                // Each echo as far again and as much softer, as many as are still worth hearing.
                let echoes: Vec<(f64, f64)> = (1..=6)
                    .map(|index| (time * f64::from(index), feedback.powi(index) * mix))
                    .filter(|(at, level)| *at <= 90_000.0 && *level >= 0.01)
                    .collect();
                if !echoes.is_empty() {
                    let list = |pick: fn(&(f64, f64)) -> f64| {
                        echoes
                            .iter()
                            .map(|echo| fmt(pick(echo)))
                            .collect::<Vec<_>>()
                            .join("|")
                    };
                    filters.push(format!(
                        "aecho=in_gain=1:out_gain=1:delays={}:decays={}",
                        list(|echo| echo.0),
                        list(|echo| echo.1)
                    ));
                }
            }
            "audio.reverb" => {
                let size = value("size")?.clamp(0.0, 1.0);
                let mix = value("mix")?.clamp(0.0, 1.0);
                if mix > 0.0 {
                    // Close reflections at times that share no factor, so that they do not ring.
                    let base = 12.0 + size * 60.0;
                    let steps = [1.0, 1.7, 2.3, 3.1, 4.3, 5.9];
                    let delays: Vec<String> =
                        steps.iter().map(|step| fmt(base * step)).collect();
                    let decays: Vec<String> = steps
                        .iter()
                        .map(|step| fmt(mix * (0.8 + size * 0.15).powf(step * 2.0)))
                        .collect();
                    filters.push(format!(
                        "aecho=in_gain=1:out_gain=1:delays={}:decays={}",
                        delays.join("|"),
                        decays.join("|")
                    ));
                }
            }
            other => {
                return Err(ProgramError::unsupported(
                    owner,
                    format!("effect `{other}` on sound"),
                ))
            }
        }
    }
    Ok(filters)
}

/// A clip property, or its catalog default.
fn param(catalog: &Catalog, clip: &Clip, domain: Domain, name: &str) -> Param {
    clip.props.get(name).cloned().unwrap_or_else(|| {
        Param::Static(
            catalog
                .prop(domain, name)
                .map(|def| def.default.clone())
                .unwrap_or(Value::Number(0.0)),
        )
    })
}

enum Source {
    /// A labelled picture stream, fitted to the canvas.
    Stream(String),
    /// A source filter that generates the picture.
    Generated(String),
}

struct Layer {
    label: String,
    x: String,
    y: String,
}

struct Geometry {
    filters: Vec<String>,
    alpha: bool,
    identity: bool,
    x: String,
    y: String,
}

fn ffmpeg_color(color: Rgba) -> String {
    let base = format!("0x{:02x}{:02x}{:02x}", color.r, color.g, color.b);
    if color.a == 255 {
        base
    } else {
        format!("{base}@{}", fmt(f64::from(color.a) / 255.0))
    }
}

/// A clip-time expression: `variable` plus `offset` seconds.
fn clip_time(variable: &str, offset: Time) -> String {
    if offset == Time::ZERO {
        variable.to_owned()
    } else if offset.is_negative() {
        format!("({variable}-{})", secs(-offset))
    } else {
        format!("({variable}+{})", secs(offset))
    }
}

fn number(param: &Param, default: f64) -> f64 {
    match param {
        Param::Static(Value::Number(n)) => *n,
        _ => default,
    }
}

fn is_number(param: &Param, value: f64) -> bool {
    matches!(param, Param::Static(Value::Number(n)) if *n == value)
}

/// How many straight pieces a curve becomes in an FFmpeg expression, which cannot solve a cubic.
const CURVE_PIECES: usize = 16;

/// Where a curve is at each end of its [`CURVE_PIECES`] pieces.
fn curve_points(curve: Bezier) -> Vec<f64> {
    (0..=CURVE_PIECES)
        .map(|piece| curve.at(piece as f64 / CURVE_PIECES as f64))
        .collect()
}

/// A curve as an FFmpeg expression of `u`, in straight pieces.
fn curve_expression(curve: Bezier, u: &str) -> String {
    let points = curve_points(curve);
    let mut expr = fmt(points[CURVE_PIECES]);
    for piece in (0..CURVE_PIECES).rev() {
        let (low, high) = (points[piece], points[piece + 1]);
        let across = format!("({u}*{CURVE_PIECES}-{piece})");
        expr = format!(
            "if(lt({u},{}),({}+({})*{across}),{expr})",
            fmt((piece + 1) as f64 / CURVE_PIECES as f64),
            fmt(low),
            fmt(high - low)
        );
    }
    format!("({expr})")
}

/// A numeric parameter as an FFmpeg expression of clip time `tau`, with
/// the same interpolation curves as `Param::value_at`.
fn expression(param: &Param, tau: &str) -> String {
    let keys = match param {
        Param::Static(value) => return fmt(value.as_number().unwrap_or(0.0)),
        Param::Animated(keys) => keys,
    };
    let value = |index: usize| keys[index].value.as_number().unwrap_or(0.0);
    let last = keys.len() - 1;
    let mut expr = fmt(value(last));
    for index in (0..last).rev() {
        let (left, right) = (&keys[index], &keys[index + 1]);
        let u = format!("(({tau}-{})/{})", secs(left.at), secs(right.at - left.at));
        let curve = match left.interp {
            Interp::Linear => u,
            Interp::Hold => "0".into(),
            Interp::EaseIn => format!("({u}*{u})"),
            Interp::EaseOut => format!("(1-(1-{u})*(1-{u}))"),
            Interp::Ease => format!("({u}*{u}*(3-2*{u}))"),
            Interp::Bezier(curve) => curve_expression(curve, &u),
        };
        let segment = format!(
            "({}+({})*{curve})",
            fmt(value(index)),
            fmt(value(index + 1) - value(index))
        );
        expr = format!("if(lt({tau},{}),{segment},{expr})", secs(right.at));
    }
    format!(
        "if(lt({tau},{}),{},{expr})",
        secs(keys[0].at),
        fmt(value(0))
    )
}

fn reveal_filter(reveal: Reveal, start: f64, duration: f64) -> String {
    let progress = format!("clip((T-{})/{},0,1)", fmt(start), fmt(duration));
    match reveal {
        Reveal::Dissolve | Reveal::Crossfade { .. } => {
            format!("fade=t=in:st={}:d={}:alpha=1", fmt(start), fmt(duration))
        }
        Reveal::Wipe { angle, softness } => {
            let (dy, dx) = angle.to_radians().sin_cos();
            let low = dx.min(0.0) + dy.min(0.0);
            let span = (dx.max(0.0) + dy.max(0.0) - low).max(1e-6);
            let soft = softness.max(0.001);
            let position = format!(
                "((X/W)*{}+(Y/H)*{}-({}))/{}",
                fmt(dx),
                fmt(dy),
                fmt(low),
                fmt(span)
            );
            format!(
                "geq=r='r(X,Y)':g='g(X,Y)':b='b(X,Y)':a='alpha(X,Y)*clip(({progress}*{}-{position})/{},0,1)'",
                fmt(1.0 + soft),
                fmt(soft)
            )
        }
    }
}

struct ColorParams {
    exposure: f64,
    contrast: f64,
    brightness: f64,
    saturation: f64,
    temperature: f64,
    tint: f64,
    gamma: f64,
    highlights: f64,
    shadows: f64,
    vibrance: f64,
    mix: f64,
}

/// `color.basic`, with `mix` scaling every correction toward identity.
fn color_filters(params: &ColorParams) -> Vec<String> {
    let identity = |value: f64, base: f64| (value - base).abs() <= 1e-9;
    let mix = params.mix;
    let mut filters = Vec::new();
    if mix == 0.0 {
        return filters;
    }
    let exposure = params.exposure * mix;
    if !identity(exposure, 0.0) {
        filters.push(format!("exposure=exposure={}", fmt(exposure)));
    }
    let mut eq = Vec::new();
    for (name, value, base) in [
        ("contrast", 1.0 + (params.contrast - 1.0) * mix, 1.0),
        ("brightness", params.brightness * mix, 0.0),
        ("saturation", 1.0 + (params.saturation - 1.0) * mix, 1.0),
        ("gamma", 1.0 + (params.gamma - 1.0) * mix, 1.0),
    ] {
        if !identity(value, base) {
            eq.push(format!("{name}={}", fmt(value)));
        }
    }
    if !eq.is_empty() {
        filters.push(format!("eq={}", eq.join(":")));
    }
    let balance = |value: f64| value.clamp(-1.0, 1.0);
    let temperature = params.temperature / 1000.0 * mix;
    let (tint, shadows, highlights) = (
        params.tint * mix,
        params.shadows * mix,
        params.highlights * mix,
    );
    let channels = [
        balance(temperature + tint + shadows),
        balance(-tint + shadows),
        balance(-temperature + tint + shadows),
        balance(highlights),
        balance(highlights),
        balance(highlights),
    ];
    if channels.iter().any(|value| !identity(*value, 0.0)) {
        let [rs, gs, bs, rh, gh, bh] = channels.map(fmt);
        filters.push(format!(
            "colorbalance=rs={rs}:gs={gs}:bs={bs}:rh={rh}:gh={gh}:bh={bh}:pl=1"
        ));
    }
    let vibrance = params.vibrance * mix;
    if !identity(vibrance, 0.0) {
        filters.push(format!("vibrance=intensity={}", fmt(vibrance)));
    }
    filters
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod sound_effect_tests {
    use super::*;
    use kerf_lang::parse_document;

    /// The one clip of a document whose audio clip carries `effects`.
    fn clip_with(effects: &str) -> Clip {
        let text = format!(
            "kerf 1\nproject name=\"t\"\n\
             media m1 path=\"/media/a.wav\" rate=24 dur=00:01:00:00 audio=2ch@48000\n\
             sequence s1 rate=24 size=1920x1080 audio=48000\n\
             \x20 track A1 audio\n\
             \x20   clip a1 media=m1 src=0f at=0f dur=24f\n\
             {effects}"
        );
        let project = parse_document(&text, Catalog::builtin())
            .unwrap_or_else(|errors| panic!("{errors:?}"))
            .project;
        project.clip("a1").expect("a1").clone()
    }

    fn lowered(effects: &str) -> Vec<String> {
        audio_effects(Catalog::builtin(), &clip_with(effects)).expect("lowers")
    }

    #[test]
    fn channel_eq_lowers_to_a_high_pass_and_a_presence_band() {
        assert_eq!(
            lowered("      fx audio.eq lowcut_hz=80 presence_db=1.5\n"),
            ["highpass=f=80", "equalizer=f=3000:t=q:w=1:g=1.5"]
        );
        // A low cut at 0 is off; so is a flat presence band.
        assert_eq!(
            lowered("      fx audio.eq presence_db=-3\n"),
            ["equalizer=f=3000:t=q:w=1:g=-3"]
        );
        assert_eq!(
            lowered("      fx audio.eq lowcut_hz=120\n"),
            ["highpass=f=120"]
        );
        assert!(lowered("      fx audio.eq\n").is_empty());
    }

    #[test]
    fn the_limiter_lowers_to_alimiter_at_a_linear_limit() {
        assert_eq!(
            lowered("      fx audio.limiter limit_db=-6 release_ms=200\n"),
            ["alimiter=limit=0.501187:release=200:level=false:latency=true"]
        );
        // Unset parameters take the catalog defaults: -1 dBFS and 50 ms.
        assert_eq!(
            lowered("      fx audio.limiter\n"),
            ["alimiter=limit=0.891251:release=50:level=false:latency=true"]
        );
        // Effects chain in order, and a disabled one lowers to nothing.
        let mut clip =
            clip_with("      fx audio.eq lowcut_hz=80\n      fx audio.limiter limit_db=-2\n");
        assert_eq!(
            audio_effects(Catalog::builtin(), &clip).unwrap(),
            [
                "highpass=f=80",
                "alimiter=limit=0.794328:release=50:level=false:latency=true"
            ]
        );
        clip.effects[1].enabled = false;
        assert_eq!(
            audio_effects(Catalog::builtin(), &clip).unwrap(),
            ["highpass=f=80"]
        );
    }

    #[test]
    fn animated_eq_and_limiter_parameters_are_refused() {
        let clip = clip_with("      fx audio.limiter limit_db={0f:-1 12f:-6}\n");
        let error = audio_effects(Catalog::builtin(), &clip).unwrap_err();
        assert!(error.to_string().contains("limit_db"), "{error}");
    }

    #[test]
    fn ffmpeg_accepts_the_lowered_filters() {
        let chain = lowered(
            "      fx audio.eq lowcut_hz=80 presence_db=1.5\n      fx audio.limiter limit_db=-6 release_ms=200\n",
        )
        .join(",");
        let run = std::process::Command::new("ffmpeg")
            .args(["-hide_banner", "-nostdin", "-f", "lavfi", "-i"])
            .arg("sine=frequency=440:sample_rate=48000:duration=0.2")
            .args(["-af", &chain, "-f", "null", "-"])
            .output();
        let Ok(output) = run else {
            eprintln!("skipped: ffmpeg is not on PATH");
            return;
        };
        assert!(
            output.status.success(),
            "ffmpeg refused `{chain}`: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

#[cfg(test)]
mod curve_tests {
    use super::*;

    #[test]
    fn straight_pieces_stay_close_to_the_curve() {
        for text in [
            "0.2,0.8,0.4,1",
            "0.7,0,0.3,1",
            "0.42,0,0.58,1",
            "0.3,1.6,0.7,1",
        ] {
            let Some(Interp::Bezier(curve)) = Interp::parse(text) else {
                panic!("{text}");
            };
            let points = curve_points(curve);
            for step in 0..=1000 {
                let t = f64::from(step) / 1000.0;
                let place = t * CURVE_PIECES as f64;
                let piece = (place as usize).min(CURVE_PIECES - 1);
                let line =
                    points[piece] + (points[piece + 1] - points[piece]) * (place - piece as f64);
                assert!((line - curve.at(t)).abs() < 0.02, "{text} at {t}");
            }
        }
    }

    #[test]
    fn an_expression_has_every_piece() {
        let Some(Interp::Bezier(curve)) = Interp::parse("0.2,0.8,0.4,1") else {
            panic!("a curve");
        };
        let expression = curve_expression(curve, "u");
        assert_eq!(expression.matches("if(lt(u,").count(), CURVE_PIECES);
        assert_eq!(
            expression.matches('(').count(),
            expression.matches(')').count()
        );
    }
}
