//! A native CMX3600 edit decision list of a sequence, for conform and for
//! review paperwork. Pure Rust: unlike [`crate::otio`], nothing runs Python.
//!
//! ```text
//! TITLE: Main Edit
//! FCM: NON-DROP FRAME
//!
//! 001  A001     V     C        14:22:35:00 14:22:41:12 01:00:00:00 01:00:06:12
//! * FROM CLIP NAME: A001_C003.mov
//! * SOURCE FILE: A001_C003.mov
//! * LOC: 01:00:05:00 RED     pickup needed
//! ```
//!
//! Every enabled picture track writes `V` events and every audible sound
//! track `A`, `A2`, `A3`, … events, in track order. A transition becomes the
//! two-line form (`D` for dissolves and fades, `W001` for wipes) and a speed
//! change an `M2` line. Generators and nested sequences are `AX` sources
//! named by their `FROM CLIP NAME`. Locators (`* LOC:`) follow the event
//! they fall in. Timecode is on the sequence's rate: 23.976, 24, 25, 29.97
//! (drop-frame when the sequence is), 30, 50, 59.94, or 60.

use kerf_model::time::format_timecode;
use kerf_model::{
    Clip, Content, Id, MarkerColor, Project, Rate, Sequence, Time, TrackKind, Transition,
};
use std::collections::HashMap;
use std::fmt::{self, Write as _};
use std::path::Path;

/// A note at a time on the sequence (an offset from its first frame), as a
/// `* LOC:` line: a marker or a review comment.
#[derive(Clone, Debug, PartialEq)]
pub struct Locator {
    pub at: Time,
    /// An Avid locator colour such as `RED`; written upper-case.
    pub color: String,
    pub text: String,
}

impl Locator {
    pub fn new(at: Time, color: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            at,
            color: color.into(),
            text: text.into(),
        }
    }
}

impl From<(Time, &str, &str)> for Locator {
    fn from((at, color, text): (Time, &str, &str)) -> Self {
        Self::new(at, color, text)
    }
}

/// The Avid locator colour nearest a marker colour.
pub fn locator_color(color: Option<MarkerColor>) -> &'static str {
    match color {
        None | Some(MarkerColor::Red) | Some(MarkerColor::Orange) => "RED",
        Some(MarkerColor::Yellow) => "YELLOW",
        Some(MarkerColor::Green) => "GREEN",
        Some(MarkerColor::Cyan) => "CYAN",
        Some(MarkerColor::Blue) => "BLUE",
        Some(MarkerColor::Purple) | Some(MarkerColor::Pink) => "MAGENTA",
        Some(MarkerColor::White) => "WHITE",
    }
}

/// The markers of `sequence` as locators: the name, then the note.
pub fn marker_locators(sequence: &Sequence) -> Vec<Locator> {
    sequence
        .markers
        .iter()
        .map(|marker| {
            let text = [marker.name.as_deref(), marker.note.as_deref()]
                .into_iter()
                .flatten()
                .filter(|part| !part.trim().is_empty())
                .collect::<Vec<_>>()
                .join(" - ");
            Locator::new(marker.at, locator_color(marker.color), text)
        })
        .collect()
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EdlError {
    /// CMX3600 timecode has no form for this rate.
    UnsupportedRate(Rate),
}

impl fmt::Display for EdlError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedRate(rate) => write!(
                f,
                "an EDL cannot carry {rate} fps; use 23.976, 24, 25, 29.97, 30, 50, 59.94, or 60"
            ),
        }
    }
}

impl std::error::Error for EdlError {}

/// The rates an EDL's timecode can count.
const RATES: [(u32, u32); 8] = [
    (24_000, 1001),
    (24, 1),
    (25, 1),
    (30_000, 1001),
    (30, 1),
    (50, 1),
    (60_000, 1001),
    (60, 1),
];

