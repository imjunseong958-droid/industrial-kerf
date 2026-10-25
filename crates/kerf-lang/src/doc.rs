//! The `.kerf` document: the canonical, human- and agent-readable project.
//!
//! ```text
//! kerf 1
//! project name="Short Film"
//!
//! media m1 path="A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000
//!
//! sequence s1 name="Main" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
//!   track V1 video
//!     clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12
//!       set opacity=0.9
//!       fx color.basic@1 exposure=0.3
//!   marker mk1 at=01:00:05:00 name="pickup" color=red
//! ```
//!
//! Positions are timecode; durations count frames as `HH:MM:SS:FF`. Printing
//! is deterministic and omits defaults, so a document diff is an edit diff.

use crate::lex::{lex, Arg, Atom, Line};
use crate::text::{number, print_fx_ref, print_param, quote, Clock, Fields};
use kerf_model::time::parse_timecode;
use kerf_model::Align;
use kerf_model::{
    validate, AudioInfo, Catalog, Clip, Content, Diagnostic, Domain, Effect, FxDef, Id, Marker,
    MarkerColor, Mask, MaskMode, MaskShape, Media, Param, ParamDef, ParamType, Params, Project,
    Rate, Ratio, Sequence, Time, Track, TrackKind, Transition, Value, VideoInfo,
};
use std::collections::HashMap;
use std::fmt::Write as _;

pub const VERSION: u32 = 1;

/// A parsed, valid project and any warnings about it.
#[derive(Clone, Debug, PartialEq)]
pub struct Document {
    pub project: Project,
    pub warnings: Vec<Diagnostic>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PrintOptions {
    /// Append comments with end times and source outs, for agents and
    /// readers. Parsing ignores them.
    pub annotate: bool,
}

/// Parse and validate a document. Errors carry line and column.
pub fn parse_document(text: &str, catalog: &Catalog) -> Result<Document, Vec<Diagnostic>> {
    let lines = lex(text)?;
    let mut parser = Parser {
        catalog,
        project: Project::default(),
        lines: HashMap::new(),
        errors: Vec::new(),
    };
    parser.headers(&lines);
    if parser.errors.is_empty() {
        parser.bodies(&lines);
    }
    if !parser.errors.is_empty() {
        return Err(parser.errors);
    }
    let (errors, warnings): (Vec<_>, Vec<_>) = validate(&parser.project, catalog)
        .into_iter()
        .map(|diagnostic| {
            match diagnostic
                .entity
                .as_ref()
                .and_then(|id| parser.lines.get(id))
            {
                Some(line) => diagnostic.at(*line, 1),
                None => diagnostic,
            }
        })
        .partition(Diagnostic::is_error);
    if errors.is_empty() {
        Ok(Document {
            project: parser.project,
            warnings,
        })
    } else {
        Err(errors)
    }
}

struct Parser<'c> {
    catalog: &'c Catalog,
    project: Project,
    lines: HashMap<Id, usize>,
    errors: Vec<Diagnostic>,
}

type Step<T = ()> = Result<T, Diagnostic>;

fn keyword(line: &Line) -> &str {
    match line.args.first().map(|arg| (&arg.key, &arg.value)) {
        Some((None, Atom::Word(word))) => word,
        _ => "",
    }
}

