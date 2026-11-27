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
//!
//! A clip that follows another ([`Clip::follows`]) says where it is by that
//! one, `at=c1.start+12f`: so long after it starts, or with `-` before, or
//! with no amount as it does. Where it is on the sequence is worked out as
//! the document is read, whichever of the two comes first in it, so the line
//! of a clip that only went where the one it follows went does not change.
//!
//! A media that Kerf or a model makes has the steps that make it under its
//! line ([`crate::making`]):
//!
//! ```text
//! media m3 path="shots/open.mp4" rate=16 dur=65f video=832x480
//!   render s2 {guide.mp4} pass=depth size=832x480 rate=16
//!   generate model=shot prompt="a boat leaves the harbour" guide={guide.mp4} seed=7
//! ```

use crate::lex::{lex, Arg, Atom, Line};
use crate::text::{
    number, parse_speed, print_fx_ref, print_param, print_speed, quote, Clock, Fields,
};
use kerf_model::time::parse_timecode;
use kerf_model::Align;
use kerf_model::{
    validate, AudioInfo, Catalog, Clip, Content, Diagnostic, Domain, Effect, FxDef, Id, Interp,
    Marker, MarkerColor, Mask, MaskMode, MaskShape, Media, MissingKind, OutlineKey, Param,
    ParamDef, ParamType, Params, Project, Rate, Sequence, Speed, Synced, Time, Track, TrackKind,
    Transition, Value, VideoInfo,
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
        following: Vec::new(),
    };
    parser.headers(&lines);
    if parser.errors.is_empty() {
        parser.bodies(&lines);
    }
    if parser.errors.is_empty() {
        parser.follow();
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
    /// The clips that say where they are by another, each with how long after that one
    /// starts it does: placed once every clip is read ([`Parser::follow`]).
    following: Vec<Following>,
}

/// A clip whose line says `at=<clip>.start` and an amount.
struct Following {
    sequence: usize,
    clip: Id,
    followed: Id,
    after: Time,
    line: usize,
    column: usize,
}

/// `at=` as a clip's line has it: a place on the sequence, or another clip's start and how
/// long after it.
enum At {
    Placed(Time),
    Following(Id, Time),
}