/// The CMX3600 EDL of `sequence` in `project`, with `locators` under the
/// events they fall in.
pub fn to_edl(
    project: &Project,
    sequence: &Sequence,
    locators: &[Locator],
) -> Result<String, EdlError> {
    let rate = sequence.rate;
    if !RATES.contains(&(rate.num(), rate.den())) {
        return Err(EdlError::UnsupportedRate(rate));
    }
    let drop = sequence.drop_frame && rate.supports_drop_frame();
    let writer = Writer {
        project,
        sequence,
        rate,
        drop,
    };
    let mut events = Vec::new();
    let mut audio = 0;
    for track in &sequence.tracks {
        let channel = match track.kind {
            TrackKind::Video if track.enabled => "V".to_owned(),
            TrackKind::Audio if track.enabled && !track.muted => {
                audio += 1;
                if audio == 1 {
                    "A".to_owned()
                } else {
                    format!("A{audio}")
                }
            }
            _ => continue,
        };
        let incoming: HashMap<&Id, &Transition> = track
            .transitions
            .iter()
            .map(|transition| (&transition.to, transition))
            .collect();
        let outgoing: HashMap<&Id, &Transition> = track
            .transitions
            .iter()
            .map(|transition| (&transition.from, transition))
            .collect();
        for clip in track.clips.iter().filter(|clip| clip.enabled) {
            let entering = incoming
                .get(&clip.id)
                .and_then(|t| Some((*t, track.clip(t.from.as_str())?)));
            let before_in = entering.map_or(Time::ZERO, |(t, _)| t.split_at_cut(rate).0);
            let before_out = outgoing
                .get(&clip.id)
                .map_or(Time::ZERO, |t| t.split_at_cut(rate).0);
            let from = clip.start - before_in;
            let to = clip.end() - before_out;
            if to <= from {
                continue;
            }
            events.push(writer.event(&channel, clip, entering, from, to));
        }
    }

    let mut out = String::new();
    let title: String = sequence
        .name
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    writeln!(out, "TITLE: {}", title.trim()).unwrap();
    writeln!(
        out,
        "FCM: {}",
        if drop { "DROP FRAME" } else { "NON-DROP FRAME" }
    )
    .unwrap();
    // Each locator goes under the first event playing at its time, else
    // under the last event before it, else after the header.
    let mut placed: Vec<Vec<&Locator>> = vec![Vec::new(); events.len()];
    let mut loose = Vec::new();
    let mut locators: Vec<&Locator> = locators.iter().collect();
    locators.sort_by_key(|locator| locator.at);
    for locator in locators {
        let inside = events
            .iter()
            .position(|event| event.from <= locator.at && locator.at < event.to);
        let before = || {
            events
                .iter()
                .enumerate()
                .filter(|(_, event)| event.from <= locator.at)
                .max_by_key(|(_, event)| event.from)
                .map(|(index, _)| index)
        };
        match inside.or_else(before) {
            Some(index) => placed[index].push(locator),
            None => loose.push(locator),
        }
    }
    for locator in loose {
        out.push('\n');
        writer.locator(&mut out, locator);
    }
    for (number, (event, locators)) in events.iter().zip(&placed).enumerate() {
        out.push('\n');
        let number = number + 1;
        for line in &event.lines {
            writeln!(out, "{number:03}  {line}").unwrap();
        }
        for line in &event.motion {
            writeln!(out, "{line}").unwrap();
        }
        for comment in &event.comments {
            writeln!(out, "{comment}").unwrap();
        }
        for locator in locators {
            writer.locator(&mut out, locator);
        }
    }
    Ok(out)
}

struct Event {
    /// Record in and out, as sequence offsets.
    from: Time,
    to: Time,
    /// The event lines after the event number, the `M2` motion lines that
    /// follow them unnumbered, and the comments below.
    lines: Vec<String>,
    motion: Vec<String>,
    comments: Vec<String>,
}

/// What an event names as its source.
struct Source {
    reel: String,
    name: String,
    file: Option<String>,
}

struct Writer<'a> {
    project: &'a Project,
    sequence: &'a Sequence,
    rate: Rate,
    drop: bool,
}