impl Parser<'_> {
    fn record(&mut self, id: &Id, line: &Line) {
        self.lines.entry(id.clone()).or_insert(line.number);
    }

    /// First pass: the header, project, media, and sequence lines, so clips
    /// can resolve timecode against any media or sequence in the file.
    fn headers(&mut self, lines: &[Line]) {
        let Some(first) = lines.first() else {
            self.errors
                .push(Diagnostic::error("syntax", "empty document; start with `kerf 1`").at(1, 1));
            return;
        };
        if keyword(first) != "kerf" || first.args.len() != 2 || first.depth != 0 {
            self.errors
                .push(first.error(1, "a document starts with `kerf 1`"));
            return;
        }
        match &first.args[1].value {
            Atom::Word(version) if version == &VERSION.to_string() => {}
            _ => self.errors.push(first.error(
                first.args[1].column,
                format!("this reader understands `kerf {VERSION}`"),
            )),
        }
        for line in &lines[1..] {
            if line.depth != 0 {
                continue;
            }
            let step = match keyword(line) {
                "project" => self.project_line(line),
                "media" => self.media_line(line),
                "sequence" => self.sequence_line(line),
                "kerf" => Err(line.error(1, "`kerf` appears only once, on the first line")),
                other => Err(line.error(
                    1,
                    format!("expected project, media, or sequence, not `{other}`"),
                )),
            };
            if let Err(error) = step {
                self.errors.push(error);
            }
        }
    }

    fn project_line(&mut self, line: &Line) -> Step {
        let mut fields = Fields::new(line)?;
        if let Some(name) = fields.take_text("name")? {
            self.project.name = name.to_owned();
        }
        if let Some(retired) = fields.take_ids("retired")? {
            self.project.retired.extend(retired);
        }
        fields.finish(1)
    }

    fn media_line(&mut self, line: &Line) -> Step {
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a media id")?;
        let media = media_fields(&mut fields, id)?;
        fields.finish(2)?;
        self.record(&media.id, line);
        self.project.media.push(media);
        Ok(())
    }

    fn sequence_line(&mut self, line: &Line) -> Step {
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a sequence id")?;
        let sequence = sequence_fields(&mut fields, id)?;
        fields.finish(2)?;
        self.record(&sequence.id, line);
        self.project.sequences.push(sequence);
        Ok(())
    }

    /// Second pass: tracks, clips, transitions, markers, and clip details.
    fn bodies(&mut self, lines: &[Line]) {
        let mut sequence: Option<usize> = None;
        let mut track: Option<usize> = None;
        let mut clip: Option<usize> = None;
        let mut sequence_count = 0;
        for line in &lines[1..] {
            let result =
                match (line.depth, keyword(line)) {
                    (0, "sequence") => {
                        sequence = Some(sequence_count);
                        sequence_count += 1;
                        track = None;
                        clip = None;
                        Ok(())
                    }
                    (0, _) => {
                        sequence = None;
                        Ok(())
                    }
                    (1, word) => match sequence {
                        None => Err(line.error(1, "indented lines belong under a sequence")),
                        Some(s) => {
                            clip = None;
                            match word {
                                "track" => self.track_line(s, line).map(|t| track = Some(t)),
                                "marker" => {
                                    track = None;
                                    self.marker_line(s, line)
                                }
                                other => Err(line
                                    .error(1, format!("expected track or marker, not `{other}`"))),
                            }
                        }
                    },
                    (2, word) => match (sequence, track) {
                        (Some(s), Some(t)) => match word {
                            "clip" => self.clip_line(s, t, line).map(|c| clip = Some(c)),
                            "transition" => {
                                clip = None;
                                self.transition_line(s, t, line)
                            }
                            other => Err(line
                                .error(1, format!("expected clip or transition, not `{other}`"))),
                        },
                        _ => Err(line.error(1, "clips and transitions belong under a track")),
                    },
                    (3, word) => {
                        match (sequence, track, clip) {
                            (Some(s), Some(t), Some(c)) => match word {
                                "set" => self.set_line(s, t, c, line),
                                "mask" => self.mask_line(s, c, t, line),
                                "fx" => self.fx_line(s, t, c, line),
                                other => Err(line
                                    .error(1, format!("expected set, mask, or fx, not `{other}`"))),
                            },
                            _ => Err(line.error(1, "set, mask, and fx lines belong under a clip")),
                        }
                    }
                    _ => Err(line.error(1, "too deeply indented")),
                };
            if let Err(error) = result {
                self.errors.push(error);
            }
        }
    }

    fn track_line(&mut self, s: usize, line: &Line) -> Step<usize> {
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a track id")?;
        let kind_word = fields.word(2, "a track kind (video, audio, subtitle)")?;
        let kind = TrackKind::parse(kind_word).ok_or_else(|| {
            fields.error(
                fields.positional()[2],
                format!("`{kind_word}` is not video, audio, or subtitle"),
            )
        })?;
        let mut track = Track::new(id, kind);
        track.name = fields.take_text("name")?.map(str::to_owned);
        track.enabled = fields.take_bool("enabled")?.unwrap_or(true);
        track.locked = fields.take_bool("locked")?.unwrap_or(false);
        track.muted = fields.take_bool("muted")?.unwrap_or(false);
        fields.finish(3)?;
        let tracks = &mut self.project.sequences[s].tracks;
        tracks.push(track);
        Ok(tracks.len() - 1)
    }

    fn marker_line(&mut self, s: usize, line: &Line) -> Step {
        let clock = Clock::of_sequence(&self.project.sequences[s]);
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a marker id")?;
        let at = fields
            .take_with("at", |text| clock.position(text))?
            .ok_or_else(|| line.error(1, "missing `at=`"))?;
        let duration = fields
            .take_with("dur", |text| clock.duration(text))?
            .unwrap_or(Time::ZERO);
        let name = fields.take_text("name")?.map(str::to_owned);
        let color = fields.take_with("color", parse_marker_color)?;
        let note = fields.take_text("note")?.map(str::to_owned);
        fields.finish(2)?;
        self.record(&id, line);
        self.project.sequences[s].markers.push(Marker {
            id,
            at,
            duration,
            name,
            color,
            note,
        });
        Ok(())
    }

    fn clip_line(&mut self, s: usize, t: usize, line: &Line) -> Step<usize> {
        let clock = Clock::of_sequence(&self.project.sequences[s]);
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a clip id")?;
        let (content, _) = content(&mut fields, &self.project, self.catalog)?;
        let start = fields
            .take_with("at", |text| clock.position(text))?
            .ok_or_else(|| line.error(1, "missing `at=`"))?;
        let duration = fields
            .take_with("dur", |text| clock.duration(text))?
            .ok_or_else(|| line.error(1, "missing `dur=`"))?;
        let mut clip = Clip::new(id, content, start, duration);
        clip.name = fields.take_text("name")?.map(str::to_owned);
        clip.enabled = fields.take_bool("enabled")?.unwrap_or(true);
        clip.link = fields.take_id("link")?;
        fields.finish(2)?;
        self.record(&clip.id, line);
        if let Some(link) = &clip.link {
            self.record(link, line);
        }
        let clips = &mut self.project.sequences[s].tracks[t].clips;
        clips.push(clip);
        Ok(clips.len() - 1)
    }

    fn set_line(&mut self, s: usize, t: usize, c: usize, line: &Line) -> Step {
        let rate = self.project.sequences[s].rate;
        let domain = self.project.sequences[s].tracks[t].kind.domain();
        let mut fields = Fields::new(line)?;
        let catalog = self.catalog;
        let clip = &mut self.project.sequences[s].tracks[t].clips[c];
        for (name, arg) in fields.rest() {
            let domain = domain.ok_or_else(|| fields.error(arg, "captions carry no properties"))?;
            let (def, target) = match catalog.prop(domain, name) {
                Some(def) => (def, &mut clip.props),
                None => match &mut clip.content {
                    Content::Generator { fx, params, .. } => {
                        let generator = catalog.fx(fx.as_str());
                        match generator.and_then(|g| g.find(name)) {
                            Some(def) => (def, params),
                            None => {
                                return Err(unknown_param(
                                    &fields,
                                    arg,
                                    name,
                                    catalog.props(domain),
                                    generator.map(|g| g.params.as_slice()),
                                ))
                            }
                        }
                    }
                    _ => {
                        return Err(unknown_param(
                            &fields,
                            arg,
                            name,
                            catalog.props(domain),
                            None,
                        ))
                    }
                },
            };
            let param = crate::text::parse_param(&arg.value, def, rate)
                .map_err(|m| fields.error(arg, m))?;
            target.insert(name.to_owned(), param);
        }
        fields.finish(1)
    }

    fn mask_line(&mut self, s: usize, c: usize, t: usize, line: &Line) -> Step {
        let rate = self.project.sequences[s].rate;
        let mut fields = Fields::new(line)?;
        let name = fields.id(1, "a mask name")?;
        let shape = take_mask_shape(&mut fields)?
            .ok_or_else(|| line.error(1, "a mask needs rect=, ellipse=, or points="))?;
        let mode = fields
            .take_with("mode", parse_mask_mode)?
            .unwrap_or_default();
        let inverted = fields.take_bool("inverted")?.unwrap_or(false);
        let params = parse_params(&mut fields, self.catalog.mask_props(), &name, rate)?;
        fields.finish(2)?;
        self.project.sequences[s].tracks[t].clips[c]
            .masks
            .push(Mask {
                name,
                shape,
                mode,
                inverted,
                params,
            });
        Ok(())
    }

    fn fx_line(&mut self, s: usize, t: usize, c: usize, line: &Line) -> Step {
        let rate = self.project.sequences[s].rate;
        let mut fields = Fields::new(line)?;
        let (fx, version) = fields.fx_ref(1, "an effect id")?;
        let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
            fields.error(
                fields.positional()[1],
                kerf_model::catalog::unknown("effect", fx.as_str()),
            )
        })?;
        let version = pinned(def, version)
            .map_err(|message| fields.error(fields.positional()[1], message))?;
        let enabled = fields.take_bool("enabled")?.unwrap_or(true);
        let params = parse_params(&mut fields, def.params.as_slice(), &fx, rate)?;
        fields.finish(2)?;
        self.project.sequences[s].tracks[t].clips[c]
            .effects
            .push(Effect {
                fx,
                version,
                enabled,
                params,
            });
        Ok(())
    }

    fn transition_line(&mut self, s: usize, t: usize, line: &Line) -> Step {
        let clock = Clock::of_sequence(&self.project.sequences[s]);
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a transition id")?;
        let from = fields
            .take_id("from")?
            .ok_or_else(|| line.error(1, "missing `from=`"))?;
        let to = fields
            .take_id("to")?
            .ok_or_else(|| line.error(1, "missing `to=`"))?;
        let (fx, version) = fields
            .take_fx_ref("fx")?
            .ok_or_else(|| line.error(1, "missing `fx=`"))?;
        let duration = fields
            .take_with("dur", |text| clock.duration(text))?
            .ok_or_else(|| line.error(1, "missing `dur=`"))?;
        let align = fields.take_with("align", parse_align)?.unwrap_or_default();
        let def = self.catalog.fx(fx.as_str()).ok_or_else(|| {
            line.error(1, kerf_model::catalog::unknown("transition", fx.as_str()))
        })?;
        let version = pinned(def, version).map_err(|message| line.error(1, message))?;
        let params = parse_params(&mut fields, def.params.as_slice(), &fx, clock.rate)?;
        fields.finish(2)?;
        self.record(&id, line);
        self.project.sequences[s].tracks[t]
            .transitions
            .push(Transition {
                id,
                from,
                to,
                fx,
                version,
                duration,
                align,
                params,
            });
        Ok(())
    }
}