/// Read a clip's `at=`. A clip's id may hold `.start` and `-` itself, so of every `.start`
/// in it the last that leaves nothing or an amount is the one meant.
fn read_at(text: &str, clock: &Clock) -> Result<At, String> {
    const START: &str = ".start";
    if !text.contains(START) {
        if text.contains(".end") || text.contains('@') {
            return Err(format!(
                "`{text}`: a clip follows another from its start, as `at=c1.start+12f`; the other anchors are a script's"
            ));
        }
        return clock.position(text).map(At::Placed);
    }
    text.rmatch_indices(START)
        .find_map(|(at, _)| {
            let id = Id::new(&text[..at]).ok()?;
            let after = match &text[at + START.len()..] {
                "" => Time::ZERO,
                amount if amount.starts_with(['+', '-']) => clock.delta(amount).ok()?,
                _ => return None,
            };
            Some(At::Following(id, after))
        })
        .ok_or_else(|| {
            format!(
                "`{text}`: after `<clip>.start` comes an amount such as +12f or -0.5s, or nothing"
            )
        })
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
        let mut bus: Option<usize> = None;
        let mut sequence_count = 0;
        // The media whose steps the lines under it are, and the files those so far make.
        let mut media: Option<usize> = None;
        let mut media_count = 0;
        let mut made: Vec<String> = Vec::new();
        for line in &lines[1..] {
            let result = match (line.depth, keyword(line)) {
                (0, "sequence") => {
                    sequence = Some(sequence_count);
                    sequence_count += 1;
                    media = None;
                    track = None;
                    clip = None;
                    bus = None;
                    Ok(())
                }
                (0, "media") => {
                    sequence = None;
                    media = Some(media_count);
                    media_count += 1;
                    made.clear();
                    Ok(())
                }
                (0, _) => {
                    sequence = None;
                    media = None;
                    Ok(())
                }
                (1, word) if media.is_some() => {
                    if crate::making::starts(word) {
                        crate::making::read(line, 0, &made).map(|step| {
                            made.push(step.makes().to_owned());
                            let open = media.expect("the match guard checked a media is open");
                            self.project.media[open].steps.push(step);
                        })
                    } else {
                        Err(line.error(
                            1,
                            format!(
                                "under a media come the steps that make its file, `render`, `generate`, or `join`, not `{word}`"
                            ),
                        ))
                    }
                }
                (1, word) => match sequence {
                    None => Err(line.error(
                        1,
                        "indented lines belong under a sequence, or under a media as the steps that make it",
                    )),
                    Some(s) => {
                        clip = None;
                        match word {
                            "track" => self.track_line(s, line).map(|t| {
                                track = Some(t);
                                bus = None;
                            }),
                            "bus" => self.bus_line(s, line).map(|b| {
                                track = None;
                                bus = Some(b);
                            }),
                            "marker" => {
                                track = None;
                                bus = None;
                                self.marker_line(s, line)
                            }
                            other => Err(line.error(
                                1,
                                format!("expected track, bus, or marker, not `{other}`"),
                            )),
                        }
                    }
                },
                (2, "fx") if bus.is_some() => match (sequence, bus) {
                    (Some(s), Some(b)) => self.bus_fx_line(s, b, line),
                    _ => Err(line.error(1, "fx lines belong under a clip or a bus")),
                },
                (2, word) => {
                    match (sequence, track) {
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
                    }
                }
                (3, word) => match (sequence, track, clip) {
                    (Some(s), Some(t), Some(c)) => match word {
                        "set" => self.set_line(s, t, c, line),
                        "mask" => self.mask_line(s, c, t, line),
                        "fx" => self.fx_line(s, t, c, line),
                        other => {
                            Err(line.error(1, format!("expected set, mask, or fx, not `{other}`")))
                        }
                    },
                    _ => Err(line.error(1, "set, mask, and fx lines belong under a clip")),
                },
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
        track.role = take_role(&mut fields)?.flatten();
        track.sync_locked = take_sync(&mut fields)?.unwrap_or(true);
        track.bus = fields.take_id("bus")?;
        fields.finish(3)?;
        let tracks = &mut self.project.sequences[s].tracks;
        tracks.push(track);
        Ok(tracks.len() - 1)
    }

    fn bus_line(&mut self, s: usize, line: &Line) -> Step<usize> {
        let rate = self.project.sequences[s].rate;
        let mut fields = Fields::new(line)?;
        let id = fields.id(1, "a bus id")?;
        let mut bus = kerf_model::Bus::new(id.clone());
        bus.name = fields.take_text("name")?.map(str::to_owned);
        bus.to = fields.take_id("to")?;
        bus.params = parse_params(&mut fields, self.catalog.props(Domain::Audio), &id, rate)?;
        fields.finish(2)?;
        let buses = &mut self.project.sequences[s].buses;
        buses.push(bus);
        Ok(buses.len() - 1)
    }

    fn bus_fx_line(&mut self, s: usize, b: usize, line: &Line) -> Step {
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
        self.project.sequences[s].buses[b].effects.push(Effect {
            fx,
            version,
            enabled,
            params,
        });
        Ok(())
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
        let (content, _) = content(&mut fields, &self.project, self.catalog, clock.rate)?;
        let at_column = line
            .args
            .iter()
            .find(|arg| arg.key.as_deref() == Some("at"))
            .map_or(1, |arg| arg.column);
        let at = fields
            .take_with("at", |text| read_at(text, &clock))?
            .ok_or_else(|| line.error(1, "missing `at=`"))?;
        let duration = fields
            .take_with("dur", |text| clock.duration(text))?
            .ok_or_else(|| {
                line.error(1, "missing `dur=`; `look` with `file` gives the whole line")
            })?;
        // What follows another is placed once every clip is read.
        let start = match &at {
            At::Placed(start) => *start,
            At::Following(..) => Time::ZERO,
        };
        let mut clip = Clip::new(id, content, start, duration);
        if let At::Following(followed, after) = at {
            self.following.push(Following {
                sequence: s,
                clip: clip.id.clone(),
                followed: followed.clone(),
                after,
                line: line.number,
                column: at_column,
            });
            clip.follows = Some(followed);
        }
        clip.name = fields.take_text("name")?.map(str::to_owned);
        clip.enabled = fields.take_bool("enabled")?.unwrap_or(true);
        clip.link = fields.take_id("link")?;
        clip.parent = fields.take_id("parent")?;
        if let Some(retime) = fields.take_with("retime", parse_retime)? {
            if !matches!(clip.content, Content::Media { .. }) {
                return Err(line.error(1, "`retime=` is for footage"));
            }
            clip.retime = retime;
        }
        clip.angle = fields.take_id("angle")?;
        fields.finish(2)?;
        self.record(&clip.id, line);
        if let Some(link) = &clip.link {
            self.record(link, line);
        }
        let clips = &mut self.project.sequences[s].tracks[t].clips;
        clips.push(clip);
        Ok(clips.len() - 1)
    }

    /// Place every clip that follows another: so long after that one starts, those that are
    /// themselves followed first, and each put on its track where it then starts. One that
    /// follows no clip of its sequence, or itself through others, is an error at its `at=`.
    fn follow(&mut self) {
        let mut waiting = std::mem::take(&mut self.following);
        let followers: Vec<Id> = waiting.iter().map(|one| one.clip.clone()).collect();
        while !waiting.is_empty() {
            let unplaced: Vec<Id> = waiting.iter().map(|one| one.clip.clone()).collect();
            let (mut placed, mut rest) = (false, Vec::new());
            for one in waiting {
                let tracks = &mut self.project.sequences[one.sequence].tracks;
                let followed = tracks
                    .iter()
                    .find_map(|track| track.clip(one.followed.as_str()))
                    .map(|clip| clip.start);
                let Some(followed) = followed else {
                    self.errors.push(
                        Diagnostic::error(
                            "clip.follows",
                            format!(
                                "`{}` follows `{}`, not a clip of its sequence",
                                one.clip, one.followed
                            ),
                        )
                        .entity(&one.clip)
                        .at(one.line, one.column),
                    );
                    // Read, though placed nowhere: what follows it is not told it follows itself.
                    placed = true;
                    continue;
                };
                if unplaced.contains(&one.followed) {
                    rest.push(one);
                    continue;
                }
                let clip = tracks.iter_mut().find_map(|track| {
                    let index = track.clip_index(one.clip.as_str())?;
                    Some(&mut track.clips[index])
                });
                if let Some(clip) = clip {
                    clip.start = followed + one.after;
                }
                placed = true;
            }
            if !placed {
                // Each of the rest waits for another of them.
                for one in &rest {
                    self.errors.push(
                        Diagnostic::error(
                            "clip.follows",
                            format!("`{}` follows itself through `{}`", one.clip, one.followed),
                        )
                        .entity(&one.clip)
                        .at(one.line, one.column),
                    );
                }
                return;
            }
            waiting = rest;
        }
        // Each onto its track where it starts, among the clips whose lines say where they
        // are, which stay in the order the document has them: every follower of a track
        // taken off it first, so that each goes in among clips that are in order.
        for sequence in &mut self.project.sequences {
            for track in &mut sequence.tracks {
                let (following, placed): (Vec<Clip>, Vec<Clip>) = std::mem::take(&mut track.clips)
                    .into_iter()
                    .partition(|clip| followers.contains(&clip.id));
                track.clips = placed;
                for clip in following {
                    let before = track
                        .clips
                        .iter()
                        .position(|other| other.start > clip.start)
                        .unwrap_or(track.clips.len());
                    track.clips.insert(before, clip);
                }
            }
        }
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
        let shape = take_mask_shape(&mut fields, rate)?
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
    rate: Rate,
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
        (Some(id), _) => Some(project.media(id.as_str()).map(Clock::of_media).ok_or_else(
            || {
                line.error(1, format!("unknown media `{id}`"))
                    .missing(MissingKind::Media, id.as_str())
            },
        )?),
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
    let speed = match fields.take("speed") {
        Some(arg) if source_clock.is_some() => {
            parse_speed(&arg.value, rate).map_err(|message| fields.error(arg, message))?
        }
        _ => Speed::ONE,
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
    // A script writes a parent with `set`, so a document written by hand often does too.
    if name == "parent" {
        return fields.error(
            arg,
            "`parent=` goes on the clip's own line in a document \
             (`clip c2 gen=shape@1 at=0f dur=24f parent=n1`); a script writes it `set c2 parent=n1`"
                .to_owned(),
        );
    }
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

/// `path= rate= [tc=] dur= [video= [alpha=] [bits=] [color=]] [audio=] [reel=] [recipe=] [sync=]`.
pub(crate) fn media_fields(fields: &mut Fields<'_>, id: Id) -> Result<Media, Diagnostic> {
    let line = fields.line();
    let path = fields
        .require("path")
        .and_then(|arg| fields.text(arg))?
        .to_owned();
    let rate = fields
        .take_with("rate", |text| Rate::parse(text).map_err(|e| e.to_string()))?
        .ok_or_else(|| {
            line.error(
                1,
                "missing `rate=`; `look` with `file` gives the whole line",
            )
        })?;
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
    let bits = fields.take_with("bits", |text| match text {
        "8" | "10" | "12" | "16" => text.parse::<u8>().map_err(|error| error.to_string()),
        _ => Err(format!("`{text}` is not 8, 10, 12, or 16 bits")),
    })?;
    let color = fields.take_with("color", |text| {
        kerf_model::ColorSpace::from_id(text).ok_or_else(|| {
            let known: Vec<&str> = kerf_model::ColorSpace::ALL
                .iter()
                .map(|space| space.id())
                .collect();
            format!(
                "unknown color space `{text}`; use one of {}",
                known.join(", ")
            )
        })
    })?;
    if (bits.is_some() || color.is_some()) && size.is_none() {
        return Err(line.error(1, "`bits=` and `color=` need `video=`"));
    }
    let video = size.map(|(width, height)| VideoInfo {
        width,
        height,
        alpha,
        bits: bits.unwrap_or(8),
        color: color.unwrap_or_default(),
    });
    let audio = fields.take_with("audio", parse_audio)?;
    let reel = fields.take_text("reel")?.map(str::to_owned);
    let recipe = fields.take_text("recipe")?.map(str::to_owned);
    if recipe
        .as_deref()
        .is_some_and(|recipe| recipe.trim().is_empty())
    {
        return Err(line.error(1, "`recipe=` names no file; leave it out instead"));
    }
    let sync = fields.take_with("sync", parse_synced)?;
    if sync.as_ref().is_some_and(|sync| sync.with == id) {
        return Err(line.error(1, "`sync=` names the media's own id"));
    }
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
        recipe,
        steps: Vec::new(),
        sync,
    })
}

/// A media's `sync=`: another recording of the same thing and where this one begins in it,
/// `m1@1.5s`, or `m1@-0.25s` where this one began first. In seconds, which are not rounded to a
/// frame as a position's are, since sound lines up finer than frames; or in flicks.
pub(crate) fn parse_synced(text: &str) -> Result<Synced, String> {
    let shape = || {
        format!(
            "`{text}` is not a media and where this one begins in it, in seconds, as in `m1@1.5s` or `m1@-0.25s`"
        )
    };
    let (with, at) = text.split_once('@').ok_or_else(shape)?;
    let with = Id::new(with).map_err(|error| error.to_string())?;
    let at = if let Some(flicks) = at.strip_suffix("fl") {
        Time::from_flicks(flicks.parse().map_err(|_| shape())?)
    } else {
        let seconds: f64 = at
            .strip_suffix('s')
            .and_then(|seconds| seconds.parse().ok())
            .ok_or_else(shape)?;
        Time::from_seconds(seconds).ok_or_else(shape)?
    };
    if !at.in_range() {
        return Err(format!("`{text}` is outside the supported range"));
    }
    Ok(Synced { with, at })
}

/// A media's `sync=` as documents write it: seconds where it is a whole number of
/// milliseconds, as a measurement is, and flicks otherwise.
pub(crate) fn print_synced(sync: &Synced) -> String {
    const MILLISECOND: i64 = kerf_model::time::FLICKS_PER_SECOND / 1000;
    let flicks = sync.at.flicks();
    if flicks % MILLISECOND == 0 {
        format!("{}@{}s", sync.with, (flicks / MILLISECOND) as f64 / 1000.0)
    } else {
        format!("{}@{flicks}fl", sync.with)
    }
}

/// A clip's `retime=`.
pub(crate) fn parse_retime(text: &str) -> Result<kerf_model::Retime, String> {
    kerf_model::Retime::from_id(text)
        .ok_or_else(|| format!("`{text}` is not a retime; use frames, blend, or flow"))
}

/// A sequence's `color=`: Rec.709 or an HDR signal, which is what a finished file can be.
pub(crate) fn parse_sequence_color(text: &str) -> Result<kerf_model::ColorSpace, String> {
    use kerf_model::ColorSpace;
    match ColorSpace::from_id(text) {
        Some(color @ (ColorSpace::Rec709 | ColorSpace::Rec2100Pq | ColorSpace::Rec2100Hlg)) => {
            Ok(color)
        }
        _ => Err(format!(
            "`{text}` is not a sequence color; use rec709, rec2100-pq, or rec2100-hlg"
        )),
    }
}

/// `[name=] rate= size= [audio=] [color=] [start=]`, with no tracks.
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
    let color = fields
        .take_with("color", parse_sequence_color)?
        .unwrap_or_default();
    let (start, drop_frame) = fields
        .take_with("start", |text| {
            parse_timecode(text, rate).map_err(|e| e.to_string())
        })?
        .unwrap_or((Time::ZERO, false));
    Ok(Sequence {
        buses: Vec::new(),
        id,
        name,
        rate,
        drop_frame,
        width,
        height,
        sample_rate,
        color,
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
/// `role=`: a word or quoted text; empty clears it.
pub(crate) fn take_role(fields: &mut Fields<'_>) -> Result<Option<Option<String>>, Diagnostic> {
    fields.take_with("role", |text| {
        kerf_model::track_role(text).map_err(str::to_owned)
    })
}

/// `sync=on|off`: whether ripple edits elsewhere move the track.
pub(crate) fn take_sync(fields: &mut Fields<'_>) -> Result<Option<bool>, Diagnostic> {
    fields.take_with("sync", |text| match text {
        "on" | "true" => Ok(true),
        "off" | "false" => Ok(false),
        other => Err(format!("`sync` is on or off, not `{other}`")),
    })
}

/// Text as a bare word when it reads back as one, else quoted.
pub(crate) fn word_or_quote(text: &str) -> String {
    let word = !text.is_empty()
        && !text.starts_with('#')
        && !text.chars().any(|c| {
            c.is_whitespace() || c.is_control() || matches!(c, '=' | '"' | '{' | '}' | '\\')
        });
    if word {
        text.to_owned()
    } else {
        quote(text)
    }
}

pub(crate) fn take_mask_shape(
    fields: &mut Fields<'_>,
    rate: Rate,
) -> Result<Option<MaskShape>, Diagnostic> {
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
    let four = |arg: &Arg, text: &str, what: &str| -> Result<[f64; 4], Diagnostic> {
        numbers(arg, text)?
            .try_into()
            .map_err(|_| fields.error(arg, format!("expected {what}")))
    };
    // One outline as a word, or keyframes of them: `{0f:0,0,1,1 24f:0.1,0,1,1:ease}`.
    let moving = |arg: &Arg,
                  one: &dyn Fn(&str) -> Result<MaskShape, Diagnostic>|
     -> Result<MaskShape, Diagnostic> {
        let Atom::Group(words) = &arg.value else {
            return one(fields.text(arg)?);
        };
        let keys = words
            .iter()
            .map(|word| {
                let parts: Vec<&str> = word.split(':').collect();
                let (at, shape, interp) = match parts[..] {
                    [at, shape] => (at, shape, Interp::Linear),
                    [at, shape, interp] => (
                        at,
                        shape,
                        Interp::parse(interp).ok_or_else(|| {
                            fields.error(arg, format!("unknown interpolation `{interp}`; use linear, hold, ease-in, ease-out, ease, or a curve's x1,y1,x2,y2"))
                        })?,
                    ),
                    _ => {
                        return Err(fields.error(
                            arg,
                            format!("keyframe `{word}` should read time:outline or time:outline:interp"),
                        ))
                    }
                };
                Ok(OutlineKey {
                    at: crate::text::amount(at, rate).map_err(|message| fields.error(arg, message))?,
                    shape: one(shape)?,
                    interp,
                })
            })
            .collect::<Result<Vec<_>, Diagnostic>>()?;
        Ok(MaskShape::Moving(keys))
    };
    if let Some(arg) = rect {
        return moving(arg, &|text| {
            Ok(MaskShape::Rect(four(arg, text, "left,top,right,bottom")?))
        })
        .map(Some);
    }
    if let Some(arg) = ellipse {
        return moving(arg, &|text| {
            Ok(MaskShape::Ellipse(four(
                arg,
                text,
                "x,y,radius across,radius down",
            )?))
        })
        .map(Some);
    }
    let Some(arg) = points else {
        return Ok(None);
    };
    let closed = match closed.map(|closed| fields.text(closed)).transpose()? {
        None | Some("true") => true,
        Some("false") => false,
        Some(other) => {
            return Err(fields.error(arg, format!("`closed` is true or false, not `{other}`")))
        }
    };
    moving(arg, &|text| {
        // A quoted outline is spaced, and within keyframes corners are split by `;`.
        let corners = text
            .split(|c: char| c.is_whitespace() || c == ';')
            .filter(|point| !point.is_empty())
            .map(|point| match numbers(arg, point)?.as_slice() {
                [x, y] => Ok(([*x, *y], None)),
                [x, y, soft] => Ok(([*x, *y], Some(*soft))),
                _ => Err(fields.error(arg, format!("`{point}` is not x,y or x,y,soft"))),
            })
            .collect::<Result<Vec<_>, _>>()?;
        // A corner that says nothing of how soft the edge is there says 0.
        let soft = if corners.iter().any(|(_, soft)| soft.is_some()) {
            corners
                .iter()
                .map(|(_, soft)| soft.unwrap_or(0.0))
                .collect()
        } else {
            Vec::new()
        };
        Ok(MaskShape::Polygon {
            points: corners.into_iter().map(|(point, _)| point).collect(),
            closed,
            soft,
        })
    })
    .map(Some)
}

/// `rect=`, `ellipse=`, or `points=` (and `closed=false`) for an outline.
pub(crate) fn mask_shape_text(shape: &MaskShape, rate: Rate) -> String {
    let list = |values: &[f64]| {
        values
            .iter()
            .map(|v| number(*v))
            .collect::<Vec<_>>()
            .join(",")
    };
    let corners = |points: &[[f64; 2]], soft: &[f64]| -> Vec<String> {
        let at = |corner: usize| soft.get(corner).into_iter().copied();
        points
            .iter()
            .enumerate()
            .map(|(corner, point)| {
                list(&point.iter().copied().chain(at(corner)).collect::<Vec<_>>())
            })
            .collect()
    };
    match shape {
        MaskShape::Rect(rect) => format!(" rect={}", list(rect)),
        MaskShape::Ellipse(ellipse) => format!(" ellipse={}", list(ellipse)),
        MaskShape::Polygon {
            points,
            closed,
            soft,
        } => {
            let open = if *closed { "" } else { " closed=false" };
            format!(" points={}{open}", quote(&corners(points, soft).join(" ")))
        }
        MaskShape::Moving(keys) => {
            let key_text = |key: &OutlineKey| {
                let shape = match &key.shape {
                    MaskShape::Rect(values) | MaskShape::Ellipse(values) => list(values),
                    MaskShape::Polygon { points, soft, .. } => corners(points, soft).join(";"),
                    MaskShape::Moving(_) => String::new(),
                };
                let mut text = format!("{}:{shape}", crate::text::print_offset(key.at, rate));
                if key.interp != Interp::Linear {
                    text.push(':');
                    text.push_str(&key.interp.name());
                }
                text
            };
            let words: Vec<String> = keys.iter().map(key_text).collect();
            let (key, open) = match keys.first().map(|key| &key.shape) {
                Some(MaskShape::Ellipse(_)) => ("ellipse", ""),
                Some(MaskShape::Polygon { closed: false, .. }) => ("points", " closed=false"),
                Some(MaskShape::Polygon { .. }) => ("points", ""),
                _ => ("rect", ""),
            };
            format!(" {key}={{{}}}{open}", words.join(" "))
        }
    }
}

/// A mask after its name: its outline, mode and inversion when not the
/// defaults, and its properties.
pub(crate) fn mask_text(mask: &Mask, catalog: &Catalog, rate: Rate) -> String {
    let mut out = mask_shape_text(&mask.shape, rate);
    if mask.mode != MaskMode::Add {
        write!(out, " mode={}", mask.mode.name()).expect("writing to a String cannot fail");
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
    writeln!(out, "kerf {VERSION}").expect("writing to a String cannot fail");
    write!(out, "project name={}", quote(&project.name)).expect("writing to a String cannot fail");
    if !project.retired.is_empty() {
        let retired: Vec<&str> = project.retired.iter().map(Id::as_str).collect();
        write!(out, " retired={}", retired.join(",")).expect("writing to a String cannot fail");
    }
    out.push('\n');
    if !project.media.is_empty() {
        out.push('\n');
        for media in &project.media {
            out.push_str(&media_line(media));
            out.push('\n');
            for step in &media.steps {
                writeln!(out, "  {}", crate::making::print_step(step))
                    .expect("writing to a String cannot fail");
            }
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
        write!(line, " tc={}", clock.print_position(Time::ZERO))
            .expect("writing to a String cannot fail");
    }
    write!(line, " dur={}", clock.print_duration(media.duration))
        .expect("writing to a String cannot fail");
    if let Some(video) = media.video {
        write!(line, " video={}x{}", video.width, video.height)
            .expect("writing to a String cannot fail");
        if video.alpha {
            line.push_str(" alpha=true");
        }
        if video.bits != 8 {
            write!(line, " bits={}", video.bits).expect("writing to a String cannot fail");
        }
        if video.color != kerf_model::ColorSpace::default() {
            write!(line, " color={}", video.color.id()).expect("writing to a String cannot fail");
        }
    }
    if let Some(audio) = media.audio {
        write!(line, " audio={}ch@{}", audio.channels, audio.sample_rate)
            .expect("writing to a String cannot fail");
    }
    if let Some(reel) = &media.reel {
        write!(line, " reel={}", quote(reel)).expect("writing to a String cannot fail");
    }
    if let Some(recipe) = &media.recipe {
        write!(line, " recipe={}", quote(recipe)).expect("writing to a String cannot fail");
    }
    if let Some(sync) = &media.sync {
        write!(line, " sync={}", print_synced(sync)).expect("writing to a String cannot fail");
    }
    line
}

pub(crate) fn sequence_line(sequence: &Sequence) -> String {
    let clock = Clock::of_sequence(sequence);
    let color = if sequence.color == kerf_model::ColorSpace::default() {
        String::new()
    } else {
        format!(" color={}", sequence.color.id())
    };
    format!(
        "sequence {} name={} rate={} size={}x{} audio={}{color} start={}",
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
        .expect("writing to a String cannot fail");
    }
    out.push('\n');
    for bus in &sequence.buses {
        write!(out, "  bus {}", bus.id).expect("writing to a String cannot fail");
        if let Some(name) = &bus.name {
            write!(out, " name={}", quote(name)).expect("writing to a String cannot fail");
        }
        if let Some(to) = &bus.to {
            write!(out, " to={to}").expect("writing to a String cannot fail");
        }
        out.push_str(&params_text(
            &bus.params,
            catalog.props(Domain::Audio),
            clock.rate,
        ));
        out.push('\n');
        for effect in &bus.effects {
            write!(out, "    fx {}", print_fx_ref(&effect.fx, effect.version))
                .expect("writing to a String cannot fail");
            if !effect.enabled {
                out.push_str(" enabled=false");
            }
            if let Some(def) = catalog.fx(effect.fx.as_str()) {
                out.push_str(&params_text(
                    &effect.params,
                    def.params.as_slice(),
                    clock.rate,
                ));
            }
            out.push('\n');
        }
    }
    for track in &sequence.tracks {
        write!(out, "  track {} {}", track.id, track.kind.name())
            .expect("writing to a String cannot fail");
        if let Some(name) = &track.name {
            write!(out, " name={}", quote(name)).expect("writing to a String cannot fail");
        }
        for (flag, value, default) in [
            ("enabled", track.enabled, true),
            ("locked", track.locked, false),
            ("muted", track.muted, false),
        ] {
            if value != default {
                write!(out, " {flag}={value}").expect("writing to a String cannot fail");
            }
        }
        if let Some(role) = &track.role {
            write!(out, " role={}", word_or_quote(role)).expect("writing to a String cannot fail");
        }
        if !track.sync_locked {
            out.push_str(" sync=off");
        }
        if let Some(bus) = &track.bus {
            write!(out, " bus={bus}").expect("writing to a String cannot fail");
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
            .expect("writing to a String cannot fail");
            if transition.align != Align::Center {
                write!(out, " align={}", transition.align.name())
                    .expect("writing to a String cannot fail");
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
        .expect("writing to a String cannot fail");
        if marker.duration != Time::ZERO {
            write!(out, " dur={}", clock.print_duration(marker.duration))
                .expect("writing to a String cannot fail");
        }
        if let Some(name) = &marker.name {
            write!(out, " name={}", quote(name)).expect("writing to a String cannot fail");
        }
        if let Some(color) = marker.color {
            write!(out, " color={}", color.name()).expect("writing to a String cannot fail");
        }
        if let Some(note) = &marker.note {
            write!(out, " note={}", quote(note)).expect("writing to a String cannot fail");
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
    // Where it is by the clip it follows, when it follows one.
    let followed = clip.follows.as_ref().and_then(|followed| {
        let start = project.clip(followed.as_str())?.start;
        Some((followed, clip.start - start))
    });
    let at = match followed {
        Some((followed, Time::ZERO)) => format!("{followed}.start"),
        Some((followed, after)) => format!("{followed}.start{}", clock.print_delta(after)),
        None => clock.print_position(clip.start),
    };
    write!(
        out,
        "    clip {} {} at={at} dur={}",
        clip.id,
        content_text(project, &clip.content),
        clock.print_duration(clip.duration)
    )
    .expect("writing to a String cannot fail");
    if !clip.speed().is_one() {
        write!(out, " speed={}", print_speed(clip.speed(), clock.rate))
            .expect("writing to a String cannot fail");
    }
    if clip.retime != kerf_model::Retime::default() {
        write!(out, " retime={}", clip.retime.id()).expect("writing to a String cannot fail");
    }
    if let Some(angle) = &clip.angle {
        write!(out, " angle={angle}").expect("writing to a String cannot fail");
    }
    if let Some(name) = &clip.name {
        write!(out, " name={}", quote(name)).expect("writing to a String cannot fail");
    }
    if !clip.enabled {
        out.push_str(" enabled=false");
    }
    if let Some(link) = &clip.link {
        write!(out, " link={link}").expect("writing to a String cannot fail");
    }
    if let Some(parent) = &clip.parent {
        write!(out, " parent={parent}").expect("writing to a String cannot fail");
    }
    if options.annotate {
        out.push_str("  #");
        if followed.is_some() {
            write!(out, " at {},", clock.print_position(clip.start))
                .expect("writing to a String cannot fail");
        }
        write!(
            out,
            " ends {} ({})",
            clock.print_position(clip.end()),
            clip.duration
        )
        .expect("writing to a String cannot fail");
        if let (Some((_, source_out)), Some(source)) =
            (clip.source_range(), source_clock(project, &clip.content))
        {
            write!(out, ", src out {}", source.print_position(source_out))
                .expect("writing to a String cannot fail");
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
        writeln!(out, "      set{set}").expect("writing to a String cannot fail");
    }
    for mask in &clip.masks {
        writeln!(
            out,
            "      mask {}{}",
            mask.name,
            mask_text(mask, catalog, clock.rate)
        )
        .expect("writing to a String cannot fail");
    }
    for effect in &clip.effects {
        write!(out, "      fx {}", print_fx_ref(&effect.fx, effect.version))
            .expect("writing to a String cannot fail");
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
        // A track, mask, or marker name is a word, as ids are.
        let reference = defs.iter().any(|def| {
            def.name == *name
                && matches!(
                    def.ty,
                    ParamType::Track | ParamType::Sound | ParamType::Mask | ParamType::Time
                )
        });
        let time = defs
            .iter()
            .any(|def| def.name == *name && def.ty == ParamType::Time);
        match param {
            Param::Static(Value::Text(id)) if reference && Id::new(id.as_str()).is_ok() => {
                write!(out, " {name}={id}").expect("writing to a String cannot fail");
            }
            Param::Static(Value::Number(seconds)) if time => {
                let at = Time::from_seconds(*seconds).unwrap_or(Time::ZERO);
                write!(out, " {name}={}", crate::text::print_offset(at, rate))
                    .expect("writing to a String cannot fail");
            }
            _ => write!(out, " {name}={}", print_param(param, rate))
                .expect("writing to a String cannot fail"),
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
}