impl Writer<'_> {
    fn tc(&self, time: Time) -> String {
        format_timecode(time, self.rate, self.drop)
    }

    /// Record timecode of a sequence offset.
    fn record(&self, time: Time) -> String {
        self.tc(self.sequence.start + time)
    }

    fn source(&self, clip: &Clip) -> Source {
        let name = |fallback: &str| clip.name.clone().unwrap_or_else(|| fallback.to_owned());
        match &clip.content {
            Content::Media { media, .. } => {
                let found = self.project.media(media.as_str());
                let file = found.and_then(|media| {
                    Path::new(&media.path)
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                });
                let stem = found.and_then(|media| {
                    Path::new(&media.path)
                        .file_stem()
                        .map(|stem| stem.to_string_lossy().into_owned())
                });
                let reel = found
                    .and_then(|media| media.reel.clone())
                    .or(stem)
                    .unwrap_or_else(|| media.to_string());
                Source {
                    reel: reel_name(&reel),
                    name: name(file.as_deref().unwrap_or(media.as_str())),
                    file,
                }
            }
            Content::Sequence { sequence, .. } => Source {
                reel: "AX".into(),
                name: name(
                    self.project
                        .sequence(sequence.as_str())
                        .map_or(sequence.as_str(), |nested| nested.name.as_str()),
                ),
                file: None,
            },
            Content::Generator { fx, .. } => Source {
                reel: "AX".into(),
                name: name(fx.as_str()),
                file: None,
            },
            Content::Caption { text } => Source {
                reel: "AX".into(),
                name: name(text),
                file: None,
            },
        }
    }

    /// The source time `clip` plays at sequence offset `at`, on its media's
    /// (or nested sequence's) timecode.
    fn source_at(&self, clip: &Clip, at: Time) -> Time {
        let origin = match &clip.content {
            Content::Media { media, .. } => self
                .project
                .media(media.as_str())
                .map_or(Time::ZERO, |media| media.start),
            Content::Sequence { sequence, .. } => self
                .project
                .sequence(sequence.as_str())
                .map_or(Time::ZERO, |nested| nested.start),
            Content::Generator { .. } | Content::Caption { .. } => Time::ZERO,
        };
        origin + clip.source_at(at).unwrap_or(at - clip.start)
    }

    /// One event line after the number: reel, channel, transition, then
    /// source in and out and record in and out. The source runs as long as
    /// the record; an `M2` line says at what speed.
    fn line(
        &self,
        reel: &str,
        channel: &str,
        transition: &str,
        source: Time,
        from: Time,
        to: Time,
    ) -> String {
        format!(
            "{reel:<8} {channel:<5} {transition:<8} {} {} {} {}",
            self.tc(source),
            self.tc(source + (to - from)),
            self.record(from),
            self.record(to)
        )
    }

    fn event(
        &self,
        channel: &str,
        clip: &Clip,
        entering: Option<(&Transition, &Clip)>,
        from: Time,
        to: Time,
    ) -> Event {
        let source = self.source(clip);
        let src_in = self.source_at(clip, from);
        let mut lines = Vec::new();
        let mut motion = Vec::new();
        let mut comments = Vec::new();
        match entering {
            None => lines.push(self.line(&source.reel, channel, "C", src_in, from, to)),
            Some((transition, outgoing)) => {
                let previous = self.source(outgoing);
                let held = self.source_at(outgoing, from);
                lines.push(self.line(&previous.reel, channel, "C", held, from, from));
                let code = match transition.fx.as_str() {
                    "wipe" => "W001",
                    _ => "D",
                };
                let frames = self.rate.frames_round(transition.duration);
                lines.push(self.line(
                    &source.reel,
                    channel,
                    &format!("{code:<4} {frames:03}"),
                    src_in,
                    from,
                    to,
                ));
                comments.push(format!("* FROM CLIP NAME: {}", comment(&previous.name)));
                comments.push(format!("* TO CLIP NAME: {}", comment(&source.name)));
                if !matches!(transition.fx.as_str(), "dissolve" | "crossfade" | "wipe") {
                    comments.push(format!(
                        "* EFFECT NAME: {}",
                        comment(transition.fx.as_str())
                    ));
                }
            }
        }
        let speed = clip.speed();
        if speed.value() != 1.0 {
            motion.push(format!(
                "M2   {:<8}       {:>05.1}                {}",
                source.reel,
                self.rate.fps() * speed.value(),
                self.tc(src_in)
            ));
        }
        if entering.is_none() {
            comments.push(format!("* FROM CLIP NAME: {}", comment(&source.name)));
        }
        if let Some(file) = &source.file {
            comments.push(format!("* SOURCE FILE: {}", comment(file)));
        }
        Event {
            from,
            to,
            lines,
            motion,
            comments,
        }
    }

    fn locator(&self, out: &mut String, locator: &Locator) {
        let color = locator.color.trim().to_uppercase();
        let color = if color.is_empty() {
            "RED".to_owned()
        } else {
            color
        };
        let line = format!(
            "* LOC: {} {color:<7} {}",
            self.record(locator.at),
            comment(&locator.text)
        );
        writeln!(out, "{}", line.trim_end()).unwrap();
    }
}

/// A reel name an EDL reader takes: letters, digits, and `_`, at most
/// eight of them.
fn reel_name(reel: &str) -> String {
    let name: String = reel
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
        .take(8)
        .collect();
    if name.is_empty() {
        "AX".into()
    } else {
        name
    }
}

/// Comment text on one line.
fn comment(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}