/// The version a reference to `def` names, or the catalog's when it names
/// none, as long as this Kerf reads it.
pub(crate) fn pinned(def: &FxDef, version: Option<u32>) -> Result<u32, String> {
    let version = version.unwrap_or(def.version);
    def.reads(version)?;
    Ok(version)
}

/// Read the content of a clip: `media=`, `seq=`, `gen=`, or `caption=`, with
/// `src=` and `speed=` for media and nested sequences. Also returns the
/// source clock, if any. Shared with the script's insert and overwrite.
pub(crate) fn content(
    fields: &mut Fields<'_>,
    project: &Project,
    catalog: &Catalog,
) -> Result<(Content, Option<Clock>), Diagnostic> {
    let line = fields.line();
    let media = fields.take_id("media")?;
    let sequence = fields.take_id("seq")?;
    let generator = fields.take_fx_ref("gen")?;
    let caption = fields.take_text("caption")?.map(str::to_owned);
    let given = [
        media.is_some(),
        sequence.is_some(),
        generator.is_some(),
        caption.is_some(),
    ];
    if given.iter().filter(|g| **g).count() != 1 {
        return Err(line.error(1, "give exactly one of media=, seq=, gen=, or caption="));
    }
    let source_clock = match (&media, &sequence) {
        (Some(id), _) => Some(
            project
                .media(id.as_str())
                .map(Clock::of_media)
                .ok_or_else(|| line.error(1, format!("unknown media `{id}`")))?,
        ),
        (_, Some(id)) => Some(
            project
                .sequence(id.as_str())
                .map(Clock::of_sequence)
                .ok_or_else(|| line.error(1, format!("unknown sequence `{id}`")))?,
        ),
        _ => None,
    };
    let source_in = match source_clock {
        Some(clock) => fields
            .take_with("src", |text| clock.position(text))?
            .unwrap_or(Time::ZERO),
        None => Time::ZERO,
    };
    let speed = if source_clock.is_some() {
        fields
            .take_with("speed", |text| {
                Ratio::parse(text).map_err(|e| e.to_string())
            })?
            .unwrap_or(Ratio::ONE)
    } else {
        Ratio::ONE
    };
    let content = match (media, sequence, generator, caption) {
        (Some(media), ..) => Content::Media {
            media,
            source_in,
            speed,
        },
        (_, Some(sequence), ..) => Content::Sequence {
            sequence,
            source_in,
            speed,
        },
        (_, _, Some((fx, version)), _) => {
            // An unknown generator keeps the version it names; validation
            // reports the generator.
            let version = match catalog.fx(fx.as_str()) {
                Some(def) => pinned(def, version).map_err(|message| line.error(1, message))?,
                None => version.unwrap_or(1),
            };
            Content::Generator {
                fx,
                version,
                params: Params::new(),
            }
        }
        (.., Some(text)) => Content::Caption { text },
        _ => unreachable!("exactly one content key"),
    };
    Ok((content, source_clock))
}

fn parse_params(
    fields: &mut Fields<'_>,
    defs: &[ParamDef],
    fx: &Id,
    rate: Rate,
) -> Result<Params, Diagnostic> {
    let mut params = Params::new();
    for (name, arg) in fields.rest() {
        let def = defs.iter().find(|d| d.name == name).ok_or_else(|| {
            let known: Vec<&str> = defs.iter().map(|d| d.name.as_str()).collect();
            fields.error(
                arg,
                if known.is_empty() {
                    format!("`{fx}` takes no parameters")
                } else {
                    format!(
                        "`{fx}` has no parameter `{name}` (has: {})",
                        known.join(", ")
                    )
                },
            )
        })?;
        let param =
            crate::text::parse_param(&arg.value, def, rate).map_err(|m| fields.error(arg, m))?;
        params.insert(name.to_owned(), param);
    }
    Ok(params)
}

fn unknown_param(
    fields: &Fields<'_>,
    arg: &crate::lex::Arg,
    name: &str,
    props: &[ParamDef],
    generator: Option<&[ParamDef]>,
) -> Diagnostic {
    let known: Vec<&str> = props
        .iter()
        .chain(generator.into_iter().flatten())
        .map(|d| d.name.as_str())
        .collect();
    fields.error(
        arg,
        format!("no property `{name}` here (has: {})", known.join(", ")),
    )
}

/// `path= rate= [tc=] dur= [video= [alpha=]] [audio=] [reel=]`.
pub(crate) fn media_fields(fields: &mut Fields<'_>, id: Id) -> Result<Media, Diagnostic> {
    let line = fields.line();
    let path = fields
        .require("path")
        .and_then(|arg| fields.text(arg))?
        .to_owned();
    let rate = fields
        .take_with("rate", |text| Rate::parse(text).map_err(|e| e.to_string()))?
        .ok_or_else(|| line.error(1, "missing `rate=`"))?;
    let (start, drop_frame) = fields
        .take_with("tc", |text| {
            parse_timecode(text, rate).map_err(|e| e.to_string())
        })?
        .unwrap_or((Time::ZERO, false));
    let clock = Clock {
        rate,
        origin: start,
        drop_frame,
    };
    let duration = fields
        .take_with("dur", |text| clock.duration(text))?
        .ok_or_else(|| line.error(1, "missing `dur=`"))?;
    let size = fields.take_with("video", parse_size)?;
    let alpha = fields.take_bool("alpha")?.unwrap_or(false);
    if alpha && size.is_none() {
        return Err(line.error(1, "`alpha=` needs `video=`"));
    }
    let video = size.map(|(width, height)| VideoInfo {
        width,
        height,
        alpha,
    });
    let audio = fields.take_with("audio", parse_audio)?;
    let reel = fields.take_text("reel")?.map(str::to_owned);
    Ok(Media {
        id,
        path,
        rate,
        drop_frame,
        start,
        duration,
        video,
        audio,
        reel,
    })
}

/// `[name=] rate= size= [audio=] [start=]`, with no tracks.
pub(crate) fn sequence_fields(fields: &mut Fields<'_>, id: Id) -> Result<Sequence, Diagnostic> {
    let line = fields.line();
    let name = fields.take_text("name")?.unwrap_or_default().to_owned();
    let rate = fields
        .take_with("rate", |text| Rate::parse(text).map_err(|e| e.to_string()))?
        .ok_or_else(|| line.error(1, "missing `rate=`"))?;
    let (width, height) = fields
        .take_with("size", parse_size)?
        .ok_or_else(|| line.error(1, "missing `size=`"))?;
    let sample_rate = fields
        .take_with("audio", parse_sample_rate)?
        .unwrap_or(48_000);
    let (start, drop_frame) = fields
        .take_with("start", |text| {
            parse_timecode(text, rate).map_err(|e| e.to_string())
        })?
        .unwrap_or((Time::ZERO, false));
    Ok(Sequence {
        id,
        name,
        rate,
        drop_frame,
        width,
        height,
        sample_rate,
        start,
        tracks: Vec::new(),
        markers: Vec::new(),
    })
}

pub(crate) fn parse_sample_rate(text: &str) -> Result<u32, String> {
    text.parse()
        .map_err(|_| format!("`{text}` is not a sample rate"))
}

pub(crate) fn parse_size(text: &str) -> Result<(u32, u32), String> {
    let invalid = || format!("`{text}` is not a size like 1920x1080");
    let (width, height) = text.split_once('x').ok_or_else(invalid)?;
    Ok((
        width.parse().map_err(|_| invalid())?,
        height.parse().map_err(|_| invalid())?,
    ))
}

pub(crate) fn parse_audio(text: &str) -> Result<AudioInfo, String> {
    let invalid = || format!("`{text}` is not an audio format like 2ch@48000");
    let (channels, rate) = text.split_once("ch@").ok_or_else(invalid)?;
    Ok(AudioInfo {
        channels: channels.parse().map_err(|_| invalid())?,
        sample_rate: rate.parse().map_err(|_| invalid())?,
    })
}

pub(crate) fn parse_mask_mode(text: &str) -> Result<MaskMode, String> {
    MaskMode::parse(text).ok_or_else(|| {
        let names: Vec<&str> = MaskMode::ALL.iter().map(|mode| mode.name()).collect();
        format!("`{text}` is not a mask mode ({})", names.join(", "))
    })
}

/// A mask's outline from `rect=`, `ellipse=`, or `points=` (with
/// `closed=`), in fractions of the picture; none when absent.
pub(crate) fn take_mask_shape(fields: &mut Fields<'_>) -> Result<Option<MaskShape>, Diagnostic> {
    let rect = fields.take("rect");
    let ellipse = fields.take("ellipse");
    let points = fields.take("points");
    let closed = fields.take("closed");
    let given: Vec<&Arg> = [rect, ellipse, points].into_iter().flatten().collect();
    if let Some(second) = given.get(1) {
        return Err(fields.error(
            second,
            "a mask has one outline: rect=, ellipse=, or points=",
        ));
    }
    if let (Some(closed), None) = (closed, points) {
        return Err(fields.error(closed, "`closed=` goes with `points=`"));
    }
    let numbers = |arg: &Arg, text: &str| -> Result<Vec<f64>, Diagnostic> {
        text.split(',')
            .map(|part| {
                part.trim()
                    .parse::<f64>()
                    .ok()
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| fields.error(arg, format!("`{part}` is not a number")))
            })
            .collect()
    };
    let four = |arg: &Arg, what: &str| -> Result<[f64; 4], Diagnostic> {
        let values = numbers(arg, fields.text(arg)?)?;
        values
            .try_into()
            .map_err(|_| fields.error(arg, format!("expected {what}")))
    };
    if let Some(arg) = rect {
        return Ok(Some(MaskShape::Rect(four(arg, "left,top,right,bottom")?)));
    }
    if let Some(arg) = ellipse {
        return Ok(Some(MaskShape::Ellipse(four(
            arg,
            "x,y,radius across,radius down",
        )?)));
    }
    let Some(arg) = points else {
        return Ok(None);
    };
    let points = fields
        .text(arg)?
        .split_whitespace()
        .map(|point| {
            let values = numbers(arg, point)?;
            <[f64; 2]>::try_from(values)
                .map_err(|_| fields.error(arg, format!("`{point}` is not x,y")))
        })
        .collect::<Result<Vec<_>, _>>()?;
    let closed = match closed.map(|closed| fields.text(closed)).transpose()? {
        None | Some("true") => true,
        Some("false") => false,
        Some(other) => {
            return Err(fields.error(arg, format!("`closed` is true or false, not `{other}`")))
        }
    };
    Ok(Some(MaskShape::Polygon { points, closed }))
}

/// `rect=`, `ellipse=`, or `points=` (and `closed=false`) for an outline.
pub(crate) fn mask_shape_text(shape: &MaskShape) -> String {
    let list = |values: &[f64]| {
        values
            .iter()
            .map(|v| number(*v))
            .collect::<Vec<_>>()
            .join(",")
    };
    match shape {
        MaskShape::Rect(rect) => format!(" rect={}", list(rect)),
        MaskShape::Ellipse(ellipse) => format!(" ellipse={}", list(ellipse)),
        MaskShape::Polygon { points, closed } => {
            let points: Vec<String> = points.iter().map(|point| list(point)).collect();
            let open = if *closed { "" } else { " closed=false" };
            format!(" points={}{open}", quote(&points.join(" ")))
        }
    }
}

/// A mask after its name: its outline, mode and inversion when not the
/// defaults, and its properties.
pub(crate) fn mask_text(mask: &Mask, catalog: &Catalog, rate: Rate) -> String {
    let mut out = mask_shape_text(&mask.shape);
    if mask.mode != MaskMode::Add {
        write!(out, " mode={}", mask.mode.name()).unwrap();
    }
    if mask.inverted {
        out.push_str(" inverted=true");
    }
    out.push_str(&params_text(&mask.params, catalog.mask_props(), rate));
    out
}

pub(crate) fn parse_align(text: &str) -> Result<Align, String> {
    Align::parse(text).ok_or_else(|| format!("`{text}` is not center, start, or end"))
}

pub(crate) fn parse_marker_color(text: &str) -> Result<MarkerColor, String> {
    MarkerColor::parse(text).ok_or_else(|| {
        let names: Vec<&str> = MarkerColor::ALL.iter().map(|c| c.name()).collect();
        format!("`{text}` is not a marker color ({})", names.join(", "))
    })
}

// ---- printing ---------------------------------------------------------------

pub fn print_document(project: &Project, catalog: &Catalog, options: PrintOptions) -> String {
    let mut out = String::new();
    writeln!(out, "kerf {VERSION}").unwrap();
    write!(out, "project name={}", quote(&project.name)).unwrap();
    if !project.retired.is_empty() {
        let retired: Vec<&str> = project.retired.iter().map(Id::as_str).collect();
        write!(out, " retired={}", retired.join(",")).unwrap();
    }
    out.push('\n');
    if !project.media.is_empty() {
        out.push('\n');
        for media in &project.media {
            out.push_str(&media_line(media));
            out.push('\n');
        }
    }
    for sequence in &project.sequences {
        out.push('\n');
        print_sequence(&mut out, project, sequence, catalog, options);
    }
    out
}

pub(crate) fn media_line(media: &Media) -> String {
    let clock = Clock::of_media(media);
    let mut line = format!(
        "media {} path={} rate={}",
        media.id,
        quote(&media.path),
        media.rate
    );
    if media.start != Time::ZERO || media.drop_frame {
        write!(line, " tc={}", clock.print_position(Time::ZERO)).unwrap();
    }
    write!(line, " dur={}", clock.print_duration(media.duration)).unwrap();
    if let Some(video) = media.video {
        write!(line, " video={}x{}", video.width, video.height).unwrap();
        if video.alpha {
            line.push_str(" alpha=true");
        }
    }
    if let Some(audio) = media.audio {
        write!(line, " audio={}ch@{}", audio.channels, audio.sample_rate).unwrap();
    }
    if let Some(reel) = &media.reel {
        write!(line, " reel={}", quote(reel)).unwrap();
    }
    line
}

pub(crate) fn sequence_line(sequence: &Sequence) -> String {
    let clock = Clock::of_sequence(sequence);
    format!(
        "sequence {} name={} rate={} size={}x{} audio={} start={}",
        sequence.id,
        quote(&sequence.name),
        sequence.rate,
        sequence.width,
        sequence.height,
        sequence.sample_rate,
        clock.print_position(Time::ZERO)
    )
}

fn print_sequence(
    out: &mut String,
    project: &Project,
    sequence: &Sequence,
    catalog: &Catalog,
    options: PrintOptions,
) {
    let clock = Clock::of_sequence(sequence);
    out.push_str(&sequence_line(sequence));
    if options.annotate {
        write!(
            out,
            "  # duration {}",
            clock.print_duration(sequence.duration())
        )
        .unwrap();
    }
    out.push('\n');
    for track in &sequence.tracks {
        write!(out, "  track {} {}", track.id, track.kind.name()).unwrap();
        if let Some(name) = &track.name {
            write!(out, " name={}", quote(name)).unwrap();
        }
        for (flag, value, default) in [
            ("enabled", track.enabled, true),
            ("locked", track.locked, false),
            ("muted", track.muted, false),
        ] {
            if value != default {
                write!(out, " {flag}={value}").unwrap();
            }
        }
        out.push('\n');
        let domain = track.kind.domain();
        for clip in &track.clips {
            print_clip(out, project, &clock, clip, domain, catalog, options);
        }
        let mut transitions: Vec<&Transition> = track.transitions.iter().collect();
        transitions.sort_by_key(|t| track.clip_index(t.from.as_str()));
        for transition in transitions {
            write!(
                out,
                "    transition {} from={} to={} fx={} dur={}",
                transition.id,
                transition.from,
                transition.to,
                print_fx_ref(&transition.fx, transition.version),
                clock.print_duration(transition.duration)
            )
            .unwrap();
            if transition.align != Align::Center {
                write!(out, " align={}", transition.align.name()).unwrap();
            }
            let defs = catalog
                .fx(transition.fx.as_str())
                .map(|d| d.params.as_slice())
                .unwrap_or_default();
            out.push_str(&params_text(&transition.params, defs, clock.rate));
            out.push('\n');
        }
    }
    for marker in &sequence.markers {
        write!(
            out,
            "  marker {} at={}",
            marker.id,
            clock.print_position(marker.at)
        )
        .unwrap();
        if marker.duration != Time::ZERO {
            write!(out, " dur={}", clock.print_duration(marker.duration)).unwrap();
        }
        if let Some(name) = &marker.name {
            write!(out, " name={}", quote(name)).unwrap();
        }
        if let Some(color) = marker.color {
            write!(out, " color={}", color.name()).unwrap();
        }
        if let Some(note) = &marker.note {
            write!(out, " note={}", quote(note)).unwrap();
        }
        out.push('\n');
    }
}

/// Source clock and the `media=`/`seq=` key for a clip's content.
pub(crate) fn source_clock(project: &Project, content: &Content) -> Option<Clock> {
    match content {
        Content::Media { media, .. } => project.media(media.as_str()).map(Clock::of_media),
        Content::Sequence { sequence, .. } => {
            project.sequence(sequence.as_str()).map(Clock::of_sequence)
        }
        _ => None,
    }
}

/// `media=m1 src=…`, `seq=…`, `gen=…`, or `caption=…`; speed is separate.
pub(crate) fn content_text(project: &Project, content: &Content) -> String {
    match content {
        Content::Media {
            media: id,
            source_in,
            ..
        }
        | Content::Sequence {
            sequence: id,
            source_in,
            ..
        } => {
            let key = if matches!(content, Content::Media { .. }) {
                "media"
            } else {
                "seq"
            };
            let src = source_clock(project, content).map_or_else(
                || format!("{}fl", source_in.flicks()),
                |clock| clock.print_position(*source_in),
            );
            format!("{key}={id} src={src}")
        }
        Content::Generator { fx, version, .. } => format!("gen={}", print_fx_ref(fx, *version)),
        Content::Caption { text } => format!("caption={}", quote(text)),
    }
}

fn print_clip(
    out: &mut String,
    project: &Project,
    clock: &Clock,
    clip: &Clip,
    domain: Option<Domain>,
    catalog: &Catalog,
    options: PrintOptions,
) {
    write!(
        out,
        "    clip {} {} at={} dur={}",
        clip.id,
        content_text(project, &clip.content),
        clock.print_position(clip.start),
        clock.print_duration(clip.duration)
    )
    .unwrap();
    if clip.speed() != Ratio::ONE {
        write!(out, " speed={}", clip.speed()).unwrap();
    }
    if let Some(name) = &clip.name {
        write!(out, " name={}", quote(name)).unwrap();
    }
    if !clip.enabled {
        out.push_str(" enabled=false");
    }
    if let Some(link) = &clip.link {
        write!(out, " link={link}").unwrap();
    }
    if options.annotate {
        write!(
            out,
            "  # ends {} ({})",
            clock.print_position(clip.end()),
            clip.duration
        )
        .unwrap();
        if let (Some((_, source_out)), Some(source)) =
            (clip.source_range(), source_clock(project, &clip.content))
        {
            write!(out, ", src out {}", source.print_position(source_out)).unwrap();
        }
    }
    out.push('\n');
    let mut set = String::new();
    if let Some(domain) = domain {
        set.push_str(&params_text(&clip.props, catalog.props(domain), clock.rate));
    }
    if let Content::Generator { fx, params, .. } = &clip.content {
        let defs = catalog
            .fx(fx.as_str())
            .map(|d| d.params.as_slice())
            .unwrap_or_default();
        set.push_str(&params_text(params, defs, clock.rate));
    }
    if !set.is_empty() {
        writeln!(out, "      set{set}").unwrap();
    }
    for mask in &clip.masks {
        writeln!(
            out,
            "      mask {}{}",
            mask.name,
            mask_text(mask, catalog, clock.rate)
        )
        .unwrap();
    }
    for effect in &clip.effects {
        write!(out, "      fx {}", print_fx_ref(&effect.fx, effect.version)).unwrap();
        if !effect.enabled {
            out.push_str(" enabled=false");
        }
        let defs = catalog
            .fx(effect.fx.as_str())
            .map(|d| d.params.as_slice())
            .unwrap_or_default();
        out.push_str(&params_text(&effect.params, defs, clock.rate));
        out.push('\n');
    }
}

/// ` name=value` pairs in catalog order, then any unknown names.
pub(crate) fn params_text(params: &Params, defs: &[ParamDef], rate: Rate) -> String {
    let mut out = String::new();
    let known = defs
        .iter()
        .filter_map(|def| params.get_key_value(def.name.as_str()));
    let unknown = params
        .iter()
        .filter(|(name, _)| !defs.iter().any(|d| &d.name == *name));
    for (name, param) in known.chain(unknown) {
        // A track or mask name is a word, as ids are.
        let reference = defs.iter().any(|def| {
            def.name == *name
                && matches!(
                    def.ty,
                    ParamType::Track | ParamType::Sound | ParamType::Mask
                )
        });
        let time = defs
            .iter()
            .any(|def| def.name == *name && def.ty == ParamType::Time);
        match param {
            Param::Static(Value::Text(id)) if reference && Id::new(id.as_str()).is_ok() => {
                write!(out, " {name}={id}").unwrap();
            }
            Param::Static(Value::Number(seconds)) if time => {
                let at = Time::from_seconds(*seconds).unwrap_or(Time::ZERO);
                write!(out, " {name}={}", crate::text::print_offset(at, rate)).unwrap();
            }
            _ => write!(out, " {name}={}", print_param(param, rate)).unwrap(),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"kerf 1
project name="Short Film"

media m1 path="footage/A001_C003.mov" rate=23.976 tc=14:22:31:05 dur=00:04:12:00 video=3840x2160 audio=2ch@48000 reel="A001"
media m2 path="music.wav" rate=23.976 dur=00:03:00:00 audio=2ch@48000

sequence s1 name="Main Edit" rate=23.976 size=1920x1080 audio=48000 start=01:00:00:00
  track V1 video
    clip c1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set x=0.1 scale={0f:1 48f:1.1:ease} opacity=0.9
      fx color.basic@1 exposure=0.3 saturation=1.2
    clip c2 media=m1 src=14:23:02:00 at=01:00:06:12 dur=00:00:03:00 speed=-1.5
    transition t1 from=c1 to=c2 fx=dip@1 dur=00:00:00:12 align=start color=#ffffff
  track V2 video name="Titles"
    clip c3 gen=title@1 at=01:00:01:00 dur=00:00:02:00
      set y=0.3 text="Day One" size=0.1
  track A1 audio
    clip a1 media=m1 src=14:22:35:00 at=01:00:00:00 dur=00:00:06:12 link=L1
      set gain=-3
  track A2 audio locked=true
    clip a2 media=m2 src=00:00:10:00 at=01:00:00:00 dur=00:00:20:00
      fx audio.eq3@1 low=-6
  track S1 subtitle
    clip cap1 caption="Where are we?" at=01:00:01:00 dur=00:00:02:00
  marker mk1 at=01:00:05:00 name="pickup needed" color=red note="ADR \"line 4\""
"#;

    #[test]
    fn canonical_documents_round_trip_exactly() {
        let document =
            parse_document(SAMPLE, Catalog::builtin()).unwrap_or_else(|e| panic!("{e:#?}"));
        assert_eq!(document.warnings, vec![]);
        let printed = print_document(
            &document.project,
            Catalog::builtin(),
            PrintOptions::default(),
        );
        assert_eq!(printed, SAMPLE);
        let again = parse_document(&printed, Catalog::builtin()).unwrap();
        assert_eq!(again.project, document.project);
    }

    #[test]
    fn references_pin_the_version_they_were_read_with() {
        // Without a version, a reference takes this Kerf's, and prints it.
        let bare = SAMPLE
            .replace("fx color.basic@1", "fx color.basic")
            .replace("fx=dip@1", "fx=dip")
            .replace("gen=title@1", "gen=title");
        let project = parse_document(&bare, Catalog::builtin()).unwrap().project;
        assert_eq!(
            print_document(&project, Catalog::builtin(), PrintOptions::default()),
            SAMPLE
        );
        for (from, to, needle) in [
            (
                "fx color.basic@1",
                "fx color.basic@2",
                "newer than this Kerf",
            ),
            ("fx=dip@1", "fx=dip@9", "`dip@9` is newer"),
            ("gen=title@1", "gen=title@2", "`title@2` is newer"),
            (
                "fx color.basic@1",
                "fx color.basic@0",
                "a whole version from 1",
            ),
            (
                "fx color.basic@1",
                "fx color.basic@01",
                "a whole version from 1",
            ),
            ("fx=dip@1", "fx=dip@", "a whole version from 1"),
        ] {
            let errors = parse_document(&SAMPLE.replace(from, to), Catalog::builtin()).unwrap_err();
            assert!(
                errors.iter().any(|e| e.message.contains(needle)),
                "{to}: {errors:#?}"
            );
        }
    }

    #[test]
    fn older_versions_still_read_are_drawn_by_the_current_one() {
        let builtin = Catalog::builtin();
        let mut catalog = Catalog::new(
            builtin.props(Domain::Video).to_vec(),
            builtin.props(Domain::Audio).to_vec(),
        );
        catalog
            .define(
                FxDef::new(
                    "test.glow",
                    kerf_model::FxKind::Effect,
                    Domain::Video,
                    "A test effect.",
                )
                .version(3, 2),
            )
            .unwrap();
        let document = |version: u32| {
            format!(
                "kerf 1\nproject name=\"\"\n\n\
                 media m1 path=\"a.mov\" rate=24 dur=00:00:10:00 video=1920x1080\n\n\
                 sequence s1 name=\"\" rate=24 size=1920x1080 audio=48000 start=00:00:00:00\n  \
                 track V1 video\n    \
                 clip c1 media=m1 src=00:00:00:00 at=00:00:00:00 dur=00:00:01:00\n      \
                 fx test.glow@{version}\n"
            )
        };
        let current = parse_document(&document(3), &catalog).unwrap();
        assert_eq!(current.warnings, vec![]);
        let older = parse_document(&document(2), &catalog).unwrap();
        assert!(
            older
                .warnings
                .iter()
                .any(|w| w.code == "fx.version_older"
                    && w.message.contains("draws as `test.glow@3`")),
            "{:#?}",
            older.warnings
        );
        // The document keeps the version it names.
        assert_eq!(
            print_document(&older.project, &catalog, PrintOptions::default()),
            document(2)
        );
        let gone = parse_document(&document(1), &catalog).unwrap_err();
        assert!(gone[0].message.contains("no longer read"), "{gone:#?}");
        let newer = parse_document(&document(4), &catalog).unwrap_err();
        assert!(
            newer[0].message.contains("newer than this Kerf"),
            "{newer:#?}"
        );
    }

    #[test]
    fn transparent_pictures_round_trip() {
        let text = SAMPLE.replace("video=3840x2160", "video=3840x2160 alpha=true");
        let project = parse_document(&text, Catalog::builtin()).unwrap().project;
        assert!(project.media[0].video.unwrap().alpha);
        assert_eq!(
            print_document(&project, Catalog::builtin(), PrintOptions::default()),
            text
        );
        let without = SAMPLE.replace("video=3840x2160", "alpha=true");
        let errors = parse_document(&without, Catalog::builtin()).unwrap_err();
        assert!(errors[0].message.contains("needs `video=`"), "{errors:?}");
    }

    #[test]
    fn retired_ids_round_trip_on_the_project_line() {
        let text = SAMPLE.replace(
            "project name=\"Short Film\"",
            "project name=\"Short Film\" retired=L2,c4,c9",
        );
        let project = parse_document(&text, Catalog::builtin()).unwrap().project;
        let retired: Vec<&str> = project.retired.iter().map(Id::as_str).collect();
        assert_eq!(retired, ["L2", "c4", "c9"]);
        assert_eq!(
            print_document(&project, Catalog::builtin(), PrintOptions::default()),
            text
        );
        let reused = SAMPLE.replace(
            "project name=\"Short Film\"",
            "project name=\"Short Film\" retired=c2",
        );
        let errors = parse_document(&reused, Catalog::builtin()).unwrap_err();
        assert!(errors.iter().any(|d| d.code == "id.retired"), "{errors:?}");
    }

    #[test]
    fn annotated_documents_parse_back_to_the_same_project() {
        let project = parse_document(SAMPLE, Catalog::builtin()).unwrap().project;
        let annotated = print_document(
            &project,
            Catalog::builtin(),
            PrintOptions { annotate: true },
        );
        assert!(annotated.contains("# ends 01:00:06:12"));
        assert!(annotated.contains("src out 14:22:41:12"));
        assert_eq!(
            parse_document(&annotated, Catalog::builtin())
                .unwrap()
                .project,
            project
        );
    }

    #[test]
    fn timecode_resolves_against_media_and_sequence_origins() {
        let project = parse_document(SAMPLE, Catalog::builtin()).unwrap().project;
        let clip = project.clip("c1").unwrap();
        let rate = Rate::FPS_23_976;
        assert_eq!(rate.frames_exact(clip.start), Some(0));
        let (source_in, _) = clip.source_range().unwrap();
        assert_eq!(rate.frames_exact(source_in), Some(4 * 24 - 5));
    }

    #[test]
    fn errors_point_at_the_offending_line() {
        let broken = SAMPLE.replace(
            "dur=00:00:03:00 speed=-1.5",
            "dur=00:00:03:00 speed=-1.5 colour=red",
        );
        let errors = parse_document(&broken, Catalog::builtin()).unwrap_err();
        assert_eq!(errors[0].span.unwrap().line, 12);
        assert!(errors[0].message.contains("colour"));

        let overlapping = SAMPLE.replace(
            "at=01:00:06:12 dur=00:00:03:00",
            "at=01:00:06:00 dur=00:00:03:00",
        );
        let errors = parse_document(&overlapping, Catalog::builtin()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.code == "clip.overlap" && e.span.unwrap().line == 12),
            "{errors:#?}"
        );

        let unknown = SAMPLE.replace("fx color.basic", "fx color.fancy");
        assert!(parse_document(&unknown, Catalog::builtin()).unwrap_err()[0]
            .message
            .contains("color.fancy"));
    }

    #[test]
    fn sound_parameters_name_sound_tracks() {
        let drawing = |track: &str| {
            SAMPLE.replace(
                "      set y=0.3 text=\"Day One\" size=0.1\n",
                &format!(
                    "      set y=0.3 text=\"Day One\" size=0.1\n      fx generate.audiowaveform audioLayer={track}\n"
                ),
            )
        };
        let document = parse_document(&drawing("A2"), Catalog::builtin()).unwrap();
        assert!(print_document(
            &document.project,
            Catalog::builtin(),
            PrintOptions::default()
        )
        .contains("audioLayer=A2\n"));
        let errors = parse_document(&drawing("V1"), Catalog::builtin()).unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.message.contains("not a sound track")),
            "{errors:#?}"
        );
    }

    #[test]
    fn masks_round_trip_and_name_what_effects_read() {
        let masked = SAMPLE.replace(
            "      fx color.basic@1 exposure=0.3 saturation=1.2\n",
            "      mask k1 rect=0.1,0.2,0.9,0.8 feather=6 opacity={0f:0 12f:1}\n      \
             mask ring ellipse=0.5,0.5,0.25,0.3 mode=none inverted=true\n      \
             mask path points=\"0,0 0.5,1 1,0\" closed=false mode=none\n      \
             fx color.basic@1 exposure=0.3 saturation=1.2\n      \
             fx generate.audiowaveform@1 path=ring\n",
        );
        let document = parse_document(&masked, Catalog::builtin()).unwrap();
        let printed = print_document(
            &document.project,
            Catalog::builtin(),
            PrintOptions::default(),
        );
        assert_eq!(printed, masked);
        for (from, to, needle) in [
            ("path=ring", "path=nothing", "not a mask of clip"),
            ("mask path points", "mask k1 points", "two masks named"),
            (
                "ellipse=0.5,0.5,0.25,0.3",
                "ellipse=0.5,0.5,0,0.3",
                "radii above 0",
            ),
            ("feather=6", "feather=-1", "outside"),
        ] {
            let errors = parse_document(&masked.replace(from, to), Catalog::builtin()).unwrap_err();
            assert!(
                errors.iter().any(|e| e.message.contains(needle)),
                "{needle}: {errors:#?}"
            );
        }
    }

    #[test]
    fn effects_read_other_picture_tracks_only() {
        let reading = |c1: &str, c3: &str| {
            SAMPLE
                .replace(
                    "      fx color.basic@1 exposure=0.3 saturation=1.2\n",
                    &format!("      fx color.basic@1 exposure=0.3 saturation=1.2\n{c1}"),
                )
                .replace(
                    "      set y=0.3 text=\"Day One\" size=0.1\n",
                    &format!("      set y=0.3 text=\"Day One\" size=0.1\n{c3}"),
                )
        };
        let reads = |track: &str| format!("      fx key.trackmatte@1 matte={track}\n");
        let document = parse_document(&reading(&reads("V2"), ""), Catalog::builtin()).unwrap();
        // The track name prints back as a word.
        assert!(print_document(
            &document.project,
            Catalog::builtin(),
            PrintOptions::default()
        )
        .contains("matte=V2\n"));
        for (c1, c3, needle) in [
            (reads("V9"), String::new(), "not a picture track"),
            (reads("A1"), String::new(), "not a picture track"),
            (reads("V1"), String::new(), "the track it is on"),
            (reads("V2"), reads("V1"), "reads track V1 back"),
        ] {
            let errors = parse_document(&reading(&c1, &c3), Catalog::builtin()).unwrap_err();
            assert!(
                errors
                    .iter()
                    .any(|e| e.code == "track.read" && e.message.contains(needle)),
                "{needle}: {errors:#?}"
            );
        }
    }

    #[test]
    fn rejects_structure_out_of_place() {
        for (text, needle) in [
            ("", "kerf 1"),
            ("kerf 2\n", "kerf 1"),
            ("kerf 1\n  track V1 video\n", "under a sequence"),
            (
                "kerf 1\nsequence s1 rate=24 size=2x2\n    clip c1 caption=\"x\" at=0f dur=1f\n",
                "under a track",
            ),
        ] {
            let errors = parse_document(text, Catalog::builtin()).unwrap_err();
            assert!(errors[0].message.contains(needle), "{text:?}: {errors:?}");
        }
    }
}
